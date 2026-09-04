use std::io;
use std::path::Path;

use base64::Engine;
use cockpit_core::{InspectionError, TerminalSession};
use cockpit_protocol::v1::{
    TerminalCommand, TerminalMode, TerminalMouseButton, TerminalMouseKind, TerminalOpenRequest,
    TerminalOwnershipState, TerminalScrollDirection, TerminalScrollSource, TerminalStreamMessage,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

const HERDR_PROTOCOL_VERSION: u32 = 20;
const MAX_HERDR_FRAME_SIZE: usize = 32 * 1024 * 1024;

#[allow(dead_code)]
#[derive(Debug, Serialize)]
enum RenderEncoding {
    SemanticFrame,
    TerminalAnsi,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
enum ClientKeybindings {
    Server,
    Local { keys_toml: String },
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
enum ClientLaunchMode {
    App,
    AppDirectGraphics,
    TerminalAttach,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
enum AttachScrollDirection {
    Up,
    Down,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
enum AttachScrollSource {
    Wheel,
    PageKey { input: Vec<u8> },
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
enum ClientMouseButton {
    Left,
    Right,
    Middle,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
enum ClientMouseKind {
    Down(ClientMouseButton),
    Up(ClientMouseButton),
    Drag(ClientMouseButton),
    Moved,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
enum ClientInputEvent {
    Key,
    TextCommit(String),
    Mouse {
        kind: ClientMouseKind,
        column: u16,
        row: u16,
        modifiers: u8,
    },
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
enum ClientMessage {
    Hello {
        version: u32,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
        requested_encoding: RenderEncoding,
        keybindings: ClientKeybindings,
        launch_mode: ClientLaunchMode,
    },
    Input {
        data: Vec<u8>,
    },
    ClipboardImage {
        extension: String,
        data: Vec<u8>,
    },
    Resize {
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    },
    Detach,
    AttachTerminal {
        terminal_id: String,
        takeover: bool,
    },
    AttachScroll {
        source: AttachScrollSource,
        direction: AttachScrollDirection,
        lines: u16,
        column: Option<u16>,
        row: Option<u16>,
        modifiers: u8,
    },
    InputEvents {
        events: Vec<ClientInputEvent>,
    },
    ObserveTerminal {
        target: String,
    },
    ControlTerminal {
        target: String,
        takeover: bool,
    },
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, PartialEq, Eq)]
enum ServerRenderEncoding {
    SemanticFrame,
    TerminalAnsi,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
enum NotifyKind {
    Sound,
    Toast,
    SystemToast,
}

#[derive(Debug, Deserialize)]
struct TerminalFrame {
    seq: u64,
    width: u16,
    height: u16,
    full: bool,
    bytes: Vec<u8>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
enum ServerMessage {
    Welcome {
        version: u32,
        encoding: ServerRenderEncoding,
        error: Option<String>,
    },
    Frame(()),
    Terminal(TerminalFrame),
    Graphics {
        bytes: Vec<u8>,
    },
    ServerShutdown {
        reason: Option<String>,
    },
    Notify {
        kind: NotifyKind,
        message: String,
        body: Option<String>,
    },
    Clipboard {
        data: String,
    },
    WindowTitle {
        title: Option<String>,
    },
    ReloadSoundConfig,
    MouseCapture {
        enabled: bool,
        sgr_pixels: bool,
    },
    KittyKeyboardReportAll {
        enabled: bool,
    },
    PrefixInputSource {
        active: bool,
    },
    TerminalBell {
        count: u16,
    },
    GraphicsFile {
        path: String,
        expected_len: u64,
        image_id: u32,
        transfer_id: u64,
        leading: Vec<u8>,
        control: String,
    },
    GraphicsTransmissionRetired {
        transfer_id: u64,
        image_id: u32,
    },
}

pub(crate) async fn open_terminal(
    socket_path: &Path,
    request: &TerminalOpenRequest,
    stream_id: String,
) -> Result<TerminalSession, InspectionError> {
    debug_assert_eq!(request.mode, TerminalMode::Control);
    let mut socket = UnixStream::connect(socket_path).await.map_err(|_| {
        InspectionError::new(
            "terminal_attach_failed",
            "Herdr terminal protocol connection failed",
        )
    })?;
    send_message(
        &mut socket,
        &ClientMessage::Hello {
            version: HERDR_PROTOCOL_VERSION,
            cols: request.cols,
            rows: request.rows,
            cell_width_px: 0,
            cell_height_px: 0,
            requested_encoding: RenderEncoding::TerminalAnsi,
            keybindings: ClientKeybindings::Server,
            launch_mode: ClientLaunchMode::TerminalAttach,
        },
    )
    .await
    .map_err(handshake_error)?;
    match read_message(&mut socket).await.map_err(handshake_error)? {
        ServerMessage::Welcome {
            version,
            encoding: ServerRenderEncoding::TerminalAnsi,
            error: None,
        } if version == HERDR_PROTOCOL_VERSION => {}
        ServerMessage::Welcome {
            error: Some(error), ..
        } => {
            return Err(InspectionError::new(
                "terminal_attach_failed",
                bounded_reason(&error),
            ));
        }
        _ => {
            return Err(InspectionError::new(
                "terminal_attach_failed",
                "Herdr terminal protocol negotiation failed",
            ));
        }
    }
    send_message(
        &mut socket,
        &ClientMessage::ControlTerminal {
            target: request.pane_id.clone(),
            takeover: request.takeover,
        },
    )
    .await
    .map_err(handshake_error)?;

    let (reader, writer) = socket.into_split();
    let (sender, receiver) = mpsc::channel(64);
    let (commands, command_receiver) = mpsc::channel(32);
    let context = TerminalContext {
        session_id: request.session_id.clone(),
        pane_id: request.pane_id.clone(),
        stream_id: stream_id.clone(),
    };
    tokio::spawn(run_terminal(
        reader,
        writer,
        command_receiver,
        sender,
        context,
    ));
    Ok(TerminalSession {
        stream_id,
        messages: receiver,
        commands,
    })
}

fn handshake_error(_: io::Error) -> InspectionError {
    InspectionError::new(
        "terminal_attach_failed",
        "Herdr terminal protocol negotiation failed",
    )
}

struct TerminalContext {
    session_id: String,
    pane_id: String,
    stream_id: String,
}

async fn run_terminal<R, W>(
    mut reader: R,
    mut writer: W,
    mut commands: mpsc::Receiver<TerminalCommand>,
    sender: mpsc::Sender<TerminalStreamMessage>,
    context: TerminalContext,
) where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let TerminalContext {
        session_id,
        pane_id,
        stream_id,
    } = context;
    let _ = sender
        .send(TerminalStreamMessage::Ownership {
            session_id: session_id.clone(),
            pane_id: pane_id.clone(),
            stream_id: stream_id.clone(),
            state: TerminalOwnershipState::Pending,
            message: None,
        })
        .await;
    let mut previous_seq: Option<u64> = None;
    let mut owned = false;

    loop {
        tokio::select! {
            message = read_message(&mut reader) => {
                match message {
                    Ok(ServerMessage::Terminal(frame)) => {
                        if previous_seq.is_none() && !frame.full {
                            send_error(&sender, &session_id, &pane_id, &stream_id, "terminal_baseline_required", "terminal stream must begin with a full frame").await;
                            return;
                        }
                        if let Some(previous) = previous_seq
                            && frame.seq != previous.saturating_add(1)
                        {
                            send_error(&sender, &session_id, &pane_id, &stream_id, "terminal_sequence_error", "terminal frame sequence is not consecutive").await;
                            return;
                        }
                        previous_seq = Some(frame.seq);
                        if !owned {
                            owned = true;
                            let _ = sender.send(TerminalStreamMessage::Ownership {
                                session_id: session_id.clone(),
                                pane_id: pane_id.clone(),
                                stream_id: stream_id.clone(),
                                state: TerminalOwnershipState::Owned,
                                message: None,
                            }).await;
                        }
                        let bytes = base64::engine::general_purpose::STANDARD.encode(frame.bytes);
                        if sender.send(TerminalStreamMessage::Frame {
                            session_id: session_id.clone(),
                            pane_id: pane_id.clone(),
                            stream_id: stream_id.clone(),
                            seq: frame.seq.to_string(),
                            encoding: "ansi".to_owned(),
                            width: frame.width,
                            height: frame.height,
                            full: frame.full,
                            bytes,
                        }).await.is_err() {
                            return;
                        }
                    }
                    Ok(ServerMessage::ServerShutdown { reason }) => {
                        let reason = bounded_reason(reason.as_deref().unwrap_or("closed"));
                        let lower = reason.to_ascii_lowercase();
                        if lower.contains("already controlled") || lower.contains("takeover") && !lower.contains("taken over") {
                            let _ = sender.send(TerminalStreamMessage::Ownership {
                                session_id,
                                pane_id,
                                stream_id,
                                state: TerminalOwnershipState::Conflict,
                                message: Some(reason),
                            }).await;
                        } else if lower.contains("taken over") {
                            let _ = sender.send(TerminalStreamMessage::Ownership {
                                session_id,
                                pane_id,
                                stream_id,
                                state: TerminalOwnershipState::Lost,
                                message: Some(reason),
                            }).await;
                        } else {
                            let _ = sender.send(TerminalStreamMessage::Closed {
                                session_id,
                                pane_id,
                                stream_id,
                                reason,
                            }).await;
                        }
                        return;
                    }
                    Ok(_) => {}
                    Err(_) => {
                        let _ = sender.send(TerminalStreamMessage::Disconnected {
                            session_id,
                            pane_id,
                            stream_id,
                            code: "terminal_disconnected".to_owned(),
                            message: "Herdr terminal protocol disconnected".to_owned(),
                        }).await;
                        return;
                    }
                }
            }
            command = commands.recv() => {
                let Some(command) = command else { return; };
                if command.validate().is_err() {
                    send_error(&sender, &session_id, &pane_id, &stream_id, "invalid_terminal_command", "terminal command is invalid").await;
                    continue;
                }
                if matches!(command, TerminalCommand::Release) {
                    let _ = send_message(&mut writer, &ClientMessage::Detach).await;
                    let _ = sender.send(TerminalStreamMessage::Ownership {
                        session_id,
                        pane_id,
                        stream_id,
                        state: TerminalOwnershipState::Released,
                        message: None,
                    }).await;
                    return;
                }
                if !owned {
                    send_error(&sender, &session_id, &pane_id, &stream_id, "terminal_command_rejected", "terminal control is pending ownership").await;
                    continue;
                }
                let message = match command_to_message(command) {
                    Ok(message) => message,
                    Err(message) => {
                        send_error(&sender, &session_id, &pane_id, &stream_id, "invalid_terminal_command", message).await;
                        continue;
                    }
                };
                if send_message(&mut writer, &message).await.is_err() {
                    let _ = sender.send(TerminalStreamMessage::Disconnected {
                        session_id,
                        pane_id,
                        stream_id,
                        code: "terminal_disconnected".to_owned(),
                        message: "Herdr terminal protocol disconnected".to_owned(),
                    }).await;
                    return;
                }
            }
        }
    }
}

fn command_to_message(command: TerminalCommand) -> Result<ClientMessage, &'static str> {
    match command {
        TerminalCommand::Input {
            text: Some(text), ..
        } => Ok(ClientMessage::Input {
            data: text.into_bytes(),
        }),
        TerminalCommand::Input {
            bytes: Some(bytes), ..
        } => base64::engine::general_purpose::STANDARD
            .decode(bytes)
            .map(|data| ClientMessage::Input { data })
            .map_err(|_| "terminal input bytes are not valid base64"),
        TerminalCommand::Input { .. } => Err("terminal input is invalid"),
        TerminalCommand::Resize {
            cols,
            rows,
            cell_width_px,
            cell_height_px,
        } => Ok(ClientMessage::Resize {
            cols,
            rows,
            cell_width_px,
            cell_height_px,
        }),
        TerminalCommand::Scroll {
            direction,
            lines,
            source,
            column,
            row,
            modifiers,
        } => Ok(ClientMessage::AttachScroll {
            source: match source {
                TerminalScrollSource::Wheel => AttachScrollSource::Wheel,
                TerminalScrollSource::PageKey => AttachScrollSource::PageKey {
                    input: match direction {
                        TerminalScrollDirection::Up => b"\x1b[5~".to_vec(),
                        TerminalScrollDirection::Down => b"\x1b[6~".to_vec(),
                    },
                },
            },
            direction: match direction {
                TerminalScrollDirection::Up => AttachScrollDirection::Up,
                TerminalScrollDirection::Down => AttachScrollDirection::Down,
            },
            lines: u16::try_from(lines).map_err(|_| "terminal scroll lines are invalid")?,
            column,
            row,
            modifiers,
        }),
        TerminalCommand::Mouse {
            kind,
            button,
            column,
            row,
            modifiers,
        } => Ok(ClientMessage::InputEvents {
            events: vec![ClientInputEvent::Mouse {
                kind: mouse_kind(kind, button)?,
                column,
                row,
                modifiers,
            }],
        }),
        TerminalCommand::Release => Ok(ClientMessage::Detach),
    }
}

fn mouse_kind(
    kind: TerminalMouseKind,
    button: Option<TerminalMouseButton>,
) -> Result<ClientMouseKind, &'static str> {
    let button = || {
        button.map(|button| match button {
            TerminalMouseButton::Left => ClientMouseButton::Left,
            TerminalMouseButton::Right => ClientMouseButton::Right,
            TerminalMouseButton::Middle => ClientMouseButton::Middle,
        })
    };
    match kind {
        TerminalMouseKind::Down => button().map(ClientMouseKind::Down),
        TerminalMouseKind::Up => button().map(ClientMouseKind::Up),
        TerminalMouseKind::Drag => button().map(ClientMouseKind::Drag),
        TerminalMouseKind::Moved if button().is_none() => Some(ClientMouseKind::Moved),
        TerminalMouseKind::Moved => None,
    }
    .ok_or("terminal mouse button does not match event kind")
}

async fn send_error(
    sender: &mpsc::Sender<TerminalStreamMessage>,
    session_id: &str,
    pane_id: &str,
    stream_id: &str,
    code: &str,
    message: &str,
) {
    let _ = sender
        .send(TerminalStreamMessage::Error {
            session_id: session_id.to_owned(),
            pane_id: pane_id.to_owned(),
            stream_id: stream_id.to_owned(),
            code: code.to_owned(),
            message: message.to_owned(),
        })
        .await;
}

fn bounded_reason(reason: &str) -> String {
    reason.chars().take(512).collect()
}

async fn send_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &ClientMessage,
) -> io::Result<()> {
    let payload = encode_message(message)?;
    let size = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Herdr frame is too large"))?;
    writer.write_all(&size.to_le_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await
}

fn encode_message(message: &ClientMessage) -> io::Result<Vec<u8>> {
    bincode::serde::encode_to_vec(message, bincode::config::standard())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Herdr message encoding failed"))
}

async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<ServerMessage> {
    let size = reader.read_u32_le().await? as usize;
    if size == 0 || size > MAX_HERDR_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Herdr frame size is invalid",
        ));
    }
    let mut payload = vec![0; size];
    reader.read_exact(&mut payload).await?;
    let (message, consumed): (ServerMessage, usize) =
        bincode::serde::decode_from_slice(&payload, bincode::config::standard()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "Herdr message decoding failed")
        })?;
    if consumed != payload.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Herdr message has trailing data",
        ));
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_20_handshake_matches_herdr_wire() {
        let payload = encode_message(&ClientMessage::Hello {
            version: 20,
            cols: 80,
            rows: 24,
            cell_width_px: 0,
            cell_height_px: 0,
            requested_encoding: RenderEncoding::TerminalAnsi,
            keybindings: ClientKeybindings::Server,
            launch_mode: ClientLaunchMode::TerminalAttach,
        })
        .unwrap();
        assert_eq!(payload, [0, 20, 80, 24, 0, 0, 1, 0, 2]);
    }

    #[test]
    fn control_and_mouse_messages_match_herdr_wire() {
        let control = encode_message(&ClientMessage::ControlTerminal {
            target: "w1:p1".to_owned(),
            takeover: true,
        })
        .unwrap();
        assert_eq!(control, [9, 5, b'w', b'1', b':', b'p', b'1', 1]);

        let mouse = command_to_message(TerminalCommand::Mouse {
            kind: TerminalMouseKind::Down,
            button: Some(TerminalMouseButton::Left),
            column: 37,
            row: 7,
            modifiers: 0,
        })
        .unwrap();
        assert_eq!(encode_message(&mouse).unwrap(), [7, 1, 2, 0, 0, 37, 7, 0]);
    }

    #[test]
    fn page_scroll_preserves_the_original_key_bytes() {
        let page_up = command_to_message(TerminalCommand::Scroll {
            direction: TerminalScrollDirection::Up,
            lines: 1,
            source: TerminalScrollSource::PageKey,
            column: None,
            row: None,
            modifiers: 0,
        })
        .unwrap();
        assert_eq!(
            encode_message(&page_up).unwrap(),
            [6, 1, 4, 27, b'[', b'5', b'~', 0, 1, 0, 0, 0]
        );

        let page_down = command_to_message(TerminalCommand::Scroll {
            direction: TerminalScrollDirection::Down,
            lines: 1,
            source: TerminalScrollSource::PageKey,
            column: None,
            row: None,
            modifiers: 0,
        })
        .unwrap();
        assert_eq!(
            encode_message(&page_down).unwrap(),
            [6, 1, 4, 27, b'[', b'6', b'~', 1, 1, 0, 0, 0]
        );
    }

    #[tokio::test]
    async fn terminal_frame_decodes_from_herdr_wire() {
        let payload = [2, 42, 80, 24, 1, 3, b'a', b'b', b'c'];
        let mut framed = Vec::from((payload.len() as u32).to_le_bytes());
        framed.extend_from_slice(&payload);
        let message = read_message(&mut framed.as_slice()).await.unwrap();
        let ServerMessage::Terminal(frame) = message else {
            panic!("expected terminal frame");
        };
        assert_eq!(frame.seq, 42);
        assert_eq!(frame.width, 80);
        assert_eq!(frame.height, 24);
        assert!(frame.full);
        assert_eq!(frame.bytes, b"abc");
    }
}
