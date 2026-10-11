//! Fresh host observations. None of these identities grants cleanup authority.
use std::{path::Path, os::unix::fs::MetadataExt};
use cockpit_protocol::orchestration::*;
use sha2::{Digest, Sha256};
use crate::{InspectionError, process_identity, space_git};
use super::{OrchestrationService, claims::{AttachmentReference, WriterExitProof}};

fn unavailable(text: &str) -> InspectionError { InspectionError::new("project_identity_unavailable", text) }
fn path_text(path: &Path) -> Result<String, InspectionError> {
    path.to_str().map(str::to_owned).ok_or_else(|| unavailable("Physical directory is not UTF-8"))
}
fn key(domain: &[u8], machine: &str, pieces: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    for piece in std::iter::once(machine).chain(pieces.iter().copied()) {
        digest.update((piece.len() as u64).to_be_bytes()); digest.update(piece.as_bytes());
    }
    format!("{:x}", digest.finalize())
}
fn directory(path: &Path) -> Result<(String, DirectoryIdentity), InspectionError> {
    if !path.is_absolute() { return Err(unavailable("Authoritative directory must be absolute")); }
    let canonical = std::fs::canonicalize(path).map_err(|_| unavailable("Physical directory cannot be resolved"))?;
    let metadata = std::fs::metadata(&canonical).map_err(|_| unavailable("Physical directory cannot be inspected"))?;
    if !metadata.is_dir() { return Err(unavailable("Physical checkout is not a directory")); }
    Ok((path_text(&canonical)?, DirectoryIdentity { device: metadata.dev().to_string(), inode: metadata.ino().to_string() }))
}

/// Recheck immutable physical inputs while the caller holds the state lock.
pub(crate) fn revalidate_checkout(checkout: &CheckoutIdentity) -> Result<(), InspectionError> {
    let changed = || InspectionError::new("checkout_identity_changed", "Physical checkout directory identity changed");
    if process_identity::machine_id().as_ref() != Some(&checkout.machine_id)
        || directory(Path::new(&checkout.path))? != (checkout.path.clone(), checkout.directory_identity.clone()) {
        return Err(changed());
    }
    if let Some(git) = &checkout.git {
        if directory(Path::new(&git.common_dir))?.0 != git.common_dir
            || directory(Path::new(&git.worktree_git_dir))?.0 != git.worktree_git_dir {
            return Err(changed());
        }
    }
    Ok(())
}

/// Only for the owner-created dedicated supervisor directory. Ordinary targets
/// must use Git discovery, never this directory-only constructor.
pub(crate) fn resolve_owned_directory(path: &Path) -> Result<(ProjectIdentity, CheckoutIdentity), InspectionError> {
    let machine = process_identity::machine_id().ok_or_else(|| unavailable("Stable local machine identity is unavailable"))?;
    let (root, identity) = directory(path)?;
    if Path::new(&root).join(".git").exists() { return Err(unavailable("Owned supervisor directory unexpectedly contains Git metadata")); }
    let project = ProjectIdentity { key: key(b"cockpit:project:directory:v1\0", &machine, &[&root]), machine_id: machine.clone(),
        source: ProjectSource::Directory { root: root.clone() }, repository_id: None, source_workspace_id: None };
    let checkout = CheckoutIdentity { key: key(b"cockpit:checkout:v1\0", &machine, &[&root, ""]), machine_id: machine,
        path: root, directory_identity: identity, git: None };
    Ok((project, checkout))
}

/// Git inspection is bounded and disables hooks, prompts, fsmonitor and ambient
/// Git path overrides. Only Git's explicit not-a-repository reply admits a plain
/// directory; unavailable Git never silently changes a project's identity.
pub async fn resolve_directory(path: &Path, repository_id: Option<String>, source_workspace_id: Option<String>) -> Result<(ProjectIdentity, CheckoutIdentity), InspectionError> {
    let machine = process_identity::machine_id().ok_or_else(|| unavailable("Stable local machine identity is unavailable"))?;
    let (canonical, before) = directory(path)?;
    let probe = space_git::git_output(Path::new(&canonical), &["rev-parse", "--show-toplevel"]).await?;
    let (project_source, checkout_path, git) = if probe.status.success() {
        let root = std::str::from_utf8(&probe.stdout).ok().map(str::trim).filter(|s| !s.is_empty() && !s.contains('\n'))
            .ok_or_else(|| unavailable("Git root evidence is malformed"))?;
        let (root, _) = directory(Path::new(root))?;
        let common = space_git::git_line(Path::new(&root), &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await?;
        let git_dir = space_git::git_line(Path::new(&root), &["rev-parse", "--absolute-git-dir"]).await?;
        let (common, _) = directory(Path::new(&common))?;
        let (git_dir, _) = directory(Path::new(&git_dir))?;
        (ProjectSource::Git { common_dir: common.clone() }, root, Some(CheckoutGitIdentity { common_dir: common, worktree_git_dir: git_dir }))
    } else {
        let error = std::str::from_utf8(&probe.stderr).unwrap_or("");
        if !error.starts_with("fatal: not a git repository") { return Err(unavailable("Git could not establish repository or plain-directory identity")); }
        (ProjectSource::Directory { root: canonical.clone() }, canonical.clone(), None)
    };
    if directory(path)? != (canonical, before) { return Err(unavailable("Source directory changed during project inspection")); }
    let (checkout_path, directory_identity) = directory(Path::new(&checkout_path))?;
    let project_key = match &project_source {
        ProjectSource::Git { common_dir } => key(b"cockpit:project:git:v1\0", &machine, &[common_dir]),
        ProjectSource::Directory { root } => key(b"cockpit:project:directory:v1\0", &machine, &[root]),
    };
    let checkout_key = key(b"cockpit:checkout:v1\0", &machine, &[&checkout_path, git.as_ref().map_or("", |g| g.worktree_git_dir.as_str())]);
    Ok((ProjectIdentity { key: project_key, machine_id: machine.clone(), source: project_source, repository_id, source_workspace_id },
        CheckoutIdentity { key: checkout_key, machine_id: machine, path: checkout_path, directory_identity, git }))
}

/// A reboot proves the old OS incarnations gone, not a restored/new occupant.
/// Reconcile the protected physical containers before publishing a replacement.
pub async fn replacement_consumers_preflight(herdr: &dyn super::herdr::OrchestrationHerdr, run: &Run) -> Result<(), InspectionError> {
    let protected = || InspectionError::new("consumer_activity_unproven",
        "The original terminal has pending, shared, foreign or newly restored consumers. Its checkout claim remains protected.");
    let attachment = run.attachment.as_ref().ok_or_else(protected)?;
    let location = attachment.location.as_ref().ok_or_else(protected)?;
    let runtime = herdr.runtime(&run.session_id).await.map_err(|_| InspectionError::new(
        "resume_observation_unavailable", "Fresh physical consumer evidence is unavailable; no replacement was launched."))?;
    let same_endpoint = runtime.endpoint_identity == location.endpoint_identity;
    let panes: Vec<_> = runtime.panes.iter().filter(|pane|
        pane.tab_label == location.launch_tag
            || pane.agent_name.as_deref() == Some(location.launch_tag.as_str())
            || location.terminal_id.as_ref().is_some_and(|id| pane.terminal_id.as_ref() == Some(id))
            || (same_endpoint && (pane.pane_id == location.pane_id || pane.tab_id == location.tab_id))
    ).collect();
    if panes.len() > 1 { return Err(protected()); }
    for pane in panes {
        if pane.launch_pending
            || pane.workspace_id != location.workspace_id
            || pane.agent_kind.as_deref().is_some_and(|kind| !super::NativeAgentKind::is_omp(kind))
            || pane.native_session_id.as_ref().is_some_and(|id| attachment.main_session_id.as_ref() != Some(id)) {
            return Err(protected());
        }
        let info = herdr.pane_process_info(&run.session_id, &runtime.endpoint_identity, &pane.pane_id)
            .await.map_err(|_| protected())?;
        if info.pane_id != pane.pane_id
            || info.shell_pid.is_none() || info.shell_identity.is_none() || info.processes.is_empty() {
            return Err(protected());
        }
        let original = |pid: u32| attachment.process.iter().chain(attachment.shell.iter().map(|s| &s.process))
            .any(|process| process.pid == pid && matches!(
                process_identity::same_machine_process_running(process, attachment.machine_id.as_deref()), Ok(true)));
        if info.shell_pid.is_some_and(|pid| !original(pid))
            || info.processes.iter().any(|(pid, _)| !original(*pid)) {
            return Err(protected());
        }
    }
    Ok(())
}

impl OrchestrationService {
    /// Private proof constructor: the host cannot invent `all_consumers_absent`.
    /// A stale projection, heartbeat, endpoint failure or closed status is never
    /// substituted for OS incarnation evidence.
    pub fn observe_writer_exit(&self, expected: &Run, checkout: &CheckoutIdentity) -> Result<WriterExitProof, InspectionError> {
        self.observe_attachment_exit(expected, Some(checkout))
    }

    pub fn observe_root_exit(&self, expected: &Run) -> Result<WriterExitProof, InspectionError> {
        if expected.kind == RunKind::Worker { return Err(exit_unproven()); }
        self.observe_attachment_exit(expected, None)
    }

    fn observe_attachment_exit(&self, expected: &Run, checkout: Option<&CheckoutIdentity>) -> Result<WriterExitProof, InspectionError> {
        let locked = self.store.lock()?;
        let state = locked.read()?;
        if let Some(checkout) = checkout { revalidate_checkout(checkout)?; }
        let run = state.runs.iter().find(|r| r.run_id == expected.run_id).ok_or_else(exit_unproven)?;
        let old = run.attachment.as_ref().ok_or_else(exit_unproven)?;
        if expected.attachment.as_ref().is_none_or(|a| a.attachment_id != old.attachment_id || a.epoch != old.epoch)
            || checkout.is_some_and(|c| run.checkout.as_ref() != Some(c)) { return Err(exit_unproven()); }
        let machine = process_identity::machine_id().ok_or_else(exit_unproven)?;
        let boot = process_identity::kernel_boot_id().ok_or_else(exit_unproven)?;
        if old.machine_id.as_deref() != Some(machine.as_str())
            || checkout.is_some_and(|c| c.machine_id != machine) { return Err(exit_unproven()); }
        let previous = old.process.as_ref().and_then(|p| p.kernel_boot_id.as_ref())
            .or_else(|| old.shell.as_ref().and_then(|s| s.process.kernel_boot_id.as_ref())).ok_or_else(exit_unproven)?;
        if previous == &boot { return Err(consumer_unproven()); }
        let absent = |attachment: &ExecutionAttachment, kind: RunKind| -> bool {
            if attachment.machine_id.as_deref() != Some(machine.as_str()) { return false; }
            let processes = attachment.process.iter().chain(attachment.shell.iter().map(|s| &s.process));
            let mut count = 0;
            for process in processes {
                count += 1;
                if process.kernel_boot_id.as_deref().is_none_or(|b| b.is_empty())
                    || process_identity::same_machine_process_running(process, Some(&machine)).unwrap_or(true) { return false; }
            }
            // Managed launches always retain shell proof; adopted roots can
            // instead have a locally attested process without a launch shell.
            count > 0 && (attachment.shell.is_some() || kind == RunKind::Adopted)
        };
        if !absent(old, run.kind) { return Err(exit_unproven()); }
        let mut owners = std::collections::HashSet::from([run.run_id.as_str()]);
        loop {
            let before = owners.len();
            for child in &state.runs {
                if child.parent_run_id.as_deref().is_some_and(|parent| owners.contains(parent)) {
                    owners.insert(child.run_id.as_str());
                }
            }
            if owners.len() == before { break; }
        }
        for child in state.runs.iter().filter(|r| r.run_id != run.run_id && owners.contains(r.run_id.as_str())) {
            if child.attachment.as_ref().is_none_or(|a| !absent(a, child.kind)) { return Err(exit_unproven()); }
            let attachment = child.attachment.as_ref().ok_or_else(exit_unproven)?;
            if attachment.process.iter().chain(attachment.shell.iter().map(|shell| &shell.process))
                .any(|process| process.kernel_boot_id.as_deref() == Some(boot.as_str())) {
                return Err(consumer_unproven());
            }
        }
        let incarnation = AttachmentReference { run_id: run.run_id.clone(), attachment_id: old.attachment_id.clone(), attachment_epoch: old.epoch };
        let claim_revision = if let Some(checkout) = checkout {
            let claim = state.checkout_claims.iter().find(|c| Some(c.claim_id.as_str()) == run.checkout_claim_id.as_deref()).ok_or_else(exit_unproven)?;
            if claim.checkout != *checkout || claim.holder.as_ref() != Some(&incarnation) { return Err(exit_unproven()); }
            claim.revision
        } else { 0 };
        Ok(WriterExitProof { incarnation, claim_revision, checkout: checkout.cloned(), machine_id: machine,
            previous_kernel_boot_id: previous.clone(), observed_kernel_boot_id: boot, all_consumers_absent: true })
    }
}

fn exit_unproven() -> InspectionError {
    InspectionError::new("writer_exit_unproven", "Recorded writer, launch shell and every known consumer must be proven absent on the same local machine")
}


fn consumer_unproven() -> InspectionError {
    InspectionError::new("consumer_activity_unproven",
        "This OMP runtime cannot prove all same-boot checkout consumers stopped. The checkout remains protected until same-machine reboot.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn plain_nested_directory_keeps_exact_project_but_distinct_checkout() {
        if process_identity::machine_id().is_none() { return; }
        let root = std::env::temp_dir().join(format!("cockpit-project-identity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        let (outer, a) = resolve_directory(&root, None, None).await.unwrap();
        let (inner, b) = resolve_directory(&root.join("nested"), None, None).await.unwrap();
        assert_ne!(outer.key, inner.key); assert_ne!(a.key, b.key);
        assert!(super::super::claims::checkouts_conflict(&a, &b));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn relative_directory_never_resolves_from_cli_cwd() {
        assert!(resolve_directory(Path::new("."), None, None).await.is_err());
    }
    #[tokio::test]
    async fn linked_worktrees_share_project_not_checkout_and_clones_stay_distinct() {
        if process_identity::machine_id().is_none() { return; }
        use crate::space_git::tests::git;
        let root = std::env::temp_dir().join(format!("cockpit-project-git-{}", uuid::Uuid::new_v4()));
        let main = root.join("main");
        let linked = root.join("linked");
        let clone = root.join("clone");
        std::fs::create_dir_all(&main).unwrap();
        git(&main, &["init", "-b", "main"]);
        git(&main, &["config", "user.name", "Fixture"]);
        git(&main, &["config", "user.email", "fixture@example.invalid"]);
        git(&main, &["commit", "--allow-empty", "-m", "fixture"]);
        git(&main, &["worktree", "add", "-b", "worker", linked.to_str().unwrap()]);
        git(&root, &["clone", main.to_str().unwrap(), clone.to_str().unwrap()]);
        let (a, ac) = resolve_directory(&main, Some("alias-a".into()), Some("space-a".into())).await.unwrap();
        let (b, bc) = resolve_directory(&linked, Some("alias-b".into()), Some("space-b".into())).await.unwrap();
        let (c, _) = resolve_directory(&clone, None, None).await.unwrap();
        assert_eq!(a.key, b.key);
        assert_ne!(ac.key, bc.key);
        assert_ne!(a.key, c.key);
        assert_ne!(ac.git.as_ref().unwrap().worktree_git_dir, bc.git.as_ref().unwrap().worktree_git_dir);
        std::fs::remove_dir_all(root).unwrap();
    }
}
