use std::path::{Component, Path};

use cockpit_protocol::orchestration::{AttachmentState, CheckoutIdentity, ExecutionAttachment, RunKind};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{credentials::CredentialState, store::OrchestrationState, MAX_SAFE_COUNTER};
use crate::InspectionError;

pub(crate) use cockpit_protocol::orchestration::AttachmentReference;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClaimState { Reserved, Held, Quiescing, Released, Conflict }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckoutClaim {
    pub claim_id: String,
    pub checkout: CheckoutIdentity,
    pub revision: u64,
    pub state: ClaimState,
    pub holder: Option<AttachmentReference>,
    pub transition_operation_id: Option<String>,
}
/// Trusted observer evidence, intentionally not serializable or accepted on wire.
/// The physical dispatcher constructs this only after covering every writer
/// consumer (including queued launch shells, children and async consumers).
#[derive(Debug, Clone)]
pub struct WriterExitProof {
    pub(crate) incarnation: AttachmentReference,
    pub(crate) claim_revision: u64,
    pub(crate) checkout: Option<CheckoutIdentity>,
    pub(crate) machine_id: String,
    pub(crate) previous_kernel_boot_id: String,
    pub(crate) observed_kernel_boot_id: String,
    pub(crate) all_consumers_absent: bool,
}

/// Recovery authority for the liaison main only, never consumer closure or
/// checkout transfer. Constructed from fresh physical root-container evidence.
#[derive(Debug, Clone)]
pub struct RootMainRecoveryProof {
    pub(crate) run_id: String,
    pub(crate) session_id: String,
    pub(crate) attachment: ExecutionAttachment,
    pub(crate) dispatch: Option<cockpit_protocol::orchestration::DispatchState>,
    pub(crate) retained_checkout: Option<cockpit_protocol::orchestration::CheckoutIdentity>,
    pub(crate) idle_shell: Option<cockpit_protocol::orchestration::NativeShellIdentity>,
}

fn refusal(code: &str, text: &str) -> InspectionError { InspectionError::new(code, text) }
fn absolute_normal(path: &str) -> bool {
    Path::new(path).is_absolute() && !Path::new(path).components().any(|c| matches!(c, Component::ParentDir | Component::CurDir))
}
pub(crate) fn paths_overlap(left: &str, right: &str) -> bool {
    Path::new(left).starts_with(right) || Path::new(right).starts_with(left)
}
pub(crate) fn checkouts_conflict(left: &CheckoutIdentity, right: &CheckoutIdentity) -> bool {
    left.key == right.key || (left.machine_id == right.machine_id && paths_overlap(&left.path, &right.path))
}
fn validate_checkout(checkout: &CheckoutIdentity) -> Result<(), InspectionError> {
    if checkout.key.is_empty() || checkout.machine_id.is_empty() || !absolute_normal(&checkout.path)
        || checkout.directory_identity.device.is_empty() || checkout.directory_identity.inode.is_empty()
        || checkout.git.as_ref().is_some_and(|git| !absolute_normal(&git.common_dir) || !absolute_normal(&git.worktree_git_dir)) {
        return Err(refusal("checkout_identity_unavailable", "verified physical checkout identity is required"));
    }
    Ok(())
}
fn validate_reference(state: &OrchestrationState, reference: &AttachmentReference) -> Result<usize, InspectionError> {
    let index = state.runs.iter().position(|run| run.run_id == reference.run_id)
        .ok_or_else(|| refusal("run_not_found", "claim holder run was not found"))?;
    let run = &state.runs[index];
    if run.kind != RunKind::Worker || run.attachment.as_ref().is_none_or(|a| a.attachment_id != reference.attachment_id || a.epoch != reference.attachment_epoch || a.state == AttachmentState::Revoked) {
        return Err(refusal("attachment_conflict", "claim holder is not the current worker attachment"));
    }
    Ok(index)
}

/// Caller holds the one state lock and supplies freshly revalidated checkout
/// evidence. This helper never saves or recursively acquires a lock.
pub(crate) fn reserve_claim(state: &mut OrchestrationState, checkout: &CheckoutIdentity, holder: &AttachmentReference, operation_id: &str) -> Result<String, InspectionError> {
    validate_checkout(checkout)?;
    super::tasks_md::validate_uuid(operation_id)?;
    let index = validate_reference(state, holder)?;
    let mut overlapping = state.checkout_claims.iter().filter(|claim| claim.state != ClaimState::Released && checkouts_conflict(&claim.checkout, checkout));
    let existing = overlapping.next();
    if overlapping.next().is_some() {
        return Err(refusal("checkout_claim_conflict", "multiple protected claims overlap this checkout"));
    }
    if let Some(claim) = existing {
        if claim.checkout != *checkout {
            return Err(refusal("checkout_identity_changed", "checkout directory or worktree identity changed"));
        }
        if matches!(claim.state, ClaimState::Reserved | ClaimState::Held) && claim.holder.as_ref() == Some(holder) {
            return Ok(claim.claim_id.clone());
        }
        return Err(refusal("checkout_claim_conflict", "checkout has a protected managed writer"));
    }
    let claim_id = Uuid::new_v4().to_string();
    state.checkout_claims.push(CheckoutClaim { claim_id: claim_id.clone(), checkout: checkout.clone(), revision: 1, state: ClaimState::Reserved,
        holder: Some(holder.clone()), transition_operation_id: Some(operation_id.into()) });
    state.runs[index].checkout = Some(checkout.clone());
    state.runs[index].checkout_claim_id = Some(claim_id.clone());
    Ok(claim_id)
}

pub(crate) fn hold_claim(state: &mut OrchestrationState, claim_id: &str, holder: &AttachmentReference, expected_revision: u64) -> Result<u64, InspectionError> {
    validate_reference(state, holder)?;
    let claim = state.checkout_claims.iter_mut().find(|c| c.claim_id == claim_id).ok_or_else(|| refusal("checkout_claim_missing", "checkout claim was not found"))?;
    if claim.revision != expected_revision || claim.holder.as_ref() != Some(holder) || !matches!(claim.state, ClaimState::Reserved | ClaimState::Held) {
        return Err(refusal("checkout_claim_conflict", "checkout claim changed before connection"));
    }
    if claim.state == ClaimState::Held { return Ok(claim.revision); }
    let next = claim.revision.checked_add(1).filter(|n| *n <= MAX_SAFE_COUNTER).ok_or_else(|| refusal("orchestration_state_full", "claim revision exhausted"))?;
    claim.state = ClaimState::Held;
    claim.revision = next;
    Ok(next)
}

/// Atomic epoch cutover in memory; all refusal paths precede any mutation.
pub(crate) fn revoke_and_reserve_replacement(state: &mut OrchestrationState, claim_id: &str, proof: &WriterExitProof, replacement: ExecutionAttachment, operation_id: &str) -> Result<u64, InspectionError> {
    super::tasks_md::validate_uuid(operation_id)?;
    let run_index = validate_reference(state, &proof.incarnation)?;
    let claim_index = state.checkout_claims.iter().position(|c| c.claim_id == claim_id).ok_or_else(|| refusal("checkout_claim_missing", "checkout claim was not found"))?;
    let claim = &state.checkout_claims[claim_index];
    let old = state.runs[run_index].attachment.as_ref().expect("validated attachment");
    let old_boot = old.process.as_ref().and_then(|p| p.kernel_boot_id.as_ref()).or_else(|| old.shell.as_ref().and_then(|s| s.process.kernel_boot_id.as_ref()));
    let epoch = old.epoch.checked_add(1).filter(|n| *n <= MAX_SAFE_COUNTER).ok_or_else(|| refusal("orchestration_state_full", "attachment epoch exhausted"))?;
    let next = claim.revision.checked_add(1).filter(|n| *n <= MAX_SAFE_COUNTER).ok_or_else(|| refusal("orchestration_state_full", "claim revision exhausted"))?;
    if claim.revision != proof.claim_revision || proof.checkout.as_ref() != Some(&claim.checkout) || claim.holder.as_ref() != Some(&proof.incarnation)
        || claim.state == ClaimState::Released || claim.state == ClaimState::Conflict
        || old.machine_id.as_ref() != Some(&proof.machine_id) || proof.machine_id != claim.checkout.machine_id
        || old_boot != Some(&proof.previous_kernel_boot_id) || proof.previous_kernel_boot_id.is_empty() || proof.observed_kernel_boot_id.is_empty()
        || !proof.all_consumers_absent || proof.previous_kernel_boot_id == proof.observed_kernel_boot_id
        || replacement.epoch != epoch || replacement.attachment_id == old.attachment_id
        || replacement.state != AttachmentState::Reserved || replacement.process.is_some() || replacement.shell.is_some()
        || replacement.location.is_some() || replacement.conversation != old.conversation {
        return Err(refusal("writer_exit_unproven", "replacement requires current same-machine proof covering every protected writer"));
    }
    super::tasks_md::validate_uuid(&replacement.attachment_id)?;
    let holder = AttachmentReference { run_id: proof.incarnation.run_id.clone(), attachment_id: replacement.attachment_id.clone(), attachment_epoch: replacement.epoch };
    for credential in &mut state.credentials {
        if credential.run_id == holder.run_id && credential.attachment_id == proof.incarnation.attachment_id && credential.attachment_epoch == proof.incarnation.attachment_epoch {
            credential.state = CredentialState::Revoked;
        }
    }
    state.runs[run_index].attachment = Some(replacement);
    let claim = &mut state.checkout_claims[claim_index];
    claim.state = ClaimState::Reserved;
    claim.revision = next;
    claim.holder = Some(holder);
    claim.transition_operation_id = Some(operation_id.into());
    Ok(next)
}

pub(crate) fn revoke_root_and_reserve_replacement(state: &mut OrchestrationState, proof: &RootMainRecoveryProof, replacement: ExecutionAttachment, operation_id: &str) -> Result<(), InspectionError> {
    super::tasks_md::validate_uuid(operation_id)?;
    super::tasks_md::validate_uuid(&replacement.attachment_id)?;
    let index = state.runs.iter().position(|r| r.run_id == proof.run_id)
        .ok_or_else(|| refusal("run_not_found", "root run was not found"))?;
    let run = &state.runs[index];
    super::project_identity::revalidate_root_recovery(state, run, proof)?;
    let old = run.attachment.as_ref().ok_or_else(|| refusal("attachment_conflict", "root attachment is missing"))?;
    let epoch = old.epoch.checked_add(1).filter(|e| *e <= MAX_SAFE_COUNTER)
        .ok_or_else(|| refusal("orchestration_state_full", "attachment epoch exhausted"))?;
    let attempt = old.launch_attempt.checked_add(1)
        .ok_or_else(|| refusal("orchestration_state_full", "launch attempt exhausted"))?;
    if replacement.epoch != epoch || replacement.attachment_id == old.attachment_id
        || replacement.state != AttachmentState::Reserved || replacement.process.is_some() || replacement.shell.is_some()
        || replacement.location.is_some() || replacement.conversation != old.conversation
        || replacement.machine_id.is_some() || replacement.launch_mode.is_some() || replacement.launch_attempt != attempt {
        return Err(refusal("root_main_exit_unproven", "Root replacement must reserve exactly the next main attachment"));
    }
    for credential in &mut state.credentials {
        if credential.run_id == run.run_id && credential.attachment_id == old.attachment_id && credential.attachment_epoch == old.epoch {
            credential.state = CredentialState::Revoked;
        }
    }
    state.runs[index].attachment = Some(replacement);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::orchestration::DirectoryIdentity;
    fn checkout(path: &str, key: &str) -> CheckoutIdentity {
        CheckoutIdentity { key: key.into(), machine_id: "machine".into(), path: path.into(), directory_identity: DirectoryIdentity { device: "1".into(), inode: "2".into() }, git: None }
    }
    fn worker(state: &mut OrchestrationState, id: &str) -> AttachmentReference {
        let run = super::super::new_run(id, "session", RunKind::Worker, id.into(), "root", Some("root".into()), Some("task".into()), 1);
        let reference = AttachmentReference { run_id: id.into(), attachment_id: run.attachment.as_ref().unwrap().attachment_id.clone(), attachment_epoch: 1 };
        state.runs.push(run);
        reference
    }
    #[test]
    fn writers_are_exclusive_across_roots_and_identity_changes_fail_without_mutation() {
        let mut state = OrchestrationState::default();
        let a = worker(&mut state, "a");
        let b = worker(&mut state, "b");
        state.runs[1].root_id = "another-root".into();
        let checkout = checkout("/repo", "physical-checkout");
        let operation = Uuid::new_v4().to_string();
        let claim = reserve_claim(&mut state, &checkout, &a, &operation).unwrap();
        let before = serde_json::to_vec(&state).unwrap();
        assert_eq!(reserve_claim(&mut state, &checkout, &b, &Uuid::new_v4().to_string()).unwrap_err().code, "checkout_claim_conflict");
        assert_eq!(serde_json::to_vec(&state).unwrap(), before);
        assert_eq!(reserve_claim(&mut state, &checkout, &a, &operation).unwrap(), claim);
        let mut changed = checkout.clone();
        changed.directory_identity.inode = "replacement-directory".into();
        assert_eq!(reserve_claim(&mut state, &changed, &a, &operation).unwrap_err().code, "checkout_identity_changed");
        assert_eq!(serde_json::to_vec(&state).unwrap(), before);
    }
    #[test]
    fn replacement_requires_full_exit_proof_and_atomically_reserves_next_epoch() {
        let mut state = OrchestrationState::default();
        let old = worker(&mut state, "worker");
        let checkout = checkout("/repo", "checkout");
        let op = Uuid::new_v4().to_string();
        let claim = reserve_claim(&mut state, &checkout, &old, &op).unwrap();
        let attachment = state.runs[0].attachment.as_mut().unwrap();
        attachment.machine_id = Some("machine".into());
        attachment.process = Some(cockpit_protocol::orchestration::NativeProcessIdentity { pid: 99, start_ticks: 7, kernel_boot_id: Some("old-boot".into()) });
        let replacement = super::super::reserved_attachment(2, 1);
        let mut proof = WriterExitProof { incarnation: old.clone(), claim_revision: 1, checkout: Some(checkout),
            machine_id: "machine".into(), previous_kernel_boot_id: "old-boot".into(), observed_kernel_boot_id: "new-boot".into(), all_consumers_absent: false };
        let before = serde_json::to_vec(&state).unwrap();
        assert_eq!(revoke_and_reserve_replacement(&mut state, &claim, &proof, replacement.clone(), &op).unwrap_err().code, "writer_exit_unproven");
        assert_eq!(serde_json::to_vec(&state).unwrap(), before);
        proof.all_consumers_absent = true;
        proof.observed_kernel_boot_id = "old-boot".into();
        assert_eq!(revoke_and_reserve_replacement(&mut state, &claim, &proof, replacement.clone(), &op).unwrap_err().code, "writer_exit_unproven");
        assert_eq!(serde_json::to_vec(&state).unwrap(), before);
        proof.observed_kernel_boot_id = "new-boot".into();
        proof.machine_id = "different-machine".into();
        assert!(revoke_and_reserve_replacement(&mut state, &claim, &proof, replacement.clone(), &op).is_err());
        assert_eq!(serde_json::to_vec(&state).unwrap(), before);
        proof.machine_id = "machine".into();
        assert_eq!(revoke_and_reserve_replacement(&mut state, &claim, &proof, replacement, &op).unwrap(), 2);
        assert_eq!(state.runs[0].attachment.as_ref().unwrap().epoch, 2);
        assert_eq!(state.checkout_claims[0].state, ClaimState::Reserved);
        assert_eq!(state.checkout_claims[0].holder.as_ref().unwrap().attachment_epoch, 2);
        assert!(revoke_and_reserve_replacement(&mut state, &claim, &proof, super::super::reserved_attachment(2, 1), &op).is_err());
    }
    #[test]
    fn ancestor_writer_cannot_be_bypassed_by_subdirectory() {
        assert!(checkouts_conflict(&checkout("/repo", "a"), &checkout("/repo/sub", "b")));
        assert!(!checkouts_conflict(&checkout("/repo", "a"), &checkout("/repository", "b")));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn root_main_cutover_revalidates_incarnation_dispatch_and_claim_fences_atomically() {
        use cockpit_protocol::orchestration::{NativeProcessIdentity, RunLocation, RunStage, DispatchStep};
        let mut original = OrchestrationState::default();
        let id = Uuid::new_v4().to_string();
        let mut root = super::super::new_run(&id, "session", RunKind::Supervisor, "Root".into(), &id, None, None, 1);
        root.stage = RunStage::Active;
        let attachment = root.attachment.as_mut().unwrap();
        attachment.state = AttachmentState::Disconnected;
        attachment.launch_attempt = 1;
        attachment.machine_id = crate::process_identity::machine_id();
        attachment.conversation = Some(cockpit_protocol::orchestration::ConversationIdentity {
            harness: "omp".into(), session_id: "root-conversation".into(),
        });
        attachment.process = Some(NativeProcessIdentity { pid: i32::MAX as u32, start_ticks: 1,
            kernel_boot_id: crate::process_identity::kernel_boot_id() });
        attachment.location = Some(RunLocation {
            endpoint_identity: "endpoint".into(), boot_id: None, session_id: "session".into(),
            workspace_id: "space".into(), tab_id: "tab".into(), pane_id: "pane".into(),
            terminal_id: Some("terminal".into()), native_session_id: Some("native".into()), launch_tag: "tag".into(),
        });
        let mut dispatch = super::super::dispatch(DispatchStep::Launched);
        dispatch.launch_tag = Some("tag".into());
        dispatch.endpoint_identity = Some("endpoint".into());
        root.dispatch = Some(dispatch);
        let proof = RootMainRecoveryProof { run_id: id.clone(), session_id: "session".into(),
            attachment: root.attachment.clone().unwrap(), dispatch: root.dispatch.clone(), retained_checkout: None, idle_shell: None };
        original.runs.push(root);
        let holder = worker(&mut original, "independent-worker");
        let claim_id = reserve_claim(&mut original, &checkout("/worker", "worker-checkout"), &holder, &Uuid::new_v4().to_string()).unwrap();
        for fence in 0..9 {
            let mut state = original.clone();
            let root = &mut state.runs[0];
            match fence {
                0 => {}
                1 => root.attachment.as_mut().unwrap().epoch += 1,
                2 => root.attachment.as_mut().unwrap().location.as_mut().unwrap().endpoint_identity = "newer-endpoint".into(),
                3 => root.dispatch.as_mut().unwrap().launch_attempt += 1,
                4 => root.dispatch.as_mut().unwrap().step = DispatchStep::LaunchUnknown,
                5 => root.attachment.as_mut().unwrap().process.as_mut().unwrap().start_ticks += 1,
                6 => root.checkout_claim_id = Some(claim_id.clone()),
                7 => root.attachment.as_mut().unwrap().state = AttachmentState::Quiescing,
                8 => {
                    state.checkout_claims[0].holder = Some(AttachmentReference { run_id: id.clone(),
                        attachment_id: proof.attachment.attachment_id.clone(), attachment_epoch: proof.attachment.epoch });
                }
                _ => unreachable!(),
            }
            let before = serde_json::to_vec(&state).unwrap();
            let mut replacement = super::super::reserved_attachment(2, 2);
            replacement.conversation = proof.attachment.conversation.clone();
            let result = revoke_root_and_reserve_replacement(&mut state, &proof,
                replacement, &Uuid::new_v4().to_string());
            if fence == 0 {
                result.unwrap();
                assert_eq!(state.runs[0].attachment.as_ref().unwrap().epoch, 2);
                assert_eq!(state.runs[0].attachment.as_ref().unwrap().conversation, proof.attachment.conversation);
                assert_eq!(serde_json::to_value(&state.runs[1]).unwrap(), serde_json::to_value(&original.runs[1]).unwrap());
                assert_eq!(serde_json::to_value(&state.checkout_claims).unwrap(), serde_json::to_value(&original.checkout_claims).unwrap());
            } else {
                assert!(result.is_err(), "fence {fence}");
                assert_eq!(serde_json::to_vec(&state).unwrap(), before, "refusal {fence} must not mutate any authority");
            }
        }
    }
}
