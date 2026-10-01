//! Space-scoped Git writes. The caller supplies expectations, never path authority.
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::Arc;
use std::time::Duration;

use cockpit_protocol::v1::{
    SpaceGitAction, SpaceGitActionOutcome, SpaceGitActionRequest, SpaceGitActionResponse,
    SpaceGitCheckout, SpaceGitRefusal, SpaceGitUpstream,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::{CockpitService, InspectionError, space_git};

const WRITE_TIMEOUT: Duration = Duration::from_secs(300);
const WRITE_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Default)]
pub struct SpaceGitActions {
    roots: Mutex<HashSet<PathBuf>>,
    repos: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
}

struct RootReservation {
    state: Arc<SpaceGitActions>,
    root: PathBuf,
}
impl Drop for RootReservation {
    fn drop(&mut self) {
        self.state.roots.lock().remove(&self.root);
    }
}

impl SpaceGitActions {
    fn reserve(self: &Arc<Self>, root: &Path) -> Result<RootReservation, InspectionError> {
        if !self.roots.lock().insert(root.to_owned()) {
            return Err(InspectionError::new(
                "space_git_action_in_progress",
                "A Git action is already running for this checkout.",
            ));
        }
        Ok(RootReservation {
            state: self.clone(),
            root: root.to_owned(),
        })
    }
    fn repository(&self, common: &Path) -> Arc<tokio::sync::Mutex<()>> {
        self.repos
            .lock()
            .entry(common.to_owned())
            .or_default()
            .clone()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct FileIdentity {
    canonical: PathBuf,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}
fn identity(path: &Path) -> Result<FileIdentity, InspectionError> {
    let canonical =
        std::fs::canonicalize(path).map_err(|_| not_run("Checkout identity could not be read."))?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|_| not_run("Checkout identity could not be read."))?;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Ok(FileIdentity {
        canonical,
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
    })
}

#[derive(PartialEq, Eq)]
struct Target {
    root: PathBuf,
    common: PathBuf,
    identities: [FileIdentity; 3],
    branch: String,
    upstream: String,
    metadata: space_git::BranchMetadata,
    head: String,
    // Effective, action-specific destinations. Never expose these credential-bearing values.
    remote_urls: Vec<String>,
}

fn not_run(message: &str) -> InspectionError {
    InspectionError::new("space_git_not_run", space_git::safe_detail(message))
}
fn unknown(message: &str) -> InspectionError {
    InspectionError::new("space_git_outcome_unknown", space_git::safe_detail(message))
}
fn changed() -> InspectionError {
    InspectionError::new(
        "space_git_target_changed",
        "Git was not run: this Space's checkout, branch or upstream changed. Refresh its status before trying again.",
    )
}
fn ineligible(message: &str) -> InspectionError {
    InspectionError::new("space_git_action_ineligible", message)
}
fn validate(request: &SpaceGitActionRequest) -> Result<(), InspectionError> {
    let valid = crate::validate_resource_id(&request.space_id, "space").is_ok()
        && [
            &request.expected_root,
            &request.expected_branch,
            &request.expected_upstream,
        ]
        .iter()
        .all(|value| {
            !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
        });
    if !valid {
        return Err(InspectionError::new(
            "invalid_space_git_action",
            "Git action expectations and Space ID are invalid.",
        ));
    }
    Ok(())
}

async fn resolve(
    service: &CockpitService,
    session_id: &str,
    request: &SpaceGitActionRequest,
) -> Result<Target, InspectionError> {
    let snapshot = service
        .session_snapshot(session_id)
        .await
        .map_err(|e| not_run(&e.message))?;
    let space = snapshot
        .spaces
        .iter()
        .find(|space| space.id == request.space_id)
        .ok_or_else(changed)?;
    let folder = space_git::space_folder(&snapshot, space)
        .ok_or_else(|| ineligible("This Space has no checkout folder."))?;
    let resolved = space_git::resolve_checkout(Path::new(folder)).await;
    let (root, branch, upstream) = match resolved.checkout {
        SpaceGitCheckout::Branch {
            root,
            branch,
            upstream,
        } => (root, branch, upstream),
        SpaceGitCheckout::Detached { .. } => {
            return Err(ineligible(
                "Checkout is detached; switch to a branch in your terminal.",
            ));
        }
        SpaceGitCheckout::Unavailable { code, message, .. } => {
            return Err(if code == "not_a_checkout" || code == "checkout_missing" {
                ineligible("Checkout folder is unavailable or is not a Git checkout.")
            } else {
                not_run(&message)
            });
        }
    };
    if root != request.expected_root || branch != request.expected_branch {
        return Err(changed());
    }
    let upstream = match upstream {
        SpaceGitUpstream::Tracked { name, .. } => name,
        SpaceGitUpstream::None => {
            return Err(ineligible(
                "No upstream configured; set one in your terminal.",
            ));
        }
        SpaceGitUpstream::Gone { .. } => {
            return Err(ineligible(
                "Upstream branch is gone; repair it in your terminal.",
            ));
        }
        SpaceGitUpstream::Local { .. } => {
            return Err(ineligible("Upstream is a local branch, not a remote."));
        }
        SpaceGitUpstream::Unavailable { message, .. } => return Err(ineligible(&message)),
    };
    if upstream != request.expected_upstream {
        return Err(changed());
    }
    let root = PathBuf::from(root);
    let metadata = resolved.metadata.ok_or_else(changed)?;
    if metadata.name != upstream
        || metadata.remote.is_empty()
        || metadata.remote == "."
        || metadata.remote.starts_with('-')
        || metadata
            .remote
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, ':' | '\\' | '='))
    {
        return Err(not_run("Git returned an unsafe upstream remote."));
    }
    let branch_ref = format!("refs/heads/{branch}");
    for reference in [
        branch_ref.as_str(),
        metadata.full_ref.as_str(),
        metadata.remote_ref.as_str(),
    ] {
        if !reference.starts_with("refs/")
            || (reference == metadata.remote_ref && !reference.starts_with("refs/heads/"))
        {
            return Err(not_run("Git returned an invalid upstream branch ref."));
        }
        let output = space_git::git_output(&root, &["check-ref-format", reference])
            .await
            .map_err(|e| not_run(&e.message))?;
        if !output.status.success() {
            return Err(not_run("Git returned an invalid branch ref."));
        }
    }
    let common = PathBuf::from(
        space_git::git_line(
            &root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .await
        .map_err(|e| not_run(&e.message))?,
    );
    let git_dir = PathBuf::from(
        space_git::git_line(&root, &["rev-parse", "--absolute-git-dir"])
            .await
            .map_err(|e| not_run(&e.message))?,
    );
    let identities = [identity(&root)?, identity(&common)?, identity(&git_dir)?];
    let common = identities[1].canonical.clone();
    let head = space_git::git_line(&root, &["rev-parse", "--verify", "HEAD"])
        .await
        .map_err(|e| not_run(&e.message))?;
    let remote_urls = effective_remote_urls(&root, &metadata.remote, &request.action).await?;
    Ok(Target {
        root,
        common,
        identities,
        branch,
        upstream,
        metadata,
        head,
        remote_urls,
    })
}

pub(crate) async fn run_action(
    service: &CockpitService,
    session_id: &str,
    request: &SpaceGitActionRequest,
) -> Result<SpaceGitActionResponse, InspectionError> {
    run_action_with_deadline(service, session_id, request, WRITE_TIMEOUT).await
}

async fn run_action_with_deadline(
    service: &CockpitService,
    session_id: &str,
    request: &SpaceGitActionRequest,
    deadline: Duration,
) -> Result<SpaceGitActionResponse, InspectionError> {
    crate::validate_session_id(session_id)?;
    validate(request)?;
    let initial = resolve(service, session_id, request).await?;
    let reservation = service.git_actions.reserve(&initial.root)?;
    let repository = service.git_actions.repository(&initial.common);
    let service = service.clone();
    let session_id = session_id.to_owned();
    let request = request.clone();
    tokio::spawn(async move {
        let _reservation = reservation;
        let _repository = repository.lock_owned().await;
        let target = resolve(&service, &session_id, &request).await?;
        // Cached counts may change while another worktree is operating; target identity must not.
        if target.root != initial.root
            || target.common != initial.common
            || target.identities != initial.identities
            || target.branch != initial.branch
            || target.upstream != initial.upstream
            || target.metadata.full_ref != initial.metadata.full_ref
            || target.metadata.remote != initial.metadata.remote
            || target.metadata.remote_ref != initial.metadata.remote_ref
            || target.head != initial.head
            || target.remote_urls != initial.remote_urls
        {
            return Err(changed());
        }
        let output = run_write(git_write_command(&target, &request.action), deadline).await?;
        let outcome = classify(&target, &request.action, output).await?;
        Ok(SpaceGitActionResponse {
            session_id,
            space_id: request.space_id,
            action: request.action,
            root: target.root.to_string_lossy().into_owned(),
            branch: target.branch,
            upstream: target.upstream,
            outcome,
        })
    })
    .await
    .map_err(|_| unknown("Git action task ended without a proven result."))?
}

fn git_write_base(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY");
    command
}

async fn effective_remote_urls(
    root: &Path,
    remote: &str,
    action: &SpaceGitAction,
) -> Result<Vec<String>, InspectionError> {
    let mut command = git_write_base(root);
    command.args(["remote", "get-url", "--all"]);
    if *action == SpaceGitAction::Push {
        command.arg("--push");
    }
    command.arg("--").arg(remote);
    let output = crate::process::run_bounded_command(
        command,
        4096,
        4096,
        Duration::from_secs(2),
        "Git remote configuration",
    )
    .await
    .map_err(|_| not_run("The effective Git remote destination could not be read."))?;
    if !output.status.success() {
        return Err(not_run(
            "The effective Git remote destination could not be read.",
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| not_run("Git returned an invalid remote destination."))?;
    let urls: Vec<_> = text.lines().map(str::to_owned).collect();
    if urls.is_empty()
        || urls
            .iter()
            .any(|url| url.is_empty() || url.chars().any(char::is_control))
    {
        return Err(not_run("Git returned an invalid remote destination."));
    }
    Ok(urls)
}

fn git_write_command(target: &Target, action: &SpaceGitAction) -> Command {
    let mut command = git_write_base(&target.root);
    match action {
        SpaceGitAction::Pull => {
            command.args([
                "pull",
                "--ff-only",
                "--no-rebase",
                "--no-autostash",
                "--no-recurse-submodules",
                "--no-stat",
                "--no-tags",
                "--no-prune",
            ]);
            // Explicit refmap overrides *all* configured fetch destinations; only
            // the exact remote-tracking ref may be updated, never a local branch.
            if target.metadata.full_ref.starts_with("refs/remotes/") {
                command.arg(format!(
                    "--refmap=+{}:{}",
                    target.metadata.remote_ref, target.metadata.full_ref
                ));
            } else {
                command.arg("--refmap=");
            }
            command
                .arg("--")
                .arg(&target.metadata.remote)
                .arg(&target.metadata.remote_ref);
        }
        SpaceGitAction::Push => {
            // --no-mirror alone does not override remote.<name>.mirror: Git rejects
            // the explicit refspec before applying that flag. Override only this
            // destructive setting in-process; never edit repository configuration.
            command
                .arg("-c")
                .arg(format!("remote.{}.mirror=false", target.metadata.remote))
                .args([
                    "push",
                    "--porcelain",
                    "--no-follow-tags",
                    "--recurse-submodules=no",
                    "--no-mirror",
                    "--",
                ])
                .arg(&target.metadata.remote)
                .arg(format!(
                    "refs/heads/{}:{}",
                    target.branch, target.metadata.remote_ref
                ));
        }
    }
    command
}

async fn capped<R: AsyncRead + Unpin>(reader: R) -> Result<Vec<u8>, InspectionError> {
    let mut bytes = Vec::new();
    reader
        .take((WRITE_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| unknown("Git output could not be read."))?;
    if bytes.len() > WRITE_OUTPUT_BYTES {
        return Err(unknown("Git output exceeded its safety limit."));
    }
    Ok(bytes)
}
async fn run_write(mut command: Command, deadline: Duration) -> Result<Output, InspectionError> {
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .spawn()
        .map_err(|_| not_run("Git could not be started."))?;
    let mut child = crate::process::OwnedChild::new(child);
    let Some(stdout) = child.child.as_mut().and_then(|c| c.stdout.take()) else {
        child.kill_and_reap().await;
        return Err(unknown("Git stdout was not captured."));
    };
    let Some(stderr) = child.child.as_mut().and_then(|c| c.stderr.take()) else {
        child.kill_and_reap().await;
        return Err(unknown("Git stderr was not captured."));
    };
    let result = tokio::time::timeout(deadline, async {
        tokio::try_join!(capped(stdout), capped(stderr), async {
            child
                .child
                .as_mut()
                .expect("owned Git child")
                .wait()
                .await
                .map_err(|_| unknown("Git could not be waited for."))
        })
    })
    .await;
    match result {
        Ok(Ok((stdout, stderr, status))) => {
            child.reaped = true;
            if status.code().is_none() {
                return Err(unknown("Git terminated without a proven result."));
            }
            Ok(Output {
                status,
                stdout,
                stderr,
            })
        }
        Ok(Err(error)) => {
            child.kill_and_reap().await;
            Err(error)
        }
        Err(_) => {
            child.kill_and_reap().await;
            Err(unknown(
                "Git exceeded its deadline; its outcome is unknown.",
            ))
        }
    }
}

async fn count(root: &Path, range: &str) -> Option<u32> {
    space_git::git_line(root, &["rev-list", "--count", range])
        .await
        .ok()?
        .parse()
        .ok()
}
async fn classify(
    target: &Target,
    action: &SpaceGitAction,
    output: Output,
) -> Result<SpaceGitActionOutcome, InspectionError> {
    let git_dir = space_git::git_line(&target.root, &["rev-parse", "--absolute-git-dir"])
        .await
        .map_err(|_| unknown("The resulting checkout identity could not be read."))?;
    let common = space_git::git_line(
        &target.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await
    .map_err(|_| unknown("The resulting repository identity could not be read."))?;
    let resulting = [
        identity(&target.root),
        identity(Path::new(&common)),
        identity(Path::new(&git_dir)),
    ];
    if resulting
        .iter()
        .zip(&target.identities)
        .any(|(identity, expected)| identity.as_ref().ok() != Some(expected))
    {
        return Err(unknown("The checkout was replaced while Git was running."));
    }
    let stderr = std::str::from_utf8(&output.stderr)
        .map_err(|_| unknown("Git returned unreadable diagnostics."))?;
    let detail = space_git::safe_detail(stderr);
    match action {
        SpaceGitAction::Pull => {
            let head = space_git::git_line(&target.root, &["rev-parse", "--verify", "HEAD"])
                .await
                .map_err(|_| unknown("Git's resulting branch could not be read."))?;
            let branch = space_git::git_line(&target.root, &["symbolic-ref", "--quiet", "HEAD"])
                .await
                .map_err(|_| unknown("Git's resulting branch could not be read."))?;
            if branch != format!("refs/heads/{}", target.branch) {
                return Err(unknown(
                    "The checkout branch changed while Git was running.",
                ));
            }
            if output.status.success() {
                if head == target.head {
                    return Ok(SpaceGitActionOutcome::UpToDate);
                }
                return Ok(SpaceGitActionOutcome::Updated {
                    commits: count(&target.root, &format!("{}..{head}", target.head)).await,
                });
            }
            if head != target.head {
                return Err(unknown("Git failed after the branch changed."));
            }
            if stderr.contains("Not possible to fast-forward")
                || stderr.contains("diverging branches")
            {
                Ok(SpaceGitActionOutcome::Refused {
                    reason: SpaceGitRefusal::NotFastForward,
                    detail,
                })
            } else if stderr.contains("would be overwritten")
                || stderr.contains("commit your changes or stash them")
            {
                Ok(SpaceGitActionOutcome::Refused {
                    reason: SpaceGitRefusal::LocalChanges,
                    detail,
                })
            } else {
                Err(unknown(
                    "Git failed without a proven, side-effect-free refusal.",
                ))
            }
        }
        SpaceGitAction::Push => {
            let stdout = std::str::from_utf8(&output.stdout)
                .map_err(|_| unknown("Git returned unreadable push output."))?;
            let refspec = format!(
                "refs/heads/{}:{}",
                target.branch, target.metadata.remote_ref
            );
            let mut lines = stdout.lines().filter_map(|line| {
                let mut fields = line.split('\t');
                let flag = fields.next()?;
                (fields.next()? == refspec).then(|| (flag, fields.next().unwrap_or("")))
            });
            let first = lines.next();
            if lines.next().is_some() {
                return Err(unknown(
                    "Git reported conflicting results for the requested push.",
                ));
            }
            if let Some((flag, summary)) = first {
                match flag {
                    "=" if output.status.success() => return Ok(SpaceGitActionOutcome::UpToDate),
                    " " if output.status.success() => {
                        let range = summary.split_whitespace().next().unwrap_or("");
                        if range.contains("..")
                            && range.chars().all(|c| c.is_ascii_hexdigit() || c == '.')
                        {
                            return Ok(SpaceGitActionOutcome::Updated {
                                commits: count(&target.root, range).await,
                            });
                        }
                    }
                    "!" => {
                        return Ok(SpaceGitActionOutcome::Refused {
                            reason: if ["(non-fast-forward)", "(fetch first)", "(stale info)"]
                                .iter()
                                .any(|marker| summary.contains(marker))
                            {
                                SpaceGitRefusal::NotFastForward
                            } else {
                                SpaceGitRefusal::RemoteRejected
                            },
                            detail: space_git::safe_detail(summary),
                        });
                    }
                    _ => {}
                }
            }
            Err(unknown(
                "Git did not report a definitive result for the requested push.",
            ))
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::space_git::tests::{commit, fixture, git, pane, snapshot, space};
    use crate::{HerdrAdapter, SessionSubscription, TerminalSession};
    use cockpit_protocol::v1::*;
    use std::os::unix::fs::PermissionsExt;

    struct Adapter {
        snapshot: Mutex<SessionSnapshotResponse>,
    }
    fn compatible() -> HerdrCompatibility {
        HerdrCompatibility::Compatible {
            identity: HerdrIdentity {
                version: "0.9.1".into(),
                protocol: 22,
                schema_version: 1,
            },
        }
    }
    fn unused<T>() -> Result<T, InspectionError> {
        Err(InspectionError::new("unused", "Unused test operation."))
    }
    #[async_trait::async_trait]
    impl HerdrAdapter for Adapter {
        async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> {
            Ok(compatible())
        }
        async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> {
            Ok(compatible())
        }
        async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
            unused()
        }
        async fn session_snapshot(
            &self,
            _: &str,
        ) -> Result<SessionSnapshotResponse, InspectionError> {
            Ok(self.snapshot.lock().clone())
        }
        async fn focus(&self, _: &str, _: &FocusRequest) -> Result<FocusResponse, InspectionError> {
            unused()
        }
        async fn mutate(
            &self,
            _: &str,
            _: &ResourceMutationRequest,
        ) -> Result<ResourceMutationResponse, InspectionError> {
            unused()
        }
        async fn subscribe_session(
            &self,
            _: &str,
            _: &SessionSnapshotResponse,
        ) -> Result<SessionSubscription, InspectionError> {
            unused()
        }
        async fn open_terminal(
            &self,
            _: &TerminalOpenRequest,
        ) -> Result<TerminalSession, InspectionError> {
            unused()
        }
    }

    struct Fixture {
        root: PathBuf,
        origin: PathBuf,
        checkout: PathBuf,
        service: CockpitService,
        adapter: Arc<Adapter>,
    }
    impl Fixture {
        fn new() -> Self {
            let (root, origin, checkout) = fixture();
            // Local config isolates hooks from the developer's environment, without global env changes.
            git(
                &checkout,
                &[
                    "config",
                    "core.hooksPath",
                    checkout.join(".git/hooks").to_str().unwrap(),
                ],
            );
            let adapter = Arc::new(Adapter {
                snapshot: Mutex::new(snapshot(vec![space("w1", &checkout)])),
            });
            let service = CockpitService::new(CockpitMode::Normal, adapter.clone());
            Self {
                root,
                origin,
                checkout,
                service,
                adapter,
            }
        }
        fn request(&self, action: SpaceGitAction) -> SpaceGitActionRequest {
            SpaceGitActionRequest {
                space_id: "w1".into(),
                action,
                expected_root: self.checkout.to_string_lossy().into_owned(),
                expected_branch: "main".into(),
                expected_upstream: "origin/main".into(),
            }
        }
        fn other(&self) -> PathBuf {
            let other = self.root.join("other");
            git(&self.root, &["clone", "origin.git", "other"]);
            git(&other, &["config", "user.email", "fixture@example.test"]);
            git(&other, &["config", "user.name", "Fixture"]);
            other
        }
        fn hook(&self, text: &str) {
            let path = self.checkout.join(".git/hooks/pre-push");
            std::fs::write(&path, format!("#!/bin/sh\n{text}\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        async fn action(
            &self,
            action: SpaceGitAction,
        ) -> Result<SpaceGitActionResponse, InspectionError> {
            self.service
                .space_git_action("session", &self.request(action))
                .await
        }
        async fn block_repository(&self) -> tokio::sync::OwnedMutexGuard<()> {
            let common = self.checkout.join(".git").canonicalize().unwrap();
            self.service
                .git_actions
                .repository(&common)
                .lock_owned()
                .await
        }
        async fn wait_reserved(&self, checkout: &Path) {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if self.service.git_actions.roots.lock().contains(checkout) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("action reserved checkout");
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }
    async fn wait_file(path: &Path) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !path.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("hook reached marker");
    }
    fn launch(
        fixture: &Fixture,
        request: SpaceGitActionRequest,
    ) -> tokio::task::JoinHandle<Result<SpaceGitActionResponse, InspectionError>> {
        let service = fixture.service.clone();
        tokio::spawn(async move { service.space_git_action("session", &request).await })
    }
    fn write_commit(root: &Path, value: &str) {
        std::fs::write(root.join("tracked.txt"), value).unwrap();
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-m", value]);
    }

    #[tokio::test]
    async fn push_targets_upstream_despite_push_defaults_and_never_mirrors_or_follows_tags() {
        let f = Fixture::new();
        let old = git(&f.origin, &["rev-parse", "refs/heads/main"]);
        git(&f.checkout, &["branch", "unrelated"]);
        git(&f.checkout, &["tag", "-a", "not-pushed", "-m", "local tag"]);
        git(&f.checkout, &["config", "push.default", "nothing"]);
        git(
            &f.checkout,
            &[
                "config",
                "remote.origin.push",
                "+refs/heads/main:refs/heads/wrong",
            ],
        );
        git(&f.checkout, &["config", "remote.origin.mirror", "true"]);
        git(&f.checkout, &["config", "push.followTags", "true"]);
        git(
            &f.checkout,
            &["config", "branch.main.pushRemote", "missing"],
        );
        commit(&f.checkout, "local");
        assert_eq!(
            f.action(SpaceGitAction::Push).await.unwrap().outcome,
            SpaceGitActionOutcome::Updated { commits: Some(1) }
        );
        assert_ne!(old, git(&f.origin, &["rev-parse", "refs/heads/main"]));
        assert_eq!(
            git(&f.origin, &["rev-parse", "refs/heads/main"]),
            git(&f.checkout, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            git(
                &f.origin,
                &[
                    "for-each-ref",
                    "--format=%(refname)",
                    "refs/tags/",
                    "refs/heads/wrong",
                    "refs/heads/unrelated"
                ]
            ),
            ""
        );
        assert_eq!(
            f.action(SpaceGitAction::Push).await.unwrap().outcome,
            SpaceGitActionOutcome::UpToDate
        );
    }

    #[tokio::test]
    async fn rejected_non_fast_forward_push_preserves_remote_branch() {
        let f = Fixture::new();
        let other = f.other();
        commit(&other, "remote");
        git(&other, &["push", "origin", "main"]);
        let remote = git(&f.origin, &["rev-parse", "refs/heads/main"]);
        commit(&f.checkout, "local");
        assert!(matches!(
            f.action(SpaceGitAction::Push).await.unwrap().outcome,
            SpaceGitActionOutcome::Refused {
                reason: SpaceGitRefusal::NotFastForward,
                ..
            }
        ));
        assert_eq!(git(&f.origin, &["rev-parse", "refs/heads/main"]), remote);
    }

    #[tokio::test]
    async fn pull_fast_forwards_exact_upstream_without_rebase_or_autostash() {
        let f = Fixture::new();
        let other = f.other();
        write_commit(&other, "remote");
        git(&other, &["push", "origin", "main"]);
        git(&f.checkout, &["config", "pull.rebase", "true"]);
        git(&f.checkout, &["config", "pull.ff", "false"]);
        git(&f.checkout, &["config", "rebase.autoStash", "true"]);
        git(&f.checkout, &["config", "merge.autoStash", "true"]);
        assert_eq!(
            f.action(SpaceGitAction::Pull).await.unwrap().outcome,
            SpaceGitActionOutcome::Updated { commits: Some(1) }
        );
        assert_eq!(
            std::fs::read(f.checkout.join("tracked.txt")).unwrap(),
            b"remote"
        );
        assert_eq!(
            git(&f.checkout, &["rev-parse", "HEAD"]),
            git(&other, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            f.action(SpaceGitAction::Pull).await.unwrap().outcome,
            SpaceGitActionOutcome::UpToDate
        );
    }

    #[tokio::test]
    async fn diverged_pull_preserves_head_and_dirty_overlapping_pull_preserves_files() {
        let f = Fixture::new();
        let other = f.other();
        commit(&other, "remote");
        git(&other, &["push", "origin", "main"]);
        commit(&f.checkout, "local");
        let head = git(&f.checkout, &["rev-parse", "HEAD"]);
        assert!(matches!(
            f.action(SpaceGitAction::Pull).await.unwrap().outcome,
            SpaceGitActionOutcome::Refused {
                reason: SpaceGitRefusal::NotFastForward,
                ..
            }
        ));
        assert_eq!(git(&f.checkout, &["rev-parse", "HEAD"]), head);

        let f = Fixture::new();
        write_commit(&f.checkout, "base");
        git(&f.checkout, &["push", "origin", "main"]);
        let other = f.other();
        write_commit(&other, "remote");
        git(&other, &["push", "origin", "main"]);
        std::fs::write(f.checkout.join("tracked.txt"), b"user edits").unwrap();
        git(&f.checkout, &["config", "merge.autoStash", "true"]);
        let head = git(&f.checkout, &["rev-parse", "HEAD"]);
        assert!(matches!(
            f.action(SpaceGitAction::Pull).await.unwrap().outcome,
            SpaceGitActionOutcome::Refused {
                reason: SpaceGitRefusal::LocalChanges,
                ..
            }
        ));
        assert_eq!(
            std::fs::read(f.checkout.join("tracked.txt")).unwrap(),
            b"user edits"
        );
        assert_eq!(git(&f.checkout, &["rev-parse", "HEAD"]), head);
        assert_eq!(git(&f.checkout, &["stash", "list"]), "");
    }

    #[tokio::test]
    async fn pane_folder_authority_uses_checkout_root_and_never_accepts_client_path() {
        let f = Fixture::new();
        let nested = f.checkout.join("nested");
        std::fs::create_dir(&nested).unwrap();
        {
            let mut state = f.adapter.snapshot.lock();
            state.spaces[0].git = None;
            state.panes.push(pane("w1", &nested));
        }
        commit(&f.checkout, "local");
        let mut request = f.request(SpaceGitAction::Push);
        request.expected_root = f.root.to_string_lossy().into_owned();
        let remote = git(&f.origin, &["rev-parse", "refs/heads/main"]);
        assert_eq!(
            f.service
                .space_git_action("session", &request)
                .await
                .unwrap_err()
                .code,
            "space_git_target_changed"
        );
        assert_eq!(git(&f.origin, &["rev-parse", "refs/heads/main"]), remote);
        assert!(matches!(
            f.action(SpaceGitAction::Push).await.unwrap().outcome,
            SpaceGitActionOutcome::Updated { .. }
        ));
    }

    #[tokio::test]
    async fn stale_expected_branch_upstream_and_detached_checkout_never_write() {
        let f = Fixture::new();
        commit(&f.checkout, "local");
        let remote = git(&f.origin, &["rev-parse", "refs/heads/main"]);
        for field in ["branch", "upstream", "root"] {
            let mut request = f.request(SpaceGitAction::Push);
            match field {
                "branch" => request.expected_branch = "other".into(),
                "upstream" => request.expected_upstream = "else/main".into(),
                _ => request.expected_root = "/different".into(),
            }
            assert_eq!(
                f.service
                    .space_git_action("session", &request)
                    .await
                    .unwrap_err()
                    .code,
                "space_git_target_changed"
            );
        }
        git(&f.checkout, &["switch", "--detach"]);
        assert_eq!(
            f.action(SpaceGitAction::Push).await.unwrap_err().code,
            "space_git_action_ineligible"
        );
        assert_eq!(git(&f.origin, &["rev-parse", "refs/heads/main"]), remote);
    }

    #[tokio::test]
    async fn queued_action_revalidates_space_branch_upstream_remote_ref_and_head() {
        for mutation in ["space", "branch", "upstream", "remote_ref", "head"] {
            let f = Fixture::new();
            commit(&f.checkout, "local");
            let remote = git(&f.origin, &["rev-parse", "refs/heads/main"]);
            let guard = f.block_repository().await;
            let job = launch(&f, f.request(SpaceGitAction::Push));
            f.wait_reserved(&f.checkout).await;
            match mutation {
                "space" => f.adapter.snapshot.lock().spaces.clear(),
                "branch" => {
                    git(&f.checkout, &["switch", "-c", "different"]);
                }
                "upstream" => {
                    git(&f.checkout, &["branch", "--unset-upstream"]);
                }
                "remote_ref" => {
                    // The displayed short upstream remains identical but its write destination changed.
                    git(
                        &f.checkout,
                        &[
                            "config",
                            "--add",
                            "remote.origin.fetch",
                            "+refs/heads/other:refs/remotes/origin/main",
                        ],
                    );
                    git(
                        &f.checkout,
                        &["config", "branch.main.merge", "refs/heads/other"],
                    );
                }
                _ => {
                    commit(&f.checkout, "replacement");
                }
            }
            drop(guard);
            let error = job.await.unwrap().unwrap_err();
            assert!(
                matches!(
                    error.code.as_str(),
                    "space_git_target_changed" | "space_git_action_ineligible"
                ),
                "{mutation}: {error}"
            );
            assert_eq!(git(&f.origin, &["rev-parse", "refs/heads/main"]), remote);
        }
    }

    #[tokio::test]
    async fn queued_action_detects_replaced_checkout_even_at_same_path_and_branch() {
        let f = Fixture::new();
        let guard = f.block_repository().await;
        let job = launch(&f, f.request(SpaceGitAction::Push));
        f.wait_reserved(&f.checkout).await;
        std::fs::rename(&f.checkout, f.root.join("old")).unwrap();
        git(&f.root, &["clone", "origin.git", "clone"]);
        drop(guard);
        assert_eq!(
            job.await.unwrap().unwrap_err().code,
            "space_git_target_changed"
        );
    }

    #[tokio::test]
    async fn same_checkout_is_single_flight_and_dropped_request_does_not_cancel_push() {
        let f = Fixture::new();
        commit(&f.checkout, "local");
        let entered = f.root.join("entered");
        let release = f.root.join("release");
        f.hook(&format!(
            "touch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done",
            entered.display(),
            release.display()
        ));
        let job = launch(&f, f.request(SpaceGitAction::Push));
        wait_file(&entered).await;
        assert_eq!(
            f.action(SpaceGitAction::Push).await.unwrap_err().code,
            "space_git_action_in_progress"
        );
        job.abort();
        let _ = job.await;
        std::fs::write(release, b"go").unwrap();
        let wanted = git(&f.checkout, &["rev-parse", "HEAD"]);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if git(&f.origin, &["rev-parse", "refs/heads/main"]) == wanted
                    && !f.service.git_actions.roots.lock().contains(&f.checkout)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("detached write completes and releases reservation");
    }

    #[tokio::test]
    async fn linked_worktrees_queue_without_overlapping_shared_repository_writes() {
        let f = Fixture::new();
        let linked = f.root.join("linked");
        git(
            &f.checkout,
            &["worktree", "add", "-b", "topic", linked.to_str().unwrap()],
        );
        git(
            &f.checkout,
            &["push", "origin", "refs/heads/topic:refs/heads/topic"],
        );
        git(
            &linked,
            &["branch", "--set-upstream-to=origin/topic", "topic"],
        );
        commit(&f.checkout, "main update");
        commit(&linked, "topic update");
        f.adapter.snapshot.lock().spaces.push(space("w2", &linked));
        let main_entered = f.root.join("main-entered");
        let topic_entered = f.root.join("topic-entered");
        let release = f.root.join("release");
        f.hook(&format!("if [ \"$(git symbolic-ref --short HEAD)\" = main ]; then\n touch '{}'\n while [ ! -f '{}' ]; do sleep 0.01; done\nelse\n touch '{}'\nfi", main_entered.display(), release.display(), topic_entered.display()));
        let main = launch(&f, f.request(SpaceGitAction::Push));
        wait_file(&main_entered).await;
        let topic = launch(
            &f,
            SpaceGitActionRequest {
                space_id: "w2".into(),
                action: SpaceGitAction::Push,
                expected_root: linked.to_string_lossy().into_owned(),
                expected_branch: "topic".into(),
                expected_upstream: "origin/topic".into(),
            },
        );
        f.wait_reserved(&linked).await;
        assert!(
            !topic_entered.exists(),
            "linked write cannot enter its hook while main holds repository ownership"
        );
        std::fs::write(release, b"go").unwrap();
        assert!(matches!(
            main.await.unwrap().unwrap().outcome,
            SpaceGitActionOutcome::Updated { .. }
        ));
        assert!(matches!(
            topic.await.unwrap().unwrap().outcome,
            SpaceGitActionOutcome::Updated { .. }
        ));
        assert!(topic_entered.exists());
        assert_eq!(
            git(&f.origin, &["rev-parse", "refs/heads/main"]),
            git(&f.checkout, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            git(&f.origin, &["rev-parse", "refs/heads/topic"]),
            git(&linked, &["rev-parse", "HEAD"])
        );
    }

    #[tokio::test]
    async fn hook_deadline_and_output_cap_are_unknown_not_false_success_or_retryable() {
        let f = Fixture::new();
        commit(&f.checkout, "local");
        let remote = git(&f.origin, &["rev-parse", "refs/heads/main"]);
        f.hook("sleep 10");
        assert_eq!(
            run_action_with_deadline(
                &f.service,
                "session",
                &f.request(SpaceGitAction::Push),
                Duration::from_millis(100)
            )
            .await
            .unwrap_err()
            .code,
            "space_git_outcome_unknown"
        );
        assert_eq!(git(&f.origin, &["rev-parse", "refs/heads/main"]), remote);
        f.hook("yes oversized-output");
        assert_eq!(
            f.action(SpaceGitAction::Push).await.unwrap_err().code,
            "space_git_outcome_unknown"
        );
        assert_eq!(git(&f.origin, &["rev-parse", "refs/heads/main"]), remote);
    }

    #[tokio::test]
    async fn failed_pre_push_hook_is_unknown_and_remote_hook_rejection_is_definitive() {
        let f = Fixture::new();
        commit(&f.checkout, "local");
        f.hook("echo 'private hook failed' >&2\nexit 1");
        assert_eq!(
            f.action(SpaceGitAction::Push).await.unwrap_err().code,
            "space_git_outcome_unknown"
        );
        f.hook("exit 0");
        let hook = f.origin.join("hooks/pre-receive");
        std::fs::write(
            &hook,
            "#!/bin/sh\necho 'remote policy refused' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(
            f.action(SpaceGitAction::Push).await.unwrap().outcome,
            SpaceGitActionOutcome::Refused {
                reason: SpaceGitRefusal::RemoteRejected,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn timeout_can_mean_remote_updated_and_new_remote_ref_is_not_claimed_as_normal_update() {
        let f = Fixture::new();
        commit(&f.checkout, "local");
        let marker = f.root.join("received");
        let hook = f.origin.join("hooks/post-receive");
        std::fs::write(
            &hook,
            format!("#!/bin/sh\ntouch '{}'\nsleep 10\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o700)).unwrap();
        let service = f.service.clone();
        let request = f.request(SpaceGitAction::Push);
        let job = tokio::spawn(async move {
            run_action_with_deadline(&service, "session", &request, Duration::from_secs(1)).await
        });
        wait_file(&marker).await;
        assert_eq!(
            job.await.unwrap().unwrap_err().code,
            "space_git_outcome_unknown"
        );
        assert_eq!(
            git(&f.origin, &["rev-parse", "refs/heads/main"]),
            git(&f.checkout, &["rev-parse", "HEAD"])
        );

        let f = Fixture::new();
        git(&f.origin, &["update-ref", "-d", "refs/heads/main"]);
        assert_eq!(
            f.action(SpaceGitAction::Push).await.unwrap_err().code,
            "space_git_outcome_unknown"
        );
        assert_eq!(
            git(&f.origin, &["rev-parse", "refs/heads/main"]),
            git(&f.checkout, &["rev-parse", "HEAD"])
        );
    }

    #[tokio::test]
    async fn invalid_requests_and_unavailable_upstreams_do_not_run_git() {
        let f = Fixture::new();
        let mut request = f.request(SpaceGitAction::Push);
        request.expected_branch = "a".repeat(4097);
        assert_eq!(
            f.service
                .space_git_action("session", &request)
                .await
                .unwrap_err()
                .code,
            "invalid_space_git_action"
        );
        assert_eq!(
            f.service
                .space_git_action("../session", &f.request(SpaceGitAction::Push))
                .await
                .unwrap_err()
                .code,
            "invalid_session_id"
        );
        git(&f.checkout, &["branch", "--unset-upstream"]);
        assert_eq!(
            f.action(SpaceGitAction::Push).await.unwrap_err().code,
            "space_git_action_ineligible"
        );
        git(&f.checkout, &["branch", "local"]);
        git(&f.checkout, &["branch", "--set-upstream-to=local"]);
        assert_eq!(
            f.action(SpaceGitAction::Push).await.unwrap_err().code,
            "space_git_action_ineligible"
        );
    }

    #[tokio::test]
    async fn renamed_local_branch_and_remote_alias_still_target_configured_remote_branch() {
        let f = Fixture::new();
        git(&f.checkout, &["branch", "-m", "topic"]);
        git(&f.checkout, &["remote", "rename", "origin", "team"]);
        commit(&f.checkout, "local topic");
        let request = SpaceGitActionRequest {
            space_id: "w1".into(),
            action: SpaceGitAction::Push,
            expected_root: f.checkout.to_string_lossy().into_owned(),
            expected_branch: "topic".into(),
            expected_upstream: "team/main".into(),
        };
        assert_eq!(
            f.service
                .space_git_action("session", &request)
                .await
                .unwrap()
                .outcome,
            SpaceGitActionOutcome::Updated { commits: Some(1) }
        );
        assert_eq!(
            git(&f.origin, &["rev-parse", "refs/heads/main"]),
            git(&f.checkout, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            git(
                &f.origin,
                &["for-each-ref", "--format=%(refname)", "refs/heads/topic"]
            ),
            ""
        );
        let other = f.other();
        commit(&other, "remote main");
        git(&other, &["push", "origin", "main"]);
        let request = SpaceGitActionRequest {
            action: SpaceGitAction::Pull,
            ..request
        };
        assert_eq!(
            f.service
                .space_git_action("session", &request)
                .await
                .unwrap()
                .outcome,
            SpaceGitActionOutcome::Updated { commits: Some(1) }
        );
        assert_eq!(
            git(&f.checkout, &["rev-parse", "HEAD"]),
            git(&other, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            git(&f.checkout, &["symbolic-ref", "--short", "HEAD"]).trim(),
            "topic"
        );
    }

    #[tokio::test]
    async fn pull_ignores_configured_fetch_destinations_for_current_and_unrelated_local_branches() {
        let f = Fixture::new();
        let other = f.other();
        commit(&other, "remote diverged");
        git(&other, &["push", "origin", "main"]);
        commit(&f.checkout, "local only");
        let local = git(&f.checkout, &["rev-parse", "HEAD"]);
        git(
            &f.checkout,
            &["config", "--unset-all", "remote.origin.fetch"],
        );
        git(
            &f.checkout,
            &[
                "config",
                "--add",
                "remote.origin.fetch",
                "+refs/heads/main:refs/heads/main",
            ],
        );
        let request = SpaceGitActionRequest {
            expected_upstream: "main".into(),
            ..f.request(SpaceGitAction::Pull)
        };
        assert!(matches!(
            f.service
                .space_git_action("session", &request)
                .await
                .unwrap()
                .outcome,
            SpaceGitActionOutcome::Refused {
                reason: SpaceGitRefusal::NotFastForward,
                ..
            }
        ));
        assert_eq!(git(&f.checkout, &["rev-parse", "HEAD"]), local);

        let f = Fixture::new();
        git(&f.checkout, &["switch", "-c", "victim"]);
        commit(&f.checkout, "victim unique history");
        let victim = git(&f.checkout, &["rev-parse", "HEAD"]);
        git(&f.checkout, &["switch", "main"]);
        let other = f.other();
        commit(&other, "remote advance");
        git(&other, &["push", "origin", "main"]);
        git(
            &f.checkout,
            &[
                "config",
                "--add",
                "remote.origin.fetch",
                "+refs/heads/main:refs/heads/victim",
            ],
        );
        assert_eq!(
            f.action(SpaceGitAction::Pull).await.unwrap().outcome,
            SpaceGitActionOutcome::Updated { commits: Some(1) }
        );
        assert_eq!(
            git(&f.checkout, &["rev-parse", "refs/heads/victim"]),
            victim
        );
        assert_eq!(
            git(&f.checkout, &["rev-parse", "HEAD"]),
            git(&other, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            git(&f.checkout, &["rev-parse", "refs/remotes/origin/main"]),
            git(&other, &["rev-parse", "HEAD"])
        );
    }

    #[tokio::test]
    async fn hook_that_publishes_then_prints_auth_failure_never_offers_retry() {
        let f = Fixture::new();
        commit(&f.checkout, "published by hook");
        f.hook("git -c core.hooksPath=/dev/null push --quiet origin refs/heads/main:refs/heads/main || exit 2\necho 'authentication failed' >&2\nexit 1");
        assert_eq!(
            f.action(SpaceGitAction::Push).await.unwrap_err().code,
            "space_git_outcome_unknown"
        );
        assert_eq!(
            git(&f.origin, &["rev-parse", "refs/heads/main"]),
            git(&f.checkout, &["rev-parse", "HEAD"])
        );
    }

    #[tokio::test]
    async fn queued_action_refuses_changed_effective_fetch_push_and_rewritten_remote_urls() {
        for mutation in [
            "url",
            "pushurl",
            "insteadOf",
            "pushInsteadOf",
            "extraPushUrl",
            "fetchUrl",
        ] {
            let f = Fixture::new();
            commit(&f.checkout, "local unpublished");
            let replacement = f.root.join("replacement.git");
            git(
                &f.root,
                &["clone", "--bare", "origin.git", "replacement.git"],
            );
            let original_tip = git(&f.origin, &["rev-parse", "refs/heads/main"]);
            let replacement_tip = git(&replacement, &["rev-parse", "refs/heads/main"]);
            let original_url = git(&f.checkout, &["remote", "get-url", "origin"]);
            let guard = f.block_repository().await;
            let action = if mutation == "fetchUrl" {
                SpaceGitAction::Pull
            } else {
                SpaceGitAction::Push
            };
            let job = launch(&f, f.request(action));
            f.wait_reserved(&f.checkout).await;
            match mutation {
                "url" | "fetchUrl" => {
                    git(
                        &f.checkout,
                        &["remote", "set-url", "origin", replacement.to_str().unwrap()],
                    );
                }
                "pushurl" => {
                    git(
                        &f.checkout,
                        &[
                            "remote",
                            "set-url",
                            "--push",
                            "origin",
                            replacement.to_str().unwrap(),
                        ],
                    );
                }
                "extraPushUrl" => {
                    git(
                        &f.checkout,
                        &["remote", "set-url", "--push", "origin", original_url.trim()],
                    );
                    git(
                        &f.checkout,
                        &[
                            "remote",
                            "set-url",
                            "--add",
                            "--push",
                            "origin",
                            replacement.to_str().unwrap(),
                        ],
                    );
                }
                kind => {
                    let key = format!("url.{}.{kind}", replacement.display());
                    git(&f.checkout, &["config", &key, original_url.trim()]);
                }
            }
            drop(guard);
            assert_eq!(
                job.await.unwrap().unwrap_err().code,
                "space_git_target_changed",
                "{mutation}"
            );
            assert_eq!(
                git(&f.origin, &["rev-parse", "refs/heads/main"]),
                original_tip
            );
            assert_eq!(
                git(&replacement, &["rev-parse", "refs/heads/main"]),
                replacement_tip
            );
        }
    }

    #[test]
    fn git_diagnostics_are_bounded_and_do_not_expose_urls_or_credentials() {
        for message in [
            "fatal: unable to access https://user:secret@example.test/repo",
            "error: Authorization: Basic abcdef",
            "fatal: token secret",
            "error: git@example.test:private/repo",
        ] {
            let detail = space_git::safe_detail(message);
            assert!(
                !detail.contains("secret")
                    && !detail.contains("abcdef")
                    && !detail.contains("example.test")
            );
            assert!(detail.chars().count() <= 512);
        }
        assert_eq!(
            space_git::safe_detail(&format!("error: {}", "x".repeat(1000))).len(),
            512
        );
    }
}
