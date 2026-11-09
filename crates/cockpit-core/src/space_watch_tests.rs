use super::space_watch::{SpaceWatch, SpaceWatchSnapshot, SpaceWatchStatus, SpaceWatchTopic, TestWaitRequest};
use std::{path::{Path, PathBuf}, process::Command, time::Duration};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

fn request(git: u64, tags: u64, timeout_ms: u32) -> TestWaitRequest {
    TestWaitRequest { after_git: git, after_tags: tags, timeout_ms }
}
async fn quiet(watch: &SpaceWatch, mut state: SpaceWatchSnapshot) -> SpaceWatchSnapshot {
    loop {
        let next = watch.wait("s", &request(state.git, state.tags, 100)).await;
        if next.git == state.git && next.tags == state.tags { return next; }
        state = next;
    }
}

async fn tag_change(watch: &SpaceWatch, mut state: SpaceWatchSnapshot) -> SpaceWatchSnapshot {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let tags = state.tags;
    while state.tags == tags {
        state = tokio::time::timeout_at(deadline, watch.wait("s", &request(state.git, state.tags, 2000))).await.expect("tag notification");
    }
    state
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-space-watch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

#[tokio::test]
async fn filesystem_changes_invalidate_and_reads_do_not() {
    let root = Fixture::new();
    let repo = root.0.join("repo");
    let tags = root.0.join("tags");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&tags).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["commit", "--allow-empty", "-m", "initial"]);
    let watch = SpaceWatch::default();
    watch.set_tag_dir(&tags);
    let initial = watch.wait("s", &request(0, 0, 0)).await;
    assert_eq!(initial.tags_watch, SpaceWatchStatus::Watching);
    watch.register_git(watch.begin_git("s").unwrap(), vec![(repo.join(".git"), repo.join(".git"))], true);
    let armed = watch.wait("s", &request(initial.git, initial.tags, 1000)).await;
    assert_eq!(armed.git_watch, SpaceWatchStatus::Watching);
    assert_ne!(armed.git, initial.git);
    let armed = quiet(&watch, armed).await;
    // Neither Git reads nor lock-file writes invalidate the sidebar.
    git(&repo, &["status", "--porcelain"]);
    git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join(".git/refs/heads/unused.lock"), b"lock").unwrap();
    let unchanged = watch.wait("s", &request(armed.git, armed.tags, 200)).await;
    assert_eq!((unchanged.git, unchanged.tags), (armed.git, armed.tags));
    git(&repo, &["commit", "--allow-empty", "-m", "next"]);
    let committed = watch.wait("s", &request(unchanged.git, unchanged.tags, 2000)).await;
    assert_ne!(committed.git, unchanged.git);
    let committed = quiet(&watch, committed).await;
    git(&repo, &["switch", "-c", "other"]);
    let switched = watch.wait("s", &request(committed.git, committed.tags, 2000)).await;
    assert_ne!(switched.git, committed.git);
    let switched = quiet(&watch, switched).await;
    git(&repo, &["branch", "--set-upstream-to=main", "other"]);
    let upstream = watch.wait("s", &request(switched.git, switched.tags, 2000)).await;
    assert_ne!(upstream.git, switched.git);
    let upstream = quiet(&watch, upstream).await;
    std::fs::write(tags.join("temporary"), b"{}").unwrap();
    std::fs::rename(tags.join("temporary"), tags.join("space-tags.json")).unwrap();
    let tagged = tag_change(&watch, upstream.clone()).await;
    assert_ne!(tagged.tags, upstream.tags);
}

#[tokio::test]
async fn fetch_updates_remote_refs() {
    let root = Fixture::new();
    let source = root.0.join("source");
    let clone = root.0.join("clone");
    std::fs::create_dir_all(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    git(&source, &["config", "user.email", "test@example.invalid"]);
    git(&source, &["config", "user.name", "Test"]);
    git(&source, &["commit", "--allow-empty", "-m", "initial"]);
    git(&root.0, &["clone", source.to_str().unwrap(), clone.to_str().unwrap()]);
    let watch = SpaceWatch::default();
    watch.register_git(watch.begin_git("s").unwrap(), vec![(clone.join(".git"), clone.join(".git"))], true);
    let initial = quiet(&watch, watch.wait("s", &request(0, 0, 0)).await).await;
    git(&source, &["commit", "--allow-empty", "-m", "remote change"]);
    git(&clone, &["fetch", "origin"]);
    let fetched = watch.wait("s", &request(initial.git, initial.tags, 2000)).await;
    assert_ne!(fetched.git, initial.git);
}
#[tokio::test]
async fn failed_watch_is_unavailable_and_can_be_rearmed() {
    let root = Fixture::new();
    let watch = SpaceWatch::default();
    watch.register_git(watch.begin_git("s").unwrap(), vec![(root.0.join("missing"), root.0.join("missing"))], true);
    let failed = watch.wait("s", &request(0, 0, 0)).await;
    assert_eq!(failed.git_watch, SpaceWatchStatus::Unavailable);
    std::fs::create_dir_all(root.0.join("missing/refs")).unwrap();
    std::fs::write(root.0.join("missing/config"), b"").unwrap();
    watch.register_git(watch.begin_git("s").unwrap(), vec![(root.0.join("missing"), root.0.join("missing"))], true);
    let recovered = watch.wait("s", &request(failed.git, failed.tags, 1000)).await;
    assert_eq!(recovered.git_watch, SpaceWatchStatus::Watching);
    assert_ne!(recovered.git, failed.git);
}

#[tokio::test]
async fn stale_git_read_cannot_replace_newer_watch_set() {
    let root = Fixture::new();
    for name in ["old", "new"] {
        std::fs::create_dir_all(root.0.join(name).join("refs")).unwrap();
        std::fs::write(root.0.join(name).join("config"), b"").unwrap();
    }
    let watch = SpaceWatch::default();
    let old_generation = watch.next_read();
    let new_generation = watch.next_read();
    let new = watch.begin_git_for("s", "test-server", new_generation).unwrap();
    let path = root.0.join("new");
    watch.register_git(new, vec![(path.clone(), path)], true);
    let initial = quiet(&watch, watch.wait("s", &request(0, 0, 0)).await).await;
    // The older snapshot completed late, after the newer watch was installed.
    let old = watch.begin_git_for("s", "test-server", old_generation).unwrap();
    let path = root.0.join("old");
    watch.register_git(old, vec![(path.clone(), path)], true);
    std::fs::write(root.0.join("new/HEAD"), b"ref: refs/heads/main\n").unwrap();
    let changed = watch.wait("s", &request(initial.git, initial.tags, 2000)).await;
    assert_ne!(changed.git, initial.git);
}

#[test]
fn git_first_arm_failure_wakes_pending_subscription() {
    let watch = SpaceWatch::default();
    let lease = watch.subscribe("s", "test-server", SpaceWatchTopic::Git).unwrap();
    let receiver = lease.receiver();
    let before = lease.checkpoint();
    assert!(!before.git_initialized);
    watch.register_git(watch.begin_git("s").unwrap(), Vec::new(), false);
    let after = lease.checkpoint();
    assert!(after.git_initialized);
    assert_eq!(after.git_watch, SpaceWatchStatus::Unavailable);
    assert_ne!(after.git, before.git);
    assert!(receiver.has_changed().unwrap());
}

#[test]
fn active_subscriptions_are_not_evicted() {
    let watch = SpaceWatch::default();
    let mut leases = Vec::new();
    for id in 0..256 {
        leases.push(watch.subscribe(&format!("s{id}"), "server", SpaceWatchTopic::Git).unwrap());
    }
    assert!(matches!(watch.subscribe("overflow", "server", SpaceWatchTopic::Git), Err(error) if error.code == "watch_limit"));
    leases.pop();
    assert!(watch.subscribe("overflow", "server", SpaceWatchTopic::Git).is_ok());
}

#[test]
fn reused_session_id_does_not_share_watch_state_across_server_incarnations() {
    let watch = SpaceWatch::default();
    let old = watch.subscribe("s", "old-server", SpaceWatchTopic::Git).unwrap();
    let new = watch.subscribe("s", "new-server", SpaceWatchTopic::Git).unwrap();
    let before = new.checkpoint();
    let read = watch.begin_git_for("s", "old-server", watch.next_read()).unwrap();
    watch.register_git(read, Vec::new(), false);
    assert!(old.checkpoint().git_initialized);
    assert_eq!(new.checkpoint(), before);
    let read = watch.begin_git_for("s", "new-server", watch.next_read()).unwrap();
    watch.register_git(read, Vec::new(), false);
    assert!(new.checkpoint().git_initialized);
}
