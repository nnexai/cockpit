//! Generation-1 Herdr endpoint metadata. The binary layout is frozen in Herdr
//! v0.9.2's protocol/wire.rs; only commands and popup metadata are retained.
use std::path::Path;
use std::time::Duration;

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
            "mouse_capture": false, "surface_reuse": false, "surface_delta": false,
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
        let message = decode_message(bytes)?;
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
                // Patches carry no popup metadata; only complete surfaces can close it.
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
    popup: Option<HerdrPopup>,
}
enum Message {
    Welcome(Welcome),
    Snapshot(Snapshot),
    Surface(SurfaceMetadata),
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
        Ok(count)
    }
    fn frame(&mut self) -> Result<(), InspectionError> {
        let cells = self.cells()?;
        let width: u16 = self.value()?;
        let height: u16 = self.value()?;
        if cells != usize::from(width) * usize::from(height) {
            return Err(malformed("Shell grid dimensions do not match its cells"));
        }
        self.cursor()?;
        for _ in 0..self.count()? {
            self.text()?;
        }
        self.blob()
    }
    fn rect(&mut self) -> Result<(), InspectionError> {
        for _ in 0..4 {
            let _: u16 = self.value()?;
        }
        Ok(())
    }
    fn panes(&mut self) -> Result<(), InspectionError> {
        for _ in 0..self.count()? {
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
        for _ in 0..self.count()? {
            self.variant(2)?;
            let _: u16 = self.value()?;
            self.rect()?;
            self.rect()?;
            for _ in 0..self.count()? {
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
    fn popup(&mut self) -> Result<Option<HerdrPopup>, InspectionError> {
        if !self.flag()? {
            return Ok(None);
        }
        let popup = HerdrPopup {
            terminal_id: self.text()?.to_owned(),
            title: self.text()?.to_owned(),
            width: self.popup_size()?,
            height: self.popup_size()?,
        };
        self.frame()?;
        self.flag()?;
        self.flag()?;
        let _: u32 = self.value()?;
        let _: u32 = self.value()?;
        Ok(Some(popup))
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
        for _ in 0..self.count()? {
            self.asset_key()?;
            self.blob()?;
        }
        for _ in 0..self.count()? {
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
        for _ in 0..self.count()? {
            self.asset_key()?;
        }
        Ok(())
    }
}

fn decode_message(bytes: &[u8]) -> Result<Message, InspectionError> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME_SIZE {
        return Err(malformed("Invalid shell frame size"));
    }
    let mut decoder = Decoder { bytes };
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
        13 => {
            let boot_id = decoder.text()?.to_owned();
            let projection_revision = decoder.number()?;
            let surface_revision = decoder.number()?;
            decoder.frame()?;
            decoder.panes()?;
            decoder.splits()?;
            let popup = decoder.popup()?;
            decoder.graphics()?;
            Message::Surface(SurfaceMetadata {
                boot_id,
                projection_revision,
                surface_revision,
                popup,
            })
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(value: impl serde::Serialize) -> Vec<u8> {
        bincode::serde::encode_to_vec(value, bincode::config::standard()).unwrap()
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
        }
    }

    #[test]
    fn rejects_malformed_and_oversized_frames() {
        assert!(frame_length((MAX_FRAME_SIZE as u32 + 1).to_le_bytes()).is_err());
        assert!(frame_length(0_u32.to_le_bytes()).is_err());
        let valid = empty_surface("boot", 1, Some(("popup", "Title")));
        for end in 0..valid.len() {
            assert!(decode_message(&valid[..end]).is_err());
        }
        let mut trailing = valid;
        trailing.push(0);
        assert!(decode_message(&trailing).is_err());
        assert!(decode_message(&encode((13_u32, "boot", 1_u64, 1_u64, u64::MAX))).is_err());
        assert!(decode_message(&encode(21_u32)).is_err());
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
    fn rejects_unnegotiated_surface_extensions() {
        for kind in [
            "shell.snapshot.v2",
            "endpoint.surface-reuse.v1",
            "endpoint.surface-delta.v1",
            "endpoint.surface-scroll.v1",
        ] {
            assert!(decode_message(&encode((20_u32, kind, "{}"))).is_err());
        }
    }

    #[tokio::test]
    async fn rejects_cross_endpoint_snapshot_and_surface() {
        let mut connection = connection("expected");
        assert!(
            connection
                .apply(decode_message(&empty_surface("other", 1, None)).unwrap())
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
                .apply(decode_message(&empty_surface("boot", 1, Some(("popup", "Title")))).unwrap())
                .unwrap()
        );
        let popup = connection.state.popup.as_ref().unwrap();
        assert_eq!(popup.terminal_id, "popup");
        assert_eq!(popup.width, Some(HerdrPopupSize::Cells { value: 30 }));
        assert_eq!(popup.height, Some(HerdrPopupSize::Percent { value: 50 }));
        let patch = encode((19_u32, "boot", 1_u64, 1_u64, 2_u64, 0_u64, 0_u64, false));
        assert!(!connection.apply(decode_message(&patch).unwrap()).unwrap());
        assert!(connection.state.popup.is_some());
        assert!(
            connection
                .apply(decode_message(&empty_surface("boot", 3, None)).unwrap())
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
