use cockpit_core::orchestration::{Actor, OrchestrationService};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_protocol::orchestration::OperatorOrigin;
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


/// Integration: route `agent send REF --text TEXT [--json]` to this function.
/// The common endpoint/config options are reused without credential admission.
pub(super) async fn send(common: &OrchestrationArgs, reference: &str, text: String, json_output: bool) -> Result<(), CliError> {
    let configuration = common.configuration()?;
    let service = OrchestrationService::open(&configuration)?;
    let endpoint = endpoint(common, &service)?;
    let adapter = HerdrCliAdapter::new(endpoint.config);
    let result = cockpit_host::agent_send::send_to_agent(&service, &adapter, &endpoint.session, common.root.clone(), reference, text, OperatorOrigin::Native, true, false, |snapshot, reference| {
        match resolve::run(snapshot, reference) {
            Ok(id) => Ok(Some(id)),
            Err(error) if error.code == "reference_not_found" => Ok(None),
            Err(error) => Err(cockpit_core::InspectionError::new(&error.code, error.message)),
        }
    }).await?;
    emit(&result, json_output)
}


