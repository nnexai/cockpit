use super::*;
use std::{fs, os::unix::fs::{PermissionsExt, symlink}};
use cockpit_protocol::orchestration::{CheckoutIdentity, DirectoryIdentity, ExecutionAttachment};
use super::super::claims::{AttachmentReference, CheckoutClaim};

#[test]
fn staging_io_diagnostics_keep_phase_and_errno_without_input_details() {
    let error = credential_io_at("directory_permissions", io::Error::from_raw_os_error(9));
    assert_eq!(error.code, "job_credential_io");
    assert!(error.message.contains("phase=directory_permissions"));
    assert!(error.message.contains("errno=Some(9)"));
    let error = credential_io_at("file_write", io::Error::new(io::ErrorKind::PermissionDenied, "private-path/secret-envelope"));
    assert!(error.message.contains("PermissionDenied"));
    assert!(!error.message.contains("private-path"));
    assert!(!error.message.contains("secret-envelope"));
}

struct Fixture {
    base: PathBuf,
    state: OrchestrationState,
    root_auth: JobAuth,
    worker_auth: JobAuth,
    worker_path: PathBuf,
}

fn run(kind: RunKind, root_id: Option<&str>) -> Run {
    let run_id = Uuid::new_v4().to_string();
    let worker = kind == RunKind::Worker;
    Run {
        session_id: "diagnostic-herdr-session".into(),
        brief: String::new(),
        root_id: root_id.unwrap_or(&run_id).to_owned(),
        parent_run_id: root_id.map(str::to_owned),
        task_id: worker.then(|| Uuid::new_v4().to_string()),
        run_id,
        kind,
        label: "credential regression".into(),
        attempt: 1,
        task_revision_at_start: None,
        stage: if worker { RunStage::Working } else { RunStage::Active },
        close_reason: None,
        dispatch: None,
        target: None,
        setup: None,
        cleanup_policy: None,
        last_report: None,
        result: None,
        source_output: None,
        integration: cockpit_protocol::orchestration::Integration::Pending,
        annotations: Vec::new(),
        project: None,
        checkout: None,
        attachment: Some(ExecutionAttachment {
            attachment_id: Uuid::new_v4().to_string(),
            epoch: 1,
            state: AttachmentState::Connected,
            location: None,
            main_session_id: Some("conversation-one".into()),
            process: None,
            shell: None,
            machine_id: None,
            launch_attempt: 1,
            launch_mode: None,
            updated_at: super::super::now(),
        }),
        checkout_claim_id: None,
        resume: None,
        retirement: None,
        created_at: super::super::now(),
        updated_at: super::super::now(),
    }
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("cockpit-job-credentials-{}", Uuid::new_v4()));
        fs::create_dir(&base).unwrap();
        let root = run(RunKind::Supervisor, None);
        let mut worker = run(RunKind::Worker, Some(&root.run_id));
        let claim_id = Uuid::new_v4().to_string();
        worker.checkout_claim_id = Some(claim_id.clone());
        worker.checkout = Some(CheckoutIdentity {
            key: "physical-checkout".into(),
            machine_id: "machine".into(),
            path: "/test-checkout".into(),
            directory_identity: DirectoryIdentity { device: "1".into(), inode: "2".into() },
            git: None,
        });
        let attachment = worker.attachment.as_ref().unwrap();
        let holder = AttachmentReference {
            run_id: worker.run_id.clone(),
            attachment_id: attachment.attachment_id.clone(),
            attachment_epoch: attachment.epoch,
        };
        let claim = CheckoutClaim {
            claim_id,
            checkout: worker.checkout.clone().unwrap(),
            revision: 1,
            state: ClaimState::Held,
            holder: Some(holder),
            transition_operation_id: None,
        };
        let (root_record, root_path) = stage_credential(&base, &root, JobRole::Root, None).unwrap();
        let (worker_record, worker_path) = stage_credential(&base, &worker, JobRole::Worker, None).unwrap();
        let root_auth = read_job_auth(&root_path).unwrap();
        let worker_auth = read_job_auth(&worker_path).unwrap();
        let mut state = OrchestrationState::default();
        state.runs = vec![root, worker];
        state.credentials = vec![root_record, worker_record];
        state.checkout_claims = vec![claim];
        Self { base, state, root_auth, worker_auth, worker_path }
    }

    fn envelope(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(&self.worker_path).unwrap()).unwrap()
    }

    fn replace_envelope(&self, value: &serde_json::Value) {
        fs::write(&self.worker_path, serde_json::to_vec(value).unwrap()).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn denied(state: &OrchestrationState, auth: &JobAuth) {
    assert!(matches!(validate_job_auth(state, auth), Err(error) if error.code == "job_unauthorized"));
}

#[test]
fn debug_redacts_secret_and_digest_and_store_contains_only_digest() {
    let fixture = Fixture::new();
    let envelope = fixture.envelope();
    let secret = envelope["secret"].as_str().unwrap();
    let digest = &fixture.state.credentials[1].secret_digest;
    for debug in [
        format!("{:?}", fixture.worker_auth.secret),
        format!("{:?}", fixture.worker_auth),
        format!("{:?}", fixture.state),
    ] {
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(secret));
        assert!(!debug.contains(digest));
    }
    let state_json = serde_json::to_string(&fixture.state).unwrap();
    assert!(!state_json.contains(secret));
    assert!(state_json.contains(digest));
}

#[test]
fn staged_files_are_private_durable_and_do_not_activate_themselves() {
    let fixture = Fixture::new();
    assert_eq!(fs::metadata(fixture.base.join(CREDENTIAL_DIRECTORY)).unwrap().permissions().mode() & 0o7777, 0o700);
    assert_eq!(fs::metadata(&fixture.worker_path).unwrap().permissions().mode() & 0o7777, 0o600);
    let (record, path) = stage_credential(&fixture.base, &fixture.state.runs[1], JobRole::Worker, None).unwrap();
    assert_ne!(path, fixture.worker_path);
    assert_ne!(record.credential_id, fixture.worker_auth.credential_id);
    denied(&fixture.state, &read_job_auth(&path).unwrap());
    assert!(validate_job_auth(&fixture.state, &fixture.worker_auth).is_ok());
}

#[test]
fn rejects_bounds_nonprivate_permissions_and_hardlinks() {
    let fixture = Fixture::new();
    fs::set_permissions(&fixture.worker_path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "unsafe_path");
    fs::set_permissions(&fixture.worker_path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&fixture.worker_path, fixture.base.join("linked-secret")).unwrap();
    assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "unsafe_path");
    fs::remove_file(fixture.base.join("linked-secret")).unwrap();
    fs::write(&fixture.worker_path, vec![b'x'; MAX_CREDENTIAL_BYTES as usize + 1]).unwrap();
    assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "unsafe_path");
}

#[test]
fn rejects_symlink_file_and_directory_and_world_accessible_directory() {
    let fixture = Fixture::new();
    let saved = fixture.base.join("saved-file");
    fs::rename(&fixture.worker_path, &saved).unwrap();
    symlink(&saved, &fixture.worker_path).unwrap();
    assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "unsafe_path");
    fs::remove_file(&fixture.worker_path).unwrap();
    fs::rename(&saved, &fixture.worker_path).unwrap();
    let directory = fixture.base.join(CREDENTIAL_DIRECTORY);
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "unsafe_path");
    assert!(stage_credential(&fixture.base, &fixture.state.runs[1], JobRole::Worker, None).is_err());
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let moved = fixture.base.join("saved-directory");
    fs::rename(&directory, &moved).unwrap();
    symlink(&moved, &directory).unwrap();
    assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "unsafe_path");
    assert!(stage_credential(&fixture.base, &fixture.state.runs[1], JobRole::Worker, None).is_err());
}

#[test]
fn parser_rejects_unknown_missing_trailing_and_malformed_secret_without_echoing_it() {
    let fixture = Fixture::new();
    let original = fixture.envelope();
    let mut changed = original.clone();
    changed["unexpected"] = serde_json::json!("private-marker-do-not-echo");
    fixture.replace_envelope(&changed);
    let error = read_job_auth(&fixture.worker_path).unwrap_err();
    assert_eq!(error.code, "job_credential_invalid");
    assert!(!format!("{error:?}").contains("private-marker-do-not-echo"));
    changed = original.clone();
    changed.as_object_mut().unwrap().remove("subject_child_id");
    fixture.replace_envelope(&changed);
    assert!(read_job_auth(&fixture.worker_path).is_err());
    changed = original.clone();
    changed["secret"] = serde_json::json!(STANDARD.encode([1u8; 31]));
    fixture.replace_envelope(&changed);
    assert!(read_job_auth(&fixture.worker_path).is_err());
    changed["secret"] = serde_json::json!(STANDARD.encode([1u8; 33]));
    fixture.replace_envelope(&changed);
    assert!(read_job_auth(&fixture.worker_path).is_err());
    let mut bytes = serde_json::to_vec(&original).unwrap();
    bytes.extend_from_slice(b"{}");
    fs::write(&fixture.worker_path, bytes).unwrap();
    assert!(read_job_auth(&fixture.worker_path).is_err());
}

#[test]
fn wrong_secret_epoch_attachment_and_revocation_are_denied() {
    let fixture = Fixture::new();
    let mut auth = fixture.worker_auth.clone();
    auth.secret.0[0] ^= 1;
    denied(&fixture.state, &auth);
    auth = fixture.worker_auth.clone();
    auth.attachment_epoch += 1;
    denied(&fixture.state, &auth);
    auth = fixture.worker_auth.clone();
    auth.attachment_id = Uuid::new_v4().to_string();
    denied(&fixture.state, &auth);
    let mut state = fixture.state.clone();
    state.credentials[1].state = CredentialState::Revoked;
    denied(&state, &fixture.worker_auth);
    state = fixture.state.clone();
    state.runs[1].attachment.as_mut().unwrap().state = AttachmentState::Revoked;
    denied(&state, &fixture.worker_auth);
}

#[test]
fn wrong_role_root_and_closed_subject_are_denied_without_stranding_live_descendants() {
    let fixture = Fixture::new();
    let mut state = fixture.state.clone();
    state.credentials[1].role = JobRole::Root;
    denied(&state, &fixture.worker_auth);
    state = fixture.state.clone();
    state.credentials[1].role = JobRole::Child;
    denied(&state, &fixture.worker_auth);
    state = fixture.state.clone();
    state.credentials[1].root_id = Uuid::new_v4().to_string();
    denied(&state, &fixture.worker_auth);
    state = fixture.state.clone();
    state.runs[1].stage = RunStage::Closed;
    denied(&state, &fixture.worker_auth);
    state = fixture.state.clone();
    state.runs[0].stage = RunStage::Closed;
    denied(&state, &fixture.root_auth);
    assert!(validate_job_auth(&state, &fixture.worker_auth).is_ok());
}

#[test]
fn worker_and_child_require_exact_nonconflicting_held_or_reserved_claim() {
    let fixture = Fixture::new();
    let mut state = fixture.state.clone();
    state.checkout_claims[0].state = ClaimState::Reserved;
    assert!(validate_job_auth(&state, &fixture.worker_auth).is_ok());
    for claim_state in [ClaimState::Conflict, ClaimState::Released, ClaimState::Quiescing] {
        state.checkout_claims[0].state = claim_state;
        denied(&state, &fixture.worker_auth);
    }
    state = fixture.state.clone();
    state.checkout_claims[0].holder.as_mut().unwrap().attachment_epoch += 1;
    denied(&state, &fixture.worker_auth);
    state = fixture.state.clone();
    let mut competing = state.checkout_claims[0].clone();
    competing.claim_id = Uuid::new_v4().to_string();
    competing.checkout.key = "subdirectory-distinct-key".into();
    competing.checkout.path.push_str("/subdirectory");
    state.checkout_claims.push(competing);
    denied(&state, &fixture.worker_auth);
    let (child_record, child_path) = stage_credential(&fixture.base, &fixture.state.runs[1], JobRole::Child, Some("actual-child")).unwrap();
    let child_auth = read_job_auth(&child_path).unwrap();
    let mut child_state = fixture.state.clone();
    child_state.credentials.push(child_record);
    let authorized = validate_job_auth(&child_state, &child_auth).unwrap();
    assert_eq!(authorized.role, JobRole::Child);
    assert_eq!(authorized.subject_child_id.as_deref(), Some("actual-child"));
    child_state.checkout_claims[0].state = ClaimState::Conflict;
    denied(&child_state, &child_auth);
}

#[test]
fn root_does_not_need_a_claim_but_cannot_be_a_writer() {
    let fixture = Fixture::new();
    let authorized = validate_job_auth(&fixture.state, &fixture.root_auth).unwrap();
    assert_eq!(authorized.role, JobRole::Root);
    assert!(authorized.checkout_claim_id.is_none());
    assert!(validate_retirement_job_auth(&fixture.state, &fixture.root_auth).is_err());
    let mut state = fixture.state.clone();
    state.runs[0].checkout_claim_id = state.runs[1].checkout_claim_id.clone();
    denied(&state, &fixture.root_auth);
}

#[test]
fn restart_and_conversation_rollover_preserve_auth_but_replacement_revokes_old_epoch() {
    let fixture = Fixture::new();
    let mut reopened: OrchestrationState = serde_json::from_slice(&serde_json::to_vec(&fixture.state).unwrap()).unwrap();
    let path = credential_path(&fixture.base, &reopened.credentials[1]).unwrap();
    assert_eq!(path, fixture.worker_path);
    let auth = read_job_auth(&path).unwrap();
    reopened.runs[1].session_id = "new-herdr-diagnostic".into();
    reopened.runs[1].attachment.as_mut().unwrap().main_session_id = Some("conversation-two".into());
    reopened.runs[1].attachment.as_mut().unwrap().state = AttachmentState::Disconnected;
    assert!(validate_job_auth(&reopened, &auth).is_ok());
    reopened.runs[1].attachment.as_mut().unwrap().epoch += 1;
    reopened.runs[1].attachment.as_mut().unwrap().attachment_id = Uuid::new_v4().to_string();
    denied(&reopened, &auth);
    assert_eq!(read_job_auth(&path).unwrap().attachment_epoch, 1);
}

#[test]
fn rediscovery_rejects_digest_role_run_and_envelope_tampering() {
    let fixture = Fixture::new();
    let record = &fixture.state.credentials[1];
    let mut changed = record.clone();
    changed.secret_digest = "00".repeat(32);
    assert!(credential_path(&fixture.base, &changed).is_err());
    changed = record.clone();
    changed.role = JobRole::Child;
    changed.subject_child_id = Some("other-child".into());
    assert!(credential_path(&fixture.base, &changed).is_err());
    changed = record.clone();
    changed.run_id = Uuid::new_v4().to_string();
    assert!(credential_path(&fixture.base, &changed).is_err());
    let mut envelope = fixture.envelope();
    envelope["secret"] = serde_json::json!(STANDARD.encode([42u8; 32]));
    fixture.replace_envelope(&envelope);
    assert!(credential_path(&fixture.base, record).is_err());
}

#[test]
fn cleanup_admission_returns_no_writer_claim_and_never_admits_children_or_stale_credentials() {
    let fixture = Fixture::new();
    assert!(validate_cleanup_job_auth(&fixture.state, &fixture.root_auth).is_ok());
    let mut state = fixture.state.clone();
    state.runs[1].stage = RunStage::Closed;
    state.runs[1].attachment.as_mut().unwrap().state = AttachmentState::Quiescing;
    state.checkout_claims.clear();
    state.runs[1].checkout_claim_id = None;
    let cleanup = validate_cleanup_job_auth(&state, &fixture.worker_auth).unwrap();
    assert_eq!(cleanup.role, JobRole::Worker);
    assert!(cleanup.checkout_claim_id.is_none());
    denied(&state, &fixture.worker_auth);
    state.credentials[1].state = CredentialState::Revoked;
    assert!(validate_cleanup_job_auth(&state, &fixture.worker_auth).is_err());
    state = fixture.state.clone();
    let child = test_job_auth(&mut state, 1, JobRole::Child, Some("cleanup-child"));
    assert!(validate_cleanup_job_auth(&state, &child).is_err());
    let mut stale = fixture.worker_auth.clone();
    stale.attachment_epoch += 1;
    assert!(validate_cleanup_job_auth(&state, &stale).is_err());
    state.runs[0].stage = RunStage::Closed;
    assert!(validate_cleanup_job_auth(&state, &fixture.root_auth).is_err());
}

#[test]
fn explicit_retirement_admission_is_not_ordinary_claim_or_task_authority() {
    let fixture = Fixture::new();
    let mut state = fixture.state.clone();
    state.runs[0].stage = RunStage::Closed;
    state.runs[1].stage = RunStage::Closed;
    state.runs[1].attachment.as_mut().unwrap().state = AttachmentState::Quiescing;
    state.checkout_claims[0].state = ClaimState::Conflict;
    state.checkout_claims[0].holder = None;
    denied(&state, &fixture.worker_auth);
    assert!(validate_retirement_job_auth(&state, &fixture.worker_auth).is_ok());
    state.checkout_claims.clear();
    state.runs[1].checkout_claim_id = None;
    assert!(validate_retirement_job_auth(&state, &fixture.worker_auth).is_ok());
    state.credentials[1].state = CredentialState::Revoked;
    assert!(validate_retirement_job_auth(&state, &fixture.worker_auth).is_err());
}

#[test]
fn retirement_still_requires_worker_digest_current_epoch_and_structural_root() {
    let fixture = Fixture::new();
    assert!(validate_retirement_job_auth(&fixture.state, &fixture.root_auth).is_err());
    let mut state = fixture.state.clone();
    let child = test_job_auth(&mut state, 1, JobRole::Child, Some("native-child"));
    assert!(validate_job_auth(&state, &child).is_ok());
    assert!(validate_retirement_job_auth(&state, &child).is_err());
    let mut wrong = fixture.worker_auth.clone();
    wrong.secret.0[1] ^= 1;
    assert!(validate_retirement_job_auth(&fixture.state, &wrong).is_err());
    wrong = fixture.worker_auth.clone();
    wrong.attachment_epoch += 1;
    assert!(validate_retirement_job_auth(&fixture.state, &wrong).is_err());
    state = fixture.state.clone();
    state.runs[1].attachment.as_mut().unwrap().state = AttachmentState::Revoked;
    assert!(validate_retirement_job_auth(&state, &fixture.worker_auth).is_err());
    state = fixture.state.clone();
    state.runs[0].kind = RunKind::Worker;
    assert!(validate_retirement_job_auth(&state, &fixture.worker_auth).is_err());
}

#[test]
fn independently_constructed_fixture_actors_share_exact_credential_identity() {
    let fixture = Fixture::new();
    let run = &fixture.state.runs[1];
    let (first, auth) = test_job_credential(run, JobRole::Worker, None);
    let (second, alias) = test_job_credential(run, JobRole::Worker, None);
    assert_eq!(first.credential_id, second.credential_id);
    assert_eq!(auth.credential_id, alias.credential_id);
    assert!(matches_digest(&alias.secret, &first.secret_digest));
    let (child, _) = test_job_credential(run, JobRole::Child, Some("child-one"));
    let (other_child, _) = test_job_credential(run, JobRole::Child, Some("child-two"));
    assert_ne!(first.credential_id, child.credential_id);
    assert_ne!(child.credential_id, other_child.credential_id);
}

#[test]
fn stage_rejects_wrong_role_child_subject_and_revoked_attachment() {
    let fixture = Fixture::new();
    let worker = &fixture.state.runs[1];
    assert!(stage_credential(&fixture.base, worker, JobRole::Root, None).is_err());
    assert!(stage_credential(&fixture.base, worker, JobRole::Child, None).is_err());
    assert!(stage_credential(&fixture.base, worker, JobRole::Worker, Some("child")).is_err());
    let mut worker = worker.clone();
    worker.attachment.as_mut().unwrap().state = AttachmentState::Revoked;
    assert!(stage_credential(&fixture.base, &worker, JobRole::Worker, None).is_err());
}
