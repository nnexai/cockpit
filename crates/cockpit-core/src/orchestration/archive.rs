//! Local-operator archival with transparent, ordered historical reads.
use std::collections::{BTreeMap, HashSet};

use cockpit_protocol::orchestration::{ActorRef, CleanupState, DeliveryStage, Message, Run, RunStage, TaskOwnership, ResourceRequestRecord, OwnedResourceRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use serde_json::{json, Value};

use super::{error, tasks_md::validate_uuid, OrchestrationService, OrchestrationState};
use super::store::{LockedStore, MAX_STATE_BYTES, DurableOperationRecord};
use super::credentials::JobCredentialRecord;
use super::claims::CheckoutClaim;
use crate::{InspectionError, project_store::{atomic_write_bytes, read_bytes_bounded}};
// Each archived element gains a JSON position tuple. Even if the original
// state consisted solely of single-byte elements, a 64-bit position adds at
// most 23 bytes to every two bytes of input (element plus separator).
// Allow segment metadata as well, while keeping reads explicitly bounded.
const MAX_ARCHIVE_BYTES: usize = MAX_STATE_BYTES * 16 + 1024 * 1024;


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchiveRef {
    id: String,
    digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Segment {
    version: u32,
    id: String,
    root_id: String,
    task_id: String,
    task_revision: String,
    created_at: String,
    runs: Vec<(usize, Run)>,
    messages: Vec<(usize, Message)>,
    task_ownership: Vec<(usize, TaskOwnership)>,
    operations: Vec<(usize, DurableOperationRecord)>,
    credentials: Vec<(usize, JobCredentialRecord)>,
    checkout_claims: Vec<(usize, CheckoutClaim)>,
    resource_requests: Vec<(usize, ResourceRequestRecord)>,
    owned_resources: Vec<(usize, OwnedResourceRecord)>,
}

fn filename(id: &str) -> Result<String, InspectionError> {
    let uuid = validate_uuid(id)?;
    if uuid.to_string() != id { return Err(error("archive_invalid_id", "Archive ID must be a canonical UUID")); }
    Ok(format!("archive-{id}.json"))
}
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn segment(locked: &LockedStore<'_>, reference: &ArchiveRef) -> Result<Segment, InspectionError> {
    let bytes = read_bytes_bounded(locked.state_dir(), &filename(&reference.id)?, MAX_ARCHIVE_BYTES as u64)?;
    if digest(&bytes) != reference.digest { return Err(error("archive_corrupt", "Archive digest mismatch")); }
    let segment: Segment = serde_json::from_slice(&bytes).map_err(|e| error("archive_corrupt", e.to_string()))?;
    if segment.version != 1 || segment.id != reference.id || (segment.messages.is_empty() && segment.runs.is_empty()) {
        return Err(error("archive_corrupt", "Invalid archive segment"));
    }
    Ok(segment)
}

pub(crate) fn validate_refs(locked: &LockedStore<'_>, state: &OrchestrationState) -> Result<(), InspectionError> {
    for reference in &state.archives {
        let bytes = read_bytes_bounded(locked.state_dir(), &filename(&reference.id)?, MAX_ARCHIVE_BYTES as u64)?;
        if digest(&bytes) != reference.digest {
            return Err(error("archive_corrupt", "Archive digest mismatch"));
        }
    }
    Ok(())
}

fn load_segments(locked: &LockedStore<'_>, state: &OrchestrationState) -> Result<Vec<Segment>, InspectionError> {
    let mut ids = HashSet::new();
    state.archives.iter().map(|reference| {
        if !ids.insert(&reference.id) { return Err(error("archive_corrupt", "Duplicate archive reference")); }
        segment(locked, reference)
    }).collect()
}

fn message_key(message: &Message) -> (u8, String, String) {
    let (kind, sender) = match &message.from {
        ActorRef::Operator => (0, String::new()),
        ActorRef::Dispatcher => (1, String::new()),
        ActorRef::Run { run_id } => (2, run_id.clone()),
    };
    (kind, sender, message.message_id.clone())
}

fn entries(segments: &[Segment]) -> Result<BTreeMap<usize, Message>, InspectionError> {
    let mut positions = BTreeMap::new();
    let mut ids = HashSet::new();
    for segment in segments {
        for (position, message) in &segment.messages {
            if !ids.insert(message_key(message)) || positions.insert(*position, message.clone()).is_some() {
                return Err(error("archive_corrupt", "Duplicate archived message or position"));
            }
        }
    }
    Ok(positions)
}

fn run_entries(segments: &[Segment]) -> Result<BTreeMap<usize, Run>, InspectionError> {
    let mut positions = BTreeMap::new();
    let mut ids = HashSet::new();
    for segment in segments {
        for (position, run) in &segment.runs {
            if !ids.insert(run.run_id.clone()) || positions.insert(*position, run.clone()).is_some() {
                return Err(error("archive_corrupt", "Duplicate archived run or position"));
            }
        }
    }
    Ok(positions)
}

fn merge_positions<T>(active: &mut Vec<T>, positions: BTreeMap<usize, T>) -> Result<(), InspectionError> {
    let total = active.len().checked_add(positions.len()).ok_or_else(|| error("archive_corrupt", "History length overflow"))?;
    if positions.keys().next_back().is_some_and(|position| *position >= total) {
        return Err(error("archive_corrupt", "Invalid archived position"));
    }
    let mut saved = positions.into_iter().peekable();
    let mut current = std::mem::take(active).into_iter();
    let mut merged = Vec::with_capacity(total);
    for index in 0..total {
        if saved.peek().is_some_and(|(position, _)| *position == index) {
            merged.push(saved.next().expect("peeked entry").1);
        } else {
            merged.push(current.next().ok_or_else(|| error("archive_corrupt", "Invalid archived positions"))?);
        }
    }
    *active = merged;
    Ok(())
}

macro_rules! hydrate_group {
    ($segments:expr, $state:expr, $field:ident, $key:expr) => {{
        let mut positions = BTreeMap::new();
        let key = $key;
        let mut ids: HashSet<_> = $state.$field.iter().map(&key).collect();
        for segment in $segments {
            for (position, item) in &segment.$field {
                if !ids.insert(key(item)) || positions.insert(*position, item.clone()).is_some() {
                    return Err(error("archive_corrupt", concat!("Duplicate archived ", stringify!($field))));
                }
            }
        }
        merge_positions(&mut $state.$field, positions)?;
    }};
}

macro_rules! compact_group {
    ($segments:expr, $compact:expr, $field:ident) => {{
        let mut positions = BTreeMap::new();
        for segment in $segments {
            for (position, item) in &segment.$field {
                if positions.insert(*position, item).is_some() {
                    return Err(error("archive_corrupt", concat!("Duplicate position in ", stringify!($field))));
                }
            }
        }
        for (position, expected) in &positions {
            let actual = $compact.$field.get(*position).ok_or_else(|| error("archive_conflict", "Archived record was removed"))?;
            if serde_json::to_value(actual).map_err(|e| error("archive_corrupt", e.to_string()))?
                != serde_json::to_value(expected).map_err(|e| error("archive_corrupt", e.to_string()))? {
                return Err(error("archive_conflict", "Archived record changed; restore it first"));
            }
        }
        let mut removed = positions.keys().copied().peekable();
        let mut index = 0;
        $compact.$field.retain(|_| {
            let keep = if removed.peek() == Some(&index) { removed.next(); false } else { true };
            index += 1;
            keep
        });
    }};
}

pub(crate) fn hydrate(locked: &LockedStore<'_>, state: &mut OrchestrationState) -> Result<(), InspectionError> {
    let segments = load_segments(locked, state)?;
    let positions = entries(&segments)?;
    let runs = run_entries(&segments)?;
    let mut run_ids: HashSet<String> = state.runs.iter().map(|r| r.run_id.clone()).collect();
    for run in runs.values() {
        if !run_ids.insert(run.run_id.clone()) {
            return Err(error("archive_corrupt", "Archived run conflicts with active history"));
        }
    }
    merge_positions(&mut state.runs, runs)?;
    let mut ids: HashSet<_> = state.messages.iter().map(message_key).collect();
    for message in positions.values() {
        if !ids.insert(message_key(message)) {
            return Err(error("archive_corrupt", "Archived message conflicts with active history"));
        }
    }
    merge_positions(&mut state.messages, positions)?;
    hydrate_group!(&segments, state, task_ownership, |r: &TaskOwnership| (r.root_id.clone(), r.task_id.clone()));
    hydrate_group!(&segments, state, operations, |r: &DurableOperationRecord| (r.principal_key.clone(), r.operation_id.clone()));
    hydrate_group!(&segments, state, credentials, |r: &JobCredentialRecord| r.credential_id.clone());
    hydrate_group!(&segments, state, checkout_claims, |r: &CheckoutClaim| r.claim_id.clone());
    hydrate_group!(&segments, state, resource_requests, |r: &ResourceRequestRecord| r.request_id.clone());
    hydrate_group!(&segments, state, owned_resources, |r: &OwnedResourceRecord| r.resource_id.clone());
    Ok(())
}

pub(crate) fn compact(locked: &LockedStore<'_>, state: &OrchestrationState) -> Result<OrchestrationState, InspectionError> {
    let segments = load_segments(locked, state)?;
    let mut compact = state.clone();
    compact_group!(&segments, compact, runs);
    compact_group!(&segments, compact, messages);
    compact_group!(&segments, compact, task_ownership);
    compact_group!(&segments, compact, operations);
    compact_group!(&segments, compact, credentials);
    compact_group!(&segments, compact, checkout_claims);
    compact_group!(&segments, compact, resource_requests);
    compact_group!(&segments, compact, owned_resources);
    Ok(compact)
}

fn candidates(locked: &LockedStore<'_>, state: &OrchestrationState, root_id: &str, task_id: &str) -> Result<(String, Vec<(usize, Run)>, Vec<(usize, Message)>), InspectionError> {
    validate_uuid(root_id)?;
    validate_uuid(task_id)?;
    let root = state.runs.iter().find(|r| r.run_id == root_id && r.root_id == root_id && r.parent_run_id.is_none())
        .ok_or_else(|| error("root_not_found", "Root not found"))?;
    let doc = locked.tasks(root_id)?;
    let task = doc.tasks.iter().find(|t| t.task_id == task_id).ok_or_else(|| error("task_not_found", "Task not found"))?;
    if !task.checked || task.diagnostic.is_some() || task.relations_diagnostic.is_some() || doc.unidentified_items != 0 {
        return Err(error("archive_ineligible", "Task must be checked and unambiguous"));
    }
    super::archive_eligibility::check(state, root_id, task_id)?;
    let segments = load_segments(locked, state)?;
    for previous in &segments {
        if previous.root_id == root_id && previous.task_id == task_id {
            return Err(error("archive_ineligible", "Task already has an active archive"));
        }
    }
    let runs: HashSet<&str> = state.runs.iter().filter(|r| r.root_id == root_id && r.task_id.as_deref() == Some(task_id)).map(|r| r.run_id.as_str()).collect();
    if runs.is_empty() || state.runs.iter().any(|r| runs.contains(r.run_id.as_str()) && (r.stage != RunStage::Closed || r.session_id != root.session_id)) {
        return Err(error("archive_ineligible", "Task requires closed runs in the root session"));
    }
    let already: HashSet<usize> = entries(&segments)?.into_keys().collect();
    let mut selected = Vec::new();
    for (position, message) in state.messages.iter().enumerate() {
        let related = runs.contains(message.to_run_id.as_str()) || matches!(&message.from, ActorRef::Run { run_id } if runs.contains(run_id.as_str()));
        if related {
            if message.stage != DeliveryStage::Acked || message.acked_at.is_none() {
                return Err(error("archive_ineligible", "All related messages must be acknowledged"));
            }
            if !already.contains(&position) { selected.push((position, message.clone())); }
        }
    }
    let selected_runs = state.runs.iter().enumerate().filter(|(_, r)| runs.contains(r.run_id.as_str()))
        .map(|(position, run)| (position, run.clone())).collect();
    Ok((task.task_revision.clone(), selected_runs, selected))
}

impl OrchestrationService {
    pub fn archive_plan(&self, root_id: &str, task_id: &str) -> Result<Value, InspectionError> {
        let locked = self.store.lock()?;
        let state = locked.read()?;
        let (task_revision, runs, messages) = candidates(&locked, &state, root_id, task_id)?;
        Ok(json!({"root_id": root_id, "task_id": task_id, "revision": state.revision, "task_revision": task_revision, "runs": runs.len(), "messages": messages.len()}))
    }
    pub fn archive_apply(&self, root_id: &str, task_id: &str, expected_revision: u64) -> Result<Value, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        if state.revision != expected_revision { return Err(error("orchestration_revision_conflict", "State revision changed")); }
        let (task_revision, runs, messages) = candidates(&locked, &state, root_id, task_id)?;
        let id = uuid::Uuid::new_v4().to_string();
        let name = filename(&id)?;
        if locked.state_dir().symlink_metadata(&name).is_ok() { return Err(error("archive_conflict", "Archive ID already exists")); }
        let run_ids: HashSet<&str> = runs.iter().map(|(_, r)| r.run_id.as_str()).collect();
        let claim_ids: HashSet<&str> = runs.iter().filter_map(|(_, r)| r.checkout_claim_id.as_deref())
            .filter(|id| !state.runs.iter().any(|r| !run_ids.contains(r.run_id.as_str()) && r.checkout_claim_id.as_deref() == Some(*id))).collect();
        let task_ownership = state.task_ownership.iter().enumerate().filter(|(_, r)| r.root_id == root_id && r.task_id == task_id).map(|(i, r)| (i, r.clone())).collect();
        let operations = state.operations.iter().enumerate().filter(|(_, r)| r.root_id == root_id
            && r.run_id.as_deref().is_some_and(|id| run_ids.contains(id))
            && r.actor_run_id.as_deref().is_none_or(|id| run_ids.contains(id))).map(|(i, r)| (i, r.clone())).collect();
        let credentials = state.credentials.iter().enumerate().filter(|(_, r)| run_ids.contains(r.run_id.as_str())).map(|(i, r)| (i, r.clone())).collect();
        let checkout_claims = state.checkout_claims.iter().enumerate().filter(|(_, r)| claim_ids.contains(r.claim_id.as_str())).map(|(i, r)| (i, r.clone())).collect();
        let resource_requests = state.resource_requests.iter().enumerate().filter(|(_, r)| run_ids.contains(r.holder.run_id.as_str())).map(|(i, r)| (i, r.clone())).collect();
        // Kept resources remain actionable by cleanup and must stay in the mutable state.
        let owned_resources = state.owned_resources.iter().enumerate().filter(|(_, r)| r.cleanup.state == CleanupState::Removed
            && run_ids.contains(r.run_id.as_str())
            && r.consumers.iter().all(|c| run_ids.contains(c.run_id.as_str()))).map(|(i, r)| (i, r.clone())).collect();
        let segment = Segment { version: 1, id: id.clone(), root_id: root_id.into(), task_id: task_id.into(), task_revision, created_at: super::now(), runs, messages,
            task_ownership, operations, credentials, checkout_claims, resource_requests, owned_resources };
        let bytes = serde_json::to_vec(&segment).map_err(|e| error("archive_write", e.to_string()))?;
        if bytes.len() > MAX_ARCHIVE_BYTES { return Err(error("archive_full", "Archive exceeds size limit")); }
        atomic_write_bytes(locked.state_dir(), &name, &bytes).map_err(|e| error("archive_write", format!("Archive publication outcome may be unknown: {e}")))?;
        state.archives.push(ArchiveRef { id: id.clone(), digest: digest(&bytes) });
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(json!({"archive_id": id, "revision": revision, "runs": segment.runs.len(), "messages": segment.messages.len()}))
    }
    pub fn archive_list(&self) -> Result<Value, InspectionError> {
        let locked = self.store.lock()?;
        let state = locked.read()?;
        let mut list = Vec::new();
        for reference in &state.archives {
            let item = segment(&locked, reference)?;
            list.push(json!({"archive_id": item.id, "status": "published", "root_id": item.root_id, "task_id": item.task_id, "created_at": item.created_at, "runs": item.runs.len(), "messages": item.messages.len()}));
        }
        let published: HashSet<&str> = state.archives.iter().map(|r| r.id.as_str()).collect();
        let mut unreferenced = Vec::new();
        for entry in locked.state_dir().entries().map_err(|e| error("archive_read", e.to_string()))? {
            let entry = entry.map_err(|e| error("archive_read", e.to_string()))?;
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|name| name.strip_prefix("archive-")).and_then(|name| name.strip_suffix(".json")) else { continue };
            if filename(id).is_ok() && !published.contains(id) {
                unreferenced.push(id.to_string());
            }
        }
        unreferenced.sort();
        list.extend(unreferenced.into_iter().map(|id| json!({"archive_id": id, "status": "unreferenced"})));
        Ok(json!({"archives": list, "revision": state.revision}))
    }
    pub fn archive_show(&self, archive_id: &str) -> Result<Value, InspectionError> {
        let locked = self.store.lock()?;
        let state = locked.read()?;
        if let Some(reference) = state.archives.iter().find(|r| r.id == archive_id) {
            return serde_json::to_value(segment(&locked, reference)?).map_err(|e| error("archive_corrupt", e.to_string()));
        }
        let bytes = read_bytes_bounded(locked.state_dir(), &filename(archive_id)?, MAX_ARCHIVE_BYTES as u64)?;
        let reference = ArchiveRef { id: archive_id.into(), digest: digest(&bytes) };
        Ok(json!({"status": "unreferenced", "integrity": "unverified", "segment": segment(&locked, &reference)?}))
    }
    pub fn archive_restore(&self, archive_id: &str) -> Result<Value, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = state.archives.iter().position(|r| r.id == archive_id).ok_or_else(|| error("archive_not_found", "Archive not found"))?;
        state.archives.remove(index);
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(json!({"archive_id": archive_id, "revision": revision}))
    }
}

#[cfg(test)]
pub(super) fn test_segment(locked: &LockedStore<'_>, messages: Vec<(usize, Message)>) -> ArchiveRef {
    let id = uuid::Uuid::new_v4().to_string();
    let item = Segment { version: 1, id: id.clone(), root_id: "root".into(), task_id: "task".into(),
        task_revision: "revision".into(), created_at: "time".into(), runs: Vec::new(), messages,
        task_ownership: Vec::new(), operations: Vec::new(), credentials: Vec::new(), checkout_claims: Vec::new(),
        resource_requests: Vec::new(), owned_resources: Vec::new() };
    let bytes = serde_json::to_vec(&item).unwrap();
    atomic_write_bytes(locked.state_dir(), &filename(&id).unwrap(), &bytes).unwrap();
    ArchiveRef { id, digest: digest(&bytes) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::store::{DurableOperationKind, OrchestrationStore};
    use cockpit_protocol::orchestration::MessageKind;

    fn operation(principal: &str) -> DurableOperationRecord {
        DurableOperationRecord {
            operation_id: "same-id".into(), actor_run_id: None, principal_key: principal.into(),
            root_id: "root".into(), kind: DurableOperationKind::TaskScopeUpdate,
            request_digest: "digest".into(), run_id: None, resume_operation_id: None,
            result: None, effect_receipts: vec![], created_at: "now".into(), updated_at: "now".into(),
        }
    }

    #[test]
    fn operations_with_same_id_different_principals_survive_hydration_and_restore() {
        let path = std::env::temp_dir().join(format!("cockpit-archive-operation-{}", uuid::Uuid::new_v4()));
        let store = OrchestrationStore::open(&path).unwrap();
        let lock = store.lock().unwrap();
        let mut state = lock.read().unwrap();
        let message = Message { message_id: "message".into(), to_run_id: "run".into(), seq: 1,
            from: ActorRef::Operator, kind: MessageKind::Instruction, text: "history".into(), images: vec![],
            in_reply_to: None, report: None, thread_id: None, handoff: None, stale: false,
            escalated_from: None, stage: DeliveryStage::Acked, created_at: "now".into(), acked_at: Some("now".into()) };
        state.messages.push(message.clone());
        state.operations = vec![operation("first"), operation("second")];
        let id = uuid::Uuid::new_v4().to_string();
        let item = Segment { version: 1, id: id.clone(), root_id: "root".into(), task_id: "task".into(),
            task_revision: "revision".into(), created_at: "now".into(), runs: vec![], messages: vec![(0, message)],
            task_ownership: vec![], operations: vec![(0, state.operations[0].clone())], credentials: vec![],
            checkout_claims: vec![], resource_requests: vec![], owned_resources: vec![] };
        let bytes = serde_json::to_vec(&item).unwrap();
        atomic_write_bytes(lock.state_dir(), &filename(&id).unwrap(), &bytes).unwrap();
        state.archives.push(ArchiveRef { id, digest: digest(&bytes) });
        lock.save(&mut state).unwrap();
        let mut hydrated = lock.read().unwrap();
        assert_eq!(hydrated.operations.iter().map(|op| op.principal_key.as_str()).collect::<Vec<_>>(), vec!["first", "second"]);
        hydrated.archives.clear();
        lock.save(&mut hydrated).unwrap();
        assert_eq!(lock.read().unwrap().operations.len(), 2);
        drop(lock);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn segment_can_exceed_state_limit_due_to_position_overhead() {
        let path = std::env::temp_dir().join(format!("cockpit-archive-size-{}", uuid::Uuid::new_v4()));
        let store = OrchestrationStore::open(&path).unwrap();
        let lock = store.lock().unwrap();
        let mut state = OrchestrationState::default();
        let message = Message { message_id: String::new(), to_run_id: "run".into(), seq: 1,
            from: ActorRef::Operator, kind: MessageKind::Instruction, text: "x".repeat(32_000), images: vec![],
            in_reply_to: None, report: None, thread_id: None, handoff: None, stale: false,
            escalated_from: None, stage: DeliveryStage::Acked, created_at: "now".into(), acked_at: Some("now".into()) };
        state.messages = (0..2_000).map(|i| {
            let mut m = message.clone();
            m.message_id = i.to_string();
            m
        }).collect();
        let state_len = serde_json::to_vec(&state).unwrap().len();
        assert!(state_len < MAX_STATE_BYTES);
        state.messages[0].text.push_str(&"x".repeat(MAX_STATE_BYTES - state_len - 1));
        assert_eq!(serde_json::to_vec(&state).unwrap().len(), MAX_STATE_BYTES - 1);
        let id = uuid::Uuid::new_v4().to_string();
        let item = Segment { version: 1, id: id.clone(), root_id: "root".into(), task_id: "task".into(),
            task_revision: "revision".into(), created_at: "now".into(), runs: vec![],
            messages: state.messages.into_iter().enumerate().collect(), task_ownership: vec![],
            operations: vec![], credentials: vec![], checkout_claims: vec![], resource_requests: vec![], owned_resources: vec![] };
        let bytes = serde_json::to_vec(&item).unwrap();
        assert!(bytes.len() > MAX_STATE_BYTES && bytes.len() <= MAX_ARCHIVE_BYTES);
        atomic_write_bytes(lock.state_dir(), &filename(&id).unwrap(), &bytes).unwrap();
        let reference = ArchiveRef { id, digest: digest(&bytes) };
        assert_eq!(segment(&lock, &reference).unwrap().messages.len(), 2_000);
        drop(lock);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
}
