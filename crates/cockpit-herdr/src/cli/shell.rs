use super::*;
use cockpit_protocol::herdr_shell::{HerdrShellState, HerdrShellStatus};

#[derive(Debug, Clone)]
pub(super) struct CachedShell {
    server_instance: String,
    state: HerdrShellState,
    listeners: Vec<mpsc::Sender<SessionChange>>,
    running: bool,
    generation: u64,
}

fn unavailable(error: &InspectionError, previous: Option<&HerdrShellState>) -> HerdrShellState {
    let mut state = previous.cloned().unwrap_or(HerdrShellState {
        status: HerdrShellStatus::Disconnected,
        prefix_bindings: Vec::new(),
        commands: Vec::new(),
        popup: None,
        error: None,
    });
    state.status = if error.code == "shell_unsupported" {
        HerdrShellStatus::Unsupported
    } else {
        HerdrShellStatus::Disconnected
    };
    state.error = Some(format!("{}: {}", error.code, error.message));
    state
}

fn server_instance(identity: &str) -> String {
    format!(
        "{:016x}",
        u64::from_be_bytes(
            Sha256::digest(identity.as_bytes())[..8]
                .try_into()
                .expect("SHA-256 prefix")
        )
    )
}

impl HerdrCliAdapter {
    async fn connect_shell(
        &self,
        session_id: &str,
        expected_instance: &str,
    ) -> Result<(crate::shell_wire::ShellConnection, String), InspectionError> {
        let path = self.client_socket_path(session_id)?;
        if !path
            .try_exists()
            .map_err(|error| InspectionError::new("shell_disconnected", error.to_string()))?
        {
            return Err(InspectionError::new(
                "shell_disconnected",
                "Herdr client-shell socket is unavailable",
            ));
        }
        let (identity, _) = self.browser_endpoint_identity(session_id).await?;
        if server_instance(&identity) != expected_instance {
            return Err(InspectionError::new(
                "session_identity_mismatch",
                "Herdr shell endpoint differs from the snapshot server",
            ));
        }
        let connection = crate::shell_wire::connect(&path, &identity).await?;
        let (after, _) = self.browser_endpoint_identity(session_id).await?;
        if after != identity {
            return Err(InspectionError::new(
                "session_identity_mismatch",
                "Herdr endpoint changed while reading shell metadata",
            ));
        }
        Ok((connection, identity))
    }

    pub(super) async fn shell_snapshot(
        &self,
        session_id: &str,
        expected_instance: &str,
    ) -> HerdrShellState {
        {
            let cache = self.shell_states.lock().await;
            if let Some(shell) = cache.get(session_id)
                && shell.running
                && shell.server_instance == expected_instance
            {
                return shell.state.clone();
            }
        }
        let result = self.connect_shell(session_id, expected_instance).await;
        let mut cache = self.shell_states.lock().await;
        let previous = cache
            .get(session_id)
            .filter(|shell| shell.server_instance == expected_instance);
        match result {
            Ok((connection, _)) => {
                let state = connection.state.clone();
                if !previous.is_some_and(|shell| shell.running) {
                    cache.insert(
                        session_id.to_owned(),
                        CachedShell {
                            server_instance: expected_instance.to_owned(),
                            state: state.clone(),
                            listeners: Vec::new(),
                            running: false,
                            generation: 0,
                        },
                    );
                }
                state
            }
            Err(error) => {
                let state = unavailable(&error, previous.map(|shell| &shell.state));
                if !previous.is_some_and(|shell| shell.running) {
                    cache.insert(
                        session_id.to_owned(),
                        CachedShell {
                            server_instance: expected_instance.to_owned(),
                            state: state.clone(),
                            listeners: Vec::new(),
                            running: false,
                            generation: 0,
                        },
                    );
                }
                state
            }
        }
    }

    pub(super) fn subscribe_shell(
        &self,
        session_id: String,
        expected_instance: String,
        sender: mpsc::Sender<SessionChange>,
    ) {
        let adapter = self.clone();
        tokio::spawn(async move {
            let generation = {
                let mut cache = adapter.shell_states.lock().await;
                let entry = cache
                    .entry(session_id.clone())
                    .or_insert_with(|| CachedShell {
                        server_instance: expected_instance.clone(),
                        state: HerdrShellState {
                            status: HerdrShellStatus::Connecting,
                            prefix_bindings: Vec::new(),
                            commands: Vec::new(),
                            popup: None,
                            error: None,
                        },
                        listeners: Vec::new(),
                        running: false,
                        generation: 0,
                    });
                if entry.server_instance != expected_instance {
                    *entry = CachedShell {
                        server_instance: expected_instance.clone(),
                        state: HerdrShellState {
                            status: HerdrShellStatus::Connecting,
                            prefix_bindings: Vec::new(),
                            commands: Vec::new(),
                            popup: None,
                            error: None,
                        },
                        listeners: Vec::new(),
                        running: false,
                        generation: 0,
                    };
                }
                entry.listeners.push(sender);
                if entry.running {
                    return;
                }
                entry.running = true;
                entry.generation = NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed);
                entry.generation
            };
            let mut delay = 100;
            loop {
                let connected = tokio::select! {
                    _ = adapter.shell_listeners_closed(&session_id, generation) => break,
                    result = adapter.connect_shell(&session_id, &expected_instance) => result,
                };
                let result = match connected {
                    Ok((mut connection, _)) => {
                        delay = 100;
                        adapter
                            .publish_shell(&session_id, generation, Ok(connection.state.clone()))
                            .await;
                        loop {
                            let result = tokio::select! {
                                _ = adapter.shell_listeners_closed(&session_id, generation) => break Ok(()),
                                result = connection.next() => result,
                            };
                            match result {
                                Ok(true) => {
                                    adapter
                                        .publish_shell(
                                            &session_id,
                                            generation,
                                            Ok(connection.state.clone()),
                                        )
                                        .await
                                }
                                Ok(false) => {}
                                Err(error) => break Err(error),
                            }
                        }
                    }
                    Err(error) => Err(error),
                };
                let Err(error) = result else {
                    break;
                };
                let terminal = matches!(
                    error.code.as_str(),
                    "session_identity_mismatch"
                        | "shell_identity_mismatch"
                        | "stale_identity"
                        | "shell_unsupported"
                );
                adapter
                    .publish_shell(&session_id, generation, Err(error))
                    .await;
                if terminal {
                    break;
                }
                tokio::select! {
                    _ = adapter.shell_listeners_closed(&session_id, generation) => break,
                    _ = tokio::time::sleep(Duration::from_millis(delay)) => {},
                }
                delay = (delay * 2).min(2000);
            }
            if let Some(shell) = adapter.shell_states.lock().await.get_mut(&session_id) {
                if shell.generation != generation {
                    return;
                }
                shell.running = false;
                shell.listeners.clear();
                if shell.state.status == HerdrShellStatus::Live {
                    shell.state.status = HerdrShellStatus::Disconnected;
                    shell.state.error = Some("Herdr shell subscription closed".into());
                }
            }
        });
    }

    async fn shell_listeners_closed(&self, session_id: &str, generation: u64) {
        loop {
            {
                let mut cache = self.shell_states.lock().await;
                let Some(shell) = cache.get_mut(session_id) else {
                    return;
                };
                if shell.generation != generation {
                    return;
                }
                shell.listeners.retain(|listener| !listener.is_closed());
                if shell.listeners.is_empty() {
                    shell.running = false;
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn publish_shell(
        &self,
        session_id: &str,
        generation: u64,
        result: Result<HerdrShellState, InspectionError>,
    ) {
        let listeners = {
            let mut cache = self.shell_states.lock().await;
            let Some(shell) = cache.get_mut(session_id) else {
                return;
            };
            if shell.generation != generation {
                return;
            }
            let state = match result {
                Ok(state) => state,
                Err(error) => unavailable(&error, Some(&shell.state)),
            };
            if shell.state == state {
                return;
            }
            shell.state = state;
            shell.listeners.retain(|listener| !listener.is_closed());
            shell.listeners.clone()
        };
        // Invalidations are coalescible: an already queued notification causes
        // a fresh cache read. One slow consumer must never block the shell reader.
        for listener in listeners {
            let _ = listener.try_send(SessionChange::Changed);
        }
    }

    pub(super) async fn validate_popup_target(
        &self,
        request: &TerminalOpenRequest,
    ) -> Result<String, InspectionError> {
        let snapshot = self
            .read_structure_with_identity(&request.session_id, None)
            .await?;
        // Re-read the surface at attach time: a stale cached terminal ID must never
        // attach to a replacement popup or to an ordinary pane by coincidence.
        let (connection, identity) = self
            .connect_shell(&request.session_id, &snapshot.server_instance)
            .await?;
        if connection.state.status != HerdrShellStatus::Live
            || !connection
                .state
                .popup
                .as_ref()
                .is_some_and(|popup| popup.terminal_id == request.pane_id)
        {
            return Err(InspectionError::new(
                "popup_not_open",
                "terminal target is not the live Herdr popup",
            ));
        }
        Ok(identity)
    }

    pub(super) async fn validate_command_target(
        &self,
        session_id: &str,
        request: &ResourceMutationRequest,
    ) -> Result<Option<String>, InspectionError> {
        let ResourceMutationRequest::CommandInvoke {
            command_id,
            space_id,
            tab_id,
            pane_id,
        } = request
        else {
            return Ok(None);
        };
        let snapshot = self.read_structure_with_identity(session_id, None).await?;
        if !snapshot.spaces.iter().any(|space| &space.id == space_id)
            || !snapshot
                .tabs
                .iter()
                .any(|tab| &tab.id == tab_id && &tab.space_id == space_id)
            || pane_id.as_ref().is_some_and(|id| {
                !snapshot.panes.iter().any(|pane| {
                    &pane.id == id && &pane.tab_id == tab_id && &pane.space_id == space_id
                })
            })
        {
            return Err(InspectionError::new(
                "command_target_invalid",
                "command target is not a member of the requested workspace and tab",
            ));
        }
        let (connection, identity) = self
            .connect_shell(session_id, &snapshot.server_instance)
            .await?;
        if !connection.state.commands.iter().any(|command| {
            &command.command_id == command_id
                && command.action != cockpit_protocol::herdr_shell::HerdrCommandAction::Unknown
        }) {
            return Err(InspectionError::new(
                "command_not_available",
                "command is no longer advertised by the live Herdr endpoint",
            ));
        }
        Ok(Some(identity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::herdr_shell::HerdrPopup;

    #[test]
    fn disconnect_preserves_popup_but_disables_shell() {
        let previous = HerdrShellState {
            status: HerdrShellStatus::Live,
            prefix_bindings: vec!["ctrl+x".into()],
            commands: Vec::new(),
            popup: Some(HerdrPopup {
                terminal_id: "popup-1".into(),
                title: "Actions".into(),
                width: None,
                height: None,
            }),
            error: None,
        };
        let state = unavailable(
            &InspectionError::new("disconnected", "closed"),
            Some(&previous),
        );
        assert_eq!(state.status, HerdrShellStatus::Disconnected);
        assert_eq!(state.popup, previous.popup);
        assert_eq!(state.prefix_bindings, previous.prefix_bindings);
        assert!(state.error.as_deref().unwrap().contains("closed"));
    }

    fn adapter() -> HerdrCliAdapter {
        HerdrCliAdapter::new(
            HerdrCliConfig::from_options(
                None,
                Some("default".into()),
                Some(PathBuf::from("/tmp/cockpit-shell-unit-unused.sock")),
            )
            .unwrap(),
        )
    }

    fn popup_state() -> HerdrShellState {
        HerdrShellState {
            status: HerdrShellStatus::Live,
            prefix_bindings: vec!["ctrl+x".into()],
            commands: Vec::new(),
            popup: Some(HerdrPopup {
                terminal_id: "popup-1".into(),
                title: "Actions".into(),
                width: None,
                height: None,
            }),
            error: None,
        }
    }

    #[tokio::test]
    async fn slow_listener_cannot_block_popup_close_for_other_consumers() {
        let adapter = adapter();
        let (slow, _slow_receiver) = mpsc::channel(1);
        slow.try_send(SessionChange::Changed).unwrap();
        let (fast, mut fast_receiver) = mpsc::channel(1);
        adapter.shell_states.lock().await.insert(
            "default".into(),
            CachedShell {
                server_instance: "instance".into(),
                state: popup_state(),
                listeners: vec![slow, fast],
                running: true,
                generation: 7,
            },
        );
        let mut closed = popup_state();
        closed.popup = None;
        tokio::time::timeout(
            Duration::from_millis(100),
            adapter.publish_shell("default", 7, Ok(closed)),
        )
        .await
        .expect("a full listener must not block the shared reader");
        assert!(matches!(
            fast_receiver.try_recv(),
            Ok(SessionChange::Changed)
        ));
        let snapshot = adapter.shell_snapshot("default", "instance").await;
        assert_eq!(snapshot.status, HerdrShellStatus::Live);
        assert_eq!(snapshot.popup, None);
    }

    #[tokio::test]
    async fn snapshot_during_subscription_setup_preserves_listener_and_generation() {
        let adapter = adapter();
        let (listener, mut receiver) = mpsc::channel(1);
        let mut connecting = popup_state();
        connecting.status = HerdrShellStatus::Connecting;
        adapter.shell_states.lock().await.insert(
            "default".into(),
            CachedShell {
                server_instance: "instance".into(),
                state: connecting,
                listeners: vec![listener],
                running: true,
                generation: 9,
            },
        );
        let snapshot = adapter.shell_snapshot("default", "instance").await;
        assert_eq!(snapshot.status, HerdrShellStatus::Connecting);
        adapter
            .publish_shell(
                "default",
                9,
                Err(InspectionError::new("disconnected", "closed")),
            )
            .await;
        assert!(matches!(receiver.try_recv(), Ok(SessionChange::Changed)));
        let snapshot = adapter.shell_snapshot("default", "instance").await;
        assert_eq!(snapshot.status, HerdrShellStatus::Disconnected);
        assert_eq!(snapshot.popup, popup_state().popup);
    }
}
