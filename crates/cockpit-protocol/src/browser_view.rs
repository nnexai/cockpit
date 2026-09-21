use crate::browser::BrowserTarget;
use crate::browser_feedback::{
    BrowserAnnotationKind, BrowserCaptureSaved, BrowserCaptureSubmission, BrowserElementEvidence,
    BrowserPoint, BrowserRect,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Maximum size accepted for any opaque browser-view identity.
pub const BROWSER_VIEW_MAX_ID_BYTES: usize = 512;
/// Maximum size accepted for user-provided text sent to a page in one command.
pub const BROWSER_VIEW_MAX_TEXT_BYTES: usize = 64 * 1024;
/// Maximum size accepted for a navigation URL before it reaches a browser runtime.
pub const BROWSER_VIEW_MAX_URL_BYTES: usize = 8 * 1024;
/// Maximum number of opaque file selections accepted for one chooser response.
pub const BROWSER_VIEW_MAX_FILE_SELECTIONS: usize = 64;
/// Maximum number of annotations named by one capture operation.
pub const BROWSER_VIEW_MAX_CAPTURE_ANNOTATIONS: usize = 64;
/// Maximum number of points accepted for one freehand annotation.
pub const BROWSER_VIEW_MAX_ANNOTATION_POINTS: usize = 8_192;
/// Maximum UTF-16 code units retained for one annotation note editor.
pub const BROWSER_VIEW_MAX_NOTE_TEXT_CODE_UNITS: usize = 4_000;
/// Maximum dimensions accepted from the frame transport.
pub const BROWSER_VIEW_FRAME_MAX_WIDTH: u32 = 2_560;
pub const BROWSER_VIEW_FRAME_MAX_HEIGHT: u32 = 1_600;
pub const BROWSER_VIEW_FRAME_MAX_PIXELS: u32 =
    BROWSER_VIEW_FRAME_MAX_WIDTH * BROWSER_VIEW_FRAME_MAX_HEIGHT;
pub const BROWSER_VIEW_FRAME_MAX_JPEG_BYTES: u32 = 6 * 1024 * 1024;

/// Big-endian `IBFV` frame-envelope v2 constants.
pub const BROWSER_VIEW_FRAME_V2_MAGIC: u32 = 0x4942_4656;
pub const BROWSER_VIEW_FRAME_V2_VERSION: u16 = 2;
pub const BROWSER_VIEW_FRAME_V2_HEADER_BYTES: u16 = 96;
pub const BROWSER_VIEW_FRAME_V2_MAGIC_OFFSET: u16 = 0;
pub const BROWSER_VIEW_FRAME_V2_VERSION_OFFSET: u16 = 4;
pub const BROWSER_VIEW_FRAME_V2_HEADER_BYTES_OFFSET: u16 = 6;
pub const BROWSER_VIEW_FRAME_V2_STREAM_EPOCH_OFFSET: u16 = 8;
pub const BROWSER_VIEW_FRAME_V2_SEQUENCE_OFFSET: u16 = 16;
pub const BROWSER_VIEW_FRAME_V2_DOCUMENT_GENERATION_OFFSET: u16 = 24;
pub const BROWSER_VIEW_FRAME_V2_VIEWPORT_REVISION_OFFSET: u16 = 32;
pub const BROWSER_VIEW_FRAME_V2_IMAGE_WIDTH_OFFSET: u16 = 40;
pub const BROWSER_VIEW_FRAME_V2_IMAGE_HEIGHT_OFFSET: u16 = 44;
pub const BROWSER_VIEW_FRAME_V2_VIEWPORT_WIDTH_OFFSET: u16 = 48;
pub const BROWSER_VIEW_FRAME_V2_VIEWPORT_HEIGHT_OFFSET: u16 = 52;
pub const BROWSER_VIEW_FRAME_V2_VIEWPORT_OFFSET_X_OFFSET: u16 = 56;
pub const BROWSER_VIEW_FRAME_V2_VIEWPORT_OFFSET_Y_OFFSET: u16 = 60;
pub const BROWSER_VIEW_FRAME_V2_SCROLL_X_OFFSET: u16 = 64;
pub const BROWSER_VIEW_FRAME_V2_SCROLL_Y_OFFSET: u16 = 68;
pub const BROWSER_VIEW_FRAME_V2_CAPTURE_TIMESTAMP_MICROS_OFFSET: u16 = 72;
pub const BROWSER_VIEW_FRAME_V2_JPEG_LENGTH_OFFSET: u16 = 80;
pub const BROWSER_VIEW_FRAME_V2_FLAGS_OFFSET: u16 = 84;
pub const BROWSER_VIEW_FRAME_V2_RESERVED_OFFSET: u16 = 88;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewPresentation {
    Split,
    BrowserOnly,
}

/// The controller-owned browser viewport requested by a Cockpit presentation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewViewportRequest {
    pub css_width: u32,
    pub css_height: u32,
    pub device_pixel_ratio: f64,
}

impl BrowserViewViewportRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.css_width == 0 || self.css_height == 0 {
            return Err("browser view viewport dimensions must be positive");
        }
        if self.css_width > BROWSER_VIEW_FRAME_MAX_WIDTH
            || self.css_height > BROWSER_VIEW_FRAME_MAX_HEIGHT
        {
            return Err("browser view viewport dimensions exceed frame bounds");
        }
        if !self.device_pixel_ratio.is_finite() || self.device_pixel_ratio <= 0.0 {
            return Err("browser view device pixel ratio must be finite and positive");
        }
        Ok(())
    }
}

/// Attach a Cockpit presentation to the browser selected by fresh Herdr authority.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewOpenRequest {
    pub target: BrowserTarget,
    pub client_id: String,
    pub presentation: BrowserViewPresentation,
    pub viewport: BrowserViewViewportRequest,
    pub takeover: bool,
}

impl BrowserViewOpenRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_id(&self.client_id)?;
        self.viewport.validate()
    }
}

/// Immutable association and stream identity issued for one attached view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewIdentity {
    pub association_key: String,
    pub browser_incarnation: String,
    pub view_id: String,
    #[ts(type = "number")]
    pub stream_epoch: u64,
}

/// The real browser target kind, not a Cockpit or Herdr tab kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewTargetKind {
    Page,
    Popup,
    Background,
    Internal,
}

/// A browser target available in the attached association.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewTargetSummary {
    pub target_id: String,
    pub kind: BrowserViewTargetKind,
    pub title: String,
    pub url: String,
    pub order: u32,
    pub opener_target_id: Option<String>,
    pub can_close: bool,
}

/// A document is distinct from a URL: reloading the same URL changes generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewDocumentState {
    pub target_id: String,
    pub frame_id: String,
    #[ts(type = "number")]
    pub document_generation: u64,
    #[ts(type = "number")]
    pub frame_generation: u64,
}

/// Geometry that is authoritative for a displayed document and frame transform.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewViewportState {
    #[ts(type = "number")]
    pub viewport_revision: u64,
    pub css_width: f64,
    pub css_height: f64,
    pub visual_offset_x: f64,
    pub visual_offset_y: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub visual_scale: f64,
    pub page_scale: f64,
    pub device_pixel_ratio: f64,
    pub geometry_fresh: bool,
}

/// Page navigation and confirmed history state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewNavigationState {
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub requested_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewCursor {
    Default,
    Pointer,
    Text,
    Crosshair,
    Move,
    NotAllowed,
    Wait,
    Grab,
    Grabbing,
    Cell,
    Help,
    Progress,
    ZoomIn,
    ZoomOut,
    ColumnResize,
    RowResize,
    EastResize,
    WestResize,
    NorthResize,
    SouthResize,
    NortheastResize,
    NorthwestResize,
    SoutheastResize,
    SouthwestResize,
}

/// Cursor information is versioned against the stationary pointer sample it describes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewCursorState {
    pub cursor: BrowserViewCursor,
    #[ts(type = "number")]
    pub pointer_sample_sequence: u64,
    pub target_id: String,
    #[ts(type = "number")]
    pub document_generation: u64,
    #[ts(type = "number")]
    pub viewport_revision: u64,
}

/// Page focus information deliberately excludes page values and selection contents.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewFocusState {
    pub page_focused: bool,
    pub editable: bool,
    pub selection_available: bool,
    pub composition_active: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewBlockerKind {
    Dialog,
    FileChooser,
    Download,
    Permission,
    Unsupported,
}

/// A visible browser facility that would otherwise block the image-only surface.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewBlocker {
    pub blocker_id: String,
    pub kind: BrowserViewBlockerKind,
    pub message: String,
    pub default_prompt: Option<String>,
    pub target_id: String,
    #[ts(type = "number")]
    pub document_generation: u64,
    pub cancellable: bool,
}

/// Capability states are explicit so unavailable native-browser facilities do not hang silently.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewCapability {
    Supported,
    Unsupported,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewCapabilities {
    pub pointer_input: BrowserViewCapability,
    pub keyboard_input: BrowserViewCapability,
    pub text_input: BrowserViewCapability,
    pub composition_input: BrowserViewCapability,
    pub clipboard_read: BrowserViewCapability,
    pub clipboard_write: BrowserViewCapability,
    pub dialogs: BrowserViewCapability,
    pub file_chooser: BrowserViewCapability,
    pub downloads: BrowserViewCapability,
    pub permissions: BrowserViewCapability,
    pub inspection: BrowserViewCapability,
    pub capture: BrowserViewCapability,
    pub drafts: BrowserViewCapability,
    pub audio: BrowserViewCapability,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewControlStatus {
    Observing,
    Pending,
    Controlled,
    Revoked,
    Lost,
}

/// Lease ownership is local to Cockpit and does not claim exclusive page control from agents.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewControlState {
    pub status: BrowserViewControlStatus,
    pub controller_view_id: Option<String>,
    #[ts(type = "number")]
    pub lease_generation: u64,
    #[ts(type = "number")]
    pub next_input_sequence: u64,
    pub can_take_control: bool,
}

/// Public description of the fixed v2 binary frame envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewFrameEnvelopeV2 {
    pub magic: u32,
    pub version: u16,
    pub header_bytes: u16,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u32,
    pub max_jpeg_bytes: u32,
}

impl Default for BrowserViewFrameEnvelopeV2 {
    fn default() -> Self {
        Self {
            magic: BROWSER_VIEW_FRAME_V2_MAGIC,
            version: BROWSER_VIEW_FRAME_V2_VERSION,
            header_bytes: BROWSER_VIEW_FRAME_V2_HEADER_BYTES,
            max_width: BROWSER_VIEW_FRAME_MAX_WIDTH,
            max_height: BROWSER_VIEW_FRAME_MAX_HEIGHT,
            max_pixels: BROWSER_VIEW_FRAME_MAX_PIXELS,
            max_jpeg_bytes: BROWSER_VIEW_FRAME_MAX_JPEG_BYTES,
        }
    }
}

/// A short-lived opaque grant for this view's frame lane. It is not a control grant.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewFrameGrant {
    pub view_id: String,
    #[ts(type = "number")]
    pub stream_epoch: u64,
    pub grant: String,
    pub expires_at: String,
    pub envelope: BrowserViewFrameEnvelopeV2,
}

/// Immutable metadata paired with one JPEG frame. The payload is not represented in JSON DTOs.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewFrameDescriptor {
    pub target_id: String,
    #[ts(type = "number")]
    pub stream_epoch: u64,
    #[ts(type = "number")]
    pub frame_sequence: u64,
    #[ts(type = "number")]
    pub document_generation: u64,
    #[ts(type = "number")]
    pub viewport_revision: u64,
    pub image_width: u32,
    pub image_height: u32,
    pub viewport_css_width: f64,
    pub viewport_css_height: f64,
    pub viewport_offset_x: f64,
    pub viewport_offset_y: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    #[ts(type = "number")]
    pub capture_timestamp_micros: u64,
    pub jpeg_length: u32,
}

impl BrowserViewFrameDescriptor {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.image_width == 0 || self.image_height == 0 {
            return Err("browser frame dimensions must be positive");
        }
        if self.image_width > BROWSER_VIEW_FRAME_MAX_WIDTH
            || self.image_height > BROWSER_VIEW_FRAME_MAX_HEIGHT
            || self.image_width.saturating_mul(self.image_height) > BROWSER_VIEW_FRAME_MAX_PIXELS
        {
            return Err("browser frame dimensions exceed bounds");
        }
        if self.jpeg_length == 0 || self.jpeg_length > BROWSER_VIEW_FRAME_MAX_JPEG_BYTES {
            return Err("browser frame JPEG length exceeds bounds");
        }
        Ok(())
    }
}

/// Atomic baseline for an ordered browser-view metadata subscription.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewSnapshot {
    pub identity: BrowserViewIdentity,
    #[ts(type = "number")]
    pub metadata_sequence: u64,
    pub targets: Vec<BrowserViewTargetSummary>,
    pub displayed_target_id: Option<String>,
    pub document: Option<BrowserViewDocumentState>,
    pub viewport: Option<BrowserViewViewportState>,
    pub navigation: Option<BrowserViewNavigationState>,
    pub cursor: Option<BrowserViewCursorState>,
    pub focus: BrowserViewFocusState,
    pub blocker: Option<BrowserViewBlocker>,
    pub capabilities: BrowserViewCapabilities,
    pub control: BrowserViewControlState,
    pub frame_grant: Option<BrowserViewFrameGrant>,
}

/// Metadata common to every member of the closed event union.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewEventMetadata {
    pub view_id: String,
    #[ts(type = "number")]
    pub stream_epoch: u64,
    /// Sequence is contiguous within one subscription; a gap requires a new snapshot.
    #[ts(type = "number")]
    pub metadata_sequence: u64,
}

/// Ordered, closed browser-view metadata event vocabulary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserViewEvent {
    Attached {
        metadata: BrowserViewEventMetadata,
        snapshot: BrowserViewSnapshot,
    },
    TargetsChanged {
        metadata: BrowserViewEventMetadata,
        targets: Vec<BrowserViewTargetSummary>,
        displayed_target_id: Option<String>,
    },
    DocumentChanged {
        metadata: BrowserViewEventMetadata,
        document: Option<BrowserViewDocumentState>,
    },
    ViewportChanged {
        metadata: BrowserViewEventMetadata,
        viewport: Option<BrowserViewViewportState>,
    },
    NavigationChanged {
        metadata: BrowserViewEventMetadata,
        navigation: Option<BrowserViewNavigationState>,
    },
    CursorChanged {
        metadata: BrowserViewEventMetadata,
        cursor: Option<BrowserViewCursorState>,
    },
    FocusChanged {
        metadata: BrowserViewEventMetadata,
        focus: BrowserViewFocusState,
    },
    BlockerChanged {
        metadata: BrowserViewEventMetadata,
        blocker: Option<BrowserViewBlocker>,
    },
    CapabilitiesChanged {
        metadata: BrowserViewEventMetadata,
        capabilities: BrowserViewCapabilities,
    },
    ControlChanged {
        metadata: BrowserViewEventMetadata,
        control: BrowserViewControlState,
    },
    FrameDescriptor {
        metadata: BrowserViewEventMetadata,
        descriptor: BrowserViewFrameDescriptor,
    },
    FrameTransportRevoked {
        metadata: BrowserViewEventMetadata,
        code: String,
        message: String,
    },
    Failed {
        metadata: BrowserViewEventMetadata,
        code: String,
        message: String,
    },
    Closed {
        metadata: BrowserViewEventMetadata,
        reason: String,
    },
}

/// The target/document/frame presentation proof needed before issuing a location-sensitive command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewLocation {
    pub target_id: String,
    #[ts(type = "number")]
    pub document_generation: u64,
    #[ts(type = "number")]
    pub viewport_revision: u64,
    #[ts(type = "number")]
    pub presented_frame_sequence: u64,
    #[ts(type = "number")]
    pub lease_generation: u64,
}

/// Browser target/document identity for commands that do not use image coordinates.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewDocumentCommandContext {
    pub target_id: String,
    #[ts(type = "number")]
    pub document_generation: u64,
    #[ts(type = "number")]
    pub lease_generation: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewPointerKind {
    Move,
    Down,
    Up,
    Cancel,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewPointerButton {
    Left,
    Middle,
    Right,
}

/// A pointer event expressed in the presented frame's viewport CSS coordinates.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewPointerInput {
    pub kind: BrowserViewPointerKind,
    pub button: Option<BrowserViewPointerButton>,
    pub x: f64,
    pub y: f64,
    pub buttons: u8,
    pub modifiers: u8,
    pub click_count: u8,
    #[ts(type = "number")]
    pub input_sequence: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewWheelInput {
    pub x: f64,
    pub y: f64,
    pub delta_x_css: f64,
    pub delta_y_css: f64,
    pub modifiers: u8,
    #[ts(type = "number")]
    pub input_sequence: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewKeyKind {
    Down,
    Up,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewKeyboardInput {
    pub kind: BrowserViewKeyKind,
    pub key: String,
    pub code: String,
    pub location: u8,
    pub modifiers: u8,
    pub repeat: bool,
    #[ts(type = "number")]
    pub input_sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewTextInput {
    pub text: String,
    #[ts(type = "number")]
    pub input_sequence: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewCompositionKind {
    Start,
    Update,
    Commit,
    Cancel,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewCompositionInput {
    pub kind: BrowserViewCompositionKind,
    pub text: String,
    #[ts(type = "number")]
    pub input_sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserViewClipboardCommand {
    Paste { text: String },
    Copy,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserViewNavigationCommand {
    Navigate { url: String },
    Back,
    Forward,
    Reload,
    Stop,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserViewTabCommand {
    Select { target_id: String },
    Create { url: Option<String> },
    Close { target_id: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserViewDialogCommand {
    Accept { text: Option<String> },
    Dismiss,
}

/// File chooser selections are opaque IDs supplied by the local validated file adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserViewFileCommand {
    Choose { selection_ids: Vec<String> },
    Cancel,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserViewDownloadCommand {
    Accept,
    Cancel,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewPermissionDecision {
    Allow,
    Deny,
    Cancel,
}

/// The selected outcome for a visible permission request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewPermissionCommand {
    pub decision: BrowserViewPermissionDecision,
}

/// Public evidence and geometry from a narrow point inspection.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewInspectResult {
    pub location: BrowserViewLocation,
    pub frame_id: String,
    #[ts(type = "number")]
    pub frame_generation: u64,
    /// None identifies an explicit frame-bound local inspection sample.
    #[ts(type = "number | null")]
    pub pointer_sample_sequence: Option<u64>,
    pub bounds: Option<BrowserRect>,
    pub evidence: Option<BrowserElementEvidence>,
    pub inspectable: bool,
    pub freshness: BrowserViewInspectionFreshness,
    pub limitation: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserViewInspectionFreshness {
    Fresh,
    ReviewRequired,
    Stale,
    Unavailable,
}

/// Inspection never accepts arbitrary scripts or returns internal page handles.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewInspectCommand {
    pub location: BrowserViewLocation,
    /// None supplies a new read-only local point, without moving the remote pointer.
    #[ts(type = "number | null")]
    pub pointer_sample_sequence: Option<u64>,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewCaptureCommand {
    pub location: BrowserViewLocation,
    pub draft_id: String,
    #[ts(type = "number")]
    pub draft_revision: u64,
    pub annotation_ids: Vec<String>,
    pub capture_as_shown: bool,
}

/// A draft annotation uses only public evidence; live inspection handles never enter durable DTOs.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewDraftAnnotation {
    pub id: String,
    pub kind: BrowserAnnotationKind,
    pub color: String,
    pub points: Vec<BrowserPoint>,
    pub bounds: Option<BrowserRect>,
    pub evidence: Option<BrowserElementEvidence>,
    pub comment: Option<String>,
}

/// Small persisted browser-local state for restoring a draft editor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewDraftEditorState {
    pub selected_annotation_id: Option<String>,
    pub notes_open: bool,
    #[serde(default)]
    pub note_annotation_id: Option<String>,
    #[serde(default)]
    pub note_text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewDraftState {
    pub draft_id: String,
    pub target_id: String,
    #[ts(type = "number")]
    pub document_generation: u64,
    #[ts(type = "number")]
    pub revision: u64,
    pub annotations: Vec<BrowserViewDraftAnnotation>,
    pub freshness: BrowserViewInspectionFreshness,
    pub stale: bool,
    pub editor: BrowserViewDraftEditorState,
}

/// Recovery information is scoped to the current browser association, while
/// each entry remains pinned to its original target and document generation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewDraftInventory {
    pub drafts: Vec<BrowserViewDraftState>,
    pub active_draft_limit: usize,
    pub pending_capture: Option<BrowserViewPendingCapture>,
}

/// The payload itself stays owner-persisted. Recovery callers receive the
/// original association/incarnation and selected annotation IDs, never a duplicate PNG blob.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewPendingCapture {
    pub association_key: String,
    pub browser_incarnation: String,
    pub capture_id: String,
    pub draft_id: String,
    #[ts(type = "number")]
    pub draft_revision: u64,
    pub annotation_ids: Vec<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BrowserViewCaptureOutcome {
    Absent,
    Pending {
        pending: BrowserViewPendingCapture,
    },
    Saved {
        saved: BrowserCaptureSaved,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserViewDraftCommand {
    List,
    Open {
        draft_id: Option<String>,
    },
    UpsertAnnotation {
        annotation: BrowserViewDraftAnnotation,
    },
    RemoveAnnotation {
        annotation_id: String,
    },
    Clear,
    Discard,
    /// Persists the exact composed PNG before the store attempt. A later retry
    /// always submits the same capture ID and bytes.
    SaveCapture {
        submission: BrowserCaptureSubmission,
        annotation_ids: Vec<String>,
        provenance: crate::browser_feedback::BrowserInlineCaptureProvenance,
    },
    RetryPending,
    DiscardPending,
}

/// Recovery is available without an attached inline view. The target still
/// resolves through fresh Herdr authority before Cockpit reads or changes the
/// association-owned records.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserDraftRecoveryAction {
    List,
    RetryPending,
    DiscardPending,
    SetEditor {
        draft_id: String,
        #[ts(type = "number")]
        expected_revision: u64,
        editor: BrowserViewDraftEditorState,
    },
    UpsertAnnotation {
        draft_id: String,
        #[ts(type = "number")]
        expected_revision: u64,
        annotation: BrowserViewDraftAnnotation,
    },
    RemoveAnnotation {
        draft_id: String,
        #[ts(type = "number")]
        expected_revision: u64,
        annotation_id: String,
    },
    DiscardDraft {
        draft_id: String,
        #[ts(type = "number")]
        expected_revision: u64,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserDraftRecoveryRequest {
    pub target: BrowserTarget,
    pub action: BrowserDraftRecoveryAction,
}

impl BrowserDraftRecoveryRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        match &self.action {
            BrowserDraftRecoveryAction::SetEditor { draft_id, editor, .. } => {
                validate_id(draft_id)?;
                validate_draft_editor(editor)
            }
            BrowserDraftRecoveryAction::UpsertAnnotation { draft_id, annotation, .. } => {
                validate_id(draft_id)?;
                if annotation.points.len() > BROWSER_VIEW_MAX_ANNOTATION_POINTS {
                    return Err("browser draft annotation points exceed bounds");
                }
                validate_id(&annotation.id)
            }
            BrowserDraftRecoveryAction::RemoveAnnotation { draft_id, annotation_id, .. } => {
                validate_id(draft_id)?;
                validate_id(annotation_id)
            }
            BrowserDraftRecoveryAction::DiscardDraft { draft_id, .. } => validate_id(draft_id),
            BrowserDraftRecoveryAction::List
            | BrowserDraftRecoveryAction::RetryPending
            | BrowserDraftRecoveryAction::DiscardPending => Ok(()),
        }
    }
}

/// Typed browser-view commands. This is deliberately not a general page automation API.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserViewCommand {
    TakeControl {
        viewport: BrowserViewViewportRequest,
    },
    ReleaseControl {
        #[ts(type = "number")]
        lease_generation: u64,
    },
    Detach,
    Resize {
        context: BrowserViewDocumentCommandContext,
        viewport: BrowserViewViewportRequest,
    },
    Pointer {
        location: BrowserViewLocation,
        input: BrowserViewPointerInput,
    },
    Wheel {
        location: BrowserViewLocation,
        input: BrowserViewWheelInput,
    },
    Keyboard {
        context: BrowserViewDocumentCommandContext,
        input: BrowserViewKeyboardInput,
    },
    Text {
        context: BrowserViewDocumentCommandContext,
        input: BrowserViewTextInput,
    },
    Composition {
        context: BrowserViewDocumentCommandContext,
        input: BrowserViewCompositionInput,
    },
    Clipboard {
        context: BrowserViewDocumentCommandContext,
        command: BrowserViewClipboardCommand,
    },
    Navigation {
        context: BrowserViewDocumentCommandContext,
        command: BrowserViewNavigationCommand,
    },
    Tab {
        command: BrowserViewTabCommand,
    },
    Dialog {
        blocker_id: String,
        command: BrowserViewDialogCommand,
    },
    File {
        blocker_id: String,
        command: BrowserViewFileCommand,
    },
    Download {
        blocker_id: String,
        command: BrowserViewDownloadCommand,
    },
    Permission {
        blocker_id: String,
        command: BrowserViewPermissionCommand,
    },
    Inspect {
        command: BrowserViewInspectCommand,
    },
    Capture {
        command: BrowserViewCaptureCommand,
    },
    Draft {
        context: BrowserViewDocumentCommandContext,
        draft_id: Option<String>,
        #[ts(type = "number | null")]
        expected_revision: Option<u64>,
        command: BrowserViewDraftCommand,
    },
}

/// Every browser-view command is bound to one view stream and client-generated request identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewCommandRequest {
    pub view_id: String,
    #[ts(type = "number")]
    pub stream_epoch: u64,
    pub request_id: String,
    pub command: BrowserViewCommand,
}

impl BrowserViewCommandRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_id(&self.view_id)?;
        validate_id(&self.request_id)?;
        match &self.command {
            BrowserViewCommand::Navigation {
                command: BrowserViewNavigationCommand::Navigate { url },
                ..
            } => validate_browser_url(url),
            BrowserViewCommand::TakeControl { viewport } => viewport.validate(),
            BrowserViewCommand::Resize { viewport, .. } => viewport.validate(),
            BrowserViewCommand::Text { input, .. } => validate_text(&input.text),
            BrowserViewCommand::Composition { input, .. } => validate_text(&input.text),
            BrowserViewCommand::Clipboard {
                command: BrowserViewClipboardCommand::Paste { text },
                ..
            } => validate_text(text),
            BrowserViewCommand::File {
                command: BrowserViewFileCommand::Choose { selection_ids },
                ..
            } if selection_ids.len() > BROWSER_VIEW_MAX_FILE_SELECTIONS => {
                Err("browser view file selection count exceeds bounds")
            }
            BrowserViewCommand::Capture { command }
                if command.annotation_ids.len() > BROWSER_VIEW_MAX_CAPTURE_ANNOTATIONS =>
            {
                Err("browser view capture annotation count exceeds bounds")
            }
            BrowserViewCommand::Draft {
                command: BrowserViewDraftCommand::UpsertAnnotation { annotation },
                ..
            } if annotation.points.len() > BROWSER_VIEW_MAX_ANNOTATION_POINTS => {
                Err("browser view draft annotation points exceed bounds")
            }
            BrowserViewCommand::Draft {
                command: BrowserViewDraftCommand::SaveCapture { annotation_ids, .. },
                ..
            } if annotation_ids.len() > BROWSER_VIEW_MAX_CAPTURE_ANNOTATIONS => {
                Err("browser view capture annotation count exceeds bounds")
            }
            _ => Ok(()),
        }
    }
}

/// Result payloads are small confirmations, never a promise that input pixels have changed.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserViewCommandOutcome {
    None,
    Snapshot {
        snapshot: BrowserViewSnapshot,
    },
    Control {
        control: BrowserViewControlState,
    },
    Inspection {
        inspection: BrowserViewInspectResult,
    },
    CapturePrepared {
        capture_id: String,
        descriptor: BrowserViewFrameDescriptor,
    },
    Draft {
        draft: BrowserViewDraftState,
    },
    DraftInventory {
        inventory: BrowserViewDraftInventory,
    },
    Capture {
        capture: BrowserViewCaptureOutcome,
    },
    Clipboard {
        text: Option<String>,
    },
}

/// Finite response states distinguish refusal, stale identity and uncertain side effects.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BrowserViewCommandResponse {
    Accepted {
        view_id: String,
        #[ts(type = "number")]
        stream_epoch: u64,
        request_id: String,
        outcome: BrowserViewCommandOutcome,
    },
    Rejected {
        view_id: String,
        #[ts(type = "number")]
        stream_epoch: u64,
        request_id: String,
        code: String,
        message: String,
    },
    Stale {
        view_id: String,
        #[ts(type = "number")]
        stream_epoch: u64,
        request_id: String,
        #[ts(type = "number")]
        current_stream_epoch: u64,
        #[ts(type = "number")]
        current_metadata_sequence: u64,
        code: String,
        message: String,
    },
    Unsupported {
        view_id: String,
        #[ts(type = "number")]
        stream_epoch: u64,
        request_id: String,
        capability: String,
        message: String,
    },
    OutcomeUnknown {
        view_id: String,
        #[ts(type = "number")]
        stream_epoch: u64,
        request_id: String,
        code: String,
        message: String,
    },
}

fn validate_id(value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > BROWSER_VIEW_MAX_ID_BYTES {
        return Err("browser view identity must be non-empty and bounded");
    }
    Ok(())
}

fn validate_text(value: &str) -> Result<(), &'static str> {
    if value.len() > BROWSER_VIEW_MAX_TEXT_BYTES {
        return Err("browser view text exceeds bounds");
    }
    Ok(())
}
fn validate_draft_editor(editor: &BrowserViewDraftEditorState) -> Result<(), &'static str> {
    if let Some(annotation_id) = &editor.note_annotation_id {
        validate_id(annotation_id)?;
    } else if !editor.note_text.is_empty() {
        return Err("browser draft note text requires an active annotation");
    }
    if editor.note_text.encode_utf16().count() > BROWSER_VIEW_MAX_NOTE_TEXT_CODE_UNITS {
        return Err("browser draft note text exceeds bounds");
    }
    Ok(())
}

fn validate_browser_url(value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > BROWSER_VIEW_MAX_URL_BYTES {
        return Err("invalid_browser_url: browser URL must be non-empty and bounded");
    }

    let Some((scheme, remainder)) = value.split_once(':') else {
        return Err("invalid_browser_url: browser URL must be absolute");
    };

    if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
        if !remainder.starts_with("//") {
            return Err("invalid_browser_url: browser URL must be absolute");
        }

        let authority = remainder[2..]
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default();
        if authority.is_empty() {
            return Err("invalid_browser_url: browser URL must include an authority");
        }
        if authority.contains('@') {
            return Err("invalid_browser_url: browser URL must not contain userinfo");
        }
        return Ok(());
    }

    if scheme.eq_ignore_ascii_case("about") {
        if remainder.is_empty() {
            return Err("invalid_browser_url: browser URL must not be empty");
        }
        if remainder.starts_with("//") {
            let authority = remainder[2..]
                .split(['/', '?', '#'])
                .next()
                .unwrap_or_default();
            if authority.contains('@') {
                return Err("invalid_browser_url: browser URL must not contain userinfo");
            }
        }
        return Ok(());
    }

    Err("invalid_browser_url: browser URL has an unsupported or unsafe scheme")
}
