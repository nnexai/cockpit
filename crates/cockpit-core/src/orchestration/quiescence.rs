use std::{collections::HashSet, time::Duration};

use cockpit_protocol::orchestration::{ActorRef, DispatchStep, MessageKind, OrchestrationSnapshotRequest, Presence, RunStage, RuntimeObservation};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use super::{board_digest, herdr::RuntimeView, messages, projection, OrchestrationService};
use crate::InspectionError;

#[cfg(test)]
mod tests;

impl OrchestrationService {
    /// Reconcile once per semantic board state, with no periodic timer after a quiet window.
    pub(crate) fn record_quiescence(&self, runtime: &RuntimeView, session_id: &str, quiet: Duration) -> Result<Option<Duration>, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let now = OffsetDateTime::now_utc();
        let now_text = super::now();
        let roots: Vec<_> = state.runs.iter().filter(|r| r.session_id == session_id && r.parent_run_id.is_none() && r.root_id == r.run_id && r.stage != RunStage::Closed && super::has_current_binding(&state, r)).map(|r| r.run_id.clone()).collect();
        let mut deadline: Option<Duration> = None;
        let mut changed = false;
        for root_id in roots {
            let runs: Vec<_> = state.runs.iter().filter(|r| r.session_id == session_id && r.root_id == root_id).collect();
            let ids: HashSet<_> = runs.iter().map(|r| r.run_id.as_str()).collect();
            let live: Vec<_> = runs.iter().copied().filter(|r| r.stage != RunStage::Closed).collect();
            let (observation, _) = projection::observe(&live, Ok(runtime.clone()), &now_text);
            let RuntimeObservation::Fresh { runs: observed, .. } = observation else { continue };
            if !observed.iter().any(|o| o.run_id == root_id && o.presence == Presence::Present) { continue; }
            if live.iter().any(|r| {
                let plan_failed = r.dispatch.as_ref().is_some_and(|d| d.step == DispatchStep::PlanFailed);
                !plan_failed && (matches!(r.stage, RunStage::Starting | RunStage::Queued)
                    || r.dispatch.as_ref().is_some_and(|d| !d.agent_started))
            }) || observed.iter().any(|o| o.agent_status.as_deref() == Some("working")) { continue; }
            let parse = |s: &str| OffsetDateTime::parse(s, &Rfc3339).ok();
            let last = runs.iter().filter_map(|r| parse(&r.updated_at))
                .chain(state.messages.iter().filter(|m| ids.contains(m.to_run_id.as_str()) || matches!(&m.from, ActorRef::Run { run_id } if ids.contains(run_id.as_str()))).filter_map(|m| parse(&m.created_at)))
                .chain(state.operations.iter().filter(|o| o.root_id == root_id).filter_map(|o| parse(&o.updated_at)))
                .chain(observed.iter().filter_map(|o| o.state_changed_at.as_deref().and_then(parse)))
                .chain(state.owned_resources.iter().filter(|r| r.root_id == root_id).filter_map(|r| parse(&r.cleanup.updated_at)))
                .max().unwrap_or(now);
            let elapsed = now - last;
            let quiet_time = time::Duration::try_from(quiet).unwrap_or(time::Duration::MAX);
            if elapsed < quiet_time {
                let remaining = quiet_time - elapsed;
                let delay = Duration::try_from(remaining).unwrap_or(quiet);
                if !delay.is_zero() { deadline = Some(deadline.map_or(delay, |old| old.min(delay))); }
                continue;
            }
            let board = self.board_for_root(&locked, &root_id)?;
            let snapshot = projection::snapshot(&OrchestrationSnapshotRequest { session_id: session_id.into(), root_id: Some(root_id.clone()) }, &state, String::new(), vec![board], Ok(runtime.clone()), &now_text, self.store.identity())?;
            let digest = board_digest::digest(&snapshot, &root_id, Some(runtime), &now_text)?;
            if digest.open_work == 0 { continue; }
            let reconcile = format!("reconcile:{root_id}:{}", digest.key);
            let stalled = format!("stalled:{root_id}:{}", digest.key);
            let (id, text) = if !state.messages.iter().any(|m| m.message_id == reconcile) {
                let checklist = board_digest::RECONCILE_CHECKLIST.replace("board quiet,", &format!("board quiet {} min,", quiet.as_secs() / 60));
                (reconcile, format!("{checklist}\n\n{}", digest.text))
            } else if !state.messages.iter().any(|m| m.message_id == stalled) {
                (stalled, "Work remains stalled after the supervisor reconciled this board.".to_owned())
            } else { continue };
            messages::append(&mut state, messages::AppendMessage { images: Vec::new(), thread_id: None, handoff: None, from: ActorRef::Dispatcher, to_run_id: &root_id, message_id: &id, kind: MessageKind::Observation, text: &text, in_reply_to: None, report: None, stale: false, escalated_from: None })?;
            changed = true;
        }
        if changed { let revision = locked.save(&mut state)?; self.revision.send_replace(revision); }
        Ok(deadline)
    }
}
