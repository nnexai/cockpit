use cap_fs_ext::DirExt;
use cap_std::fs::MetadataExt;
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    net::{TcpStream, UnixStream},
    task::JoinSet,
};

use super::cdp::cdp_record;
use super::receipts::{
    BrowserReceipt, MAX_ASSOCIATIONS, MAX_RECEIPT_BYTES, ReceiptIntent, ReceiptState, path_string,
    read_regular,
};
use super::{BrowserService, cleanup};
use crate::{InspectionError, config::BrowserConfiguration, process::run_bounded_command};

const CLI_OUTPUT_LIMIT: usize = 64 * 1024;
const CLI_TIMEOUT: Duration = Duration::from_secs(15);
const REQUIRED_PLAYWRIGHT_CLI_VERSION: &str = "0.1.5";
pub(super) const SOCKET_TIMEOUT: Duration = Duration::from_secs(2);

impl BrowserService {
    /// Recorded Cockpit keys authorize only their derived session name and cwd.
    /// Confirm daemon process exit and browser closure; uncertainty fails reset.
    pub(crate) async fn stop_previous_sessions(&self) -> Result<(), InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let io_error = |error: std::io::Error| {
            InspectionError::new("browser_startup_stop_failed", error.to_string())
        };
        let root =
            crate::project_store::open_dir_nofollow_absolute(&self.root).map_err(io_error)?;
        let device = root.dir_metadata().map_err(io_error)?.dev();
        let mut keys = std::collections::BTreeSet::new();
        let mut inspected = 0usize;
        for name in ["tab-associations", "associations"] {
            let metadata = match root.symlink_metadata(name) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error(error)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            if metadata.dev() != device {
                return Err(InspectionError::new(
                    "browser_startup_stop_failed",
                    "recorded browser directory crosses filesystem device",
                ));
            }
            let dir = root.open_dir_nofollow(name).map_err(io_error)?;
            let opened = dir.dir_metadata().map_err(io_error)?;
            if opened.dev() != device || opened.ino() != metadata.ino() {
                return Err(InspectionError::new(
                    "browser_startup_stop_failed",
                    "recorded browser directory identity changed",
                ));
            }
            for entry in dir.entries().map_err(io_error)? {
                inspected += 1;
                if inspected > MAX_ASSOCIATIONS * 2 {
                    return Err(InspectionError::new(
                        "browser_startup_stop_failed",
                        "too many recorded browser entries",
                    ));
                }
                let filename = entry.map_err(io_error)?.file_name();
                let path = Path::new(&filename);
                if path.extension().and_then(|name| name.to_str()) != Some("json") {
                    continue;
                }
                let Some(key) = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .filter(|key| cleanup::valid_key(key))
                else {
                    continue;
                };
                let metadata = dir.symlink_metadata(&filename).map_err(io_error)?;
                if !metadata.file_type().is_symlink() && metadata.dev() != device {
                    return Err(InspectionError::new(
                        "browser_startup_stop_failed",
                        "recorded browser entry crosses filesystem device",
                    ));
                }
                if metadata.is_file() && !metadata.file_type().is_symlink() {
                    keys.insert(key.to_owned());
                }
            }
        }
        let mut tasks = JoinSet::new();
        let mut first_error = None;
        for key in keys {
            let service = self.clone();
            tasks.spawn(async move { service.stop_previous_session(&key).await });
            if tasks.len() >= 4 {
                match tasks.join_next().await.expect("pending browser stop") {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        first_error.get_or_insert(error);
                    }
                    Err(error) => {
                        first_error.get_or_insert(InspectionError::new(
                            "browser_startup_stop_failed",
                            error.to_string(),
                        ));
                    }
                }
            }
        }
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    first_error.get_or_insert(error);
                }
                Err(error) => {
                    first_error.get_or_insert(InspectionError::new(
                        "browser_startup_stop_failed",
                        error.to_string(),
                    ));
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn stop_previous_session(&self, key: &str) -> Result<(), InspectionError> {
        let workspace = self.root.join("workspaces").join(key);
        let session = format!("cockpit-{key}");
        let profile = self.root.join("profiles").join(key);
        let daemon = previous_daemon(&workspace, &profile, &session).await?;
        if daemon.is_none() {
            if previous_browser_closed(&profile).await? {
                return Ok(());
            }
            return Err(InspectionError::new(
                "browser_startup_stop_failed",
                format!("{session}: browser is live without a reachable managed daemon"),
            ));
        }
        let daemon = daemon.expect("reachable daemon");
        let _cwd =
            crate::project_store::open_dir_nofollow_absolute(&workspace).map_err(|error| {
                InspectionError::new(
                    "browser_startup_stop_failed",
                    format!("{}: {error}", workspace.display()),
                )
            })?;
        let browser_dir =
            crate::project_store::open_dir_nofollow_absolute(&self.root).map_err(|error| {
                InspectionError::new("browser_startup_stop_failed", error.to_string())
            })?;
        if _cwd
            .dir_metadata()
            .map_err(|error| {
                InspectionError::new("browser_startup_stop_failed", error.to_string())
            })?
            .dev()
            != browser_dir
                .dir_metadata()
                .map_err(|error| {
                    InspectionError::new("browser_startup_stop_failed", error.to_string())
                })?
                .dev()
        {
            return Err(InspectionError::new(
                "browser_startup_stop_failed",
                format!(
                    "{}: workspace crosses filesystem device",
                    workspace.display()
                ),
            ));
        }
        let executable = resolve_executable(&self.configuration.playwright_cli, "Playwright CLI")?;
        let mut command = tokio::process::Command::new(executable);
        command
            .current_dir(&workspace)
            .args([format!("-s={session}"), "close".into()]);
        let result = run_bounded_command(
            command,
            CLI_OUTPUT_LIMIT,
            CLI_OUTPUT_LIMIT,
            CLI_TIMEOUT,
            "Playwright CLI",
        )
        .await;
        for _ in 0..5 {
            if !process_incarnation_running(daemon.pid, daemon.start)?
                && previous_daemon(&workspace, &profile, &session)
                    .await?
                    .is_none()
                && previous_browser_closed(&profile).await?
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let detail = match result {
            Err(error) => error.message,
            Ok(output) => format!(
                "Playwright close exited with {}; managed daemon or browser remains live",
                output.status
            ),
        };
        Err(InspectionError::new(
            "browser_startup_stop_failed",
            format!("{session}: stop was not confirmed; {detail}"),
        ))
    }

    async fn run_cli_output(
        &self,
        receipt: &BrowserReceipt,
        args: &[String],
    ) -> Result<std::process::Output, InspectionError> {
        let executable = resolve_executable(&self.configuration.playwright_cli, "Playwright CLI")?;
        let mut command = tokio::process::Command::new(executable);
        command.current_dir(&receipt.working_directory).args(args);
        run_bounded_command(
            command,
            CLI_OUTPUT_LIMIT,
            CLI_OUTPUT_LIMIT,
            CLI_TIMEOUT,
            "Playwright CLI",
        )
        .await
    }

    pub(super) async fn run_cli(
        &self,
        receipt: &BrowserReceipt,
        args: &[String],
    ) -> Result<String, InspectionError> {
        let output = self.run_cli_output(receipt, args).await?;
        if !output.status.success() {
            if args.get(1).is_some_and(|action| action == "open")
                && String::from_utf8_lossy(&output.stderr)
                    .contains("Error: Failed to launch the browser process.")
            {
                return Err(InspectionError::new(
                    "browser_launch_failed",
                    "Chromium could not start; check the configured executable and desktop display",
                ));
            }
            return Err(InspectionError::new(
                "browser_cli_failed",
                format!("Playwright CLI exited with status {}", output.status),
            ));
        }
        let text = String::from_utf8(output.stdout).map_err(|_| {
            InspectionError::new(
                "browser_cli_invalid_output",
                "Playwright CLI output was not UTF-8",
            )
        })?;
        // CLI 0.1.5 reports tool failures in stdout while exiting successfully.
        if text.lines().any(|line| line == "### Error") {
            let detail = cli_error_detail(&text);
            let message = detail.map_or_else(
                || {
                    "Playwright reported a browser operation failure; inspect the browser before another URL action"
                        .to_owned()
                },
                |detail| {
                    format!(
                        "Playwright reported a browser operation failure: {detail}"
                    )
                },
            );
            return Err(InspectionError::new("browser_action_failed", message));
        }
        Ok(text)
    }

    pub(super) async fn ensure_cli_version(&self) -> Result<(), InspectionError> {
        let executable = resolve_executable(&self.configuration.playwright_cli, "Playwright CLI")?;
        let mut command = tokio::process::Command::new(executable);
        command.arg("--version");
        let output =
            run_bounded_command(command, 1024, 1024, CLI_TIMEOUT, "Playwright CLI").await?;
        let version = String::from_utf8_lossy(&output.stdout);
        let reported_version = version.lines().next().unwrap_or("").trim();
        if !output.status.success() || !compatible_playwright_cli_version(reported_version) {
            return Err(InspectionError::new(
                "browser_cli_incompatible",
                format!(
                    "Playwright CLI {REQUIRED_PLAYWRIGHT_CLI_VERSION} or a later 0.1.x bugfix release is required (found {reported_version})",
                ),
            ));
        }
        Ok(())
    }
    pub(super) async fn inspect_live(
        &self,
        receipt: &BrowserReceipt,
    ) -> Result<String, InspectionError> {
        let daemon = daemon_receipt(receipt)?;
        let stream = tokio::time::timeout(SOCKET_TIMEOUT, UnixStream::connect(&daemon.socket_path))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "browser_daemon_unavailable",
                    "Playwright daemon connection timed out",
                )
            })?
            .map_err(|error| {
                InspectionError::new(
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) {
                        "browser_daemon_closed"
                    } else {
                        "browser_daemon_unavailable"
                    },
                    "Playwright daemon is not reachable",
                )
            })?;
        let credentials = stream.peer_cred().map_err(|_| {
            InspectionError::new(
                "browser_daemon_unavailable",
                "cannot inspect Playwright daemon peer",
            )
        })?;
        if credentials.uid() != current_uid() {
            return Err(InspectionError::new(
                "browser_daemon_foreign",
                "Playwright daemon is owned by another user",
            ));
        }
        let pid = credentials.pid().ok_or_else(|| {
            InspectionError::new(
                "browser_daemon_unavailable",
                "Playwright daemon peer has no PID",
            )
        })?;
        let start = process_start_identity(pid).ok_or_else(|| {
            InspectionError::new(
                "browser_daemon_unavailable",
                "cannot verify Playwright daemon process generation",
            )
        })?;
        let incarnation = format!("pid={pid}:start={start}:receipt={}", daemon.hash);
        if receipt.incarnation.is_none()
            && !matches!(
                receipt.intent,
                ReceiptIntent::Launch | ReceiptIntent::LaunchNavigation
            )
        {
            return Err(InspectionError::new(
                "browser_unowned_daemon",
                "A browser was started outside this association's launch; refusing to adopt it",
            ));
        }
        if receipt.incarnation.is_some() && receipt.incarnation.as_deref() != Some(&incarnation) {
            return Err(InspectionError::new(
                "browser_receipt_replaced",
                "Playwright daemon receipt or process generation changed",
            ));
        }

        let probe = self
            .run_cli_output(
                receipt,
                &[
                    format!("-s={}", receipt.playwright_session),
                    "cookie-get".into(),
                    "__cockpit_liveness_probe__".into(),
                ],
            )
            .await?;
        let stdout = String::from_utf8_lossy(&probe.stdout);
        let stderr = String::from_utf8_lossy(&probe.stderr);
        if !probe.status.success() {
            if cli_reports_browser_closed(&stdout) || cli_reports_browser_closed(&stderr) {
                return Err(InspectionError::new(
                    "browser_daemon_closed",
                    "Playwright browser session reports that Chromium is closed",
                ));
            }
            return Err(InspectionError::new(
                "browser_cli_failed",
                format!("Playwright CLI exited with status {}", probe.status),
            ));
        }
        if let Some(detail) = cli_error_detail(&stdout) {
            if cli_reports_browser_closed(&detail) {
                return Err(InspectionError::new(
                    "browser_daemon_closed",
                    "Playwright browser session reports that Chromium is closed",
                ));
            }
            return Err(InspectionError::new(
                "browser_action_failed",
                format!("Playwright reported a browser operation failure: {detail}"),
            ));
        }
        Ok(incarnation)
    }
}

struct PreviousDaemon {
    pid: i32,
    start: u64,
}

/// Persistent Playwright sessions retain their .session configuration after
/// shutdown. Prove liveness through the matching daemon socket, not that file.
async fn previous_daemon(
    workspace: &Path,
    profile: &Path,
    session: &str,
) -> Result<Option<PreviousDaemon>, InspectionError> {
    let failure = |message| InspectionError::new("browser_startup_stop_failed", message);
    let path = daemon_session_path(workspace, session)?;
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failure(error.to_string())),
        Ok(_) => {}
    }
    let bytes = read_regular(&path, MAX_RECEIPT_BYTES)?;
    let config: Value =
        serde_json::from_slice(&bytes).map_err(|error| failure(error.to_string()))?;
    let workspace = workspace
        .to_str()
        .ok_or_else(|| failure(format!("{session}: workspace path is not UTF-8")))?;
    let profile = profile
        .to_str()
        .ok_or_else(|| failure(format!("{session}: profile path is not UTF-8")))?;
    if config.get("workspaceDir").and_then(Value::as_str) != Some(workspace)
        || config
            .pointer("/browser/userDataDir")
            .and_then(Value::as_str)
            != Some(profile)
    {
        return Err(failure(format!(
            "{session}: daemon configuration does not match derived workspace/profile"
        )));
    }
    let socket = config
        .get("socketPath")
        .and_then(Value::as_str)
        .filter(|socket| Path::new(socket).is_absolute())
        .ok_or_else(|| {
            failure(format!(
                "{session}: daemon configuration lacks an absolute socket path"
            ))
        })?;
    let stream = match tokio::time::timeout(SOCKET_TIMEOUT, UnixStream::connect(socket)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None);
        }
        Ok(Err(error)) => return Err(failure(format!("{session}: daemon probe failed: {error}"))),
        Err(_) => return Err(failure(format!("{session}: daemon probe timed out"))),
    };
    let peer = stream
        .peer_cred()
        .map_err(|error| failure(error.to_string()))?;
    if peer.uid() != current_uid() {
        return Err(failure(format!(
            "{session}: daemon belongs to another user"
        )));
    }
    let pid = peer
        .pid()
        .ok_or_else(|| failure(format!("{session}: daemon has no process identity")))?;
    let start = process_start_identity(pid).ok_or_else(|| {
        failure(format!(
            "{session}: daemon process generation is unavailable"
        ))
    })?;
    Ok(Some(PreviousDaemon { pid, start }))
}

async fn previous_browser_closed(profile: &Path) -> Result<bool, InspectionError> {
    match fs::symlink_metadata(profile.join("DevToolsActivePort")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => {
            return Err(InspectionError::new(
                "browser_startup_stop_failed",
                error.to_string(),
            ));
        }
        Ok(_) => {}
    }
    let (port, _) = cdp_record(profile)?;
    match tokio::time::timeout(SOCKET_TIMEOUT, TcpStream::connect(("127.0.0.1", port))).await {
        Ok(Ok(_)) => Ok(false),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => Ok(true),
        Ok(Err(error)) => Err(InspectionError::new(
            "browser_startup_stop_failed",
            format!("browser closure probe failed: {error}"),
        )),
        Err(_) => Err(InspectionError::new(
            "browser_startup_stop_failed",
            "browser closure probe timed out",
        )),
    }
}

#[cfg(target_os = "linux")]
fn process_incarnation_running(pid: i32, start: u64) -> Result<bool, InspectionError> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(InspectionError::new(
                "browser_startup_stop_failed",
                error.to_string(),
            ));
        }
    };
    let fields = stat
        .rfind(')')
        .and_then(|close| stat.get(close + 2..))
        .ok_or_else(|| {
            InspectionError::new(
                "browser_startup_stop_failed",
                "daemon process status is invalid",
            )
        })?;
    let mut fields = fields.split_whitespace();
    let state = fields.next();
    let current_start = fields
        .nth(18)
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or_else(|| {
            InspectionError::new(
                "browser_startup_stop_failed",
                "daemon process generation is invalid",
            )
        })?;
    // Zombies have completed process exit; PID reuse is not the old daemon.
    Ok(current_start == start && !matches!(state, Some("Z" | "X")))
}

#[cfg(not(target_os = "linux"))]
fn process_incarnation_running(pid: i32, start: u64) -> Result<bool, InspectionError> {
    if let Some(current) = process_start_identity(pid) {
        return Ok(current == start);
    }
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
        Err(nix::errno::Errno::ESRCH) => Ok(false),
        _ => Err(InspectionError::new(
            "browser_startup_stop_failed",
            "daemon process exit could not be verified",
        )),
    }
}

struct DaemonReceipt {
    socket_path: PathBuf,
    hash: String,
}

pub(super) fn daemon_session_path(
    workspace: &Path,
    session: &str,
) -> Result<PathBuf, InspectionError> {
    let daemon_dir = if let Some(path) =
        env::var_os("PLAYWRIGHT_DAEMON_SESSION_DIR").filter(|path| !path.is_empty())
    {
        PathBuf::from(path)
    } else {
        #[cfg(target_os = "macos")]
        let cache = env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"));
        #[cfg(not(target_os = "macos"))]
        let cache = env::var_os("XDG_CACHE_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")));
        let cache = cache.ok_or_else(|| {
            InspectionError::new(
                "browser_daemon_unavailable",
                "cache directory is unavailable",
            )
        })?;
        cache.join("ms-playwright/daemon")
    };
    let hash = format!("{:x}", Sha1::digest(path_string(workspace)?.as_bytes()));
    Ok(daemon_dir
        .join(&hash[..16])
        .join(format!("{session}.session")))
}

fn daemon_receipt(receipt: &BrowserReceipt) -> Result<DaemonReceipt, InspectionError> {
    let workspace = Path::new(&receipt.working_directory);
    let path = daemon_session_path(workspace, &receipt.playwright_session)?;
    if fs::symlink_metadata(&path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return Err(InspectionError::new(
            "browser_daemon_closed",
            "Playwright daemon receipt is absent",
        ));
    }
    let bytes = read_regular(&path, MAX_RECEIPT_BYTES)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        InspectionError::new(
            "browser_receipt_corrupt",
            "Playwright daemon receipt is invalid",
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        InspectionError::new(
            "browser_receipt_corrupt",
            "Playwright daemon receipt is not an object",
        )
    })?;
    let string = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                InspectionError::new(
                    "browser_receipt_corrupt",
                    format!("Playwright daemon receipt lacks {key}"),
                )
            })
    };
    if string("workspaceDir")? != receipt.working_directory {
        return Err(InspectionError::new(
            "browser_receipt_mismatch",
            "Playwright daemon workspace does not match association",
        ));
    }
    let socket_path = PathBuf::from(string("socketPath")?);
    let user_data = object
        .get("browser")
        .and_then(Value::as_object)
        .and_then(|browser| browser.get("userDataDir"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_receipt_corrupt",
                "Playwright daemon receipt lacks profile",
            )
        })?;
    if user_data != receipt.profile_path {
        return Err(InspectionError::new(
            "browser_receipt_mismatch",
            "Playwright daemon profile does not match association",
        ));
    }
    let executable = object
        .get("browser")
        .and_then(Value::as_object)
        .and_then(|browser| browser.get("launchOptions"))
        .and_then(Value::as_object)
        .and_then(|options| options.get("executablePath"))
        .and_then(Value::as_str);
    let cleanup_only = matches!(
        receipt.state,
        ReceiptState::CleanupPending | ReceiptState::CleanupFailed
    ) && receipt.intent == ReceiptIntent::None
        && receipt.incarnation.is_none();
    if !cleanup_only
        && receipt_config_executable(receipt)?
            .is_some_and(|expected| Some(expected.as_str()) != executable)
    {
        return Err(InspectionError::new(
            "browser_receipt_mismatch",
            "Playwright daemon executable does not match association config",
        ));
    }
    Ok(DaemonReceipt {
        socket_path,
        hash: format!("{:x}", Sha256::digest(&bytes)),
    })
}

fn receipt_config_executable(receipt: &BrowserReceipt) -> Result<Option<String>, InspectionError> {
    let config: Value = serde_json::from_slice(&read_regular(
        Path::new(&receipt.config_path),
        MAX_RECEIPT_BYTES,
    )?)
    .map_err(|_| {
        InspectionError::new(
            "browser_config_corrupt",
            "browser launch configuration is invalid",
        )
    })?;
    match config.pointer("/browser/launchOptions/executablePath") {
        None => Ok(None),
        Some(Value::String(path)) => Ok(Some(path.clone())),
        Some(_) => Err(InspectionError::new(
            "browser_config_corrupt",
            "browser executable override is not a path",
        )),
    }
}

pub(super) fn launch_configuration(
    configuration: &BrowserConfiguration,
    hook_path: &Path,
) -> Result<Value, InspectionError> {
    let mut config = serde_json::json!({"browser": {"launchOptions": {}}});
    if let Some(path) = &configuration.chromium_executable {
        config["browser"]["launchOptions"]["executablePath"] = Value::String(path_string(
            &resolve_executable(path, "Chromium executable")?,
        )?);
    }
    // Chromium allocates the port and writes it to the association profile.
    // This avoids a global fixed-port collision and never exposes CDP beyond
    // the local owner helper.
    config["browser"]["launchOptions"]["args"] = serde_json::json!([
        "--remote-debugging-address=127.0.0.1",
        "--remote-debugging-port=0",
        "--force-dark-mode"
    ]);
    config["browser"]["initPage"] = serde_json::json!([path_string(hook_path)?]);
    Ok(config)
}

pub(super) fn locate_playwright_core(cli: &Path) -> Option<PathBuf> {
    let mut directory = cli.parent()?;
    for _ in 0..8 {
        let candidates = [
            directory.join("playwright-core"),
            directory.join("node_modules/playwright-core"),
            directory.join("lib/node_modules/playwright-core"),
            directory.join("../lib/node_modules/playwright-core"),
        ];
        for candidate in candidates {
            if candidate.join("package.json").is_file() {
                return candidate.canonicalize().ok();
            }
        }
        directory = directory.parent()?;
    }
    None
}

pub(super) fn compatible_playwright_cli_version(version: &str) -> bool {
    let mut parts = version.split('.');
    let Some(major) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return false;
    };
    let Some(minor) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return false;
    };
    let Some(patch) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return false;
    };
    major == 0 && minor == 1 && patch >= 5 && parts.next().is_none()
}

pub(super) fn resolve_regular_file(path: &Path, label: &str) -> Result<PathBuf, InspectionError> {
    let setting = match label {
        "Browser helper" => "COCKPIT_BROWSER_HELPER or [browser].browser_helper",
        _ => "the browser configuration",
    };
    if !path.is_absolute() {
        return Err(InspectionError::new(
            "browser_helper_invalid",
            format!("{label} must be an absolute path; set {setting} to browser-helper.mjs"),
        ));
    }
    let resolved = path.canonicalize().map_err(|_| {
        InspectionError::new(
            "browser_helper_missing",
            format!("{label} is missing; set {setting} to an installed helper module"),
        )
    })?;
    if !fs::metadata(&resolved).is_ok_and(|metadata| metadata.is_file()) {
        return Err(InspectionError::new(
            "browser_helper_invalid",
            format!("{label} must be a regular file; set {setting} to browser-helper.mjs"),
        ));
    }
    Ok(resolved)
}

pub(super) fn resolve_playwright_core(path: &Path) -> Result<PathBuf, InspectionError> {
    if !path.is_absolute() {
        return Err(InspectionError::new(
            "browser_core_invalid",
            "Configured Playwright-core path must be absolute; set COCKPIT_PLAYWRIGHT_CORE or \
             [browser].playwright_core to its installed package directory",
        ));
    }
    let resolved = path.canonicalize().map_err(|_| {
        InspectionError::new(
            "browser_core_missing",
            "Playwright-core package is missing; set COCKPIT_PLAYWRIGHT_CORE or \
             [browser].playwright_core to its installed package directory",
        )
    })?;
    if !resolved.is_dir() {
        return Err(InspectionError::new(
            "browser_core_invalid",
            "Configured Playwright-core path is not a package directory; set \
             COCKPIT_PLAYWRIGHT_CORE or [browser].playwright_core to playwright-core",
        ));
    }
    let package_path = resolved.join("package.json");
    let bytes = fs::read(&package_path).map_err(|_| {
        InspectionError::new(
            "browser_core_missing",
            "Configured Playwright-core package.json is missing; point \
             COCKPIT_PLAYWRIGHT_CORE or [browser].playwright_core at the package directory",
        )
    })?;
    if bytes.len() > 64 * 1024 {
        return Err(InspectionError::new(
            "browser_core_invalid",
            "Playwright-core package metadata exceeds the bounded limit",
        ));
    }
    let package: Value = serde_json::from_slice(&bytes).map_err(|_| {
        InspectionError::new(
            "browser_core_invalid",
            "Playwright-core package metadata is invalid JSON; reinstall the paired package",
        )
    })?;
    if package.get("name").and_then(Value::as_str) != Some("playwright-core") {
        return Err(InspectionError::new(
            "browser_core_invalid",
            "Configured package is not playwright-core; select the package paired with the CLI",
        ));
    }
    let entry = package
        .get("module")
        .or_else(|| package.get("main"))
        .and_then(Value::as_str)
        .unwrap_or("index.js");
    let entry_path = resolved.join(entry);
    if !entry_path.starts_with(&resolved)
        || !entry_path.is_file()
        || !resolved.join("lib").join("utilsBundle.js").is_file()
    {
        return Err(InspectionError::new(
            "browser_core_invalid",
            "Playwright-core package entry points are incomplete; reinstall the paired package",
        ));
    }
    Ok(resolved)
}

pub(super) fn resolve_executable(path: &Path, label: &str) -> Result<PathBuf, InspectionError> {
    let setting = match label {
        "Playwright CLI" => "COCKPIT_PLAYWRIGHT_CLI or [browser].playwright_cli",
        "Node runtime" => "COCKPIT_NODE_EXECUTABLE or [browser].node_executable",
        "Chromium executable" => "COCKPIT_CHROMIUM_EXECUTABLE or [browser].chromium_executable",
        _ => "the browser configuration",
    };
    let code = match label {
        "Playwright CLI" => "browser_cli_missing",
        "Node runtime" => "browser_node_missing",
        "Chromium executable" => "browser_executable_missing",
        _ => "browser_tool_missing",
    };
    let candidate = if path.components().count() == 1 {
        env::var_os("PATH")
            .and_then(|paths| {
                let mut unusable = None;
                for directory in env::split_paths(&paths) {
                    let candidate = directory.join(path);
                    if is_executable(&candidate) {
                        return Some(candidate);
                    }
                    if unusable.is_none() && fs::metadata(&candidate).is_ok() {
                        unusable = Some(candidate);
                    }
                }
                unusable
            })
            .ok_or_else(|| {
                InspectionError::new(
                    code,
                    format!("{label} is not installed; set {setting} to its absolute path"),
                )
            })?
    } else {
        path.to_owned()
    };
    let metadata = fs::metadata(&candidate).map_err(|_| {
        InspectionError::new(
            code,
            format!("{label} is missing; set {setting} to its absolute path"),
        )
    })?;
    if !metadata.is_file() {
        return Err(InspectionError::new(
            "browser_tool_invalid",
            format!("{label} must be a regular file; set {setting} to an executable"),
        ));
    }
    if !is_executable(&candidate) {
        return Err(InspectionError::new(
            "browser_tool_permission",
            format!("{label} is not executable; grant execute permission or set {setting}"),
        ));
    }
    candidate.canonicalize().map_err(|_| {
        InspectionError::new(
            "browser_tool_unavailable",
            format!("Cannot resolve {label}; verify permissions and set {setting}"),
        )
    })
}

pub(super) fn may_launch_after_inspection_failure(error: &InspectionError) -> bool {
    error.code == "browser_daemon_closed"
}

fn cli_error_detail(text: &str) -> Option<String> {
    let mut lines = text.lines();
    lines
        .position(|line| line == "### Error")
        .and_then(|_| lines.map(str::trim).find(|line| !line.is_empty()))
        .map(|line| {
            line.strip_prefix("Error: ")
                .unwrap_or(line)
                .chars()
                .take(512)
                .collect()
        })
}

fn cli_reports_browser_closed(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("target page, context or browser has been closed")
        || text.contains("browser context was closed")
        || text.contains("browser has been closed")
        || (text.contains("browser '") && text.contains(" is not open"))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

pub(super) fn tab_index(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let index = line.trim().strip_prefix("- ")?.split_once(": (current)")?.0;
        (!index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| index.to_owned())
    })
}

pub(super) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\\"'\\\"'"))
}
#[cfg(unix)]
pub(super) fn current_uid() -> u32 {
    nix::unistd::Uid::current().as_raw()
}
#[cfg(not(unix))]
pub(super) fn current_uid() -> u32 {
    0
}
#[cfg(target_os = "linux")]
pub(super) fn process_start_identity(pid: i32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    stat.get(close + 2..)?
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}
#[cfg(target_os = "macos")]
pub(super) fn process_start_identity(pid: i32) -> Option<u64> {
    use libproc::{bsd_info::BSDInfo, proc_pid::pidinfo};

    let info = pidinfo::<BSDInfo>(pid, 0).ok()?;

    if info.pbi_start_tvusec >= 1_000_000 {
        return None;
    }
    info.pbi_start_tvsec
        .checked_shl(20)
        .and_then(|value| value.checked_add(info.pbi_start_tvusec))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn process_start_identity(_pid: i32) -> Option<u64> {
    None
}
