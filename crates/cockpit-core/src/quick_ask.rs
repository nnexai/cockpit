pub mod herdr;

#[cfg(test)]
mod tests;

use std::{ffi::{OsStr, OsString}, path::Path, sync::Arc};

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
    path: tokio::sync::OnceCell<OsString>,
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

fn executable_found(name: &str, path: &OsStr) -> bool {
    std::env::split_paths(path).any(|dir| {
        let Ok(metadata) = std::fs::metadata(dir.join(name)) else { return false };
        if !metadata.is_file() { return false; }
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; metadata.permissions().mode() & 0o111 != 0 }
        #[cfg(not(unix))] { true }
    })
}

#[cfg(unix)]
async fn shell_path(shell: &Path, fallback: OsString) -> OsString {
    use tokio::io::AsyncReadExt;
    use std::{os::unix::ffi::OsStringExt, process::Stdio, time::Duration};

    let marker = format!("cockpit-path-{}", uuid::Uuid::new_v4());
    let command = format!("printf '\\000{}\\000%s\\000' \"$PATH\"", marker);
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let mut child = tokio::process::Command::new(shell)
            .args(["-ilc", &command])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .kill_on_drop(true).spawn().ok()?;
        let mut output = Vec::new();
        child.stdout.take()?.take(65537).read_to_end(&mut output).await.ok()?;
        if output.len() > 65536 || !child.wait().await.ok()?.success() { return None; }
        let frame = format!("\0{}\0", marker);
        let start = output.windows(frame.len()).position(|bytes| bytes == frame.as_bytes())? + frame.len();
        let end = output[start..].iter().position(|byte| *byte == 0)? + start;
        if start == end { return None; }
        Some(OsString::from_vec(output[start..end].to_vec()))
    }).await;
    result.ok().flatten().unwrap_or(fallback)
}

#[cfg(unix)]
fn include_nvm_clients(path: OsString, nvm: &Path) -> OsString {
    let Ok(entries) = std::fs::read_dir(nvm.join("versions/node")) else { return path };
    let mut versions = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str().and_then(|name| name.strip_prefix('v')) else { continue };
        let mut parts = name.split('.');
        let (Some(major), Some(minor), Some(patch), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { continue };
        if [major, minor, patch].iter().any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) || (part.len() > 1 && part.starts_with('0'))) { continue; }
        let (Ok(major), Ok(minor), Ok(patch)) = (major.parse::<u64>(), minor.parse::<u64>(), patch.parse::<u64>()) else { continue };
        let bin = entry.path().join("bin");
        if executable_found("node", bin.as_os_str()) { versions.push(((major, minor, patch), bin)); }
    }
    versions.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let mut paths: Vec<_> = std::env::split_paths(&path).collect();
    let mut missing: Vec<_> = HARNESS.iter().map(|(_, _, exe)| *exe).filter(|exe| !executable_found(exe, &path)).collect();
    for (_, bin) in versions {
        if missing.is_empty() { break; }
        let before = missing.len();
        missing.retain(|exe| !executable_found(exe, bin.as_os_str()));
        if missing.len() != before && !paths.contains(&bin) { paths.push(bin); }
    }
    std::env::join_paths(paths).unwrap_or(path)
}

async fn discovery_path() -> OsString {
    let fallback = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(unix)]
    {
        let shell = std::env::var_os("SHELL").filter(|s| Path::new(s).is_absolute())
            .map(std::path::PathBuf::from)
            .or_else(|| nix::unistd::User::from_uid(nix::unistd::Uid::current()).ok().flatten().map(|user| user.shell));
        let path = if let Some(shell) = shell { shell_path(&shell, fallback).await } else { fallback };
        // Relative PATH entries must refer to the same directories when the tab
        // starts in a different working directory.
        let path = if let Ok(cwd) = std::env::current_dir() {
            std::env::join_paths(std::env::split_paths(&path).map(|dir| if dir.is_absolute() { dir } else { cwd.join(dir) })).unwrap_or(path)
        } else { path };
        let nvm = std::env::var_os("NVM_DIR").map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".nvm")));
        return match nvm {
            Some(nvm) => {
                let nvm = if nvm.is_absolute() { nvm } else if let Ok(cwd) = std::env::current_dir() { cwd.join(nvm) } else { return path };
                include_nvm_clients(path, &nvm)
            }
            None => path,
        };
    }
    #[cfg(not(unix))]
    fallback
}

impl QuickAskService {
    pub fn new(adapter: Arc<dyn QuickAskHerdr>) -> Self { Self { adapter, path: tokio::sync::OnceCell::new() } }

    async fn path(&self) -> &OsStr {
        self.path.get_or_init(discovery_path).await.as_os_str()
    }

    pub async fn availability(&self) -> QuickAskAvailability {
        let path = self.path().await;
        QuickAskAvailability { agents: HARNESS.iter().map(|(h, _, exe)| QuickAskAgentAvailability { harness: *h, executable: (*exe).into(), found: executable_found(exe, path) }).collect() }
    }
    pub async fn install_update(&self, session_id: &str, space_id: &str, cwd: &str, command: &[String], label: &str) -> Result<QuickAskLocation, InspectionError> {
        self.adapter.install_update(session_id, space_id, cwd, command, label).await
    }


    pub async fn ask(&self, session_id: &str, request: QuickAskRequest) -> Result<QuickAskLocation, InspectionError> {
        crate::validate_session_id(session_id)?;
        validate(&request)?;
        let binding = self.adapter.bind(session_id, &request.server_instance, &request.space_id).await?;
        let (_, kind, _) = HARNESS.iter().find(|(h, _, _)| *h == request.harness).expect("all harnesses are supported");
        let path = self.path().await.to_str().ok_or_else(|| InspectionError::new("quick_ask_environment_unverified", "The shell PATH is not UTF-8"))?;
        self.adapter.create_tab(&binding, &request.cwd, &tab_label(&request.prompt), kind, &request.prompt, path).await
    }
}
