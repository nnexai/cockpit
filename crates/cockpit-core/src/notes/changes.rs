//! Read-only watch plans. Notifications are hints, never revisions or content.
use super::{fs, NotesService};
use crate::{InspectionError, local_changes::{LocalWatchMatch, LocalWatchPlan, LocalWatchRule}};
use cockpit_protocol::local_changes::LocalChangeTopic as Topic;
use std::{collections::BTreeSet, path::{Path, PathBuf}, sync::Arc};

// A path-based watcher must not silently follow a replaced directory. Capture every
// existing ancestor and re-open it without following links after registration and
// before delivery. Missing components are watched through their nearest parent.
#[cfg(unix)]
fn identity(path: &Path) -> Result<Option<(u64, u64)>, InspectionError> {
    use cap_std::fs::MetadataExt;
    match fs::root(path, false) {
        Ok((_, dir)) => {
            let meta = dir.dir_metadata().map_err(fs::io_error)?;
            Ok(Some((meta.dev(), meta.ino())))
        }
        Err(e) if e.code == "notes_not_found" => Ok(None),
        Err(e) => Err(e),
    }
}
#[cfg(not(unix))]
fn identity(_path: &Path) -> Result<Option<(u64, u64)>, InspectionError> {
    Err(fs::error("local_changes_unavailable", "Safe Notes directory identity is unavailable on this platform"))
}

struct PlanBuilder {
    directories: BTreeSet<PathBuf>,
    identities: Vec<(PathBuf, Option<(u64, u64)>)>,
    rules: Vec<LocalWatchRule>,
    threads: Option<(PathBuf, Vec<String>)>,
}
impl PlanBuilder {
    fn new() -> Self { Self { directories: BTreeSet::new(), identities: Vec::new(), rules: Vec::new(), threads: None } }
    fn rule(&mut self, path: PathBuf, match_kind: LocalWatchMatch, topics: &[Topic]) {
        self.rules.push(LocalWatchRule { path, match_kind, topics: topics.to_vec() });
    }
    // Return whether the entire chain exists. Only the first absent component
    // is watched: creation requires a fresh plan before deeper paths are used.
    fn directory(&mut self, path: &Path, topics: &[Topic]) -> Result<bool, InspectionError> {
        let mut chain: Vec<_> = path.ancestors().map(Path::to_path_buf).collect();
        chain.reverse();
        for component in chain {
            if self.identities.iter().any(|(p, _)| p == &component) { continue; }
            let id = identity(&component)?;
            if let Some(parent) = component.parent() {
                self.directories.insert(parent.to_path_buf());
                self.rule(component.clone(), LocalWatchMatch::Lifecycle, topics);
            }
            self.identities.push((component.clone(), id));
            if id.is_none() { return Ok(false); }
            self.directories.insert(component);
        }
        Ok(true)
    }
    fn finish(self, topics: Vec<Topic>) -> LocalWatchPlan {
        let identities = self.identities;
        let threads = self.threads;
        let revalidate = Arc::new(move || {
            for (path, expected) in &identities {
                let actual = identity(path).map_err(|_| "Notes watch directory is unavailable or unsafe".to_owned())?;
                if actual != *expected { return Err("Notes watch directory changed; reconnect to refresh".to_owned()); }
            }
            if let Some((path, expected)) = &threads {
                let (_, dir) = fs::root(path, false).map_err(|_| "Notes comments directory is unavailable".to_owned())?;
                let names = thread_names(&dir).map_err(|_| "Notes comments directory cannot be enumerated".to_owned())?;
                if &names != expected { return Err("Notes comment threads changed; reconnect to refresh".to_owned()); }
            }
            Ok(())
        });
        LocalWatchPlan::new(self.directories.into_iter().collect(), self.rules, topics, revalidate)
    }
}

fn thread_names(dir: &cap_std::fs::Dir) -> Result<Vec<String>, InspectionError> {
    Ok(fs::entries(dir, 4096)?.into_iter().filter(|name| fs::validate_id(name, 64).is_ok()).collect())
}

impl NotesService {
    /// Observe one pinned UUID without consulting the registry or Herdr.
    pub(crate) fn content_watch_plan(&self, notes_id: &str) -> Result<LocalWatchPlan, InspectionError> {
        fs::validate_uuid(notes_id)?;
        let root = fs::absolute(&self.root)?;
        let base = root.join(notes_id);
        let topics = vec![Topic::Scratchpad, Topic::Todos, Topic::Decisions, Topic::Comments];
        let mut plan = PlanBuilder::new();
        if plan.directory(&base, &topics)? {
            plan.rule(base.join("scratchpad.md"), LocalWatchMatch::Exact, &[Topic::Scratchpad]);
            plan.rule(base.join("todos.md"), LocalWatchMatch::Exact, &[Topic::Todos]);
            let decisions = base.join("decisions");
            if plan.directory(&decisions, &[Topic::Decisions])? {
                plan.rule(decisions, LocalWatchMatch::MarkdownChildren, &[Topic::Decisions]);
            }
            let comments = base.join("comments");
            if plan.directory(&comments, &[Topic::Comments])? {
                // New or removed thread directories invalidate this observation epoch.
                plan.rule(comments.clone(), LocalWatchMatch::LifecycleChildren, &[Topic::Comments]);
                let (_, dir) = fs::root(&comments, false)?;
                let names = thread_names(&dir)?;
                plan.threads = Some((comments.clone(), names.clone()));
                for name in names {
                    let thread = comments.join(name);
                    if plan.directory(&thread, &[Topic::Comments])? {
                        plan.rule(thread, LocalWatchMatch::MarkdownChildren, &[Topic::Comments]);
                    }
                }
            }
        }
        Ok(plan.finish(topics))
    }

    /// Authorize the live Space and pin its endpoint boot identity separately from
    /// the on-disk registry. Watching does not create the registry or Notes root.
    pub(crate) async fn binding_watch_plan(&self, session_id: &str, space_id: &str) -> Result<(LocalWatchPlan, String), InspectionError> {
        let auth = self.authorize(session_id, space_id).await?;
        let root = fs::absolute(&self.root)?;
        let mut plan = PlanBuilder::new();
        if plan.directory(&root.join(".cockpit"), &[Topic::Binding])? {
            plan.rule(root.join(".cockpit/registry.json"), LocalWatchMatch::Exact, &[Topic::Binding]);
        }
        Ok((plan.finish(vec![Topic::Binding]), auth.endpoint))
    }

    /// Called before delivering a binding invalidation; identity cannot drift to
    /// a new Herdr boot even when session and Space IDs are reused.
    pub(crate) async fn reauthorize_binding_watch(&self, session_id: &str, space_id: &str, endpoint: &str) -> Result<(), InspectionError> {
        if self.authorize(session_id, space_id).await?.endpoint != endpoint {
            return Err(fs::error("stale_identity", "Herdr endpoint changed while watching Notes"));
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::local_changes::{LocalChanges, LocalWatchLease};
    use std::{process::Command, time::Duration};
    use crate::{HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession, project_adapter::*};
    use cockpit_protocol::v1::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Adapter { boot: AtomicUsize }
    fn unused<T>() -> Result<T, InspectionError> { Err(fs::error("unused", "unexpected adapter call")) }
    #[async_trait::async_trait]
    impl HerdrAdapter for Adapter {
        async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> { unused() }
        async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> { unused() }
        async fn sessions(&self) -> Result<SessionListResponse, InspectionError> { unused() }
        async fn session_snapshot(&self, session: &str) -> Result<SessionSnapshotResponse, InspectionError> {
            Ok(SessionSnapshotResponse { session_id: session.into(), server_instance: "0123456789abcdef".into(), version: "test".into(), protocol: 1,
                focused_space_id: None, focused_tab_id: None, focused_pane_id: None, herdr_shell: None,
                spaces: vec![SpaceSummary { id: "space".into(), label: "Test".into(), number: 1, tab_count: 0, pane_count: 0, focused: false, agent_status: "none".into(), git: None }],
                tabs: vec![], panes: vec![], agents: vec![] })
        }
        async fn focus(&self, _: &str, _: &FocusRequest) -> Result<FocusResponse, InspectionError> { unused() }
        async fn mutate(&self, _: &str, _: &ResourceMutationRequest) -> Result<ResourceMutationResponse, InspectionError> { unused() }
        async fn subscribe_session(&self, _: &str, _: &SessionSnapshotResponse) -> Result<SessionSubscription, InspectionError> { unused() }
        async fn open_terminal(&self, _: &TerminalOpenRequest) -> Result<TerminalSession, InspectionError> { unused() }
    }
    #[async_trait::async_trait]
    impl ProjectHerdrAdapter for Adapter {
        async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> { Ok(format!("boot:{}", self.boot.load(Ordering::SeqCst))) }
        async fn project_inventory(&self, _: &str, _: &str) -> Result<ProjectInventory, InspectionError> { unused() }
        async fn project_worktree(&self, _: &str, _: &ProjectWorktreeRequest) -> Result<ProjectWorktreeResult, InspectionError> { unused() }
        async fn project_worktree_dirty(&self, _: &str, _: u32, _: u32) -> Result<crate::project_adapter::WorktreeStatus, InspectionError> { unused() }
        async fn project_close_workspace(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> { unused() }
        async fn project_remove_worktree(&self, _: &str, _: &ProjectWorktreeRemoveRequest) -> Result<(), InspectionError> { unused() }
    }

    struct Fixture { root: PathBuf, id: String }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("notes-watch-{}", uuid::Uuid::new_v4()));
            let id = uuid::Uuid::new_v4().to_string();
            std::fs::create_dir_all(root.join(&id)).unwrap();
            Self { root, id }
        }
        fn service(&self) -> NotesService { NotesService::new(self.root.clone()) }
        fn base(&self) -> PathBuf { self.root.join(&self.id) }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
    }
    async fn changed(lease: &LocalWatchLease, checkpoint: &crate::local_changes::LocalWatchSnapshot, topic: Topic) {
        let mut receiver = lease.receiver();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snapshot = lease.take_since(checkpoint);
                assert!(snapshot.unavailable.is_none(), "unexpected observer failure");
                if snapshot.topics.contains(&topic) { break; }
                receiver.changed().await.unwrap();
            }
        }).await.expect("external mutation did not wake the watch");
    }

    #[tokio::test]
    async fn external_atomic_save_and_comment_write_wake_pinned_uuid() {
        let f = Fixture::new();
        let base = f.base();
        std::fs::create_dir_all(base.join("comments/task_a")).unwrap();
        std::fs::create_dir_all(f.root.join(".cockpit")).unwrap();
        std::fs::write(f.root.join(".cockpit/registry.json"), "not valid registry").unwrap();
        std::fs::write(base.join("scratchpad.md"), "first").unwrap();
        let lease = LocalChanges::default().subscribe(f.service().content_watch_plan(&f.id).unwrap()).unwrap();
        let checkpoint = lease.checkpoint();
        // Preserve size and mtime: metadata freshness tokens must not suppress the wake.
        assert!(Command::new("sh").arg("-c").arg("printf other > \"$1.tmp\" && touch -r \"$1\" \"$1.tmp\" && mv \"$1.tmp\" \"$1\"")
            .arg("editor").arg(base.join("scratchpad.md")).status().unwrap().success());
        changed(&lease, &checkpoint, Topic::Scratchpad).await;
        assert_eq!(std::fs::read_to_string(base.join("scratchpad.md")).unwrap(), "other");
        let checkpoint = lease.checkpoint();
        let comment = base.join("comments/task_a").join(format!("{}.md", uuid::Uuid::new_v4()));
        assert!(Command::new("sh").arg("-c").arg("printf body > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"")
            .arg("editor").arg(comment).status().unwrap().success());
        changed(&lease, &checkpoint, Topic::Comments).await;
    }

    #[tokio::test]
    async fn missing_root_is_not_created_and_creation_invalidates_epoch() {
        let f = Fixture::new();
        let absent = f.root.join("missing");
        let plan = NotesService::new(absent.clone()).content_watch_plan(&f.id).unwrap();
        let lease = LocalChanges::default().subscribe(plan).unwrap();
        assert!(!absent.exists());
        let mut receiver = lease.receiver();
        std::fs::create_dir(&absent).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if receiver.borrow_and_update().unavailable.is_some() { break; }
                receiver.changed().await.unwrap();
            }
        }).await.unwrap();
        assert!(lease.revalidate().is_err());
        assert!(!absent.join(&f.id).exists());
    }

    #[test]
    fn thread_creation_between_discovery_and_registration_fails_closed() {
        let f = Fixture::new();
        std::fs::create_dir(f.base().join("comments")).unwrap();
        let plan = f.service().content_watch_plan(&f.id).unwrap();
        std::fs::create_dir(f.base().join("comments/task_a")).unwrap();
        assert!(LocalChanges::default().subscribe(plan).is_err());
    }

    #[test]
    fn replaced_or_symlinked_directory_is_never_adopted() {
        let f = Fixture::new();
        let plan = f.service().content_watch_plan(&f.id).unwrap();
        let moved = f.root.join("old");
        std::fs::rename(f.base(), &moved).unwrap();
        std::os::unix::fs::symlink(&moved, f.base()).unwrap();
        assert!(LocalChanges::default().subscribe(plan).is_err());
        assert!(f.service().content_watch_plan(&f.id).is_err());
    }

    #[tokio::test]
    async fn binding_registry_replacement_wakes_and_boot_change_revokes() {
        let f = Fixture::new();
        std::fs::create_dir(f.root.join(".cockpit")).unwrap();
        let adapter = Arc::new(Adapter { boot: AtomicUsize::new(1) });
        let service = f.service().with_herdr(adapter.clone());
        let (plan, endpoint) = service.binding_watch_plan("session", "space").await.unwrap();
        let lease = LocalChanges::default().subscribe(plan).unwrap();
        let checkpoint = lease.checkpoint();
        let registry = f.root.join(".cockpit/registry.json");
        assert!(Command::new("sh").arg("-c").arg("printf '{}' > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"")
            .arg("writer").arg(registry).status().unwrap().success());
        changed(&lease, &checkpoint, Topic::Binding).await;
        service.reauthorize_binding_watch("session", "space", &endpoint).await.unwrap();
        adapter.boot.store(2, Ordering::SeqCst);
        assert_eq!(service.reauthorize_binding_watch("session", "space", &endpoint).await.unwrap_err().code, "stale_identity");
        assert_eq!(service.binding_watch_plan("session", "other").await.err().unwrap().code, "notes_space_unavailable");
    }

    #[tokio::test]
    async fn association_transfer_notifies_existing_space() {
        let f = Fixture::new();
        let (_, root) = fs::root(&f.root, false).unwrap();
        let old = super::super::registry::Authority {
            endpoint: "boot:1".into(), session_id: "session".into(),
            space_id: "space".into(), label: "Test".into(),
        };
        let (id, _) = super::super::registry::bind(&root, &old, None).unwrap();
        let service = f.service().with_herdr(Arc::new(Adapter { boot: AtomicUsize::new(1) }));
        let (plan, _) = service.binding_watch_plan("session", "space").await.unwrap();
        let lease = LocalChanges::default().subscribe(plan).unwrap();
        let checkpoint = lease.checkpoint();
        let new = super::super::registry::Authority { space_id: "other".into(), ..old.clone() };
        super::super::registry::bind(&root, &new, Some(&id)).unwrap();
        changed(&lease, &checkpoint, Topic::Binding).await;
        assert_eq!(super::super::registry::resolve(&root, &old).unwrap_err().code, "notes_unbound");
        assert_eq!(super::super::registry::resolve(&root, &new).unwrap(), id);
    }

    #[test]
    fn invalid_uuid_and_unavailable_herdr_are_refused() {
        let f = Fixture::new();
        assert_eq!(f.service().content_watch_plan("../other").err().unwrap().code, "notes_invalid_target");
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let error = runtime.block_on(f.service().binding_watch_plan("session", "space")).err().unwrap();
        assert_eq!(error.code, "notes_space_unavailable");
    }
}
