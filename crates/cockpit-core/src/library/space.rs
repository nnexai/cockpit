use super::{LibraryService, operations, refs, store::{Store, SpaceContextRecord, error}};
use crate::{InspectionError, project_store::open_dir_nofollow_absolute, repositories::RepositoryCatalog};
use cockpit_protocol::{library::*, projects::ProjectDiagnostic};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path};

const MAX_ITEMS: usize = 5_000;
const MAX_REPOSITORIES: usize = 64;

#[derive(Clone)]
struct AuthorizedSpace {
    context_id: String,
    label: String,
    checkout_path: Option<String>,
}

pub(super) fn context_id(endpoint: &str, target: &SpaceTarget) -> String {
    let mut hash = Sha256::new();
    hash.update(endpoint.as_bytes());
    hash.update([0]);
    hash.update(target.session_id.as_bytes());
    hash.update([0]);
    hash.update(target.space_id.as_bytes());
    format!("space:{:x}", hash.finalize())
}
fn bounded_ids(ids: &[String]) -> Result<Vec<String>, InspectionError> {
    if ids.len() > MAX_ITEMS || ids.iter().any(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control)) {
        return Err(error("invalid_library_request", "Space selection must contain at most 5000 bounded item IDs"));
    }
    Ok(ids.iter().cloned().collect::<BTreeSet<_>>().into_iter().collect())
}
fn diagnostic(path: &str) -> ProjectDiagnostic {
    ProjectDiagnostic { code: "space_repository_unavailable".into(), message: "Selected repository is missing or no longer a configured no-follow Git checkout".into(), path: Some(path.into()) }
}

impl LibraryService {
    async fn authorize_space(&self, target: &SpaceTarget) -> Result<AuthorizedSpace, InspectionError> {
        if target.session_id.is_empty() || target.space_id.is_empty() || target.session_id.len() > 256 || target.space_id.len() > 256
            || target.session_id.chars().any(char::is_control) || target.space_id.chars().any(char::is_control) {
            return Err(error("invalid_library_request", "Invalid Space target"));
        }
        let adapter = self.herdr.as_ref().ok_or_else(|| error("space_context_unavailable", "Herdr is unavailable"))?;
        let endpoint = adapter.project_endpoint_identity(&target.session_id).await?;
        if endpoint.is_empty() { return Err(error("stale_identity", "Herdr endpoint identity is missing")); }
        let snapshot = adapter.session_snapshot(&target.session_id).await?;
        if snapshot.session_id != target.session_id { return Err(error("stale_identity", "Herdr returned a different session")); }
        let space = snapshot.spaces.iter().find(|space| space.id == target.space_id)
            .ok_or_else(|| error("space_context_unavailable", "Space is absent from the fresh Herdr snapshot"))?;
        if adapter.project_endpoint_identity(&target.session_id).await? != endpoint {
            return Err(error("stale_identity", "Herdr endpoint changed while resolving Space context"));
        }
        Ok(AuthorizedSpace { context_id: context_id(&endpoint, target), label: space.label.clone(), checkout_path: space.git.as_ref().map(|git| git.checkout_path.clone()) })
    }

    async fn validate_repositories(&self, paths: &[String]) -> Result<Vec<String>, InspectionError> {
        if paths.len() > MAX_REPOSITORIES || paths.iter().any(|path| path.is_empty() || path.len() > 4096 || path.chars().any(char::is_control)) {
            return Err(error("invalid_library_request", "Select at most 64 bounded repository paths"));
        }
        let paths = paths.iter().cloned().collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
        if paths.is_empty() { return Ok(paths); }
        let catalog = RepositoryCatalog::new(self.configuration.clone());
        let listed = catalog.list().await?;
        for path in &paths {
            let candidate = listed.repositories.iter().find(|candidate| &candidate.checkout_path == path)
                .ok_or_else(|| error("repository_not_configured", "Selected path is not in the configured repository catalog"))?;
            open_dir_nofollow_absolute(Path::new(path)).map_err(|_| error("repository_not_configured", "Selected repository path is unsafe or missing"))?;
            let fresh = catalog.discover_checkout(Path::new(path)).await?;
            if fresh.checkout_path != *path || fresh.repository_id != candidate.repository_id {
                return Err(error("repository_identity_stale", "Selected repository identity changed"));
            }
        }
        Ok(paths)
    }

    async fn mutate_space(&self, target: &SpaceTarget, add: &[String], remove: &[String], repositories: Option<Vec<String>>) -> Result<(), InspectionError> {
        let authorized = self.authorize_space(target).await?;
        let store = self.open()?;
        // Never hold a blocking filesystem lock across an adapter await. If a
        // host is busy, wait off-runtime, then resolve fresh authority again.
        let (fresh, lock) = loop {
            if let Some(paths) = &repositories { self.validate_repositories(paths).await?; }
            let fresh = self.authorize_space(target).await?;
            if fresh.context_id != authorized.context_id {
                return Err(error("stale_identity", "Space endpoint changed while waiting to select context"));
            }
            if let Some(lock) = store.try_exclusive()? { break (fresh, lock); }
            let waiting_store = store.clone();
            tokio::task::spawn_blocking(move || {
                let lock = waiting_store.exclusive()?;
                drop(lock);
                Ok::<_, InspectionError>(())
            }).await.map_err(|e| error("library_unavailable", e.to_string()))??;
        };
        if add.is_empty() && remove.is_empty() && repositories.is_none() {
            drop(lock);
            return Ok(());
        }
        store.mutate_index_locked(|index| {
            if !add.is_empty() && index.items.iter().filter(|entry| add.binary_search(&entry.summary.item_id).is_ok()).count() != add.len() {
                return Err(error("library_item_not_found", "Selected Library item no longer exists"));
            }
            let at = match index.space_contexts.iter().position(|context| context.space_context_id == fresh.context_id) {
                Some(at) => at,
                None => {
                    index.space_contexts.push(SpaceContextRecord { space_context_id: fresh.context_id.clone(), session_id: target.session_id.clone(), space_id: target.space_id.clone(), item_ids: vec![], repository_paths: vec![], legacy_migrated: true });
                    index.space_contexts.len() - 1
                }
            };
            let context = &mut index.space_contexts[at];
            context.item_ids.retain(|id| remove.binary_search(id).is_err());
            context.item_ids.extend(add.iter().cloned());
            context.item_ids.sort();
            context.item_ids.dedup();
            if context.item_ids.len() > MAX_ITEMS { return Err(error("invalid_library_request", "Space selection exceeds 5000 items")); }
            if let Some(paths) = repositories { context.repository_paths = paths; }
            let reference = LibraryItemRef::Space { space_context_id: fresh.context_id.clone() };
            for entry in &mut index.items {
                if add.binary_search(&entry.summary.item_id).is_ok() { refs::insert_ref(&mut entry.summary, reference.clone()); }
                else if remove.binary_search(&entry.summary.item_id).is_ok() {
                    refs::remove_ref(&mut entry.summary, &reference);
                    if entry.summary.refs.is_empty() {
                        entry.summary.purge_after = Some((crate::project_store::timestamp().parse::<u128>().unwrap_or(0) + refs::TOMBSTONE_GRACE_MS).to_string());
                    }
                }
            }
            Ok(())
        })?;
        drop(lock);
        Ok(())
    }

    pub async fn space_listing(&self, target: &SpaceTarget) -> Result<SpaceContextListing, InspectionError> {
        let authorized = self.authorize_space(target).await?;
        let store = self.open()?;
        let (items, repository_paths) = {
            let _lock = store.shared()?;
            let index = store.index_shared()?;
            match index.space_contexts.iter().find(|context| context.space_context_id == authorized.context_id) {
                Some(context) => {
                    let items = index.items.iter().filter(|entry| context.item_ids.binary_search(&entry.summary.item_id).is_ok()).map(|entry| entry.summary.clone()).collect();
                    (items, context.repository_paths.clone())
                }
                None => (vec![], vec![]),
            }
        };
        let mut diagnostics = vec![];
        if !repository_paths.is_empty() {
            let catalog = RepositoryCatalog::new(self.configuration.clone());
            let listed = catalog.list().await?;
            for path in &repository_paths {
                if !listed.repositories.iter().any(|candidate| candidate.checkout_path == *path)
                    || open_dir_nofollow_absolute(Path::new(path)).is_err()
                    || catalog.discover_checkout(Path::new(path)).await.is_err() {
                    diagnostics.push(diagnostic(path));
                }
            }
        }
        let current = self.authorize_space(target).await?;
        if current.context_id != authorized.context_id {
            return Err(error("stale_identity", "Space endpoint changed while reading context"));
        }
        Ok(SpaceContextListing { target: target.clone(), space_label: current.label, library_root: self.root_path().into(), checkout_path: current.checkout_path, items, repository_paths, diagnostics })
    }

    pub async fn start_space_add(&self, request: SpaceAddRequest) -> Result<LibraryOperation, InspectionError> {
        let ids = bounded_ids(&request.item_ids)?;
        self.authorize_space(&request.target).await?;
        let handle = operations::runtime()?;
        let store = self.open()?;
        let (record, lease) = operations::create(&store, LibraryOperationKind::SpaceAdd, Some(ids.len() as u32))?;
        let record = operations::set_target(&store, &record.operation_id, request.target.clone())?;
        let id = record.operation_id.clone();
        let service = self.clone();
        let worker_store = store.clone();
        operations::spawn(handle, store, id.clone(), lease, async move {
            service.select_saved_items(&worker_store, &id, &request.target, &ids).await
        });
        Ok(record)
    }
    pub async fn space_remove(&self, request: SpaceRemoveRequest) -> Result<SpaceContextListing, InspectionError> {
        let ids = bounded_ids(&request.item_ids)?;
        self.mutate_space(&request.target, &[], &ids, None).await?;
        self.space_listing(&request.target).await
    }
    pub async fn space_repositories(&self, request: SpaceRepositoriesRequest) -> Result<SpaceContextListing, InspectionError> {
        self.authorize_space(&request.target).await?;
        let paths = self.validate_repositories(&request.repository_paths).await?;
        self.mutate_space(&request.target, &[], &[], Some(paths)).await?;
        self.space_listing(&request.target).await
    }
    pub(super) async fn select_saved_items(&self, store: &Store, operation: &str, target: &SpaceTarget, ids: &[String]) -> Result<(), InspectionError> {
        let ids = bounded_ids(ids)?;
        operations::begin_space(store, operation, ids.len() as u32)?;
        if operations::cancelled(store, operation)? { return Ok(()); }
        self.mutate_space(target, &ids, &[], None).await?;
        operations::space_result(store, operation, SpacePhaseResult { space_id: target.space_id.clone(), item_ids: ids.clone() }, ids.len() as u32)
    }
}

#[cfg(test)]
pub(in crate::library) mod tests {
    use super::*;
    use crate::{HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession, project_adapter::*};
    use cockpit_protocol::v1::*;
    use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
    use super::super::tests::{fixture, add, finished, reopen};

    pub(in crate::library) struct Adapter {
        pub(in crate::library) reachable: AtomicBool,
        present: AtomicBool,
        endpoint: AtomicUsize,
        endpoint_calls: AtomicUsize,
        change_on_call: AtomicUsize,
        space: String,
    }
    pub(in crate::library) fn adapter(space: &str) -> Arc<Adapter> {
        Arc::new(Adapter { reachable: AtomicBool::new(true), present: AtomicBool::new(true), endpoint: AtomicUsize::new(0), endpoint_calls: AtomicUsize::new(0), change_on_call: AtomicUsize::new(usize::MAX), space: space.into() })
    }
    pub(in crate::library) fn target() -> SpaceTarget {
        SpaceTarget { session_id: "session".into(), space_id: "space".into() }
    }
    fn unused<T>() -> Result<T, InspectionError> { Err(error("unused", "Unexpected adapter call")) }
    #[async_trait::async_trait]
    impl HerdrAdapter for Adapter {
        async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> { unused() }
        async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> { unused() }
        async fn sessions(&self) -> Result<SessionListResponse, InspectionError> { unused() }
        async fn session_snapshot(&self, session: &str) -> Result<SessionSnapshotResponse, InspectionError> {
            if !self.reachable.load(Ordering::SeqCst) { return Err(error("disconnected", "Herdr stopped")); }
            Ok(SessionSnapshotResponse { session_id: session.into(), server_instance: "0123456789abcdef".into(), version: "test".into(), protocol: 1,
                focused_space_id: None, focused_tab_id: None, focused_pane_id: None, herdr_shell: None,
                spaces: if self.present.load(Ordering::SeqCst) { vec![SpaceSummary { id: self.space.clone(), label: "Test Space".into(), number: 1, tab_count: 0, pane_count: 0, focused: false, agent_status: "none".into(), git: None }] } else { vec![] },
                tabs: vec![], panes: vec![], agents: vec![] })
        }
        async fn focus(&self, _: &str, _: &FocusRequest) -> Result<FocusResponse, InspectionError> { unused() }
        async fn mutate(&self, _: &str, _: &ResourceMutationRequest) -> Result<ResourceMutationResponse, InspectionError> { unused() }
        async fn subscribe_session(&self, _: &str, _: &SessionSnapshotResponse) -> Result<SessionSubscription, InspectionError> { unused() }
        async fn open_terminal(&self, _: &TerminalOpenRequest) -> Result<TerminalSession, InspectionError> { unused() }
    }
    #[async_trait::async_trait]
    impl ProjectHerdrAdapter for Adapter {
        async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> {
            let call = self.endpoint_calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == self.change_on_call.load(Ordering::SeqCst) { self.endpoint.fetch_add(1, Ordering::SeqCst); }
            Ok(format!("endpoint:{}", self.endpoint.load(Ordering::SeqCst)))
        }
        async fn project_inventory(&self, _: &str, _: &str) -> Result<ProjectInventory, InspectionError> { unused() }
        async fn project_worktree(&self, _: &str, _: &ProjectWorktreeRequest) -> Result<ProjectWorktreeResult, InspectionError> { unused() }
        async fn project_terminal(&self, _: &str, _: &ProjectTerminalRequest) -> Result<ProjectTerminalResult, InspectionError> { unused() }
        async fn project_worktree_dirty(&self, _: &str, _: u32, _: u32) -> Result<bool, InspectionError> { unused() }
        async fn project_close_workspace(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> { unused() }
        async fn project_remove_worktree(&self, _: &str, _: &ProjectWorktreeRemoveRequest) -> Result<(), InspectionError> { unused() }
    }
    async fn select(service: &LibraryService, ids: Vec<String>) -> LibraryOperation {
        finished(service, service.start_space_add(SpaceAddRequest { target: target(), item_ids: ids }).await.unwrap()).await
    }

    #[tokio::test]
    async fn selections_persist_and_keep_saved_items_without_copying_content() {
        let f = fixture();
        let runtime = adapter("space");
        let service = f.service.clone().with_herdr(runtime.clone());
        let store = service.open().unwrap();
        let generation = store.index().unwrap().generation;
        let empty = service.space_listing(&target()).await.unwrap();
        assert!(empty.items.is_empty());
        assert!(empty.repository_paths.is_empty());
        let index = store.index().unwrap();
        assert!(index.space_contexts.is_empty());
        assert_eq!(index.generation, generation);
        finished(&service, service.start_add(add(1)).await.unwrap()).await;
        let item = service.listing(None).await.unwrap().items.remove(0);
        let notes = f.root.join("existing-notes");
        std::fs::create_dir(&notes).unwrap();
        std::fs::write(notes.join("notes.md"), b"keep my notes").unwrap();
        let operation = select(&service, vec![item.item_id.clone()]).await;
        assert_eq!(operation.space.unwrap().item_ids, vec![item.item_id.clone()]);
        let reopened = reopen(&f).with_herdr(runtime);
        let listing = reopened.space_listing(&target()).await.unwrap();
        assert_eq!(listing.space_label, "Test Space");
        assert_eq!(listing.items[0].item_id, item.item_id);
        assert_eq!(std::fs::read(notes.join("notes.md")).unwrap(), b"keep my notes");
        let store = service.open().unwrap();
        store.mutate_index(|index| {
            let saved = &mut index.items[0].summary;
            refs::remove_ref(saved, &LibraryItemRef::Manual);
            assert!(!saved.refs.is_empty());
            saved.purge_after = Some("0".into());
            Ok(())
        }).unwrap();
        let (sweep, _lease) = operations::create(&store, LibraryOperationKind::Refresh, None).unwrap();
        refs::purge_expired(&store, &sweep.operation_id).unwrap();
        assert_eq!(service.listing(None).await.unwrap().items.len(), 1);
        let removed = service.space_remove(SpaceRemoveRequest { target: target(), item_ids: vec![item.item_id] }).await.unwrap();
        assert!(removed.items.is_empty());
        assert_eq!(service.listing(None).await.unwrap().items.len(), 1);
    }

    #[tokio::test]
    async fn unrelated_concurrent_selections_and_repository_replacement_are_preserved() {
        let f = fixture();
        let service = f.service.clone().with_herdr(adapter("space"));
        finished(&service, service.start_add(add(1)).await.unwrap()).await;
        finished(&service, service.start_add(add(2)).await.unwrap()).await;
        let ids = service.listing(None).await.unwrap().items.iter().map(|item| item.item_id.clone()).collect::<Vec<_>>();
        assert!(ids.len() >= 2);
        let (a, b) = tokio::join!(select(&service, vec![ids[0].clone()]), select(&service, vec![ids[1].clone()]));
        assert!(a.phases.iter().all(|phase| phase.state == LibraryPhaseState::Done));
        assert!(b.phases.iter().all(|phase| phase.state == LibraryPhaseState::Done));
        let listing = service.space_repositories(SpaceRepositoriesRequest { target: target(), repository_paths: vec![] }).await.unwrap();
        assert_eq!(listing.items.len(), 2);
        assert!(service.space_repositories(SpaceRepositoriesRequest { target: target(), repository_paths: vec![f.root.to_string_lossy().into_owned()] }).await.is_err());
        assert_eq!(service.space_listing(&target()).await.unwrap().items.len(), 2);
    }

    #[tokio::test]
    async fn changed_endpoint_and_missing_space_never_reuse_old_selection() {
        let f = fixture();
        let runtime = adapter("space");
        let service = f.service.clone().with_herdr(runtime.clone());
        finished(&service, service.start_add(add(1)).await.unwrap()).await;
        let id = service.listing(None).await.unwrap().items[0].item_id.clone();
        select(&service, vec![id]).await;
        runtime.endpoint.fetch_add(1, Ordering::SeqCst);
        assert!(service.space_listing(&target()).await.unwrap().items.is_empty());
        runtime.present.store(false, Ordering::SeqCst);
        assert_eq!(service.space_listing(&target()).await.unwrap_err().code, "space_context_unavailable");
    }

    #[tokio::test]
    async fn failed_selection_keeps_saved_library_and_rejects_racing_authority() {
        let f = fixture();
        let runtime = adapter("space");
        let service = f.service.clone().with_herdr(runtime.clone());
        runtime.reachable.store(false, Ordering::SeqCst);
        let mut request = add(1);
        request.target = Some(target());
        let operation = finished(&service, service.start_add(request).await.unwrap()).await;
        assert!(!operation.item_ids.is_empty());
        assert_eq!(operation.phases.last().unwrap().state, LibraryPhaseState::Failed);
        assert_eq!(service.listing(None).await.unwrap().items.len(), 1);
        runtime.reachable.store(true, Ordering::SeqCst);
        runtime.change_on_call.store(runtime.endpoint_calls.load(Ordering::SeqCst) + 2, Ordering::SeqCst);
        assert_eq!(service.space_listing(&target()).await.unwrap_err().code, "stale_identity");
        runtime.change_on_call.store(runtime.endpoint_calls.load(Ordering::SeqCst) + 3, Ordering::SeqCst);
        assert_eq!(service.space_listing(&target()).await.unwrap_err().code, "stale_identity");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mutation_rechecks_endpoint_after_waiting_for_another_host() {
        let f = fixture();
        let runtime = adapter("space");
        let service = f.service.clone().with_herdr(runtime.clone());
        let store = service.open().unwrap();
        let lock = store.exclusive().unwrap();
        let worker = service.clone();
        let mutation = tokio::spawn(async move { worker.mutate_space(&target(), &[], &[], Some(vec![])).await });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while runtime.endpoint_calls.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        runtime.endpoint.fetch_add(1, Ordering::SeqCst);
        drop(lock);
        assert_eq!(mutation.await.unwrap().unwrap_err().code, "stale_identity");
        assert!(service.space_listing(&target()).await.unwrap().items.is_empty());
    }
}

#[cfg(test)]
mod repository_tests {
    use super::*;
    use super::tests::{adapter, target};
    use super::super::tests::{fixture, add, finished};

    #[tokio::test]
    async fn repository_paths_persist_deduplicate_and_missing_paths_stay_diagnostic() {
        let mut f = fixture();
        let repos = f.root.join("repos");
        let repo = repos.join("selected");
        std::fs::create_dir_all(&repo).unwrap();
        assert!(std::process::Command::new("git").args(["init", "-q"]).arg(&repo).status().unwrap().success());
        f.service.configuration.repository_roots = vec![repos.to_string_lossy().into_owned()];
        let runtime = adapter("space");
        let service = f.service.clone().with_herdr(runtime.clone());
        finished(&service, service.start_add(add(1)).await.unwrap()).await;
        let id = service.listing(None).await.unwrap().items[0].item_id.clone();
        let operation = service.start_space_add(SpaceAddRequest { target: target(), item_ids: vec![id.clone()] }).await.unwrap();
        finished(&service, operation).await;
        let path = repo.to_string_lossy().into_owned();
        let listing = service.space_repositories(SpaceRepositoriesRequest { target: target(), repository_paths: vec![path.clone(), path.clone()] }).await.unwrap();
        assert_eq!(listing.repository_paths, vec![path.clone()]);
        assert_eq!(listing.items[0].item_id, id);
        assert!(listing.diagnostics.is_empty());
        #[cfg(unix)] {
            let alias = repos.join("alias");
            std::os::unix::fs::symlink(&repo, &alias).unwrap();
            assert!(service.space_repositories(SpaceRepositoriesRequest { target: target(), repository_paths: vec![alias.to_string_lossy().into_owned()] }).await.is_err());
        }
        std::fs::rename(&repo, repos.join("moved")).unwrap();
        let restarted = LibraryService::new(service.configuration.clone(), service.sources.clone()).with_herdr(runtime);
        let missing = restarted.space_listing(&target()).await.unwrap();
        assert_eq!(missing.repository_paths, vec![path.clone()]);
        assert_eq!(missing.diagnostics[0].path.as_deref(), Some(path.as_str()));
        assert_eq!(missing.items[0].item_id, id);
        let unselected = restarted.space_remove(SpaceRemoveRequest { target: target(), item_ids: vec![id] }).await.unwrap();
        assert_eq!(unselected.repository_paths, vec![path]);
    }
}
