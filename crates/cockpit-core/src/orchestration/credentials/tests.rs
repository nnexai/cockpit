use super::*;
use std::{fs, os::unix::fs::{PermissionsExt, symlink}};
use cockpit_protocol::orchestration::{
    CheckoutIdentity, ConversationIdentity, DirectoryIdentity, ExecutionAttachment,
    TaskOwner, TaskOwnership,
};
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
        assignment_generation: worker.then_some(1),
        launch_config: Default::default(),
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
            conversation: Some(ConversationIdentity {
                harness: "omp".into(),
                session_id: "conversation-one".into(),
            }),
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
        let (mut root_record, root_path) = stage_credential(&base, &root, JobRole::Root).unwrap();
        let (mut worker_record, worker_path) = stage_credential(&base, &worker, JobRole::Worker).unwrap();
        root_record.conversation = root.attachment.as_ref().unwrap().conversation.clone();
        worker_record.conversation = worker.attachment.as_ref().unwrap().conversation.clone();
        let root_auth = read_job_auth(&root_path).unwrap();
        let worker_auth = read_job_auth(&worker_path).unwrap();
        let mut state = OrchestrationState::default();
        state.task_ownership = vec![TaskOwnership {
            root_id: worker.root_id.clone(),
            task_id: worker.task_id.clone().unwrap(),
            generation: worker.assignment_generation.unwrap(),
            owner: Some(TaskOwner {
                run_id: worker.run_id.clone(),
                conversation: worker.attachment.as_ref().unwrap().conversation.clone().unwrap(),
                claim_operation_id: worker_record.credential_id.clone(),
            }),
        }];
        state.runs = vec![root, worker];
        state.credentials = vec![root_record, worker_record];
        state.checkout_claims = vec![claim];
        Self { base, state, root_auth, worker_auth, worker_path }
    }

    fn caller(&self, index: usize) -> super::super::AgentCaller {
        let run = &self.state.runs[index];
        super::super::AgentCaller {
            auth: if index == 0 { self.root_auth.clone() } else { self.worker_auth.clone() },
            conversation: run.attachment.as_ref().unwrap().conversation.clone().unwrap(),
            assignment_generation: run.assignment_generation,
            endpoint_identity: String::new(),
            session_id: String::new(),
            workspace_id: String::new(),
            tab_id: String::new(),
            pane_id: String::new(),
            boot_id: None,
            terminal_id: None,
            native_session_id: None,
            actual_agent_kind: None,
            process: None,
        }
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
    let original_bytes = fs::read(&fixture.worker_path).unwrap();
    assert_eq!(fs::metadata(fixture.base.join(CREDENTIAL_DIRECTORY)).unwrap().permissions().mode() & 0o7777, 0o700);
    assert_eq!(fs::metadata(&fixture.worker_path).unwrap().permissions().mode() & 0o7777, 0o600);
    let (record, path) = stage_credential(&fixture.base, &fixture.state.runs[1], JobRole::Worker).unwrap();
    assert_ne!(path, fixture.worker_path);
    assert_ne!(record.credential_id, fixture.worker_auth.credential_id);
    assert!(record.conversation.is_none());
    assert_eq!(fs::read(&fixture.worker_path).unwrap(), original_bytes);
    assert_eq!(credential_path(&fixture.base, &fixture.state.credentials[1]).unwrap(), fixture.worker_path);
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
    assert!(stage_credential(&fixture.base, &fixture.state.runs[1], JobRole::Worker).is_err());
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let moved = fixture.base.join("saved-directory");
    fs::rename(&directory, &moved).unwrap();
    symlink(&moved, &directory).unwrap();
    assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "unsafe_path");
    assert!(stage_credential(&fixture.base, &fixture.state.runs[1], JobRole::Worker).is_err());
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
    changed.as_object_mut().unwrap().remove("credential_id");
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
fn worker_requires_exact_nonconflicting_held_or_reserved_claim() {
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
}

#[test]
fn root_does_not_need_a_claim_but_cannot_be_a_writer() {
    let fixture = Fixture::new();
    let authorized = validate_job_auth(&fixture.state, &fixture.root_auth).unwrap();
    assert_eq!(authorized.role, JobRole::Root);
    assert!(authorized.checkout_claim_id.is_none());
    let mut state = fixture.state.clone();
    state.runs[0].checkout_claim_id = state.runs[1].checkout_claim_id.clone();
    denied(&state, &fixture.root_auth);
}

#[test]
fn restart_and_physical_relocation_preserve_capability_but_replacement_revokes_old_epoch() {
    let fixture = Fixture::new();
    let mut reopened: OrchestrationState = serde_json::from_slice(&serde_json::to_vec(&fixture.state).unwrap()).unwrap();
    let path = credential_path(&fixture.base, &reopened.credentials[1]).unwrap();
    assert_eq!(path, fixture.worker_path);
    let auth = read_job_auth(&path).unwrap();
    reopened.runs[1].session_id = "new-herdr-diagnostic".into();
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
    changed.role = JobRole::Root;
    assert!(credential_path(&fixture.base, &changed).is_err());
    changed = record.clone();
    changed.run_id = Uuid::new_v4().to_string();
    assert!(credential_path(&fixture.base, &changed).is_err());
    changed = record.clone();
    changed.root_id = Uuid::new_v4().to_string();
    assert!(credential_path(&fixture.base, &changed).is_err());
    changed = record.clone();
    changed.assignment_generation = Some(2);
    assert!(credential_path(&fixture.base, &changed).is_err());
    changed = record.clone();
    changed.state = CredentialState::Revoked;
    assert!(credential_path(&fixture.base, &changed).is_err());
    let mut envelope = fixture.envelope();
    envelope["secret"] = serde_json::json!(STANDARD.encode([42u8; 32]));
    fixture.replace_envelope(&envelope);
    assert!(credential_path(&fixture.base, record).is_err());
}

#[test]
fn cleanup_admission_returns_no_writer_claim_and_rejects_stale_credentials() {
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
    let mut stale = fixture.worker_auth.clone();
    stale.attachment_epoch += 1;
    assert!(validate_cleanup_job_auth(&state, &stale).is_err());
    state.runs[0].stage = RunStage::Closed;
    assert!(validate_cleanup_job_auth(&state, &fixture.root_auth).is_err());
}


#[test]
fn independently_constructed_fixture_actors_share_exact_credential_identity() {
    let fixture = Fixture::new();
    let run = &fixture.state.runs[1];
    let (first, auth) = test_job_credential(run, JobRole::Worker);
    let (second, alias) = test_job_credential(run, JobRole::Worker);
    assert_eq!(first.credential_id, second.credential_id);
    assert_eq!(auth.credential_id, alias.credential_id);
    assert!(matches_digest(&alias.secret, &first.secret_digest));
    assert_eq!(auth.assignment_generation, Some(1));
    assert_eq!(alias.assignment_generation, first.assignment_generation);
    assert_eq!(first.conversation, run.attachment.as_ref().unwrap().conversation);
    assert_eq!(second.conversation, first.conversation);
}

#[test]
fn stage_rejects_wrong_role_generation_shape_and_revoked_attachment() {
    let fixture = Fixture::new();
    let worker = &fixture.state.runs[1];
    assert!(stage_credential(&fixture.base, worker, JobRole::Root).is_err());
    assert!(stage_credential(&fixture.base, &fixture.state.runs[0], JobRole::Worker).is_err());
    for generation in [None, Some(0), Some(super::super::MAX_SAFE_COUNTER + 1)] {
        let mut worker = worker.clone();
        worker.assignment_generation = generation;
        assert!(stage_credential(&fixture.base, &worker, JobRole::Worker).is_err());
    }
    let mut root = fixture.state.runs[0].clone();
    root.assignment_generation = Some(1);
    assert!(stage_credential(&fixture.base, &root, JobRole::Root).is_err());
    let mut worker = worker.clone();
    worker.attachment.as_mut().unwrap().state = AttachmentState::Revoked;
    assert!(stage_credential(&fixture.base, &worker, JobRole::Worker).is_err());
}

#[test]
fn schema_two_pins_worker_generation_and_omits_child_subject() {
    let fixture = Fixture::new();
    let worker = fixture.envelope();
    assert_eq!(worker["schema"], 2);
    assert_eq!(worker["role"], "worker");
    assert_eq!(worker["assignment_generation"], 1);
    assert!(!worker.as_object().unwrap().contains_key("subject_child_id"));
    assert_eq!(fixture.worker_auth.assignment_generation, Some(1));
    assert_eq!(fixture.state.credentials[1].assignment_generation, Some(1));
    let root_path = credential_path(&fixture.base, &fixture.state.credentials[0]).unwrap();
    let root: serde_json::Value = serde_json::from_slice(&fs::read(root_path).unwrap()).unwrap();
    assert_eq!(root["schema"], 2);
    assert_eq!(root["role"], "root");
    assert!(root.as_object().unwrap().contains_key("assignment_generation"));
    assert!(root["assignment_generation"].is_null());
    assert!(!root.as_object().unwrap().contains_key("subject_child_id"));
    assert_eq!(fixture.root_auth.assignment_generation, None);
}

#[test]
fn parser_rejects_old_schema_child_role_and_invalid_generation_shapes() {
    let fixture = Fixture::new();
    let original = fixture.envelope();
    for schema in [0, 1, 3] {
        let mut changed = original.clone();
        changed["schema"] = serde_json::json!(schema);
        fixture.replace_envelope(&changed);
        assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "job_credential_invalid");
    }
    let mut changed = original.clone();
    changed["subject_child_id"] = serde_json::Value::Null;
    fixture.replace_envelope(&changed);
    assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "job_credential_invalid");
    changed = original.clone();
    changed["role"] = serde_json::json!("child");
    fixture.replace_envelope(&changed);
    assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "job_credential_invalid");
    for generation in [
        serde_json::Value::Null,
        serde_json::json!(0),
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::json!("1"),
        serde_json::json!(super::super::MAX_SAFE_COUNTER + 1),
    ] {
        changed = original.clone();
        changed["assignment_generation"] = generation;
        fixture.replace_envelope(&changed);
        assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "job_credential_invalid");
    }
    changed = original.clone();
    changed.as_object_mut().unwrap().remove("assignment_generation");
    fixture.replace_envelope(&changed);
    assert_eq!(read_job_auth(&fixture.worker_path).unwrap_err().code, "job_credential_invalid");
    let root_path = credential_path(&fixture.base, &fixture.state.credentials[0]).unwrap();
    let mut root: serde_json::Value = serde_json::from_slice(&fs::read(&root_path).unwrap()).unwrap();
    root["assignment_generation"] = serde_json::json!(1);
    fs::write(&root_path, serde_json::to_vec(&root).unwrap()).unwrap();
    assert_eq!(read_job_auth(&root_path).unwrap_err().code, "job_credential_invalid");
}

#[test]
fn credential_generation_must_match_auth_record_and_run() {
    let fixture = Fixture::new();
    for generation in [None, Some(0), Some(2)] {
        let mut auth = fixture.worker_auth.clone();
        auth.assignment_generation = generation;
        denied(&fixture.state, &auth);
        let mut state = fixture.state.clone();
        state.credentials[1].assignment_generation = generation;
        denied(&state, &fixture.worker_auth);
        state = fixture.state.clone();
        state.runs[1].assignment_generation = generation;
        denied(&state, &fixture.worker_auth);
    }
    let mut auth = fixture.root_auth.clone();
    auth.assignment_generation = Some(1);
    denied(&fixture.state, &auth);
    let mut state = fixture.state.clone();
    state.credentials[0].assignment_generation = Some(1);
    denied(&state, &fixture.root_auth);
    state.runs[0].assignment_generation = Some(1);
    denied(&state, &auth);
}

#[test]
fn caller_authority_requires_current_owner_conversation_and_generation_not_process_evidence() {
    let fixture = Fixture::new();
    let caller = fixture.caller(1);
    assert!(validate_caller(&fixture.state, &caller).is_ok());
    assert!(validate_caller(&fixture.state, &fixture.caller(0)).is_ok());
    let mut wrong = caller.clone();
    wrong.conversation.session_id = "different-opaque-session".into();
    assert!(validate_caller(&fixture.state, &wrong).is_err());
    wrong = caller.clone();
    wrong.conversation.harness = "different-harness".into();
    assert!(validate_caller(&fixture.state, &wrong).is_err());
    for generation in [None, Some(0), Some(2)] {
        wrong = caller.clone();
        wrong.assignment_generation = generation;
        assert!(validate_caller(&fixture.state, &wrong).is_err());
    }
    let mut state = fixture.state.clone();
    state.task_ownership[0].generation += 1;
    assert!(validate_caller(&state, &caller).is_err());
    state = fixture.state.clone();
    state.task_ownership[0].owner = None;
    assert!(validate_caller(&state, &caller).is_err());
    state = fixture.state.clone();
    state.task_ownership[0].owner.as_mut().unwrap().run_id = Uuid::new_v4().to_string();
    assert!(validate_caller(&state, &caller).is_err());
    state = fixture.state.clone();
    state.task_ownership[0].owner.as_mut().unwrap().conversation.session_id = "reassigned".into();
    assert!(validate_caller(&state, &caller).is_err());
    state = fixture.state.clone();
    state.task_ownership.clear();
    assert!(validate_caller(&state, &caller).is_err());
    state = fixture.state.clone();
    state.runs[1].attachment.as_mut().unwrap().conversation = None;
    assert!(validate_caller(&state, &caller).is_err());
    state = fixture.state.clone();
    state.credentials[1].conversation = None;
    assert!(validate_caller(&state, &caller).is_err());
    state.credentials[1].conversation = Some(ConversationIdentity {
        harness: caller.conversation.harness.clone(),
        session_id: "other-credential-conversation".into(),
    });
    assert!(validate_caller(&state, &caller).is_err());
}

#[test]
fn missing_duplicate_and_retargeted_run_or_credential_records_are_denied() {
    let fixture = Fixture::new();
    let mut state = fixture.state.clone();
    state.credentials.clear();
    denied(&state, &fixture.worker_auth);
    state = fixture.state.clone();
    state.credentials.push(state.credentials[1].clone());
    denied(&state, &fixture.worker_auth);
    state = fixture.state.clone();
    state.runs.remove(1);
    denied(&state, &fixture.worker_auth);
    state = fixture.state.clone();
    state.runs.push(state.runs[1].clone());
    denied(&state, &fixture.worker_auth);
    state = fixture.state.clone();
    state.credentials[1].run_id = Uuid::new_v4().to_string();
    denied(&state, &fixture.worker_auth);
    state = fixture.state.clone();
    state.runs[0].kind = RunKind::Worker;
    denied(&state, &fixture.worker_auth);
}

#[test]
fn new_capability_does_not_inherit_an_existing_attachment_conversation() {
    let fixture = Fixture::new();
    for (index, role) in [(0, JobRole::Root), (1, JobRole::Worker)] {
        let run = &fixture.state.runs[index];
        let (record, path) = stage_credential(&fixture.base, run, role).unwrap();
        assert!(record.conversation.is_none());
        let mut caller = fixture.caller(index);
        caller.auth = read_job_auth(&path).unwrap();
        let mut state = fixture.state.clone();
        state.credentials[index] = record;
        assert!(state.runs[index].attachment.as_ref().unwrap().conversation.is_some());
        assert!(validate_caller(&state, &caller).is_err());
        state.credentials[index].conversation = Some(caller.conversation.clone());
        assert!(validate_caller(&state, &caller).is_ok());
    }
}
