use crate::lifecycle::{HerdrEndpoint, ImageIdentity, LIFECYCLE_SOCKET, LifecycleInfo};
use cockpit_core::InspectionError;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    future::Future,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    pin::Pin,
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
    time::{Instant, sleep, timeout},
};
use uuid::Uuid;

pub const OWNER_PROTOCOL: u32 = 1;
const MAX_FRAME: usize = 1024 * 1024;
fn err(code: &str, e: impl std::fmt::Display) -> InspectionError {
    InspectionError::new(code, e.to_string())
}
fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
fn dir(root: &Path) -> PathBuf {
    root.join("backend")
}
fn socket(root: &Path) -> PathBuf {
    dir(root).join(LIFECYCLE_SOCKET)
}
fn private_dir(path: &Path) -> Result<(), InspectionError> {
    fs::create_dir_all(path).map_err(|e| err("backend_unavailable", e))?;
    let m = fs::symlink_metadata(path).map_err(|e| err("backend_unavailable", e))?;
    if !m.is_dir() || m.file_type().is_symlink() || m.uid() != nix::unistd::Uid::current().as_raw()
    {
        return Err(err("unsafe_path", "backend directory is not owned"));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| err("backend_unavailable", e))
}
fn open_lock(path: &Path) -> Result<File, InspectionError> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC | nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| err("backend_unavailable", e))?;
    let m = f.metadata().map_err(|e| err("backend_unavailable", e))?;
    if !m.is_file() || m.uid() != nix::unistd::Uid::current().as_raw() {
        return Err(err("unsafe_path", "lock is not an owned regular file"));
    }
    Ok(f)
}
fn image(path: &Path) -> Result<ImageIdentity, InspectionError> {
    let m = fs::metadata(path).map_err(|e| err("not_installed", e))?;
    Ok(ImageIdentity {
        dev: m.dev(),
        ino: m.ino(),
    })
}
pub fn installed_cli_path() -> PathBuf {
    let paths = crate::installation::installed_paths(&std::env::vars().collect());
    #[cfg(target_os = "macos")]
    if let Some(bundle) = &paths.app_bundle {
        let bundled = bundle.join("Contents/MacOS/cockpit-cli");
        if bundled.is_file() {
            return bundled;
        }
    }
    crate::installation::installed_cli(&paths)
}
pub fn make_identity(root: &Path, herdr: HerdrEndpoint) -> Result<LifecycleInfo, InspectionError> {
    let env: BTreeMap<String, String> = std::env::vars().collect();
    let paths = crate::installation::installed_paths(&env);
    let (exe, image) =
        crate::installation::startup_image().map_err(|e| err("backend_unavailable", e))?;
    Ok(LifecycleInfo {
        instance_id: Uuid::new_v4(),
        pid: std::process::id(),
        started_at: now(),
        owner_protocol: OWNER_PROTOCOL,
        build_commit: option_env!("COCKPIT_BUILD_COMMIT").map(str::to_owned),
        image,
        installed: crate::installation::is_installed(&exe, &paths),
        herdr,
        state_root: fs::canonicalize(root)
            .unwrap_or_else(|_| root.to_path_buf())
            .to_string_lossy()
            .into_owned(),
        dispatcher: Some(false),
        library_sync: Some(false),
        ephemeral: None,
    })
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastExit {
    pub instance_id: Uuid,
    pub code: String,
    pub at: String,
    pub message: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Explicit,
    Signal,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackendProbe {
    Running(LifecycleInfo),
    Starting,
    Stopped {
        last_exit: Option<LastExit>,
    },
    LegacyOwner,
    Unresponsive,
    EndpointMismatch {
        running: HerdrEndpoint,
        expected: HerdrEndpoint,
    },
}
fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<(), InspectionError> {
    let parent = path
        .parent()
        .ok_or_else(|| err("backend_unavailable", "missing parent"))?;
    private_dir(parent)?;
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        Uuid::new_v4()
    ));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| err("backend_unavailable", e))?;
    let result = (|| {
        serde_json::to_writer(&mut f, value).map_err(|e| err("backend_unavailable", e))?;
        f.sync_all().map_err(|e| err("backend_unavailable", e))?;
        fs::rename(&tmp, path).map_err(|e| err("backend_unavailable", e))
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
pub fn write_last_exit(
    root: &Path,
    identity: &LifecycleInfo,
    code: &str,
    message: Option<&str>,
) -> Result<(), InspectionError> {
    atomic_json(
        &dir(root).join("last-exit.json"),
        &LastExit {
            instance_id: identity.instance_id,
            code: code.into(),
            at: now(),
            message: message.map(str::to_owned),
        },
    )
}
async fn call(root: &Path, request: Value, deadline: Duration) -> Result<Value, InspectionError> {
    timeout(deadline, async {
        let mut stream = UnixStream::connect(socket(root))
            .await
            .map_err(|e| err("backend_unavailable", e))?;
        let peer = stream.peer_cred().map_err(|e| err("backend_unavailable", e))?;
        if peer.uid() != nix::unistd::Uid::current().as_raw() {
            return Err(err("backend_unavailable", "lifecycle socket peer is not owned"));
        }
        let mut b = serde_json::to_vec(&request).map_err(|e| err("backend_unavailable", e))?;
        b.push(b'\n');
        stream
            .write_all(&b)
            .await
            .map_err(|e| err("backend_unavailable", e))?;
        let mut line = Vec::new();
        let mut reader = BufReader::new(stream).take((MAX_FRAME + 1) as u64);
        reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(|e| err("backend_unavailable", e))?;
        if line.len() > MAX_FRAME {
            return Err(err("backend_unavailable", "frame too large"));
        }
        let v: Value = serde_json::from_slice(&line).map_err(|e| err("backend_unavailable", e))?;
        if v.get("ok") == Some(&Value::Bool(false)) {
            return Err(InspectionError::new(
                v["code"].as_str().unwrap_or("backend_unavailable"),
                v["message"].as_str().unwrap_or("backend request failed"),
            ));
        }
        Ok(v)
    })
    .await
    .map_err(|_| err("backend_unresponsive", "lifecycle request timed out"))?
}

pub async fn web_call(
    root: &Path,
    expected_instance: Uuid,
    op: &str,
    payload: Value,
) -> Result<Value, InspectionError> {
    call(
        root,
        json!({"op": op, "expected_instance": expected_instance, "payload": payload}),
        Duration::from_secs(30),
    )
    .await
}

type WebHandler =
    Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Value> + Send>> + Send + Sync>;
use tokio::io::AsyncReadExt;
pub struct BackendServer {
    listener: UnixListener,
    lock: File,
    path: PathBuf,
    dev: u64,
    ino: u64,
    status: LifecycleInfo,
    web_handler: Option<WebHandler>,
}
impl BackendServer {
    pub fn try_lock(root: &Path) -> Result<Option<File>, InspectionError> {
        private_dir(&dir(root))?;
        let f = open_lock(&dir(root).join("backend.lock"))?;
        match f.try_lock_exclusive() {
            Ok(()) => Ok(Some(f)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(err("backend_unavailable", e)),
        }
    }
    pub fn bind(root: &Path, lock: File, status: LifecycleInfo) -> Result<Self, InspectionError> {
        let path = socket(root);
        match fs::symlink_metadata(&path) {
            Ok(m)
                if m.file_type().is_socket() && m.uid() == nix::unistd::Uid::current().as_raw() =>
            {
                fs::remove_file(&path).map_err(|e| err("backend_unavailable", e))?
            }
            Ok(_) => return Err(err("unsafe_path", "lifecycle socket is not owned")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(err("backend_unavailable", e)),
        }
        let listener = UnixListener::bind(&path).map_err(|e| err("backend_unavailable", e))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|e| err("backend_unavailable", e))?;
        let m = fs::symlink_metadata(&path).map_err(|e| err("backend_unavailable", e))?;
        Ok(Self {
            listener,
            lock,
            path,
            dev: m.dev(),
            ino: m.ino(),
            status,
            web_handler: None,
        })
    }
    pub fn set_web_handler(&mut self, handler: WebHandler) {
        self.web_handler = Some(handler);
    }
    pub async fn run(&mut self) -> Result<StopReason, InspectionError> {
        let peers = Arc::new(Semaphore::new(32));
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        #[cfg(unix)]
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .map_err(|e| err("backend_unavailable", e))?;
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => return Ok(StopReason::Signal),
                _ = term.recv() => return Ok(StopReason::Signal),
                Some(()) = rx.recv() => return Ok(StopReason::Explicit),
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted.map_err(|e| err("backend_unavailable", e))?;
                    if stream.peer_cred().map(|c| c.uid()).ok() != Some(nix::unistd::Uid::current().as_raw()) {
                        continue;
                    }
                    let Ok(permit) = peers.clone().try_acquire_owned() else { continue };
                    let status = self.status.clone();
                    let handler = self.web_handler.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let (r, mut w) = stream.into_split();
                        let mut line = Vec::new();
                        let mut reader = BufReader::new(r).take((MAX_FRAME + 1) as u64);
                        if reader.read_until(b'\n', &mut line).await.is_err() || line.len() > MAX_FRAME {
                            return;
                        }
                        let req: Value = match serde_json::from_slice(&line) {
                            Ok(v) => v,
                            Err(_) => return,
                        };
                        let mut stopping = false;
                        let mut frozen = false;
                        let response = match req["op"].as_str() {
                            Some("hello") | Some("status") => json!({"ok": true, "info": status}),
                            Some("stop") | Some("restart_stop") | Some("web_status") | Some("web_start") | Some("web_stop")
                                if req["expected_instance"].as_str() != Some(status.instance_id.to_string().as_str()) =>
                            {
                                json!({"ok": false, "code": "instance_changed", "message": "backend instance changed"})
                            }
                            Some("stop") => {
                                stopping = true;
                                json!({"ok": true, "stopping": true})
                            }
                            Some("restart_stop") => {
                                match handler {
                                    Some(handler) => {
                                        let mut freeze = req;
                                        freeze["op"] = json!("web_freeze");
                                        let response = handler(freeze).await;
                                        frozen = response["ok"] == Value::Bool(true);
                                        stopping = frozen;
                                        response
                                    }
                                    None => json!({"ok": false, "code": "unsupported_op", "message": "web handler is unavailable"}),
                                }
                            }
                            Some("web_status") | Some("web_start") | Some("web_stop") => {
                                match handler {
                                    Some(handler) => handler(req).await,
                                    None => json!({"ok": false, "code": "unsupported_op", "message": "web handler is unavailable"}),
                                }
                            }
                            _ => json!({"ok": false, "code": "unsupported_op", "message": "unknown lifecycle operation"}),
                        };
                        if let Ok(mut bytes) = serde_json::to_vec(&response) {
                            bytes.push(b'\n');
                            let written = timeout(Duration::from_secs(2), w.write_all(&bytes)).await;
                            if stopping && (frozen || matches!(written, Ok(Ok(())))) {
                                let _ = tx.send(()).await;
                            }
                        } else if frozen {
                            let _ = tx.send(()).await;
                        }
                    });
                }
            }
        }
    }
}
impl Drop for BackendServer {
    fn drop(&mut self) {
        if let Ok(m) = fs::symlink_metadata(&self.path) {
            if m.dev() == self.dev && m.ino() == self.ino && m.file_type().is_socket() {
                let _ = fs::remove_file(&self.path);
            }
        }
        let _ = FileExt::unlock(&self.lock);
    }
}
pub async fn probe(root: &Path, expected: &HerdrEndpoint) -> BackendProbe {
    match call(root, json!({"op":"status"}), Duration::from_secs(2)).await {
        Ok(v) => match serde_json::from_value::<LifecycleInfo>(v["info"].clone()) {
            Ok(s) if &s.herdr != expected => BackendProbe::EndpointMismatch {
                running: s.herdr,
                expected: expected.clone(),
            },
            Ok(s) => BackendProbe::Running(s),
            Err(_) => BackendProbe::Unresponsive,
        },
        Err(_) => match BackendServer::try_lock(root) {
            Ok(Some(_)) => {
                let last_exit = fs::read(dir(root).join("last-exit.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<LastExit>(&b).ok());
                if last_exit
                    .as_ref()
                    .is_some_and(|x| x.code == "legacy_owner_present")
                {
                    BackendProbe::LegacyOwner
                } else {
                    BackendProbe::Stopped { last_exit }
                }
            }
            Ok(None) if socket(root).exists() => BackendProbe::Unresponsive,
            Ok(None) => BackendProbe::Starting,
            Err(_) => BackendProbe::Unresponsive,
        },
    }
}
pub fn sanitized_environment(base: &BTreeMap<String, String>) -> Vec<(OsString, OsString)> {
    const KEYS: &[&str] = &[
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "PATH",
        "LANG",
        "TZ",
        "TMPDIR",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "XDG_BIN_HOME",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "COCKPIT_CONFIG",
        "COCKPIT_CONFIG_PATH",
        "COCKPIT_CLI_EXECUTABLE",
        "COCKPIT_CACHE_ROOT",
        "COCKPIT_WORKTREE_ROOT",
        "COCKPIT_STATE_ROOT",
        "COCKPIT_LIBRARY_ROOT",
        "COCKPIT_NOTES_ROOT",
        "COCKPIT_REPOSITORY_ROOTS",
        "COCKPIT_OMP_EXECUTABLE",
        "COCKPIT_PLAYWRIGHT_CLI",
        "COCKPIT_CHROMIUM_EXECUTABLE",
        "COCKPIT_NODE_EXECUTABLE",
        "COCKPIT_BROWSER_HELPER",
        "COCKPIT_PLAYWRIGHT_CORE",
        "COCKPIT_BROWSER_DEFAULT_URL",
        "PI_CODING_AGENT_DIR",
    ];
    base.iter()
        .filter(|(k, _)| KEYS.contains(&k.as_str()) || k.starts_with("LC_"))
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
}
fn gui_relaunch_environment(base: &BTreeMap<String, String>) -> Vec<(OsString, OsString)> {
    let mut env = sanitized_environment(base);
    const DISPLAY_KEYS: &[&str] = &[
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
    ];
    env.extend(
        base.iter()
            .filter(|(key, _)| DISPLAY_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.into(), value.into())),
    );
    env
}
pub fn spawn_detached(
    program: &Path,
    args: &[OsString],
    unit_hint: &str,
    log: &Path,
    env: &[(OsString, OsString)],
) -> Result<u32, InspectionError> {
    let parent = log
        .parent()
        .ok_or_else(|| err("backend_unavailable", "missing log parent"))?;
    private_dir(parent)?;
    let mut f = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(log)
        .map_err(|e| err("backend_unavailable", e))?;
    let m = f.metadata().map_err(|e| err("backend_unavailable", e))?;
    if !m.is_file() || m.uid() != nix::unistd::Uid::current().as_raw() {
        return Err(err("unsafe_path", "log is not owned"));
    }
    f.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|e| err("backend_unavailable", e))?;
    if m.len() > 1024 * 1024 {
        f.seek(SeekFrom::End(-1048576))
            .map_err(|e| err("backend_unavailable", e))?;
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes)
            .map_err(|e| err("backend_unavailable", e))?;
        f.set_len(0).map_err(|e| err("backend_unavailable", e))?;
        f.write_all(&bytes)
            .map_err(|e| err("backend_unavailable", e))?;
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
        let mut c = Command::new("systemd-run");
        c.arg("--user")
            .arg("--collect")
            .arg("--quiet")
            .arg(format!("--unit={unit_hint}"))
            .arg(format!(
                "--property=StandardOutput=append:{}",
                log.display()
            ))
            .arg(format!("--property=StandardError=append:{}", log.display()))
            .arg("/usr/bin/env")
            .arg("-i");
        for (k, v) in env {
            let mut x = k.clone();
            x.push("=");
            x.push(v);
            c.arg(x);
        }
        c.arg(program)
            .args(args)
            .env_clear()
            .envs(env.iter().cloned())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if c.status().is_ok_and(|s| s.success()) {
            return Ok(0);
        }
    }
    let _ = unit_hint;
    let mut c = Command::new(program);
    c.args(args)
        .env_clear()
        .envs(env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            f.try_clone().map_err(|e| err("backend_unavailable", e))?,
        ))
        .stderr(Stdio::from(f));
    unsafe {
        c.pre_exec(|| {
            if nix::libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = c.spawn().map_err(|e| err("backend_unavailable", e))?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}
pub async fn ensure(
    root: &Path,
    cli: &Path,
    endpoint: &HerdrEndpoint,
) -> Result<LifecycleInfo, InspectionError> {
    ensure_with_config(root, cli, endpoint, None).await
}
pub async fn ensure_with_config(
    root: &Path,
    cli: &Path,
    endpoint: &HerdrEndpoint,
    config: Option<&Path>,
) -> Result<LifecycleInfo, InspectionError> {
    let end = Instant::now() + Duration::from_secs(10);
    let mut spawned = false;
    loop {
        match probe(root, endpoint).await {
            BackendProbe::Running(s) => {
                let expected = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
                if Path::new(&s.state_root) != expected {
                    return Err(err("backend_config_mismatch", "backend state root differs"));
                }
                return Ok(s);
            }
            BackendProbe::EndpointMismatch { running, expected } => {
                return Err(err(
                    "backend_endpoint_mismatch",
                    format!("backend {running:?}, requested {expected:?}"),
                ));
            }
            BackendProbe::LegacyOwner if spawned && Instant::now() >= end => {
                return Err(err(
                    "backend_legacy_owner",
                    "older Cockpit owns the Browser runtime",
                ));
            }
            BackendProbe::Stopped { .. } | BackendProbe::LegacyOwner if !spawned => {
                let mut args = vec![OsString::from("service"), OsString::from("run")];
                if let Some(config) = config {
                    args.extend([OsString::from("--config"), config.as_os_str().to_owned()]);
                }
                args.extend(endpoint.to_args());
                let hash = Sha256::digest(
                    fs::canonicalize(root)
                        .unwrap_or_else(|_| root.to_path_buf())
                        .as_os_str()
                        .as_encoded_bytes(),
                );
                let id = format!("{:x}", hash);
                let mut env = sanitized_environment(&std::env::vars().collect());
                env.retain(|(k, _)| k != "COCKPIT_STATE_ROOT");
                env.push(("COCKPIT_STATE_ROOT".into(), root.as_os_str().to_owned()));
                spawn_detached(
                    cli,
                    &args,
                    &format!("cockpit-backend-{}", &id[..12]),
                    &dir(root).join("backend.log"),
                    &env,
                )?;
                spawned = true;
            }
            _ => {}
        }
        if Instant::now() >= end {
            return Err(err("backend_unresponsive", "backend did not become ready"));
        }
        sleep(Duration::from_millis(100)).await;
    }
}
pub async fn stop(root: &Path, expected: Uuid) -> Result<(), InspectionError> {
    let deadline = Duration::from_secs(30);
    call(
        root,
        json!({"op":"stop","expected_instance":expected}),
        deadline,
    )
    .await?;
    wait_for_owner_release(root, deadline).await?;
    let exit: LastExit = fs::read(dir(root).join("last-exit.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| err("backend_outcome_unknown", "backend exit receipt is unavailable"))?;
    if exit.instance_id != expected || exit.code != "explicit" {
        return Err(err("backend_outcome_unknown", "backend shutdown did not complete cleanly; inspect before retrying"));
    }
    Ok(())
}
async fn wait_for_owner_release(root: &Path, deadline: Duration) -> Result<(), InspectionError> {
    let end = Instant::now() + deadline;
    loop {
        if let Some(lock) = BackendServer::try_lock(root)? {
            drop(lock);
            return Ok(());
        }
        if Instant::now() >= end {
            return Err(err("backend_unresponsive", "backend did not stop"));
        }
        sleep(Duration::from_millis(100)).await;
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartTicket {
    pub pid: u32,
    pub started_at: String,
    pub relaunch_app: bool,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartStep {
    Lock,
    CheckInstall,
    Stop,
    Start,
    RelaunchApp,
    Done,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RestartResult {
    Running,
    Ok,
    Failed { code: String, message: String },
    Interrupted,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartRecord {
    pub pid: u32,
    pub started_at: String,
    pub relaunch_app: bool,
    pub step: RestartStep,
    pub result: RestartResult,
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<Uuid>,
}
pub fn read_restart(root: &Path) -> Result<Option<RestartRecord>, InspectionError> {
    let mut record: RestartRecord = match fs::read(dir(root).join("restart.json")) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| err("backend_unavailable", e))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(err("backend_unavailable", e)),
    };
    if matches!(record.result, RestartResult::Running) {
        let lock = open_lock(&dir(root).join("restart.lock"))?;
        if lock.try_lock_exclusive().is_ok() {
            record.result = RestartResult::Interrupted;
        }
    }
    Ok(Some(record))
}
pub async fn spawn_restart_helper(
    root: &Path,
    expected: Option<Uuid>,
    relaunch_app: bool,
    endpoint: &HerdrEndpoint,
) -> Result<RestartTicket, InspectionError> {
    spawn_restart_helper_with_config(root, expected, relaunch_app, endpoint, None).await
}
pub async fn spawn_restart_helper_with_config(
    root: &Path,
    expected: Option<Uuid>,
    relaunch_app: bool,
    endpoint: &HerdrEndpoint,
    config: Option<&Path>,
) -> Result<RestartTicket, InspectionError> {
    let cli = installed_cli_path();
    if !cli.is_file() {
        return Err(err("not_installed", "installed CLI is unavailable"));
    }
    if read_restart(root)?.is_some_and(|r| matches!(r.result, RestartResult::Running)) {
        return Err(err("update_busy", "restart already running"));
    }
    let token = Uuid::new_v4();
    let mut args = vec!["service".into(), "restart-helper".into()];
    if let Some(id) = expected {
        args.extend([OsString::from("--expected-instance"), id.to_string().into()]);
    }
    if relaunch_app {
        args.push("--relaunch-app".into());
    }
    if let Some(config) = config {
        args.extend([OsString::from("--config"), config.as_os_str().to_owned()]);
    }
    args.extend(endpoint.to_args());
    let base = std::env::vars().collect();
    let mut env = if relaunch_app {
        gui_relaunch_environment(&base)
    } else {
        sanitized_environment(&base)
    };
    env.retain(|(k, _)| {
        k != "COCKPIT_STATE_ROOT" && (config.is_none() || k != "COCKPIT_CONFIG_PATH")
    });
    env.push(("COCKPIT_STATE_ROOT".into(), root.as_os_str().to_owned()));
    if let Some(config) = config {
        env.push(("COCKPIT_CONFIG_PATH".into(), config.as_os_str().to_owned()));
    }
    env.push(("COCKPIT_RESTART_TOKEN".into(), token.to_string().into()));
    let pid = spawn_detached(
        &cli,
        &args,
        &format!("cockpit-restart-{}", Uuid::new_v4()),
        &dir(root).join("restart.log"),
        &env,
    )?;
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(r) = read_restart(root)? {
            if r.token == Some(token) && (pid == 0 || pid == r.pid) {
                return Ok(RestartTicket {
                    pid: r.pid,
                    started_at: r.started_at,
                    relaunch_app: r.relaunch_app,
                });
            }
        }
        if Instant::now() >= end {
            return Err(err(
                "backend_unavailable",
                "restart helper did not acknowledge",
            ));
        }
        sleep(Duration::from_millis(50)).await;
    }
}
pub async fn run_restart_helper(
    root: &Path,
    expected: Option<Uuid>,
    endpoint: &HerdrEndpoint,
    relaunch_app: bool,
) -> Result<(), InspectionError> {
    let paths = crate::installation::installed_paths(&std::env::vars().collect());
    run_restart_helper_with_paths(root, expected, endpoint, relaunch_app, &paths).await
}

async fn run_restart_helper_with_paths(
    root: &Path,
    expected: Option<Uuid>,
    endpoint: &HerdrEndpoint,
    relaunch_app: bool,
    paths: &crate::installation::InstalledPaths,
) -> Result<(), InspectionError> {
    private_dir(&dir(root))?;
    let helper_lock = open_lock(&dir(root).join("restart.lock"))?;
    helper_lock
        .try_lock_exclusive()
        .map_err(|_| err("update_busy", "restart already running"))?;
    let mut record = RestartRecord {
        pid: std::process::id(),
        started_at: now(),
        relaunch_app,
        step: RestartStep::Lock,
        result: RestartResult::Running,
        finished_at: None,
        token: std::env::var("COCKPIT_RESTART_TOKEN")
            .ok()
            .and_then(|s| Uuid::parse_str(&s).ok()),
    };
    atomic_json(&dir(root).join("restart.json"), &record)?;
    let result = async {
        let lock = crate::installation::acquire_update_lock(paths, Duration::from_secs(30))?;
        record.step = RestartStep::CheckInstall;
        atomic_json(&dir(root).join("restart.json"), &record)?;
        if paths
            .data_home
            .join(".cockpit-install.pending.json")
            .exists()
        {
            return Err(err("install_incomplete", "installation journal exists"));
        }
        let cli = installed_cli_path();
        if !cli.is_file() {
            return Err(err("not_installed", "installed CLI is unavailable"));
        }
        let instance = match probe(root, endpoint).await {
            BackendProbe::Running(s) => {
                if expected.is_some_and(|id| id != s.instance_id) {
                    return Err(err("instance_changed", "backend instance changed"));
                }
                s.instance_id
            }
            BackendProbe::EndpointMismatch { .. } => {
                return Err(err("backend_endpoint_mismatch", "Herdr endpoint differs"));
            }
            _ => return Err(err("backend_unavailable", "backend is not running")),
        };
        record.step = RestartStep::Stop;
        atomic_json(&dir(root).join("restart.json"), &record)?;
        let web_status = call(
            root,
            json!({"op":"restart_stop","expected_instance":instance}),
            Duration::from_secs(30),
        )
        .await
        .map_err(|e| {
            if matches!(e.code.as_str(), "backend_unavailable" | "backend_unresponsive") {
                err("restart_outcome_unknown", format!("restart stop outcome is unknown: {}; inspect the backend before retrying", e.message))
            } else {
                e
            }
        })?;
        if web_status.get("ok") != Some(&Value::Bool(true)) {
            return Err(err("restart_outcome_unknown", "invalid restart stop response; inspect the backend before retrying"));
        }
        wait_for_owner_release(root, Duration::from_secs(30)).await.map_err(|e| {
            err("restart_outcome_unknown", format!("backend was frozen but owner release was not confirmed: {}; inspect the backend before retrying", e.message))
        })?;
        let exit: LastExit = fs::read(dir(root).join("last-exit.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| err("restart_outcome_unknown", "backend exit receipt is unavailable"))?;
        if exit.instance_id != instance || exit.code != "explicit" {
            return Err(err("restart_outcome_unknown", "backend shutdown did not complete cleanly; inspect before retrying"));
        }
        let web_config = match web_status.get("info") {
            Some(Value::Null) => None,
            Some(info) => Some(info.get("config").filter(|config| config.is_object()).cloned()
                .ok_or_else(|| err("restart_outcome_unknown", "restart stop snapshot has no config; backend has stopped"))?),
            None => return Err(err("restart_outcome_unknown", "restart stop response has no snapshot; backend has stopped")),
        };
        record.step = RestartStep::Start;
        atomic_json(&dir(root).join("restart.json"), &record)?;
        let expected_image = image(&cli)?;
        let config = std::env::var_os("COCKPIT_CONFIG_PATH").map(PathBuf::from);
        let identity = ensure_with_config(root, &cli, endpoint, config.as_deref()).await?;
        if identity.image != expected_image {
            return Err(err(
                "successor_not_installed",
                "backend image differs from installed CLI",
            ));
        }
        if let Some(config) = web_config {
            web_call(root, identity.instance_id, "web_start", config).await?;
        }
        if relaunch_app {
            record.step = RestartStep::RelaunchApp;
            atomic_json(&dir(root).join("restart.json"), &record)?;
            let env = gui_relaunch_environment(&std::env::vars().collect());
            #[cfg(target_os = "linux")]
            {
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default();
                let launcher = std::env::var_os("XDG_BIN_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".local/bin"))
                    .join("cockpit");
                spawn_detached(
                    &launcher,
                    &[],
                    &format!("cockpit-app-{}", Uuid::new_v4()),
                    &dir(root).join("restart.log"),
                    &env,
                )?;
            }
            #[cfg(target_os = "macos")]
            {
                let app = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                    .join("Applications/Cockpit.app");
                spawn_detached(
                    Path::new("/usr/bin/open"),
                    &["-n".into(), app.into_os_string()],
                    "cockpit-app",
                    &dir(root).join("restart.log"),
                    &env,
                )?;
            }
        }
        drop(lock);
        Ok(())
    }
    .await;
    if result.is_ok() {
        record.step = RestartStep::Done;
    }
    record.finished_at = Some(now());
    record.result = match &result {
        Ok(()) => RestartResult::Ok,
        Err(e) => RestartResult::Failed {
            code: e.code.clone(),
            message: e.message.clone(),
        },
    };
    atomic_json(&dir(root).join("restart.json"), &record)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_excludes_worker_and_endpoint_authority() {
        let env = BTreeMap::from([
            ("HOME".into(), "/home/test".into()),
            ("LC_TIME".into(), "C".into()),
            ("COCKPIT_STATE_ROOT".into(), "/tmp/state".into()),
            ("COCKPIT_HERDR_SESSION".into(), "worker".into()),
            ("COCKPIT_HERDR_SOCKET".into(), "/tmp/worker.sock".into()),
            ("COCKPIT_RUN_ID".into(), "run".into()),
            (
                "COCKPIT_JOB_CREDENTIAL_FILE".into(),
                "/tmp/credential".into(),
            ),
        ]);
        let actual = sanitized_environment(&env);
        assert!(actual.iter().any(|(key, _)| key == "COCKPIT_STATE_ROOT"));
        assert!(actual.iter().any(|(key, _)| key == "LC_TIME"));
        assert!(!actual.iter().any(|(key, _)| key == "COCKPIT_RUN_ID"
            || key == "COCKPIT_JOB_CREDENTIAL_FILE"
            || key == "COCKPIT_HERDR_SESSION"
            || key == "COCKPIT_HERDR_SOCKET"));
    }

    #[test]
    fn gui_relaunch_keeps_display_but_not_worker_authority() {
        let base = BTreeMap::from([
            ("WAYLAND_DISPLAY".into(), "wayland-1".into()),
            ("DISPLAY".into(), ":1".into()),
            ("XAUTHORITY".into(), "/tmp/xauth".into()),
            ("COCKPIT_RUN_ID".into(), "worker".into()),
            (
                "COCKPIT_JOB_CREDENTIAL_FILE".into(),
                "/tmp/credential".into(),
            ),
            ("HERDR_SESSION".into(), "worker".into()),
        ]);
        let gui = gui_relaunch_environment(&base);
        assert!(
            gui.iter()
                .any(|(key, value)| key == "WAYLAND_DISPLAY" && value == "wayland-1")
        );
        assert!(
            gui.iter()
                .any(|(key, value)| key == "DISPLAY" && value == ":1")
        );
        assert!(
            gui.iter()
                .any(|(key, value)| key == "XAUTHORITY" && value == "/tmp/xauth")
        );
        assert!(!gui.iter().any(|(key, _)| key == "COCKPIT_RUN_ID"
            || key == "COCKPIT_JOB_CREDENTIAL_FILE"
            || key == "HERDR_SESSION"));
        assert!(
            !sanitized_environment(&base)
                .iter()
                .any(|(key, _)| key == "DISPLAY" || key == "WAYLAND_DISPLAY")
        );
    }

    #[tokio::test]
    async fn pending_install_refuses_restart_without_stopping_backend() {
        let root = std::env::temp_dir().join(format!("cockpit-restart-journal-{}", Uuid::new_v4()));
        let state = root.join("state");
        let data = root.join("data");
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join(".cockpit-install.pending.json"), b"{}").unwrap();
        let paths = crate::installation::InstalledPaths {
            data_home: data,
            bin_home: root.join("bin"),
            app_bundle: None,
        };
        let endpoint = HerdrEndpoint {
            executable: "herdr".into(),
            session: None,
            socket: None,
        };
        let info = make_identity(&state, endpoint.clone()).unwrap();
        let instance = info.instance_id;
        let lock = BackendServer::try_lock(&state).unwrap().unwrap();
        let mut server = BackendServer::bind(&state, lock, info).unwrap();
        let task = tokio::spawn(async move { server.run().await });
        let error = run_restart_helper_with_paths(&state, Some(instance), &endpoint, false, &paths)
            .await
            .unwrap_err();
        assert_eq!(error.code, "install_incomplete");
        assert!(
            matches!(probe(&state, &endpoint).await, BackendProbe::Running(info) if info.instance_id == instance)
        );
        call(
            &state,
            json!({"op":"stop","expected_instance":instance}),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(task.await.unwrap().unwrap(), StopReason::Explicit);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn lifecycle_status_and_fenced_stop_ignore_owner_protocol_version() {
        for protocol in [OWNER_PROTOCOL, OWNER_PROTOCOL + 1] {
            let root = std::env::temp_dir().join(format!("cockpit-lifecycle-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            let endpoint = HerdrEndpoint {
                executable: "herdr".into(),
                session: None,
                socket: None,
            };
            let mut info = make_identity(&root, endpoint.clone()).unwrap();
            info.owner_protocol = protocol;
            let instance = info.instance_id;
            let lock = BackendServer::try_lock(&root).unwrap().unwrap();
            let mut server = BackendServer::bind(&root, lock, info).unwrap();
            let task = tokio::spawn(async move { server.run().await });

            match probe(&root, &endpoint).await {
                BackendProbe::Running(info) => assert_eq!(info.owner_protocol, protocol),
                other => panic!("expected running backend, got {other:?}"),
            }
            let error = call(
                &root,
                json!({"op":"stop","expected_instance":Uuid::new_v4()}),
                Duration::from_secs(2),
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, "instance_changed");
            assert!(matches!(
                probe(&root, &endpoint).await,
                BackendProbe::Running(_)
            ));
            call(
                &root,
                json!({"op":"stop","expected_instance":instance}),
                Duration::from_secs(2),
            )
            .await
            .unwrap();
            assert_eq!(task.await.unwrap().unwrap(), StopReason::Explicit);
            fs::remove_dir_all(root).unwrap();
        }
    }
}
