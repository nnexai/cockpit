use std::{
    fmt,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::orchestration::{AttachmentState, Run, RunKind, RunStage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{AuthorizedJob, claims::{ClaimState, checkouts_conflict}, store::OrchestrationState};
use crate::{InspectionError, project_store::open_dir_nofollow_absolute};

const CREDENTIAL_DIRECTORY: &str = "credentials";
const ENVELOPE_SCHEMA: u32 = 2;
const MAX_CREDENTIAL_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum JobRole {
    Root,
    Worker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CredentialState {
    Active,
    Revoked,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobCredentialRecord {
    pub(crate) credential_id: String,
    pub(crate) run_id: String,
    pub(crate) root_id: String,
    pub(crate) role: JobRole,
    pub(crate) assignment_generation: Option<u64>,
    pub(crate) conversation: Option<cockpit_protocol::orchestration::ConversationIdentity>,
    pub(crate) attachment_id: String,
    pub(crate) attachment_epoch: u64,
    pub(crate) secret_digest: String,
    pub(crate) state: CredentialState,
    pub(crate) issued_at: String,
}

impl fmt::Debug for JobCredentialRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobCredentialRecord")
            .field("credential_id", &self.credential_id)
            .field("run_id", &self.run_id)
            .field("root_id", &self.root_id)
            .field("role", &self.role)
            .field("assignment_generation", &self.assignment_generation)
            .field("attachment_id", &self.attachment_id)
            .field("attachment_epoch", &self.attachment_epoch)
            .field("secret_digest", &"[REDACTED]")
            .field("state", &self.state)
            .field("issued_at", &self.issued_at)
            .finish()
    }
}

/// Deliberately has no serialization implementation or public byte accessor.
#[derive(Clone)]
pub struct SecretBytes([u8; 32]);

impl SecretBytes {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretBytes([REDACTED])")
    }
}

/// Private transport context, never an action payload or public protocol DTO.
#[derive(Clone)]
pub struct JobAuth {
    pub credential_id: String,
    pub secret: SecretBytes,
    pub attachment_id: String,
    pub attachment_epoch: u64,
    pub assignment_generation: Option<u64>,
}

impl fmt::Debug for JobAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobAuth")
            .field("credential_id", &self.credential_id)
            .field("secret", &self.secret)
            .field("attachment_id", &self.attachment_id)
            .field("attachment_epoch", &self.attachment_epoch)
            .field("assignment_generation", &self.assignment_generation)
            .finish()
    }
}

// Keep secret-bearing serialization confined to the immutable private envelope.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialEnvelope {
    schema: u32,
    credential_id: String,
    run_id: String,
    root_id: String,
    role: JobRole,
    assignment_generation: Option<u64>,
    attachment_id: String,
    attachment_epoch: u64,
    secret: String,
}


/// Make one immutable, durable epoch file. The caller must publish the returned
/// digest under the orchestration lock; a staged, unreferenced file grants nothing.
pub(crate) fn stage_credential(
    base: &Path,
    run: &Run,
    role: JobRole,
) -> Result<(JobCredentialRecord, PathBuf), InspectionError> {
    validate_role(run, role)?;
    valid_uuid(&run.run_id)?;
    valid_uuid(&run.root_id)?;
    let attachment = run.attachment.as_ref().ok_or_else(unauthorized)?;
    if matches!(attachment.state, AttachmentState::Revoked | AttachmentState::Quiescing) {
        return Err(unauthorized());
    }
    valid_uuid(&attachment.attachment_id)?;
    valid_epoch(attachment.epoch)?;
    if !base.is_absolute() {
        return Err(unsafe_file());
    }
    let directory = credential_directory(base)?;
    let secret = secure_secret()?;
    let credential_id = Uuid::new_v4().to_string();
    let record = JobCredentialRecord {
        credential_id: credential_id.clone(),
        run_id: run.run_id.clone(),
        root_id: run.root_id.clone(),
        role,
        assignment_generation: run.assignment_generation,
        conversation: None,
        attachment_id: attachment.attachment_id.clone(),
        attachment_epoch: attachment.epoch,
        secret_digest: format!("{:x}", Sha256::digest(secret.0)),
        state: CredentialState::Active,
        issued_at: super::now(),
    };
    let envelope = CredentialEnvelope {
        schema: ENVELOPE_SCHEMA,
        credential_id,
        run_id: run.run_id.clone(),
        root_id: run.root_id.clone(),
        role,
        assignment_generation: record.assignment_generation,
        attachment_id: record.attachment_id.clone(),
        attachment_epoch: record.attachment_epoch,
        secret: STANDARD.encode(secret.0),
    };
    let bytes = serde_json::to_vec(&envelope).map_err(|_| credential_io())?;
    if bytes.len() as u64 > MAX_CREDENTIAL_BYTES {
        return Err(invalid_file());
    }
    let name = credential_name(&record.attachment_id, record.attachment_epoch, &record.credential_id);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).follow(cap_fs_ext::FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = directory.open_with(&name, &options).map_err(|e| credential_io_at("file_create", e))?;
    #[cfg(unix)]
    rustix::fs::fchmod(&file, rustix::fs::Mode::from_raw_mode(0o600))
        .map_err(|e| credential_io_at("file_permissions", e.into()))?;
    check_private_file(&file.metadata().map_err(|_| unsafe_file())?)?;
    file.write_all(&bytes).map_err(|e| credential_io_at("file_write", e))?;
    file.sync_all().map_err(|e| credential_io_at("file_sync", e))?;
    directory.open(".").and_then(|file| file.sync_all()).map_err(|e| credential_io_at("directory_sync", e))?;
    Ok((record, base.join(CREDENTIAL_DIRECTORY).join(name)))
}

/// Read through a pinned directory/file descriptor. Neither serde nor I/O errors
/// are forwarded: their text could contain secret input or its private path.
pub fn read_job_auth(path: &Path) -> Result<JobAuth, InspectionError> {
    let (envelope, secret) = read_envelope(path)?;
    Ok(JobAuth {
        credential_id: envelope.credential_id,
        secret,
        attachment_id: envelope.attachment_id,
        attachment_epoch: envelope.attachment_epoch,
        assignment_generation: envelope.assignment_generation,
    })
}

/// Rediscover only the exact immutable file whose digest is already published.
/// Restart must never rotate the secret or deliver a different attachment file.
pub(crate) fn credential_path(
    base: &Path,
    record: &JobCredentialRecord,
) -> Result<PathBuf, InspectionError> {
    valid_uuid(&record.credential_id)?;
    valid_uuid(&record.run_id)?;
    valid_uuid(&record.root_id)?;
    valid_uuid(&record.attachment_id)?;
    valid_epoch(record.attachment_epoch)?;
    if !base.is_absolute() || record.state != CredentialState::Active {
        return Err(unauthorized());
    }
    let path = base.join(CREDENTIAL_DIRECTORY).join(credential_name(
        &record.attachment_id, record.attachment_epoch, &record.credential_id,
    ));
    let (envelope, secret) = read_envelope(&path)?;
    if envelope.run_id != record.run_id || envelope.root_id != record.root_id
        || envelope.role != record.role || envelope.assignment_generation != record.assignment_generation
        || !matches_digest(&secret, &record.secret_digest)
    {
        return Err(unauthorized());
    }
    Ok(path)
}

fn read_envelope(path: &Path) -> Result<(CredentialEnvelope, SecretBytes), InspectionError> {
    if !path.is_absolute() {
        return Err(unsafe_file());
    }
    let parent = path.parent().ok_or_else(unsafe_file)?;
    if parent.file_name().and_then(|name| name.to_str()) != Some(CREDENTIAL_DIRECTORY) {
        return Err(unsafe_file());
    }
    let name = path.file_name().and_then(|name| name.to_str()).ok_or_else(unsafe_file)?;
    let directory = open_dir_nofollow_absolute(parent).map_err(|_| unsafe_file())?;
    check_private_directory(&directory)?;
    let metadata = directory.symlink_metadata(name).map_err(|_| unsafe_file())?;
    check_private_file(&metadata)?;
    let mut options = OpenOptions::new();
    options.read(true).follow(cap_fs_ext::FollowSymlinks::No).nonblock(true);
    let mut file = directory.open_with(name, &options).map_err(|_| unsafe_file())?;
    let metadata = file.metadata().map_err(|_| unsafe_file())?;
    check_private_file(&metadata)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&mut file).take(MAX_CREDENTIAL_BYTES + 1).read_to_end(&mut bytes)
        .map_err(|_| invalid_file())?;
    if bytes.len() as u64 > MAX_CREDENTIAL_BYTES {
        return Err(invalid_file());
    }
    let envelope: CredentialEnvelope = serde_json::from_slice(&bytes).map_err(|_| invalid_file())?;
    if envelope.schema != ENVELOPE_SCHEMA {
        return Err(invalid_file());
    }
    valid_uuid(&envelope.credential_id).map_err(|_| invalid_file())?;
    valid_uuid(&envelope.run_id).map_err(|_| invalid_file())?;
    valid_uuid(&envelope.root_id).map_err(|_| invalid_file())?;
    valid_uuid(&envelope.attachment_id).map_err(|_| invalid_file())?;
    valid_epoch(envelope.attachment_epoch).map_err(|_| invalid_file())?;
    validate_generation(envelope.role, envelope.assignment_generation).map_err(|_| invalid_file())?;
    if (envelope.role == JobRole::Root) != (envelope.run_id == envelope.root_id)
        || name != credential_name(&envelope.attachment_id, envelope.attachment_epoch, &envelope.credential_id)
        || envelope.secret.len() != 44
    {
        return Err(invalid_file());
    }
    let mut secret = [0u8; 32];
    let decoded = STANDARD.decode_slice(envelope.secret.as_bytes(), &mut secret)
        .map_err(|_| invalid_file())?;
    if decoded != secret.len() {
        return Err(invalid_file());
    }
    Ok((envelope, SecretBytes(secret)))
}

/// Call only against the same locked state that will receive the mutation.
/// Conversation/process provenance cannot enlarge this durable role authority.
pub(crate) fn validate_job_auth(
    state: &OrchestrationState,
    auth: &JobAuth,
) -> Result<AuthorizedJob, InspectionError> {
    validate_auth(state, auth, AuthPurpose::Ordinary)
}

pub(crate) fn validate_registration(
    state: &OrchestrationState, caller: &super::AgentCaller,
) -> Result<AuthorizedJob, InspectionError> {
    validate_conversation(caller.conversation.as_ref())?;
    let job = validate_auth(state, &caller.auth, AuthPurpose::Registration)?;
    let record = state.credentials.iter().find(|record| record.credential_id == caller.auth.credential_id).expect("validated capability");
    let run = &state.runs[job.run_index];
    if conversation_mismatch(record.conversation.as_ref(), caller.conversation.as_ref())
        || caller.assignment_generation != job.assignment_generation
        || run.assignment_generation != job.assignment_generation
        || caller.conversation.as_ref().is_some_and(|actual| run.launch_config.harness_kind != actual.harness) {
        return Err(unauthorized());
    }
    let attachment = run.attachment.as_ref().ok_or_else(unauthorized)?;
    if conversation_mismatch(attachment.conversation.as_ref(), caller.conversation.as_ref()) {
        return Err(unauthorized());
    }
    if job.role == JobRole::Worker {
        let generation = job.assignment_generation.ok_or_else(unauthorized)?;
        let task_id = run.task_id.as_ref().ok_or_else(unauthorized)?;
        let ownership = state.task_ownership.iter()
            .find(|record| record.root_id == run.root_id && &record.task_id == task_id)
            .ok_or_else(unauthorized)?;
        if let Some(owner) = &ownership.owner {
            if ownership.generation != generation || owner.run_id != run.run_id
                || conversation_mismatch(owner.conversation.as_ref(), caller.conversation.as_ref()) {
                return Err(unauthorized());
            }
        } else if ownership.generation.checked_add(1) != Some(generation) {
            return Err(unauthorized());
        }
    }
    Ok(job)
}

pub(crate) fn validate_caller(
    state: &OrchestrationState, caller: &super::AgentCaller,
) -> Result<AuthorizedJob, InspectionError> {
    let job = validate_job_auth(state, &caller.auth)?;
    validate_current_assignment(state, caller, &job)?;
    Ok(job)
}

pub(crate) fn validate_current_assignment(
    state: &OrchestrationState, caller: &super::AgentCaller, job: &AuthorizedJob,
) -> Result<(), InspectionError> {
    let bound = state.credentials.iter().find(|record| record.credential_id == caller.auth.credential_id)
        .and_then(|record| record.conversation.as_ref());
    if conversation_mismatch(bound, caller.conversation.as_ref()) { return Err(unauthorized()); }
    validate_assignment(state, caller, job)
}

fn validate_assignment(
    state: &OrchestrationState, caller: &super::AgentCaller, job: &AuthorizedJob,
) -> Result<(), InspectionError> {
    validate_conversation(caller.conversation.as_ref())?;
    let run = &state.runs[job.run_index];
    if conversation_mismatch(run.attachment.as_ref().and_then(|a| a.conversation.as_ref()), caller.conversation.as_ref())
        || caller.assignment_generation != job.assignment_generation {
        return Err(unauthorized());
    }
    if job.role == JobRole::Worker {
        let ownership = state.task_ownership.iter().find(|record|
            record.root_id == run.root_id && Some(&record.task_id) == run.task_id.as_ref())
            .ok_or_else(unauthorized)?;
        let owner = ownership.owner.as_ref().ok_or_else(unauthorized)?;
        if Some(ownership.generation) != job.assignment_generation
            || run.assignment_generation != job.assignment_generation
            || owner.run_id != run.run_id || conversation_mismatch(owner.conversation.as_ref(), caller.conversation.as_ref()) {
            return Err(unauthorized());
        }
    } else if caller.assignment_generation.is_some() {
        return Err(unauthorized());
    }
    Ok(())
}

fn conversation_mismatch(bound: Option<&cockpit_protocol::orchestration::ConversationIdentity>,
    actual: Option<&cockpit_protocol::orchestration::ConversationIdentity>) -> bool {
    matches!((bound, actual), (Some(bound), Some(actual)) if bound != actual)
}

fn validate_conversation(actual: Option<&cockpit_protocol::orchestration::ConversationIdentity>) -> Result<(), InspectionError> {
    if let Some(actual) = actual {
        super::validate_identity(&actual.harness)?;
        super::validate_identity(&actual.session_id)?;
    }
    Ok(())
}


/// Scheduling only: existing owned-resource cleanup, never source writes,
/// resource registration or retirement. The caller fences exact request scope.
pub(crate) fn validate_cleanup_job_auth(
    state: &OrchestrationState, auth: &JobAuth,
) -> Result<AuthorizedJob, InspectionError> {
    validate_auth(state, auth, AuthPurpose::Cleanup)
}

/// Read-only admission for an already committed main Result receipt. The
/// The facade verifies its principal and exact committed payload digest;
/// new writes, unlike inert receipts, also require a fresh task/claim fence.
/// This purpose cannot admit a new report or any lifecycle effect.
pub(crate) fn validate_committed_result_auth(
    state: &OrchestrationState, auth: &JobAuth, message_id: &str,
) -> Result<AuthorizedJob, InspectionError> {
    let job = validate_auth(state, auth, AuthPurpose::CommittedResult)?;
    let run = &state.runs[job.run_index];
    if run.result.as_ref().is_none_or(|r| r.message_id != message_id)
        || !state.operations.iter().any(|r| r.operation_id == message_id
            && r.actor_run_id.as_deref() == Some(run.run_id.as_str())
            && r.kind == super::store::DurableOperationKind::Report && r.result.is_some()) {
        return Err(unauthorized());
    }
    Ok(job)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthPurpose { Ordinary, Registration, CommittedResult, Cleanup }

fn validate_auth(
    state: &OrchestrationState,
    auth: &JobAuth,
    purpose: AuthPurpose,
) -> Result<AuthorizedJob, InspectionError> {
    valid_uuid(&auth.credential_id)?;
    valid_uuid(&auth.attachment_id)?;
    valid_epoch(auth.attachment_epoch)?;
    let mut records = state.credentials.iter().filter(|record| record.credential_id == auth.credential_id);
    let record = records.next().ok_or_else(unauthorized)?;
    if records.next().is_some() || record.state != CredentialState::Active
        || (purpose == AuthPurpose::CommittedResult && record.role != JobRole::Worker)
    {
        return Err(unauthorized());
    }
    if !matches_digest(&auth.secret, &record.secret_digest)
        || record.attachment_id != auth.attachment_id
        || record.attachment_epoch != auth.attachment_epoch
        || record.assignment_generation != auth.assignment_generation
    {
        return Err(unauthorized());
    }
    let mut matching_runs = state.runs.iter().enumerate().filter(|(_, run)| run.run_id == record.run_id);
    let (run_index, run) = matching_runs.next().ok_or_else(unauthorized)?;
    if matching_runs.next().is_some() || run.root_id != record.root_id
        || run.assignment_generation != record.assignment_generation {
        return Err(unauthorized());
    }
    validate_run_role(run, record.role)?;
    if matches!(purpose, AuthPurpose::Ordinary | AuthPurpose::Registration) && run.stage == RunStage::Closed {
        return Err(unauthorized());
    }
    if purpose == AuthPurpose::Cleanup && record.role == JobRole::Root && run.stage != RunStage::Active {
        return Err(unauthorized());
    }
    let attachment = run.attachment.as_ref().ok_or_else(unauthorized)?;
    if attachment.attachment_id != auth.attachment_id
        || attachment.epoch != auth.attachment_epoch
        || attachment.state == AttachmentState::Revoked
        || (matches!(purpose, AuthPurpose::Ordinary | AuthPurpose::Registration) && attachment.state == AttachmentState::Quiescing)
    {
        return Err(unauthorized());
    }
    validate_root_chain(state, run)?;
    let checkout_claim_id = if purpose == AuthPurpose::Cleanup {
        // No task/claim writer authority is returned by cleanup admission.
        None
    } else { match record.role {
        JobRole::Root => {
            if run.checkout_claim_id.is_some() {
                return Err(unauthorized());
            }
            None
        }
        JobRole::Worker => {
            let claim_id = run.checkout_claim_id.as_ref().ok_or_else(unauthorized)?;
            let mut claims = state.checkout_claims.iter().filter(|claim| &claim.claim_id == claim_id);
            let claim = claims.next().ok_or_else(unauthorized)?;
            if claims.next().is_some() || !matches!(claim.state, ClaimState::Reserved | ClaimState::Held) {
                return Err(unauthorized());
            }
            let holder = claim.holder.as_ref().ok_or_else(unauthorized)?;
            if holder.run_id != run.run_id
                || holder.attachment_id != auth.attachment_id
                || holder.attachment_epoch != auth.attachment_epoch
                || run.checkout.as_ref().is_none_or(|checkout| checkout != &claim.checkout)
                || state.checkout_claims.iter().any(|other| {
                    other.claim_id != claim.claim_id
                        && other.state != ClaimState::Released
                        && checkouts_conflict(&claim.checkout, &other.checkout)
                })
            {
                return Err(unauthorized());
            }
            Some(claim_id.clone())
        }
    }};
    Ok(AuthorizedJob {
        run_index,
        role: record.role,
        assignment_generation: record.assignment_generation,
        attachment_id: record.attachment_id.clone(),
        attachment_epoch: record.attachment_epoch,
        checkout_claim_id,
    })
}

fn validate_root_chain(state: &OrchestrationState, run: &Run) -> Result<(), InspectionError> {
    let mut current = run;
    for _ in 0..state.runs.len() {
        if current.root_id != run.root_id {
            return Err(unauthorized());
        }
        if current.run_id == run.root_id {
            return validate_run_role(current, JobRole::Root);
        }
        if current.kind != RunKind::Worker {
            return Err(unauthorized());
        }
        let parent_id = current.parent_run_id.as_ref().ok_or_else(unauthorized)?;
        let mut parents = state.runs.iter().filter(|parent| &parent.run_id == parent_id);
        current = parents.next().ok_or_else(unauthorized)?;
        if parents.next().is_some() {
            return Err(unauthorized());
        }
    }
    Err(unauthorized())
}

fn validate_role(run: &Run, role: JobRole) -> Result<(), InspectionError> {
    validate_run_role(run, role)?;
    if run.stage == RunStage::Closed {
        return Err(unauthorized());
    }
    Ok(())
}

fn validate_run_role(run: &Run, role: JobRole) -> Result<(), InspectionError> {
    validate_generation(role, run.assignment_generation)?;
    match role {
        JobRole::Root if matches!(run.kind, RunKind::Supervisor | RunKind::Adopted)
            && run.run_id == run.root_id && run.parent_run_id.is_none() => Ok(()),
        JobRole::Worker if run.kind == RunKind::Worker
            && run.run_id != run.root_id && run.parent_run_id.is_some() => Ok(()),
        _ => Err(unauthorized()),
    }
}

fn validate_generation(role: JobRole, generation: Option<u64>) -> Result<(), InspectionError> {
    match (role, generation) {
        (JobRole::Root, None) => Ok(()),
        (JobRole::Worker, Some(value)) if value > 0 && value <= super::MAX_SAFE_COUNTER => Ok(()),
        _ => Err(unauthorized()),
    }
}


fn valid_uuid(value: &str) -> Result<(), InspectionError> {
    Uuid::parse_str(value).map(|_| ()).map_err(|_| unauthorized())
}

fn valid_epoch(epoch: u64) -> Result<(), InspectionError> {
    if epoch == 0 || epoch > super::MAX_SAFE_COUNTER { Err(unauthorized()) } else { Ok(()) }
}

fn credential_name(attachment_id: &str, epoch: u64, credential_id: &str) -> String {
    format!("{attachment_id}-{epoch}-{credential_id}.json")
}

fn matches_digest(secret: &SecretBytes, digest: &str) -> bool {
    let Some(expected) = decode_digest(digest) else { return false; };
    let actual: [u8; 32] = Sha256::digest(secret.0).into();
    expected.iter().zip(actual.iter())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right)) == 0
}

fn decode_digest(digest: &str) -> Option<[u8; 32]> {
    if digest.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (out, pair) in bytes.iter_mut().zip(digest.as_bytes().chunks_exact(2)) {
        let nibble = |byte| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        *out = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(bytes)
}

#[cfg(unix)]
fn credential_directory(base: &Path) -> Result<Dir, InspectionError> {
    use cap_std::fs::{DirBuilder, DirBuilderExt};
    let parent = open_dir_nofollow_absolute(base).map_err(|_| unsafe_file())?;
    let mut builder = DirBuilder::new();
    builder.mode(0o700);
    let created = match parent.create_dir_with(CREDENTIAL_DIRECTORY, &builder) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
        Err(e) => return Err(credential_io_at("directory_create", e)),
    };
    let directory = parent.open_dir_nofollow(CREDENTIAL_DIRECTORY).map_err(|_| unsafe_file())?;
    if created {
        // Capability directories are O_PATH descriptors on Linux, which
        // fchmod rejects with EBADF. Reopen the pinned directory for reading
        // rather than chmod through its name or weaken the mode check.
        let readable = directory.open(".").map_err(|e| credential_io_at("directory_reopen", e))?;
        rustix::fs::fchmod(&readable, rustix::fs::Mode::from_raw_mode(0o700))
            .map_err(|e| credential_io_at("directory_permissions", e.into()))?;
    }
    check_private_directory(&directory)?;
    parent.open(".").and_then(|file| file.sync_all()).map_err(|e| credential_io_at("parent_sync", e))?;
    Ok(directory)
}

#[cfg(not(unix))]
fn credential_directory(_base: &Path) -> Result<Dir, InspectionError> {
    Err(unsupported())
}

#[cfg(unix)]
fn check_private_directory(directory: &Dir) -> Result<(), InspectionError> {
    use cap_std::fs::MetadataExt;
    let metadata = directory.dir_metadata().map_err(|_| unsafe_file())?;
    if !metadata.is_dir() || metadata.mode() & 0o7777 != 0o700
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
    {
        return Err(unsafe_file());
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_private_directory(_directory: &Dir) -> Result<(), InspectionError> {
    Err(unsupported())
}

#[cfg(unix)]
fn check_private_file(metadata: &cap_std::fs::Metadata) -> Result<(), InspectionError> {
    use cap_std::fs::MetadataExt;
    if !metadata.is_file() || metadata.file_type().is_symlink()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.nlink() != 1 || metadata.len() > MAX_CREDENTIAL_BYTES
    {
        return Err(unsafe_file());
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_private_file(_metadata: &cap_std::fs::Metadata) -> Result<(), InspectionError> {
    Err(unsupported())
}

#[cfg(unix)]
fn secure_secret() -> Result<SecretBytes, InspectionError> {
    use cap_std::fs::MetadataExt;
    let directory = open_dir_nofollow_absolute(Path::new("/dev")).map_err(|_| entropy_error())?;
    let mut options = OpenOptions::new();
    options.read(true).follow(cap_fs_ext::FollowSymlinks::No).nonblock(true);
    let mut file = directory.open_with("urandom", &options).map_err(|_| entropy_error())?;
    if file.metadata().map_err(|_| entropy_error())?.mode() & 0o170000 != 0o020000 {
        return Err(entropy_error());
    }
    let mut bytes = [0u8; 32];
    file.read_exact(&mut bytes).map_err(|_| entropy_error())?;
    Ok(SecretBytes(bytes))
}

#[cfg(not(unix))]
fn secure_secret() -> Result<SecretBytes, InspectionError> {
    Err(unsupported())
}

fn unauthorized() -> InspectionError {
    InspectionError::new("job_unauthorized", "Job credential is not authorized for the current attachment and checkout claim")
}
fn unsafe_file() -> InspectionError {
    InspectionError::new("unsafe_path", "Job credential must be an owner-private bounded regular file in a private no-follow directory")
}
fn invalid_file() -> InspectionError {
    InspectionError::new("job_credential_invalid", "Job credential envelope is invalid")
}
fn credential_io() -> InspectionError {
    InspectionError::new("job_credential_io", "Cannot durably stage job credential")
}
fn credential_io_at(phase: &'static str, error: io::Error) -> InspectionError {
    // Preserve actionable OS categories without forwarding an error string,
    // filename, envelope value, or secret-bearing input.
    InspectionError::new("job_credential_io", format!(
        "Cannot durably stage job credential (phase={phase}, kind={:?}, errno={:?})",
        error.kind(), error.raw_os_error(),
    ))
}
#[cfg(unix)]
fn entropy_error() -> InspectionError {
    InspectionError::new("job_credential_entropy", "Secure operating-system randomness is unavailable")
}
#[cfg(not(unix))]
fn unsupported() -> InspectionError {
    InspectionError::new("job_credential_unsupported", "Private job credentials are unavailable on this platform")
}

/// Shared in-memory fixture issuer. No file I/O, external effects or production
/// entropy fallback; tests still exercise the real role/epoch/claim admission.
#[cfg(test)]
pub(crate) fn test_job_auth(
    state: &mut OrchestrationState,
    index: usize,
    role: JobRole,
) -> JobAuth {
    use cockpit_protocol::orchestration::{CheckoutIdentity, DirectoryIdentity, ExecutionAttachment};
    use super::claims::{AttachmentReference, CheckoutClaim};

    let run = &mut state.runs[index];
    run.attachment.get_or_insert_with(|| ExecutionAttachment {
        attachment_id: Uuid::new_v4().to_string(),
        epoch: 1,
        state: AttachmentState::Connected,
        location: None,
        conversation: None,
        process: None,
        shell: None,
        machine_id: None,
        process_scope: None,
        launch_attempt: 1,
        launch_mode: None,
        updated_at: super::now(),
    });
    let (record, auth) = test_job_credential(run, role);
    if role == JobRole::Root {
        run.checkout_claim_id = None;
    } else {
        let checkout = run.checkout.get_or_insert_with(|| CheckoutIdentity {
            key: format!("test-checkout-{}", run.run_id),
            path: format!("/test-checkouts/{}", run.run_id),
            directory_identity: DirectoryIdentity { device: "test-device".into(), inode: run.run_id.clone() },
            git: None,
        });
        let claim_id = run.checkout_claim_id.get_or_insert_with(|| Uuid::new_v4().to_string()).clone();
        let holder = AttachmentReference {
            run_id: run.run_id.clone(),
            attachment_id: auth.attachment_id.clone(),
            attachment_epoch: auth.attachment_epoch,
        };
        let claim = CheckoutClaim {
            claim_id: claim_id.clone(),
            checkout: checkout.clone(),
            revision: 1,
            state: ClaimState::Held,
            holder: Some(holder),
            transition_operation_id: None,
        };
        if let Some(existing) = state.checkout_claims.iter_mut().find(|existing| existing.claim_id == claim_id) {
            *existing = claim;
        } else {
            state.checkout_claims.push(claim);
        }
        if let (Some(generation), Some(task_id)) = (run.assignment_generation, run.task_id.clone()) {
            let ownership = cockpit_protocol::orchestration::TaskOwnership { root_id: run.root_id.clone(), task_id,
                generation, schedule: None, owner: Some(cockpit_protocol::orchestration::TaskOwner { run_id: run.run_id.clone(),
                    conversation: run.attachment.as_ref().and_then(|a| a.conversation.clone()), claim_operation_id: auth.credential_id.clone() }) };
            if let Some(existing) = state.task_ownership.iter_mut().find(|record|
                record.root_id == ownership.root_id && record.task_id == ownership.task_id) {
                *existing = ownership;
            } else { state.task_ownership.push(ownership); }
        }
    }
    if let Some(existing) = state.credentials.iter_mut().find(|existing| existing.credential_id == record.credential_id) {
        *existing = record;
    } else {
        state.credentials.push(record);
    }
    auth
}

/// Deterministic issuer for independently constructed, aliased test actors.
/// Provision the attachment/claim first; unlike test_job_auth this never edits
/// the supplied run or grants any permissions on its own.
#[cfg(test)]
pub(crate) fn test_job_credential(
    run: &Run,
    role: JobRole,
) -> (JobCredentialRecord, JobAuth) {
    let attachment = run.attachment.as_ref().expect("credential test run has an attachment");
    let mut identity = Sha256::new();
    identity.update(b"cockpit-test-job-credential-v1\0");
    for field in [&run.run_id, &run.root_id, &attachment.attachment_id] {
        identity.update(field.as_bytes());
        identity.update([0]);
    }
    identity.update(attachment.epoch.to_le_bytes());
    identity.update([role as u8]);
    identity.update(run.assignment_generation.unwrap_or(0).to_le_bytes());
    let digest = identity.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    let auth = JobAuth {
        credential_id: Uuid::from_bytes(bytes).to_string(),
        secret: SecretBytes::new([0xa7; 32]),
        attachment_id: attachment.attachment_id.clone(),
        attachment_epoch: attachment.epoch,
        assignment_generation: run.assignment_generation,
    };
    let record = JobCredentialRecord {
        credential_id: auth.credential_id.clone(),
        run_id: run.run_id.clone(),
        root_id: run.root_id.clone(),
        role,
        assignment_generation: run.assignment_generation,
        conversation: attachment.conversation.clone(),
        attachment_id: auth.attachment_id.clone(),
        attachment_epoch: auth.attachment_epoch,
        secret_digest: format!("{:x}", Sha256::digest(auth.secret.0)),
        state: CredentialState::Active,
        issued_at: run.updated_at.clone(),
    };
    (record, auth)
}

#[cfg(all(test, unix))]
mod tests;
