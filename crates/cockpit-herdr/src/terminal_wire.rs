use std::io;
use std::path::Path;
use std::time::Duration;

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
use tokio::time::timeout;

const HERDR_PROTOCOL_VERSION: u32 = 22;
const MAX_NORMAL_FRAME_SIZE: usize = 2 * 1024 * 1024;
const MAX_GRAPHICS_FRAME_SIZE: usize = 32 * 1024 * 1024;
const READER_BUFFERED_MESSAGES: usize = 8;
/// Terminal frames are streaming, but the finite connection/negotiation phase
/// must not leave a replaced pane waiting forever.
const TERMINAL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
enum ClientMessage {
    Hello {
        version: u32,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
        pixel_mouse: bool,
    },
    Input {
        data: Vec<u8>,
    },
    Resize {
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
        pixel_mouse: bool,
    },
    Detach,
    AttachScroll {
        source: AttachScrollSource,
        direction: AttachScrollDirection,
        lines: u16,
        column: Option<u16>,
        row: Option<u16>,
        modifiers: u8,
    },
    ObserveTerminal {
        target: String,
    },
    ControlTerminal {
        target: String,
        takeover: bool,
    },
    AttachMouse {
        kind: ClientMouseKind,
        position: ClientMousePosition,
        geometry: Option<ClientMouseGeometry>,
        modifiers: u8,
        lines: u16,
    },
}

#[derive(Debug, Serialize)]
enum ClientMouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Serialize)]
enum ClientMouseKind {
    Down(ClientMouseButton),
    Up(ClientMouseButton),
    Drag(ClientMouseButton),
    Moved,
}

#[derive(Debug, Serialize)]
enum ClientMousePosition {
    Cell { column: u16, row: u16 },
}

#[derive(Debug, Serialize)]
struct ClientMouseGeometry {
    cols: u16,
    rows: u16,
    width_px: u32,
    height_px: u32,
}

#[derive(Debug, Serialize)]
enum AttachScrollDirection {
    Up,
    Down,
}

#[derive(Debug, Serialize)]
enum AttachScrollSource {
    Wheel,
    PageKey { input: Vec<u8> },
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum ServerRenderEncoding {
    SemanticFrame,
    TerminalAnsi,
}

#[derive(Debug, Deserialize)]
struct WelcomePayload {
    version: u32,
    encoding: ServerRenderEncoding,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TerminalFrame {
    seq: u64,
    width: u16,
    height: u16,
    full: bool,
    bytes: Vec<u8>,
}

#[derive(Debug)]
enum ServerMessage {
    Welcome(WelcomePayload),
    Terminal(TerminalFrame),
    ServerShutdown { reason: Option<String> },
    MouseCapture { enabled: bool },
    Parked,
}

pub(crate) async fn open_terminal(
    socket_path: &Path,
    request: &TerminalOpenRequest,
    stream_id: String,
) -> Result<TerminalSession, InspectionError> {
    request
        .validate()
        .map_err(|message| InspectionError::new("invalid_terminal_dimensions", message))?;

    let mut socket = timeout(TERMINAL_HANDSHAKE_TIMEOUT, UnixStream::connect(socket_path))
        .await
        .map_err(|_| {
            InspectionError::new(
                "terminal_attach_timeout",
                "Herdr terminal connection timed out",
            )
        })?
        .map_err(|error| {
            InspectionError::new(
                "terminal_attach_failed",
                format!("Herdr terminal protocol connection failed: {error}"),
            )
        })?;
    timeout(
        TERMINAL_HANDSHAKE_TIMEOUT,
        send_message(
            &mut socket,
            &ClientMessage::Hello {
                version: HERDR_PROTOCOL_VERSION,
                cols: request.cols,
                rows: request.rows,
                cell_width_px: request.cell_width_px,
                cell_height_px: request.cell_height_px,
                pixel_mouse: false,
            },
        ),
    )
    .await
    .map_err(|_| InspectionError::new("terminal_attach_timeout", "Herdr terminal hello timed out"))?
    .map_err(handshake_error)?;

    match timeout(TERMINAL_HANDSHAKE_TIMEOUT, read_message(&mut socket))
        .await
        .map_err(|_| {
            InspectionError::new(
                "terminal_attach_timeout",
                "Herdr terminal negotiation timed out",
            )
        })?
        .map_err(handshake_error)?
    {
        ServerMessage::Welcome(WelcomePayload {
            version,
            encoding: ServerRenderEncoding::TerminalAnsi,
            error: None,
        }) if version == HERDR_PROTOCOL_VERSION => {}
        ServerMessage::Welcome(WelcomePayload {
            error: Some(error), ..
        }) => {
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

    let attach = match request.mode {
        TerminalMode::Observe => ClientMessage::ObserveTerminal {
            target: request.pane_id.clone(),
        },
        TerminalMode::Control => ClientMessage::ControlTerminal {
            target: request.pane_id.clone(),
            takeover: request.takeover,
        },
    };
    timeout(
        TERMINAL_HANDSHAKE_TIMEOUT,
        send_message(&mut socket, &attach),
    )
    .await
    .map_err(|_| {
        InspectionError::new(
            "terminal_attach_timeout",
            "Herdr terminal attach request timed out",
        )
    })?
    .map_err(handshake_error)?;

    let (reader, writer) = socket.into_split();
    let (sender, receiver) = mpsc::channel(64);
    let (commands, command_receiver) = mpsc::channel(32);
    let context = TerminalContext {
        session_id: request.session_id.clone(),
        pane_id: request.pane_id.clone(),
        stream_id: stream_id.clone(),
        mode: request.mode,
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

fn handshake_error(error: io::Error) -> InspectionError {
    InspectionError::new(
        "terminal_attach_failed",
        format!("Herdr terminal protocol negotiation failed: {error}"),
    )
}

struct TerminalContext {
    session_id: String,
    pane_id: String,
    stream_id: String,
    mode: TerminalMode,
}

async fn run_terminal<R, W>(
    reader: R,
    mut writer: W,
    mut commands: mpsc::Receiver<TerminalCommand>,
    sender: mpsc::Sender<TerminalStreamMessage>,
    context: TerminalContext,
) where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Unpin,
{
    let TerminalContext {
        session_id,
        pane_id,
        stream_id,
        mode,
    } = context;
    if sender
        .send(TerminalStreamMessage::Ownership {
            session_id: session_id.clone(),
            pane_id: pane_id.clone(),
            stream_id: stream_id.clone(),
            state: TerminalOwnershipState::Pending,
            message: None,
        })
        .await
        .is_err()
    {
        return;
    }

    let (message_sender, mut messages) = mpsc::channel(READER_BUFFERED_MESSAGES);
    let reader_task = tokio::spawn(read_terminal_messages(reader, message_sender));
    let mut previous_seq: Option<u64> = None;
    let mut attached = false;

    let mut mouse_enabled = false;
    loop {
        tokio::select! {
            message = messages.recv() => {
                match message {
                    Some(Ok(ServerMessage::Terminal(frame))) => {
                        if previous_seq.is_none() && !frame.full {
                            send_error(&sender, &session_id, &pane_id, &stream_id, "terminal_baseline_required", "terminal stream must begin with a full frame").await;
                            break;
                        }
                        if let Some(previous) = previous_seq
                            && frame.seq != previous.saturating_add(1)
                        {
                            send_error(&sender, &session_id, &pane_id, &stream_id, "terminal_sequence_error", "terminal frame sequence is not consecutive").await;
                            break;
                        }
                        previous_seq = Some(frame.seq);
                        if !attached {
                            // A terminal frame can only follow server-side mode selection, so it
                            // is the verified attach outcome; opening the socket never grants
                            // control.
                            attached = true;
                            let state = match mode {
                                TerminalMode::Observe => TerminalOwnershipState::Observing,
                                TerminalMode::Control => TerminalOwnershipState::Owned,
                            };
                            if sender.send(TerminalStreamMessage::Ownership {
                                session_id: session_id.clone(),
                                pane_id: pane_id.clone(),
                                stream_id: stream_id.clone(),
                                state,
                                message: None,
                            }).await.is_err() {
                                break;
                            }
                        }
                        if sender.send(TerminalStreamMessage::Frame {
                            session_id: session_id.clone(),
                            pane_id: pane_id.clone(),
                            stream_id: stream_id.clone(),
                            seq: frame.seq.to_string(),
                            encoding: "ansi".to_owned(),
                            width: frame.width,
                            height: frame.height,
                            full: frame.full,
                            bytes: base64::engine::general_purpose::STANDARD.encode(frame.bytes),
                        }).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(ServerMessage::MouseCapture { enabled })) => {
                        mouse_enabled = enabled;
                        if sender.send(TerminalStreamMessage::MouseMode {
                            session_id: session_id.clone(),
                            pane_id: pane_id.clone(),
                            stream_id: stream_id.clone(),
                            enabled,
                        }).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(ServerMessage::ServerShutdown { reason })) => {
                        let reason = bounded_reason(reason.as_deref().unwrap_or("closed"));
                        let lower = reason.to_ascii_lowercase();
                        let ownership = match mode {
                            TerminalMode::Control if !attached && lower.contains("already has an attached client") => Some(TerminalOwnershipState::Conflict),
                            TerminalMode::Control if attached && lower.contains("taken over") => Some(TerminalOwnershipState::Lost),
                            _ => None,
                        };
                        if let Some(state) = ownership {
                            let _ = sender.send(TerminalStreamMessage::Ownership {
                                session_id,
                                pane_id,
                                stream_id,
                                state,
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
                        break;
                    }
                    Some(Ok(ServerMessage::Parked)) => {}
                    Some(Ok(ServerMessage::Welcome(_))) => {
                        send_error(&sender, &session_id, &pane_id, &stream_id, "terminal_protocol_unsupported", "stable terminal attach received a second handshake").await;
                        break;
                    }
                    Some(Err(_)) | None => {
                        let _ = sender.send(TerminalStreamMessage::Disconnected {
                            session_id,
                            pane_id,
                            stream_id,
                            code: "terminal_disconnected".to_owned(),
                            message: "Herdr terminal protocol disconnected".to_owned(),
                        }).await;
                        break;
                    }
                }
            }
            command = commands.recv() => {
                let Some(command) = command else { break; };
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
                    break;
                }
                if !attached {
                    send_error(&sender, &session_id, &pane_id, &stream_id, "terminal_command_rejected", "terminal attach is still pending").await;
                    continue;
                }
                if mode == TerminalMode::Observe && !matches!(command, TerminalCommand::Resize { .. }) {
                    send_error(&sender, &session_id, &pane_id, &stream_id, "terminal_command_rejected", "terminal observe stream is read-only").await;
                    continue;
                }
                if matches!(command, TerminalCommand::Mouse { .. }) && !mouse_enabled {
                    send_error(&sender, &session_id, &pane_id, &stream_id, "terminal_command_rejected", "terminal application mouse mode is disabled").await;
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
                    break;
                }
            }
        }
    }

    reader_task.abort();
    let _ = reader_task.await;
}

async fn read_terminal_messages<R>(mut reader: R, sender: mpsc::Sender<io::Result<ServerMessage>>)
where
    R: AsyncRead + Unpin,
{
    loop {
        let message = read_message(&mut reader).await;
        let terminal = message.is_err();
        if sender.send(message).await.is_err() || terminal {
            return;
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
            pixel_mouse: false,
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
        } => {
            let map_button = |button| match button {
                TerminalMouseButton::Left => ClientMouseButton::Left,
                TerminalMouseButton::Right => ClientMouseButton::Right,
                TerminalMouseButton::Middle => ClientMouseButton::Middle,
            };
            let kind = match kind {
                TerminalMouseKind::Down => ClientMouseKind::Down(map_button(
                    button.ok_or("terminal mouse down events require a button")?,
                )),
                TerminalMouseKind::Up => ClientMouseKind::Up(map_button(
                    button.ok_or("terminal mouse up events require a button")?,
                )),
                TerminalMouseKind::Drag => ClientMouseKind::Drag(map_button(
                    button.ok_or("terminal mouse drag events require a button")?,
                )),
                TerminalMouseKind::Moved => {
                    if button.is_some() {
                        return Err("terminal moved mouse events cannot have a button");
                    }
                    ClientMouseKind::Moved
                }
            };
            Ok(ClientMessage::AttachMouse {
                kind,
                position: ClientMousePosition::Cell { column, row },
                geometry: None,
                modifiers,
                lines: 1,
            })
        }
        TerminalCommand::Release => Ok(ClientMessage::Detach),
    }
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
    let payload = encode_client_message(message)?;
    if payload.len() > MAX_NORMAL_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Herdr frame exceeds the 2 MiB limit",
        ));
    }
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(&payload).await?;
    writer.flush().await
}

fn encode_client_message(message: &ClientMessage) -> io::Result<Vec<u8>> {
    let config = bincode::config::standard();
    let payload = match message {
        ClientMessage::Hello {
            version,
            cols,
            rows,
            cell_width_px,
            cell_height_px,
            pixel_mouse,
        } => bincode::serde::encode_to_vec(
            &(
                0_u32,
                version,
                cols,
                rows,
                cell_width_px,
                cell_height_px,
                pixel_mouse,
            ),
            config,
        ),
        ClientMessage::Input { data } => bincode::serde::encode_to_vec(&(1_u32, data), config),
        ClientMessage::Resize {
            cols,
            rows,
            cell_width_px,
            cell_height_px,
            pixel_mouse,
        } => bincode::serde::encode_to_vec(
            &(
                3_u32,
                cols,
                rows,
                cell_width_px,
                cell_height_px,
                pixel_mouse,
            ),
            config,
        ),
        ClientMessage::Detach => bincode::serde::encode_to_vec(&4_u32, config),
        ClientMessage::AttachScroll {
            source,
            direction,
            lines,
            column,
            row,
            modifiers,
        } => bincode::serde::encode_to_vec(
            &(6_u32, source, direction, lines, column, row, modifiers),
            config,
        ),
        ClientMessage::ObserveTerminal { target } => {
            bincode::serde::encode_to_vec(&(7_u32, target), config)
        }
        ClientMessage::ControlTerminal { target, takeover } => {
            bincode::serde::encode_to_vec(&(8_u32, target, takeover), config)
        }
        ClientMessage::AttachMouse {
            kind,
            position,
            geometry,
            modifiers,
            lines,
        } => bincode::serde::encode_to_vec(
            &(16_u32, kind, position, geometry, modifiers, lines),
            config,
        ),
    };
    payload.map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<ServerMessage> {
    let size = reader.read_u32_le().await? as usize;
    if size == 0 {
        return Err(invalid_frame("Herdr frame is empty"));
    }

    let tag = reader.read_u8().await?;
    match tag {
        2 => {
            if size > MAX_GRAPHICS_FRAME_SIZE {
                return Err(frame_too_large(MAX_GRAPHICS_FRAME_SIZE));
            }
            read_graphics_payload(reader, size - 1).await?;
            Ok(ServerMessage::Parked)
        }
        0 | 1 | 3 | 8 => {
            if size > MAX_NORMAL_FRAME_SIZE {
                return Err(frame_too_large(MAX_NORMAL_FRAME_SIZE));
            }
            let mut payload = vec![0; size - 1];
            reader.read_exact(&mut payload).await?;
            match tag {
                0 => decode_exact(&payload).map(ServerMessage::Welcome),
                1 => decode_exact(&payload).map(ServerMessage::Terminal),
                3 => decode_exact(&payload).map(|reason| ServerMessage::ServerShutdown { reason }),
                // Pixel reporting is not negotiated by this cell-coordinate client.
                8 => decode_exact::<(bool, bool)>(&payload)
                    .map(|(enabled, _)| ServerMessage::MouseCapture { enabled }),
                _ => unreachable!(),
            }
        }
        4..=7 | 9..=20 => {
            if size > MAX_NORMAL_FRAME_SIZE {
                return Err(frame_too_large(MAX_NORMAL_FRAME_SIZE));
            }
            discard_payload(reader, size - 1).await?;
            Ok(ServerMessage::Parked)
        }
        _ => Err(invalid_frame("Herdr server message has an unknown tag")),
    }
}

fn decode_exact<T: for<'de> Deserialize<'de>>(payload: &[u8]) -> io::Result<T> {
    let (message, consumed) =
        bincode::serde::decode_from_slice(payload, bincode::config::standard())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if consumed != payload.len() {
        return Err(invalid_frame("Herdr message has trailing data"));
    }
    Ok(message)
}

async fn read_graphics_payload<R: AsyncRead + Unpin>(
    reader: &mut R,
    payload_size: usize,
) -> io::Result<()> {
    let mut remaining = payload_size;
    let graphics_size = read_bincode_length(reader, &mut remaining).await?;
    if graphics_size != remaining {
        return Err(invalid_frame(
            "Herdr graphics payload length does not match frame",
        ));
    }
    discard_payload(reader, remaining).await
}

async fn read_bincode_length<R: AsyncRead + Unpin>(
    reader: &mut R,
    remaining: &mut usize,
) -> io::Result<usize> {
    let marker = read_counted_byte(reader, remaining).await?;
    match marker {
        0..=250 => Ok(marker as usize),
        251 => Ok(u16::from_le_bytes(read_counted(reader, remaining).await?) as usize),
        252 => Ok(u32::from_le_bytes(read_counted(reader, remaining).await?) as usize),
        253 => usize::try_from(u64::from_le_bytes(read_counted(reader, remaining).await?))
            .map_err(|_| invalid_frame("Herdr graphics length does not fit usize")),
        _ => Err(invalid_frame("Herdr graphics length is invalid")),
    }
}

async fn read_counted_byte<R: AsyncRead + Unpin>(
    reader: &mut R,
    remaining: &mut usize,
) -> io::Result<u8> {
    if *remaining == 0 {
        return Err(invalid_frame("Herdr graphics payload is truncated"));
    }
    *remaining -= 1;
    reader.read_u8().await
}

async fn read_counted<const N: usize, R: AsyncRead + Unpin>(
    reader: &mut R,
    remaining: &mut usize,
) -> io::Result<[u8; N]> {
    if *remaining < N {
        return Err(invalid_frame("Herdr graphics payload is truncated"));
    }
    *remaining -= N;
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes).await?;
    Ok(bytes)
}

async fn discard_payload<R: AsyncRead + Unpin>(
    reader: &mut R,
    mut remaining: usize,
) -> io::Result<()> {
    let mut buffer = [0; 8192];
    while remaining > 0 {
        let count = remaining.min(buffer.len());
        reader.read_exact(&mut buffer[..count]).await?;
        remaining -= count;
    }
    Ok(())
}

fn frame_too_large(limit: usize) -> io::Error {
    invalid_frame(&format!(
        "Herdr frame exceeds the {} MiB limit",
        limit / 1024 / 1024
    ))
}

fn invalid_frame(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;
    use tokio::time::timeout;

    fn client_payload(message: ClientMessage) -> Vec<u8> {
        encode_client_message(&message).unwrap()
    }

    fn framed(payload: Vec<u8>) -> Vec<u8> {
        let mut frame = Vec::with_capacity(4 + payload.len());
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend(payload);
        frame
    }

    fn terminal_frame(seq: u64, full: bool, bytes: &[u8]) -> Vec<u8> {
        framed(
            bincode::serde::encode_to_vec(
                &(1_u32, seq, 80_u16, 24_u16, full, bytes),
                bincode::config::standard(),
            )
            .unwrap(),
        )
    }

    async fn next_message(
        receiver: &mut mpsc::Receiver<TerminalStreamMessage>,
    ) -> TerminalStreamMessage {
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("terminal actor stalled")
            .expect("terminal actor closed stream")
    }

    async fn read_client_payload<R: AsyncRead + Unpin>(reader: &mut R) -> Vec<u8> {
        let size = timeout(Duration::from_secs(1), reader.read_u32_le())
            .await
            .expect("terminal command was not written")
            .unwrap() as usize;
        let mut payload = vec![0; size];
        reader.read_exact(&mut payload).await.unwrap();
        payload
    }

    #[test]
    fn protocol_22_used_packets_match_stable_wire() {
        assert_eq!(
            client_payload(ClientMessage::Hello {
                version: 22,
                cols: 80,
                rows: 24,
                cell_width_px: 0,
                cell_height_px: 0,
                pixel_mouse: false,
            }),
            [0, 22, 80, 24, 0, 0, 0],
        );
        assert_eq!(
            client_payload(ClientMessage::Input {
                data: b"abc".to_vec(),
            }),
            [1, 3, b'a', b'b', b'c'],
        );
        assert_eq!(
            client_payload(ClientMessage::Resize {
                cols: 80,
                rows: 24,
                cell_width_px: 8,
                cell_height_px: 16,
                pixel_mouse: false,
            }),
            [3, 80, 24, 8, 16, 0],
        );
        assert_eq!(client_payload(ClientMessage::Detach), [4]);
        assert_eq!(
            client_payload(ClientMessage::AttachScroll {
                source: AttachScrollSource::PageKey {
                    input: b"\x1b[5~".to_vec(),
                },
                direction: AttachScrollDirection::Up,
                lines: 1,
                column: None,
                row: None,
                modifiers: 0,
            }),
            [6, 1, 4, 27, b'[', b'5', b'~', 0, 1, 0, 0, 0],
        );
        assert_eq!(
            client_payload(ClientMessage::ObserveTerminal {
                target: "w1:p".to_owned(),
            }),
            [7, 4, b'w', b'1', b':', b'p'],
        );
        assert_eq!(
            client_payload(ClientMessage::ControlTerminal {
                target: "w1:p".to_owned(),
                takeover: true,
            }),
            [8, 4, b'w', b'1', b':', b'p', 1],
        );
    }

    #[test]
    fn structured_mouse_uses_cell_coordinates_and_no_raw_input() {
        let message = command_to_message(TerminalCommand::Mouse {
            kind: TerminalMouseKind::Down,
            button: Some(TerminalMouseButton::Left),
            column: 4,
            row: 2,
            modifiers: 0,
        })
        .expect("semantic mouse should become structured input");
        assert_eq!(client_payload(message), [16, 0, 0, 0, 4, 2, 0, 0, 1],);
        let message = command_to_message(TerminalCommand::Mouse {
            kind: TerminalMouseKind::Moved,
            button: None,
            column: 4,
            row: 2,
            modifiers: 7,
        })
        .expect("semantic motion should become structured input");
        assert_eq!(client_payload(message), [16, 3, 0, 4, 2, 0, 7, 1]);
    }

    #[test]
    fn direct_input_preserves_raw_bytes() {
        let message = command_to_message(TerminalCommand::input_bytes("AP+A".to_owned())).unwrap();
        let ClientMessage::Input { data } = message else {
            panic!("expected raw input");
        };
        assert_eq!(data, [0, 255, 128]);
    }

    #[tokio::test]
    async fn terminal_reader_keeps_partial_frame_while_commands_arrive() {
        let (client, mut server) = tokio::io::duplex(256);
        let (reader, writer) = tokio::io::split(client);
        let (command_sender, command_receiver) = mpsc::channel(4);
        let (stream_sender, mut stream_receiver) = mpsc::channel(8);
        let task = tokio::spawn(run_terminal(
            reader,
            writer,
            command_receiver,
            stream_sender,
            TerminalContext {
                session_id: "session".to_owned(),
                pane_id: "pane".to_owned(),
                stream_id: "stream".to_owned(),
                mode: TerminalMode::Control,
            },
        ));

        assert!(matches!(
            next_message(&mut stream_receiver).await,
            TerminalStreamMessage::Ownership {
                state: TerminalOwnershipState::Pending,
                ..
            }
        ));
        server
            .write_all(&terminal_frame(1, true, b"one"))
            .await
            .unwrap();
        assert!(matches!(
            next_message(&mut stream_receiver).await,
            TerminalStreamMessage::Ownership {
                state: TerminalOwnershipState::Owned,
                ..
            }
        ));
        assert!(matches!(
            next_message(&mut stream_receiver).await,
            TerminalStreamMessage::Frame { seq, .. } if seq == "1"
        ));

        let second = terminal_frame(2, false, b"two");
        server.write_all(&second[..2]).await.unwrap();
        tokio::task::yield_now().await;
        command_sender
            .send(TerminalCommand::input_text("input"))
            .await
            .unwrap();
        assert_eq!(
            read_client_payload(&mut server).await,
            [1, 5, b'i', b'n', b'p', b'u', b't']
        );

        server.write_all(&second[2..6]).await.unwrap();
        tokio::task::yield_now().await;
        command_sender
            .send(TerminalCommand::Resize {
                cols: 100,
                rows: 30,
                cell_width_px: 8,
                cell_height_px: 16,
            })
            .await
            .unwrap();
        command_sender
            .send(TerminalCommand::Scroll {
                direction: TerminalScrollDirection::Down,
                lines: 2,
                source: TerminalScrollSource::Wheel,
                column: None,
                row: None,
                modifiers: 0,
            })
            .await
            .unwrap();
        assert_eq!(
            read_client_payload(&mut server).await,
            [3, 100, 30, 8, 16, 0],
        );
        assert_eq!(
            read_client_payload(&mut server).await,
            [6, 0, 1, 2, 0, 0, 0]
        );

        server.write_all(&second[6..]).await.unwrap();
        server.write_all(&framed(vec![8, 1, 0])).await.unwrap();
        server
            .write_all(&framed(vec![2, 3, 1, 2, 3]))
            .await
            .unwrap();
        server
            .write_all(&terminal_frame(3, false, b"three"))
            .await
            .unwrap();
        assert!(matches!(
            next_message(&mut stream_receiver).await,
            TerminalStreamMessage::Frame { seq, .. } if seq == "2"
        ));
        assert!(matches!(
            next_message(&mut stream_receiver).await,
            TerminalStreamMessage::MouseMode { enabled: true, .. }
        ));
        assert!(matches!(
            next_message(&mut stream_receiver).await,
            TerminalStreamMessage::Frame { seq, .. } if seq == "3"
        ));

        command_sender.send(TerminalCommand::Release).await.unwrap();
        assert!(matches!(
            next_message(&mut stream_receiver).await,
            TerminalStreamMessage::Ownership {
                state: TerminalOwnershipState::Released,
                ..
            }
        ));
        timeout(Duration::from_secs(1), task)
            .await
            .expect("terminal reader was not released")
            .unwrap();
    }

    #[tokio::test]
    async fn terminal_open_times_out_when_the_handshake_never_replies() {
        let path = std::env::temp_dir().join(format!(
            "cockpit-terminal-handshake-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
        ));
        let listener = UnixListener::bind(&path).expect("listener");
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.expect("accept");
            tokio::time::sleep(TERMINAL_HANDSHAKE_TIMEOUT + Duration::from_millis(100)).await;
        });
        let request = TerminalOpenRequest {
            session_id: "session".to_owned(),
            pane_id: "pane".to_owned(),
            mode: TerminalMode::Observe,
            takeover: false,
            cols: 80,
            rows: 24,
            cell_width_px: 8,
            cell_height_px: 16,
        };
        assert_eq!(
            open_terminal(&path, &request, "stream".to_owned())
                .await
                .expect_err("timeout")
                .code,
            "terminal_attach_timeout"
        );
        server.abort();
        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn auxiliary_messages_are_bounded_between_terminal_frames() {
        let mut stream = terminal_frame(1, true, b"one");
        stream.extend(framed(vec![8, 1, 0]));
        stream.extend(framed(vec![2, 3, 1, 2, 3]));
        stream.extend(terminal_frame(2, false, b"two"));
        let mut reader = stream.as_slice();

        assert!(matches!(
            read_message(&mut reader).await.unwrap(),
            ServerMessage::Terminal(TerminalFrame { seq: 1, .. })
        ));
        assert!(matches!(
            read_message(&mut reader).await.unwrap(),
            ServerMessage::MouseCapture { enabled: true }
        ));
        assert!(matches!(
            read_message(&mut reader).await.unwrap(),
            ServerMessage::Parked
        ));
        assert!(matches!(
            read_message(&mut reader).await.unwrap(),
            ServerMessage::Terminal(TerminalFrame { seq: 2, .. })
        ));
    }

    #[tokio::test]
    async fn graphics_frame_uses_32_mib_ceiling_without_materializing_bytes() {
        let (mut writer, mut reader) = tokio::io::duplex(32 * 1024);
        let writer_task = tokio::spawn(async move {
            let graphics_len = MAX_GRAPHICS_FRAME_SIZE - 6;
            writer
                .write_all(&(MAX_GRAPHICS_FRAME_SIZE as u32).to_le_bytes())
                .await
                .unwrap();
            writer
                .write_all(&[
                    2,
                    252,
                    graphics_len as u8,
                    (graphics_len >> 8) as u8,
                    (graphics_len >> 16) as u8,
                    (graphics_len >> 24) as u8,
                ])
                .await
                .unwrap();
            let zeros = [0; 8192];
            let mut remaining = graphics_len;
            while remaining > 0 {
                let count = remaining.min(zeros.len());
                writer.write_all(&zeros[..count]).await.unwrap();
                remaining -= count;
            }
        });

        assert!(matches!(
            read_message(&mut reader).await.unwrap(),
            ServerMessage::Parked
        ));
        writer_task.await.unwrap();
    }

    #[tokio::test]
    async fn decoder_rejects_malformed_graphics_and_oversized_normal_frames() {
        let malformed_graphics = framed(vec![2, 3, 1, 2]);
        assert!(
            read_message(&mut malformed_graphics.as_slice())
                .await
                .is_err()
        );

        let mut oversized_terminal = Vec::from(((MAX_NORMAL_FRAME_SIZE + 1) as u32).to_le_bytes());
        oversized_terminal.push(1);
        assert!(
            read_message(&mut oversized_terminal.as_slice())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn decoder_rejects_trailing_terminal_frame_data() {
        let mut frame = Vec::from((10_u32).to_le_bytes());
        frame.extend([1, 1, 80, 24, 1, 1, b'x', 9, 9, 9]);
        assert!(read_message(&mut frame.as_slice()).await.is_err());
    }
}
