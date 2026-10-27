use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::Arc;

use cockpit_protocol::v1::{AgentAttentionChange, AgentAttentionChangeRequest, AgentAttentionCompletion, AgentAttentionLast, AgentAttentionObservation, AgentAttentionRecord, AgentAttentionRefusal, AgentAttentionResponse, AgentAttentionStamp, SessionSnapshotResponse};
use serde::{Deserialize, Serialize};

use crate::project_store::{atomic_write_bytes, read_json_bounded, ProjectStore};
use crate::{InspectionError, ProjectHerdrAdapter};

const FILE: &str = "agent-attention.json";
const MAX_BYTES: u64 = 1024 * 1024;
const MAX_RECORDS: usize = 4096;
const INFER_IDLE_ADVANCE: bool = true;

#[derive(Clone)]
pub struct AgentAttentionService {
    store: ProjectStore,
    herdr: Arc<dyn ProjectHerdrAdapter>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    schema: u32,
    epochs: Vec<Epoch>,
    agents: Vec<StoredAgent>,
    #[serde(default)]
    retired: Vec<RetiredAgent>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Epoch {
    ordinal: u64,
    session_namespace: String,
    session_id: String,
    server_instance: String,
    first_seen_at: String,
}

#[derive(Deserialize, Serialize)]
struct StoredAgent {
    session_namespace: String,
    session_id: String,
    revision: String,
    #[serde(flatten)]
    record: AgentAttentionRecord,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RetiredAgent {
    session_namespace: String,
    session_id: String,
    pane_id: String,
    terminal_id: String,
    agent: String,
    epoch: u64,
    seq: u64,
}

fn retire(stored: &mut Stored, index: usize) -> Result<(), InspectionError> {
    let a = &stored.agents[index];
    let r = &a.record;
    if let Some(old) = stored.retired.iter_mut().find(|old| old.session_namespace == a.session_namespace
        && old.session_id == a.session_id && old.pane_id == r.pane_id
        && old.terminal_id == r.terminal_id && old.agent == r.agent && old.epoch == r.last.epoch)
    {
        old.seq = old.seq.max(r.last.seq);
    } else {
        if stored.retired.len() >= MAX_RECORDS { return Err(limit()); }
        stored.retired.push(RetiredAgent {
            session_namespace: a.session_namespace.clone(), session_id: a.session_id.clone(),
            pane_id: r.pane_id.clone(), terminal_id: r.terminal_id.clone(), agent: r.agent.clone(),
            epoch: r.last.epoch, seq: r.last.seq,
        });
    }
    Ok(())
}

fn now() -> String {
    time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).expect("UTC timestamp is representable")
}
fn invalid() -> InspectionError { InspectionError::new("invalid_agent_attention_change", "Invalid agent attention identity or observation") }
fn target_changed() -> InspectionError { InspectionError::new("agent_attention_target_changed", "Herdr session changed; refresh agent attention") }
fn valid_id(s: &str) -> bool { !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control) }
fn valid_status(s: &str) -> bool { matches!(s, "idle" | "working" | "blocked" | "done" | "unknown") }
fn limit() -> InspectionError { InspectionError::new("agent_attention_limit", "Agent attention storage limit reached; no records were removed") }
fn revision() -> String { uuid::Uuid::new_v4().to_string() }
fn key(e: u64, s: u64) -> (u64, u64) { (e, s) }
fn last_key(r: &AgentAttentionRecord) -> (u64, u64) { key(r.last.epoch, r.last.seq) }
fn stamp_key(s: &AgentAttentionStamp) -> (u64, u64) { key(s.epoch, s.seq) }
fn unseen(r: &AgentAttentionRecord) -> bool {
    r.completion.as_ref().is_some_and(|c| r.acked.as_ref().is_none_or(|a| stamp_key(a) < key(c.epoch, c.seq)))
}
fn stamp(mark: (u64, u64), now: &str) -> AgentAttentionStamp { AgentAttentionStamp { epoch: mark.0, seq: mark.1, at: now.to_owned() } }

fn observe(stored: &mut Stored, ns: &str, session: &str, observation: &AgentAttentionObservation, epoch: u64, now: &str) -> Result<bool, InspectionError> {
    let id = key(epoch, observation.seq);
    if stored.retired.iter().any(|old| old.session_namespace == ns && old.session_id == session
        && old.pane_id == observation.pane_id && old.terminal_id == observation.terminal_id
        && old.agent == observation.agent && old.epoch == epoch && observation.seq <= old.seq)
    {
        return Ok(false);
    }
    let index = stored.agents.iter().position(|a| a.session_namespace == ns && a.session_id == session && a.record.pane_id == observation.pane_id && a.record.historical_id.is_none());
    if let Some(index) = index {
        let r = &stored.agents[index].record;
        // A late literal completion is evidence even when a newer observation has
        // already advanced last, provided it belongs to the same incarnation.
        if id < last_key(r) {
            if r.last.epoch == epoch && r.agent == observation.agent && r.terminal_id == observation.terminal_id
                && observation.status == "done"
                && r.acked.as_ref().is_none_or(|a| stamp_key(a) < id)
                && r.completion.as_ref().is_none_or(|c| key(c.epoch, c.seq) < id)
            {
                let a = &mut stored.agents[index];
                a.record.completion = Some(AgentAttentionCompletion { epoch, seq: observation.seq, at: now.to_owned(), inferred: false });
                a.revision = revision();
                return Ok(true);
            }
            return Ok(false);
        }
        if r.agent != observation.agent || (r.last.epoch == epoch && r.terminal_id != observation.terminal_id) {
            // Equal-sequence identity changes are ambiguous: a delayed old incarnation
            // must not displace the current one. The unmatched live row stays visible.
            if id == last_key(r) { return Ok(false); }
            let was_unseen = unseen(r);
            retire(stored, index)?;
            if was_unseen {
                let a = &mut stored.agents[index];
                a.record.historical_id = Some(revision());
                a.revision = revision();
            } else {
                stored.agents.remove(index);
            }
        } else {
            let a = &mut stored.agents[index];
            let r = &mut a.record;
            let previous = last_key(r);
            let old_status = r.last.status.as_str();
            let mut completion = None;
            if epoch > r.last.epoch {
                if observation.status == "done" { completion = Some(false); }
                // A restored pane/name does not prove process identity across server restarts.
            } else if observation.status == "done" && (id > previous || old_status != "done") {
                completion = Some(false);
            } else if id > previous && observation.status == "idle" && (matches!(old_status, "working" | "blocked") || (old_status == "idle" && INFER_IDLE_ADVANCE)) {
                completion = Some(true);
            }
            let changed = id != previous || r.last.status != observation.status || r.terminal_id != observation.terminal_id;
            if let Some(inferred) = completion {
                if r.acked.as_ref().is_none_or(|a| stamp_key(a) < id) {
                    r.completion = Some(AgentAttentionCompletion { epoch: id.0, seq: id.1, at: now.to_owned(), inferred });
                }
            }
            if changed {
                r.terminal_id = observation.terminal_id.clone();
                r.last = AgentAttentionLast { epoch, seq: observation.seq, status: observation.status.clone(), observed_at: now.to_owned() };
                a.revision = revision();
            }
            return Ok(changed);
        }
    }
    if stored.agents.len() >= MAX_RECORDS { return Err(limit()); }
    stored.agents.push(StoredAgent {
        session_namespace: ns.to_owned(), session_id: session.to_owned(), revision: revision(),
        record: AgentAttentionRecord {
            pane_id: observation.pane_id.clone(), terminal_id: observation.terminal_id.clone(), agent: observation.agent.clone(), historical_id: None,
            last: AgentAttentionLast { epoch, seq: observation.seq, status: observation.status.clone(), observed_at: now.to_owned() },
            completion: (observation.status == "done").then(|| AgentAttentionCompletion { epoch, seq: observation.seq, at: now.to_owned(), inferred: false }),
            acked: None,
        },
    });
    Ok(true)
}

// GET can observe the end of a transition before queued intermediate frames.
fn observe_batch(stored: &mut Stored, ns: &str, session: &str, observations: &[AgentAttentionObservation], epoch: u64, now: &str) -> Result<bool, InspectionError> {
    let mut previous = std::collections::HashMap::<&str, &AgentAttentionObservation>::new();
    let mut changed = false;
    for observation in observations {
        changed |= observe(stored, ns, session, observation, epoch, now)?;
        if let Some(before) = previous.insert(&observation.pane_id, observation) {
            if before.agent == observation.agent && before.terminal_id == observation.terminal_id
                && before.seq < observation.seq && observation.status == "idle"
                && (matches!(before.status.as_str(), "working" | "blocked") || (before.status == "idle" && INFER_IDLE_ADVANCE))
            {
                if let Some(a) = stored.agents.iter_mut().find(|a| a.session_namespace == ns && a.session_id == session && a.record.pane_id == observation.pane_id && a.record.historical_id.is_none()) {
                    let r = &mut a.record;
                    let id = key(epoch, observation.seq);
                    if r.last.epoch == epoch && r.agent == observation.agent && r.terminal_id == observation.terminal_id
                        && id <= last_key(r) && r.acked.as_ref().is_none_or(|ack| stamp_key(ack) < id)
                        && r.completion.as_ref().is_none_or(|c| key(c.epoch, c.seq) < id)
                    {
                        r.completion = Some(AgentAttentionCompletion { epoch, seq: observation.seq, at: now.to_owned(), inferred: true });
                        a.revision = revision();
                        changed = true;
                    }
                }
            }
        }
    }
    Ok(changed)
}

fn refusal(code: &str, message: &str) -> AgentAttentionRefusal { AgentAttentionRefusal { code: code.into(), message: message.into() } }
fn apply_change(stored: &mut Stored, ns: &str, session: &str, change: &AgentAttentionChange, now: &str) -> Result<bool, AgentAttentionRefusal> {
    if let AgentAttentionChange::SeenHistorical { historical_id } = change {
        let index = stored.agents.iter().position(|a| a.session_namespace == ns && a.session_id == session && a.record.historical_id.as_ref() == Some(historical_id))
            .ok_or_else(|| refusal("agent_attention_target_changed", "Historical completion is no longer present"))?;
        stored.agents.remove(index);
        return Ok(true);
    }
    let AgentAttentionChange::Seen { pane_id, terminal_id, agent, transition } = change else {
        unreachable!();
    };
    let a = stored.agents.iter_mut().find(|a| a.session_namespace == ns && a.session_id == session && a.record.historical_id.is_none() && a.record.pane_id == *pane_id && a.record.agent == *agent)
        .ok_or_else(|| refusal("agent_attention_target_changed", "Agent identity changed"))?;
    let r = &mut a.record;
    let id = key(transition.epoch, transition.seq);
    if r.terminal_id != *terminal_id || id > last_key(r) {
        return Err(refusal("agent_attention_target_changed", "Agent identity changed"));
    }
    let changed = r.acked.as_ref().is_none_or(|a| stamp_key(a) < id);
    if changed { r.acked = Some(stamp(id, now)); }
    if changed { a.revision = revision(); }
    Ok(changed)
}

fn load(store: &ProjectStore) -> Result<Stored, InspectionError> {
    match store.state_dir().symlink_metadata(FILE) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Stored { schema: 1, ..Stored::default() }),
        Err(e) => return Err(InspectionError::new("agent_attention_read", e.to_string())),
        Ok(_) => {}
    }
    let stored: Stored = read_json_bounded(store.state_dir(), FILE, MAX_BYTES).map_err(|e| InspectionError::new(if e.code == "state_corrupt" || e.code == "unsafe_path" { "agent_attention_corrupt" } else { "agent_attention_read" }, e.message))?;
    validate_stored(&stored)?;
    Ok(stored)
}
fn validate_stored(s: &Stored) -> Result<(), InspectionError> {
    let corrupt = || InspectionError::new("agent_attention_corrupt", "Invalid agent attention store");
    if s.schema != 1 || s.agents.len() > MAX_RECORDS || s.epochs.len() > MAX_RECORDS || s.retired.len() > MAX_RECORDS { return Err(corrupt()); }
    let mut epochs = HashSet::new();
    for e in &s.epochs {
        if e.ordinal == 0 || e.session_namespace.len() != 64 || !e.session_namespace.bytes().all(|b| b.is_ascii_hexdigit()) || !valid_id(&e.session_id) || !valid_id(&e.server_instance) || e.first_seen_at.is_empty() || !epochs.insert((&e.session_namespace, &e.session_id)) { return Err(corrupt()); }
    }
    let mut retired = HashSet::new();
    for old in &s.retired {
        let e = s.epochs.iter().find(|e| e.session_namespace == old.session_namespace && e.session_id == old.session_id).ok_or_else(corrupt)?;
        if !valid_id(&old.pane_id) || !valid_id(&old.terminal_id) || !valid_id(&old.agent)
            || old.epoch == 0 || old.epoch > e.ordinal
            || !retired.insert((&old.session_namespace, &old.session_id, &old.pane_id, &old.terminal_id, &old.agent, old.epoch))
        { return Err(corrupt()); }
    }
    let mut records = HashSet::new();
    let mut revisions = HashSet::new();
    let mut historical_ids = HashSet::new();
    for a in &s.agents {
        let r = &a.record;
        let e = s.epochs.iter().find(|e| e.session_namespace == a.session_namespace && e.session_id == a.session_id).ok_or_else(corrupt)?;
        if !valid_id(&r.pane_id) || !valid_id(&r.terminal_id) || !valid_id(&r.agent) || !valid_status(&r.last.status) || r.last.epoch == 0 || r.last.epoch > e.ordinal || r.last.observed_at.is_empty() || uuid::Uuid::parse_str(&a.revision).is_err() || !revisions.insert(&a.revision) { return Err(corrupt()); }
        if let Some(id) = &r.historical_id {
            if uuid::Uuid::parse_str(id).is_err() || !historical_ids.insert(id) || !unseen(r) { return Err(corrupt()); }
        } else if !records.insert((&a.session_namespace, &a.session_id, &r.pane_id)) { return Err(corrupt()); }
        if r.completion.as_ref().is_some_and(|c| c.epoch == 0 || key(c.epoch, c.seq) > last_key(r) || c.at.is_empty()) || r.acked.as_ref().is_some_and(|v| v.epoch == 0 || stamp_key(v) > last_key(r) || v.at.is_empty()) { return Err(corrupt()); }
    }
    Ok(())
}
fn persist(store: &ProjectStore, s: &Stored) -> Result<(), InspectionError> {
    if s.agents.len() > MAX_RECORDS || s.epochs.len() > MAX_RECORDS || s.retired.len() > MAX_RECORDS { return Err(limit()); }
    let bytes = serde_json::to_vec_pretty(s).map_err(|e| InspectionError::new("agent_attention_outcome_unknown", e.to_string()))?;
    if bytes.len() as u64 > MAX_BYTES { return Err(limit()); }
    atomic_write_bytes(store.state_dir(), FILE, &bytes).map_err(|e| InspectionError::new("agent_attention_outcome_unknown", e.to_string()))
}
fn current<'a>(s: &'a Stored, ns: &str, session: &str) -> Option<&'a Epoch> { s.epochs.iter().find(|e| e.session_namespace == ns && e.session_id == session) }
fn register(s: &mut Stored, ns: &str, session: &str, instance: &str, now: &str) -> Result<(u64, bool), InspectionError> {
    if let Some(e) = s.epochs.iter_mut().find(|e| e.session_namespace == ns && e.session_id == session) {
        if e.server_instance == instance { return Ok((e.ordinal, false)); }
        e.ordinal = e.ordinal.checked_add(1).ok_or_else(limit)?;
        e.server_instance = instance.to_owned(); e.first_seen_at = now.to_owned();
        return Ok((e.ordinal, true));
    }
    if s.epochs.len() >= MAX_RECORDS { return Err(limit()); }
    s.epochs.push(Epoch { ordinal: 1, session_namespace: ns.into(), session_id: session.into(), server_instance: instance.into(), first_seen_at: now.into() });
    Ok((1, true))
}
fn response(s: &Stored, ns: &str, session: &str, instance: &str, epoch: u64, refused: Option<AgentAttentionRefusal>) -> AgentAttentionResponse {
    AgentAttentionResponse { session_id: session.into(), server_instance: instance.into(), epoch, records: s.agents.iter().filter(|a| a.session_namespace == ns && a.session_id == session).map(|a| a.record.clone()).collect(), refused }
}
fn validate_request(session: &str, request: &AgentAttentionChangeRequest) -> Result<(), InspectionError> {
    if !valid_id(session) || !valid_id(&request.server_instance) || request.observations.len() > 256 || request.observations.iter().any(|o| !valid_id(&o.pane_id) || !valid_id(&o.terminal_id) || !valid_id(&o.agent) || !valid_status(&o.status)) { return Err(invalid()); }
    if let Some(change) = &request.change {
        let valid = match change {
            AgentAttentionChange::Seen { pane_id, terminal_id, agent, transition } => valid_id(pane_id) && valid_id(terminal_id) && valid_id(agent) && transition.epoch > 0,
            AgentAttentionChange::SeenHistorical { historical_id } => uuid::Uuid::parse_str(historical_id).is_ok(),
        };
        if !valid { return Err(invalid()); }
    }
    Ok(())
}

impl AgentAttentionService {
    pub fn new(state_root: &Path, herdr: Arc<dyn ProjectHerdrAdapter>) -> Result<Self, InspectionError> {
        Ok(Self { store: ProjectStore::new(state_root.join("agent-attention"))?, herdr })
    }
    async fn prove(&self, session: &str, ns: &str) -> Result<SessionSnapshotResponse, InspectionError> {
        let endpoint = self.herdr.project_endpoint_identity(session).await?;
        let observed = self.herdr.project_snapshot_at_endpoint(session, &endpoint).await?;
        if observed.endpoint_identity != endpoint || observed.snapshot.session_id != session || !valid_id(&observed.snapshot.server_instance) || crate::space_tags::session_namespace(&self.herdr, session).await? != ns { return Err(target_changed()); }
        Ok(observed.snapshot)
    }
    pub async fn list(&self, session_id: &str) -> Result<AgentAttentionResponse, InspectionError> {
        if !valid_id(session_id) { return Err(invalid()); }
        let ns = crate::space_tags::session_namespace(&self.herdr, session_id).await?;
        let store = self.store.clone();
        let session = session_id.to_owned();
        let namespace = ns.clone();
        let (candidates, before) = tokio::task::spawn_blocking(move || {
            let s = load(&store)?;
            Ok::<_, InspectionError>((s.agents.iter().map(|a| a.revision.clone()).collect::<HashSet<_>>(), current(&s, &namespace, &session).map(|e| (e.ordinal, e.server_instance.clone()))))
        }).await.map_err(|e| InspectionError::new("agent_attention_read", e.to_string()))??;
        let snapshot = self.prove(session_id, &ns).await?;
        let store = self.store.clone(); let session = session_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let _lock = store.acquire_named_lock(".agent-attention.lock", "agent_attention_lock")?;
            let mut s = load(&store)?;
            let actual = current(&s, &ns, &session).map(|e| (e.ordinal, e.server_instance.clone()));
            if actual != before && actual.as_ref().is_some_and(|(_, instance)| instance != &snapshot.server_instance) { return Err(target_changed()); }
            let time = now();
            let (epoch, mut changed) = register(&mut s, &ns, &session, &snapshot.server_instance, &time)?;
            for agent in &snapshot.agents {
                if let Some(pane) = snapshot.panes.iter().find(|p| p.id == agent.pane_id) {
                    let o = AgentAttentionObservation { pane_id: agent.pane_id.clone(), terminal_id: pane.terminal_id.clone(), agent: agent.name.clone(), status: agent.status.clone(), seq: agent.state_change_seq };
                    if !valid_id(&o.pane_id) || !valid_id(&o.terminal_id) || !valid_id(&o.agent) || !valid_status(&o.status) { return Err(invalid()); }
                    // A proven snapshot can disambiguate an equal-sequence replacement
                    // only if no concurrent writer changed the candidate.
                    if let Some(index) = s.agents.iter().position(|a| a.session_namespace == ns && a.session_id == session
                        && a.record.historical_id.is_none() && a.record.pane_id == o.pane_id
                        && last_key(&a.record) == key(epoch, o.seq) && candidates.contains(&a.revision)
                        && (a.record.agent != o.agent || a.record.terminal_id != o.terminal_id))
                    {
                        retire(&mut s, index)?;
                        if unseen(&s.agents[index].record) {
                            s.agents[index].record.historical_id = Some(revision());
                            s.agents[index].revision = revision();
                        } else { s.agents.remove(index); }
                        changed = true;
                    }
                    changed |= observe(&mut s, &ns, &session, &o, epoch, &time)?;
                }
            }
            for index in (0..s.agents.len()).rev() {
                let a = &s.agents[index];
                if a.session_namespace == ns && a.session_id == session && a.record.historical_id.is_none()
                    && !snapshot.agents.iter().any(|agent| agent.pane_id == a.record.pane_id)
                    && candidates.contains(&a.revision)
                {
                    let was_unseen = unseen(&a.record);
                    retire(&mut s, index)?;
                    if was_unseen {
                        s.agents[index].record.historical_id = Some(revision());
                        s.agents[index].revision = revision();
                    } else {
                        s.agents.remove(index);
                    }
                    changed = true;
                }
            }
            if changed { persist(&store, &s)?; }
            let mut result = response(&s, &ns, &session, &snapshot.server_instance, epoch, None);
            result.records.retain(|r| r.historical_id.is_some() || snapshot.agents.iter().any(|a| a.pane_id == r.pane_id));
            Ok(result)
        }).await.map_err(|e| InspectionError::new("agent_attention_outcome_unknown", e.to_string()))?
    }
    pub async fn change(&self, session_id: &str, request: &AgentAttentionChangeRequest) -> Result<AgentAttentionResponse, InspectionError> {
        validate_request(session_id, request)?;
        let ns = crate::space_tags::session_namespace(&self.herdr, session_id).await?;
        let store = self.store.clone(); let namespace = ns.clone(); let session = session_id.to_owned();
        let before = tokio::task::spawn_blocking(move || { let s = load(&store)?; Ok::<_, InspectionError>(current(&s, &namespace, &session).map(|e| (e.ordinal, e.server_instance.clone()))) }).await.map_err(|e| InspectionError::new("agent_attention_read", e.to_string()))??;
        let proof = if before.as_ref().is_none_or(|(_, instance)| instance != &request.server_instance) {
            let snapshot = self.prove(session_id, &ns).await?;
            if snapshot.server_instance != request.server_instance { return Err(target_changed()); }
            true
        } else { false };
        let store = self.store.clone(); let session = session_id.to_owned(); let request = request.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = store.acquire_named_lock(".agent-attention.lock", "agent_attention_lock")?;
            let mut s = load(&store)?;
            let actual = current(&s, &ns, &session).map(|e| (e.ordinal, e.server_instance.clone()));
            if actual != before && actual.as_ref().is_none_or(|(_, instance)| instance != &request.server_instance) { return Err(target_changed()); }
            if !proof && actual.as_ref().is_none_or(|(_, instance)| instance != &request.server_instance) { return Err(target_changed()); }
            let time = now();
            let (epoch, mut changed) = register(&mut s, &ns, &session, &request.server_instance, &time)?;
            changed |= observe_batch(&mut s, &ns, &session, &request.observations, epoch, &time)?;
            let refused = match &request.change { Some(change) => match apply_change(&mut s, &ns, &session, change, &time) { Ok(did_change) => { changed |= did_change; None }, Err(refused) => Some(refused) }, None => None };
            if changed { persist(&store, &s)?; }
            Ok(response(&s, &ns, &session, &request.server_instance, epoch, refused))
        }).await.map_err(|e| InspectionError::new("agent_attention_outcome_unknown", e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::v1::AgentAttentionMark;
    use crate::{HerdrAdapter, SessionSubscription, TerminalSession, project_adapter::*};
    use cockpit_protocol::v1::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Notify;

    struct Adapter {
        instance: AtomicUsize,
        seq: AtomicUsize,
        present: AtomicBool,
        fail: AtomicBool,
        pause: AtomicBool,
        captured: Notify,
        release: Notify,
    }
    impl Adapter {
        fn new() -> Arc<Self> {
            Arc::new(Self { instance: AtomicUsize::new(0), seq: AtomicUsize::new(0), present: AtomicBool::new(true), fail: AtomicBool::new(false), pause: AtomicBool::new(false), captured: Notify::new(), release: Notify::new() })
        }
        fn instance(&self) -> String { format!("instance-{}", self.instance.load(Ordering::SeqCst)) }
    }
    fn unused<T>() -> Result<T, InspectionError> { Err(InspectionError::new("unused", "Unexpected adapter call")) }
    #[async_trait::async_trait]
    impl HerdrAdapter for Adapter {
        fn session_socket_path(&self, _: &str) -> Option<PathBuf> { Some(std::env::temp_dir()) }
        async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> { unused() }
        async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> { unused() }
        async fn sessions(&self) -> Result<SessionListResponse, InspectionError> { unused() }
        async fn session_snapshot(&self, session: &str) -> Result<SessionSnapshotResponse, InspectionError> {
            let present = self.present.load(Ordering::SeqCst);
            Ok(SessionSnapshotResponse {
                session_id: session.into(), server_instance: self.instance(), version: "test".into(), protocol: 1,
                focused_space_id: None, focused_tab_id: None, focused_pane_id: None, herdr_shell: None,
                spaces: vec![], tabs: vec![],
                panes: if present { vec![PaneSummary { id: "pane".into(), terminal_id: "terminal".into(), space_id: "space".into(), tab_id: "tab".into(), title: None, focused: false, agent: Some("agent".into()), agent_status: "idle".into(), revision: 0, cwd: None }] } else { vec![] },
                agents: if present { vec![AgentSummary { pane_id: "pane".into(), space_id: "space".into(), tab_id: "tab".into(), name: "agent".into(), status: "idle".into(), title: None, focused: false, state_change_seq: self.seq.load(Ordering::SeqCst) as u64 }] } else { vec![] },
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
            if self.fail.load(Ordering::SeqCst) { return Err(InspectionError::new("endpoint_identity_unavailable", "unavailable")); }
            Ok(self.instance())
        }
        async fn project_snapshot_at_endpoint(&self, session: &str, _: &str) -> Result<ProjectSnapshot, InspectionError> {
            let snapshot = self.session_snapshot(session).await?;
            if self.pause.swap(false, Ordering::SeqCst) {
                self.captured.notify_one();
                self.release.notified().await;
            }
            Ok(ProjectSnapshot { snapshot, endpoint_identity: self.instance(), boot_id: None, peer_process: None })
        }
        async fn project_inventory(&self, _: &str, _: &str) -> Result<ProjectInventory, InspectionError> { unused() }
        async fn project_worktree(&self, _: &str, _: &ProjectWorktreeRequest) -> Result<ProjectWorktreeResult, InspectionError> { unused() }
        async fn project_worktree_dirty(&self, _: &str, _: u32, _: u32) -> Result<WorktreeStatus, InspectionError> { unused() }
        async fn project_close_workspace(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> { unused() }
        async fn project_remove_worktree(&self, _: &str, _: &ProjectWorktreeRemoveRequest) -> Result<(), InspectionError> { unused() }
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self { Self(std::env::temp_dir().join(format!("cockpit-attention-test-{}", uuid::Uuid::new_v4()))) }
        fn service(&self, adapter: Arc<Adapter>) -> AgentAttentionService { AgentAttentionService::new(&self.0, adapter).unwrap() }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    fn request(adapter: &Adapter, observations: Vec<AgentAttentionObservation>, change: Option<AgentAttentionChange>) -> AgentAttentionChangeRequest {
        AgentAttentionChangeRequest { server_instance: adapter.instance(), observations, change }
    }

    fn observation(status: &str, seq: u64) -> AgentAttentionObservation {
        AgentAttentionObservation { pane_id: "pane".into(), terminal_id: "terminal".into(), agent: "agent".into(), status: status.into(), seq }
    }
    fn see(seq: u64) -> AgentAttentionChange {
        AgentAttentionChange::Seen { pane_id: "pane".into(), terminal_id: "terminal".into(), agent: "agent".into(), transition: AgentAttentionMark { epoch: 1, seq } }
    }
    fn update(s: &mut Stored, status: &str, seq: u64) {
        observe(s, "namespace", "session", &observation(status, seq), 1, "time").unwrap();
    }

    #[test]
    fn completion_detection_and_reordering() {
        for initial in ["working", "blocked", "idle"] {
            let mut s = Stored::default();
            update(&mut s, initial, 1);
            update(&mut s, "idle", 2);
            assert_eq!(s.agents[0].record.completion.as_ref().unwrap().inferred, initial != "idle" || INFER_IDLE_ADVANCE);
            assert!(unseen(&s.agents[0].record));
            assert!(!observe(&mut s, "namespace", "session", &observation("working", 1), 1, "later").unwrap());
            assert!(!observe(&mut s, "namespace", "session", &observation("idle", 2), 1, "later").unwrap());
        }
        let mut s = Stored::default();
        update(&mut s, "working", 1);
        update(&mut s, "done", 1);
        assert!(!s.agents[0].record.completion.as_ref().unwrap().inferred);
    }

    #[test]
    fn late_done_preserves_completion_without_regressing_last_or_ack() {
        let mut s = Stored::default();
        update(&mut s, "working", 1);
        update(&mut s, "working", 3);
        assert!(observe(&mut s, "namespace", "session", &observation("done", 2), 1, "later").unwrap());
        assert_eq!(s.agents[0].record.last.seq, 3);
        assert_eq!(s.agents[0].record.completion.as_ref().unwrap().seq, 2);
        apply_change(&mut s, "namespace", "session", &see(2), "time").unwrap();
        assert!(!observe(&mut s, "namespace", "session", &observation("done", 2), 1, "later").unwrap());
        assert!(!unseen(&s.agents[0].record));
        let mut stale = observation("done", 1);
        stale.terminal_id = "old".into();
        assert!(!observe(&mut s, "namespace", "session", &stale, 1, "later").unwrap());
        assert_eq!(s.agents[0].record.acked.as_ref().unwrap().seq, 2);
    }

    #[test]
    fn queued_transitions_after_initial_snapshot_preserve_completion() {
        let mut s = Stored::default();
        update(&mut s, "idle", 2);
        let batch = [observation("working", 1), observation("idle", 2)];
        assert!(observe_batch(&mut s, "namespace", "session", &batch, 1, "later").unwrap());
        assert_eq!(s.agents[0].record.last.seq, 2);
        assert!(s.agents[0].record.completion.as_ref().unwrap().inferred);
        assert!(unseen(&s.agents[0].record));
        apply_change(&mut s, "namespace", "session", &see(2), "time").unwrap();
        assert!(!observe_batch(&mut s, "namespace", "session", &batch, 1, "later").unwrap());
        assert!(!unseen(&s.agents[0].record));
    }

    #[test]
    fn seen_flip_preserves_pending_ack_and_monotonic_seen() {
        for next_seq in [1, 2] {
            let mut s = Stored::default();
            update(&mut s, "done", 1);
            update(&mut s, "idle", next_seq);
            assert!(unseen(&s.agents[0].record));
            apply_change(&mut s, "namespace", "session", &see(1), "time").unwrap();
            assert!(!unseen(&s.agents[0].record));
            update(&mut s, "working", 3);
            update(&mut s, "done", 4);
            apply_change(&mut s, "namespace", "session", &see(4), "time").unwrap();
            apply_change(&mut s, "namespace", "session", &see(1), "time").unwrap();
            assert_eq!(s.agents[0].record.acked.as_ref().unwrap().seq, 4);
        }
    }

    #[test]
    fn epoch_restart_is_fail_visible_and_retains_unseen() {
        let mut s = Stored::default();
        update(&mut s, "done", 1);
        observe(&mut s, "namespace", "session", &observation("idle", 1), 2, "later").unwrap();
        assert!(unseen(&s.agents[0].record));
        assert_eq!(s.agents[0].record.completion.as_ref().unwrap().epoch, 1);
        let mut s = Stored::default();
        update(&mut s, "idle", 1);
        observe(&mut s, "namespace", "session", &observation("idle", 1), 2, "later").unwrap();
        assert!(s.agents[0].record.completion.is_none());
    }

    #[test]
    fn incarnation_changes_and_stale_observations() {
        let mut s = Stored::default();
        update(&mut s, "done", 4);
        let mut stale = observation("idle", 3);
        stale.terminal_id = "old".into();
        observe(&mut s, "namespace", "session", &stale, 1, "later").unwrap();
        assert_eq!(s.agents[0].record.terminal_id, "terminal");
        stale.seq = 4;
        observe(&mut s, "namespace", "session", &stale, 1, "later").unwrap();
        assert_eq!(s.agents[0].record.terminal_id, "terminal");
        let mut replacement = observation("idle", 5);
        replacement.terminal_id = "new".into();
        observe(&mut s, "namespace", "session", &replacement, 1, "later").unwrap();
        assert!(unseen(&s.agents[0].record));
        let historical_id = s.agents[0].record.historical_id.clone().unwrap();
        assert!(s.agents[1].record.completion.is_none());
        replacement.agent = "other".into();
        replacement.seq = 6;
        observe(&mut s, "namespace", "session", &replacement, 1, "later").unwrap();
        assert_eq!(s.agents[1].record.agent, "other");
        apply_change(&mut s, "namespace", "session", &AgentAttentionChange::SeenHistorical { historical_id }, "time").unwrap();
        assert_eq!(s.agents.len(), 1);
        assert_eq!(s.agents[0].record.agent, "other");
    }

    #[test]
    fn storage_roundtrip_rejects_corruption_and_preserves_data() {
        let root = std::env::temp_dir().join(format!("cockpit-attention-test-{}", uuid::Uuid::new_v4()));
        let store = ProjectStore::new(root.clone()).unwrap();
        let mut s = Stored { schema: 1, ..Stored::default() };
        let namespace = "a".repeat(64);
        register(&mut s, &namespace, "session", "instance", "time").unwrap();
        observe(&mut s, &namespace, "session", &observation("done", 1), 1, "time").unwrap();
        persist(&store, &s).unwrap();
        let loaded = load(&store).unwrap();
        assert!(unseen(&loaded.agents[0].record));
        assert!(serde_json::to_value(&loaded.agents[0].record).unwrap().get("dismissed").is_none());
        apply_change(&mut s, &namespace, "session", &see(1), "time").unwrap();
        persist(&store, &s).unwrap();
        let loaded = load(&store).unwrap();
        assert!(!unseen(&loaded.agents[0].record));
        assert_eq!(loaded.agents[0].record.acked.as_ref().unwrap().seq, 1);
        std::fs::write(root.join(FILE), b"{broken").unwrap();
        assert_eq!(load(&store).err().unwrap().code, "agent_attention_corrupt");
        assert_eq!(std::fs::read(root.join(FILE)).unwrap(), b"{broken");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removed_attention_operations_are_rejected() {
        for change in [
            serde_json::json!({ "kind": "dismiss", "pane_id": "pane", "terminal_id": "terminal", "agent": "agent", "transition": { "epoch": 1, "seq": 1 } }),
            serde_json::json!({ "kind": "restore", "pane_id": "pane", "agent": "agent" }),
            serde_json::json!({ "kind": "restore_all" }),
        ] {
            assert!(serde_json::from_value::<AgentAttentionChange>(change).is_err());
        }
    }

    #[test]
    fn store_validation_rejects_invalid_records() {
        let mut s = Stored { schema: 1, ..Stored::default() };
        let namespace = "a".repeat(64);
        register(&mut s, &namespace, "session", "instance", "time").unwrap();
        observe(&mut s, &namespace, "session", &observation("idle", 1), 1, "time").unwrap();
        assert!(validate_stored(&s).is_ok());
        s.agents[0].record.last.status = "invalid".into();
        assert_eq!(validate_stored(&s).err().unwrap().code, "agent_attention_corrupt");
    }

    #[tokio::test]
    async fn two_services_merge_seen_and_reject_stale_instances() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let first = fixture.service(adapter.clone());
        let second = fixture.service(adapter.clone());
        first.change("session", &request(&adapter, vec![observation("done", 2)], None)).await.unwrap();
        let older = request(&adapter, vec![], Some(see(1)));
        let newer = request(&adapter, vec![], Some(see(2)));
        let (a, b) = tokio::join!(first.change("session", &older), second.change("session", &newer));
        a.unwrap();
        b.unwrap();
        let result = first.change("session", &request(&adapter, vec![], None)).await.unwrap();
        assert_eq!(result.records[0].acked.as_ref().unwrap().seq, 2);
        adapter.instance.store(1, Ordering::SeqCst);
        let current = second.change("session", &request(&adapter, vec![], None)).await.unwrap();
        assert_eq!(current.epoch, 2);
        assert_eq!(first.change("session", &newer).await.unwrap_err().code, "agent_attention_target_changed");
    }

    #[tokio::test]
    async fn prune_requires_proof_and_does_not_remove_concurrently_changed_record() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let first = fixture.service(adapter.clone());
        let second = fixture.service(adapter.clone());
        first.change("session", &request(&adapter, vec![observation("idle", 1)], None)).await.unwrap();
        adapter.present.store(false, Ordering::SeqCst);
        adapter.fail.store(true, Ordering::SeqCst);
        assert!(first.list("session").await.is_err());
        assert_eq!(load(&first.store).unwrap().agents.len(), 1);
        adapter.fail.store(false, Ordering::SeqCst);
        adapter.pause.store(true, Ordering::SeqCst);
        let listing = tokio::spawn(async move { first.list("session").await });
        adapter.captured.notified().await;
        second.change("session", &request(&adapter, vec![observation("working", 2)], None)).await.unwrap();
        adapter.release.notify_one();
        listing.await.unwrap().unwrap();
        assert_eq!(load(&second.store).unwrap().agents.len(), 1);
        second.list("session").await.unwrap();
        assert!(load(&second.store).unwrap().agents.is_empty());
    }

    #[tokio::test]
    async fn disappeared_completion_is_returned_until_explicitly_seen() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let service = fixture.service(adapter.clone());
        service.change("session", &request(&adapter, vec![observation("done", 1)], None)).await.unwrap();
        adapter.present.store(false, Ordering::SeqCst);
        let listed = service.list("session").await.unwrap();
        let id = listed.records[0].historical_id.clone().unwrap();
        assert!(unseen(&listed.records[0]));
        assert_eq!(service.list("session").await.unwrap().records[0].historical_id.as_ref(), Some(&id));
        let seen = AgentAttentionChange::SeenHistorical { historical_id: id };
        let result = service.change("session", &request(&adapter, vec![], Some(seen))).await.unwrap();
        assert!(result.records.is_empty());
        assert!(service.list("session").await.unwrap().records.is_empty());
    }

    #[tokio::test]
    async fn snapshot_advance_does_not_acknowledge_queued_new_completion() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let service = fixture.service(adapter.clone());
        service.change("session", &request(&adapter, vec![observation("done", 1)], Some(see(1)))).await.unwrap();
        adapter.seq.store(3, Ordering::SeqCst);
        let listed = service.list("session").await.unwrap();
        assert_eq!(listed.records[0].acked.as_ref().unwrap().seq, 1);
        assert!(!unseen(&listed.records[0]));

        let queued = vec![observation("working", 2), observation("done", 3), observation("idle", 3)];
        let result = service.change("session", &request(&adapter, queued.clone(), None)).await.unwrap();
        assert!(unseen(&result.records[0]));
        assert_eq!(result.records[0].completion.as_ref().unwrap().seq, 3);
        assert!(!result.records[0].completion.as_ref().unwrap().inferred);
        let result = service.change("session", &request(&adapter, vec![], Some(see(3)))).await.unwrap();
        assert!(!unseen(&result.records[0]));
        let result = service.change("session", &request(&adapter, queued, None)).await.unwrap();
        assert!(!unseen(&result.records[0]));
    }

    #[tokio::test]
    async fn retired_incarnation_fences_delayed_observations_after_historical_seen() {
        let fixture = Fixture::new();
        let adapter = Adapter::new();
        let service = fixture.service(adapter.clone());
        service.change("session", &request(&adapter, vec![observation("done", 1)], None)).await.unwrap();
        adapter.present.store(false, Ordering::SeqCst);
        let historical = service.list("session").await.unwrap().records[0].historical_id.clone().unwrap();
        let delayed = vec![observation("done", 1), observation("idle", 1)];
        let result = service.change("session", &request(&adapter, delayed.clone(), None)).await.unwrap();
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0].historical_id.as_ref(), Some(&historical));
        service.change("session", &request(&adapter, vec![], Some(AgentAttentionChange::SeenHistorical { historical_id: historical }))).await.unwrap();
        let result = service.change("session", &request(&adapter, delayed, None)).await.unwrap();
        assert!(result.records.is_empty());
        let result = service.change("session", &request(&adapter, vec![observation("done", 2)], None)).await.unwrap();
        assert_eq!(result.records.len(), 1);
        assert!(unseen(&result.records[0]));
    }
}
