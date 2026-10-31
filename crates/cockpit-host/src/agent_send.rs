use cockpit_core::{InspectionError, orchestration::{Actor, OrchestrationService, herdr::{OrchestrationHerdr, RuntimePane, RuntimeView}}};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_protocol::orchestration::{AgentSendTarget, MessageKind, OperatorOrigin, OrchestrationAction, OrchestrationMutationRequest, OrchestrationSnapshotRequest, Run, RunKind, RunStage};
use serde_json::json;

#[derive(Debug, PartialEq, Eq)]
enum DeliveryRoute { Inbox, Prompt, Liaison }

fn delivery_route(kind: Option<RunKind>) -> DeliveryRoute {
    match kind {
        Some(RunKind::Worker) => DeliveryRoute::Inbox,
        Some(RunKind::Supervisor) => DeliveryRoute::Liaison,
        Some(RunKind::Adopted) | None => DeliveryRoute::Prompt,
    }
}

fn pane_candidates<'a>(reference: &str, runtime: &'a RuntimeView) -> Vec<&'a RuntimePane> {
    let agents: Vec<_> = runtime.panes.iter().filter(|p| p.agent_kind.as_deref().is_some_and(|s| !s.is_empty())).collect();
    let exact: Vec<_> = agents.iter().copied().filter(|p| p.pane_id == reference).collect();
    if !exact.is_empty() { return exact; }
    agents.into_iter().filter(|p| p.agent_name.as_deref().is_some_and(|name| name.eq_ignore_ascii_case(reference))
        || p.workspace_label.eq_ignore_ascii_case(reference)).collect()
}

fn location_pane(run: &cockpit_protocol::orchestration::Run) -> Option<&str> {
    run.attachment.as_ref()?.location.as_ref().map(|location| location.pane_id.as_str())
}

// A reviewed direct-send target is only a fence, never authority to route to a
// managed run. Check it before resolving a reference or selecting a route.
fn check_direct_target(
    expected: &AgentSendTarget, reference: &str, runtime: &RuntimeView, runs: &[Run],
) -> Result<(), InspectionError> {
    let stale = || InspectionError::new("stale_identity", "The selected agent has changed; select it again");
    if reference != expected.pane_id || runtime.endpoint_identity != expected.endpoint_identity
        || expected.terminal_id.is_empty() || expected.agent_kind.is_empty() {
        return Err(stale());
    }
    let mut matches = runtime.panes.iter().filter(|pane| pane.pane_id == expected.pane_id);
    let pane = matches.next().ok_or_else(stale)?;
    if matches.next().is_some()
        || runtime.panes.iter().filter(|pane| pane.terminal_id.as_deref() == Some(expected.terminal_id.as_str())).count() != 1
        || pane.workspace_id != expected.workspace_id || pane.tab_id != expected.tab_id
        || pane.terminal_id.as_deref() != Some(expected.terminal_id.as_str())
        || pane.agent_kind.as_deref() != Some(expected.agent_kind.as_str())
        || pane.agent_name != expected.agent_name
        || pane.native_session_id != expected.native_session_id
        || pane.native_session_file != expected.native_session_file
        || pane.state_changed_at != expected.state_changed_at
        || runs.iter().any(|run| run.stage != RunStage::Closed && run.attachment.as_ref()
            .and_then(|attachment| attachment.location.as_ref())
            .is_some_and(|location| location.pane_id == expected.pane_id
                || location.terminal_id.as_deref() == Some(expected.terminal_id.as_str())))
    {
        return Err(stale());
    }
    Ok(())
}

pub async fn send_to_agent<F>(
    service: &OrchestrationService, adapter: &HerdrCliAdapter, session: &str,
    root: Option<String>, reference: &str, text: String, origin: OperatorOrigin, announce: bool, idle_only: bool,
    expected_target: Option<AgentSendTarget>, resolve_run: F,
) -> Result<serde_json::Value, InspectionError>
where F: FnOnce(&cockpit_protocol::orchestration::OrchestrationSnapshot, &str) -> Result<Option<String>, InspectionError> {
    if reference.trim().is_empty() || text.trim().is_empty() {
        return Err(InspectionError::new("orchestration_usage", "agent send requires a reference and nonempty text"));
    }
    let snapshot = service.snapshot_saved(&OrchestrationSnapshotRequest { session_id: session.to_owned(), root_id: root })?;
    let runtime = adapter.runtime(session).await?;
    if let Some(expected) = expected_target.as_ref() {
        check_direct_target(expected, reference, &runtime, &snapshot.runs)?;
    }
    let candidates = pane_candidates(reference, &runtime);
    if candidates.len() > 1 {
        let names = candidates.iter().take(5).map(|pane| format!("{}  {}  {}", pane.pane_id, pane.agent_name.as_deref().unwrap_or(""), pane.workspace_label)).collect::<Vec<_>>().join(", ");
        return Err(InspectionError::new("ambiguous_reference", format!("Reference matches multiple agents: {names}")));
    }
    let run_ref = resolve_run(&snapshot, reference)?;
    let pane = candidates.first().copied();
    if idle_only && !pane.is_some_and(|p| p.pane_id == reference && p.agent_status.as_deref().is_some_and(|status| status.eq_ignore_ascii_case("idle"))) {
        return Err(InspectionError::new("agent_not_idle", "Agent is no longer idle or is unavailable"));
    }
    let pane_run = pane.and_then(|pane| {
        let mut matching = snapshot.runs.iter().filter(|run| run.stage != RunStage::Closed && location_pane(run) == Some(pane.pane_id.as_str()));
        let first = matching.next();
        (matching.next().is_none()).then_some(first).flatten()
    });
    if let (Some(id), Some(pane)) = (&run_ref, pane) {
        if snapshot.runs.iter().find(|run| &run.run_id == id).and_then(location_pane) != Some(pane.pane_id.as_str()) {
            return Err(InspectionError::new("ambiguous_reference", "Reference matches different run and agent targets"));
        }
    }
    let run = run_ref.as_ref().and_then(|id| snapshot.runs.iter().find(|run| &run.run_id == id)).or(pane_run);
    if let Some(run) = run {
        if delivery_route(Some(run.kind)) == DeliveryRoute::Liaison {
            return Err(InspectionError::new("liaison_target", "Cannot send to a liaison pane"));
        }
        if delivery_route(Some(run.kind)) == DeliveryRoute::Inbox {
            let message_id = uuid::Uuid::new_v4().to_string();
            if announce { eprintln!("Message ID: {message_id}"); }
            let response = service.mutate_resolved(
                &Actor::Operator(origin),
                OrchestrationMutationRequest { session_id: session.to_owned(), expected_revision: None,
                    action: OrchestrationAction::MessageSend { message_id: message_id.clone(), to_run_id: run.run_id.clone(),
                        kind: MessageKind::Instruction, text, in_reply_to: None,
                        original_task_revision: None, original_claim_id: None } },
                &cockpit_core::orchestration::ResolvedMutationInput::default(),
            )?;
            return Ok(json!({"delivery":"inbox", "run_id":run.run_id, "message_id":message_id, "result":response}));
        }
    }
    let pane = pane.or_else(|| run.and_then(location_pane).and_then(|id|
        runtime.panes.iter().find(|pane| pane.pane_id == id && pane.agent_kind.as_deref().is_some_and(|kind| !kind.is_empty()))))
        .ok_or_else(|| InspectionError::new("reference_not_found", "No live agent matches the reference"))?;
    if snapshot.runs.iter().any(|run| run.stage != RunStage::Closed
        && location_pane(run) == Some(pane.pane_id.as_str())
        && delivery_route(Some(run.kind)) == DeliveryRoute::Liaison) {
        return Err(InspectionError::new("liaison_target", "Cannot send to a liaison pane"));
    }
    // Never prompt a pane associated with multiple managed runs as if unmanaged.
    if snapshot.runs.iter().filter(|run| run.stage != RunStage::Closed && location_pane(run) == Some(pane.pane_id.as_str())).count() > 1
        || snapshot.runs.iter().any(|run| run.stage != RunStage::Closed && run.kind == RunKind::Worker && location_pane(run) == Some(pane.pane_id.as_str())) {
        return Err(InspectionError::new("ambiguous_reference", "Agent pane has ambiguous managed run bindings"));
    }
    adapter.prompt_unmanaged_agent(session, &runtime, pane, &text).await?;
    Ok(json!({"delivery":"prompted", "pane_id":pane.pane_id}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str, name: &str, space: &str) -> RuntimePane {
        RuntimePane {
            workspace_id: space.into(), workspace_label: space.into(), tab_id: id.into(),
            tab_label: String::new(), pane_id: id.into(), cwd: None, terminal_id: Some(id.into()),
            native_session_id: None, native_session_file: None, agent_name: Some(name.into()),
            agent_kind: Some("omp".into()), launch_pending: false, agent_status: Some("idle".into()),
            state_changed_at: None,
        }
    }

    fn direct_fixture() -> (RuntimeView, AgentSendTarget) {
        let mut observed = pane("pane", "alpha", "space");
        observed.tab_id = "tab".into();
        observed.terminal_id = Some("terminal".into());
        observed.native_session_id = Some("session".into());
        observed.native_session_file = Some("/session".into());
        observed.state_changed_at = Some("2026-10-01T00:00:00Z".into());
        let target = AgentSendTarget {
            endpoint_identity: "endpoint".into(), workspace_id: "space".into(), tab_id: "tab".into(),
            pane_id: "pane".into(), terminal_id: "terminal".into(), agent_kind: "omp".into(),
            agent_name: Some("alpha".into()), native_session_id: Some("session".into()),
            native_session_file: Some("/session".into()), state_changed_at: Some("2026-10-01T00:00:00Z".into()),
        };
        (RuntimeView { endpoint_identity: "endpoint".into(), boot_id: None, peer_process: None,
            focused_workspace_id: None, workspaces: vec![], panes: vec![observed] }, target)
    }

    #[test]
    fn reviewed_target_rejects_every_changed_identity_before_routing() {
        let (runtime, target) = direct_fixture();
        assert!(check_direct_target(&target, "pane", &runtime, &[]).is_ok());
        let mut changes: Vec<RuntimeView> = Vec::new();
        let mut changed = runtime.clone(); changed.endpoint_identity = "other".into(); changes.push(changed);
        let mut changed = runtime.clone(); changed.panes[0].workspace_id = "other".into(); changes.push(changed);
        let mut changed = runtime.clone(); changed.panes[0].tab_id = "other".into(); changes.push(changed);
        let mut changed = runtime.clone(); changed.panes[0].pane_id = "other".into(); changes.push(changed);
        let mut changed = runtime.clone(); changed.panes[0].terminal_id = Some("other".into()); changes.push(changed);
        let mut changed = runtime.clone(); changed.panes[0].agent_kind = Some("other".into()); changes.push(changed);
        let mut changed = runtime.clone(); changed.panes[0].agent_name = Some("other".into()); changes.push(changed);
        let mut changed = runtime.clone(); changed.panes[0].native_session_id = Some("other".into()); changes.push(changed);
        let mut changed = runtime.clone(); changed.panes[0].native_session_file = Some("/other".into()); changes.push(changed);
        let mut changed = runtime.clone(); changed.panes[0].state_changed_at = Some("later".into()); changes.push(changed);
        let mut changed = runtime.clone(); changed.panes.push(changed.panes[0].clone()); changes.push(changed);
        for changed in changes {
            assert_eq!(check_direct_target(&target, "pane", &changed, &[]).unwrap_err().code, "stale_identity");
        }
        assert_eq!(check_direct_target(&target, "alpha", &runtime, &[]).unwrap_err().code, "stale_identity");
    }

    #[test]
    fn reviewed_unmanaged_target_cannot_be_redirected_to_a_managed_run() {
        let (runtime, target) = direct_fixture();
        let mut run: Run = serde_json::from_value(json!({
            "session_id":"fixture","brief":"","run_id":"run","kind":"worker",
            "label":"Worker","root_id":"root","attempt":1,"stage":"active",
            "task_revision_at_start":null,"cleanup_policy":null,"assignment_generation":null,
            "launch_config":{"harness_kind":"omp","command":"omp","extra_args":[],
                "resume_args":["--resume","{session_id}"],"inbox_prompt_hints":false,"routes":[]},
            "dispatch":{"launch_tag":"tag","endpoint_identity":"endpoint","agent_started":false,
                "step":"launch_unknown","launch_attempt":0,"updated_at":"2026-10-06T00:00:00Z"},
            "annotations":[],
            "attachment":{"attachment_id":"attachment","epoch":1,"state":"disconnected",
                "location":{"endpoint_identity":"endpoint","session_id":"fixture","workspace_id":"space",
                    "tab_id":"tab","pane_id":"pane","launch_tag":"tag","terminal_id":"terminal"},
                "conversation":null,"process":null,"shell":null,"machine_id":null,
                "launch_mode":"shell_agent","launch_attempt":0,"updated_at":"2026-10-06T00:00:00Z"},
            "created_at":"2026-10-06T00:00:00Z","updated_at":"2026-10-06T00:00:00Z"
        })).unwrap();
        for kind in [RunKind::Worker, RunKind::Supervisor, RunKind::Adopted] {
            run.kind = kind;
            assert_eq!(check_direct_target(&target, "pane", &runtime, std::slice::from_ref(&run)).unwrap_err().code, "stale_identity");
        }
        run.attachment.as_mut().unwrap().location.as_mut().unwrap().pane_id = "old-pane".into();
        assert_eq!(check_direct_target(&target, "pane", &runtime, std::slice::from_ref(&run)).unwrap_err().code, "stale_identity");
        run.stage = RunStage::Closed;
        assert!(check_direct_target(&target, "pane", &runtime, &[run]).is_ok());
    }

    #[test]
    fn pane_id_is_exact_and_space_or_name_must_be_unique() {
        let runtime = RuntimeView {
            endpoint_identity: "endpoint".into(), boot_id: None, peer_process: None,
            focused_workspace_id: None, workspaces: vec![],
            panes: vec![pane("one", "alpha", "shared"), pane("two", "beta", "shared")],
        };
        assert_eq!(pane_candidates("one", &runtime)[0].agent_name.as_deref(), Some("alpha"));
        assert_eq!(pane_candidates("ALPHA", &runtime).len(), 1);
        assert_eq!(pane_candidates("shared", &runtime).len(), 2);
        assert!(pane_candidates("missing", &runtime).is_empty());
    }
    #[test]
    fn managed_workers_use_inbox_unmanaged_use_prompt_and_liaisons_are_refused() {
        assert_eq!(delivery_route(Some(RunKind::Worker)), DeliveryRoute::Inbox);
        assert_eq!(delivery_route(None), DeliveryRoute::Prompt);
        assert_eq!(delivery_route(Some(RunKind::Supervisor)), DeliveryRoute::Liaison);
        assert_eq!(delivery_route(Some(RunKind::Adopted)), DeliveryRoute::Prompt);
    }
}

