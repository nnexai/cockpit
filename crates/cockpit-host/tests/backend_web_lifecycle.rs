#![cfg(unix)]

use cockpit_host::{
    backend::{self, BackendProbe},
    lifecycle::HerdrEndpoint,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{sleep, timeout},
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use uuid::Uuid;

struct Fixture {
    root: PathBuf,
    state: PathBuf,
    assets: PathBuf,
    endpoint: HerdrEndpoint,
    child: Option<Child>,
    detached: bool,
}

impl Fixture {
    fn new() -> Self {
        // Keep the Unix-socket prefix short while respecting the test temp root.
        let root = std::env::temp_dir().join(format!("cw-{}", Uuid::new_v4().simple()));
        std::fs::create_dir(&root).unwrap();
        let assets = root.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(root.join("runtime")).unwrap();
        std::fs::create_dir_all(root.join("repositories")).unwrap();
        std::fs::write(assets.join("index.html"), "web lifecycle fixture").unwrap();
        let endpoint = HerdrEndpoint {
            executable: root.join("no-herdr").to_string_lossy().into_owned(),
            session: None,
            socket: None,
        };
        Self {
            state: root.join("owner"),
            root,
            assets,
            endpoint,
            child: None,
            detached: false,
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cockpit"));
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("XDG_RUNTIME_DIR", self.root.join("runtime"))
            .env("COCKPIT_STATE_ROOT", &self.state)
            .env("COCKPIT_REPOSITORY_ROOTS", self.root.join("repositories"))
            .env("COCKPIT_HERDR_EXECUTABLE", &self.endpoint.executable)
            .env("COCKPIT_CLI_EXECUTABLE", env!("CARGO_BIN_EXE_cockpit"));
        cmd
    }

    fn spawn(&mut self, args: &[&str]) {
        let stderr = std::fs::File::create(self.root.join("service.stderr")).unwrap();
        self.child = Some(
            self.command()
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::from(stderr))
                .spawn()
                .unwrap(),
        );
    }

    async fn ready(&mut self) -> Uuid {
        timeout(Duration::from_secs(30), async {
            loop {
                if let BackendProbe::Running(info) =
                    backend::probe(&self.state, &self.endpoint).await
                {
                    return info.instance_id;
                }
                if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                    panic!(
                        "backend exited before becoming ready ({status}): {}",
                        self.logs()
                    );
                }
                sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("backend readiness timeout: {}", self.logs()))
    }

    fn logs(&self) -> String {
        format!(
            "service stderr: {}\ndetached backend log: {}",
            std::fs::read_to_string(self.root.join("service.stderr")).unwrap_or_default(),
            std::fs::read_to_string(self.state.join("backend/backend.log")).unwrap_or_default()
        )
    }

    fn config(&self) -> Value {
        json!({"bind":"127.0.0.1:0", "static_dir":self.assets, "config_file":null})
    }

    async fn stop_owner(&mut self, id: Uuid) {
        backend::stop(&self.state, id).await.unwrap();
        self.detached = false;
        if let Some(mut child) = self.child.take() {
            let status = tokio::task::spawn_blocking(move || child.wait().unwrap())
                .await
                .unwrap();
            assert!(status.success());
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if self.detached {
            let state = self.state.clone();
            let endpoint = self.endpoint.clone();
            let stopped = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async {
                    // A failed launcher can still have a backend in startup. Keep the
                    // fixture until its exact owner is reachable and fenced to stop.
                    for _ in 0..200 {
                        match backend::probe(&state, &endpoint).await {
                            BackendProbe::Running(info) => {
                                return backend::stop(&state, info.instance_id).await.is_ok();
                            }
                            BackendProbe::Stopped { last_exit: Some(_) } => return true,
                            _ => sleep(Duration::from_millis(50)).await,
                        }
                    }
                    false
                })
            })
            .join()
            .unwrap_or(false);
            if !stopped {
                eprintln!(
                    "backend cleanup unconfirmed; preserving fixture {}",
                    self.root.display()
                );
                return;
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn fetch(bind: &str) -> String {
    timeout(Duration::from_secs(5), async {
        let mut stream = TcpStream::connect(bind).await.unwrap();
        stream
            .write_all(
                format!("GET / HTTP/1.1\r\nHost: {bind}\r\nConnection: close\r\n\r\n").as_bytes(),
            )
            .await
            .unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        String::from_utf8(bytes).unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn service_first_web_lifecycle_preserves_owner_and_fences_stale_instances() {
    let mut f = Fixture::new();
    f.spawn(&["service", "run"]);
    let owner = f.ready().await;
    assert!(
        backend::web_call(&f.state, owner, "web_status", Value::Null)
            .await
            .unwrap()["info"]
            .is_null()
    );
    let config = f.config();
    let first = backend::web_call(&f.state, owner, "web_start", config.clone())
        .await
        .unwrap()["info"]
        .clone();
    let web_id = first["instance_id"].as_str().unwrap();
    let bind = first["config"]["bind"].as_str().unwrap();
    assert!(fetch(bind).await.contains("web lifecycle fixture"));
    assert_eq!(
        backend::web_call(&f.state, owner, "web_start", config.clone())
            .await
            .unwrap()["info"],
        first
    );
    let wrong_owner = Uuid::new_v4();
    assert_eq!(
        backend::web_call(&f.state, wrong_owner, "web_status", Value::Null)
            .await
            .unwrap_err()
            .code,
        "instance_changed"
    );
    assert_eq!(
        backend::web_call(&f.state, wrong_owner, "web_start", config.clone())
            .await
            .unwrap_err()
            .code,
        "instance_changed"
    );
    let mut different_config = config.clone();
    different_config["bind"] = json!("127.0.0.1:12345");
    assert!(
        backend::web_call(&f.state, owner, "web_start", different_config)
            .await
            .is_err()
    );
    assert_eq!(
        backend::web_call(
            &f.state,
            wrong_owner,
            "web_stop",
            json!({"web_instance":web_id})
        )
        .await
        .unwrap_err()
        .code,
        "instance_changed"
    );
    assert!(
        backend::web_call(
            &f.state,
            owner,
            "web_stop",
            json!({"web_instance":Uuid::new_v4()})
        )
        .await
        .is_err()
    );
    assert!(fetch(bind).await.contains("web lifecycle fixture"));
    let mut request = format!("ws://{bind}/api/v1/widgets/events")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", format!("http://{bind}").parse().unwrap());
    let socket = TcpStream::connect(bind).await.unwrap();
    let (mut websocket, response) = tokio_tungstenite::client_async(request, socket)
        .await
        .unwrap();
    assert_eq!(response.status(), 101);
    let snapshot = timeout(Duration::from_secs(5), websocket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(snapshot) = snapshot else {
        panic!("expected widget snapshot")
    };
    let snapshot: Value = serde_json::from_str(&snapshot).unwrap();
    assert_eq!(snapshot["type"], "snapshot");
    backend::web_call(&f.state, owner, "web_stop", json!({"web_instance":web_id}))
        .await
        .unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            match websocket.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .expect("web_stop left an upgraded widget stream alive");
    assert!(
        backend::web_call(&f.state, owner, "web_status", Value::Null)
            .await
            .unwrap()["info"]
            .is_null()
    );
    assert!(TcpStream::connect(bind).await.is_err());
    assert!(
        matches!(backend::probe(&f.state, &f.endpoint).await, BackendProbe::Running(info) if info.instance_id == owner)
    );
    let second = backend::web_call(&f.state, owner, "web_start", config)
        .await
        .unwrap()["info"]
        .clone();
    assert_ne!(second["instance_id"], first["instance_id"]);
    assert!(
        backend::web_call(&f.state, owner, "web_stop", json!({"web_instance":web_id}))
            .await
            .is_err()
    );
    assert!(
        fetch(second["config"]["bind"].as_str().unwrap())
            .await
            .contains("web lifecycle fixture")
    );
    let second_bind = second["config"]["bind"].as_str().unwrap().to_owned();
    f.stop_owner(owner).await;
    assert!(TcpStream::connect(second_bind).await.is_err());
}

#[tokio::test]
async fn serve_starts_detached_owner_and_managed_web_listener() {
    let mut f = Fixture::new();
    let assets = f.assets.to_str().unwrap().to_owned();
    f.detached = true;
    let output = f
        .command()
        .args(["serve", "--bind", "127.0.0.1:0", "--static-dir", &assets])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "serve failed: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        f.logs()
    );
    let owner = match backend::probe(&f.state, &f.endpoint).await {
        BackendProbe::Running(info) => info.instance_id,
        other => panic!("serve returned without a running owner: {other:?}"),
    };
    let info = backend::web_call(&f.state, owner, "web_status", Value::Null)
        .await
        .unwrap()["info"]
        .clone();
    let bind = info["config"]["bind"].as_str().unwrap().to_owned();
    assert!(fetch(&bind).await.contains("web lifecycle fixture"));
    assert_eq!(
        backend::web_call(&f.state, owner, "web_start", f.config())
            .await
            .unwrap()["info"],
        info
    );
    backend::web_call(
        &f.state,
        owner,
        "web_stop",
        json!({"web_instance":info["instance_id"]}),
    )
    .await
    .unwrap();
    assert!(TcpStream::connect(&bind).await.is_err());
    assert!(
        matches!(backend::probe(&f.state, &f.endpoint).await, BackendProbe::Running(current) if current.instance_id == owner)
    );
    f.stop_owner(owner).await;
}

#[tokio::test]
async fn restart_stop_fences_stale_owner_snapshots_web_and_shuts_down_cleanly() {
    let mut f = Fixture::new();
    f.spawn(&["service", "run"]);
    let owner = f.ready().await;
    let config = f.config();
    let web = backend::web_call(&f.state, owner, "web_start", config.clone())
        .await
        .unwrap()["info"]
        .clone();
    let web_id = web["instance_id"].as_str().unwrap().to_owned();
    let bind = web["config"]["bind"].as_str().unwrap().to_owned();

    let error = backend::web_call(&f.state, Uuid::new_v4(), "restart_stop", Value::Null)
        .await
        .unwrap_err();
    assert_eq!(error.code, "instance_changed");
    assert_eq!(
        backend::web_call(&f.state, owner, "web_status", Value::Null)
            .await
            .unwrap()["info"],
        web
    );
    assert!(fetch(&bind).await.contains("web lifecycle fixture"));

    let snapshot = backend::web_call(&f.state, owner, "restart_stop", Value::Null)
        .await
        .unwrap();
    assert_eq!(snapshot["info"], web);
    // Once freeze has acknowledged the snapshot, neither operation may succeed:
    // the server may reject it while frozen or close its socket during shutdown.
    let (start, stop) = tokio::join!(
        backend::web_call(&f.state, owner, "web_start", config),
        backend::web_call(&f.state, owner, "web_stop", json!({"web_instance":web_id})),
    );
    assert!(start.is_err(), "web_start succeeded after restart snapshot");
    assert!(stop.is_err(), "web_stop succeeded after restart snapshot");

    let status = timeout(Duration::from_secs(30), async {
        loop {
            if let Some(status) = f.child.as_mut().unwrap().try_wait().unwrap() {
                break status;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("frozen backend did not exit");
    assert!(status.success(), "backend exit failed: {}", f.logs());
    f.child.take();
    let exit: backend::LastExit =
        serde_json::from_slice(&std::fs::read(f.state.join("backend/last-exit.json")).unwrap())
            .unwrap();
    assert_eq!(exit.instance_id, owner);
    assert_eq!(exit.code, "explicit");
    assert!(TcpStream::connect(bind).await.is_err());
}

#[tokio::test]
async fn web_start_expected_instance_attaches_without_starting_an_owner() {
    let mut f = Fixture::new();
    f.spawn(&["service", "run"]);
    let owner = f.ready().await;
    let owner_arg = owner.to_string();
    let stale_arg = Uuid::new_v4().to_string();
    let assets = f.assets.to_str().unwrap().to_owned();

    let run_start = |expected: &str| {
        f.command()
            .args([
                "web",
                "start",
                "--bind",
                "127.0.0.1:0",
                "--static-dir",
                &assets,
                "--expected-instance",
                expected,
                "--require-stopped",
            ])
            .output()
            .unwrap()
    };
    let stale = run_start(&stale_arg);
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("instance changed"));
    assert!(
        backend::web_call(&f.state, owner, "web_status", Value::Null)
            .await
            .unwrap()["info"]
            .is_null()
    );

    let attached = run_start(&owner_arg);
    assert!(
        attached.status.success(),
        "{}",
        String::from_utf8_lossy(&attached.stderr)
    );
    let info: Value = serde_json::from_slice(&attached.stdout).unwrap();
    let bind = info["config"]["bind"].as_str().unwrap();
    let web_id = info["instance_id"].as_str().unwrap().to_owned();
    let duplicate = run_start(&owner_arg);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("already running"));
    assert_eq!(
        backend::web_call(&f.state, owner, "web_status", Value::Null)
            .await
            .unwrap()["info"],
        info
    );
    assert!(fetch(bind).await.contains("web lifecycle fixture"));
    assert!(matches!(
        backend::probe(&f.state, &f.endpoint).await,
        BackendProbe::Running(info) if info.instance_id == owner
    ));

    for operation in ["stop", "restart"] {
        let stale = f
            .command()
            .args(["web", operation, "--expected-instance", &stale_arg])
            .output()
            .unwrap();
        assert!(!stale.status.success());
        assert!(String::from_utf8_lossy(&stale.stderr).contains("instance changed"));
        assert_eq!(
            backend::web_call(&f.state, owner, "web_status", Value::Null)
                .await
                .unwrap()["info"],
            info
        );
        assert!(fetch(bind).await.contains("web lifecycle fixture"));
    }
    let stopped = f
        .command()
        .args([
            "web",
            "stop",
            "--expected-instance",
            &owner_arg,
            "--expected-web-instance",
            &web_id,
        ])
        .output()
        .unwrap();
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    assert!(
        backend::web_call(&f.state, owner, "web_status", Value::Null)
            .await
            .unwrap()["info"]
            .is_null()
    );
    let replacement = run_start(&owner_arg);
    assert!(
        replacement.status.success(),
        "{}",
        String::from_utf8_lossy(&replacement.stderr)
    );
    let replacement: Value = serde_json::from_slice(&replacement.stdout).unwrap();
    let replacement_id = replacement["instance_id"].as_str().unwrap();
    assert_ne!(replacement_id, web_id);
    for operation in ["stop", "restart"] {
        let stale = f
            .command()
            .args([
                "web",
                operation,
                "--expected-instance",
                &owner_arg,
                "--expected-web-instance",
                &web_id,
            ])
            .output()
            .unwrap();
        assert!(!stale.status.success());
        assert!(String::from_utf8_lossy(&stale.stderr).contains("web listener instance changed"));
        assert_eq!(
            backend::web_call(&f.state, owner, "web_status", Value::Null)
                .await
                .unwrap()["info"],
            replacement
        );
    }
    let stopped = f
        .command()
        .args([
            "web",
            "stop",
            "--expected-instance",
            &owner_arg,
            "--expected-web-instance",
            replacement_id,
        ])
        .output()
        .unwrap();
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    for operation in ["stop", "restart"] {
        let missing = f
            .command()
            .args([
                "web",
                operation,
                "--expected-instance",
                &owner_arg,
                "--expected-web-instance",
                replacement_id,
            ])
            .output()
            .unwrap();
        assert!(!missing.status.success());
    }
    assert!(matches!(
        backend::probe(&f.state, &f.endpoint).await,
        BackendProbe::Running(info) if info.instance_id == owner
    ));

    f.stop_owner(owner).await;
    let marker = f.state.join("preserved-marker");
    std::fs::write(&marker, "preserved").unwrap();
    let missing = f
        .command()
        .args([
            "web",
            "start",
            "--bind",
            "127.0.0.1:0",
            "--static-dir",
            &assets,
            "--expected-instance",
            &owner_arg,
        ])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    for operation in ["stop", "restart"] {
        let missing = f
            .command()
            .args(["web", operation, "--expected-instance", &owner_arg])
            .output()
            .unwrap();
        assert!(!missing.status.success());
    }
    sleep(Duration::from_millis(300)).await;
    assert!(matches!(
        backend::probe(&f.state, &f.endpoint).await,
        BackendProbe::Stopped { .. }
    ));
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "preserved");
}
