use std::collections::{BTreeSet, HashSet};
use std::io;
use std::path::Path;
use std::sync::Arc;

use cockpit_protocol::v1::{SpaceTagChangeRequest, SpaceTags, SpaceTagsResponse};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::project_store::{ProjectStore, atomic_write_bytes, read_json_bounded};
use crate::{InspectionError, ProjectHerdrAdapter};

const FILE: &str = "space-tags.json";
const MAX_BYTES: u64 = 1024 * 1024;
const MAX_CATALOG: usize = 256;
const MAX_RECORDS: usize = 4096;

/// Cockpit-owned organisation data. This directory is deliberately outside owner scratch reset.
#[derive(Clone)]
pub struct SpaceTagService {
    store: ProjectStore,
    herdr: Arc<dyn ProjectHerdrAdapter>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredTags {
    schema: u32,
    catalog: Vec<String>,
    spaces: Vec<StoredSpace>,
}

impl Default for StoredTags {
    fn default() -> Self {
        Self { schema: 1, catalog: Vec::new(), spaces: Vec::new() }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSpace {
    session_namespace: String,
    session_id: String,
    space_id: String,
    tags: Vec<String>,
    revision: String,
}

struct ProvenSession {
    session_id: String,
    server_instance: String,
    session_namespace: String,
    spaces: BTreeSet<String>,
}

impl SpaceTagService {
    pub fn new(state_root: &Path, herdr: Arc<dyn ProjectHerdrAdapter>) -> Result<Self, InspectionError> {
        Ok(Self { store: ProjectStore::new(state_root.join("space-tags"))?, herdr })
    }

    async fn prove(&self, session_id: &str) -> Result<ProvenSession, InspectionError> {
        validate_id(session_id)?;
        let session_namespace = self.session_namespace(session_id).await?;
        let endpoint = self.herdr.project_endpoint_identity(session_id).await?;
        let observed = self.herdr.project_snapshot_at_endpoint(session_id, &endpoint).await?;
        if observed.endpoint_identity != endpoint || observed.snapshot.session_id != session_id {
            return Err(InspectionError::new("stale_identity", "Herdr endpoint or session changed while reading Space tags"));
        }
        validate_id(&observed.snapshot.server_instance)?;
        if self.session_namespace(session_id).await? != session_namespace {
            return Err(InspectionError::new("stale_identity", "Herdr session socket namespace changed while reading Space tags"));
        }
        Ok(ProvenSession {
            session_id: session_id.to_owned(),
            server_instance: observed.snapshot.server_instance,
            session_namespace,
            spaces: observed.snapshot.spaces.into_iter().map(|space| space.id).collect(),
        })
    }

    async fn session_namespace(&self, session_id: &str) -> Result<String, InspectionError> {
        session_namespace(&self.herdr, session_id).await
    }

    async fn prune_candidates(&self) -> Result<HashSet<String>, InspectionError> {
        // Capture revisions before observing Herdr: an older snapshot must not erase
        // a record created or changed concurrently after that observation began.
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            Ok(load(&store)?.spaces.into_iter().map(|space| space.revision).collect())
        }).await.map_err(|error| InspectionError::new("space_tags_read", error.to_string()))?
    }

    pub async fn list(&self, session_id: &str) -> Result<SpaceTagsResponse, InspectionError> {
        let candidates = self.prune_candidates().await?;
        let proven = self.prove(session_id).await?;
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = store.acquire_named_lock(".space-tags.lock", "space_tags_lock")?;
            let mut stored = load(&store)?;
            if prune_absent(&mut stored, &proven, &candidates) { persist(&store, &stored)?; }
            Ok(response(&stored, &proven))
        }).await.map_err(|error| InspectionError::new("space_tags_outcome_unknown", error.to_string()))?
    }

    pub async fn change(&self, session_id: &str, request: &SpaceTagChangeRequest) -> Result<SpaceTagsResponse, InspectionError> {
        validate_id(&request.space_id)?;
        validate_id(&request.server_instance)?;
        let tag = normalize_tag(&request.tag)?;
        let candidates = self.prune_candidates().await?;
        let proven = self.prove(session_id).await?;
        if request.server_instance != proven.server_instance {
            return Err(InspectionError::new("space_tags_target_changed", "Herdr restarted; refresh before changing Space tags"));
        }
        let store = self.store.clone();
        let request = request.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = store.acquire_named_lock(".space-tags.lock", "space_tags_lock")?;
            let mut stored = load(&store)?;
            if prune_absent(&mut stored, &proven, &candidates) { persist(&store, &stored)?; }
            if !proven.spaces.contains(&request.space_id) {
                return Err(InspectionError::new("space_tags_space_unavailable", "The Space is no longer present in this session"));
            }
            let key = tag.to_lowercase();
            let canonical = stored.catalog.iter().find(|entry| entry.to_lowercase() == key).cloned();
            let canonical = match canonical {
                Some(tag) => tag,
                None if request.assigned => {
                    if stored.catalog.len() >= MAX_CATALOG { return Err(limit()); }
                    stored.catalog.push(tag.clone());
                    tag
                }
                None => return Ok(response(&stored, &proven)),
            };
            let index = stored.spaces.iter().position(|space| matches(space, &proven, &request.space_id));
            match (index, request.assigned) {
                (Some(index), true) => {
                    let tags = &mut stored.spaces[index].tags;
                    if tags.contains(&canonical) { return Ok(response(&stored, &proven)); }
                    tags.push(canonical);
                    stored.spaces[index].revision = uuid::Uuid::new_v4().to_string();
                }
                (Some(index), false) => {
                    let tags = &mut stored.spaces[index].tags;
                    let count = tags.len();
                    tags.retain(|tag| tag != &canonical);
                    if tags.len() == count { return Ok(response(&stored, &proven)); }
                    if tags.is_empty() { stored.spaces.remove(index); }
                    else { stored.spaces[index].revision = uuid::Uuid::new_v4().to_string(); }
                }
                (None, true) => {
                    if stored.spaces.len() >= MAX_RECORDS { return Err(limit()); }
                    stored.spaces.push(StoredSpace {
                        session_namespace: proven.session_namespace.clone(),
                        session_id: proven.session_id.clone(),
                        space_id: request.space_id,
                        tags: vec![canonical],
                        revision: uuid::Uuid::new_v4().to_string(),
                    });
                }
                (None, false) => return Ok(response(&stored, &proven)),
            }
            // Limits reject this write; they never evict another Space's tags.
            persist(&store, &stored)?;
            Ok(response(&stored, &proven))
        }).await.map_err(|error| InspectionError::new("space_tags_outcome_unknown", error.to_string()))?
    }
}

pub(crate) async fn session_namespace(herdr: &Arc<dyn ProjectHerdrAdapter>, session_id: &str) -> Result<String, InspectionError> {
    let socket = herdr.session_socket_path(session_id).ok_or_else(|| {
        InspectionError::new("endpoint_identity_unavailable", "Herdr session socket namespace is unavailable")
    })?;
    tokio::task::spawn_blocking(move || {
        let socket = std::fs::canonicalize(socket)
            .map_err(|error| InspectionError::new("endpoint_identity_unavailable", error.to_string()))?;
        Ok(format!("{:x}", Sha256::digest(socket.as_os_str().as_encoded_bytes())))
    }).await.map_err(|error| InspectionError::new("endpoint_identity_unavailable", error.to_string()))?
}

fn matches(space: &StoredSpace, proven: &ProvenSession, space_id: &str) -> bool {
    space.session_namespace == proven.session_namespace && space.session_id == proven.session_id && space.space_id == space_id
}

fn prune_absent(stored: &mut StoredTags, proven: &ProvenSession, candidates: &HashSet<String>) -> bool {
    let count = stored.spaces.len();
    stored.spaces.retain(|space| {
        space.session_namespace != proven.session_namespace || space.session_id != proven.session_id
            || proven.spaces.contains(&space.space_id) || !candidates.contains(&space.revision)
    });
    stored.spaces.len() != count
}

fn persist(store: &ProjectStore, stored: &StoredTags) -> Result<(), InspectionError> {
    // Serialize once: the exact bytes bounded here are the ones atomically persisted.
    let bytes = serde_json::to_vec_pretty(stored)
        .map_err(|error| InspectionError::new("space_tags_write", error.to_string()))?;
    if bytes.len() as u64 > MAX_BYTES { return Err(limit()); }
    atomic_write_bytes(store.state_dir(), FILE, &bytes)
        .map_err(|error| InspectionError::new("space_tags_outcome_unknown", error.to_string()))
}

fn response(stored: &StoredTags, proven: &ProvenSession) -> SpaceTagsResponse {
    SpaceTagsResponse {
        session_id: proven.session_id.clone(),
        server_instance: proven.server_instance.clone(),
        catalog: stored.catalog.clone(),
        spaces: stored.spaces.iter().filter(|space| {
            matches(space, proven, &space.space_id) && proven.spaces.contains(&space.space_id)
        }).map(|space| SpaceTags {
            space_id: space.space_id.clone(),
            tags: stored.catalog.iter().filter(|tag| space.tags.contains(tag)).cloned().collect(),
        }).collect(),
    }
}

fn load(store: &ProjectStore) -> Result<StoredTags, InspectionError> {
    match store.state_dir().symlink_metadata(FILE) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(StoredTags::default()),
        Err(error) => return Err(InspectionError::new("space_tags_read", error.to_string())),
        Ok(_) => {}
    }
    let stored: StoredTags = read_json_bounded(store.state_dir(), FILE, MAX_BYTES).map_err(|error| {
        let code = if error.code == "state_corrupt" || error.code == "unsafe_path" { "space_tags_corrupt" } else { "space_tags_read" };
        InspectionError::new(code, error.message)
    })?;
    validate_stored(&stored).map_err(|error| InspectionError::new("space_tags_corrupt", error.message))?;
    Ok(stored)
}

fn validate_stored(stored: &StoredTags) -> Result<(), InspectionError> {
    if stored.schema != 1 || stored.catalog.len() > MAX_CATALOG || stored.spaces.len() > MAX_RECORDS {
        return Err(InspectionError::new("space_tags_corrupt", "Invalid Space tag schema or collection size"));
    }
    let mut keys = HashSet::new();
    for tag in &stored.catalog {
        if normalize_tag(tag)? != *tag || !keys.insert(tag.to_lowercase()) {
            return Err(InspectionError::new("space_tags_corrupt", "Invalid or duplicate catalog tag"));
        }
    }
    let mut records = HashSet::new();
    let mut revisions = HashSet::new();
    for space in &stored.spaces {
        if space.session_namespace.len() != 64 || !space.session_namespace.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(InspectionError::new("space_tags_corrupt", "Invalid session namespace"));
        }
        if uuid::Uuid::parse_str(&space.revision).is_err() || !revisions.insert(&space.revision) {
            return Err(InspectionError::new("space_tags_corrupt", "Invalid or duplicate Space tag revision"));
        }
        validate_id(&space.session_id)?;
        validate_id(&space.space_id)?;
        if !records.insert((&space.session_namespace, &space.session_id, &space.space_id)) || space.tags.is_empty() {
            return Err(InspectionError::new("space_tags_corrupt", "Duplicate or empty Space tag record"));
        }
        let mut tags = HashSet::new();
        for tag in &space.tags {
            if !stored.catalog.contains(tag) || !tags.insert(tag) {
                return Err(InspectionError::new("space_tags_corrupt", "Unknown or duplicate Space tag"));
            }
        }
    }
    Ok(())
}

fn normalize_tag(input: &str) -> Result<String, InspectionError> {
    let mut normalized = String::with_capacity(input.len().min(128));
    let mut length = 0;
    for word in input.split_whitespace() {
        length += word.chars().count() + usize::from(!normalized.is_empty());
        if length > 32 {
            return Err(InspectionError::new("invalid_space_tag", "Tags must contain at most 32 characters"));
        }
        if !normalized.is_empty() { normalized.push(' '); }
        normalized.push_str(word);
    }
    if normalized.is_empty() || normalized.chars().any(char::is_control) {
        return Err(InspectionError::new("invalid_space_tag", "Tags must contain 1–32 characters without control characters"));
    }
    Ok(normalized)
}

fn validate_id(id: &str) -> Result<(), InspectionError> {
    if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
        return Err(InspectionError::new("invalid_space_tag", "Space tag identity is invalid"));
    }
    Ok(())
}

fn limit() -> InspectionError {
    InspectionError::new("space_tags_limit", "Space tag storage limit reached; no existing tags were removed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HerdrAdapter, SessionSubscription, TerminalSession, project_adapter::*};
    use cockpit_protocol::v1::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("cockpit-space-tags-{}", uuid::Uuid::new_v4())))
        }
        fn service(&self, adapter: Arc<Adapter>) -> SpaceTagService {
            SpaceTagService::new(&self.0, adapter).unwrap()
        }
        fn file(&self) -> PathBuf { self.0.join("space-tags").join(FILE) }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    struct Adapter {
        instance: AtomicUsize,
        namespace: AtomicUsize,
        present: AtomicBool,
        unverifiable: AtomicBool,
        mismatch: AtomicBool,
    }
    impl Adapter {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                instance: AtomicUsize::new(0),
                namespace: AtomicUsize::new(0),
                present: AtomicBool::new(true),
                unverifiable: AtomicBool::new(false),
                mismatch: AtomicBool::new(false),
            })
        }
        fn instance(&self) -> String { format!("instance-{}", self.instance.load(Ordering::SeqCst)) }
    }
    fn unused<T>() -> Result<T, InspectionError> { Err(InspectionError::new("unused", "Unexpected adapter call")) }

    #[async_trait::async_trait]
    impl HerdrAdapter for Adapter {
        fn session_socket_path(&self, _: &str) -> Option<PathBuf> {
            Some(if self.namespace.load(Ordering::SeqCst) == 0 { std::env::temp_dir() } else { std::env::current_dir().unwrap() })
        }
        async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> { unused() }
        async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> { unused() }
        async fn sessions(&self) -> Result<SessionListResponse, InspectionError> { unused() }
        async fn session_snapshot(&self, session: &str) -> Result<SessionSnapshotResponse, InspectionError> {
            Ok(SessionSnapshotResponse {
                session_id: session.into(), server_instance: self.instance(), version: "test".into(), protocol: 1,
                focused_space_id: None, focused_tab_id: None, focused_pane_id: None, herdr_shell: None,
                spaces: if self.present.load(Ordering::SeqCst) { vec![SpaceSummary {
                    id: "w1".into(), label: "Test".into(), number: 1, tab_count: 0, pane_count: 0,
                    focused: false, agent_status: "none".into(), git: None,
                }] } else { vec![] },
                tabs: vec![], panes: vec![], agents: vec![],
            })
        }
        async fn focus(&self, _: &str, _: &FocusRequest) -> Result<FocusResponse, InspectionError> { unused() }
        async fn mutate(&self, _: &str, _: &ResourceMutationRequest) -> Result<ResourceMutationResponse, InspectionError> { unused() }
        async fn subscribe_session(&self, _: &str, _: &SessionSnapshotResponse) -> Result<SessionSubscription, InspectionError> { unused() }
        async fn open_terminal(&self, _: &TerminalOpenRequest) -> Result<TerminalSession, InspectionError> { unused() }
    }

    #[async_trait::async_trait]
    impl ProjectHerdrAdapter for Adapter {
        async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> {
            if self.unverifiable.load(Ordering::SeqCst) {
                return Err(InspectionError::new("endpoint_identity_unavailable", "Peer cannot be proven"));
            }
            Ok(self.instance())
        }
        async fn project_snapshot_at_endpoint(&self, session: &str, _: &str) -> Result<ProjectSnapshot, InspectionError> {
            Ok(ProjectSnapshot {
                snapshot: self.session_snapshot(session).await?,
                endpoint_identity: if self.mismatch.load(Ordering::SeqCst) { "other".into() } else { self.instance() },
                boot_id: None, peer_process: None,
            })
        }
        async fn project_inventory(&self, _: &str, _: &str) -> Result<ProjectInventory, InspectionError> { unused() }
        async fn project_worktree(&self, _: &str, _: &ProjectWorktreeRequest) -> Result<ProjectWorktreeResult, InspectionError> { unused() }
        async fn project_worktree_dirty(&self, _: &str, _: u32, _: u32) -> Result<WorktreeStatus, InspectionError> { unused() }
        async fn project_close_workspace(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> { unused() }
        async fn project_remove_worktree(&self, _: &str, _: &ProjectWorktreeRemoveRequest) -> Result<(), InspectionError> { unused() }
    }

    fn request(adapter: &Adapter, tag: &str, assigned: bool) -> SpaceTagChangeRequest {
        SpaceTagChangeRequest { space_id: "w1".into(), server_instance: adapter.instance(), tag: tag.into(), assigned }
    }

    #[tokio::test]
    async fn canonical_duplicates_and_idempotent_unassign_preserve_other_tags_and_catalog() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let service = fixture.service(adapter.clone());
        service.change("session", &request(&adapter, "Needs testing", true)).await.unwrap();
        let duplicate = service.change("session", &request(&adapter, " needs  testing ", true)).await.unwrap();
        assert_eq!(duplicate.catalog, ["Needs testing"]);
        assert_eq!(duplicate.spaces[0].tags, ["Needs testing"]);
        service.change("session", &request(&adapter, "My check", true)).await.unwrap();
        let remaining = service.change("session", &request(&adapter, "NEEDS TESTING", false)).await.unwrap();
        assert_eq!(remaining.spaces[0].tags, ["My check"]);
        let bytes = fs::read(fixture.file()).unwrap();
        service.change("session", &request(&adapter, "Needs testing", false)).await.unwrap();
        assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
        let empty = service.change("session", &request(&adapter, "My check", false)).await.unwrap();
        assert!(empty.spaces.is_empty());
        assert_eq!(empty.catalog, ["Needs testing", "My check"]);
        assert!(load(&service.store).unwrap().spaces.is_empty());
        service.change("session", &request(&adapter, "Never created", false)).await.unwrap();
        assert_eq!(service.list("session").await.unwrap().catalog, empty.catalog);
    }

    #[tokio::test]
    async fn concurrent_windows_merge_independent_tag_toggles_and_survive_host_restart() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let first = fixture.service(adapter.clone());
        let second = fixture.service(adapter.clone());
        let a = request(&adapter, "My check", true);
        let b = request(&adapter, "Needs testing", true);
        let (a, b) = tokio::join!(first.change("session", &a), second.change("session", &b));
        a.unwrap();
        b.unwrap();
        drop(first);
        drop(second);
        let restarted = fixture.service(adapter.clone());
        let state = restarted.list("session").await.unwrap();
        assert_eq!(state.catalog.len(), 2);
        assert_eq!(state.spaces[0].tags, state.catalog);
        let a = request(&adapter, "My check", false);
        let b = request(&adapter, "Demo Friday", true);
        let (a, b) = tokio::join!(restarted.change("session", &a), restarted.change("session", &b));
        a.unwrap();
        b.unwrap();
        let final_state = restarted.list("session").await.unwrap();
        assert_eq!(final_state.spaces[0].tags, ["Needs testing", "Demo Friday"]);
    }

    #[tokio::test]
    async fn restart_preserves_tags_but_session_namespace_and_observed_closure_isolate_spaces() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let service = fixture.service(adapter.clone());
        service.change("session", &request(&adapter, "My check", true)).await.unwrap();
        assert!(service.list("other-session").await.unwrap().spaces.is_empty());
        adapter.instance.store(1, Ordering::SeqCst);
        assert_eq!(service.list("session").await.unwrap().spaces[0].tags, ["My check"]);
        adapter.namespace.store(1, Ordering::SeqCst);
        assert!(service.list("session").await.unwrap().spaces.is_empty());
        service.change("session", &request(&adapter, "Needs testing", true)).await.unwrap();
        assert_eq!(load(&service.store).unwrap().spaces.len(), 2);
        adapter.namespace.store(0, Ordering::SeqCst);
        adapter.present.store(false, Ordering::SeqCst);
        assert!(service.list("session").await.unwrap().spaces.is_empty());
        assert_eq!(load(&service.store).unwrap().spaces.len(), 1);
        adapter.present.store(true, Ordering::SeqCst);
        adapter.instance.store(2, Ordering::SeqCst);
        assert!(service.list("session").await.unwrap().spaces.is_empty());
        assert_eq!(service.list("session").await.unwrap().catalog, ["My check", "Needs testing"]);
    }

    #[tokio::test]
    async fn stale_or_missing_or_unproven_targets_never_write() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let service = fixture.service(adapter.clone());
        let stale = request(&adapter, "My check", true);
        adapter.instance.store(1, Ordering::SeqCst);
        assert_eq!(service.change("session", &stale).await.unwrap_err().code, "space_tags_target_changed");
        adapter.present.store(false, Ordering::SeqCst);
        assert_eq!(service.change("session", &request(&adapter, "My check", true)).await.unwrap_err().code, "space_tags_space_unavailable");
        adapter.present.store(true, Ordering::SeqCst);
        adapter.mismatch.store(true, Ordering::SeqCst);
        assert_eq!(service.change("session", &request(&adapter, "My check", true)).await.unwrap_err().code, "stale_identity");
        adapter.mismatch.store(false, Ordering::SeqCst);
        adapter.unverifiable.store(true, Ordering::SeqCst);
        assert_eq!(service.change("session", &request(&adapter, "My check", true)).await.unwrap_err().code, "endpoint_identity_unavailable");
        assert!(!fixture.file().exists());
    }

    #[tokio::test]
    async fn invalid_tags_and_corrupt_storage_are_rejected_without_overwriting_evidence() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let service = fixture.service(adapter.clone());
        for tag in ["".to_owned(), "a".repeat(33), "a\u{7}".to_owned()] {
            assert_eq!(service.change("session", &request(&adapter, &tag, true)).await.unwrap_err().code, "invalid_space_tag");
        }
        for corrupt in [
            b"not JSON".as_slice(),
            br#"{"schema":1,"catalog":["Check","check"],"spaces":[]}"#.as_slice(),
            br#"{"schema":1,"catalog":["Check"],"spaces":[{"session_namespace":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","session_id":"session","space_id":"w1","tags":["Missing"],"revision":"00000000-0000-4000-8000-000000000000"}]}"#.as_slice(),
        ] {
            fs::write(fixture.file(), corrupt).unwrap();
            assert_eq!(service.list("session").await.unwrap_err().code, "space_tags_corrupt");
            assert_eq!(service.change("session", &request(&adapter, "My check", true)).await.unwrap_err().code, "space_tags_corrupt");
            assert_eq!(fs::read(fixture.file()).unwrap(), corrupt);
        }
    }

    #[tokio::test]
    async fn explicit_catalog_limit_keeps_every_existing_entry_and_record() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let service = fixture.service(adapter.clone());
        let stored = StoredTags {
            schema: 1,
            catalog: (0..MAX_CATALOG).map(|index| format!("Tag {index}")).collect(),
            spaces: vec![StoredSpace {
                session_namespace: "a".repeat(64), session_id: "session".into(), space_id: "w1".into(), tags: vec!["Tag 0".into()],
                revision: uuid::Uuid::new_v4().to_string(),
            }],
        };
        let bytes = serde_json::to_vec_pretty(&stored).unwrap();
        fs::write(fixture.file(), &bytes).unwrap();
        assert_eq!(service.change("session", &request(&adapter, "New tag", true)).await.unwrap_err().code, "space_tags_limit");
        assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
        service.change("session", &request(&adapter, "Tag 0", true)).await.unwrap();
        assert_eq!(load(&service.store).unwrap().spaces.len(), 2);
    }

    #[tokio::test]
    async fn disconnected_observation_does_not_prune_last_spaces_tags() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let service = fixture.service(adapter.clone());
        service.change("session", &request(&adapter, "My check", true)).await.unwrap();
        let bytes = fs::read(fixture.file()).unwrap();
        adapter.present.store(false, Ordering::SeqCst);
        adapter.unverifiable.store(true, Ordering::SeqCst);
        assert_eq!(service.list("session").await.unwrap_err().code, "endpoint_identity_unavailable");
        assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
        adapter.unverifiable.store(false, Ordering::SeqCst);
        assert!(service.list("session").await.unwrap().spaces.is_empty());
        assert!(load(&service.store).unwrap().spaces.is_empty());
    }

    #[test]
    fn stale_prune_candidates_keep_new_and_concurrently_modified_records() {
        let namespace = "a".repeat(64);
        let old_revision = uuid::Uuid::new_v4().to_string();
        let unchanged_revision = uuid::Uuid::new_v4().to_string();
        let candidates = HashSet::from([old_revision, unchanged_revision.clone()]);
        let mut stored = StoredTags {
            schema: 1,
            catalog: vec!["My check".into()],
            spaces: ["new", "modified", "closed"].into_iter().map(|id| StoredSpace {
                session_namespace: namespace.clone(), session_id: "session".into(), space_id: id.into(),
                tags: vec!["My check".into()],
                revision: if id == "closed" { unchanged_revision.clone() } else { uuid::Uuid::new_v4().to_string() },
            }).collect(),
        };
        let proven = ProvenSession {
            session_namespace: namespace, session_id: "session".into(), server_instance: "instance".into(), spaces: BTreeSet::new(),
        };
        assert!(prune_absent(&mut stored, &proven, &candidates));
        assert_eq!(stored.spaces.iter().map(|space| space.space_id.as_str()).collect::<Vec<_>>(), ["new", "modified"]);
    }

    #[tokio::test]
    async fn record_limit_rejects_growth_without_evicting_other_namespaces() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let service = fixture.service(adapter.clone());
        let stored = StoredTags {
            schema: 1, catalog: vec!["Check".into()],
            spaces: (0..MAX_RECORDS).map(|index| StoredSpace {
                session_namespace: "a".repeat(64), session_id: "s".into(), space_id: index.to_string(),
                tags: vec!["Check".into()], revision: uuid::Uuid::new_v4().to_string(),
            }).collect(),
        };
        let bytes = serde_json::to_vec(&stored).unwrap();
        assert!(bytes.len() as u64 <= MAX_BYTES);
        fs::write(fixture.file(), &bytes).unwrap();
        assert_eq!(service.change("session", &request(&adapter, "Check", true)).await.unwrap_err().code, "space_tags_limit");
        assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
    }

    #[tokio::test]
    async fn stale_write_fence_preserves_existing_durable_tags() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let service = fixture.service(adapter.clone());
        let stale = request(&adapter, "My check", true);
        service.change("session", &stale).await.unwrap();
        let bytes = fs::read(fixture.file()).unwrap();
        adapter.instance.store(1, Ordering::SeqCst);
        assert_eq!(service.change("session", &stale).await.unwrap_err().code, "space_tags_target_changed");
        assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
        assert_eq!(service.list("session").await.unwrap().spaces[0].tags, ["My check"]);
    }
}
