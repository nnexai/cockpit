//! Generation-1 Herdr endpoint metadata, including the optional frozen v0.9.3
//! surface-delta layout. Only commands and popup metadata are retained.
use std::path::Path;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use cockpit_core::InspectionError;
use cockpit_protocol::herdr_shell::{
    HerdrCommand, HerdrCommandAction, HerdrPopup, HerdrPopupSize, HerdrShellState, HerdrShellStatus,
};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

const MAX_FRAME_SIZE: usize = 2 * 1024 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HANDSHAKE_FRAMES: usize = 256;
const SNAPSHOT_CODEC: &str = "shell.snapshot.v1";
const SURFACE_CODEC: &str = "shell.surface.v1";
const DELTA_CODEC: &str = "endpoint.surface-delta.v1";
const MAX_DELTA_SPANS: usize = 4096;

fn malformed(message: impl Into<String>) -> InspectionError {
    InspectionError::new("shell_malformed", message)
}
fn unsupported(message: impl Into<String>) -> InspectionError {
    InspectionError::new("shell_unsupported", message)
}
fn disconnected(error: impl std::fmt::Display) -> InspectionError {
    InspectionError::new(
        "shell_disconnected",
        format!("Herdr shell connection failed: {error}"),
    )
}
fn check_identity(actual: &str, expected: &str) -> Result<(), InspectionError> {
    if actual != expected {
        return Err(InspectionError::new(
            "shell_identity_mismatch",
            "Herdr shell endpoint identity changed",
        ));
    }
    Ok(())
}

fn check_peer_identity(observed: &str, expected: &str) -> Result<(), InspectionError> {
    // API and client sockets have different paths but must belong to the same
    // process, credentials, and process-start incarnation.
    let observed = observed.rsplit_once(":pid=").map(|(_, identity)| identity);
    let expected = expected.rsplit_once(":pid=").map(|(_, identity)| identity);
    if observed.is_none() || expected.is_none() || observed != expected {
        return Err(InspectionError::new(
            "shell_identity_mismatch",
            "Herdr shell socket peer does not match the inspected endpoint",
        ));
    }
    Ok(())
}

pub(crate) struct ShellConnection {
    pub state: HerdrShellState,
    socket: UnixStream,
    reader: FramedReader,
    boot_id: Option<String>,
    welcomed: bool,
    snapshot_revision: Option<u64>,
    surface: Option<SurfaceMetadata>,
    delta_negotiated: bool,
    delta_scratch: Vec<u8>,
}

pub(crate) async fn connect(
    path: &Path,
    expected_peer_identity: &str,
) -> Result<ShellConnection, InspectionError> {
    timeout(HANDSHAKE_TIMEOUT, async {
        let mut socket = UnixStream::connect(path).await.map_err(disconnected)?;
        let observed = crate::cli::HerdrCliAdapter::socket_peer_identity(path, &socket)?;
        check_peer_identity(&observed, expected_peer_identity)?;
        let hello = serde_json::json!({
            "generation": 1, "cell_width_px": 8, "cell_height_px": 16,
            "surface_size": {"cols": 120, "rows": 40}, "surface_active": true,
            "pixel_mouse": false, "direct_graphics": false, "endpoint_keybindings": false,
            "mouse_capture": false, "surface_reuse": false, "surface_delta": true,
            "surface_scroll": false, "snapshot_codecs": [SNAPSHOT_CODEC],
            "surface_codecs": [SURFACE_CODEC], "input_codecs": ["shell.input.semantic.v1"],
            "blob_codecs": ["shell.blob.v1"]
        })
        .to_string();
        // EndpointControl is frozen at tag 20 in both directions. A tuple is
        // exactly the enum tag followed by its two fields under standard bincode.
        let payload = bincode::serde::encode_to_vec(
            (20_u32, "endpoint.hello.v1", hello.as_str()),
            bincode::config::standard(),
        )
        .map_err(|error| malformed(error.to_string()))?;
        socket
            .write_all(&(payload.len() as u32).to_le_bytes())
            .await
            .map_err(disconnected)?;
        socket.write_all(&payload).await.map_err(disconnected)?;
        socket.flush().await.map_err(disconnected)?;
        let mut connection = ShellConnection {
            state: HerdrShellState {
                status: HerdrShellStatus::Connecting,
                commands: Vec::new(),
                prefix_bindings: Vec::new(),
                popup: None,
                error: None,
            },
            socket,
            reader: FramedReader::default(),
            boot_id: None,
            welcomed: false,
            snapshot_revision: None,
            surface: None,
            delta_negotiated: false,
            delta_scratch: Vec::new(),
        };
        for _ in 0..MAX_HANDSHAKE_FRAMES {
            connection.next().await?;
            if connection.welcomed
                && connection.snapshot_revision.is_some()
                && connection.surface.as_ref().is_some_and(|surface| {
                    Some(surface.projection_revision) == connection.snapshot_revision
                })
            {
                connection.state.status = HerdrShellStatus::Live;
                return Ok(connection);
            }
        }
        Err(malformed("Herdr shell handshake exceeded its frame limit"))
    })
    .await
    .map_err(|_| InspectionError::new("shell_timeout", "Herdr shell handshake timed out"))?
}

impl ShellConnection {
    /// Reads one frame. Partial reads are retained so cancellation by the
    /// adapter's subscription select does not lose the framing boundary.
    pub(crate) async fn next(&mut self) -> Result<bool, InspectionError> {
        let bytes = self.reader.read(&mut self.socket).await?;
        let message = decode_message(bytes, self.delta_negotiated, &mut self.delta_scratch)?;
        let changed = self.apply(message)?;
        self.reader.reset();
        Ok(changed)
    }

    fn apply(&mut self, message: Message) -> Result<bool, InspectionError> {
        match message {
            Message::Welcome(welcome) => {
                if self.welcomed {
                    return Err(malformed("Duplicate endpoint welcome"));
                }
                if welcome.generation != 1
                    || welcome.snapshot_codec != SNAPSHOT_CODEC
                    || welcome.surface_codec != SURFACE_CODEC
                    || welcome.input_codec != "shell.input.semantic.v1"
                    || welcome.blob_codec != "shell.blob.v1"
                {
                    return Err(unsupported(
                        "Herdr endpoint generation or mandatory codecs are unsupported",
                    ));
                }
                if let Some(error) = welcome.error {
                    return Err(unsupported(format!("{}: {}", error.code, error.message)));
                }
                // v0.9.2 welcome has no boot ID; socket peer credentials establish
                // provenance, then the first snapshot establishes the boot ID.
                self.delta_negotiated = welcome
                    .capabilities
                    .iter()
                    .any(|value| value == "surface_delta");
                self.welcomed = true;
                Ok(false)
            }
            Message::Snapshot(snapshot) => {
                self.require_welcome()?;
                if let Some(boot_id) = &self.boot_id {
                    check_identity(&snapshot.boot_id, boot_id)?;
                } else {
                    if snapshot.boot_id.is_empty() {
                        return Err(malformed("Herdr shell snapshot has an empty boot ID"));
                    }
                    self.boot_id = Some(snapshot.boot_id);
                }
                if self
                    .snapshot_revision
                    .is_some_and(|revision| snapshot.revision < revision)
                {
                    return Err(malformed("Herdr shell snapshot revision regressed"));
                }
                let prefix_bindings = decode_prefixes(snapshot.server_keybindings_toml.as_deref())?;
                let commands: Vec<_> = snapshot
                    .commands
                    .into_iter()
                    .map(Command::into_dto)
                    .collect();
                let mut changed = self.state.commands != commands
                    || self.state.prefix_bindings != prefix_bindings;
                if self.state.commands != commands {
                    self.state.commands = commands;
                }
                self.state.prefix_bindings = prefix_bindings;
                self.snapshot_revision = Some(snapshot.revision);
                if let Some(surface) = &self.surface
                    && surface.projection_revision == snapshot.revision
                    && self.state.popup != surface.popup
                {
                    self.state.popup = surface.popup.clone();
                    changed = true;
                }
                Ok(changed)
            }
            Message::Surface(surface) => {
                self.require_welcome()?;
                let boot_id = self
                    .boot_id
                    .as_deref()
                    .ok_or_else(|| malformed("Herdr shell surface preceded its snapshot"))?;
                check_identity(&surface.boot_id, boot_id)?;
                if self.surface.as_ref().is_some_and(|previous| {
                    surface.surface_revision <= previous.surface_revision
                        || surface.projection_revision < previous.projection_revision
                }) {
                    return Err(malformed("Herdr shell surface revision regressed"));
                }
                let changed = Some(surface.projection_revision) == self.snapshot_revision
                    && self.state.popup != surface.popup;
                if changed {
                    self.state.popup = surface.popup.clone();
                }
                self.surface = Some(surface);
                Ok(changed)
            }
            Message::Delta {
                base_projection_revision,
                base_surface_revision,
                surface,
                popup_update,
            } => {
                self.require_welcome()?;
                let baseline = self
                    .surface
                    .as_ref()
                    .ok_or_else(|| malformed("Herdr shell delta has no full baseline"))?;
                let boot_id = self
                    .boot_id
                    .as_deref()
                    .ok_or_else(|| malformed("Herdr shell delta preceded its snapshot"))?;
                check_identity(&surface.boot_id, boot_id)?;
                if base_projection_revision != baseline.projection_revision
                    || base_surface_revision != baseline.surface_revision
                    || surface.surface_revision <= base_surface_revision
                    || surface.projection_revision < base_projection_revision
                    || (surface.width, surface.height) != (baseline.width, baseline.height)
                {
                    return Err(malformed("Herdr shell delta does not match its baseline"));
                }
                match (surface.popup.as_ref(), popup_update) {
                    (None, None) | (Some(_), Some(PopupCellsUpdate::Replace)) => {}
                    (Some(popup), Some(PopupCellsUpdate::Patch))
                        if baseline.popup.as_ref().is_some_and(|previous| {
                            previous.terminal_id == popup.terminal_id
                                && baseline.popup_dimensions == surface.popup_dimensions
                        }) => {}
                    _ => {
                        return Err(malformed(
                            "Herdr shell popup delta does not match its baseline",
                        ));
                    }
                }
                let changed = Some(surface.projection_revision) == self.snapshot_revision
                    && self.state.popup != surface.popup;
                if changed {
                    self.state.popup = surface.popup.clone();
                }
                self.surface = Some(surface);
                Ok(changed)
            }
            Message::Patch {
                boot_id,
                projection_revision,
                base_surface_revision,
                surface_revision,
            } => {
                self.require_welcome()?;
                let expected = self
                    .boot_id
                    .as_deref()
                    .ok_or_else(|| malformed("Herdr shell patch preceded its snapshot"))?;
                check_identity(&boot_id, expected)?;
                let surface = self
                    .surface
                    .as_mut()
                    .ok_or_else(|| malformed("Herdr shell patch has no full baseline"))?;
                if projection_revision != surface.projection_revision
                    || base_surface_revision != surface.surface_revision
                    || surface_revision <= base_surface_revision
                {
                    return Err(malformed(
                        "Herdr shell patch revision does not match its baseline",
                    ));
                }
                surface.surface_revision = surface_revision;
                // Legacy patches carry no popup metadata; full surfaces and deltas do.
                Ok(false)
            }
            Message::Error(message) => {
                self.require_welcome()?;
                let changed = self.state.error.as_ref() != Some(&message);
                self.state.error = Some(message);
                Ok(changed)
            }
            Message::Shutdown(reason) => Err(disconnected(
                reason.as_deref().unwrap_or("server shut down"),
            )),
            Message::Ignored => {
                self.require_welcome()?;
                Ok(false)
            }
        }
    }

    fn require_welcome(&self) -> Result<(), InspectionError> {
        if self.welcomed {
            Ok(())
        } else {
            Err(unsupported(
                "Herdr endpoint did not send a generation-1 welcome",
            ))
        }
    }
}

#[derive(Default)]
struct FramedReader {
    prefix: [u8; 4],
    prefix_read: usize,
    payload: Vec<u8>,
    payload_read: usize,
}
impl FramedReader {
    async fn read(&mut self, socket: &mut UnixStream) -> Result<&[u8], InspectionError> {
        while self.prefix_read < 4 {
            let count = socket
                .read(&mut self.prefix[self.prefix_read..])
                .await
                .map_err(disconnected)?;
            if count == 0 {
                return Err(disconnected("end of stream"));
            }
            self.prefix_read += count;
        }
        let length = frame_length(self.prefix)?;
        self.payload.resize(length, 0);
        while self.payload_read < length {
            let count = socket
                .read(&mut self.payload[self.payload_read..])
                .await
                .map_err(disconnected)?;
            if count == 0 {
                return Err(disconnected("truncated frame"));
            }
            self.payload_read += count;
        }
        Ok(&self.payload)
    }
    fn reset(&mut self) {
        self.prefix_read = 0;
        self.payload_read = 0;
    }
}
fn frame_length(prefix: [u8; 4]) -> Result<usize, InspectionError> {
    let length = u32::from_le_bytes(prefix) as usize;
    if length == 0 || length > MAX_FRAME_SIZE {
        return Err(malformed(
            "Herdr shell frame size is invalid or exceeds 2 MiB",
        ));
    }
    Ok(length)
}

#[derive(Deserialize)]
struct Welcome {
    generation: u32,
    snapshot_codec: String,
    surface_codec: String,
    input_codec: String,
    blob_codec: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    error: Option<WelcomeError>,
}
#[derive(Deserialize)]
struct WelcomeError {
    code: String,
    message: String,
}
#[derive(Deserialize)]
struct Snapshot {
    boot_id: String,
    revision: u64,
    commands: Vec<Command>,
    #[serde(default)]
    server_keybindings_toml: Option<String>,
}

#[derive(Deserialize)]
struct PrefixProfile {
    keys: Option<PrefixKeys>,
}
#[derive(Deserialize)]
struct PrefixKeys {
    prefix: Option<PrefixBinding>,
    extra_prefixes: Option<PrefixBinding>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum PrefixBinding {
    One(String),
    Many(Vec<String>),
}
impl PrefixBinding {
    fn into_values(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

fn decode_prefixes(profile: Option<&str>) -> Result<Vec<String>, InspectionError> {
    let Some(profile) = profile else {
        return Ok(Vec::new());
    };
    // KeysConfigOverlay::set_prefixes serializes the primary normalized combo
    // as prefix and all additional aliases separately as extra_prefixes.
    let profile: PrefixProfile = toml::from_str(profile)
        .map_err(|error| malformed(format!("Invalid endpoint keybinding profile: {error}")))?;
    let Some(keys) = profile.keys else {
        return Ok(Vec::new());
    };
    let mut prefixes = keys
        .prefix
        .map(PrefixBinding::into_values)
        .unwrap_or_default();
    if let Some(extra) = keys.extra_prefixes {
        prefixes.extend(extra.into_values());
    }
    prefixes.retain(|prefix| !prefix.is_empty());
    Ok(prefixes)
}
#[derive(Deserialize)]
struct Command {
    command_id: String,
    binding_labels: Vec<String>,
    action: CommandAction,
    description: Option<String>,
}
#[derive(Deserialize)]
enum CommandAction {
    Shell,
    Pane,
    Popup,
    PluginAction,
    #[serde(other)]
    Unknown,
}
impl Command {
    fn into_dto(self) -> HerdrCommand {
        HerdrCommand {
            command_id: self.command_id,
            binding_labels: self.binding_labels,
            action: match self.action {
                CommandAction::Shell => HerdrCommandAction::Shell,
                CommandAction::Pane => HerdrCommandAction::Pane,
                CommandAction::Popup => HerdrCommandAction::Popup,
                CommandAction::PluginAction => HerdrCommandAction::PluginAction,
                CommandAction::Unknown => HerdrCommandAction::Unknown,
            },
            description: self.description,
        }
    }
}
fn json<T: serde::de::DeserializeOwned>(data: &str) -> Result<T, InspectionError> {
    serde_json::from_str(data)
        .map_err(|error| malformed(format!("Invalid shell metadata JSON: {error}")))
}

struct SurfaceMetadata {
    boot_id: String,
    projection_revision: u64,
    surface_revision: u64,
    width: u16,
    height: u16,
    popup: Option<HerdrPopup>,
    popup_dimensions: Option<(u16, u16)>,
}
enum PopupCellsUpdate {
    Patch,
    Replace,
}
enum Message {
    Welcome(Welcome),
    Snapshot(Snapshot),
    Surface(SurfaceMetadata),
    Delta {
        base_projection_revision: u64,
        base_surface_revision: u64,
        surface: SurfaceMetadata,
        popup_update: Option<PopupCellsUpdate>,
    },
    Patch {
        boot_id: String,
        projection_revision: u64,
        base_surface_revision: u64,
        surface_revision: u64,
    },
    Error(String),
    Shutdown(Option<String>),
    Ignored,
}

// Walk standard bincode without constructing cell vectors, topology, or image
// assets. Borrowed strings and blobs refer directly to the reusable frame buffer.
struct Decoder<'a> {
    bytes: &'a [u8],
    delta: bool,
}
impl<'a> Decoder<'a> {
    fn value<T: Deserialize<'a>>(&mut self) -> Result<T, InspectionError> {
        let (value, consumed) =
            bincode::serde::borrow_decode_from_slice(self.bytes, bincode::config::standard())
                .map_err(|error| malformed(format!("Invalid shell binary frame: {error}")))?;
        self.bytes = &self.bytes[consumed..];
        Ok(value)
    }
    fn text(&mut self) -> Result<&'a str, InspectionError> {
        self.value()
    }
    fn flag(&mut self) -> Result<bool, InspectionError> {
        self.value()
    }
    fn number(&mut self) -> Result<u64, InspectionError> {
        self.value()
    }
    fn count(&mut self) -> Result<usize, InspectionError> {
        let count = self.number()?;
        if count > self.bytes.len() as u64 {
            return Err(malformed("Shell collection length exceeds its frame"));
        }
        Ok(count as usize)
    }
    fn bounded_count(&mut self, limit: usize) -> Result<usize, InspectionError> {
        let count = self.count()?;
        if self.delta && count > limit {
            return Err(malformed("Shell delta collection exceeds its limit"));
        }
        Ok(count)
    }
    fn variant(&mut self, variants: u32) -> Result<u32, InspectionError> {
        let tag: u32 = self.value()?;
        if tag >= variants {
            return Err(unsupported("Unknown binary shell enum variant"));
        }
        Ok(tag)
    }
    fn optional_text(&mut self) -> Result<Option<&'a str>, InspectionError> {
        if self.flag()? {
            Ok(Some(self.text()?))
        } else {
            Ok(None)
        }
    }
    fn blob(&mut self) -> Result<(), InspectionError> {
        let length = self.count()?;
        self.bytes = &self.bytes[length..];
        Ok(())
    }
    fn cursor(&mut self) -> Result<(), InspectionError> {
        if self.flag()? {
            let _: u16 = self.value()?;
            let _: u16 = self.value()?;
            self.flag()?;
            let _: u8 = self.value()?;
        }
        Ok(())
    }
    fn cells(&mut self) -> Result<usize, InspectionError> {
        let count = self.count()?;
        self.cell_values(count)?;
        Ok(count)
    }
    fn cell_values(&mut self, count: usize) -> Result<(), InspectionError> {
        if count > self.bytes.len() / 6 {
            return Err(malformed("Shell cell count exceeds its frame"));
        }
        for _ in 0..count {
            self.text()?;
            let _: u32 = self.value()?;
            let _: u32 = self.value()?;
            let _: u16 = self.value()?;
            self.flag()?;
            if self.flag()? {
                let _: u32 = self.value()?;
            }
        }
        Ok(())
    }
    fn frame(&mut self) -> Result<(u16, u16), InspectionError> {
        let cells = if self.delta {
            self.count()?
        } else {
            self.cells()?
        };
        let width: u16 = self.value()?;
        let height: u16 = self.value()?;
        if self.delta {
            Self::grid_size(width, height)?;
            if cells != 0 {
                return Err(malformed("Shell delta metadata contains cells"));
            }
        } else if cells != usize::from(width) * usize::from(height) {
            return Err(malformed("Shell grid dimensions do not match its cells"));
        }
        self.cursor()?;
        for _ in 0..self.bounded_count(65_536)? {
            self.text()?;
        }
        self.blob()?;
        Ok((width, height))
    }
    fn grid_size(width: u16, height: u16) -> Result<usize, InspectionError> {
        let cells = usize::from(width) * usize::from(height);
        if width > 4096 || height > 4096 || cells > 1_000_000 {
            return Err(malformed("Shell delta dimensions exceed their limit"));
        }
        Ok(cells)
    }
    fn rows(&mut self, width: u16, height: u16) -> Result<(), InspectionError> {
        let budget = Self::grid_size(width, height)?;
        let count = self.bounded_count(MAX_DELTA_SPANS.min(budget))?;
        let row_width = usize::from(width);
        let mut previous_end = 0;
        let mut total_cells = 0;
        for _ in 0..count {
            let x: u16 = self.value()?;
            let y: u16 = self.value()?;
            let cells = self.count()?;
            if cells == 0 || y >= height || x >= width || cells > row_width - usize::from(x) {
                return Err(malformed("Shell delta span is outside its row"));
            }
            let start = usize::from(y) * row_width + usize::from(x);
            if start < previous_end {
                return Err(malformed("Shell delta spans overlap or are not sorted"));
            }
            total_cells += cells;
            if total_cells > budget {
                return Err(malformed("Shell delta cell budget exceeded"));
            }
            self.cell_values(cells)?;
            previous_end = start + cells;
        }
        Ok(())
    }
    fn rect(&mut self) -> Result<(), InspectionError> {
        for _ in 0..4 {
            let _: u16 = self.value()?;
        }
        Ok(())
    }
    fn panes(&mut self) -> Result<(), InspectionError> {
        for _ in 0..self.bounded_count(4096)? {
            self.text()?;
            self.number()?;
            self.rect()?;
            self.rect()?;
            if self.flag()? {
                self.rect()?;
            }
            if self.flag()? {
                for _ in 0..3 {
                    self.number()?;
                }
            }
            for _ in 0..4 {
                self.flag()?;
            }
            let _: u32 = self.value()?;
            let _: u32 = self.value()?;
        }
        Ok(())
    }
    fn splits(&mut self) -> Result<(), InspectionError> {
        for _ in 0..self.bounded_count(4096)? {
            self.variant(2)?;
            let _: u16 = self.value()?;
            self.rect()?;
            self.rect()?;
            for _ in 0..self.bounded_count(4096)? {
                self.flag()?;
            }
        }
        Ok(())
    }
    fn popup_size(&mut self) -> Result<Option<HerdrPopupSize>, InspectionError> {
        if !self.flag()? {
            return Ok(None);
        }
        Ok(Some(match self.variant(2)? {
            0 => HerdrPopupSize::Cells {
                value: self.value()?,
            },
            _ => HerdrPopupSize::Percent {
                value: self.value()?,
            },
        }))
    }
    fn popup(&mut self) -> Result<(Option<HerdrPopup>, Option<(u16, u16)>), InspectionError> {
        if !self.flag()? {
            return Ok((None, None));
        }
        let popup = HerdrPopup {
            terminal_id: self.text()?.to_owned(),
            title: self.text()?.to_owned(),
            width: self.popup_size()?,
            height: self.popup_size()?,
        };
        let dimensions = self.frame()?;
        self.flag()?;
        self.flag()?;
        let _: u32 = self.value()?;
        let _: u32 = self.value()?;
        Ok((Some(popup), Some(dimensions)))
    }
    fn asset_key(&mut self) -> Result<(), InspectionError> {
        match self.variant(2)? {
            0 => {
                self.variant(2)?;
                self.text()?;
                let _: u32 = self.value()?;
            }
            _ => {
                self.text()?;
                self.text()?;
            }
        }
        let _: u32 = self.value()?;
        let _: u32 = self.value()?;
        self.variant(3)?;
        self.number()?;
        self.number()?;
        Ok(())
    }
    fn graphics(&mut self) -> Result<(), InspectionError> {
        for _ in 0..self.bounded_count(4096)? {
            self.asset_key()?;
            self.blob()?;
        }
        for _ in 0..self.bounded_count(65_536)? {
            self.asset_key()?;
            let _: u32 = self.value()?;
            let _: u16 = self.value()?;
            let _: u16 = self.value()?;
            for _ in 0..8 {
                let _: u32 = self.value()?;
            }
            let _: i32 = self.value()?;
            let _: u32 = self.value()?;
        }
        for _ in 0..self.bounded_count(65_536)? {
            self.asset_key()?;
        }
        Ok(())
    }
    fn surface(&mut self) -> Result<SurfaceMetadata, InspectionError> {
        let boot_id = self.text()?.to_owned();
        let projection_revision = self.number()?;
        let surface_revision = self.number()?;
        let (width, height) = self.frame()?;
        self.panes()?;
        self.splits()?;
        let (popup, popup_dimensions) = self.popup()?;
        self.graphics()?;
        Ok(SurfaceMetadata {
            boot_id,
            projection_revision,
            surface_revision,
            width,
            height,
            popup,
            popup_dimensions,
        })
    }
}

fn decode_message(
    bytes: &[u8],
    delta_negotiated: bool,
    scratch: &mut Vec<u8>,
) -> Result<Message, InspectionError> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME_SIZE {
        return Err(malformed("Invalid shell frame size"));
    }
    let mut decoder = Decoder {
        bytes,
        delta: false,
    };
    let tag: u32 = decoder.value()?;
    let message = match tag {
        3 => Message::Shutdown(decoder.optional_text()?.map(str::to_owned)),
        4 => {
            decoder.variant(3)?;
            decoder.text()?;
            decoder.optional_text()?;
            Message::Ignored
        }
        5 => {
            decoder.text()?;
            Message::Ignored
        }
        6 => {
            decoder.optional_text()?;
            Message::Ignored
        }
        7 => Message::Ignored,
        8 => {
            decoder.flag()?;
            decoder.flag()?;
            Message::Ignored
        }
        9 => {
            let _: u16 = decoder.value()?;
            Message::Ignored
        }
        13 => Message::Surface(decoder.surface()?),
        14 => {
            decoder.variant(4)?;
            decoder.text()?;
            decoder.optional_text()?;
            if decoder.flag()? {
                decoder.variant(2)?;
            }
            for _ in 0..4 {
                decoder.optional_text()?;
            }
            if decoder.flag()? {
                decoder.variant(4)?;
            }
            Message::Ignored
        }
        15 => Message::Error(decoder.text()?.to_owned()),
        16 => {
            let _: u16 = decoder.value()?;
            let _: u8 = decoder.value()?;
            Message::Ignored
        }
        17 => {
            decoder.flag()?;
            Message::Ignored
        }
        // No endpoint requests are issued on this socket, so unsolicited replies
        // cannot be correlated safely and are not an ignorable opaque payload.
        18 => {
            return Err(unsupported(
                "Unsolicited endpoint response on metadata connection",
            ));
        }
        19 => {
            let boot_id = decoder.text()?.to_owned();
            let projection_revision = decoder.number()?;
            let base_surface_revision = decoder.number()?;
            let surface_revision = decoder.number()?;
            for _ in 0..decoder.count()? {
                let _: u16 = decoder.value()?;
                let _: u16 = decoder.value()?;
                decoder.cells()?;
            }
            decoder.panes()?;
            decoder.cursor()?;
            Message::Patch {
                boot_id,
                projection_revision,
                base_surface_revision,
                surface_revision,
            }
        }
        20 => {
            let kind = decoder.text()?;
            let data = decoder.text()?;
            match kind {
                "endpoint.welcome.v1" => Message::Welcome(json(data)?),
                SNAPSHOT_CODEC => Message::Snapshot(json(data)?),
                DELTA_CODEC if delta_negotiated => decode_delta(data, scratch)?,
                _ if kind.starts_with("shell.snapshot.")
                    || kind.starts_with("shell.surface.")
                    || kind.starts_with("endpoint.welcome.")
                    || kind.starts_with("endpoint.surface-reuse.")
                    || kind.starts_with("endpoint.surface-delta.")
                    || kind.starts_with("endpoint.surface-scroll.") =>
                {
                    return Err(unsupported(format!("Unsupported endpoint codec {kind}")));
                }
                // Generation 1 explicitly permits ignoring unknown named controls.
                _ => Message::Ignored,
            }
        }
        0..=2 | 10..=12 => {
            return Err(unsupported(
                "Unexpected direct-terminal or private-protocol frame on shell endpoint",
            ));
        }
        _ => {
            return Err(unsupported(format!(
                "Unknown shell server message tag {tag}"
            )));
        }
    };
    if !decoder.bytes.is_empty() {
        return Err(malformed("Trailing bytes in shell frame"));
    }
    Ok(message)
}

fn decode_delta(data: &str, scratch: &mut Vec<u8>) -> Result<Message, InspectionError> {
    if data.len() > MAX_FRAME_SIZE {
        return Err(malformed("Shell delta exceeds its frame limit"));
    }
    // The estimate includes at most two spare bytes and is bounded by the
    // already checked outer frame. Reuse this buffer across successive deltas.
    let capacity = base64::decoded_len_estimate(data.len());
    if scratch.capacity() < capacity {
        scratch.reserve_exact(capacity - scratch.len());
    }
    scratch.resize(capacity, 0);
    let length = STANDARD_NO_PAD
        .decode_slice(data, scratch.as_mut_slice())
        .map_err(|error| malformed(format!("Invalid shell delta base64: {error}")))?;
    scratch.truncate(length);
    let mut decoder = Decoder {
        bytes: scratch,
        delta: true,
    };
    let base_projection_revision = decoder.number()?;
    let base_surface_revision = decoder.number()?;
    let surface = decoder.surface()?;
    decoder.rows(surface.width, surface.height)?;
    let popup_update = if decoder.flag()? {
        let (width, height) = surface
            .popup_dimensions
            .ok_or_else(|| malformed("Shell delta popup cells have no popup metadata"))?;
        let variant: u32 = decoder.value()?;
        match variant {
            0 => {
                decoder.rows(width, height)?;
                Some(PopupCellsUpdate::Patch)
            }
            1 => {
                let count = decoder.count()?;
                if count != Decoder::grid_size(width, height)? {
                    return Err(malformed(
                        "Shell delta popup replacement dimensions do not match",
                    ));
                }
                decoder.cell_values(count)?;
                Some(PopupCellsUpdate::Replace)
            }
            _ => return Err(malformed("Invalid shell delta popup update variant")),
        }
    } else {
        None
    };
    if !decoder.bytes.is_empty() {
        return Err(malformed("Trailing bytes in shell delta"));
    }
    Ok(Message::Delta {
        base_projection_revision,
        base_surface_revision,
        surface,
        popup_update,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(value: impl serde::Serialize) -> Vec<u8> {
        bincode::serde::encode_to_vec(value, bincode::config::standard()).unwrap()
    }
    fn decode(bytes: &[u8]) -> Result<Message, InspectionError> {
        decode_message(bytes, true, &mut Vec::new())
    }
    fn empty_surface(boot_id: &str, revision: u64, popup: Option<(&str, &str)>) -> Vec<u8> {
        let mut bytes = encode((13_u32, boot_id, 1_u64, revision));
        append_empty_frame(&mut bytes);
        bytes.extend(encode((0_u64, 0_u64, popup.is_some())));
        if let Some((id, title)) = popup {
            bytes.extend(encode((
                id,
                title,
                Some((0_u32, 30_u16)),
                Some((1_u32, 50_u8)),
            )));
            append_empty_frame(&mut bytes);
            bytes.extend(encode((false, false, 0_u32, 0_u32)));
        }
        bytes.extend(encode((0_u64, 0_u64, 0_u64)));
        bytes
    }
    fn append_empty_frame(bytes: &mut Vec<u8>) {
        bytes.extend(encode((0_u64, 0_u16, 0_u16, false, 0_u64, 0_u64)));
    }

    #[derive(Clone)]
    enum PopupUpdate {
        Patch(Vec<(u16, u16, usize)>),
        Replace(usize),
        Invalid,
    }
    #[derive(Clone)]
    struct DeltaFixture {
        boot: &'static str,
        base_projection: u64,
        base_revision: u64,
        projection: u64,
        revision: u64,
        dimensions: (u16, u16),
        metadata_cells: u64,
        popup: bool,
        popup_id: &'static str,
        popup_dimensions: (u16, u16),
        popup_metadata_cells: u64,
        rows: Vec<(u16, u16, usize)>,
        popup_update: Option<PopupUpdate>,
    }
    impl Default for DeltaFixture {
        fn default() -> Self {
            Self {
                boot: "boot",
                base_projection: 1,
                base_revision: 1,
                projection: 1,
                revision: 2,
                dimensions: (3, 2),
                metadata_cells: 0,
                popup: false,
                popup_metadata_cells: 0,
                popup_id: "popup",
                popup_dimensions: (2, 1),
                rows: vec![(1, 0, 2), (0, 1, 1)],
                popup_update: None,
            }
        }
    }
    fn append_cells(bytes: &mut Vec<u8>, count: usize) {
        bytes.extend(encode(count as u64));
        for _ in 0..count {
            bytes.extend(encode(("x", 0_u32, 0_u32, 0_u16, false, None::<u32>)));
        }
    }
    fn append_rows(bytes: &mut Vec<u8>, rows: &[(u16, u16, usize)]) {
        bytes.extend(encode(rows.len() as u64));
        for &(x, y, count) in rows {
            bytes.extend(encode((x, y)));
            append_cells(bytes, count);
        }
    }
    impl DeltaFixture {
        fn payload(&self) -> Vec<u8> {
            let mut bytes = encode((
                self.base_projection,
                self.base_revision,
                self.boot,
                self.projection,
                self.revision,
            ));
            bytes.extend(encode((
                self.metadata_cells,
                self.dimensions.0,
                self.dimensions.1,
                false,
                0_u64,
                0_u64,
                0_u64,
                0_u64,
                self.popup,
            )));
            if self.popup {
                bytes.extend(encode((
                    self.popup_id,
                    "Title",
                    Some((0_u32, 30_u16)),
                    Some((1_u32, 50_u8)),
                    self.popup_metadata_cells,
                    self.popup_dimensions.0,
                    self.popup_dimensions.1,
                    false,
                    0_u64,
                    0_u64,
                    false,
                    false,
                    0_u32,
                    0_u32,
                )));
            }
            bytes.extend(encode((0_u64, 0_u64, 0_u64)));
            append_rows(&mut bytes, &self.rows);
            bytes.extend(encode(self.popup_update.is_some()));
            match &self.popup_update {
                Some(PopupUpdate::Patch(rows)) => {
                    bytes.extend(encode(0_u32));
                    append_rows(&mut bytes, rows);
                }
                Some(PopupUpdate::Replace(count)) => {
                    bytes.extend(encode(1_u32));
                    append_cells(&mut bytes, *count);
                }
                Some(PopupUpdate::Invalid) => bytes.extend(encode(2_u32)),
                None => {}
            }
            bytes
        }
        fn control(&self) -> Vec<u8> {
            delta_control(&self.payload())
        }
        fn baseline(&self) -> Vec<u8> {
            let mut bytes = encode((13_u32, self.boot, self.base_projection, self.base_revision));
            append_cells(
                &mut bytes,
                usize::from(self.dimensions.0) * usize::from(self.dimensions.1),
            );
            bytes.extend(encode((
                self.dimensions.0,
                self.dimensions.1,
                false,
                0_u64,
                0_u64,
                0_u64,
                0_u64,
                self.popup,
            )));
            if self.popup {
                bytes.extend(encode((
                    self.popup_id,
                    "Title",
                    Some((0_u32, 30_u16)),
                    Some((1_u32, 50_u8)),
                )));
                append_cells(
                    &mut bytes,
                    usize::from(self.popup_dimensions.0) * usize::from(self.popup_dimensions.1),
                );
                bytes.extend(encode((
                    self.popup_dimensions.0,
                    self.popup_dimensions.1,
                    false,
                    0_u64,
                    0_u64,
                    false,
                    false,
                    0_u32,
                    0_u32,
                )));
            }
            bytes.extend(encode((0_u64, 0_u64, 0_u64)));
            bytes
        }
    }
    fn delta_control(payload: &[u8]) -> Vec<u8> {
        encode((20_u32, DELTA_CODEC, STANDARD_NO_PAD.encode(payload)))
    }

    #[test]
    fn delta_rejects_every_truncation_and_trailing_bytes() {
        let fixture = DeltaFixture {
            popup: true,
            popup_update: Some(PopupUpdate::Replace(2)),
            ..DeltaFixture::default()
        };
        let payload = fixture.payload();
        for end in 0..payload.len() {
            assert_eq!(
                decode(&delta_control(&payload[..end])).err().unwrap().code,
                "shell_malformed"
            );
        }
        let control = fixture.control();
        for end in 0..control.len() {
            assert_eq!(
                decode(&control[..end]).err().unwrap().code,
                "shell_malformed"
            );
        }
        let mut trailing = payload;
        trailing.push(0);
        assert_eq!(
            decode(&delta_control(&trailing)).err().unwrap().code,
            "shell_malformed"
        );
        let mut trailing = control;
        trailing.push(0);
        assert_eq!(decode(&trailing).err().unwrap().code, "shell_malformed");
        for data in ["!", "Zg==", "Zh"] {
            assert_eq!(
                decode(&encode((20_u32, DELTA_CODEC, data)))
                    .err()
                    .unwrap()
                    .code,
                "shell_malformed"
            );
        }
    }

    #[test]
    fn delta_rejects_invalid_grids_spans_and_popup_updates() {
        let default = DeltaFixture::default();
        let invalid = [
            DeltaFixture {
                dimensions: (4097, 1),
                ..default.clone()
            },
            DeltaFixture {
                dimensions: (1001, 1000),
                ..default.clone()
            },
            DeltaFixture {
                metadata_cells: 1,
                ..default.clone()
            },
            DeltaFixture {
                popup: true,
                popup_metadata_cells: 1,
                ..default.clone()
            },
            DeltaFixture {
                rows: vec![(0, 0, 0)],
                ..default.clone()
            },
            DeltaFixture {
                rows: vec![(3, 0, 1)],
                ..default.clone()
            },
            DeltaFixture {
                rows: vec![(0, 2, 1)],
                ..default.clone()
            },
            DeltaFixture {
                rows: vec![(2, 0, 2)],
                ..default.clone()
            },
            DeltaFixture {
                rows: vec![(0, 1, 1), (0, 0, 1)],
                ..default.clone()
            },
            DeltaFixture {
                rows: vec![(0, 0, 2), (1, 0, 1)],
                ..default.clone()
            },
            DeltaFixture {
                rows: vec![(0, 0, 1); 7],
                ..default.clone()
            },
            DeltaFixture {
                dimensions: (4096, 2),
                rows: vec![(0, 0, 1); 4097],
                ..default.clone()
            },
            DeltaFixture {
                popup_update: Some(PopupUpdate::Replace(2)),
                ..default.clone()
            },
            DeltaFixture {
                popup: true,
                popup_update: Some(PopupUpdate::Replace(1)),
                ..default.clone()
            },
            DeltaFixture {
                popup: true,
                popup_update: Some(PopupUpdate::Patch(vec![(1, 0, 2)])),
                ..default.clone()
            },
            DeltaFixture {
                popup: true,
                popup_update: Some(PopupUpdate::Invalid),
                ..default
            },
        ];
        for fixture in invalid {
            assert_eq!(
                decode(&fixture.control()).err().unwrap().code,
                "shell_malformed"
            );
        }
    }

    #[tokio::test]
    async fn delta_popup_continuity_survives_legacy_patch_and_projection_interleave() {
        let fixture = DeltaFixture::default();
        let mut connection = connection("boot");
        assert!(
            !connection
                .apply(decode(&fixture.baseline()).unwrap())
                .unwrap()
        );
        let open = DeltaFixture {
            popup: true,
            popup_update: Some(PopupUpdate::Replace(2)),
            ..fixture.clone()
        };
        assert!(connection.apply(decode(&open.control()).unwrap()).unwrap());
        let popup = connection.state.popup.clone().unwrap();
        assert_eq!(popup.terminal_id, "popup");
        assert_eq!(popup.title, "Title");
        assert_eq!(popup.width, Some(HerdrPopupSize::Cells { value: 30 }));
        assert_eq!(popup.height, Some(HerdrPopupSize::Percent { value: 50 }));
        let full = decode(&empty_surface("boot", 2, Some(("popup", "Title")))).unwrap();
        let mut full_connection = self::connection("boot");
        assert!(full_connection.apply(full).unwrap());
        assert_eq!(connection.state.popup, full_connection.state.popup);
        let patch = encode((19_u32, "boot", 1_u64, 2_u64, 3_u64, 0_u64, 0_u64, false));
        assert!(!connection.apply(decode(&patch).unwrap()).unwrap());
        let update = DeltaFixture {
            base_revision: 3,
            revision: 4,
            popup_update: Some(PopupUpdate::Patch(vec![(0, 0, 1)])),
            ..open
        };
        assert!(
            !connection
                .apply(decode(&update.control()).unwrap())
                .unwrap()
        );
        assert_eq!(connection.state.popup.as_ref(), Some(&popup));
        let close = DeltaFixture {
            base_revision: 4,
            revision: 5,
            projection: 2,
            ..fixture
        };
        assert!(!connection.apply(decode(&close.control()).unwrap()).unwrap());
        assert_eq!(connection.state.popup.as_ref(), Some(&popup));
        let snapshot = json(r#"{"boot_id":"boot","revision":2,"commands":[]}"#).unwrap();
        assert!(connection.apply(Message::Snapshot(snapshot)).unwrap());
        assert_eq!(connection.state.popup, None);
        assert!(
            full_connection
                .apply(decode(&empty_surface("boot", 5, None)).unwrap())
                .unwrap()
        );
        assert_eq!(connection.state.popup, full_connection.state.popup);
        // A snapshot can also arrive first; the matching delta then publishes
        // immediately rather than waiting for another snapshot.
        let snapshot = json(r#"{"boot_id":"boot","revision":3,"commands":[]}"#).unwrap();
        assert!(!connection.apply(Message::Snapshot(snapshot)).unwrap());
        let reopen = DeltaFixture {
            base_projection: 2,
            base_revision: 5,
            projection: 3,
            revision: 6,
            popup: true,
            popup_update: Some(PopupUpdate::Replace(2)),
            ..DeltaFixture::default()
        };
        assert!(
            connection
                .apply(decode(&reopen.control()).unwrap())
                .unwrap()
        );
        assert_eq!(connection.state.popup.as_ref(), Some(&popup));
    }

    #[tokio::test]
    async fn delta_rejects_absent_baseline_identity_and_revision_chain_changes() {
        let default = DeltaFixture::default();
        let mut absent = connection("boot");
        assert_eq!(
            absent
                .apply(decode(&default.control()).unwrap())
                .err()
                .unwrap()
                .code,
            "shell_malformed"
        );
        let mut unwelcomed = connection("boot");
        unwelcomed.welcomed = false;
        assert_eq!(
            unwelcomed
                .apply(decode(&default.control()).unwrap())
                .err()
                .unwrap()
                .code,
            "shell_unsupported"
        );
        for (fixture, code) in [
            (
                DeltaFixture {
                    boot: "other",
                    ..default.clone()
                },
                "shell_identity_mismatch",
            ),
            (
                DeltaFixture {
                    base_revision: 2,
                    ..default.clone()
                },
                "shell_malformed",
            ),
            (
                DeltaFixture {
                    base_projection: 2,
                    ..default.clone()
                },
                "shell_malformed",
            ),
            (
                DeltaFixture {
                    revision: 1,
                    ..default.clone()
                },
                "shell_malformed",
            ),
            (
                DeltaFixture {
                    projection: 0,
                    ..default.clone()
                },
                "shell_malformed",
            ),
            (
                DeltaFixture {
                    dimensions: (2, 3),
                    rows: Vec::new(),
                    ..default.clone()
                },
                "shell_malformed",
            ),
        ] {
            let mut connection = connection("boot");
            connection
                .apply(decode(&default.baseline()).unwrap())
                .unwrap();
            assert_eq!(
                connection
                    .apply(decode(&fixture.control()).unwrap())
                    .err()
                    .unwrap()
                    .code,
                code
            );
        }
    }

    #[tokio::test]
    async fn delta_popup_patch_requires_matching_grid_baseline() {
        let base = DeltaFixture::default();
        let open = DeltaFixture {
            popup: true,
            popup_update: Some(PopupUpdate::Replace(2)),
            ..base.clone()
        };
        let next = DeltaFixture {
            base_revision: 2,
            revision: 3,
            popup_update: Some(PopupUpdate::Patch(Vec::new())),
            ..open.clone()
        };
        for invalid in [
            DeltaFixture {
                popup_id: "other-popup",
                ..next.clone()
            },
            DeltaFixture {
                popup_dimensions: (1, 2),
                ..next.clone()
            },
            DeltaFixture {
                popup_update: None,
                ..next.clone()
            },
        ] {
            let mut connection = connection("boot");
            connection.apply(decode(&base.baseline()).unwrap()).unwrap();
            connection.apply(decode(&open.control()).unwrap()).unwrap();
            let popup = connection.state.popup.clone();
            assert_eq!(
                connection
                    .apply(decode(&invalid.control()).unwrap())
                    .err()
                    .unwrap()
                    .code,
                "shell_malformed"
            );
            assert_eq!(connection.state.popup, popup);
            assert_eq!(connection.surface.as_ref().unwrap().surface_revision, 2);
        }
        let mut connection = connection("boot");
        connection.apply(decode(&base.baseline()).unwrap()).unwrap();
        let missing = DeltaFixture {
            popup: true,
            popup_update: Some(PopupUpdate::Patch(Vec::new())),
            ..base
        };
        assert_eq!(
            connection
                .apply(decode(&missing.control()).unwrap())
                .err()
                .unwrap()
                .code,
            "shell_malformed"
        );
        assert_eq!(connection.state.popup, None);
        assert_eq!(connection.surface.as_ref().unwrap().surface_revision, 1);
    }

    #[tokio::test]
    async fn delta_popup_replace_authorizes_new_or_resized_grid_and_patch_preserves_it() {
        let base = DeltaFixture::default();
        let mut connection = connection("boot");
        connection.apply(decode(&base.baseline()).unwrap()).unwrap();
        let open = DeltaFixture {
            popup: true,
            popup_update: Some(PopupUpdate::Replace(2)),
            ..base
        };
        assert!(connection.apply(decode(&open.control()).unwrap()).unwrap());
        let patch = DeltaFixture {
            base_revision: 2,
            revision: 3,
            popup_update: Some(PopupUpdate::Patch(vec![(0, 0, 1)])),
            ..open
        };
        assert!(!connection.apply(decode(&patch.control()).unwrap()).unwrap());
        let replace = DeltaFixture {
            base_revision: 3,
            revision: 4,
            popup_id: "other-popup",
            popup_update: Some(PopupUpdate::Replace(2)),
            ..patch
        };
        assert!(
            connection
                .apply(decode(&replace.control()).unwrap())
                .unwrap()
        );
        assert_eq!(
            connection.state.popup.as_ref().unwrap().terminal_id,
            "other-popup"
        );
        let resize = DeltaFixture {
            base_revision: 4,
            revision: 5,
            popup_dimensions: (2, 2),
            popup_update: Some(PopupUpdate::Replace(4)),
            ..replace
        };
        assert!(
            !connection
                .apply(decode(&resize.control()).unwrap())
                .unwrap()
        );
        let patch = DeltaFixture {
            base_revision: 5,
            revision: 6,
            popup_update: Some(PopupUpdate::Patch(vec![(1, 1, 1)])),
            ..resize
        };
        assert!(!connection.apply(decode(&patch.control()).unwrap()).unwrap());
        assert_eq!(connection.surface.as_ref().unwrap().surface_revision, 6);
    }

    #[tokio::test]
    async fn delta_popup_patch_tracks_full_surface_actual_grid_not_size_preferences() {
        let full = DeltaFixture {
            popup: true,
            popup_update: Some(PopupUpdate::Patch(vec![(1, 0, 1)])),
            ..DeltaFixture::default()
        };
        let mut connection = connection("boot");
        assert!(connection.apply(decode(&full.baseline()).unwrap()).unwrap());
        assert!(!connection.apply(decode(&full.control()).unwrap()).unwrap());
        let invalid = DeltaFixture {
            base_revision: 2,
            revision: 3,
            popup_dimensions: (1, 2),
            popup_update: Some(PopupUpdate::Patch(Vec::new())),
            ..full.clone()
        };
        assert_eq!(
            connection
                .apply(decode(&invalid.control()).unwrap())
                .err()
                .unwrap()
                .code,
            "shell_malformed"
        );
        assert_eq!(connection.surface.as_ref().unwrap().surface_revision, 2);
        let replacement = DeltaFixture {
            popup_update: Some(PopupUpdate::Replace(2)),
            ..invalid
        };
        assert!(
            !connection
                .apply(decode(&replacement.control()).unwrap())
                .unwrap()
        );
        let valid = DeltaFixture {
            base_revision: 3,
            revision: 4,
            popup_dimensions: (1, 2),
            popup_update: Some(PopupUpdate::Patch(vec![(0, 1, 1)])),
            ..full
        };
        assert!(!connection.apply(decode(&valid.control()).unwrap()).unwrap());
    }

    #[tokio::test]
    async fn delta_popup_metadata_without_cell_update_is_rejected() {
        let base = DeltaFixture::default();
        let mut connection = connection("boot");
        connection.apply(decode(&base.baseline()).unwrap()).unwrap();
        let invalid = DeltaFixture {
            popup: true,
            ..base
        };
        assert_eq!(
            connection
                .apply(decode(&invalid.control()).unwrap())
                .err()
                .unwrap()
                .code,
            "shell_malformed"
        );
        assert_eq!(connection.state.popup, None);
    }
    fn connection(boot_id: &str) -> ShellConnection {
        let (socket, _peer) = UnixStream::pair().unwrap();
        ShellConnection {
            state: HerdrShellState {
                status: HerdrShellStatus::Live,
                commands: Vec::new(),
                prefix_bindings: Vec::new(),
                popup: None,
                error: None,
            },
            socket,
            reader: FramedReader::default(),
            boot_id: Some(boot_id.to_owned()),
            welcomed: true,
            snapshot_revision: Some(1),
            surface: None,
            delta_negotiated: true,
            delta_scratch: Vec::new(),
        }
    }

    #[test]
    fn rejects_malformed_and_oversized_frames() {
        assert!(frame_length((MAX_FRAME_SIZE as u32 + 1).to_le_bytes()).is_err());
        assert!(frame_length(0_u32.to_le_bytes()).is_err());
        let valid = empty_surface("boot", 1, Some(("popup", "Title")));
        for end in 0..valid.len() {
            assert!(decode(&valid[..end]).is_err());
        }
        let mut trailing = valid;
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        assert!(decode(&encode((13_u32, "boot", 1_u64, 1_u64, u64::MAX))).is_err());
        assert!(decode(&encode(21_u32)).is_err());
    }

    #[test]
    fn checks_peer_process_incarnation_not_socket_path() {
        let expected = "unix-socket:/session/api.sock:pid=12:uid=34:gid=56:start=789";
        assert!(
            check_peer_identity(
                "unix-socket:/session/client.sock:pid=12:uid=34:gid=56:start=789",
                expected
            )
            .is_ok()
        );
        assert!(
            check_peer_identity(
                "unix-socket:/session/client.sock:pid=12:uid=34:gid=56:start=790",
                expected
            )
            .is_err()
        );
        assert!(
            check_peer_identity(
                "unix-socket:/session/client.sock:pid=13:uid=34:gid=56:start=789",
                expected
            )
            .is_err()
        );
        assert!(check_peer_identity("", expected).is_err());
    }

    #[test]
    fn delta_requires_advertised_capability_and_other_extensions_remain_unsupported() {
        let valid = DeltaFixture::default().control();
        assert!(matches!(decode(&valid).unwrap(), Message::Delta { .. }));
        assert_eq!(
            decode_message(&valid, false, &mut Vec::new())
                .err()
                .unwrap()
                .code,
            "shell_unsupported"
        );
        for negotiated in [false, true] {
            for kind in [
                "shell.snapshot.v2",
                "endpoint.surface-reuse.v1",
                "endpoint.surface-delta.v2",
                "endpoint.surface-scroll.v1",
            ] {
                assert_eq!(
                    decode_message(&encode((20_u32, kind, "{}")), negotiated, &mut Vec::new())
                        .err()
                        .unwrap()
                        .code,
                    "shell_unsupported"
                );
            }
        }
    }

    #[tokio::test]
    async fn welcome_without_delta_capability_preserves_full_surface_behavior() {
        for capabilities in [
            None,
            Some(vec!["surface_delta.v2"]),
            Some(vec!["surface_delta"]),
        ] {
            let mut welcome = serde_json::json!({
                "generation": 1, "snapshot_codec": SNAPSHOT_CODEC, "surface_codec": SURFACE_CODEC,
                "input_codec": "shell.input.semantic.v1", "blob_codec": "shell.blob.v1",
            });
            if let Some(capabilities) = &capabilities {
                welcome["capabilities"] = serde_json::json!(capabilities);
            }
            let mut connection = connection("boot");
            connection.welcomed = false;
            connection
                .apply(
                    decode(&encode((
                        20_u32,
                        "endpoint.welcome.v1",
                        welcome.to_string(),
                    )))
                    .unwrap(),
                )
                .unwrap();
            let negotiated = capabilities == Some(vec!["surface_delta"]);
            assert_eq!(connection.delta_negotiated, negotiated);
            let fixture = DeltaFixture::default();
            connection
                .apply(decode(&fixture.baseline()).unwrap())
                .unwrap();
            let result = decode_message(
                &fixture.control(),
                connection.delta_negotiated,
                &mut connection.delta_scratch,
            );
            if negotiated {
                assert!(!connection.apply(result.unwrap()).unwrap());
            } else {
                assert_eq!(result.err().unwrap().code, "shell_unsupported");
                assert!(
                    !connection
                        .apply(decode(&empty_surface("boot", 2, None)).unwrap())
                        .unwrap()
                );
            }
        }
    }

    #[tokio::test]
    async fn rejects_cross_endpoint_snapshot_and_surface() {
        let mut connection = connection("expected");
        assert!(
            connection
                .apply(decode(&empty_surface("other", 1, None)).unwrap())
                .is_err()
        );
        let snapshot = r#"{"boot_id":"other","revision":1,"commands":[]}"#;
        assert!(
            connection
                .apply(Message::Snapshot(json(snapshot).unwrap()))
                .is_err()
        );
    }

    #[tokio::test]
    async fn full_surface_closes_popup_but_patch_does_not() {
        let mut connection = connection("boot");
        assert!(
            connection
                .apply(decode(&empty_surface("boot", 1, Some(("popup", "Title")))).unwrap())
                .unwrap()
        );
        let popup = connection.state.popup.as_ref().unwrap();
        assert_eq!(popup.terminal_id, "popup");
        assert_eq!(popup.width, Some(HerdrPopupSize::Cells { value: 30 }));
        assert_eq!(popup.height, Some(HerdrPopupSize::Percent { value: 50 }));
        let patch = encode((19_u32, "boot", 1_u64, 1_u64, 2_u64, 0_u64, 0_u64, false));
        assert!(!connection.apply(decode(&patch).unwrap()).unwrap());
        assert!(connection.state.popup.is_some());
        assert!(
            connection
                .apply(decode(&empty_surface("boot", 3, None)).unwrap())
                .unwrap()
        );
        assert_eq!(connection.state.popup, None);
    }

    #[tokio::test]
    async fn prefix_bindings_track_endpoint_config_reload_without_a_default() {
        let mut connection = connection("boot");
        for (revision, profile, expected, changed) in [
            (
                1,
                Some("[keys]\nprefix = \"ctrl+a\"\nextra_prefixes = [\"f12\", \"alt+b\"]\n"),
                &["ctrl+a", "f12", "alt+b"][..],
                true,
            ),
            (
                2,
                Some("[keys]\nprefix = [\"f12\", \"ctrl+x\"]\n"),
                &["f12", "ctrl+x"][..],
                true,
            ),
            (
                3,
                Some("[keys]\nprefix = [\"f12\", \"ctrl+x\"]\n"),
                &["f12", "ctrl+x"][..],
                false,
            ),
            (4, None, &[][..], true),
        ] {
            let snapshot: Snapshot = json(
                &serde_json::json!({
                    "boot_id": "boot", "revision": revision, "commands": [],
                    "server_keybindings_toml": profile
                })
                .to_string(),
            )
            .unwrap();
            assert_eq!(
                connection.apply(Message::Snapshot(snapshot)).unwrap(),
                changed
            );
            assert_eq!(connection.state.prefix_bindings, expected);
        }
        assert!(
            decode_prefixes(Some("[keys]\nhelp = \"prefix+?\"\n"))
                .unwrap()
                .is_empty()
        );
        assert!(decode_prefixes(Some("[keys]\nprefix = [broken")).is_err());
    }

    #[test]
    fn preserves_unknown_command_actions() {
        let snapshot: Snapshot = json(r#"{"boot_id":"boot","revision":1,"commands":[{"command_id":"future","binding_labels":["Alt-F"],"action":"FutureAction","description":null},{"command_id":"plugin","binding_labels":[],"action":"PluginAction","description":"Run plugin"}]}"#).unwrap();
        let commands: Vec<_> = snapshot
            .commands
            .into_iter()
            .map(Command::into_dto)
            .collect();
        assert_eq!(commands[0].action, HerdrCommandAction::Unknown);
        assert_eq!(commands[0].command_id, "future");
        assert_eq!(commands[0].binding_labels, vec!["Alt-F"]);
        assert_eq!(commands[1].action, HerdrCommandAction::PluginAction);
    }
}
