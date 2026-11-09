//! Read-only filesystem observation for Library listing and follow status.
//!
//! Directory lifecycle events invalidate the observation epoch: a fresh plan
//! must be installed before the replacement can be observed safely.
use super::{LibraryService, error};
use crate::{
    InspectionError,
    local_changes::{LocalWatchMatch, LocalWatchPlan, LocalWatchRule},
    project_store::open_dir_nofollow_absolute,
};
use cockpit_protocol::local_changes::LocalChangeTopic;
use std::{path::{Component, Path, PathBuf}, sync::Arc};

#[cfg(unix)]
fn identity(path: &Path) -> Result<(u64, u64), String> {
    use cap_std::fs::MetadataExt;
    let dir = open_dir_nofollow_absolute(path).map_err(|e| e.to_string())?;
    let metadata = dir.dir_metadata().map_err(|e| e.to_string())?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn identity(_path: &Path) -> Result<(u64, u64), String> {
    Err("Library directory identity cannot be verified on this platform".into())
}

fn absolute_root(path: &Path) -> Result<PathBuf, InspectionError> {
    let raw = if path.is_absolute() { path.to_path_buf() } else {
        std::env::current_dir().map_err(|e| error("library_unavailable", e.to_string()))?.join(path)
    };
    let mut result = PathBuf::new();
    for component in raw.components() {
        match component {
            Component::RootDir | Component::Normal(_) => result.push(component.as_os_str()),
            Component::CurDir => {},
            _ => return Err(error("unsafe_path", "unsafe Library root path")),
        }
    }
    Ok(result)
}

impl LibraryService {
    /// Construct a validated plan without opening, recovering, or creating the store.
    pub(crate) fn listing_watch_plan(&self) -> Result<LocalWatchPlan, InspectionError> {
        let root = absolute_root(Path::new(self.root_path()))?;
        let meta = root.join(".cockpit");
        let sync = meta.join("sync");
        let mut directories = Vec::new();
        // An already-open store is descriptor-pinned. Never arm a watch on a
        // replacement path while reads still use that older descriptor.
        #[cfg(unix)]
        if let Some(store) = self.store.get() {
            use cap_std::fs::MetadataExt;
            let metadata = store.root.dir_metadata()
                .map_err(|e| error("library_unavailable", e.to_string()))?;
            let actual = identity(&root).map_err(|e| error("library_unavailable", e))?;
            if actual != (metadata.dev(), metadata.ino()) {
                return Err(error("library_unavailable", "Library root identity changed"));
            }
        }
        let mut pinned = Vec::new();
        let mut rules = Vec::new();
        let mut current = PathBuf::new();
        let mut missing = None;
        // Include every existing ancestor, so replacing any part of the path
        // cannot silently retarget an otherwise valid-looking subscription.
        for component in sync.components() {
            current.push(component.as_os_str());
            if missing.is_some() { break; }
            match identity(&current) {
                Ok(id) => {
                    if current.parent().is_some() {
                        rules.push(LocalWatchRule {
                            path: current.clone(), match_kind: LocalWatchMatch::Lifecycle,
                            topics: vec![LocalChangeTopic::Listing, LocalChangeTopic::FollowStatus],
                        });
                    }
                    pinned.push((current.clone(), id));
                    directories.push(current.clone());
                }
                Err(message) => {
                    // Only a genuinely absent component may be observed from its
                    // parent. Symlinks, permissions and non-directories fail closed.
                    match std::fs::symlink_metadata(&current) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            missing = Some(current.clone());
                            rules.push(LocalWatchRule {
                                path: current.clone(), match_kind: LocalWatchMatch::Lifecycle,
                                topics: vec![LocalChangeTopic::Listing, LocalChangeTopic::FollowStatus],
                            });
                        }
                        _ => return Err(error("library_unavailable", message)),
                    }
                }
            }
        }
        rules.push(LocalWatchRule {
            path: meta.join("index.json"), match_kind: LocalWatchMatch::Exact,
            topics: vec![LocalChangeTopic::Listing],
        });
        rules.push(LocalWatchRule {
            path: sync.join("state.json"), match_kind: LocalWatchMatch::Exact,
            topics: vec![LocalChangeTopic::FollowStatus],
        });
        let store = self.store.clone();
        let revalidate = Arc::new(move || {
            #[cfg(unix)]
            if let Some(store) = store.get() {
                use cap_std::fs::MetadataExt;
                let metadata = store.root.dir_metadata().map_err(|e| e.to_string())?;
                if identity(&root)? != (metadata.dev(), metadata.ino()) {
                    return Err("Library root identity changed".into());
                }
            }
            for (path, expected) in &pinned {
                if identity(path)? != *expected {
                    return Err("Library directory identity changed".into());
                }
            }
            if let Some(path) = &missing {
                match std::fs::symlink_metadata(path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                    _ => return Err("Library directory lifecycle changed".into()),
                }
            }
            Ok(())
        });
        Ok(LocalWatchPlan::new(directories, rules,
            vec![LocalChangeTopic::Listing, LocalChangeTopic::FollowStatus], revalidate))
    }

    /// Reopen only the configured, descriptor-verified Library root; unlike
    /// Store::open this never initializes or recovers persistent state.
    pub(crate) fn authorized_watch_root(&self, root_id: &str) -> Result<crate::context::AuthorizedRoot, InspectionError> {
        let path = absolute_root(Path::new(self.root_path()))?;
        let dir = open_dir_nofollow_absolute(&path)
            .map_err(|e| error("library_unavailable", e.to_string()))?;
        #[cfg(unix)]
        if let Some(store) = self.store.get() {
            use cap_std::fs::MetadataExt;
            let actual = dir.dir_metadata().map_err(|e| error("library_unavailable", e.to_string()))?;
            let pinned = store.root.dir_metadata().map_err(|e| error("library_unavailable", e.to_string()))?;
            if (actual.dev(), actual.ino()) != (pinned.dev(), pinned.ino()) {
                return Err(error("library_unavailable", "Library root identity changed"));
            }
        }
        #[cfg(not(unix))]
        return Err(error("library_unavailable", "Library directory identity cannot be verified on this platform"));
        #[cfg(unix)]
        {
            let root = crate::context::AuthorizedRoot::library(path, dir, self.configuration.limits.context_tree_depth)?;
            if root.root_id() != root_id {
                return Err(error("context_stale_root", "Library root identity changed"));
            }
            Ok(root)
        }
    }

    pub(crate) async fn watch_index(&self, changes: &crate::local_changes::LocalChanges, root_id: &str) -> Result<crate::local_changes::LocalWatchLease, InspectionError> {
        let permit = changes.discovery_permit().await;
        let service = self.clone();
        let changes = changes.clone();
        let root_id = root_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let root = service.authorized_watch_root(&root_id)?;
            crate::context::index_watch(&changes, root)
        }).await.map_err(|e| error("library_unavailable", e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::fixture;
    use crate::local_changes::{LocalChanges, LocalWatchLease, LocalWatchSnapshot};
    use std::time::Duration;

    async fn changed(lease: &LocalWatchLease, checkpoint: &LocalWatchSnapshot) -> LocalWatchSnapshot {
        let mut rx = lease.receiver();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let result = lease.take_since(checkpoint);
                if result.unavailable.is_some() || !result.topics.is_empty() { return result; }
                rx.changed().await.unwrap();
            }
        }).await.expect("Library watch did not observe the mutation")
    }

    #[tokio::test]
    async fn external_index_replacement_invalidates_listing() {
        let fixture = fixture();
        let store = fixture.service.open().unwrap();
        let sync = store.path.join(".cockpit/sync");
        std::fs::create_dir(&sync).unwrap();
        let lease = LocalChanges::default().subscribe(fixture.service.listing_watch_plan().unwrap()).unwrap();
        let checkpoint = lease.checkpoint();
        let other = LibraryService::new(fixture.service.configuration.clone(), fixture.service.sources.clone());
        let external = other.open().unwrap();
        external.mutate_index(|_| Ok(())).unwrap();
        let event = changed(&lease, &checkpoint).await;
        assert_eq!(event.topics, vec![LocalChangeTopic::Listing]);
        assert!(event.unavailable.is_none());

    }

    #[tokio::test]
    async fn child_process_sync_only_change_invalidates_follow_status() {
        let fixture = fixture();
        let store = fixture.service.open().unwrap();
        let sync = store.path.join(".cockpit/sync");
        std::fs::create_dir(&sync).unwrap();
        let generation = store.index().unwrap().generation;
        let lease = LocalChanges::default().subscribe(fixture.service.listing_watch_plan().unwrap()).unwrap();
        let checkpoint = lease.checkpoint();
        let status = std::process::Command::new("sh")
            .arg("-c").arg("printf '{}' > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"")
            .arg("library-state-writer").arg(sync.join("state.json")).status().unwrap();
        assert!(status.success());
        let event = changed(&lease, &checkpoint).await;
        assert_eq!(event.topics, vec![LocalChangeTopic::FollowStatus]);
        assert_eq!(store.index().unwrap().generation, generation);
    }

    #[tokio::test]
    async fn reading_and_lock_noise_do_not_invalidate() {
        let fixture = fixture();
        let store = fixture.service.open().unwrap();
        std::fs::create_dir(store.path.join(".cockpit/sync")).unwrap();
        let lease = LocalChanges::default().subscribe(fixture.service.listing_watch_plan().unwrap()).unwrap();
        let checkpoint = lease.checkpoint();
        let mut rx = lease.receiver();
        { let _guard = store.shared().unwrap(); store.index_shared().unwrap(); }
        std::fs::write(store.path.join(".cockpit/temporary-file"), b"ignored").unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(250), rx.changed()).await.is_err());
        assert!(lease.take_since(&checkpoint).topics.is_empty());
    }

    #[tokio::test]
    async fn missing_sync_directory_and_root_replacement_end_the_epoch() {
        let fixture = fixture();
        let store = fixture.service.open().unwrap();
        let observer = LocalChanges::default();
        let lease = observer.subscribe(fixture.service.listing_watch_plan().unwrap()).unwrap();
        let checkpoint = lease.checkpoint();
        std::fs::create_dir(store.path.join(".cockpit/sync")).unwrap();
        assert!(changed(&lease, &checkpoint).await.unavailable.is_some());
        drop(lease);
        let lease = observer.subscribe(fixture.service.listing_watch_plan().unwrap()).unwrap();
        let checkpoint = lease.checkpoint();
        std::fs::rename(&store.path, store.path.with_extension("previous")).unwrap();
        std::fs::create_dir(&store.path).unwrap();
        assert!(changed(&lease, &checkpoint).await.unavailable.is_some());
        assert!(lease.revalidate().is_err());
    }

    #[test]
    fn missing_components_are_not_created_by_admission() {
        let fixture = fixture();
        let root = PathBuf::from(fixture.service.root_path());
        assert!(!root.exists());
        fixture.service.listing_watch_plan().unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn existing_store_cannot_be_retargeted_after_root_replacement() {
        let fixture = fixture();
        let store = fixture.service.open().unwrap();
        let root = store.path.clone();
        let old = root.with_extension("old-library");
        std::fs::rename(&root, &old).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(fixture.service.listing_watch_plan().is_err());
    }

    #[test]
    fn standalone_index_authority_is_pinned_and_read_only() {
        let fixture = fixture();
        let root_path = PathBuf::from(fixture.service.root_path());
        assert!(fixture.service.authorized_watch_root("library:missing").is_err());
        assert!(!root_path.exists());
        let store = fixture.service.open().unwrap();
        let authorized = fixture.service.authorized_root(&store).unwrap();
        let root_id = authorized.root_id().to_owned();
        assert!(fixture.service.authorized_watch_root(&root_id).is_ok());
        assert!(fixture.service.authorized_watch_root("library:wrong").is_err());
        std::fs::rename(&root_path, root_path.with_extension("old-index")).unwrap();
        std::fs::create_dir(&root_path).unwrap();
        assert!(fixture.service.authorized_watch_root(&root_id).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_component_is_not_accepted_as_a_missing_directory() {
        use std::os::unix::fs::symlink;
        let fixture = fixture();
        let root = PathBuf::from(fixture.service.root_path());
        symlink(fixture.root.join("missing-target"), &root).unwrap();
        assert!(fixture.service.listing_watch_plan().is_err());
    }
}
