use std::{io::{BufRead, BufReader, Read}, path::Path};

use cockpit_core::orchestration::{Actor, AgentCaller, read_job_auth};
use cockpit_protocol::orchestration::ConversationIdentity;

use super::{CliError, args::OrchestrationArgs, context::Context};

const SESSION_HEADER_BYTES: usize = 64 * 1024;

fn identity_value(value: &str) -> Result<(), CliError> {
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(CliError::new("conversation_invalid", "Conversation identity must be printable, nonempty and at most 512 bytes"));
    }
    Ok(())
}

fn agree(explicit: Option<String>, inherited: Option<String>) -> Result<Option<String>, CliError> {
    if explicit.as_ref().zip(inherited.as_ref()).is_some_and(|(a, b)| a != b) {
        return Err(CliError::new("conversation_conflict", "Explicit and inherited conversation identities disagree"));
    }
    Ok(explicit.or(inherited))
}

pub(super) fn session_header(input: impl Read) -> Result<String, CliError> {
    #[derive(serde::Deserialize)]
    struct Header {
        #[serde(rename = "type")]
        kind: String,
        id: Option<String>,
    }
    let mut input = BufReader::new(input.take(SESSION_HEADER_BYTES as u64));
    let mut line = Vec::new();
    let mut total = 0;
    loop {
        line.clear();
        let count = input.read_until(b'\n', &mut line)
            .map_err(|_| CliError::new("session_header_invalid", "Cannot read session header"))?;
        if count == 0 { break; }
        total += count;
        if total == SESSION_HEADER_BYTES && line.last() != Some(&b'\n') {
            return Err(CliError::new("session_header_invalid", "No session header within 64 KiB"));
        }
        if line.iter().all(u8::is_ascii_whitespace) { continue; }
        let header: Header = serde_json::from_slice(&line)
            .map_err(|_| CliError::new("session_header_invalid", "Malformed session header prefix"))?;
        if header.kind != "session" { continue; }
        let id = header.id.ok_or_else(|| CliError::new("session_header_invalid", "Session header has no ID"))?;
        identity_value(&id)?;
        return Ok(id);
    }
    Err(CliError::new("session_header_invalid", "Session file contains no session header"))
}

pub(super) fn resolve_conversation(
    harness: Option<String>, session: Option<String>,
    inherited_harness: Option<String>, inherited_session: Option<String>, file: Option<&Path>,
) -> Result<ConversationIdentity, CliError> {
    let harness = agree(harness, inherited_harness)?
        .ok_or_else(|| CliError::usage("Supply --harness or COCKPIT_HARNESS"))?;
    identity_value(&harness)?;
    let mut session_id = agree(session, inherited_session)?;
    if harness == "omp" {
        if let Some(path) = file {
            let input = std::fs::File::open(path)
                .map_err(|_| CliError::new("session_header_invalid", "Cannot open PI_SESSION_FILE"))?;
            session_id = agree(session_id, Some(session_header(input)?))?;
        }
    }
    let session_id = session_id.ok_or_else(|| CliError::usage("Supply actual --session-id, COCKPIT_SESSION_ID or an OMP PI_SESSION_FILE header"))?;
    identity_value(&session_id)?;
    Ok(ConversationIdentity { harness, session_id })
}

impl OrchestrationArgs {
    pub(super) fn conversation(&self) -> Result<ConversationIdentity, CliError> {
        fn inherited(name: &str) -> Result<Option<String>, CliError> {
            std::env::var(name).map(Some).or_else(|error| match error {
                std::env::VarError::NotPresent => Ok(None),
                _ => Err(CliError::new("conversation_invalid", "Conversation environment must be UTF-8")),
            })
        }
        let file = std::env::var_os("PI_SESSION_FILE").map(std::path::PathBuf::from);
        resolve_conversation(self.harness.clone(), self.session_id.clone(),
            inherited("COCKPIT_HARNESS")?, inherited("COCKPIT_SESSION_ID")?, file.as_deref())
    }
}

pub(super) fn job_actor(args: &OrchestrationArgs) -> Result<Option<Actor>, CliError> {
    let Some(path) = std::env::var_os("COCKPIT_JOB_CREDENTIAL_FILE") else { return Ok(None); };
    let conversation = args.conversation()?;
    let auth = read_job_auth(Path::new(&path))?;
    let epoch = auth.attachment_epoch.to_string();
    for (name, expected) in [
        ("COCKPIT_ATTACHMENT_ID", auth.attachment_id.as_str()),
        ("COCKPIT_ATTACHMENT_EPOCH", epoch.as_str()),
    ] {
        if std::env::var_os(name).is_some_and(|value| value != std::ffi::OsStr::new(expected)) {
            return Err(CliError::new("credential_mismatch", "Launch attachment and credential differ"));
        }
    }
    Ok(Some(Actor::Agent(AgentCaller {
        assignment_generation: auth.assignment_generation,
        auth, conversation,
        endpoint_identity: String::new(), session_id: String::new(),
        workspace_id: String::new(), tab_id: String::new(), pane_id: String::new(),
        boot_id: None, terminal_id: None, native_session_id: None,
        actual_agent_kind: None, process: None,
    })))
}

impl Context {
    pub(super) async fn check_caller(&self) -> Result<(), CliError> {
        if let Some(Actor::Agent(caller)) = &self.actor { self.authenticated_run(caller)?; }
        Ok(())
    }
}
