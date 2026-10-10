use std::{path::PathBuf, sync::Arc};

use cockpit_core::orchestration::{
    Actor, AgentCaller, NativeAgentKind, OrchestrationService, herdr::OrchestrationHerdr,
};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_protocol::{
    orchestration::{
        AgentKind, MessageKind, OperatorOrigin, OrchestrationAction, OrchestrationActionResult,
        OrchestrationMutationRequest,
    },
    projects::ProjectConfiguration,
};

use super::super::context::Context;

// All authority in these tests comes through the real Unix-socket adapter.
// Gates select read phases, not permanent RPC-count or cadence assertions.
pub(super) struct ReadGate {
    reached: tokio::sync::oneshot::Sender<()>,
    response: tokio::sync::oneshot::Receiver<Option<serde_json::Value>>,
}

pub(super) struct SocketFixture {
    pub(super) root: PathBuf,
    pub(super) configuration: ProjectConfiguration,
    pub(super) context: Arc<Context>,
    pub(super) payload: Arc<parking_lot::Mutex<serde_json::Value>>,
    pub(super) gates: Arc<parking_lot::Mutex<std::collections::VecDeque<Option<ReadGate>>>>,
    pub(super) server: tokio::task::JoinHandle<()>,
}

pub(super) fn socket_payload() -> serde_json::Value {
    let mut payload: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../cockpit-herdr/tests/fixtures/session-snapshot.json"
    ))
    .unwrap();
    let snapshot = &mut payload["result"]["snapshot"];
    snapshot["boot_id"] = serde_json::json!("boot-one");
    snapshot["panes"][0]["agent"] = serde_json::json!("omp");
    snapshot["agents"][0]["agent"] = serde_json::json!("omp");
    snapshot["agents"][0]["agent_session"] =
        serde_json::json!({"kind": "id", "value": "main-native"});
    payload["result"].clone()
}

impl SocketFixture {
    pub(super) async fn new() -> Self {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let root = std::env::temp_dir().join(format!("ck-host-wait-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let socket = root.join("herdr.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let payload = Arc::new(parking_lot::Mutex::new(socket_payload()));
        let gates = Arc::new(parking_lot::Mutex::new(std::collections::VecDeque::<
            Option<ReadGate>,
        >::new()));
        let server_payload = payload.clone();
        let server_gates = gates.clone();
        let server = tokio::spawn(async move {
            let mut handlers = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (stream, _) = connection.unwrap();
                        let payload = server_payload.clone();
                        let gates = server_gates.clone();
                        handlers.spawn(async move {
                            let mut reader = BufReader::new(stream);
                            let mut line = String::new();
                            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                                return;
                            }
                            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                            let result = if request["method"] == "ping" {
                                serde_json::json!({"type": "pong", "version": "0.9.0", "protocol": 22})
                            } else {
                                assert_eq!(request["method"], "session.snapshot");
                                let gate = gates.lock().pop_front().flatten();
                                if let Some(gate) = gate {
                                    let _ = gate.reached.send(());
                                    match gate.response.await {
                                        Ok(Some(result)) => result,
                                        _ => return,
                                    }
                                } else {
                                    payload.lock().clone()
                                }
                            };
                            let response = format!("{}\n", serde_json::json!({
                                "id": request["id"], "result": result,
                            }));
                            // Cancellation deliberately closes some gated sockets.
                            let _ = reader.into_inner().write_all(response.as_bytes()).await;
                        });
                    }
                    _ = handlers.join_next(), if !handlers.is_empty() => {}
                }
            }
        });
        let configuration = ProjectConfiguration {
            repository_roots: vec![],
            branch_template: "test/{task}".into(),
            checkout_template: "{task}".into(),
            limits: cockpit_protocol::projects::ProjectLimits {
                catalog_depth: 1,
                catalog_entries: 1,
                git_timeout_ms: 1000,
                git_output_bytes: 1024,
                operation_timeout_ms: 1000,
                context_preview_bytes: 1024,
                context_preview_lines: 10,
                context_directory_entries: 10,
                context_tree_depth: 1,
                library_folder_files: 10,
                library_folder_bytes: 1024,
                library_file_bytes: 1024,
                library_space_pages: 10,
                library_attachment_bytes: 1024,
                library_item_attachment_bytes: 1024,
                library_max_items: 10,
            },
            ..ProjectConfiguration::for_tests(&root)
        };
        let adapter = Arc::new(HerdrCliAdapter::new(
            cockpit_herdr::HerdrCliConfig::from_options(None, Some("fixture".into()), Some(socket))
                .unwrap(),
        ));
        let evidence = adapter
            .source_adapter()
            .source_pane_evidence("fixture", "pane-a")
            .await
            .unwrap();
        let runtime = adapter.runtime("fixture").await.unwrap();
        let pane = &runtime.panes[0];
        let actor = Actor::Agent(AgentCaller {
            endpoint_identity: evidence.endpoint_identity.clone(),
            session_id: "fixture".into(),
            workspace_id: evidence.workspace_id.clone(),
            tab_id: evidence.tab_id.clone(),
            pane_id: evidence.pane_id.clone(),
            boot_id: runtime.boot_id.clone(),
            terminal_id: pane.terminal_id.clone(),
            native_session_id: pane.native_session_id.clone(),
            actual_agent_kind: pane.agent_kind.clone().map(NativeAgentKind::from),
            env_run: None,
            omp_session_id: Some("main-native".into()),
            main_omp_session_id: None,
            agent_kind: Some(AgentKind::Main),
            subagent_id: None,
            process: None,
        });
        let context = Arc::new(Context {
            service: OrchestrationService::open(&configuration).unwrap(),
            adapter,
            session: "fixture".into(),
            actor: Some(actor),
            evidence: Some(evidence),
        });
        Self {
            root,
            configuration,
            context,
            payload,
            gates,
            server,
        }
    }

    pub(super) fn bytes(&self) -> Option<Vec<u8>> {
        match std::fs::read(self.root.join("state/orchestration/state.json")) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("{error}"),
        }
    }

    pub(super) fn gate(
        &self,
        preceding_reads: usize,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<Option<serde_json::Value>>,
    ) {
        let (reached, ready) = tokio::sync::oneshot::channel();
        let (response, receive) = tokio::sync::oneshot::channel();
        let mut gates = self.gates.lock();
        gates.extend((0..preceding_reads).map(|_| None));
        gates.push_back(Some(ReadGate {
            reached,
            response: receive,
        }));
        (ready, response)
    }

    pub(super) async fn adopt(&self) -> String {
        let response = self
            .context
            .mutate(OrchestrationAction::RunAdopt {
                label: "Socket-backed host test".into(),
            })
            .await
            .unwrap();
        let OrchestrationActionResult::Run { run_id, .. } = response.result else {
            panic!("expected adopted run");
        };
        run_id
    }

    pub(super) fn operator(&self, action: OrchestrationAction) {
        let revision = self
            .bytes()
            .map(|bytes| {
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["revision"]
                    .as_u64()
                    .unwrap()
            })
            .unwrap_or(0);
        self.context
            .service
            .mutate(
                &Actor::Operator(OperatorOrigin::Browser),
                OrchestrationMutationRequest {
                    session_id: "fixture".into(),
                    expected_revision: Some(revision),
                    action,
                },
            )
            .unwrap();
    }

    pub(super) fn send(&self, run_id: &str, id: &str) {
        self.operator(OrchestrationAction::MessageSend {
            message_id: id.into(),
            to_run_id: run_id.into(),
            kind: MessageKind::Instruction,
            text: id.into(),
            in_reply_to: None,
        });
    }
}

impl Drop for SocketFixture {
    fn drop(&mut self) {
        self.server.abort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(super) async fn reach_gate_without_advancing_time(
    mut ready: tokio::sync::oneshot::Receiver<()>,
) {
    // Keep a runnable task while paused so socket readiness cannot auto-advance
    // the clock to an unrelated transport timeout.
    loop {
        tokio::select! {
            biased;
            result = &mut ready => { result.unwrap(); return; }
            _ = tokio::task::yield_now() => {}
        }
    }
}

pub(super) struct EndpointChild {
    child: std::process::Child,
    socket: PathBuf,
}

impl EndpointChild {
    pub(super) fn start(root: &std::path::Path) -> Self {
        use std::io::{BufRead, BufReader};
        let socket = root.join("herdr.sock");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "cli_orchestration::tests::socket_endpoint_responder_process",
                "--nocapture",
            ])
            .env("CK_HOST_TEST_SOCKET", &socket)
            .env("CK_HOST_TEST_PAYLOAD", root.join("payload.json"))
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut owned = Self { child, socket };
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 {
                panic!(
                    "endpoint child exited before readiness: {:?}",
                    owned.child.try_wait()
                );
            }
            if line.contains("CK_HOST_ENDPOINT_READY") {
                break;
            }
        }
        owned
    }
}

impl Drop for EndpointChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}
