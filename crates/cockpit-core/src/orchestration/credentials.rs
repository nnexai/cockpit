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
const ENVELOPE_SCHEMA: u32 = 1;
const MAX_CREDENTIAL_BYTES: u64 = 4096;
const MAX_CHILD_ID_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum JobRole {
    Root,
    Worker,
    Child,
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
    pub(crate) subject_child_id: Option<String>,
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
            .field("subject_child_id", &self.subject_child_id)
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
}

impl fmt::Debug for JobAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobAuth")
            .field("credential_id", &self.credential_id)
            .field("secret", &self.secret)
            .field("attachment_id", &self.attachment_id)
            .field("attachment_epoch", &self.attachment_epoch)
            .finish()
    }
}

// All fields are mandatory, including an explicit null for non-child subjects.
// Keeping the envelope separate prevents secret-bearing Serialize from escaping
// into JobAuth or the machine store.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialEnvelope {
    schema: u32,
    credential_id: String,
    run_id: String,
    root_id: String,
    role: JobRole,
    #[serde(deserialize_with = "deserialize_subject")]
    subject_child_id: Option<String>,
    attachment_id: String,
    attachment_epoch: u64,
    secret: String,
}

fn deserialize_subject<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

/// Make one immutable, durable epoch file. The caller must publish the returned
/// digest under the orchestration lock; a staged, unreferenced file grants nothing.
pub(crate) fn stage_credential(
    base: &Path,
    run: &Run,
    role: JobRole,
    subject_child_id: Option<&str>,
) -> Result<(JobCredentialRecord, PathBuf), InspectionError> {
    validate_role(run, role, subject_child_id)?;
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
        subject_child_id: subject_child_id.map(str::to_owned),
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
        subject_child_id: record.subject_child_id.clone(),
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
        || envelope.role != record.role || envelope.subject_child_id != record.subject_child_id
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
    validate_subject(envelope.role, envelope.subject_child_id.as_deref())
        .map_err(|_| invalid_file())?;
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

/// Credential admission solely for the separately pinned physical retirement
/// receipt path. The caller MUST fence its exact RetirementIdentity and purpose;
/// this result never admits ordinary task/messages or serves as their fallback.
/// Only a worker credential qualifies; the caller fences acceptance and physical identity.
pub(crate) fn validate_retirement_job_auth(
    state: &OrchestrationState,
    auth: &JobAuth,
) -> Result<AuthorizedJob, InspectionError> {
    validate_auth(state, auth, AuthPurpose::Retirement)
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
enum AuthPurpose { Ordinary, Retirement, CommittedResult, Cleanup }

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
        || (matches!(purpose, AuthPurpose::Retirement | AuthPurpose::CommittedResult) && record.role != JobRole::Worker)
        || (purpose == AuthPurpose::Cleanup && record.role == JobRole::Child)
    {
        return Err(unauthorized());
    }
    if !matches_digest(&auth.secret, &record.secret_digest)
        || record.attachment_id != auth.attachment_id
        || record.attachment_epoch != auth.attachment_epoch
    {
        return Err(unauthorized());
    }
    let mut matching_runs = state.runs.iter().enumerate().filter(|(_, run)| run.run_id == record.run_id);
    let (run_index, run) = matching_runs.next().ok_or_else(unauthorized)?;
    if matching_runs.next().is_some() || run.root_id != record.root_id {
        return Err(unauthorized());
    }
    validate_run_role(run, record.role, record.subject_child_id.as_deref())?;
    if purpose == AuthPurpose::Ordinary && run.stage == RunStage::Closed {
        return Err(unauthorized());
    }
    if purpose == AuthPurpose::Cleanup && record.role == JobRole::Root && run.stage != RunStage::Active {
        return Err(unauthorized());
    }
    let attachment = run.attachment.as_ref().ok_or_else(unauthorized)?;
    if attachment.attachment_id != auth.attachment_id
        || attachment.epoch != auth.attachment_epoch
        || attachment.state == AttachmentState::Revoked
        || (purpose == AuthPurpose::Ordinary && attachment.state == AttachmentState::Quiescing)
    {
        return Err(unauthorized());
    }
    validate_root_chain(state, run)?;
    let checkout_claim_id = if purpose == AuthPurpose::Cleanup {
        // No task/claim writer authority is returned by cleanup admission.
        None
    } else if purpose == AuthPurpose::Retirement {
        // A conflicting/released writer claim must not prevent stopping this
        // exact still-current process. It grants no source-writer authority.
        run.checkout_claim_id.clone()
    } else { match record.role {
        JobRole::Root => {
            if run.checkout_claim_id.is_some() {
                return Err(unauthorized());
            }
            None
        }
        JobRole::Worker | JobRole::Child => {
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
        subject_child_id: record.subject_child_id.clone(),
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
            return validate_run_role(current, JobRole::Root, None);
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

fn validate_role(run: &Run, role: JobRole, subject: Option<&str>) -> Result<(), InspectionError> {
    validate_run_role(run, role, subject)?;
    if run.stage == RunStage::Closed {
        return Err(unauthorized());
    }
    Ok(())
}

fn validate_run_role(run: &Run, role: JobRole, subject: Option<&str>) -> Result<(), InspectionError> {
    validate_subject(role, subject)?;
    match role {
        JobRole::Root if matches!(run.kind, RunKind::Supervisor | RunKind::Adopted)
            && run.run_id == run.root_id && run.parent_run_id.is_none() => Ok(()),
        JobRole::Worker | JobRole::Child if run.kind == RunKind::Worker
            && run.run_id != run.root_id && run.parent_run_id.is_some() => Ok(()),
        _ => Err(unauthorized()),
    }
}

fn validate_subject(role: JobRole, subject: Option<&str>) -> Result<(), InspectionError> {
    match (role, subject) {
        (JobRole::Root | JobRole::Worker, None) => Ok(()),
        (JobRole::Child, Some(subject)) if !subject.is_empty()
            && subject.len() <= MAX_CHILD_ID_BYTES
            && !subject.chars().any(char::is_control) => Ok(()),
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
    subject: Option<&str>,
) -> JobAuth {
    use cockpit_protocol::orchestration::{CheckoutIdentity, DirectoryIdentity, ExecutionAttachment};
    use super::claims::{AttachmentReference, CheckoutClaim};

    let run = &mut state.runs[index];
    run.attachment.get_or_insert_with(|| ExecutionAttachment {
        attachment_id: Uuid::new_v4().to_string(),
        epoch: 1,
        state: AttachmentState::Connected,
        location: None,
        main_session_id: None,
        process: None,
        shell: None,
        machine_id: None,
        launch_attempt: 1,
        launch_mode: None,
        updated_at: super::now(),
    });
    let (record, auth) = test_job_credential(run, role, subject);
    if role == JobRole::Root {
        run.checkout_claim_id = None;
    } else {
        let checkout = run.checkout.get_or_insert_with(|| CheckoutIdentity {
            key: format!("test-checkout-{}", run.run_id),
            machine_id: "test-machine".into(),
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
    subject: Option<&str>,
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
    identity.update(subject.unwrap_or("").as_bytes());
    let digest = identity.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    let auth = JobAuth {
        credential_id: Uuid::from_bytes(bytes).to_string(),
        secret: SecretBytes::new([0xa7; 32]),
        attachment_id: attachment.attachment_id.clone(),
        attachment_epoch: attachment.epoch,
    };
    let record = JobCredentialRecord {
        credential_id: auth.credential_id.clone(),
        run_id: run.run_id.clone(),
        root_id: run.root_id.clone(),
        role,
        subject_child_id: subject.map(str::to_owned),
        attachment_id: auth.attachment_id.clone(),
        attachment_epoch: auth.attachment_epoch,
        secret_digest: format!("{:x}", Sha256::digest(auth.secret.0)),
        state: CredentialState::Active,
        issued_at: run.updated_at.clone(),
    };
    (record, auth)
}

#[cfg(all(test, unix))]
mod tests {
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
}
