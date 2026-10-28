use cockpit_core::orchestration::{Actor, OrchestrationService, herdr::{OrchestrationHerdr, RuntimePane, RuntimeView}};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_protocol::orchestration::{MessageKind, OperatorOrigin, OrchestrationAction, OrchestrationMutationRequest, OrchestrationSnapshotRequest, RunKind, RunStage};
use serde_json::json;

use super::{CliError, args::OrchestrationArgs, output::emit, resolve};
use crate::endpoint::{AmbientEndpoint, resolve_endpoint};

fn endpoint(common: &OrchestrationArgs, service: &OrchestrationService) -> Result<crate::endpoint::Endpoint, CliError> {
    match common.endpoint() {
        Ok(endpoint) => Ok(endpoint),
        Err(error) if error.code == "orchestration_usage"
            && common.herdr_session.is_none() => {
            // Credential data is only a session-discovery fallback, never authority
            // for the operation. Explicit or inherited endpoints take precedence.
            let Some(Actor::Agent(caller)) = super::caller::job_actor(common)? else { return Err(error) };
            let job = service.job_registration_run(&caller)?;
            resolve_endpoint(common.herdr.clone(), Some(job.session_id), common.herdr_socket.clone(), &AmbientEndpoint::from_process())
                .map_err(|error| CliError::new(&error.code, error.message))
        }
        Err(error) => Err(error),
    }
}

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

/// Integration: route `agent send REF --text TEXT [--json]` to this function.
/// The common endpoint/config options are reused without credential admission.
pub(super) async fn send(common: &OrchestrationArgs, reference: &str, text: String, json_output: bool) -> Result<(), CliError> {
    if reference.trim().is_empty() || text.trim().is_empty() {
        return Err(CliError::usage("agent send requires a reference and nonempty text"));
    }
    let configuration = common.configuration()?;
    let service = OrchestrationService::open(&configuration)?;
    let endpoint = endpoint(common, &service)?;
    let session = endpoint.session;
    let adapter = HerdrCliAdapter::new(endpoint.config);
    let snapshot = service.snapshot_saved(&OrchestrationSnapshotRequest { session_id: session.clone(), root_id: common.root.clone() })?;
    let runtime = adapter.runtime(&session).await?;
    let candidates = pane_candidates(reference, &runtime);
    if candidates.len() > 1 {
        let names = candidates.iter().take(5).map(|pane| format!("{}  {}  {}", pane.pane_id, pane.agent_name.as_deref().unwrap_or(""), pane.workspace_label)).collect::<Vec<_>>().join(", ");
        return Err(CliError::new("ambiguous_reference", format!("Reference matches multiple agents: {names}")));
    }
    let run_ref = match resolve::run(&snapshot, reference) {
        Ok(id) => Some(id),
        Err(error) if error.code == "reference_not_found" => None,
        Err(error) => return Err(error),
    };
    let pane = candidates.first().copied();
    let pane_run = pane.and_then(|pane| {
        let mut matching = snapshot.runs.iter().filter(|run| run.stage != RunStage::Closed && location_pane(run) == Some(pane.pane_id.as_str()));
        let first = matching.next();
        (matching.next().is_none()).then_some(first).flatten()
    });
    if let (Some(id), Some(pane)) = (&run_ref, pane) {
        if snapshot.runs.iter().find(|run| &run.run_id == id).and_then(location_pane) != Some(pane.pane_id.as_str()) {
            return Err(CliError::new("ambiguous_reference", "Reference matches different run and agent targets"));
        }
    }
    let run = run_ref.as_ref().and_then(|id| snapshot.runs.iter().find(|run| &run.run_id == id)).or(pane_run);
    if let Some(run) = run {
        if delivery_route(Some(run.kind)) == DeliveryRoute::Liaison {
            return Err(CliError::new("liaison_target", "Cannot send to a liaison pane"));
        }
        if delivery_route(Some(run.kind)) == DeliveryRoute::Inbox {
            let message_id = uuid::Uuid::new_v4().to_string();
            eprintln!("Message ID: {message_id}");
            let response = service.mutate_resolved(
                &Actor::Operator(OperatorOrigin::Native),
                OrchestrationMutationRequest { session_id: session, expected_revision: None,
                    action: OrchestrationAction::MessageSend { message_id: message_id.clone(), to_run_id: run.run_id.clone(),
                        kind: MessageKind::Instruction, text, in_reply_to: None,
                        original_task_revision: None, original_claim_id: None } },
                &cockpit_core::orchestration::ResolvedMutationInput::default(),
            )?;
            return emit(&json!({"delivery":"inbox", "run_id":run.run_id, "message_id":message_id, "result":response}), json_output);
        }
    }
    let pane = pane.or_else(|| run.and_then(location_pane).and_then(|id|
        runtime.panes.iter().find(|pane| pane.pane_id == id && pane.agent_kind.as_deref().is_some_and(|kind| !kind.is_empty()))))
        .ok_or_else(|| CliError::new("reference_not_found", "No live agent matches the reference"))?;
    if snapshot.runs.iter().any(|run| run.stage != RunStage::Closed
        && location_pane(run) == Some(pane.pane_id.as_str())
        && delivery_route(Some(run.kind)) == DeliveryRoute::Liaison) {
        return Err(CliError::new("liaison_target", "Cannot send to a liaison pane"));
    }
    // Never prompt a pane associated with multiple managed runs as if unmanaged.
    if snapshot.runs.iter().filter(|run| run.stage != RunStage::Closed && location_pane(run) == Some(pane.pane_id.as_str())).count() > 1
        || snapshot.runs.iter().any(|run| run.stage != RunStage::Closed && run.kind == RunKind::Worker && location_pane(run) == Some(pane.pane_id.as_str())) {
        return Err(CliError::new("ambiguous_reference", "Agent pane has ambiguous managed run bindings"));
    }
    adapter.prompt_unmanaged_agent(&session, &runtime, pane, &text).await?;
    emit(&json!({"delivery":"prompted", "pane_id":pane.pane_id}), json_output)
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

