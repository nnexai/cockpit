use cockpit_core::{InspectionError, TerminalSession};
use cockpit_protocol::v1::TerminalOpenRequest;

#[cfg(not(unix))]
pub(crate) fn open(_: &TerminalOpenRequest, _: &str) -> Result<TerminalSession, InspectionError> {
    Err(InspectionError::new("scratch_terminal_unavailable", "Disposable terminals are unavailable on this platform"))
}

#[cfg(unix)]
pub(crate) fn open(request: &TerminalOpenRequest, cwd: &str) -> Result<TerminalSession, InspectionError> {
    let (shell, login) = crate::cli::terminal_shell_config();
    open_with_shell(request, cwd, shell, login, &[])
}

#[cfg(unix)]
fn open_with_shell(request: &TerminalOpenRequest, cwd: &str, shell: std::ffi::OsString, login: bool, environment: &[(&str, &str)]) -> Result<TerminalSession, InspectionError> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use cockpit_protocol::v1::{TerminalCommand, TerminalOwnershipState, TerminalStreamMessage};
    use pty_process::{Command, Size};
    use std::{collections::VecDeque, path::Path, sync::atomic::{AtomicU64, Ordering}};
    use tokio::{io::{AsyncReadExt, AsyncWriteExt}, sync::mpsc};

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let path = Path::new(cwd);
    if !path.is_absolute() || !path.is_dir() {
        return Err(InspectionError::new("scratch_cwd_unavailable", "Source pane working directory is not an available absolute directory"));
    }
    let failure = |error: &dyn std::fmt::Display| InspectionError::new("scratch_terminal_failed", error.to_string());
    let (pty, pts) = pty_process::open().map_err(|e| failure(&e))?;
    pty.resize(Size::new(request.rows, request.cols)).map_err(|e| failure(&e))?;
    let mut command = Command::new(&shell).current_dir(path).env("TERM", "xterm-256color")
        .envs(environment.iter().copied()).kill_on_drop(true);
    if login {
        let mut argv0 = std::ffi::OsString::from("-");
        argv0.push(Path::new(&shell).file_name().unwrap_or(&shell));
        command = command.arg0(argv0);
    }
    let mut child = command.spawn(pts).map_err(|e| failure(&e))?;
    let stream_id = format!("scratch-{}-{}", std::process::id(), NEXT_ID.fetch_add(1, Ordering::Relaxed));
    let session_id = request.session_id.clone();
    let pane_id = request.pane_id.clone();
    let task_stream = stream_id.clone();
    let (messages_tx, messages) = mpsc::channel(64);
    let (commands, mut commands_rx) = mpsc::channel(64);
    let (mut width, mut height) = (request.cols, request.rows);
    tokio::spawn(async move {
        // Both messages fit in the newly-created channel and establish the
        // baseline required by every terminal stream consumer.
        if messages_tx.try_send(TerminalStreamMessage::Ownership {
            session_id: session_id.clone(), pane_id: pane_id.clone(), stream_id: task_stream.clone(),
            state: TerminalOwnershipState::Owned, message: None,
        }).is_err() { return; }
        if messages_tx.try_send(TerminalStreamMessage::Frame {
            session_id: session_id.clone(), pane_id: pane_id.clone(), stream_id: task_stream.clone(),
            seq: "1".into(), encoding: "ansi".into(), width, height, full: true, bytes: String::new(),
        }).is_err() { return; }
        let (mut reader, mut writer) = pty.into_split();
        let mut seq = 1u64;
        let mut buffer = [0u8; 16384];
        let mut input = VecDeque::<Vec<u8>>::new();
        let mut input_offset = 0;
        let mut pending_frame = None;
        let reason = loop {
            tokio::select! {
                _ = messages_tx.closed() => break "released",
                _ = child.wait() => break "exited",
                permit = messages_tx.reserve(), if pending_frame.is_some() => {
                    match permit {
                        Ok(permit) => permit.send(pending_frame.take().unwrap()),
                        Err(_) => break "released",
                    }
                }
                read = reader.read(&mut buffer), if pending_frame.is_none() => {
                    match read {
                        Ok(0) | Err(_) => break "exited",
                        Ok(n) => {
                            seq += 1;
                            pending_frame = Some(TerminalStreamMessage::Frame {
                                session_id: session_id.clone(), pane_id: pane_id.clone(), stream_id: task_stream.clone(),
                                seq: seq.to_string(), encoding: "ansi".into(), width, height, full: false,
                                bytes: STANDARD.encode(&buffer[..n]),
                            });
                        }
                    }
                }
                written = writer.write(input.front().map_or(&[][..], |bytes| &bytes[input_offset..])), if !input.is_empty() => {
                    match written {
                        Ok(0) | Err(_) => break "exited",
                        Ok(n) => {
                            input_offset += n;
                            if input_offset == input.front().unwrap().len() {
                                input.pop_front();
                                input_offset = 0;
                            }
                        }
                    }
                }
                command = commands_rx.recv() => {
                    match command {
                        None | Some(TerminalCommand::Release | TerminalCommand::Terminate) => break "terminated",
                        Some(TerminalCommand::Input { text, bytes }) => {
                            let bytes = match (text, bytes) {
                                (Some(text), None) => text.into_bytes(),
                                (None, Some(bytes)) => match STANDARD.decode(bytes) { Ok(bytes) => bytes, Err(_) => continue },
                                _ => continue,
                            };
                            if !bytes.is_empty() { input.push_back(bytes); }
                        }
                        Some(TerminalCommand::Resize { cols, rows, .. }) if cols > 0 && rows > 0 => {
                            if writer.resize(Size::new(rows, cols)).is_ok() { width = cols; height = rows; }
                        }
                        Some(_) => {}
                    }
                }
            }
        };
        if reason == "exited" {
            // Preserve available trailing output, but never wait for channel
            // capacity or for a descendant holding the slave indefinitely.
            let can_drain = pending_frame.take().map_or(true, |frame| messages_tx.try_send(frame).is_ok());
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(100);
            while can_drain {
                match tokio::time::timeout_at(deadline, reader.read(&mut buffer)).await {
                    Ok(Ok(n)) if n > 0 => {
                        seq += 1;
                        if messages_tx.try_send(TerminalStreamMessage::Frame {
                            session_id: session_id.clone(), pane_id: pane_id.clone(), stream_id: task_stream.clone(),
                            seq: seq.to_string(), encoding: "ansi".into(), width, height, full: false,
                            bytes: STANDARD.encode(&buffer[..n]),
                        }).is_err() { break; }
                    }
                    _ => break,
                }
            }
        }
        // Drop both master halves before reaping; no backpressured send can
        // prevent the hangup or leave the owned shell running.
        drop(reader);
        drop(writer);
        let _ = child.start_kill();
        let _ = child.wait().await;
        let _ = messages_tx.send(TerminalStreamMessage::Closed {
            session_id, pane_id, stream_id: task_stream, reason: reason.into(),
        }).await;
    });
    Ok(TerminalSession { stream_id, messages, commands })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use cockpit_protocol::v1::{TerminalCommand, TerminalMode, TerminalStreamMessage, TerminalTargetKind};
    use tokio::time::{Duration, timeout};

    fn request() -> TerminalOpenRequest {
        TerminalOpenRequest {
            session_id: "test".into(), pane_id: "pane".into(), target_kind: TerminalTargetKind::Scratch,
            mode: TerminalMode::Control, takeover: false, cols: 80, rows: 24,
            cell_width_px: 0, cell_height_px: 0,
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn configured_login_shell_reads_profile_and_preserves_cwd() {
        let root = std::env::temp_dir().join(format!(
            "cockpit-scratch-profile-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".bash_profile"), "export COCKPIT_SCRATCH_PROFILE=profile_loaded\n").unwrap();
        let config = root.join("config.toml");
        std::fs::write(&config, "[terminal]\ndefault_shell = \"/bin/bash\"\nshell_mode = \"login\"\n").unwrap();
        let (shell, login) = crate::cli::terminal_shell_config_from(Some(&config), Some("/bin/sh".into()));
        let home = root.to_str().unwrap();
        let mut terminal = open_with_shell(&request(), home, shell, login, &[("HOME", home)]).unwrap();
        terminal.commands.send(TerminalCommand::input_text(
            "printf 'PROFILE=%s\\n' \"$COCKPIT_SCRATCH_PROFILE\"; pwd; exit\n"
        )).await.unwrap();
        let output = timeout(Duration::from_secs(10), async {
            let mut output = Vec::new();
            while let Some(message) = terminal.messages.recv().await {
                match message {
                    TerminalStreamMessage::Frame { bytes, .. } => output.extend(STANDARD.decode(bytes).unwrap()),
                    TerminalStreamMessage::Closed { .. } => return String::from_utf8_lossy(&output).into_owned(),
                    _ => {}
                }
            }
            panic!("stream ended without closed")
        }).await.unwrap();
        assert!(output.contains("PROFILE=profile_loaded"), "{output}");
        assert!(output.contains(home), "{output}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn scratch_executes_input_and_reports_exit() {
        let mut terminal = open(&request(), "/tmp").unwrap();
        terminal.commands.send(TerminalCommand::input_text("printf 'scratch-proof\\n'; exit\n")).await.unwrap();
        let mut output = Vec::new();
        timeout(Duration::from_secs(10), async {
            while let Some(message) = terminal.messages.recv().await {
                match message {
                    TerminalStreamMessage::Frame { bytes, .. } => output.extend(STANDARD.decode(bytes).unwrap()),
                    TerminalStreamMessage::Closed { .. } => return,
                    _ => {}
                }
            }
            panic!("stream ended without closed");
        }).await.unwrap();
        assert!(String::from_utf8_lossy(&output).contains("scratch-proof"));
    }

    #[tokio::test]
    async fn scratch_terminate_emits_closed() {
        let mut terminal = open(&request(), "/tmp").unwrap();
        terminal.commands.send(TerminalCommand::Terminate).await.unwrap();
        timeout(Duration::from_secs(10), async {
            while let Some(message) = terminal.messages.recv().await {
                if let TerminalStreamMessage::Closed { reason, .. } = message {
                    assert_eq!(reason, "terminated");
                    return;
                }
            }
            panic!("stream ended without closed");
        }).await.unwrap();
    }

    #[tokio::test]
    async fn scratch_baseline_and_resize_are_visible_to_stream() {
        let mut terminal = open(&request(), "/tmp").unwrap();
        assert!(matches!(terminal.messages.recv().await, Some(TerminalStreamMessage::Ownership { .. })));
        match terminal.messages.recv().await.unwrap() {
            TerminalStreamMessage::Frame { seq, full, bytes, width, height, .. } => {
                assert_eq!((seq.as_str(), full, bytes.as_str(), width, height), ("1", true, "", 80, 24));
            }
            other => panic!("expected baseline, got {other:?}"),
        }
        terminal.commands.send(TerminalCommand::Resize {
            cols: 93, rows: 37, cell_width_px: 0, cell_height_px: 0,
        }).await.unwrap();
        terminal.commands.send(TerminalCommand::input_text("stty size; exit\n")).await.unwrap();
        timeout(Duration::from_secs(10), async {
            let mut output = String::new();
            let mut next = 2;
            while let Some(message) = terminal.messages.recv().await {
                match message {
                    TerminalStreamMessage::Frame { seq, full, bytes, width, height, .. } => {
                        assert_eq!(seq, next.to_string());
                        next += 1;
                        assert!(!full);
                        if (width, height) == (93, 37) {
                            output.push_str(&String::from_utf8_lossy(&STANDARD.decode(bytes).unwrap()));
                        }
                    }
                    TerminalStreamMessage::Closed { .. } => {
                        assert!(output.contains("37 93"), "{output}");
                        return;
                    }
                    _ => {}
                }
            }
            panic!("stream ended without closed");
        }).await.unwrap();
    }

    #[tokio::test]
    async fn scratch_terminate_interrupts_backpressured_input() {
        let mut terminal = open(&request(), "/tmp").unwrap();
        terminal.commands.send(TerminalCommand::input_text("stty -echo; printf '\\nREADY\\n'; exec sleep 30\n")).await.unwrap();
        timeout(Duration::from_secs(10), async {
            let mut output = String::new();
            while let Some(message) = terminal.messages.recv().await {
                if let TerminalStreamMessage::Frame { bytes, .. } = message {
                    output.push_str(&String::from_utf8_lossy(&STANDARD.decode(bytes).unwrap()));
                    if output.contains("READY\r\n") { return; }
                }
            }
            panic!("shell exited before becoming ready");
        }).await.unwrap();
        terminal.commands.send(TerminalCommand::input_text("x".repeat(2 * 1024 * 1024))).await.unwrap();
        terminal.commands.send(TerminalCommand::Terminate).await.unwrap();
        timeout(Duration::from_secs(10), async {
            while let Some(message) = terminal.messages.recv().await {
                if let TerminalStreamMessage::Closed { reason, .. } = message {
                    assert_eq!(reason, "terminated");
                    return;
                }
            }
            panic!("stream ended without closed");
        }).await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn scratch_terminate_reaps_child_with_full_output_channel() {
        let mut terminal = open(&request(), "/tmp").unwrap();
        terminal.commands.send(TerminalCommand::input_text(
            "stty -echo; printf '\\nPID:%s\\n' \"$$\"; exec yes\n",
        )).await.unwrap();
        let pid = timeout(Duration::from_secs(10), async {
            let mut output = String::new();
            while let Some(message) = terminal.messages.recv().await {
                if let TerminalStreamMessage::Frame { bytes, .. } = message {
                    output.push_str(&String::from_utf8_lossy(&STANDARD.decode(bytes).unwrap()));
                    for line in output.split_inclusive('\n').filter(|line| line.ends_with('\n')) {
                        if let Some(pid) = line.split_once("PID:").and_then(|(_, suffix)| suffix.trim().parse::<u32>().ok()) {
                            return pid;
                        }
                    }
                }
            }
            panic!("shell exited before reporting its pid");
        }).await.unwrap();
        timeout(Duration::from_secs(10), async {
            while terminal.messages.len() < 64 {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        terminal.commands.send(TerminalCommand::Terminate).await.unwrap();
        timeout(Duration::from_secs(10), async {
            while std::path::Path::new(&format!("/proc/{pid}")).exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
    }

    #[tokio::test]
    async fn scratch_rejects_missing_or_relative_cwd() {
        assert!(open(&request(), "relative").is_err());
        assert!(open(&request(), "/definitely-not-a-cockpit-directory").is_err());
    }
}
