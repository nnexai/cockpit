pub mod herdr;

#[cfg(test)]
mod tests;

use std::{path::Path, sync::Arc};

use cockpit_protocol::quick_ask::{QuickAskAgentAvailability, QuickAskAvailability, QuickAskHarness, QuickAskLocation, QuickAskRequest};
use crate::InspectionError;
use herdr::QuickAskHerdr;

const HARNESS: [(QuickAskHarness, &str, &str); 6] = [
    (QuickAskHarness::Omp, "omp", "omp"),
    (QuickAskHarness::Pi, "pi", "pi"),
    (QuickAskHarness::Claude, "claude", "claude"),
    (QuickAskHarness::Copilot, "copilot", "copilot"),
    (QuickAskHarness::Codex, "codex", "codex"),
    (QuickAskHarness::Opencode, "opencode", "opencode"),
];

pub struct QuickAskService {
    adapter: Arc<dyn QuickAskHerdr>,
}

fn validate(request: &QuickAskRequest) -> Result<(), InspectionError> {
    crate::validate_resource_id(&request.space_id, "space")?;
    let cwd = Path::new(&request.cwd);
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err(InspectionError::new("quick_ask_invalid_cwd", "Working directory must be an existing absolute directory"));
    }
    let text = &request.prompt;
    if text.is_empty() || text.len() > 16384 || text.trim().is_empty() || text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(InspectionError::new("quick_ask_invalid_prompt", "Question must be 1–16384 UTF-8 bytes without control characters other than line feed"));
    }
    Ok(())
}

fn tab_label(prompt: &str) -> String {
    let line = prompt.lines().find(|line| !line.trim().is_empty()).unwrap_or_default();
    let mut chars = line.split_whitespace().flat_map(|word| std::iter::once(' ').chain(word.chars())).skip(1);
    let mut label: String = chars.by_ref().take(32).collect();
    if chars.next().is_some() {
        label.push('…');
    }
    label
}

fn executable_found(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| {
        let Ok(metadata) = std::fs::metadata(dir.join(name)) else { return false };
        if !metadata.is_file() { return false; }
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; metadata.permissions().mode() & 0o111 != 0 }
        #[cfg(not(unix))] { true }
    }))
}

impl QuickAskService {
    pub fn new(adapter: Arc<dyn QuickAskHerdr>) -> Self { Self { adapter } }

    pub fn availability(&self) -> QuickAskAvailability {
        QuickAskAvailability { agents: HARNESS.iter().map(|(h, _, exe)| QuickAskAgentAvailability { harness: *h, executable: (*exe).into(), found: executable_found(exe) }).collect() }
    }

    pub async fn ask(&self, session_id: &str, request: QuickAskRequest) -> Result<QuickAskLocation, InspectionError> {
        crate::validate_session_id(session_id)?;
        validate(&request)?;
        let binding = self.adapter.bind(session_id, &request.server_instance, &request.space_id).await?;
        let (_, kind, _) = HARNESS.iter().find(|(h, _, _)| *h == request.harness).expect("all harnesses are supported");
        self.adapter.create_tab(&binding, &request.cwd, &tab_label(&request.prompt), kind, &request.prompt).await
    }
}
