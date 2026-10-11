//! Supported ui_polish_runtime creation-receipt adapter. The bundled helper is
//! the same implementation used by the normal fixture driver, not a caller-
//! supplied command executor. /proc visibility is observational, not an OS sandbox.
use super::cleanup::{CleanupEffectOutcome, CleanupEvaluation};
use crate::InspectionError;
use cockpit_protocol::orchestration::{
    CleanupEffectReceipt, CleanupEffectState, CleanupReason, CleanupReasonClass,
    OwnedResourceIdentity, OwnedResourceKind, OwnedResourceRecord, Run,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;

const HELPER: &str = concat!(
    include_str!("../../../../scripts/verify/fixture_resources.py"),
    "\nimport sys\ntry:\n print(json.dumps(helper_operation(json.load(sys.stdin))))\n",
    "except (OSError, ValueError, KeyError) as error:\n",
    " print(json.dumps({'state':'unknown' if isinstance(error,OSError) else 'refused','evidence':str(error)}))\n"
);

fn reason(class: CleanupReasonClass, code: &str, message: &str) -> CleanupReason {
    CleanupReason { class, code: code.to_owned(), message: message.to_owned() }
}

async fn invoke(
    resource: &OwnedResourceIdentity,
    run_id: &str,
    root_id: &str,
    action: &str,
) -> Result<Value, InspectionError> {
    if !matches!(resource, OwnedResourceIdentity::FixtureSet { .. } | OwnedResourceIdentity::Scratch { .. }) {
        return Err(InspectionError::new("unsupported_resource", "Not a fixture or registered scratch identity"));
    }
    if !cfg!(target_os = "linux") {
        return Err(InspectionError::new("fixture_adapter_unavailable", "This fixture adapter requires Linux /proc and pidfds"));
    }
    let input = serde_json::to_vec(&json!({
        "action": action, "resource": resource,
        "run_id": run_id, "root_id": root_id,
    })).map_err(|error| InspectionError::new("fixture_request", error.to_string()))?;
    // -I ignores Python path/environment injection. Only this fixed bundled
    // helper receives the typed JSON request; no executable/argv is registered.
    let mut child = tokio::process::Command::new("python3")
        .args(["-I", "-c", HELPER])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .kill_on_drop(true).spawn()
        .map_err(|error| InspectionError::new("fixture_adapter_unavailable", error.to_string()))?;
    let operation = async {
        let mut stdin = child.stdin.take().ok_or_else(|| InspectionError::new("fixture_helper", "Helper stdin unavailable"))?;
        stdin.write_all(&input).await.map_err(|error| InspectionError::new("fixture_helper", error.to_string()))?;
        drop(stdin);
        let output = child.wait_with_output().await.map_err(|error| InspectionError::new("fixture_helper", error.to_string()))?;
        if !output.status.success() {
            return Err(InspectionError::new("fixture_helper", String::from_utf8_lossy(&output.stderr).into_owned()));
        }
        serde_json::from_slice(&output.stdout).map_err(|error| InspectionError::new("fixture_helper", error.to_string()))
    };
    tokio::time::timeout(std::time::Duration::from_secs(60), operation).await
        .map_err(|_| InspectionError::new("fixture_outcome_unknown", "Helper interrupted; inspect durable constituent receipts before any repeat"))?
}

pub(crate) async fn validate_registration(
    identity: &OwnedResourceIdentity,
    run: &Run,
) -> Result<(), InspectionError> {
    let result = invoke(identity, &run.run_id, &run.root_id, "register").await?;
    if result["state"] == "ready" {
        Ok(())
    } else {
        Err(InspectionError::new("fixture_registration_unproven", result["evidence"].as_str().unwrap_or("Creation receipt is unproven")))
    }
}

pub(crate) async fn inspect_owned_resource(
    record: &OwnedResourceRecord,
    run: &Run,
) -> Result<CleanupEvaluation, InspectionError> {
    let result = invoke(&record.identity, &run.run_id, &run.root_id, "inspect").await?;
    let message = result["evidence"].as_str().unwrap_or("Fixture inspection is unproven");
    Ok(match result["state"].as_str() {
        Some("ready") => CleanupEvaluation::Ready,
        Some("absent") => CleanupEvaluation::Absent,
        Some("refused") => CleanupEvaluation::Kept(reason(CleanupReasonClass::Authority, "fixture_protected", message)),
        _ => CleanupEvaluation::Unknown(reason(CleanupReasonClass::Reconcilable, "fixture_outcome_unknown", message)),
    })
}

pub(crate) async fn apply_owned_resource(
    record: &OwnedResourceRecord,
    run: &Run,
) -> Result<CleanupEffectOutcome, InspectionError> {
    let result = invoke(&record.identity, &run.run_id, &run.root_id, "apply").await?;
    let message = result["evidence"].as_str().unwrap_or("Fixture effect outcome is unproven");
    let (state, explanation) = match result["state"].as_str() {
        Some("absent") => (CleanupEffectState::Absent, None),
        Some("applied") => (CleanupEffectState::Applied, None),
        Some("refused") => (CleanupEffectState::Refused, Some(reason(CleanupReasonClass::Authority, "fixture_protected", message))),
        _ => (CleanupEffectState::Unknown, Some(reason(CleanupReasonClass::Reconcilable, "fixture_outcome_unknown", message))),
    };
    // Include every constituent receipt, not just the aggregate state. The
    // driver's fsynced intent journal survives helper/process interruption.
    let effects = parse_receipts(&result, record.kind)?;
    Ok(CleanupEffectOutcome { state, evidence: Some(result.to_string()), reason: explanation, effects })
}

fn parse_receipts(result: &Value, resource_kind: OwnedResourceKind) -> Result<Vec<CleanupEffectReceipt>, InspectionError> {
    let values = result.get("effects").and_then(Value::as_array).map(Vec::as_slice)
        .unwrap_or_else(|| if result.get("effect_id").is_some() { std::slice::from_ref(result) } else { &[] });
    let mut receipts = Vec::with_capacity(values.len());
    for value in values {
        let kind = match value["kind"].as_str() {
            Some("stop_process") => OwnedResourceKind::FixtureSet,
            Some("remove_scratch") => OwnedResourceKind::Scratch,
            _ => return Err(InspectionError::new("fixture_receipt", "Unsupported constituent effect kind")),
        };
        if kind != resource_kind {
            continue;
        }
        let field = |name: &str| -> Result<String, InspectionError> {
            value[name].as_str().map(str::to_owned)
                .ok_or_else(|| InspectionError::new("fixture_receipt", format!("Missing receipt field {name}")))
        };
        let state = CleanupEffectState::deserialize(&value["state"])
            .map_err(|error| InspectionError::new("fixture_receipt", error.to_string()))?;
        receipts.push(CleanupEffectReceipt {
            effect_id: field("effect_id")?, kind, identity_digest: field("identity_digest")?, state,
            evidence: value["evidence"].as_str().map(str::to_owned), updated_at: field("updated_at")?,
        });
    }
    Ok(receipts)
}

/// Recover constituent intents/outcomes after interruption without dispatching.
pub(crate) async fn effect_receipts(record: &OwnedResourceRecord) -> Result<Vec<CleanupEffectReceipt>, InspectionError> {
    let result = invoke(&record.identity, &record.run_id, &record.root_id, "receipts").await?;
    if !result.get("effects").is_some_and(Value::is_array) {
        return Err(InspectionError::new("fixture_receipt", result["evidence"].as_str().unwrap_or("Constituent journal is unobservable")));
    }
    parse_receipts(&result, record.kind)
}
