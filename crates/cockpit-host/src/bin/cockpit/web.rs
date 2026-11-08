use super::*;
use cockpit_host::{
    backend,
    lifecycle::HerdrEndpoint,
    server::{ServerConfig, serve as serve_foreground, serve_managed},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::oneshot,
    time::{Instant, sleep, timeout},
};

#[derive(Debug, Args)]
pub struct WebArgs {
    #[command(flatten)]
    herdr: HerdrArgs,
    #[command(flatten)]
    project: ProjectArgs,
    #[command(subcommand)]
    command: WebCommand,
}
#[derive(Debug, Subcommand)]
enum WebCommand {
    Start {
        #[arg(long)]
        bind: SocketAddr,
        #[arg(long)]
        static_dir: Option<PathBuf>,
    },
    Status {
        #[arg(long)]
        json: bool,
    },
    Stop {
        #[arg(long)]
        operator: bool,
    },
    Restart {
        #[arg(long)]
        operator: bool,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ManagedConfig {
    pub root: PathBuf,
    pub bind: SocketAddr,
    pub static_dir: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub herdr: HerdrEndpoint,
}
#[derive(Clone, Serialize, Deserialize)]
struct WebInfo {
    instance_id: uuid::Uuid,
    config: ManagedConfig,
}

fn root_dir(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("web");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let metadata = fs::symlink_metadata(&dir).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.uid() != nix::unistd::Uid::current().as_raw() {
        return Err("unsafe web state directory".into());
    }
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    Ok(dir)
}
fn lock(root: &Path) -> Result<Option<std::fs::File>, String> {
    let path = root_dir(root)?.join("web.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.uid() != nix::unistd::Uid::current().as_raw() {
        return Err("unsafe web lock".into());
    }
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(file)),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
async fn call(root: &Path, request: serde_json::Value) -> Result<serde_json::Value, String> {
    timeout(Duration::from_secs(3), async {
        let mut stream = UnixStream::connect(root.join("web/web.sock"))
            .await
            .map_err(|e| e.to_string())?;
        if stream.peer_cred().map_err(|e| e.to_string())?.uid()
            != nix::unistd::Uid::current().as_raw()
        {
            return Err("web socket owner mismatch".into());
        }
        let mut bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        stream.write_all(&bytes).await.map_err(|e| e.to_string())?;
        let mut line = Vec::new();
        BufReader::new(stream)
            .take(1_048_577)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|e| e.to_string())?;
        if line.len() > 1_048_576 {
            return Err("web control frame too large".into());
        }
        serde_json::from_slice(&line).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "web control timed out".to_string())?
}
async fn status(root: &Path) -> Result<Option<WebInfo>, String> {
    match call(root, serde_json::json!({"op":"status"})).await {
        Ok(value) => serde_json::from_value(value)
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(error) => {
            if lock(root)?.is_some() {
                Ok(None)
            } else {
                Err(format!("managed gateway unavailable: {error}"))
            }
        }
    }
}
async fn start(config: ManagedConfig) -> Result<WebInfo, String> {
    if let Some(info) = status(&config.root).await? {
        return Ok(info);
    }
    let mut args: Vec<OsString> = vec![
        "serve".into(),
        "--managed".into(),
        "--bind".into(),
        config.bind.to_string().into(),
    ];
    if let Some(path) = &config.static_dir {
        args.extend(["--static-dir".into(), path.as_os_str().to_owned()]);
    }
    if let Some(path) = &config.config {
        args.extend(["--config".into(), path.as_os_str().to_owned()]);
    }
    args.extend(config.herdr.to_args());
    let mut env = backend::sanitized_environment(&std::env::vars().collect());
    env.retain(|(key, _)| key != "COCKPIT_STATE_ROOT");
    env.push((
        "COCKPIT_STATE_ROOT".into(),
        config.root.as_os_str().to_owned(),
    ));
    backend::spawn_detached(
        &service_cli::cli_path()?,
        &args,
        &format!("cockpit-web-{}", uuid::Uuid::new_v4()),
        &config.root.join("web/web.log"),
        &env,
    )
    .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(Some(info)) = status(&config.root).await {
            return Ok(info);
        }
        if Instant::now() >= deadline {
            return Err("managed gateway did not become ready".into());
        }
        sleep(Duration::from_millis(100)).await;
    }
}
async fn stop(root: &Path, info: &WebInfo) -> Result<(), String> {
    let reply = call(
        root,
        serde_json::json!({"op":"stop", "instance_id":info.instance_id}),
    )
    .await?;
    if reply["ok"] != true {
        return Err("managed gateway identity changed".into());
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if lock(root)?.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("managed gateway did not stop".into());
        }
        sleep(Duration::from_millis(100)).await;
    }
}
pub async fn run(args: WebArgs) -> Result<(), String> {
    let projects = project_configuration(&args.project)?;
    let root = PathBuf::from(projects.state_root);
    match args.command {
        WebCommand::Start { bind, static_dir } => {
            cockpit_host::server::validate_bind(bind).map_err(|e| e.to_string())?;
            let info = start(ManagedConfig {
                root,
                bind,
                static_dir,
                config: args.project.config,
                herdr: service_cli::endpoint(&args.herdr)?,
            })
            .await?;
            println!(
                "{}",
                serde_json::to_string(&info).map_err(|e| e.to_string())?
            );
        }
        WebCommand::Status { json } => {
            let value = serde_json::json!({"managed":status(&root).await?, "notice":"unmanaged gateways are not tracked"});
            println!(
                "{}",
                if json {
                    serde_json::to_string_pretty(&value)
                } else {
                    serde_json::to_string(&value)
                }
                .map_err(|e| e.to_string())?
            );
        }
        WebCommand::Stop { operator } => {
            service_cli::operator_guard(operator)?;
            if let Some(info) = status(&root).await? {
                stop(&root, &info).await?;
            }
        }
        WebCommand::Restart { operator } => {
            service_cli::operator_guard(operator)?;
            let info = status(&root)
                .await?
                .ok_or("no managed web gateway is running")?;
            stop(&root, &info).await?;
            let info = start(info.config).await?;
            println!(
                "{}",
                serde_json::to_string(&info).map_err(|e| e.to_string())?
            );
        }
    }
    Ok(())
}

pub(super) async fn serve(
    config: ServerConfig,
    managed: bool,
    details: Option<ManagedConfig>,
) -> Result<(), String> {
    if !managed {
        return serve_foreground(config).await.map_err(|e| e.to_string());
    }
    let mut details = details.ok_or("managed test-mode gateway is unsupported")?;
    let lock = lock(&details.root)?.ok_or("managed gateway already running")?;
    let path = root_dir(&details.root)?.join("web.sock");
    match fs::symlink_metadata(&path) {
        Ok(meta)
            if meta.file_type().is_socket()
                && meta.uid() == nix::unistd::Uid::current().as_raw() =>
        {
            fs::remove_file(&path).map_err(|e| e.to_string())?
        }
        Ok(_) => return Err("unsafe web socket path".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.to_string()),
    }
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    let socket_meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let (ready_tx, ready_rx) = oneshot::channel();
    let server = serve_managed(
        config,
        async move {
            let _ = stop_rx.await;
        },
        ready_tx,
    );
    tokio::pin!(server);
    let address = tokio::select! { result = &mut server => return result.map_err(|e| e.to_string()), address = ready_rx => address.map_err(|e| e.to_string())? };
    details.bind = address;
    let info = WebInfo {
        instance_id: uuid::Uuid::new_v4(),
        config: details,
    };
    let mut stop_tx = Some(stop_tx);
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| e.to_string())?;
    loop {
        tokio::select! {
            result = &mut server => {
                let exit = serde_json::json!({"instance_id":info.instance_id, "code":if result.is_ok() { "stopped" } else { "failed" }});
                let tmp = info.config.root.join("web").join(format!(".last-exit-{}", uuid::Uuid::new_v4()));
                if let Ok(mut file) = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp) {
                    use std::io::Write;
                    if serde_json::to_writer(&mut file, &exit).is_ok() && file.flush().is_ok() {
                        let _ = fs::rename(&tmp, info.config.root.join("web/last-exit.json"));
                    }
                }
                cleanup(&path, &socket_meta); drop(lock); return result.map_err(|e| e.to_string());
            }
            _ = tokio::signal::ctrl_c() => { if let Some(tx) = stop_tx.take() { let _ = tx.send(()); } }
            _ = term.recv() => { if let Some(tx) = stop_tx.take() { let _ = tx.send(()); } }
            connection = listener.accept(), if stop_tx.is_some() => {
                let (mut stream, _) = connection.map_err(|e| e.to_string())?;
                if stream.peer_cred().map(|c| c.uid()).ok() != Some(nix::unistd::Uid::current().as_raw()) { continue; }
                let mut line = Vec::new();
                if timeout(Duration::from_secs(2), BufReader::new(&mut stream).take(1_048_577).read_until(b'\n', &mut line)).await.is_err() || line.len() > 1_048_576 { continue; }
                let req: serde_json::Value = match serde_json::from_slice(&line) { Ok(v) => v, Err(_) => continue };
                let stopping = req["op"] == "stop" && req["instance_id"] == info.instance_id.to_string();
                let response = if req["op"] == "status" || req["op"] == "hello" { serde_json::to_value(&info).map_err(|e| e.to_string())? } else { serde_json::json!({"ok":stopping}) };
                let mut bytes = serde_json::to_vec(&response).map_err(|e| e.to_string())?; bytes.push(b'\n');
                let _ = stream.write_all(&bytes).await;
                if stopping { if let Some(tx) = stop_tx.take() { let _ = tx.send(()); } }
            }
        }
    }
}
fn cleanup(path: &Path, original: &fs::Metadata) {
    if let Ok(current) = fs::symlink_metadata(path) {
        if current.file_type().is_socket()
            && current.dev() == original.dev()
            && current.ino() == original.ino()
        {
            let _ = fs::remove_file(path);
        }
    }
}
