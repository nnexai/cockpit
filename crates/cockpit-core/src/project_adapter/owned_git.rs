//! Exact local effects only. The caller supplies creation ownership, accepted
//! output integration or fresh root containment, and exhaustive consumer
//! quiescence. Unknown effects must be inspected before retry.
use std::{collections::HashSet, os::unix::fs::MetadataExt, path::Path,
    process::Output, time::{Duration, Instant}};

use cockpit_protocol::orchestration::{CheckoutIdentity, OwnedGitIdentity};
use tokio::process::Command;

use crate::{InspectionError, process, process_identity, orchestration::project_identity::{checkout_key, revalidate_checkout}};

fn refusal(message: &str) -> InspectionError {
    InspectionError::new("owned_git_conflict", message)
}

struct Budget { deadline: Instant, output_bytes: usize }
impl Budget {
    fn new(timeout_ms: u64, output_bytes: usize) -> Result<Self, InspectionError> {
        if timeout_ms == 0 || output_bytes == 0 { return Err(refusal("Git inspection requires nonzero bounds")); }
        let deadline = Instant::now().checked_add(Duration::from_millis(timeout_ms))
            .ok_or_else(|| refusal("Git timeout is out of range"))?;
        Ok(Self { deadline, output_bytes })
    }
    async fn git(&self, root: &str, args: &[&str]) -> Result<Output, InspectionError> {
        let remaining = self.deadline.checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero()).ok_or_else(|| refusal("Owned Git operation exceeded its deadline"))?;
        let mut command = Command::new("git");
        // Remove all inherited Git injection variables, including numbered
        // GIT_CONFIG_KEY/VALUE entries, alternates, namespaces and discovery overrides.
        for (key, _) in std::env::vars_os() {
            if key.as_encoded_bytes().starts_with(b"GIT_") { command.env_remove(key); }
        }
        command.current_dir(root)
            .args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false"])
            .args(args).env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null").env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "").env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_NO_LAZY_FETCH", "1").env("LC_ALL", "C");
        process::run_bounded_command(command, self.output_bytes, self.output_bytes,
            remaining, "owned git").await
    }
    async fn checked(&self, root: &str, args: &[&str]) -> Result<Output, InspectionError> {
        let output = self.git(root, args).await?;
        if !output.status.success() { return Err(refusal("Git could not establish or apply the exact owned resource state")); }
        Ok(output)
    }
    async fn line(&self, root: &str, args: &[&str]) -> Result<String, InspectionError> {
        let output = self.checked(root, args).await?;
        let text = std::str::from_utf8(&output.stdout).map_err(|_| refusal("Git evidence is not UTF-8"))?;
        let text = text.strip_suffix('\n').unwrap_or(text);
        if text.is_empty() || text.contains(['\n', '\r', '\0']) { return Err(refusal("Git evidence is not one exact line")); }
        Ok(text.to_owned())
    }
}

fn directory(path: &str) -> Result<(String, String, String), InspectionError> {
    let path = Path::new(path);
    if !path.is_absolute() { return Err(refusal("Owned directories must be absolute")); }
    let canonical = std::fs::canonicalize(path).map_err(|_| refusal("Owned directory cannot be resolved"))?;
    let metadata = std::fs::metadata(&canonical).map_err(|_| refusal("Owned directory cannot be inspected"))?;
    if !metadata.is_dir() { return Err(refusal("Owned path is not a directory")); }
    Ok((canonical.to_str().ok_or_else(|| refusal("Owned directory is not UTF-8"))?.to_owned(),
        metadata.dev().to_string(), metadata.ino().to_string()))
}
fn exact_directory(path: &str, device: &str, inode: &str) -> Result<(), InspectionError> {
    let (actual_path, actual_device, actual_inode) = directory(path)?;
    if actual_path != path || actual_device != device || actual_inode != inode {
        return Err(refusal("Owned directory physical identity changed"));
    }
    Ok(())
}
fn filesystem_present(path: &str) -> Result<bool, InspectionError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // ENOENT through a replaced/missing ancestor is not exact absence.
            let parent = Path::new(path).parent().ok_or_else(|| refusal("Checkout has no parent"))?;
            let canonical = std::fs::canonicalize(parent).map_err(|_| refusal("Checkout parent cannot be resolved"))?;
            if canonical != parent { return Err(refusal("Checkout parent was redirected")); }
            Ok(false)
        }
        Err(_) => Err(refusal("Checkout presence cannot be established")),
    }
}
fn oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && value.bytes().any(|b| b != b'0')
}

#[derive(Debug)]
struct Binding { path: String, head: String, branch: Option<String>, detached: bool, protected: bool }
fn bindings(bytes: &[u8]) -> Result<Vec<Binding>, InspectionError> {
    if bytes.is_empty() || !bytes.ends_with(b"\0\0") { return Err(refusal("Worktree census is incomplete")); }
    let mut result = Vec::new();
    let mut current: Option<Binding> = None;
    let mut fields = HashSet::new();
    let mut paths = HashSet::new();
    let mut branches = HashSet::new();
    for field in bytes.split(|b| *b == 0) {
        if field.is_empty() {
            if let Some(entry) = current.take() {
                if !oid(&entry.head) || entry.branch.is_some() == entry.detached {
                    return Err(refusal("Worktree census has ambiguous or unsupported bindings"));
                }
                result.push(entry); fields.clear();
            }
            continue;
        }
        let field = std::str::from_utf8(field).map_err(|_| refusal("Worktree census is not UTF-8"))?;
        let (kind, value) = field.split_once(' ').unwrap_or((field, ""));
        if !fields.insert(kind) { return Err(refusal("Worktree census contains duplicate fields")); }
        if kind == "worktree" {
            if current.is_some() || !Path::new(value).is_absolute() || !paths.insert(value) {
                return Err(refusal("Worktree census has malformed or duplicate checkout bindings"));
            }
            current = Some(Binding { path: value.into(), head: String::new(), branch: None, detached: false, protected: false });
            continue;
        }
        let entry = current.as_mut().ok_or_else(|| refusal("Worktree census lacks a checkout"))?;
        match kind {
            "HEAD" => entry.head = value.into(),
            "branch" if value.starts_with("refs/heads/") => {
                if !branches.insert(value) { return Err(refusal("Worktree census contains duplicate branch bindings")); }
                entry.branch = Some(value.into());
            }
            "detached" if value.is_empty() => entry.detached = true,
            "locked" | "prunable" => entry.protected = true,
            _ => return Err(refusal("Worktree census contains unsupported evidence")),
        }
    }
    if result.is_empty() { return Err(refusal("Retained primary worktree is missing")); }
    Ok(result)
}
async fn census(identity: &OwnedGitIdentity, budget: &Budget) -> Result<Vec<Binding>, InspectionError> {
    let output = budget.checked(&identity.repository_root, &["worktree", "list", "--porcelain", "-z"]).await?;
    let entries = bindings(&output.stdout)?;
    if entries[0].path != identity.repository_root { return Err(refusal("Retained repository is not the primary worktree")); }
    Ok(entries)
}
async fn git_paths(root: &str, budget: &Budget) -> Result<(String, String), InspectionError> {
    if budget.line(root, &["rev-parse", "--show-toplevel"]).await? != root {
        return Err(refusal("Git toplevel differs from the exact checkout"));
    }
    let common = budget.line(root, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await?;
    let git_dir = budget.line(root, &["rev-parse", "--absolute-git-dir"]).await?;
    if directory(&common)?.0 != common || directory(&git_dir)?.0 != git_dir {
        return Err(refusal("Git administrative directories were redirected"));
    }
    let dot_git = std::fs::symlink_metadata(Path::new(root).join(".git"))
        .map_err(|_| refusal("Git administrative binding is unavailable"))?;
    if dot_git.file_type().is_symlink() { return Err(refusal("Git administrative binding is a symlink")); }
    Ok((common, git_dir))
}
async fn retained(identity: &OwnedGitIdentity, expected: &str, budget: &Budget) -> Result<(), InspectionError> {
    if identity.creation_operation_id.is_empty() || identity.checkout_key.is_empty() || !oid(&identity.creation_tip) || !oid(expected)
        || process_identity::machine_id().as_ref() != Some(&identity.machine_id) || identity.repository_key != identity.common_dir {
        return Err(refusal("Owned Git receipt or expected tip is unavailable on this machine"));
    }
    exact_directory(&identity.repository_root, &identity.repository_device, &identity.repository_inode)?;
    exact_directory(&identity.common_dir, &identity.common_device, &identity.common_inode)?;
    if git_paths(&identity.repository_root, budget).await?.0 != identity.common_dir {
        return Err(refusal("Retained repository common directory changed"));
    }
    if let Some(branch) = &identity.branch_ref {
        if !branch.starts_with("refs/heads/") { return Err(refusal("Owned branch is not a heads ref")); }
        budget.checked(&identity.repository_root, &["check-ref-format", branch]).await?;
    }
    Ok(())
}
async fn branch_tip(identity: &OwnedGitIdentity, budget: &Budget) -> Result<Option<String>, InspectionError> {
    let branch = identity.branch_ref.as_deref().ok_or_else(|| refusal("Owned branch was not recorded"))?;
    let symbolic = budget.git(&identity.repository_root, &["symbolic-ref", "-q", branch]).await?;
    if symbolic.status.code() != Some(1) { return Err(refusal("Owned branch is symbolic or cannot be inspected")); }
    let exists = budget.git(&identity.repository_root, &["show-ref", "--verify", "--quiet", branch]).await?;
    if exists.status.code() == Some(1) { return Ok(None); }
    if !exists.status.success() { return Err(refusal("Owned branch presence cannot be established")); }
    let tip = budget.line(&identity.repository_root, &["show-ref", "--verify", "--hash", branch]).await?;
    if !oid(&tip) { return Err(refusal("Owned branch tip is malformed")); }
    Ok(Some(tip))
}
async fn clean_checkout(path: &str, budget: &Budget) -> Result<(), InspectionError> {
    let status = budget.checked(path, &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignore-submodules=none"]).await?;
    if !status.stdout.is_empty() { return Err(refusal("Owned checkout has tracked, untracked or submodule changes")); }
    let files = budget.checked(path, &["ls-files", "--stage", "-z"]).await?;
    // Non-force Git removal does not support worktrees containing submodules;
    // never flatten or recursively remove their otherwise unclassified contents.
    if files.stdout.split(|b| *b == 0).any(|entry| entry.starts_with(b"160000 ")) {
        return Err(refusal("Owned checkout contains submodules and cannot be removed non-force"));
    }
    let flags = budget.checked(path, &["ls-files", "-v", "-z"]).await?;
    if flags.stdout.split(|b| *b == 0).any(|entry| entry.first().is_some_and(|flag| flag.is_ascii_lowercase() || *flag == b'S')) {
        return Err(refusal("Index flags can conceal tracked checkout changes"));
    }
    Ok(())
}
async fn checkout_present(identity: &OwnedGitIdentity, expected: &str, entries: &[Binding], budget: &Budget,
    removal: bool) -> Result<bool, InspectionError> {
    let binding = entries.iter().find(|entry| entry.path == identity.checkout_path);
    let present = filesystem_present(&identity.checkout_path)?;
    if !present && binding.is_none() { return Ok(false); }
    let binding = binding.filter(|_| present).ok_or_else(|| refusal("Checkout filesystem and Git binding disagree"))?;
    if binding.protected || binding.head != expected || (removal && binding.branch != identity.branch_ref) {
        return Err(refusal("Owned checkout binding, protection or accepted HEAD changed"));
    }
    if removal && entries[0].path == binding.path { return Err(refusal("Primary worktree removal is prohibited")); }
    exact_directory(&identity.checkout_path, &identity.checkout_device, &identity.checkout_inode)?;
    let (common, git_dir) = git_paths(&identity.checkout_path, budget).await?;
    if common != identity.common_dir || checkout_key(&identity.checkout_path, &git_dir) != identity.checkout_key {
        return Err(refusal("Owned checkout Git identity changed"));
    }
    if budget.line(&identity.checkout_path, &["rev-parse", "--verify", "HEAD"]).await? != expected {
        return Err(refusal("Owned checkout HEAD differs from the accepted tip"));
    }
    let head_ref = budget.git(&identity.checkout_path, &["symbolic-ref", "-q", "HEAD"]).await?;
    match &binding.branch {
        Some(branch) => {
            if !head_ref.status.success() || head_ref.stdout != format!("{branch}\n").as_bytes() {
                return Err(refusal("Checkout HEAD binding changed"));
            }
        }
        None if head_ref.status.code() == Some(1) => {},
        None => return Err(refusal("Detached checkout binding changed")),
    }
    if removal {
        if identity.branch_ref.is_some()
            && branch_tip(identity, budget).await?.as_deref() != Some(expected) { return Err(refusal("Owned direct branch tip changed")); }
        clean_checkout(&identity.checkout_path, budget).await?;
    }
    exact_directory(&identity.checkout_path, &identity.checkout_device, &identity.checkout_inode)?;
    Ok(true)
}

/// Call only immediately after supported owned creation, never to register old resources.
pub async fn capture_owned_git_identity(operation_id: &str, repository_root: &str, checkout_path: &str,
    branch_ref: Option<&str>, timeout_ms: u64, output_bytes: usize) -> Result<OwnedGitIdentity, InspectionError> {
    let budget = Budget::new(timeout_ms, output_bytes)?;
    let machine = process_identity::machine_id().ok_or_else(|| refusal("Stable local machine identity is unavailable"))?;
    let (repository_root, repository_device, repository_inode) = directory(repository_root)?;
    let (checkout_path, checkout_device, checkout_inode) = directory(checkout_path)?;
    let (common_dir, _) = git_paths(&repository_root, &budget).await?;
    let (_, common_device, common_inode) = directory(&common_dir)?;
    let (checkout_common, git_dir) = git_paths(&checkout_path, &budget).await?;
    if checkout_common != common_dir { return Err(refusal("Created checkout belongs to a different repository")); }
    let creation_tip = budget.line(&checkout_path, &["rev-parse", "--verify", "HEAD"]).await?;
    let identity = OwnedGitIdentity { creation_operation_id: operation_id.into(), machine_id: machine,
        repository_key: common_dir.clone(), repository_root, repository_device, repository_inode,
        common_dir, common_device, common_inode, checkout_key: checkout_key(&checkout_path, &git_dir),
        checkout_path, checkout_device, checkout_inode, branch_ref: branch_ref.map(str::to_owned), creation_tip };
    retained(&identity, &identity.creation_tip, &budget).await?;
    let entries = census(&identity, &budget).await?;
    if entries[0].path == identity.checkout_path { return Err(refusal("Owned creation must be a linked worktree, not the retained primary")); }
    let binding = entries.iter().find(|entry| entry.path == identity.checkout_path).ok_or_else(|| refusal("Created checkout has no exact Git binding"))?;
    if binding.branch != identity.branch_ref { return Err(refusal("Created branch differs from checkout binding")); }
    checkout_present(&identity, &identity.creation_tip, &entries, &budget, false).await?;
    if identity.branch_ref.is_some() && branch_tip(&identity, &budget).await?.as_deref() != Some(identity.creation_tip.as_str()) {
        return Err(refusal("Created branch is not a direct ref at the creation tip"));
    }
    retained(&identity, &identity.creation_tip, &budget).await?;
    Ok(identity)
}

async fn inspect(identity: &OwnedGitIdentity, expected: &str, worktree: bool, budget: &Budget) -> Result<bool, InspectionError> {
    retained(identity, expected, budget).await?;
    let entries = census(identity, budget).await?;
    if worktree { return checkout_present(identity, expected, &entries, budget, true).await; }
    let branch = identity.branch_ref.as_deref().ok_or_else(|| refusal("No owned branch receipt"))?;
    if entries.iter().any(|entry| entry.branch.as_deref() == Some(branch)) {
        return Err(refusal("Owned branch is bound in a current worktree"));
    }
    // A replaced checkout remains a blocker even after its old branch was detached.
    checkout_present(identity, expected, &entries, budget, false).await?;
    match branch_tip(identity, budget).await? {
        None => Ok(false),
        Some(tip) if tip == expected => Ok(true),
        Some(_) => Err(refusal("Owned branch tip differs from the accepted tip")),
    }
}

async fn containment_head(checkout: &CheckoutIdentity, budget: &Budget) -> Result<String, InspectionError> {
    revalidate_checkout(checkout)?;
    let git = checkout.git.as_ref().ok_or_else(|| refusal("Merge target has no recorded Git binding"))?;
    let (common, git_dir) = git_paths(&checkout.path, budget).await?;
    if common != git.common_dir || git_dir != git.worktree_git_dir
        || checkout_key(&checkout.path, &git_dir) != checkout.key {
        return Err(refusal("Merge target Git binding changed"));
    }
    match std::fs::symlink_metadata(Path::new(&common).join("info/grafts")) {
        Ok(_) => return Err(refusal("Git grafts cannot establish accepted output containment")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(_) => return Err(refusal("Git graft presence cannot be established")),
    }
    let head = budget.line(&checkout.path, &["rev-parse", "--verify", "HEAD^{commit}"]).await?;
    if !oid(&head) { return Err(refusal("Merge target HEAD is not an exact commit object")); }
    revalidate_checkout(checkout)?;
    Ok(head)
}

/// Deletion-specific containment proof under an unchanged physical Git binding.
pub(crate) async fn contained_head(checkout: &CheckoutIdentity, accepted_head: &str,
    expected_head: Option<&str>) -> Result<Option<String>, InspectionError> {
    if !oid(accepted_head) { return Err(refusal("Accepted output is not an exact commit object")); }
    let budget = Budget::new(2_000, 4096)?;
    let head = containment_head(checkout, &budget).await?;
    let changed = || InspectionError::new("merged_target_changed", "Merge target HEAD changed during cleanup");
    if expected_head.is_some_and(|expected| expected != head) { return Err(changed()); }
    let ancestor = budget.git(&checkout.path, &["merge-base", "--is-ancestor", accepted_head, &head]).await?;
    if ancestor.status.code() == Some(1) { return Ok(None); }
    if !ancestor.status.success() { return Err(refusal("Git could not establish accepted output containment")); }
    if containment_head(checkout, &budget).await? != head { return Err(changed()); }
    Ok(Some(head))
}

/// True means present and eligible; false means exact absence, never uncertain state.
pub async fn inspect_owned_git(identity: &OwnedGitIdentity, expected_head: &str, worktree: bool,
    timeout_ms: u64, output_bytes: usize) -> Result<bool, InspectionError> {
    inspect(identity, expected_head, worktree, &Budget::new(timeout_ms, output_bytes)?).await
}

/// Non-force removal only, from the retained repository after fresh local checks.
pub async fn remove_owned_worktree(identity: &OwnedGitIdentity, expected_head: &str,
    timeout_ms: u64, output_bytes: usize) -> Result<bool, InspectionError> {
    let budget = Budget::new(timeout_ms, output_bytes)?;
    if !inspect(identity, expected_head, true, &budget).await? { return Ok(false); }
    // Census again at the effect boundary, not a durable authorization snapshot.
    retained(identity, expected_head, &budget).await?;
    let entries = census(identity, &budget).await?;
    if !checkout_present(identity, expected_head, &entries, &budget, true).await? { return Ok(false); }
    budget.checked(&identity.repository_root, &["worktree", "remove", "--", &identity.checkout_path]).await?;
    if inspect(identity, expected_head, true, &budget).await? { return Err(refusal("Worktree removal outcome is not exact absence")); }
    Ok(true)
}

/// Delete only the direct heads ref, with Git's exact old-object compare-and-swap.
pub async fn remove_owned_branch(identity: &OwnedGitIdentity, expected_tip: &str,
    timeout_ms: u64, output_bytes: usize) -> Result<bool, InspectionError> {
    let budget = Budget::new(timeout_ms, output_bytes)?;
    if !inspect(identity, expected_tip, false, &budget).await? { return Ok(false); }
    retained(identity, expected_tip, &budget).await?;
    if !inspect(identity, expected_tip, false, &budget).await? { return Ok(false); }
    let branch = identity.branch_ref.as_deref().ok_or_else(|| refusal("No owned branch receipt"))?;
    budget.checked(&identity.repository_root, &["update-ref", "--no-deref", "-d", branch, expected_tip]).await?;
    if inspect(identity, expected_tip, false, &budget).await? { return Err(refusal("Branch removal outcome is not exact absence")); }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Fixture { root: PathBuf, main: String, linked: String, identity: OwnedGitIdentity }
    impl Fixture {
        async fn new() -> Self {
            let root = std::env::temp_dir().join(format!("cockpit-owned-git-{}", uuid::Uuid::new_v4()));
            let main = root.join("main");
            let linked = root.join("linked");
            std::fs::create_dir_all(&main).unwrap();
            let main = main.to_str().unwrap().to_owned();
            let linked = linked.to_str().unwrap().to_owned();
            let budget = Budget::new(10_000, 128 * 1024).unwrap();
            budget.checked(&main, &["init", "-b", "main"]).await.unwrap();
            std::fs::write(Path::new(&main).join("tracked"), "original\n").unwrap();
            budget.checked(&main, &["add", "--", "tracked"]).await.unwrap();
            budget.checked(&main, &["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "-m", "initial"]).await.unwrap();
            budget.checked(&main, &["worktree", "add", "-b", "owned", &linked]).await.unwrap();
            let identity = capture_owned_git_identity("created", &main, &linked, Some("refs/heads/owned"), 10_000, 128 * 1024).await.unwrap();
            let (_, checkout) = crate::orchestration::project_identity::resolve_directory(Path::new(&linked), None, None).await.unwrap();
            assert_eq!(identity.checkout_key, checkout.key);
            Self { root, main, linked, identity }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { std::fs::remove_dir_all(&self.root).unwrap(); }
    }

    #[tokio::test]
    async fn containment_requires_exact_target_head_and_real_ancestry() {
        let fixture = Fixture::new().await;
        let (_, checkout) = crate::orchestration::project_identity::resolve_directory(Path::new(&fixture.main), None, None).await.unwrap();
        let tip = &fixture.identity.creation_tip;
        assert_eq!(contained_head(&checkout, tip, None).await.unwrap(), Some(tip.clone()));
        assert_eq!(contained_head(&checkout, tip, Some(tip)).await.unwrap(), Some(tip.clone()));
        let budget = Budget::new(10_000, 128 * 1024).unwrap();
        budget.checked(&fixture.linked, &["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "-m", "unmerged output"]).await.unwrap();
        let unmerged = budget.line(&fixture.linked, &["rev-parse", "--verify", "HEAD"]).await.unwrap();
        assert!(contained_head(&checkout, &unmerged, None).await.unwrap().is_none());
        budget.checked(&fixture.main, &["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "-m", "target moved"]).await.unwrap();
        assert_eq!(contained_head(&checkout, tip, Some(tip)).await.unwrap_err().code, "merged_target_changed");
        let current = budget.line(&fixture.main, &["rev-parse", "--verify", "HEAD"]).await.unwrap();
        assert_eq!(contained_head(&checkout, tip, None).await.unwrap(), Some(current));
    }

    #[tokio::test]
    async fn containment_rejects_grafts_and_changed_git_binding() {
        let fixture = Fixture::new().await;
        let (_, mut checkout) = crate::orchestration::project_identity::resolve_directory(Path::new(&fixture.main), None, None).await.unwrap();
        let grafts = Path::new(&fixture.identity.common_dir).join("info/grafts");
        std::fs::write(&grafts, format!("{}\n", fixture.identity.creation_tip)).unwrap();
        assert!(contained_head(&checkout, &fixture.identity.creation_tip, None).await.is_err());
        std::fs::remove_file(grafts).unwrap();
        checkout.git.as_mut().unwrap().worktree_git_dir = fixture.linked.clone();
        assert!(contained_head(&checkout, &fixture.identity.creation_tip, None).await.is_err());
    }

    #[tokio::test]
    async fn exact_tip_conflict_preserves_worktree_and_branch() {
        let fixture = Fixture::new().await;
        let budget = Budget::new(10_000, 128 * 1024).unwrap();
        budget.checked(&fixture.linked, &["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "-m", "later"]).await.unwrap();
        assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.is_err());
        assert!(Path::new(&fixture.linked).is_dir());
        assert!(remove_owned_branch(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.is_err());
    }

    #[tokio::test]
    async fn branch_in_another_worktree_is_preserved() {
        let fixture = Fixture::new().await;
        assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.unwrap());
        let other = fixture.root.join("other");
        let budget = Budget::new(10_000, 128 * 1024).unwrap();
        budget.checked(&fixture.main, &["worktree", "add", other.to_str().unwrap(), "owned"]).await.unwrap();
        assert!(remove_owned_branch(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.is_err());
        assert!(other.is_dir());
        assert_eq!(branch_tip(&fixture.identity, &budget).await.unwrap(), Some(fixture.identity.creation_tip.clone()));
    }

    #[tokio::test]
    async fn moved_direct_branch_tip_is_not_deleted() {
        let fixture = Fixture::new().await;
        assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.unwrap());
        let budget = Budget::new(10_000, 128 * 1024).unwrap();
        budget.checked(&fixture.main, &["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "-m", "later"]).await.unwrap();
        let later = budget.line(&fixture.main, &["rev-parse", "--verify", "HEAD"]).await.unwrap();
        budget.checked(&fixture.main, &["update-ref", "--no-deref", "refs/heads/owned", &later, &fixture.identity.creation_tip]).await.unwrap();
        assert!(remove_owned_branch(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.is_err());
        assert_eq!(branch_tip(&fixture.identity, &budget).await.unwrap(), Some(later));
    }

    #[tokio::test]
    async fn tracked_edits_and_concealing_index_flags_are_preserved() {
        for flag in ["--assume-unchanged", "--skip-worktree"] {
            let fixture = Fixture::new().await;
            let tracked = Path::new(&fixture.linked).join("tracked");
            std::fs::write(&tracked, "preserve\n").unwrap();
            assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.is_err());
            let budget = Budget::new(10_000, 128 * 1024).unwrap();
            budget.checked(&fixture.linked, &["update-index", flag, "--", "tracked"]).await.unwrap();
            assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.is_err());
            assert_eq!(std::fs::read_to_string(tracked).unwrap(), "preserve\n");
        }
    }

    #[tokio::test]
    async fn bounded_output_failure_cannot_remove_checkout() {
        let fixture = Fixture::new().await;
        assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 1).await.is_err());
        assert!(Path::new(&fixture.linked).is_dir());
    }

    #[tokio::test]
    async fn nonignored_untracked_files_are_preserved_alongside_ignored_outputs() {
        let fixture = Fixture::new().await;
        std::fs::write(Path::new(&fixture.identity.common_dir).join("info/exclude"), "ignored\n").unwrap();
        let ignored = Path::new(&fixture.linked).join("ignored");
        let untracked = Path::new(&fixture.linked).join("untracked");
        std::fs::write(&ignored, "output").unwrap();
        std::fs::write(&untracked, "preserve").unwrap();
        assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.is_err());
        assert_eq!(std::fs::read_to_string(untracked).unwrap(), "preserve");
        assert_eq!(std::fs::read_to_string(ignored).unwrap(), "output");
    }

    #[tokio::test]
    async fn ignored_only_outputs_are_removed_with_nonforce_worktree_removal() {
        let fixture = Fixture::new().await;
        std::fs::write(Path::new(&fixture.identity.common_dir).join("info/exclude"), "ignored/\n").unwrap();
        let ignored = Path::new(&fixture.linked).join("ignored");
        std::fs::create_dir(&ignored).unwrap();
        std::fs::write(ignored.join("output"), "generated").unwrap();
        assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.unwrap());
        assert!(!Path::new(&fixture.linked).exists());
        assert!(!inspect_owned_git(&fixture.identity, &fixture.identity.creation_tip, true, 10_000, 128 * 1024).await.unwrap());
        let budget = Budget::new(10_000, 128 * 1024).unwrap();
        assert_eq!(branch_tip(&fixture.identity, &budget).await.unwrap(), Some(fixture.identity.creation_tip.clone()));
        assert_eq!(std::fs::read_to_string(Path::new(&fixture.main).join("tracked")).unwrap(), "original\n");
    }

    #[tokio::test]
    async fn clean_submodule_checkout_is_preserved_by_nonforce_removal_guard() {
        let fixture = Fixture::new().await;
        let budget = Budget::new(10_000, 128 * 1024).unwrap();
        budget.checked(&fixture.linked, &["-c", "protocol.file.allow=always", "submodule", "add", "--", &fixture.main, "submodule"]).await.unwrap();
        budget.checked(&fixture.linked, &["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-m", "add submodule"]).await.unwrap();
        let expected = budget.line(&fixture.linked, &["rev-parse", "--verify", "HEAD"]).await.unwrap();
        let status = budget.checked(&fixture.linked, &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignore-submodules=none"]).await.unwrap();
        assert!(status.stdout.is_empty());
        assert!(remove_owned_worktree(&fixture.identity, &expected, 10_000, 128 * 1024).await.is_err());
        assert_eq!(std::fs::read_to_string(Path::new(&fixture.linked).join("submodule/tracked")).unwrap(), "original\n");
        assert!(Path::new(&fixture.linked).is_dir());
    }

    #[tokio::test]
    async fn inode_replacement_is_not_owned_and_not_absence() {
        let fixture = Fixture::new().await;
        let old = fixture.root.join("original");
        std::fs::rename(&fixture.linked, &old).unwrap();
        std::fs::create_dir(&fixture.linked).unwrap();
        assert!(inspect_owned_git(&fixture.identity, &fixture.identity.creation_tip, true, 10_000, 128 * 1024).await.is_err());
        assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.is_err());
        assert!(old.is_dir());
        assert!(Path::new(&fixture.linked).is_dir());
    }

    #[tokio::test]
    async fn nonforce_effects_reconcile_exact_absence() {
        let fixture = Fixture::new().await;
        assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.unwrap());
        assert!(!inspect_owned_git(&fixture.identity, &fixture.identity.creation_tip, true, 10_000, 128 * 1024).await.unwrap());
        assert!(!remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.unwrap());
        assert!(remove_owned_branch(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.unwrap());
        assert!(!remove_owned_branch(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.unwrap());
    }

    #[tokio::test]
    async fn symbolic_branch_replacement_is_preserved() {
        let fixture = Fixture::new().await;
        assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.unwrap());
        let budget = Budget::new(10_000, 128 * 1024).unwrap();
        budget.checked(&fixture.main, &["symbolic-ref", "refs/heads/owned", "refs/heads/main"]).await.unwrap();
        assert!(remove_owned_branch(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.is_err());
        assert_eq!(budget.line(&fixture.main, &["symbolic-ref", "refs/heads/owned"]).await.unwrap(), "refs/heads/main");
    }

    #[tokio::test]
    async fn locked_and_missing_but_registered_checkouts_are_not_eligible() {
        let fixture = Fixture::new().await;
        let budget = Budget::new(10_000, 128 * 1024).unwrap();
        budget.checked(&fixture.main, &["worktree", "lock", "--reason", "preserve", &fixture.linked]).await.unwrap();
        assert!(remove_owned_worktree(&fixture.identity, &fixture.identity.creation_tip, 10_000, 128 * 1024).await.is_err());
        budget.checked(&fixture.main, &["worktree", "unlock", &fixture.linked]).await.unwrap();
        std::fs::rename(&fixture.linked, fixture.root.join("moved")).unwrap();
        assert!(inspect_owned_git(&fixture.identity, &fixture.identity.creation_tip, true, 10_000, 128 * 1024).await.is_err());
    }

    #[test]
    fn census_rejects_truncation_duplicates_and_protected_entries_are_marked() {
        let tip = "1".repeat(40);
        let record = format!("worktree /repo\0HEAD {tip}\0branch refs/heads/main\0\0");
        assert!(bindings(record.trim_end_matches('\0').as_bytes()).is_err());
        assert!(bindings(format!("{record}{record}").as_bytes()).is_err());
        let other = format!("worktree /other\0HEAD {tip}\0branch refs/heads/main\0\0");
        assert!(bindings(format!("{record}{other}").as_bytes()).is_err());
        let locked = format!("worktree /repo\0HEAD {tip}\0branch refs/heads/main\0locked reason\0\0");
        assert!(bindings(locked.as_bytes()).unwrap()[0].protected);
        assert!(Budget::new(0, 4096).is_err());
        assert!(Budget::new(1000, 0).is_err());
    }
}
