use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Selects whether Cockpit performs live Herdr inspection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CockpitMode {
    Normal,
    Test,
}

/// Identity and wire versions reported by a Herdr installation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct HerdrIdentity {
    pub version: String,
    pub protocol: u32,
    pub schema_version: u32,
}

/// Result of validating the installed Herdr against Cockpit's requirements.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HerdrCompatibility {
    Compatible {
        identity: HerdrIdentity,
    },
    Incompatible {
        identity: Option<HerdrIdentity>,
        code: String,
        message: String,
    },
    Unavailable {
        code: String,
        message: String,
    },
}

/// Status returned by every Cockpit host transport.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct StatusResponse {
    pub protocol_version: String,
    pub cockpit_version: String,
    pub mode: CockpitMode,
    pub herdr: HerdrCompatibility,
}

/// Stable error envelope used by host transports.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}

/// Repository context associated with a Herdr Space.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct SpaceGitSummary {
    pub repository_key: String,
    pub repository: String,
    pub branch: Option<String>,
    pub checkout_path: String,
    pub is_linked_worktree: bool,
}

/// A summary of a Herdr Space in the current session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct SpaceSummary {
    pub id: String,
    pub label: String,
    pub number: u32,
    pub tab_count: u32,
    pub pane_count: u32,
    pub focused: bool,
    pub agent_status: String,
    pub git: Option<SpaceGitSummary>,
}

/// A summary of a tab in the current session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct TabSummary {
    pub id: String,
    pub space_id: String,
    pub label: String,
    pub number: u32,
    pub pane_count: u32,
    pub focused: bool,
}

/// A summary of a pane in the current session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct PaneSummary {
    pub id: String,
    pub terminal_id: String,
    pub space_id: String,
    pub tab_id: String,
    pub title: Option<String>,
    pub focused: bool,
    pub agent: Option<String>,
    pub agent_status: String,
    #[ts(type = "number")]
    pub revision: u64,
}

/// A summary of an agent attached to a pane.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct AgentSummary {
    pub pane_id: String,
    pub space_id: String,
    pub tab_id: String,
    pub name: String,
    pub status: String,
    pub title: Option<String>,
    pub focused: bool,
    #[ts(type = "number")]
    pub state_change_seq: u64,
}

/// A rectangle in a tab's pane layout.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct LayoutRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A pane and its rectangle within a tab layout.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct LayoutPane {
    pub pane_id: String,
    pub focused: bool,
    pub rect: LayoutRect,
}

/// The layout of a tab and its panes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct TabLayout {
    pub space_id: String,
    pub tab_id: String,
    pub area: LayoutRect,
    pub focused_pane_id: Option<String>,
    pub panes: Vec<LayoutPane>,
    pub zoomed: bool,
}

/// The read-only current Herdr session workbench.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct SessionSnapshotResponse {
    pub session_id: String,
    pub version: String,
    pub protocol: u32,
    pub focused_space_id: Option<String>,
    pub focused_tab_id: Option<String>,
    pub focused_pane_id: Option<String>,
    pub spaces: Vec<SpaceSummary>,
    pub tabs: Vec<TabSummary>,
    pub panes: Vec<PaneSummary>,
    pub layouts: Vec<TabLayout>,
    pub agents: Vec<AgentSummary>,
}

/// Read-only terminal output for a pane.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct PaneOutputResponse {
    pub pane_id: String,
    pub text: String,
    #[ts(type = "number | null")]
    pub revision: Option<u64>,
}

/// A named Herdr session available to Cockpit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct SessionSummary {
    pub id: String,
    pub label: String,
    pub is_default: bool,
    pub running: bool,
}

/// The sessions available to Cockpit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionSummary>,
}

/// A focusable Herdr resource kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum FocusKind {
    Space,
    Tab,
    Pane,
    Agent,
}

/// A request to focus a resource in a session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct FocusRequest {
    pub kind: FocusKind,
    pub target_id: String,
}

/// The result of a focus request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct FocusResponse {
    pub session_id: String,
    pub kind: FocusKind,
    pub target_id: String,
    pub accepted: bool,
}
/// Direction in which a pane is split or inserted beside another pane.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PaneSplitDirection {
    Right,
    Down,
}

/// Direction in which a pane boundary is resized.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PaneResizeDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Requested pane zoom transition.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PaneZoomMode {
    Toggle,
    On,
    Off,
}

/// Typed destination for moving a live pane.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneMoveDestination {
    ExistingTab {
        tab_id: String,
        direction: PaneSplitDirection,
        target_pane_id: Option<String>,
        ratio: Option<f64>,
    },
    NewTab {
        space_id: Option<String>,
        label: Option<String>,
    },
    NewSpace {
        label: Option<String>,
        tab_label: Option<String>,
    },
}

/// A closed set of supported Cockpit resource mutations.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResourceMutationRequest {
    SpaceCreate {
        cwd: Option<String>,
        label: Option<String>,
    },
    SpaceRename {
        space_id: String,
        label: String,
    },
    SpaceMoveBlock {
        space_ids: Vec<String>,
        before_space_id: Option<String>,
    },
    SpaceClose {
        space_id: String,
    },
    TabCreate {
        space_id: String,
        label: Option<String>,
    },
    TabRename {
        tab_id: String,
        label: String,
    },
    TabMove {
        tab_id: String,
        insert_index: u32,
    },
    TabClose {
        tab_id: String,
    },
    PaneSplit {
        pane_id: String,
        direction: PaneSplitDirection,
        ratio: Option<f64>,
    },
    PaneResize {
        pane_id: String,
        direction: PaneResizeDirection,
        amount: f64,
    },
    PaneRename {
        pane_id: String,
        label: Option<String>,
    },
    PaneSwap {
        source_pane_id: String,
        target_pane_id: String,
    },
    PaneMove {
        pane_id: String,
        destination: PaneMoveDestination,
    },
    PaneZoom {
        pane_id: String,
        mode: PaneZoomMode,
    },
    PaneClose {
        pane_id: String,
    },
}

/// Authoritative session state read after a successful resource mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct ResourceMutationResponse {
    pub session_id: String,
    pub snapshot: SessionSnapshotResponse,
}

/// A full replacement snapshot or an explicit session stream failure state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionStreamMessage {
    Snapshot {
        session_id: String,
        generation: u32,
        sequence: u32,
        snapshot: SessionSnapshotResponse,
    },
    Stale {
        session_id: String,
        generation: u32,
        sequence: u32,
        code: String,
        message: String,
    },
    Disconnected {
        session_id: String,
        generation: u32,
        sequence: u32,
        code: String,
        message: String,
    },
}

/// Whether a terminal attachment is read-only or may receive input.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TerminalMode {
    Observe,
    Control,
}

/// A request to attach to a pane's terminal stream.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct TerminalOpenRequest {
    pub session_id: String,
    pub pane_id: String,
    pub mode: TerminalMode,
    pub takeover: bool,
    pub cols: u16,
    pub rows: u16,
}

/// Terminal scroll direction.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TerminalScrollDirection {
    Up,
    Down,
}

/// The user input source for a terminal scroll command.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TerminalScrollSource {
    Wheel,
    PageKey,
}

/// Mouse button forwarded through Herdr's structured input path.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TerminalMouseButton {
    Left,
    Right,
    Middle,
}

/// Mouse event kind forwarded through Herdr's structured input path.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TerminalMouseKind {
    Down,
    Up,
    Drag,
    Moved,
}

/// A command sent to a Herdr terminal stream.
///
/// Input commands intentionally have one shared wire tag. Exactly one of
/// `text` and `bytes` must be present for an input command.
#[derive(Clone, Debug, Eq, PartialEq, TS)]
#[ts(tag = "type")]
pub enum TerminalCommand {
    #[ts(rename = "terminal.input")]
    Input {
        text: Option<String>,
        bytes: Option<String>,
    },
    #[ts(rename = "terminal.resize")]
    Resize {
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    },
    #[ts(rename = "terminal.scroll")]
    Scroll {
        direction: TerminalScrollDirection,
        lines: u32,
        source: TerminalScrollSource,
        column: Option<u16>,
        row: Option<u16>,
        modifiers: u8,
    },
    #[ts(rename = "terminal.mouse")]
    Mouse {
        kind: TerminalMouseKind,
        button: Option<TerminalMouseButton>,
        column: u16,
        row: u16,
        modifiers: u8,
    },
    #[ts(rename = "terminal.release")]
    Release,
}

const MAX_TERMINAL_INPUT_BYTES: usize = 64 * 1024;
const MAX_TERMINAL_INPUT_BASE64_LEN: usize = MAX_TERMINAL_INPUT_BYTES.div_ceil(3) * 4;
const MAX_TERMINAL_SCROLL_LINES: u32 = u16::MAX as u32;

impl TerminalCommand {
    /// Construct a text input command.
    pub fn input_text(text: impl Into<String>) -> Self {
        Self::Input {
            text: Some(text.into()),
            bytes: None,
        }
    }

    /// Construct a base64-encoded bytes input command.
    pub fn input_bytes(bytes: impl Into<String>) -> Self {
        Self::Input {
            text: None,
            bytes: Some(bytes.into()),
        }
    }

    /// Validate the command's input shape before sending it to Herdr.
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Input { text, bytes } => {
                if text.is_some() == bytes.is_some() {
                    return Err("terminal.input requires exactly one of text or bytes");
                }
                if let Some(text) = text
                    && text.len() > MAX_TERMINAL_INPUT_BYTES
                {
                    return Err("terminal.input text exceeds 64 KiB");
                }
                if let Some(bytes) = bytes
                    && bytes.len() > MAX_TERMINAL_INPUT_BASE64_LEN
                {
                    return Err("terminal.input bytes exceed the 64 KiB encoding limit");
                }
            }
            Self::Resize {
                cols,
                rows,
                cell_width_px,
                cell_height_px,
            } => {
                if *cols == 0 || *rows == 0 {
                    return Err("terminal.resize cols and rows must be positive");
                }
                if *cell_width_px > u16::MAX as u32 || *cell_height_px > u16::MAX as u32 {
                    return Err("terminal.resize cell dimensions exceed u16");
                }
            }
            Self::Scroll { lines, .. } if !(1..=MAX_TERMINAL_SCROLL_LINES).contains(lines) => {
                return Err("terminal.scroll lines must be between 1 and 65535");
            }
            Self::Mouse { kind, button, .. }
                if matches!(kind, TerminalMouseKind::Moved) == button.is_some() =>
            {
                return Err("terminal.mouse button does not match event kind");
            }
            Self::Scroll { .. } | Self::Mouse { .. } | Self::Release => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
enum TerminalCommandWire {
    #[serde(rename = "terminal.input")]
    Input {
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        bytes: Option<String>,
    },
    #[serde(rename = "terminal.resize")]
    Resize {
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    },
    #[serde(rename = "terminal.scroll")]
    Scroll {
        direction: TerminalScrollDirection,
        lines: u32,
        source: TerminalScrollSource,
        #[serde(default)]
        column: Option<u16>,
        #[serde(default)]
        row: Option<u16>,
        modifiers: u8,
    },
    #[serde(rename = "terminal.mouse")]
    Mouse {
        kind: TerminalMouseKind,
        #[serde(default)]
        button: Option<TerminalMouseButton>,
        column: u16,
        row: u16,
        modifiers: u8,
    },
    #[serde(rename = "terminal.release")]
    Release,
}

impl TryFrom<TerminalCommandWire> for TerminalCommand {
    type Error = &'static str;

    fn try_from(command: TerminalCommandWire) -> Result<Self, Self::Error> {
        let command = match command {
            TerminalCommandWire::Input { text, bytes } => Self::Input { text, bytes },
            TerminalCommandWire::Resize {
                cols,
                rows,
                cell_width_px,
                cell_height_px,
            } => Self::Resize {
                cols,
                rows,
                cell_width_px,
                cell_height_px,
            },
            TerminalCommandWire::Scroll {
                direction,
                lines,
                source,
                column,
                row,
                modifiers,
            } => Self::Scroll {
                direction,
                lines,
                source,
                column,
                row,
                modifiers,
            },
            TerminalCommandWire::Mouse {
                kind,
                button,
                column,
                row,
                modifiers,
            } => Self::Mouse {
                kind,
                button,
                column,
                row,
                modifiers,
            },
            TerminalCommandWire::Release => Self::Release,
        };
        command.validate()?;
        Ok(command)
    }
}

impl Serialize for TerminalCommand {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        let wire = match self {
            Self::Input { text, bytes } => TerminalCommandWire::Input {
                text: text.clone(),
                bytes: bytes.clone(),
            },
            Self::Resize {
                cols,
                rows,
                cell_width_px,
                cell_height_px,
            } => TerminalCommandWire::Resize {
                cols: *cols,
                rows: *rows,
                cell_width_px: *cell_width_px,
                cell_height_px: *cell_height_px,
            },
            Self::Scroll {
                direction,
                lines,
                source,
                column,
                row,
                modifiers,
            } => TerminalCommandWire::Scroll {
                direction: *direction,
                lines: *lines,
                source: *source,
                column: *column,
                row: *row,
                modifiers: *modifiers,
            },
            Self::Mouse {
                kind,
                button,
                column,
                row,
                modifiers,
            } => TerminalCommandWire::Mouse {
                kind: *kind,
                button: *button,
                column: *column,
                row: *row,
                modifiers: *modifiers,
            },
            Self::Release => TerminalCommandWire::Release,
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TerminalCommand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = TerminalCommandWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// Terminal stream ownership state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOwnershipState {
    Pending,
    Observing,
    Owned,
    Conflict,
    Released,
    Lost,
}

/// A terminal stream update.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalStreamMessage {
    Ownership {
        session_id: String,
        pane_id: String,
        stream_id: String,
        state: TerminalOwnershipState,
        message: Option<String>,
    },
    Frame {
        session_id: String,
        pane_id: String,
        stream_id: String,
        seq: String,
        encoding: String,
        width: u16,
        height: u16,
        full: bool,
        bytes: String,
    },
    Closed {
        session_id: String,
        pane_id: String,
        stream_id: String,
        reason: String,
    },
    Disconnected {
        session_id: String,
        pane_id: String,
        stream_id: String,
        code: String,
        message: String,
    },
    Error {
        session_id: String,
        pane_id: String,
        stream_id: String,
        code: String,
        message: String,
    },
}
