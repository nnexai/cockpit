use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use cockpit_core::{InspectionError, TerminalSession};
use cockpit_protocol::v1::{
    TerminalCommand, TerminalMode, TerminalMouseButton, TerminalMouseKind, TerminalOpenRequest,
    TerminalOwnershipState, TerminalScrollDirection, TerminalStreamMessage,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, mpsc};

const ENDPOINT_GENERATION: u32 = 1;
const ENDPOINT_HELLO_KIND: &str = "endpoint.hello.v1";
const ENDPOINT_WELCOME_KIND: &str = "endpoint.welcome.v1";
const SNAPSHOT_CODEC: &str = "shell.snapshot.v1";
const SURFACE_CODEC: &str = "shell.surface.v1";
const INPUT_CODEC: &str = "shell.input.semantic.v1";
const BLOB_CODEC: &str = "shell.blob.v1";
const MAX_FRAME_SIZE: usize = 32 * 1024 * 1024;
const MAX_SURFACE_DIMENSION: u16 = 4096;
const MAX_SURFACE_CELLS: usize = 1_000_000;
const MAX_GRAPHICS_CHUNK: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EndpointKey {
    session_id: String,
    client_surface_id: String,
}

#[derive(Debug, Clone)]
struct Subscriber {
    session_id: String,
    pane_id: String,
    stream_id: String,
    mode: TerminalMode,
    sender: mpsc::Sender<TerminalStreamMessage>,
    seq: u64,
}
enum ActorCommand {
    Register {
        pane_id: String,
        stream_id: String,
        mode: TerminalMode,
        sender: mpsc::Sender<TerminalStreamMessage>,
    },
    Pane {
        pane_id: String,
        command: TerminalCommand,
    },
    Unregister {
        pane_id: String,
        stream_id: String,
    },
}

/// Registry of one stable ClientShell endpoint per browser/Tauri surface and session.
#[derive(Clone, Debug, Default)]
pub(crate) struct EndpointRegistry {
    actors: Arc<Mutex<HashMap<EndpointKey, mpsc::Sender<ActorCommand>>>>,
}

impl EndpointRegistry {
    pub(crate) async fn open(
        &self,
        socket_path: &Path,
        request: &TerminalOpenRequest,
        stream_id: String,
    ) -> Result<TerminalSession, InspectionError> {
        request
            .validate()
            .map_err(|message| InspectionError::new("invalid_terminal_dimensions", message))?;
        if request.client_surface_id.is_empty() || request.client_surface_id.len() > 128 {
            return Err(InspectionError::new(
                "invalid_client_surface_id",
                "client surface ID must be non-empty and at most 128 bytes",
            ));
        }
        if request.surface_cols > MAX_SURFACE_DIMENSION
            || request.surface_rows > MAX_SURFACE_DIMENSION
            || usize::from(request.surface_cols) * usize::from(request.surface_rows)
                > MAX_SURFACE_CELLS
        {
            return Err(InspectionError::new(
                "invalid_terminal_dimensions",
                "terminal surface exceeds the safe geometry limit",
            ));
        }

        let (messages, receiver) = mpsc::channel(64);
        let (commands, command_receiver) = mpsc::channel(32);
        let key = EndpointKey {
            session_id: request.session_id.clone(),
            client_surface_id: request.client_surface_id.clone(),
        };
        let actor_tx = {
            let mut actors = self.actors.lock().await;
            if let Some(existing) = actors.get(&key) {
                existing.clone()
            } else {
                let socket = connect_endpoint(socket_path, request).await?;
                let (tx, rx) = mpsc::channel(128);
                let context = ActorContext {
                    socket,
                    session_id: request.session_id.clone(),
                    surface_cols: request.surface_cols,
                    surface_rows: request.surface_rows,
                    cell_width_px: request.cell_width_px,
                    cell_height_px: request.cell_height_px,
                    actors: self.actors.clone(),
                    key: key.clone(),
                };
                actors.insert(key.clone(), tx.clone());
                tokio::spawn(run_actor(context, rx));
                tx
            }
        };
        let register = ActorCommand::Register {
            pane_id: request.pane_id.clone(),
            stream_id: stream_id.clone(),
            mode: request.mode,
            sender: messages,
        };
        if actor_tx.send(register).await.is_err() {
            self.actors.lock().await.remove(&key);
            return Err(InspectionError::new(
                "terminal_attach_failed",
                "shared Herdr endpoint is unavailable",
            ));
        }
        let tx = self.actors.lock().await.get(&key).cloned().ok_or_else(|| {
            InspectionError::new("terminal_attach_failed", "endpoint disappeared")
        })?;
        let command_tx = commands;
        let unregister_pane_id = request.pane_id.clone();
        let unregister_stream_id = stream_id.clone();
        tokio::spawn(async move {
            let mut incoming = command_receiver;
            while let Some(command) = incoming.recv().await {
                if tx
                    .send(ActorCommand::Pane {
                        pane_id: unregister_pane_id.clone(),
                        command,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let _ = tx
                .send(ActorCommand::Unregister {
                    pane_id: unregister_pane_id,
                    stream_id: unregister_stream_id,
                })
                .await;
        });
        Ok(TerminalSession {
            stream_id,
            messages: receiver,
            commands: command_tx,
        })
    }
}

struct ActorContext {
    socket: UnixStream,
    session_id: String,
    surface_cols: u16,
    surface_rows: u16,
    cell_width_px: u32,
    cell_height_px: u32,
    actors: Arc<Mutex<HashMap<EndpointKey, mpsc::Sender<ActorCommand>>>>,
    key: EndpointKey,
}

#[derive(Debug, Clone, Deserialize)]
struct EndpointWelcome {
    generation: u32,
    snapshot_codec: String,
    surface_codec: String,
    input_codec: String,
    blob_codec: String,
    #[serde(default)]
    error: Option<EndpointError>,
}

#[derive(Debug, Clone, Deserialize)]
struct EndpointError {
    code: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EndpointHello {
    generation: u32,
    cell_width_px: u32,
    cell_height_px: u32,
    surface_size: SurfaceSize,
    pixel_mouse: bool,
    direct_graphics: bool,
    endpoint_keybindings: bool,
    mouse_capture: bool,
    snapshot_codecs: Vec<String>,
    surface_codecs: Vec<String>,
    input_codecs: Vec<String>,
    blob_codecs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct SurfaceSize {
    cols: u16,
    rows: u16,
}

async fn connect_endpoint(
    socket_path: &Path,
    request: &TerminalOpenRequest,
) -> Result<UnixStream, InspectionError> {
    let mut socket = UnixStream::connect(socket_path).await.map_err(|error| {
        InspectionError::new(
            "terminal_attach_failed",
            format!("Herdr endpoint connection failed: {error}"),
        )
    })?;
    let hello = EndpointHello {
        generation: ENDPOINT_GENERATION,
        cell_width_px: request.cell_width_px,
        cell_height_px: request.cell_height_px,
        surface_size: SurfaceSize {
            cols: request.surface_cols,
            rows: request.surface_rows,
        },
        pixel_mouse: request.cell_width_px != 0 && request.cell_height_px != 0,
        direct_graphics: false,
        endpoint_keybindings: false,
        mouse_capture: false,
        snapshot_codecs: vec![SNAPSHOT_CODEC.into()],
        surface_codecs: vec![SURFACE_CODEC.into()],
        input_codecs: vec![INPUT_CODEC.into()],
        blob_codecs: vec![BLOB_CODEC.into()],
    };
    let data = serde_json::to_string(&hello)
        .map_err(|error| InspectionError::new("terminal_attach_failed", error.to_string()))?;
    write_control(&mut socket, ENDPOINT_HELLO_KIND, &data)
        .await
        .map_err(handshake_error)?;
    let payload = read_frame(&mut socket).await.map_err(handshake_error)?;
    let (tag, used) = decode_tag(&payload).map_err(handshake_message)?;
    if tag != 20 {
        return Err(handshake_message(
            "Herdr endpoint welcome had an unexpected tag",
        ));
    }
    let (kind, data): (String, String) =
        decode_payload(&payload[used..]).map_err(handshake_message)?;
    if kind != ENDPOINT_WELCOME_KIND {
        return Err(handshake_message(
            "Herdr endpoint welcome had an unexpected kind",
        ));
    }
    let welcome: EndpointWelcome = serde_json::from_str(&data)
        .map_err(|error| handshake_message(format!("invalid Herdr endpoint welcome: {error}")))?;
    if let Some(error) = welcome.error {
        return Err(InspectionError::new(
            "terminal_attach_failed",
            bounded(&format!("{}: {}", error.code, error.message)),
        ));
    }
    if welcome.generation != ENDPOINT_GENERATION
        || welcome.snapshot_codec != SNAPSHOT_CODEC
        || welcome.surface_codec != SURFACE_CODEC
        || welcome.input_codec != INPUT_CODEC
        || welcome.blob_codec != BLOB_CODEC
    {
        return Err(handshake_message("Herdr endpoint codecs are incompatible"));
    }
    Ok(socket)
}

fn handshake_error(_: io::Error) -> InspectionError {
    handshake_message("Herdr endpoint protocol negotiation failed")
}

fn handshake_message(message: impl Into<String>) -> InspectionError {
    InspectionError::new("terminal_attach_failed", message)
}

async fn run_actor(mut context: ActorContext, mut commands: mpsc::Receiver<ActorCommand>) {
    let mut subscribers: HashMap<String, Subscriber> = HashMap::new();
    let mut cache: Option<SurfaceState> = None;
    loop {
        if subscribers.is_empty() {
            match commands.recv().await {
                Some(ActorCommand::Register {
                    pane_id,
                    stream_id,
                    mode,
                    sender,
                }) => {
                    let subscriber = Subscriber {
                        session_id: context.session_id.clone(),
                        pane_id: pane_id.clone(),
                        stream_id,
                        mode,
                        sender,
                        seq: 0,
                    };
                    send_ownership(
                        &subscriber,
                        if mode == TerminalMode::Observe {
                            TerminalOwnershipState::Observing
                        } else {
                            TerminalOwnershipState::Owned
                        },
                        None,
                    )
                    .await;
                    subscribers.insert(pane_id, subscriber);
                }
                Some(ActorCommand::Unregister { .. }) => {}
                Some(ActorCommand::Pane { .. }) => {}
                None => break,
            }
            continue;
        }
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(ActorCommand::Register { pane_id, stream_id, mode, sender }) => {
                        let subscriber = Subscriber {
                            session_id: context.session_id.clone(),
                            pane_id: pane_id.clone(),
                            stream_id,
                            mode,
                            sender,
                            seq: 0,
                        };
                        send_ownership(&subscriber, if mode == TerminalMode::Observe { TerminalOwnershipState::Observing } else { TerminalOwnershipState::Owned }, None).await;
                        subscribers.insert(pane_id.clone(), subscriber);
                        if let Some(surface) = cache.as_ref() { send_pane_full(surface, subscribers.get_mut(&pane_id).unwrap()).await; }
                    }
                    Some(ActorCommand::Pane { pane_id, command }) => {
                        handle_command(&mut context, &mut subscribers, &pane_id, command).await;
                        if subscribers.is_empty() {
                            let _ = write_payload(&mut context.socket, &encode_tag(4)).await;
                            break;
                        }
                    }
                    Some(ActorCommand::Unregister { pane_id, stream_id }) => {
                        let should_remove = subscribers
                            .get(&pane_id)
                            .is_some_and(|subscriber| subscriber.stream_id == stream_id);
                        if should_remove {
                            subscribers.remove(&pane_id);
                        }
                        if subscribers.is_empty() {
                            let _ = write_payload(&mut context.socket, &encode_tag(4)).await;
                            break;
                        }
                    }
                    None => break,
                }
            }
            result = read_frame(&mut context.socket) => {
                match result {
                    Ok(payload) => match decode_server(&payload) {
                        Ok(DecodedServer::Surface(surface)) => {
                            if let Some(previous) = cache.as_ref()
                                && surface.surface_revision <= previous.revision
                            {
                                send_all_error(&mut subscribers, "surface_revision_error", "surface revision is not monotonic").await;
                                break;
                            }
                            if validate_surface(&surface).is_err() {
                                send_all_error(&mut subscribers, "malformed_surface", "Herdr pane surface is malformed").await;
                                break;
                            }
                            let retained_assets = cache.take().map(|previous| previous.assets).unwrap_or_default();
                            cache = Some(SurfaceState::from_surface(surface, retained_assets));
                            if let Some(surface) = cache.as_ref() { send_all_full(surface, &mut subscribers).await; }
                        }
                        Ok(DecodedServer::Patch(patch)) => {
                            let Some(surface) = cache.as_mut() else {
                                send_all_error(&mut subscribers, "surface_baseline_required", "pane surface patch arrived before a full surface").await;
                                break;
                            };
                            if patch.base_surface_revision != surface.revision || patch.surface_revision <= patch.base_surface_revision || apply_patch(surface, patch).is_err() {
                                send_all_error(&mut subscribers, "surface_patch_error", "pane surface patch revision or geometry is invalid").await;
                                break;
                            }
                            send_all_patch(surface, &mut subscribers).await;
                        }
                        Ok(DecodedServer::Shutdown(reason)) => {
                            let reason = bounded(&reason.unwrap_or_else(|| "closed".into()));
                            send_all_closed(&mut subscribers, reason).await;
                            break;
                        }
                        Ok(DecodedServer::Ignored) => {}
                        Err(_) => {
                            send_all_error(&mut subscribers, "malformed_endpoint_message", "Herdr endpoint message is malformed").await;
                            break;
                        }
                    },
                    Err(_) => {
                        send_all_disconnected(&mut subscribers).await;
                        break;
                    }
                }
            }
        }
        subscribers.retain(|_, sub| !sub.sender.is_closed());
    }
    context.actors.lock().await.remove(&context.key);
}

async fn handle_command(
    context: &mut ActorContext,
    subscribers: &mut HashMap<String, Subscriber>,
    pane_id: &str,
    command: TerminalCommand,
) {
    let Some(subscriber) = subscribers.get(pane_id).cloned() else {
        return;
    };
    if command.validate().is_err() {
        send_error(
            &subscriber,
            "invalid_terminal_command",
            "terminal command is invalid",
        )
        .await;
        return;
    }
    if matches!(command, TerminalCommand::Release) {
        send_ownership(&subscriber, TerminalOwnershipState::Released, None).await;
        subscribers.remove(pane_id);
        return;
    }
    if subscriber.mode == TerminalMode::Observe
        && !matches!(&command, TerminalCommand::Resize { .. })
    {
        send_error(
            &subscriber,
            "terminal_command_rejected",
            "terminal observe stream is read-only",
        )
        .await;
        return;
    }
    let result = match command {
        TerminalCommand::Input {
            text: Some(text), ..
        } => encode_shell_input(pane_id, &ClientPaneInputEvent::TextCommit(text)),
        TerminalCommand::Input {
            bytes: Some(encoded),
            ..
        } => match base64::engine::general_purpose::STANDARD.decode(encoded) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => encode_shell_input(pane_id, &ClientPaneInputEvent::TextCommit(text)),
                Err(_) => Err("terminal input bytes are not valid UTF-8"),
            },
            Err(_) => Err("terminal input bytes are not valid base64"),
        },
        TerminalCommand::Resize {
            cols,
            rows,
            cell_width_px,
            cell_height_px,
        } => {
            if cols > MAX_SURFACE_DIMENSION
                || rows > MAX_SURFACE_DIMENSION
                || usize::from(cols) * usize::from(rows) > MAX_SURFACE_CELLS
            {
                Err("terminal surface exceeds the safe geometry limit")
            } else {
                context.surface_cols = cols;
                context.surface_rows = rows;
                context.cell_width_px = cell_width_px;
                context.cell_height_px = cell_height_px;
                encode_resize(context)
            }
        }
        TerminalCommand::Scroll {
            direction,
            lines,
            column,
            row,
            modifiers,
            ..
        } => encode_shell_input(
            pane_id,
            &ClientPaneInputEvent::Mouse {
                kind: match direction {
                    TerminalScrollDirection::Up => ClientMouseKind::ScrollUp,
                    TerminalScrollDirection::Down => ClientMouseKind::ScrollDown,
                },
                position: ClientMousePosition::Cell {
                    column: column.unwrap_or(0),
                    row: row.unwrap_or(0),
                },
                geometry: None,
                modifiers,
                lines: u16::try_from(lines).unwrap_or(u16::MAX),
            },
        ),
        TerminalCommand::Mouse {
            kind,
            button,
            column,
            row,
            modifiers,
        } => match client_mouse_kind(kind, button) {
            Ok(kind) => encode_shell_input(
                pane_id,
                &ClientPaneInputEvent::Mouse {
                    kind,
                    position: ClientMousePosition::Cell { column, row },
                    geometry: None,
                    modifiers,
                    lines: 0,
                },
            ),
            Err(error) => Err(error),
        },
        TerminalCommand::Input { .. } | TerminalCommand::Release => {
            Err("terminal input is invalid")
        }
    };
    match result {
        Ok(payload) => {
            if write_payload(&mut context.socket, &payload).await.is_err() {
                send_error(
                    &subscriber,
                    "terminal_disconnected",
                    "Herdr endpoint write failed",
                )
                .await;
            }
        }
        Err(error) => {
            send_error(&subscriber, "invalid_terminal_command", error).await;
        }
    }
}

fn encode_resize(context: &ActorContext) -> Result<Vec<u8>, &'static str> {
    let mut payload = encode_tag(12);
    payload.extend(encode_fields(&[
        bincode::serde::encode_to_vec(context.cell_width_px, bincode::config::standard())
            .map_err(|_| "resize encoding failed")?,
        bincode::serde::encode_to_vec(context.cell_height_px, bincode::config::standard())
            .map_err(|_| "resize encoding failed")?,
        bincode::serde::encode_to_vec(
            SurfaceSize {
                cols: context.surface_cols,
                rows: context.surface_rows,
            },
            bincode::config::standard(),
        )
        .map_err(|_| "resize encoding failed")?,
        bincode::serde::encode_to_vec(
            context.cell_width_px != 0 && context.cell_height_px != 0,
            bincode::config::standard(),
        )
        .map_err(|_| "resize encoding failed")?,
    ]));
    Ok(payload)
}

fn encode_shell_input(
    pane_id: &str,
    event: &ClientPaneInputEvent,
) -> Result<Vec<u8>, &'static str> {
    let mut payload = encode_tag(13);
    payload.extend(encode_fields(&[
        bincode::serde::encode_to_vec(pane_id, bincode::config::standard())
            .map_err(|_| "input encoding failed")?,
        bincode::serde::encode_to_vec(vec![event], bincode::config::standard())
            .map_err(|_| "input encoding failed")?,
    ]));
    Ok(payload)
}

fn encode_tag(tag: u32) -> Vec<u8> {
    bincode::serde::encode_to_vec(tag, bincode::config::standard()).unwrap_or_default()
}

fn encode_fields(fields: &[Vec<u8>]) -> Vec<u8> {
    fields
        .iter()
        .flat_map(|field| field.iter().copied())
        .collect()
}

async fn write_control(socket: &mut UnixStream, kind: &str, data: &str) -> io::Result<()> {
    let mut payload = encode_tag(20);
    let fields = [
        bincode::serde::encode_to_vec(kind, bincode::config::standard())
            .map_err(io::Error::other)?,
        bincode::serde::encode_to_vec(data, bincode::config::standard())
            .map_err(io::Error::other)?,
    ];
    payload.extend(encode_fields(&fields));
    write_payload(socket, &payload).await
}

async fn write_payload<W: AsyncWrite + Unpin>(writer: &mut W, payload: &[u8]) -> io::Result<()> {
    let size = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Herdr frame is too large"))?;
    writer.write_all(&size.to_le_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Vec<u8>> {
    let size = reader.read_u32_le().await? as usize;
    if size == 0 || size > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Herdr frame size is invalid",
        ));
    }
    let mut payload = vec![0; size];
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}

fn decode_tag(payload: &[u8]) -> Result<(u32, usize), String> {
    bincode::serde::decode_from_slice(payload, bincode::config::standard())
        .map_err(|error| error.to_string())
}

fn decode_payload<T: for<'de> Deserialize<'de>>(payload: &[u8]) -> Result<T, String> {
    let (value, consumed) = bincode::serde::decode_from_slice(payload, bincode::config::standard())
        .map_err(|error| error.to_string())?;
    if consumed != payload.len() {
        return Err("Herdr message has trailing data".into());
    }
    Ok(value)
}

#[derive(Debug)]
enum DecodedServer {
    Surface(PaneSurfaceFrame),
    Patch(PaneSurfacePatch),
    Shutdown(Option<String>),
    Ignored,
}

fn decode_server(payload: &[u8]) -> Result<DecodedServer, String> {
    let (tag, used) = decode_tag(payload)?;
    let rest = &payload[used..];
    match tag {
        3 => Ok(DecodedServer::Shutdown(decode_payload(rest)?)),
        13 => Ok(DecodedServer::Surface(decode_payload(rest)?)),
        19 => Ok(DecodedServer::Patch(decode_payload(rest)?)),
        _ => Ok(DecodedServer::Ignored),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CellData {
    symbol: String,
    fg: u32,
    bg: u32,
    modifier: u16,
    skip: bool,
    hyperlink: Option<u32>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct CursorState {
    x: u16,
    y: u16,
    visible: bool,
    shape: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FrameData {
    cells: Vec<CellData>,
    width: u16,
    height: u16,
    cursor: Option<CursorState>,
    hyperlinks: Vec<String>,
    graphics: Vec<u8>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct SurfaceRect {
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaneSurfacePane {
    pane_id: String,
    content_revision: u64,
    rect: SurfaceRect,
    inner_rect: SurfaceRect,
    scrollbar_rect: Option<SurfaceRect>,
    scroll: Option<ScrollMetrics>,
    focused: bool,
    mouse_reporting: bool,
    sgr_pixel_mouse: bool,
    alternate_screen_active: bool,
    pixel_width: u32,
    pixel_height: u32,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct ScrollMetrics {
    offset_from_bottom: u64,
    max_offset_from_bottom: u64,
    viewport_rows: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
enum PaneSurfaceSplitDirection {
    Horizontal,
    Vertical,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaneSurfaceSplit {
    direction: PaneSurfaceSplitDirection,
    pos: u16,
    area: SurfaceRect,
    hit_rect: SurfaceRect,
    path: Vec<bool>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum ClientShellPopupSize {
    Cells(u16),
    Percent(u8),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientShellPopupSurface {
    terminal_id: String,
    title: String,
    width: Option<ClientShellPopupSize>,
    height: Option<ClientShellPopupSize>,
    frame: FrameData,
    mouse_reporting: bool,
    sgr_pixel_mouse: bool,
    pixel_width: u32,
    pixel_height: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaneSurfaceFrame {
    boot_id: String,
    projection_revision: u64,
    surface_revision: u64,
    frame: FrameData,
    panes: Vec<PaneSurfacePane>,
    splits: Vec<PaneSurfaceSplit>,
    popup: Option<Box<ClientShellPopupSurface>>,
    graphics: SurfaceGraphicsScene,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaneSurfacePatch {
    boot_id: String,
    projection_revision: u64,
    base_surface_revision: u64,
    surface_revision: u64,
    rows: Vec<PaneSurfacePatchRow>,
    panes: Vec<PaneSurfacePane>,
    cursor: Option<CursorState>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaneSurfacePatchRow {
    x: u16,
    y: u16,
    cells: Vec<CellData>,
}
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
enum SurfaceGraphicsTarget {
    Pane { pane_id: String },
    Popup { terminal_id: String },
}
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
enum SurfaceGraphicsSource {
    Terminal {
        target: SurfaceGraphicsTarget,
        image_id: u32,
    },
    PaneLayer {
        pane_id: String,
        layer_id: String,
    },
}
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
enum SurfaceGraphicsFormat {
    Rgb,
    Rgba,
    Png,
}
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
struct SurfaceGraphicsAssetKey {
    source: SurfaceGraphicsSource,
    image_width: u32,
    image_height: u32,
    format: SurfaceGraphicsFormat,
    data_len: u64,
    data_fingerprint: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SurfaceGraphicsAsset {
    key: SurfaceGraphicsAssetKey,
    data: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SurfaceGraphicsPlacement {
    asset: SurfaceGraphicsAssetKey,
    logical_placement_id: u32,
    x: u16,
    y: u16,
    cols: u32,
    rows: u32,
    source_x: u32,
    source_y: u32,
    source_width: u32,
    source_height: u32,
    x_offset: u32,
    y_offset: u32,
    z: i32,
    scrollback_offset: u32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SurfaceGraphicsScene {
    assets: Vec<SurfaceGraphicsAsset>,
    placements: Vec<SurfaceGraphicsPlacement>,
    retained_assets: Vec<SurfaceGraphicsAssetKey>,
}

#[derive(Debug, Clone)]
struct SurfaceState {
    frame: FrameData,
    panes: Vec<PaneSurfacePane>,
    graphics: SurfaceGraphicsScene,
    assets: HashMap<SurfaceGraphicsAssetKey, Vec<u8>>,
    revision: u64,
    boot_id: String,
    projection_revision: u64,
}

impl SurfaceState {
    fn from_surface(
        surface: PaneSurfaceFrame,
        mut assets: HashMap<SurfaceGraphicsAssetKey, Vec<u8>>,
    ) -> Self {
        let PaneSurfaceFrame {
            boot_id,
            projection_revision,
            surface_revision,
            frame,
            panes,
            splits: _,
            popup: _,
            mut graphics,
        } = surface;
        for asset in graphics.assets.drain(..) {
            assets.insert(asset.key, asset.data);
        }
        assets.retain(|key, _| {
            graphics.retained_assets.contains(key)
                || graphics
                    .placements
                    .iter()
                    .any(|placement| &placement.asset == key)
        });
        Self {
            frame,
            panes,
            graphics,
            assets,
            revision: surface_revision,
            boot_id,
            projection_revision,
        }
    }
}

fn validate_surface(surface: &PaneSurfaceFrame) -> Result<(), ()> {
    let cols = surface.frame.width;
    let rows = surface.frame.height;
    if cols == 0
        || rows == 0
        || cols > MAX_SURFACE_DIMENSION
        || rows > MAX_SURFACE_DIMENSION
        || usize::from(cols) * usize::from(rows) > MAX_SURFACE_CELLS
        || surface.frame.cells.len() != usize::from(cols) * usize::from(rows)
    {
        return Err(());
    }
    for pane in &surface.panes {
        if !rect_fits(pane.inner_rect, surface.frame.width, surface.frame.height)
            || pane.inner_rect.width == 0
            || pane.inner_rect.height == 0
        {
            return Err(());
        }
    }
    for asset in &surface.graphics.assets {
        if asset.data.len() > MAX_FRAME_SIZE
            || u64::try_from(asset.data.len()).ok() != Some(asset.key.data_len)
        {
            return Err(());
        }
    }
    Ok(())
}
fn rect_fits(rect: SurfaceRect, width: u16, height: u16) -> bool {
    rect.x.checked_add(rect.width).is_some_and(|x| x <= width)
        && rect.y.checked_add(rect.height).is_some_and(|y| y <= height)
}
fn apply_patch(surface: &mut SurfaceState, patch: PaneSurfacePatch) -> Result<(), ()> {
    if patch.boot_id != surface.boot_id
        || patch.projection_revision != surface.projection_revision
        || patch.base_surface_revision != surface.revision
        || patch.surface_revision != patch.base_surface_revision + 1
    {
        return Err(());
    }
    for row in &patch.rows {
        let end = row
            .x
            .checked_add(u16::try_from(row.cells.len()).map_err(|_| ())?)
            .ok_or(())?;
        if row.y >= surface.frame.height || end > surface.frame.width {
            return Err(());
        }
        let offset = usize::from(row.y) * usize::from(surface.frame.width) + usize::from(row.x);
        surface.frame.cells[offset..offset + row.cells.len()].clone_from_slice(&row.cells);
    }
    for pane in patch.panes {
        if let Some(existing) = surface
            .panes
            .iter_mut()
            .find(|item| item.pane_id == pane.pane_id)
        {
            *existing = pane;
        } else {
            return Err(());
        }
    }
    surface.frame.cursor = patch.cursor;
    surface.revision = patch.surface_revision;
    Ok(())
}

async fn send_all_full(surface: &SurfaceState, subscribers: &mut HashMap<String, Subscriber>) {
    for subscriber in subscribers.values_mut() {
        send_pane_full(surface, subscriber).await;
    }
}
async fn send_pane_full(surface: &SurfaceState, subscriber: &mut Subscriber) {
    let Some(pane) = surface
        .panes
        .iter()
        .find(|pane| pane.pane_id == subscriber.pane_id)
    else {
        send_error(
            subscriber,
            "pane_not_found",
            "pane is not present in the selected surface",
        )
        .await;
        return;
    };
    let bytes = encode_text(surface, pane, None);
    let bytes = base64::engine::general_purpose::STANDARD.encode(bytes);
    subscriber.seq = subscriber.seq.saturating_add(1);
    let message = TerminalStreamMessage::Frame {
        session_id: subscriber.session_id.clone(),
        pane_id: subscriber.pane_id.clone(),
        stream_id: subscriber.stream_id.clone(),
        seq: subscriber.seq.to_string(),
        encoding: "ansi".into(),
        width: pane.inner_rect.width,
        height: pane.inner_rect.height,
        full: true,
        bytes,
    };
    let _ = subscriber.sender.send(message).await;
    send_graphics(surface, pane, subscriber).await;
}
async fn send_all_patch(surface: &SurfaceState, subscribers: &mut HashMap<String, Subscriber>) {
    for subscriber in subscribers.values_mut() {
        if let Some(pane) = surface
            .panes
            .iter()
            .find(|pane| pane.pane_id == subscriber.pane_id)
        {
            send_pane_patch(surface, pane, subscriber).await;
        }
    }
}
async fn send_pane_patch(
    surface: &SurfaceState,
    pane: &PaneSurfacePane,
    subscriber: &mut Subscriber,
) {
    let bytes = encode_text(surface, pane, None);
    subscriber.seq = subscriber.seq.saturating_add(1);
    let message = TerminalStreamMessage::Frame {
        session_id: subscriber.session_id.clone(),
        pane_id: subscriber.pane_id.clone(),
        stream_id: subscriber.stream_id.clone(),
        seq: subscriber.seq.to_string(),
        encoding: "ansi".into(),
        width: pane.inner_rect.width,
        height: pane.inner_rect.height,
        full: true,
        bytes: base64::engine::general_purpose::STANDARD.encode(bytes),
    };
    let _ = subscriber.sender.send(message).await;
    send_graphics(surface, pane, subscriber).await;
}
fn encode_text(
    surface: &SurfaceState,
    pane: &PaneSurfacePane,
    _rows: Option<&[PaneSurfacePatchRow]>,
) -> Vec<u8> {
    let mut output = b"\x1b[?25l\x1b[2J\x1b[H".to_vec();
    let mut last_style = None;
    let mut active_hyperlink: Option<u32> = None;
    for y in 0..pane.inner_rect.height {
        output.extend_from_slice(format!("\x1b[{};1H", y + 1).as_bytes());
        for x in 0..pane.inner_rect.width {
            let index = usize::from(pane.inner_rect.y + y) * usize::from(surface.frame.width)
                + usize::from(pane.inner_rect.x + x);
            let cell = &surface.frame.cells[index];
            if cell.skip {
                continue;
            }
            let style = (cell.fg, cell.bg, cell.modifier);
            if last_style != Some(style) {
                output.extend_from_slice(build_sgr(cell.fg, cell.bg, cell.modifier).as_bytes());
                last_style = Some(style);
            }
            if active_hyperlink != cell.hyperlink {
                output.extend_from_slice(b"\x1b]8;;\x1b\\");
                if let Some(uri) = cell
                    .hyperlink
                    .and_then(|link| surface.frame.hyperlinks.get(link as usize))
                {
                    output.extend_from_slice(b"\x1b]8;;");
                    output.extend_from_slice(uri.as_bytes());
                    output.extend_from_slice(b"\x1b\\");
                }
                active_hyperlink = cell.hyperlink;
            }
            if cell.symbol.is_empty() {
                output.push(b' ');
            } else {
                output.extend_from_slice(cell.symbol.as_bytes());
            }
        }
    }
    if active_hyperlink.is_some() {
        output.extend_from_slice(b"\x1b]8;;\x1b\\");
    }
    output.extend_from_slice(b"\x1b[0m");
    if let Some(cursor) = surface.frame.cursor
        && cursor.x >= pane.inner_rect.x
        && cursor.x < pane.inner_rect.x + pane.inner_rect.width
        && cursor.y >= pane.inner_rect.y
        && cursor.y < pane.inner_rect.y + pane.inner_rect.height
    {
        output.extend_from_slice(
            format!(
                "\x1b[{};{}H\x1b[{} q\x1b[?25{}",
                cursor.y - pane.inner_rect.y + 1,
                cursor.x - pane.inner_rect.x + 1,
                cursor.shape.min(6),
                if cursor.visible { "h" } else { "l" },
            )
            .as_bytes(),
        );
    }
    output
}

fn color_to_sgr(value: u32, foreground: bool) -> String {
    let reset = if foreground { 39 } else { 49 };
    let base = if foreground { 30 } else { 40 };
    match value >> 24 {
        0x00 => match value & 0xff {
            0 => reset.to_string(),
            1..=8 => (base + (value & 0xff) - 1).to_string(),
            9..=16 => (base + 60 + (value & 0xff) - 9).to_string(),
            _ => reset.to_string(),
        },
        0x01 => format!("{};5;{}", if foreground { 38 } else { 48 }, value & 0xff),
        0x02 => format!(
            "{};2;{};{};{}",
            if foreground { 38 } else { 48 },
            (value >> 16) & 0xff,
            (value >> 8) & 0xff,
            value & 0xff,
        ),
        _ => reset.to_string(),
    }
}

fn build_sgr(fg: u32, bg: u32, modifier: u16) -> String {
    let mut parts = vec!["0".to_owned()];
    for (mask, code) in [
        (1 << 0, "1"),
        (1 << 1, "2"),
        (1 << 2, "3"),
        (1 << 4, "5"),
        (1 << 5, "6"),
        (1 << 6, "7"),
        (1 << 7, "8"),
        (1 << 8, "9"),
    ] {
        if modifier & mask != 0 {
            parts.push(code.to_owned());
        }
    }
    if modifier & (1 << 3) != 0 {
        parts.push(
            match (modifier & 0xf000) >> 12 {
                2 => "4:2",
                3 => "4:3",
                4 => "4:4",
                5 => "4:5",
                _ => "4",
            }
            .to_owned(),
        );
    }
    parts.push(color_to_sgr(fg, true));
    parts.push(color_to_sgr(bg, false));
    format!("\x1b[{}m", parts.join(";"))
}
async fn send_graphics(
    surface: &SurfaceState,
    pane: &PaneSurfacePane,
    subscriber: &mut Subscriber,
) {
    let bytes = match encode_graphics(
        &surface.graphics,
        &surface.assets,
        &pane.pane_id,
        pane.inner_rect,
    ) {
        Ok(bytes) => bytes,
        Err(message) => {
            send_error(subscriber, "graphics_asset_missing", message).await;
            return;
        }
    };
    let message = TerminalStreamMessage::Graphics {
        session_id: subscriber.session_id.clone(),
        pane_id: subscriber.pane_id.clone(),
        stream_id: subscriber.stream_id.clone(),
        revision: surface.revision.to_string(),
        bytes: base64::engine::general_purpose::STANDARD.encode(bytes),
    };
    let _ = subscriber.sender.send(message).await;
}
fn graphics_targets_pane(source: &SurfaceGraphicsSource, pane_id: &str) -> bool {
    match source {
        SurfaceGraphicsSource::PaneLayer {
            pane_id: target, ..
        } => target == pane_id,
        SurfaceGraphicsSource::Terminal {
            target: SurfaceGraphicsTarget::Pane { pane_id: target },
            ..
        } => target == pane_id,
        _ => false,
    }
}
fn encode_graphics(
    scene: &SurfaceGraphicsScene,
    assets: &HashMap<SurfaceGraphicsAssetKey, Vec<u8>>,
    pane_id: &str,
    inner: SurfaceRect,
) -> Result<Vec<u8>, &'static str> {
    let mut output = b"\x1b_Ga=d,d=A,q=2\x1b\\".to_vec();
    let mut placements: Vec<_> = scene
        .placements
        .iter()
        .filter(|placement| graphics_targets_pane(&placement.asset.source, pane_id))
        .collect();
    placements.sort_by_key(|placement| placement.z);
    for (index, placement) in placements.into_iter().enumerate() {
        let data = assets
            .get(&placement.asset)
            .ok_or("Herdr graphics placement references an unavailable asset")?;
        let local_image_id =
            u32::try_from(index + 1).map_err(|_| "Herdr graphics scene has too many placements")?;
        let format = match placement.asset.format {
            SurfaceGraphicsFormat::Png => 100,
            SurfaceGraphicsFormat::Rgb => 24,
            SurfaceGraphicsFormat::Rgba => 32,
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        for (chunk_index, chunk) in encoded.as_bytes().chunks(MAX_GRAPHICS_CHUNK).enumerate() {
            let more = usize::from((chunk_index + 1) * MAX_GRAPHICS_CHUNK < encoded.len());
            if chunk_index == 0 {
                output.extend_from_slice(
                    format!(
                        "\x1b_Ga=t,t=d,f={format},s={},v={},i={local_image_id},q=2,m={more};",
                        placement.asset.image_width, placement.asset.image_height,
                    )
                    .as_bytes(),
                );
            } else {
                output.extend_from_slice(format!("\x1b_Gm={more};").as_bytes());
            }
            output.extend_from_slice(chunk);
            output.extend_from_slice(b"\x1b\\");
        }
        let column = placement.x.saturating_sub(inner.x) + 1;
        let row = placement.y.saturating_sub(inner.y) + 1;
        output.extend_from_slice(
            format!(
                "\x1b7\x1b[{row};{column}H\x1b_Ga=p,i={local_image_id},p={},x={},y={},w={},h={},X={},Y={},c={},r={},z={},C=1,q=2\x1b\\\x1b8",
                placement.logical_placement_id,
                placement.source_x,
                placement.source_y,
                placement.source_width,
                placement.source_height,
                placement.x_offset,
                placement.y_offset,
                placement.cols,
                placement.rows,
                placement.z,
            )
            .as_bytes(),
        );
    }
    Ok(output)
}

fn client_mouse_kind(
    kind: TerminalMouseKind,
    button: Option<TerminalMouseButton>,
) -> Result<ClientMouseKind, &'static str> {
    let button = || {
        Ok(
            match button.ok_or("terminal mouse button does not match event kind")? {
                TerminalMouseButton::Left => ClientMouseButton::Left,
                TerminalMouseButton::Right => ClientMouseButton::Right,
                TerminalMouseButton::Middle => ClientMouseButton::Middle,
            },
        )
    };
    match kind {
        TerminalMouseKind::Down => Ok(ClientMouseKind::Down(button()?)),
        TerminalMouseKind::Up => Ok(ClientMouseKind::Up(button()?)),
        TerminalMouseKind::Drag => Ok(ClientMouseKind::Drag(button()?)),
        TerminalMouseKind::Moved => Ok(ClientMouseKind::Moved),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ClientKeyCode {
    Null,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum ClientMouseButton {
    Left,
    Right,
    Middle,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum ClientMousePosition {
    Cell {
        column: u16,
        row: u16,
    },
    Pixels {
        x: u32,
        y: u32,
        column: u16,
        row: u16,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct ClientMouseGeometry {
    cols: u16,
    rows: u16,
    width_px: u32,
    height_px: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
enum ClientPaneInputEvent {
    Key {
        code: ClientKeyCode,
        modifiers: u8,
        kind: u8,
        repeat_count: u16,
        shifted_codepoint: Option<u32>,
        generated_text: Option<String>,
        tracks_release: bool,
        physical_key_id: Option<u32>,
        windows_record: Option<()>,
    },
    TextCommit(String),
    Mouse {
        kind: ClientMouseKind,
        position: ClientMousePosition,
        geometry: Option<ClientMouseGeometry>,
        modifiers: u8,
        lines: u16,
    },
    Paste(String),
}

async fn send_ownership(
    subscriber: &Subscriber,
    state: TerminalOwnershipState,
    message: Option<String>,
) {
    let _ = subscriber
        .sender
        .send(TerminalStreamMessage::Ownership {
            session_id: subscriber.session_id.clone(),
            pane_id: subscriber.pane_id.clone(),
            stream_id: subscriber.stream_id.clone(),
            state,
            message,
        })
        .await;
}
async fn send_error(subscriber: &Subscriber, code: &str, message: &str) {
    let _ = subscriber
        .sender
        .send(TerminalStreamMessage::Error {
            session_id: subscriber.session_id.clone(),
            pane_id: subscriber.pane_id.clone(),
            stream_id: subscriber.stream_id.clone(),
            code: code.into(),
            message: message.into(),
        })
        .await;
}
async fn send_all_error(subscribers: &mut HashMap<String, Subscriber>, code: &str, message: &str) {
    for subscriber in subscribers.values() {
        send_error(subscriber, code, message).await;
    }
}
async fn send_all_closed(subscribers: &mut HashMap<String, Subscriber>, reason: String) {
    for subscriber in subscribers.values() {
        let _ = subscriber
            .sender
            .send(TerminalStreamMessage::Closed {
                session_id: subscriber.session_id.clone(),
                pane_id: subscriber.pane_id.clone(),
                stream_id: subscriber.stream_id.clone(),
                reason: reason.clone(),
            })
            .await;
    }
}
async fn send_all_disconnected(subscribers: &mut HashMap<String, Subscriber>) {
    for subscriber in subscribers.values() {
        let _ = subscriber
            .sender
            .send(TerminalStreamMessage::Disconnected {
                session_id: subscriber.session_id.clone(),
                pane_id: subscriber.pane_id.clone(),
                stream_id: subscriber.stream_id.clone(),
                code: "terminal_disconnected".into(),
                message: "Herdr endpoint disconnected".into(),
            })
            .await;
    }
}
fn bounded(value: &str) -> String {
    value.chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_hello_uses_generation_one_and_all_codecs() {
        let hello = EndpointHello {
            generation: 1,
            cell_width_px: 8,
            cell_height_px: 16,
            surface_size: SurfaceSize { cols: 80, rows: 24 },
            pixel_mouse: true,
            direct_graphics: false,
            endpoint_keybindings: false,
            mouse_capture: false,
            snapshot_codecs: vec![SNAPSHOT_CODEC.into()],
            surface_codecs: vec![SURFACE_CODEC.into()],
            input_codecs: vec![INPUT_CODEC.into()],
            blob_codecs: vec![BLOB_CODEC.into()],
        };
        let value = serde_json::to_value(hello).unwrap();
        assert_eq!(value["generation"], 1);
        assert_eq!(value["direct_graphics"], false);
    }
    #[test]
    fn pane_input_text_has_frozen_client_shell_tag() {
        let payload =
            encode_shell_input("w1:p1", &ClientPaneInputEvent::TextCommit("hello".into())).unwrap();
        assert_eq!(decode_tag(&payload).unwrap().0, 13);
    }
    #[test]
    fn pane_surface_slices_inner_rect_without_leaking_neighbors() {
        let cells = ["a", "b", "c", "d", "e", "f"]
            .into_iter()
            .map(|symbol| CellData {
                symbol: symbol.into(),
                fg: 0,
                bg: 0,
                modifier: 0,
                skip: false,
                hyperlink: None,
            })
            .collect();
        let surface = SurfaceState {
            frame: FrameData {
                cells,
                width: 3,
                height: 2,
                cursor: None,
                hyperlinks: Vec::new(),
                graphics: Vec::new(),
            },
            panes: Vec::new(),
            graphics: SurfaceGraphicsScene::default(),
            assets: HashMap::new(),
            revision: 1,
            boot_id: "boot".into(),
            projection_revision: 1,
        };
        let pane = PaneSurfacePane {
            pane_id: "p1".into(),
            content_revision: 1,
            rect: SurfaceRect {
                x: 0,
                y: 0,
                width: 3,
                height: 2,
            },
            inner_rect: SurfaceRect {
                x: 1,
                y: 0,
                width: 2,
                height: 1,
            },
            scrollbar_rect: None,
            scroll: None,
            focused: true,
            mouse_reporting: false,
            sgr_pixel_mouse: false,
            alternate_screen_active: false,
            pixel_width: 0,
            pixel_height: 0,
        };
        let bytes = encode_text(&surface, &pane, None);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("bc"));
        assert!(!text.contains("ad"));
    }

    #[test]
    fn stale_surface_patch_is_rejected_before_mutation() {
        let mut surface = SurfaceState {
            frame: FrameData {
                cells: vec![CellData {
                    symbol: " ".into(),
                    fg: 0,
                    bg: 0,
                    modifier: 0,
                    skip: false,
                    hyperlink: None,
                }],
                width: 1,
                height: 1,
                cursor: None,
                hyperlinks: Vec::new(),
                graphics: Vec::new(),
            },
            panes: Vec::new(),
            graphics: SurfaceGraphicsScene::default(),
            assets: HashMap::new(),
            revision: 4,
            boot_id: "boot".into(),
            projection_revision: 2,
        };
        let patch = PaneSurfacePatch {
            boot_id: "boot".into(),
            projection_revision: 2,
            base_surface_revision: 3,
            surface_revision: 4,
            rows: Vec::new(),
            panes: Vec::new(),
            cursor: None,
        };
        assert!(apply_patch(&mut surface, patch).is_err());
        assert_eq!(surface.revision, 4);
    }

    #[test]
    fn graphics_placement_is_rebased_to_pane_origin() {
        let key = SurfaceGraphicsAssetKey {
            source: SurfaceGraphicsSource::PaneLayer {
                pane_id: "p1".into(),
                layer_id: "image".into(),
            },
            image_width: 1,
            image_height: 1,
            format: SurfaceGraphicsFormat::Png,
            data_len: 3,
            data_fingerprint: 0,
        };
        let scene = SurfaceGraphicsScene {
            assets: vec![SurfaceGraphicsAsset {
                key: key.clone(),
                data: vec![1, 2, 3],
            }],
            placements: vec![SurfaceGraphicsPlacement {
                asset: key.clone(),
                logical_placement_id: 7,
                x: 4,
                y: 3,
                cols: 1,
                rows: 1,
                source_x: 0,
                source_y: 0,
                source_width: 1,
                source_height: 1,
                x_offset: 0,
                y_offset: 0,
                z: 0,
                scrollback_offset: 0,
            }],
            retained_assets: Vec::new(),
        };
        let assets = HashMap::from([(key, vec![1, 2, 3])]);
        let bytes = encode_graphics(
            &scene,
            &assets,
            "p1",
            SurfaceRect {
                x: 3,
                y: 2,
                width: 4,
                height: 4,
            },
        )
        .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("\u{1b}[2;2H"));
        assert!(text.contains("a=p,i=1,p=7,x=0,y=0"));
    }

    #[test]
    fn mouse_motion_does_not_require_a_button() {
        assert!(matches!(
            client_mouse_kind(TerminalMouseKind::Moved, None),
            Ok(ClientMouseKind::Moved)
        ));
        assert!(client_mouse_kind(TerminalMouseKind::Down, None).is_err());
    }
}
