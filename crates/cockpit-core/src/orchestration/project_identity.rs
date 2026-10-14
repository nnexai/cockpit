//! Fresh local filesystem observations. None of these identities grants cleanup authority.
use std::{path::Path, os::unix::fs::MetadataExt};
use cockpit_protocol::orchestration::*;
use sha2::{Digest, Sha256};
use crate::{InspectionError, process_identity, space_git};
use super::{OrchestrationService, ResolvedMutationInput, claims::{AttachmentReference, RootMainRecoveryProof, WriterExitProof}};

fn unavailable(text: &str) -> InspectionError { InspectionError::new("project_identity_unavailable", text) }
fn path_text(path: &Path) -> Result<String, InspectionError> {
    path.to_str().map(str::to_owned).ok_or_else(|| unavailable("Physical directory is not UTF-8"))
}
fn key(domain: &[u8], pieces: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    for piece in pieces {
        digest.update((piece.len() as u64).to_be_bytes()); digest.update(piece.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// Stable local checkout key; directory device/inode is revalidated separately.
pub(crate) fn checkout_key(checkout_path: &str, worktree_git_dir: &str) -> String {
    key(b"cockpit:checkout:v1\0", &[checkout_path, worktree_git_dir])
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
    if directory(Path::new(&checkout.path))? != (checkout.path.clone(), checkout.directory_identity.clone()) {
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
pub fn resolve_owned_directory(path: &Path) -> Result<(ProjectIdentity, CheckoutIdentity), InspectionError> {
    let (root, identity) = directory(path)?;
    if Path::new(&root).join(".git").exists() { return Err(unavailable("Owned supervisor directory unexpectedly contains Git metadata")); }
    let project = ProjectIdentity { key: key(b"cockpit:project:directory:v1\0", &[&root]),
        source: ProjectSource::Directory { root: root.clone() }, repository_id: None, source_workspace_id: None };
    let checkout = CheckoutIdentity { key: checkout_key(&root, ""),
        path: root, directory_identity: identity, git: None };
    Ok((project, checkout))
}

/// Exact owner-created attempt checkout; its original project identity remains
/// separate. This constructor never discovers or creates a replacement project.
pub fn owned_attempt_checkout(path: &Path) -> Result<CheckoutIdentity, InspectionError> {
    use cap_std::fs::MetadataExt as _;
    let directory = crate::project_store::open_dir_nofollow_absolute(path)
        .map_err(|_| unavailable("Owned attempt directory cannot be opened without following links"))?;
    let metadata = directory.dir_metadata().map_err(|_| unavailable("Owned attempt directory cannot be inspected"))?;
    if !metadata.is_dir() || metadata.mode() & 0o7777 != 0o700
        || metadata.uid() != nix::unistd::Uid::effective().as_raw() {
        return Err(unavailable("Owned attempt directory must be owner-private with mode 0700"));
    }
    let root = path_text(path)?;
    Ok(CheckoutIdentity { key: checkout_key(&root, ""), path: root,
        directory_identity: DirectoryIdentity { device: metadata.dev().to_string(), inode: metadata.ino().to_string() }, git: None })
}

impl OrchestrationService {
    /// Revalidate a physical checkout without confusing a private attempt cwd
    /// with its unchanged source project. Directory projects never probe Git.
    pub async fn resolve_run_checkout(&self, run: &Run) -> Result<(ProjectIdentity, CheckoutIdentity), InspectionError> {
        let saved = run.checkout.as_ref().ok_or_else(|| unavailable("Run has no recorded checkout"))?;
        let project = run.project.as_ref().ok_or_else(|| unavailable("Run has no recorded source project"))?;
        if let Some(DispatchTarget::OwnedAttemptDirectory { source_root, .. }) = &run.target {
            let ProjectSource::Directory { root } = &project.source else {
                return Err(unavailable("Owned attempt directory requires its original Directory project"));
            };
            let expected = self.base().join("attempts").join(&run.run_id);
            if Path::new(&saved.path) != expected || source_root != root || directory(Path::new(root))?.0 != *root {
                return Err(unavailable("Owned attempt checkout differs from its exact private run path or original source"));
            }
            return Ok((project.clone(), owned_attempt_checkout(&expected)?));
        }
        if let ProjectSource::Directory { root } = &project.source {
            let (path, identity) = directory(Path::new(&saved.path))?;
            if directory(Path::new(root))?.0 != *root {
                return Err(unavailable("Original Directory project root changed"));
            }
            return Ok((project.clone(), CheckoutIdentity { key: checkout_key(&path, ""), path,
                directory_identity: identity, git: None }));
        }
        resolve_directory(Path::new(&saved.path), project.repository_id.clone(), project.source_workspace_id.clone()).await
    }
}

/// Git inspection is bounded and disables hooks, prompts, fsmonitor and ambient
/// Git path overrides. Only Git's explicit not-a-repository reply admits a plain
/// directory; unavailable Git never silently changes a project's identity.
pub async fn resolve_directory(path: &Path, repository_id: Option<String>, source_workspace_id: Option<String>) -> Result<(ProjectIdentity, CheckoutIdentity), InspectionError> {
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
        ProjectSource::Git { common_dir } => key(b"cockpit:project:git:v1\0", &[common_dir]),
        ProjectSource::Directory { root } => key(b"cockpit:project:directory:v1\0", &[root]),
    };
    let checkout_key = checkout_key(&checkout_path, git.as_ref().map_or("", |g| g.worktree_git_dir.as_str()));
    Ok((ProjectIdentity { key: project_key, source: project_source, repository_id, source_workspace_id },
        CheckoutIdentity { key: checkout_key, path: checkout_path, directory_identity, git }))
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
            || pane.agent_kind.as_deref().is_some_and(|kind| kind != run.launch_config.harness_kind)
            || !super::optional_available(&location.native_session_id, &pane.native_session_id) {
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

fn root_unproven() -> InspectionError {
    InspectionError::new("root_main_exit_unproven", "The exact saved root main must be positively absent and its owned terminal freshly safe")
}

fn root_container_unproven() -> InspectionError {
    InspectionError::new("consumer_activity_unproven", "The root terminal is pending, busy, foreign, ambiguous or lacks exact ownership evidence; no liaison was launched")
}

pub(crate) fn same_location(left: Option<&RunLocation>, right: Option<&RunLocation>) -> bool {
    match (left, right) {
        (Some(a), Some(b)) => a.endpoint_identity == b.endpoint_identity && a.boot_id == b.boot_id
            && a.peer_process == b.peer_process
            && a.session_id == b.session_id && a.workspace_id == b.workspace_id
            && a.tab_id == b.tab_id && a.pane_id == b.pane_id && a.terminal_id == b.terminal_id
            && a.native_session_id == b.native_session_id && a.launch_tag == b.launch_tag,
        (None, None) => true,
        _ => false,
    }
}

pub(crate) fn same_root_attachment(a: &ExecutionAttachment, b: &ExecutionAttachment) -> bool {
    a.attachment_id == b.attachment_id && a.epoch == b.epoch && a.state == b.state
        && a.process == b.process && a.shell == b.shell && a.machine_id == b.machine_id
        && a.conversation == b.conversation && a.launch_mode == b.launch_mode
        && a.launch_attempt == b.launch_attempt && same_location(a.location.as_ref(), b.location.as_ref())
}

fn same_dispatch(a: Option<&DispatchState>, b: Option<&DispatchState>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.launch_attempt == b.launch_attempt && a.launch_tag == b.launch_tag
            && a.step == b.step && a.endpoint_identity == b.endpoint_identity && a.agent_started == b.agent_started
            && a.updated_at == b.updated_at,
        (None, None) => true,
        _ => false,
    }
}

fn root_recovery_target(state: &super::store::OrchestrationState, run: &Run) -> Result<(), InspectionError> {
    if !matches!(run.kind, RunKind::Supervisor | RunKind::Adopted) || run.run_id != run.root_id
        || run.parent_run_id.is_some() || run.checkout_claim_id.is_some() || run.stage == RunStage::Closed
        || run.stage == RunStage::Reported || run.result.is_some()
        || (run.kind == RunKind::Supervisor && run.dispatch.is_none())
        || state.checkout_claims.iter().any(|claim| claim.state != super::claims::ClaimState::Released
            && claim.holder.as_ref().is_some_and(|holder| holder.run_id == run.run_id)) {
        return Err(root_unproven());
    }
    let attachment = run.attachment.as_ref().ok_or_else(root_unproven)?;
    if matches!(attachment.state, AttachmentState::Quiescing | AttachmentState::Revoked)
        || attachment.location.as_ref().is_none_or(|location| location.session_id != run.session_id)
        || run.dispatch.as_ref().is_some_and(|dispatch|
            matches!(dispatch.step, DispatchStep::SetupRunning | DispatchStep::SetupUnknown
                | DispatchStep::LaunchIntent | DispatchStep::LaunchPending | DispatchStep::LaunchUnknown)
            || dispatch.launch_attempt != attachment.launch_attempt
            || attachment.location.as_ref().is_none_or(|location|
                dispatch.endpoint_identity.as_deref() != Some(location.endpoint_identity.as_str())
                    || dispatch.launch_tag.as_deref() != Some(location.launch_tag.as_str()))) {
        return Err(root_unproven());
    }
    Ok(())
}

fn root_main_running(attachment: &ExecutionAttachment) -> Result<bool, InspectionError> {
    let process = attachment.process.as_ref().ok_or_else(root_unproven)?;
    if process.pid == 0 || process.start_ticks == 0
        || process.kernel_boot_id.as_deref().is_none_or(str::is_empty)
        || attachment.machine_id.as_deref().is_none_or(str::is_empty) {
        return Err(root_unproven());
    }
    process_identity::same_machine_process_running(process, attachment.machine_id.as_deref())
        .map_err(|_| root_unproven())
}

pub(crate) fn revalidate_root_recovery(state: &super::store::OrchestrationState, run: &Run, proof: &RootMainRecoveryProof) -> Result<(), InspectionError> {
    root_recovery_target(state, run)?;
    let attachment = run.attachment.as_ref().ok_or_else(root_unproven)?;
    if run.session_id != proof.session_id || run.run_id != proof.run_id
        || !same_root_attachment(attachment, &proof.attachment)
        || !same_dispatch(run.dispatch.as_ref(), proof.dispatch.as_ref()) || root_main_running(attachment)? {
        return Err(root_unproven());
    }
    if run.kind == RunKind::Adopted && run.dispatch.is_none() {
        let checkout = run.checkout.as_ref().ok_or_else(root_unproven)?;
        if run.target.is_some() || run.setup.is_some() || proof.retained_checkout.as_ref() != Some(checkout) {
            return Err(root_unproven());
        }
    }
    if let Some(checkout) = &proof.retained_checkout {
        if run.checkout.as_ref() != Some(checkout) { return Err(root_unproven()); }
        if run.dispatch.is_some() { restored_root_receipt_checkout(run)?; }
        revalidate_checkout(checkout)?;
    }
    if let Some(shell) = &proof.idle_shell {
        if process_identity::executable_identity(shell.process.pid as i32, shell.process.start_ticks).ok().as_ref() != Some(shell)
            || !matches!(process_identity::incarnation_foreground(shell.process.pid as i32, shell.process.start_ticks), Ok(true)) {
            return Err(root_container_unproven());
        }
    }
    Ok(())
}

pub(crate) fn revalidate_root_observation(state: &super::store::OrchestrationState, run: &Run, expected: &ExecutionAttachment) -> Result<(), InspectionError> {
    root_recovery_target(state, run)?;
    let attachment = run.attachment.as_ref().ok_or_else(root_unproven)?;
    let process = attachment.process.as_ref().ok_or_else(root_unproven)?;
    if !same_root_attachment(attachment, expected) || !root_main_running(attachment)?
        || !matches!(process_identity::incarnation_foreground(process.pid as i32, process.start_ticks), Ok(true)) {
        return Err(root_unproven());
    }
    Ok(())
}

/// Restored PTYs may get new terminal IDs. Equivalent ownership is only a
/// settled managed launch receipt plus its exact topology and physical checkout,
/// never a label/name/conversation alias or authority to touch the restored pane.
fn restored_root_receipt_checkout(run: &Run) -> Result<&CheckoutIdentity, InspectionError> {
    let attachment = run.attachment.as_ref().ok_or_else(root_container_unproven)?;
    let location = attachment.location.as_ref().ok_or_else(root_container_unproven)?;
    let dispatch = run.dispatch.as_ref().ok_or_else(root_container_unproven)?;
    let setup = run.setup.as_ref().ok_or_else(root_container_unproven)?;
    let checkout = run.checkout.as_ref().ok_or_else(root_container_unproven)?;
    if location.launch_tag.is_empty() || dispatch.step != DispatchStep::Launched || !dispatch.agent_started
        || dispatch.launch_tag.as_deref() != Some(location.launch_tag.as_str())
        || dispatch.launch_attempt != attachment.launch_attempt
        || setup.workspace_id.as_ref() != Some(&location.workspace_id) || setup.checkout_path != checkout.path
        || attachment.machine_id.is_none() || attachment.machine_id != process_identity::machine_id() {
        return Err(root_container_unproven());
    }
    Ok(checkout)
}

fn fresh_root_checkout<'a>(runtime: &super::herdr::RuntimeView, run: &'a Run) -> Result<&'a CheckoutIdentity, InspectionError> {
    let checkout = run.checkout.as_ref().ok_or_else(root_container_unproven)?;
    let location = run.attachment.as_ref().and_then(|attachment| attachment.location.as_ref()).ok_or_else(root_container_unproven)?;
    let mut workspaces = runtime.workspaces.iter().filter(|workspace| workspace.workspace_id == location.workspace_id);
    let workspace = workspaces.next().ok_or_else(root_container_unproven)?;
    if workspaces.next().is_some() { return Err(root_container_unproven()); }
    let (path, identity) = directory(Path::new(&workspace.cwd)).map_err(|_| root_container_unproven())?;
    if path != checkout.path || identity != checkout.directory_identity {
        return Err(root_container_unproven());
    }
    revalidate_checkout(checkout).map_err(|_| root_container_unproven())?;
    Ok(checkout)
}

fn owned_root_pane<'a>(runtime: &'a super::herdr::RuntimeView, run: &Run, allow_restored_shell: bool) -> Result<Option<&'a super::herdr::RuntimePane>, InspectionError> {
    let attachment = run.attachment.as_ref().ok_or_else(root_container_unproven)?;
    let location = attachment.location.as_ref().ok_or_else(root_container_unproven)?;
    let same_endpoint = runtime.endpoint_identity == location.endpoint_identity
        && super::optional_available(&location.boot_id, &runtime.boot_id);
    if !same_endpoint && location.terminal_id.is_none() {
        if !allow_restored_shell { return Err(root_container_unproven()); }
        restored_root_receipt_checkout(run)?;
        fresh_root_checkout(runtime, run)?;
    }
    let exact_layout = |pane: &super::herdr::RuntimePane| pane.workspace_id == location.workspace_id
        && pane.tab_id == location.tab_id && pane.pane_id == location.pane_id;
    let mut candidates = runtime.panes.iter().filter(|pane|
        location.terminal_id.as_ref().is_some_and(|id| pane.terminal_id.as_ref() == Some(id))
        || exact_layout(pane)
        || location.native_session_id.as_ref().is_some_and(|id| pane.native_session_id.as_ref() == Some(id))
        || (!location.launch_tag.is_empty() && (pane.tab_label == location.launch_tag
            || pane.agent_name.as_deref() == Some(location.launch_tag.as_str()))));
    let Some(pane) = candidates.next() else { return Ok(None) };
    if candidates.next().is_some() || pane.workspace_id != location.workspace_id || pane.launch_pending {
        return Err(root_container_unproven());
    }
    let same_terminal = location.terminal_id.as_ref().is_some_and(|id| pane.terminal_id.as_ref() == Some(id));
    if !same_terminal && !(same_endpoint && exact_layout(pane) && location.terminal_id.is_none()) {
        if same_endpoint || !allow_restored_shell || !exact_layout(pane) || pane.tab_label != location.launch_tag
            || pane.terminal_id.as_deref().is_none_or(str::is_empty) {
            return Err(root_container_unproven());
        }
        restored_root_receipt_checkout(run)?;
        fresh_root_checkout(runtime, run)?;
    }
    Ok(Some(pane))
}

impl OrchestrationService {
    /// One observation path for explicit Resume and automatic root recovery.
    /// It never closes a pane, signals a process or inspects worker consumers.
    pub async fn observe_root_resume(&self, herdr: &dyn super::herdr::OrchestrationHerdr, expected: &Run) -> ResolvedMutationInput {
        let mut input = ResolvedMutationInput { project: expected.project.clone(), checkout: expected.checkout.clone(), ..Default::default() };
        match self.observe_root_main(herdr, expected).await {
            Ok((attachment, proof)) => {
                input.root_attachment_expected = if attachment.is_some() { expected.attachment.clone() } else { None };
                input.attachment_observation = attachment;
                input.root_main_recovery = proof;
            }
            Err(error) => input.observation_cause = Some(ResumeCause {
                code: error.code, message: error.message, class: ResumeCauseClass::Recover,
            }),
        }
        input
    }

    async fn observe_root_main(&self, herdr: &dyn super::herdr::OrchestrationHerdr, expected: &Run) -> Result<(Option<ExecutionAttachment>, Option<RootMainRecoveryProof>), InspectionError> {
        {
            let locked = self.store.lock()?;
            let state = locked.read()?;
            let run = state.runs.iter().find(|run| run.run_id == expected.run_id).ok_or_else(root_unproven)?;
            root_recovery_target(&state, run)?;
            if !same_root_attachment(run.attachment.as_ref().ok_or_else(root_unproven)?, expected.attachment.as_ref().ok_or_else(root_unproven)?)
                || !same_dispatch(run.dispatch.as_ref(), expected.dispatch.as_ref()) {
                return Err(root_unproven());
            }
        }
        let attachment = expected.attachment.as_ref().ok_or_else(root_unproven)?;
        let alive = root_main_running(attachment)?;
        let runtime = herdr.runtime(&expected.session_id).await.map_err(|_| InspectionError::new(
            "resume_observation_unavailable", "Fresh owned root-container evidence is unavailable; the saved incarnation is preserved"))?;
        let pane = owned_root_pane(&runtime, expected, !alive)?;
        let mut idle_shell = None;
        let mut observed = None;
        if let Some(pane) = pane {
            let info = herdr.pane_process_info(&expected.session_id, &runtime.endpoint_identity, &pane.pane_id)
                .await.map_err(|_| root_container_unproven())?;
            if info.pane_id != pane.pane_id { return Err(root_container_unproven()); }
            if alive {
                let process = attachment.process.as_ref().ok_or_else(root_unproven)?;
                if pane.agent_kind.as_deref().is_some_and(|kind| kind != expected.launch_config.harness_kind)
                    || attachment.location.as_ref().is_some_and(|location| !super::optional_available(&location.native_session_id, &pane.native_session_id))
                    || !info.processes.iter().any(|(pid, _)| *pid == process.pid)
                    || !matches!(process_identity::incarnation_foreground(process.pid as i32, process.start_ticks), Ok(true)) {
                    return Err(root_container_unproven());
                }
                let mut fresh = attachment.clone();
                let location = fresh.location.as_mut().ok_or_else(root_container_unproven)?;
                location.endpoint_identity = runtime.endpoint_identity.clone();
                location.boot_id = runtime.boot_id.clone();
                location.peer_process = runtime.peer_process.clone();
                location.tab_id = pane.tab_id.clone();
                location.pane_id = pane.pane_id.clone();
                location.terminal_id = pane.terminal_id.clone();
                location.native_session_id = pane.native_session_id.clone();
                fresh.state = AttachmentState::Connected;
                observed = Some(fresh);
            } else {
                // Cached native identity is not current occupancy. A genuine
                // shell-only foreground fingerprint, not its label, proves idle.
                if pane.agent_kind.as_deref().is_some_and(|kind| kind != expected.launch_config.harness_kind)
                    || attachment.location.as_ref().is_some_and(|location| !super::optional_available(&location.native_session_id, &pane.native_session_id)) {
                    return Err(root_container_unproven());
                }
                let shell = info.shell_identity.as_ref().ok_or_else(root_container_unproven)?;
                if info.shell_pid != Some(shell.process.pid) || info.foreground_pgid != info.shell_pid
                    || info.processes.is_empty() || info.processes.iter().any(|(pid, _)| Some(*pid) != info.shell_pid)
                    || process_identity::executable_identity(shell.process.pid as i32, shell.process.start_ticks).ok().as_ref() != Some(shell)
                    || attachment.process.as_ref() == Some(&shell.process) {
                    return Err(root_container_unproven());
                }
                idle_shell = Some(shell.clone());
            }
        } else if alive {
            return Err(root_container_unproven());
        }
        // Re-read daemon ownership after process inspection. Ambiguity or a
        // new/restored occupant must not turn an old empty observation into exit.
        let fresh_runtime = herdr.runtime(&expected.session_id).await.map_err(|_| root_container_unproven())?;
        let fresh_pane = owned_root_pane(&fresh_runtime, expected, !alive)?;
        if fresh_runtime.endpoint_identity != runtime.endpoint_identity || fresh_runtime.boot_id != runtime.boot_id
            || fresh_runtime.peer_process != runtime.peer_process
            || match (pane, fresh_pane) {
                (Some(a), Some(b)) => a.pane_id != b.pane_id || a.tab_id != b.tab_id || a.terminal_id != b.terminal_id
                    || a.native_session_id != b.native_session_id || a.agent_kind != b.agent_kind,
                (None, None) => false,
                _ => true,
            } || root_main_running(attachment)? != alive {
            return Err(root_container_unproven());
        }
        if let Some(observed) = observed { return Ok((Some(observed), None)); }
        let location = attachment.location.as_ref().ok_or_else(root_unproven)?;
        let restored_receipt = (fresh_runtime.endpoint_identity != location.endpoint_identity
            || !super::optional_available(&location.boot_id, &fresh_runtime.boot_id))
            && (location.terminal_id.is_none() || fresh_pane.is_some_and(|pane| pane.terminal_id != location.terminal_id));
        let retained_checkout = if expected.kind == RunKind::Adopted && expected.dispatch.is_none() {
            Some(fresh_root_checkout(&fresh_runtime, expected)?.clone())
        } else if restored_receipt {
            restored_root_receipt_checkout(expected)?;
            Some(fresh_root_checkout(&fresh_runtime, expected)?.clone())
        } else { None };
        let proof = RootMainRecoveryProof { run_id: expected.run_id.clone(), session_id: expected.session_id.clone(),
            attachment: attachment.clone(), dispatch: expected.dispatch.clone(), retained_checkout, idle_shell };
        let locked = self.store.lock()?;
        let state = locked.read()?;
        let current = state.runs.iter().find(|run| run.run_id == expected.run_id).ok_or_else(root_unproven)?;
        revalidate_root_recovery(&state, current, &proof)?;
        Ok((None, Some(proof)))
    }
}

impl OrchestrationService {
    /// Worker-only all-consumer proof; never used to recover a root liaison.
    /// A stale projection, heartbeat, endpoint failure or closed status is never
    /// substituted for OS incarnation evidence.
    pub fn observe_writer_exit(&self, expected: &Run, checkout: &CheckoutIdentity,
        scope: Option<&crate::process_scope::ScopeObservation>, herdr_cwd_clear: bool) -> Result<WriterExitProof, InspectionError> {
        if expected.kind != RunKind::Worker { return Err(exit_unproven()); }
        let locked = self.store.lock()?;
        let state = locked.read()?;
        revalidate_checkout(checkout)?;
        let run = state.runs.iter().find(|r| r.run_id == expected.run_id).ok_or_else(exit_unproven)?;
        let old = run.attachment.as_ref().ok_or_else(exit_unproven)?;
        if run.kind != RunKind::Worker
            || expected.attachment.as_ref().is_none_or(|a| a.attachment_id != old.attachment_id || a.epoch != old.epoch)
            || run.checkout.as_ref() != Some(checkout) { return Err(exit_unproven()); }
        let machine = process_identity::machine_id().ok_or_else(exit_unproven)?;
        let boot = process_identity::kernel_boot_id().ok_or_else(exit_unproven)?;
        if old.machine_id.as_deref() != Some(machine.as_str()) { return Err(exit_unproven()); }
        let previous = old.process.as_ref().and_then(|p| p.kernel_boot_id.as_ref())
            .or_else(|| old.shell.as_ref().and_then(|s| s.process.kernel_boot_id.as_ref())).ok_or_else(exit_unproven)?;
        let same_boot = previous == &boot;
        let basis = if same_boot {
            let observation = scope.ok_or_else(consumer_unproven)?;
            let saved = old.process_scope.as_ref().ok_or_else(consumer_unproven)?;
            let observed_at = time::OffsetDateTime::parse(&observation.observed_at, &time::format_description::well_known::Rfc3339)
                .map_err(|_| consumer_unproven())?;
            let age = time::OffsetDateTime::now_utc() - observed_at;
            if &observation.scope != saved || saved.machine_id != machine || saved.kernel_boot_id != boot
                || !matches!(observation.state, crate::process_scope::ScopeState::Drained) || !herdr_cwd_clear
                || age.is_negative() || age > time::Duration::seconds(10) {
                return Err(consumer_unproven());
            }
            super::claims::ExitBasis::ScopeDrained { unit: saved.unit.clone(), invocation_id: saved.invocation_id.clone() }
        } else { super::claims::ExitBasis::Reboot };
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
        if !same_boot && !absent(old, run.kind) { return Err(exit_unproven()); }
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
            if same_boot {
                // Separately launched descendants need their own completed
                // release proof. The parent scope cannot prove another scope empty.
                let released = child.stage == RunStage::Closed && child.checkout_claim_id.as_ref().is_some_and(|claim_id|
                    state.checkout_claims.iter().any(|claim| &claim.claim_id == claim_id && claim.state == super::claims::ClaimState::Released));
                let contained_here = child.attachment.as_ref().and_then(|attachment| attachment.process_scope.as_ref())
                    .zip(old.process_scope.as_ref()).is_some_and(|(child_scope, parent_scope)|
                        child_scope.unit == parent_scope.unit && child_scope.invocation_id == parent_scope.invocation_id);
                let never_launched = child.stage == RunStage::Closed && child.attachment.is_none()
                    && child.dispatch.as_ref().is_none_or(|dispatch| matches!(dispatch.step, DispatchStep::Planning | DispatchStep::PlanFailed));
                if !released && !contained_here && !never_launched { return Err(consumer_unproven()); }
                continue;
            }
            if child.attachment.as_ref().is_none_or(|a| !absent(a, child.kind)) { return Err(exit_unproven()); }
            let attachment = child.attachment.as_ref().ok_or_else(exit_unproven)?;
            if attachment.process.iter().chain(attachment.shell.iter().map(|shell| &shell.process))
                .any(|process| process.kernel_boot_id.as_deref() == Some(boot.as_str())) {
                return Err(consumer_unproven());
            }
        }
        let incarnation = AttachmentReference { run_id: run.run_id.clone(), attachment_id: old.attachment_id.clone(), attachment_epoch: old.epoch };
        let claim = state.checkout_claims.iter().find(|c| Some(c.claim_id.as_str()) == run.checkout_claim_id.as_deref()).ok_or_else(exit_unproven)?;
        if claim.checkout != *checkout || claim.holder.as_ref() != Some(&incarnation) { return Err(exit_unproven()); }
        Ok(WriterExitProof { incarnation, claim_revision: claim.revision, checkout: Some(checkout.clone()), machine_id: machine,
            previous_kernel_boot_id: previous.clone(), observed_kernel_boot_id: boot, all_consumers_absent: true, basis })
    }
}

/// Fresh Herdr evidence covering panes outside the attempt's process scope.
/// Only the exact successfully retired original pane is excluded.
pub async fn checkout_consumers_clear(herdr: &dyn super::herdr::OrchestrationHerdr,
    run: &Run, checkout: &CheckoutIdentity) -> Result<bool, InspectionError> {
    let runtime = herdr.runtime(&run.session_id).await?;
    let retired = run.retirement.as_ref().is_some_and(|retirement|
        matches!(retirement.state, cockpit_protocol::orchestration::RetirementState::Retired { .. }));
    let inferred_identity = if run.retirement.is_none() { super::retirement::identity_for(run) } else { None };
    let Some(server_identity) = run.retirement.as_ref().and_then(|retirement| retirement.identity.as_ref())
        .or(inferred_identity.as_ref()) else { return Ok(false); };
    if super::retirement::server_fence(server_identity, &runtime).is_err() { return Ok(false); }
    let checkout_path = std::path::Path::new(&checkout.path);
    for pane in &runtime.panes {
        if retired && server_identity.workspace_id == pane.workspace_id
            && server_identity.tab_id == pane.tab_id && server_identity.pane_id == pane.pane_id
            && pane.terminal_id.as_deref() == Some(server_identity.terminal_id.as_str()) { continue; }
        let Some(cwd) = pane.cwd.as_deref() else { return Ok(false); };
        let path = std::path::Path::new(cwd);
        if !path.is_absolute() || path.components().any(|component| matches!(component, std::path::Component::ParentDir))
            || path.starts_with(checkout_path) { return Ok(false); }
        // Resolve aliases without accepting disappeared directories as proof.
        let Ok(canonical) = path.canonicalize() else { return Ok(false); };
        if canonical.starts_with(checkout_path) { return Ok(false); }
    }
    Ok(true)
}

fn exit_unproven() -> InspectionError {
    InspectionError::new("writer_exit_unproven", "Recorded writer, launch shell and every known consumer must be proven absent on the same local machine")
}


fn consumer_unproven() -> InspectionError {
    InspectionError::new("consumer_activity_unproven",
        "All checkout consumers require a matching drained owned process scope and clear live-pane evidence, or same-machine reboot proof.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn plain_nested_directory_keeps_exact_project_but_distinct_checkout() {
        let root = std::env::temp_dir().join(format!("cockpit-project-identity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        let (outer, a) = resolve_directory(&root, None, None).await.unwrap();
        let (inner, b) = resolve_directory(&root.join("nested"), None, None).await.unwrap();
        assert_ne!(outer.key, inner.key); assert_ne!(a.key, b.key);
        assert!(super::super::claims::checkouts_conflict(&a, &b));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn local_keys_ignore_aliases_but_revalidation_rejects_replaced_directory() {
        let root = std::env::temp_dir().join(format!("cockpit-project-local-{}", uuid::Uuid::new_v4()));
        let checkout_path = root.join("checkout");
        let alias = root.join("alias");
        std::fs::create_dir_all(&checkout_path).unwrap();
        std::os::unix::fs::symlink(&checkout_path, &alias).unwrap();
        let (project, checkout) = resolve_directory(&checkout_path, Some("first".into()), Some("space-a".into())).await.unwrap();
        let (aliased_project, aliased_checkout) = resolve_directory(&alias, Some("second".into()), Some("space-b".into())).await.unwrap();
        assert_eq!(project.key, aliased_project.key);
        assert_eq!(checkout, aliased_checkout);
        let (owned_project, owned_checkout) = resolve_owned_directory(&checkout_path).unwrap();
        assert_eq!(project.key, owned_project.key);
        assert_eq!(checkout, owned_checkout);
        revalidate_checkout(&checkout).unwrap();
        std::fs::rename(&checkout_path, root.join("old-checkout")).unwrap();
        std::fs::create_dir(&checkout_path).unwrap();
        let (replacement_project, replacement) = resolve_directory(&checkout_path, None, None).await.unwrap();
        assert_eq!(project.key, replacement_project.key);
        assert_eq!(checkout.key, replacement.key);
        assert_ne!(checkout.directory_identity, replacement.directory_identity);
        assert_eq!(revalidate_checkout(&checkout).unwrap_err().code, "checkout_identity_changed");
        revalidate_checkout(&replacement).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn relative_directory_never_resolves_from_cli_cwd() {
        assert!(resolve_directory(Path::new("."), None, None).await.is_err());
    }
    #[tokio::test]
    async fn linked_worktrees_share_project_not_checkout_and_clones_stay_distinct() {
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
