use std::{path::Path, sync::{Arc, Mutex as StdMutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

use cockpit_protocol::projects::{RepositoryCandidate, RepositoryListResponse};
use tokio::sync::Mutex;

use crate::{InspectionError, repositories::RepositoryCatalog};

struct RefreshFlag(Arc<AtomicBool>);

impl Drop for RefreshFlag {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct Snapshot {
    listed: RepositoryListResponse,
    generation: u64,
    updated: Instant,
}

/// Read-only authorization hint. Mutating setup and teardown paths never use it.
pub(crate) struct RepositoryDiscoveryCache {
    state: StdMutex<Option<Snapshot>>,
    // (per-directory discovery is intentionally not cached; see `discover`)
    refill: Mutex<()>,
    refreshing: Arc<AtomicBool>,
    fresh: Duration,
    stale_max: Duration,
}

impl RepositoryDiscoveryCache {
    pub(crate) fn new(fresh: Duration, stale_max: Duration) -> Self {
        Self { state: StdMutex::new(None), refill: Mutex::new(()), refreshing: Arc::new(AtomicBool::new(false)), fresh, stale_max }
    }

    pub(crate) fn publish(&self, listed: RepositoryListResponse, generation: u64) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.as_ref().is_some_and(|current| current.generation > generation) {
            return;
        }
        *state = Some(Snapshot { listed, generation, updated: Instant::now() });

    }

    pub(crate) async fn list(self: &std::sync::Arc<Self>, catalog: &RepositoryCatalog, generation: u64) -> Result<RepositoryListResponse, InspectionError> {
        if let Some(snapshot) = self.state.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
            && snapshot.generation == generation && snapshot.updated.elapsed() < self.fresh {
            return Ok(snapshot.listed.clone());
        }
        if let Some(snapshot) = self.state.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
            && snapshot.generation == generation && snapshot.updated.elapsed() < self.stale_max {
            let stale = snapshot.listed.clone();
            self.refresh_in_background(catalog, generation);
            return Ok(stale);
        }
        let _single = self.refill.lock().await;
        if let Some(snapshot) = self.state.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            if snapshot.generation == generation && snapshot.updated.elapsed() < self.fresh {
                return Ok(snapshot.listed.clone());
            }
            if snapshot.generation == generation && snapshot.updated.elapsed() < self.stale_max {
                let stale = snapshot.listed.clone();
                self.refresh_in_background(catalog, generation);
                return Ok(stale);
            }
        }
        let listed = catalog.list().await?;
        self.publish(listed.clone(), generation);
        Ok(listed)
    }

    fn refresh_in_background(self: &std::sync::Arc<Self>, catalog: &RepositoryCatalog, generation: u64) {
        if self.refreshing.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return;
        }
        let cache = self.clone();
        let catalog = catalog.clone();
        let refreshing = Arc::clone(&self.refreshing);
        tokio::spawn(async move {
            let _refresh = RefreshFlag(refreshing);
            let _single = cache.refill.lock().await;
            if let Some(snapshot) = cache.state.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
                && snapshot.generation == generation && snapshot.updated.elapsed() < cache.fresh {
                return;
            }
            if let Ok(listed) = catalog.list().await {
                cache.publish(listed, generation);
            }
        });
    }

    /// Per-directory discovery is never cached: it decides which checkout a runtime path belongs to, so it must see the
    /// current Git boundary and reject symbolic links in the original path (about 10 ms of git work, unlike the catalog scan).
    pub(crate) async fn discover(&self, catalog: &RepositoryCatalog, cwd: &Path, _generation: u64) -> Result<RepositoryCandidate, InspectionError> {
        catalog.discover_checkout(cwd).await
    }
}

#[cfg(test)]
mod tests {
    use super::RepositoryDiscoveryCache;
    use crate::repositories::RepositoryCatalog;
    use cockpit_protocol::projects::{ProjectConfiguration, ProjectLimits};
    use std::{collections::BTreeMap, path::{Path, PathBuf}, sync::Arc, time::Duration};

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("cockpit-repository-cache-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(std::fs::canonicalize(path).unwrap())
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn git_init(directory: &Path) {
        std::fs::create_dir_all(directory).unwrap();
        let init = std::process::Command::new("git").args(["init", "-q"]).current_dir(directory).output().unwrap();
        assert!(init.status.success(), "git init failed: {}", String::from_utf8_lossy(&init.stderr));
    }

    fn configuration(root: &Path) -> ProjectConfiguration {
        ProjectConfiguration { version: 1, orchestration: Default::default(), repository_roots: vec![root.display().to_string()],
        worktree_root: "worktrees".into(),
        companion_root: "companions".into(),
        state_root: "state".into(),
        cache_root: "cache".into(),
        library_root: "library".into(),
        branch_template: "{repo}/{task_id}".into(),
        checkout_template: "{repo}-{task_id}".into(),
        providers: Vec::new(),
        limits: ProjectLimits {
            catalog_depth: 3,
            catalog_entries: 100,
            git_timeout_ms: 5000,
            git_output_bytes: 1024 * 1024,
            operation_timeout_ms: 5000,
            context_preview_bytes: 1024 * 1024,
            context_preview_lines: 5000,
            context_directory_entries: 1000,
            context_tree_depth: 32,
            library_folder_files: 512,
            library_folder_bytes: 32 * 1024 * 1024,
            library_file_bytes: 4 * 1024 * 1024,
            library_space_pages: 200,
            library_attachment_bytes: 25 * 1024 * 1024,
            library_item_attachment_bytes: 100 * 1024 * 1024,
            library_max_items: 20_000,
        },
        origins: BTreeMap::new(), }
    }

    #[tokio::test]
    async fn regression_discovery_rejects_a_symlink_to_a_checkout_after_the_cache_is_warm() {
        let temp = TempRoot::new();
        let repositories = temp.0.join("repositories");
        let checkout = repositories.join("app");
        git_init(&checkout);
        let link = temp.0.join("app-link");
        std::os::unix::fs::symlink(&checkout, &link).unwrap();
        let catalog = RepositoryCatalog::new(configuration(&repositories));
        let cache = Arc::new(RepositoryDiscoveryCache::new(Duration::from_secs(60), Duration::from_secs(120)));

        let listed = cache.list(&catalog, 1).await.unwrap();
        assert_eq!(listed.repositories.len(), 1);
        assert_eq!(listed.repositories[0].checkout_path, checkout.to_string_lossy());

        let direct = cache.discover(&catalog, &checkout, 1).await.unwrap();
        assert_eq!(direct.checkout_path, checkout.to_string_lossy());
        let error = cache.discover(&catalog, &link, 1).await.unwrap_err();
        assert_eq!(error.code, "repository_checkout_mismatch");
    }

    #[tokio::test]
    async fn regression_discovery_sees_a_nested_git_init_despite_a_warm_cache() {
        let temp = TempRoot::new();
        let repositories = temp.0.join("repositories");
        let parent = repositories.join("parent");
        git_init(&parent);
        let nested = parent.join("packages/nested");
        std::fs::create_dir_all(&nested).unwrap();
        let catalog = RepositoryCatalog::new(configuration(&repositories));
        let cache = Arc::new(RepositoryDiscoveryCache::new(Duration::from_secs(60), Duration::from_secs(120)));

        let listed = cache.list(&catalog, 1).await.unwrap();
        assert_eq!(listed.repositories.len(), 1);
        assert_eq!(listed.repositories[0].checkout_path, parent.to_string_lossy());

        let before = cache.discover(&catalog, &nested, 1).await.unwrap();
        assert_eq!(before.checkout_path, parent.to_string_lossy());

        git_init(&nested);
        let after = cache.discover(&catalog, &nested, 1).await.unwrap();
        assert_eq!(after.checkout_path, nested.to_string_lossy());
        assert_ne!(after.repository_id, before.repository_id);
    }
}
