# Agent widgets: interface contracts (example sketches for `02-implementation-plan.md`)

EXAMPLE, not product code. These are the shared interfaces the slices in `02-implementation-plan.md` fix up front so workers do not invent them. Names are binding; field order, derives and doc comments are the implementer's. Phase C additions are marked `[C]` and are added only when Phase C starts (never as unused variants in Phase B).

## 1. Protocol DTOs — `crates/cockpit-protocol/src/widget.rs` (slice B1)

All types derive `Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS` like `v1.rs:184-225`; enums use `#[serde(rename_all = "snake_case")]`; tagged enums use `#[serde(tag = "type", rename_all = "snake_case")]`. Every `u64` timestamp/revision carries `#[ts(type = "number")]` as `PaneSummary.revision` does (`v1.rs:219-220`).

```rust
// Limits (also enforced by the CLI before connecting where noted).
pub const WIDGET_MAX_HTML_BYTES: usize = 1024 * 1024;        // CLI + owner
pub const WIDGET_MAX_CHOICES_BYTES: usize = 64 * 1024;       // CLI + owner
pub const WIDGET_MAX_SELECTION_BYTES: usize = 16 * 1024;     // owner
pub const WIDGET_MAX_LIVE_PER_TAB: usize = 8;
pub const WIDGET_MAX_TOMBSTONES_PER_TAB: usize = 64;
pub const WIDGET_MAX_TOTAL_HTML_BYTES: usize = 64 * 1024 * 1024; // owner-wide
pub const WIDGET_MAX_ARRIVALS_PER_MINUTE: usize = 6;          // per source, creates + reopens only
pub const WIDGET_MAX_SELECTION_WAITERS: usize = 8;            // owner-wide
pub const WIDGET_MAX_WAIT_SECONDS: u64 = 3600;
pub const WIDGET_DEFAULT_WAIT_SECONDS: u64 = 300;

// ---- CLI -> owner (owner socket only; never an HTTP route) ----
pub enum WidgetLocator {            // tag = "type"
    CurrentPane,
    Pane { pane_id: String },
    Tab { tab_id: String },
    Space { space_id: String },
}
pub struct WidgetAddress {
    pub session_id: String,
    pub endpoint_path: Option<String>,   // CLI's effective Herdr socket, checked like BrowserTarget (browser.rs:620-622)
    pub source_pane_id: Option<String>,  // from `herdr pane current --current`; None = unattributed
    pub locator: WidgetLocator,
    pub space_check: Option<String>,     // `--space S` together with --pane/--tab/--current
}
pub enum WidgetInputKind { File, Stdin }
pub enum WidgetContentInput {      // tag = "type"
    Html { content_base64: String, sha256: String, from: WidgetInputKind, name: Option<String> },
    Choices { spec_json: String, sha256: String, from: WidgetInputKind, name: Option<String> },
}
pub struct WidgetShowRequest {
    pub address: WidgetAddress,
    pub id: String,
    pub title: Option<String>,
    pub content: WidgetContentInput,
    pub reopen: bool,
    pub clear_selection: bool,
}
pub enum WidgetShowResult { Opened, Replaced, Unchanged, Reopened }
pub enum WidgetDisplayed { Now, WhenVisible, WhenTabSelected, WhenOpened, NoWindow }
pub enum WidgetPresentation { StaticPreview, Choices /* [C] Active */ }
pub enum WidgetResolvedFrom { CurrentPane, Pane, Tab, SpaceFocusedTab, Stored }
pub struct WidgetContentFacts { pub sha256: String, pub bytes: u64, pub from: WidgetInputKind, pub name: Option<String> }
pub struct WidgetSourceFacts { pub pane_id: String, pub tab_id: String, pub space_id: String }
pub struct WidgetTargetFacts { pub session_id: String, pub space_id: String, pub tab_id: String, pub resolved_from: WidgetResolvedFrom }
pub struct WidgetShowResponse {
    pub id: String,
    pub revision: u64,
    pub result: WidgetShowResult,
    pub displayed: WidgetDisplayed,
    pub location: String,                 // "Space api · tab 1" from Herdr labels/numbers
    pub presentation: WidgetPresentation,
    pub content: WidgetContentFacts,
    pub source: Option<WidgetSourceFacts>,
    pub target: WidgetTargetFacts,
    pub warnings: Vec<String>,            // "external_reference_removed: <url>", "unattributed: …", "not_focused_tab: …"
}
pub struct WidgetCloseRequest { pub address: WidgetAddress, pub id: String }
pub enum WidgetCloseResult { Closed, AlreadyRemoved }
pub struct WidgetCloseResponse { pub id: String, pub result: WidgetCloseResult }
pub struct WidgetListRequest { pub address: WidgetAddress }
pub enum WidgetListState { Live, RemovedByUser }
pub struct WidgetListEntry {
    pub id: String, pub state: WidgetListState, pub revision: u64,
    pub presentation: Option<WidgetPresentation>, pub displayed: Option<WidgetDisplayed>,
    pub removed_at_ms: Option<u64>,
}
pub struct WidgetListResponse { pub widgets: Vec<WidgetListEntry> }
pub struct WidgetSelectionRequest { pub address: WidgetAddress, pub id: String, pub wait_seconds: Option<u64> }
pub enum WidgetSelectionStatus { None, Selected, Timeout, Dismissed, Retired }
pub struct WidgetSelectionResponse {
    pub id: String,
    pub revision: Option<u64>,
    pub status: WidgetSelectionStatus,
    pub value_json: Option<String>,       // canonical JSON text, ≤ 16 KiB; the CLI re-embeds it as `value`
    pub at_ms: Option<u64>,
    pub removed_at_ms: Option<u64>,
}

// ---- owner -> windows (event stream) and window -> owner ----
pub struct WidgetKey { pub session_id: String, pub tab_id: String, pub id: String }
pub enum WidgetKind { Html, Choices }
pub enum WidgetArrival { OwnTab, CrossSource }
pub enum WidgetChange { Opened, Replaced, Reopened, Updated /* metadata only: source status, selection facts */ }
pub enum WidgetSourceStatus { Present, Closed, Restarted, Unknown }
pub struct WidgetSourceSummary {
    pub pane_id: String, pub tab_id: String, pub space_id: String,
    pub terminal_id: String, pub agent_label: Option<String>,
    pub fingerprint_prefix: Option<String>, // first 12 hex chars; never the full value
    pub status: WidgetSourceStatus,
}
pub struct WidgetSelectionFacts { pub revision: u64, pub at_ms: u64, pub read_at_ms: Option<u64> }
pub struct WidgetSummary {
    pub key: WidgetKey,
    pub space_id: String,
    pub title: String,                      // sanitized plain text ≤ 80 chars
    pub revision: u64,
    pub created_seq: u64,                   // order = first publish; reopen gets a new seq (appended)
    pub kind: WidgetKind,
    pub presentation: WidgetPresentation,
    pub content: WidgetContentFacts,
    pub warnings: Vec<String>,
    pub source: Option<WidgetSourceSummary>,
    pub arrival: WidgetArrival,
    pub resolved_from: WidgetResolvedFrom,
    pub change: WidgetChange,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub selection: Option<WidgetSelectionFacts>,
}
pub enum WidgetRemovalReason { User, Agent, Retired }
pub enum WidgetEvent {                      // tag = "type"
    Snapshot { sequence: u64, widgets: Vec<WidgetSummary> },
    Upserted { sequence: u64, widget: WidgetSummary },
    Removed { sequence: u64, key: WidgetKey, reason: WidgetRemovalReason },
}
pub enum WidgetBlocker { Library, Zoom, Drag, TooNarrow }
pub struct WidgetWindowReport {
    pub session_id: Option<String>,
    pub displayed_tab_id: Option<String>,   // selected tab of the selected Space, canvas mounted, document visible
    pub blocker: Option<WidgetBlocker>,
    // [C] pub runtime_proof: Option<WidgetRuntimeProof>,
}
pub struct WidgetContentRequest { pub key: WidgetKey, pub revision: u64 }
pub enum WidgetBody {                       // tag = "type"
    Html { document: String },              // owner-preflighted full document
    Choices { spec: WidgetChoicesSpec },
}
pub struct WidgetContent { pub key: WidgetKey, pub revision: u64, pub sha256: String, pub body: WidgetBody }
pub struct WidgetRemoveRequest { pub key: WidgetKey }
pub enum WidgetRemoveResult { Removed, AlreadyRemoved }
pub struct WidgetRemoveResponse { pub result: WidgetRemoveResult }
pub enum WidgetSelectValue {                // tag = "type"
    Choice { choice_id: String },
    // [C] Page { value_json: String },
}
pub struct WidgetSelectRequest { pub key: WidgetKey, pub revision: u64, pub value: WidgetSelectValue }
pub struct WidgetSelectResponse { pub at_ms: u64 }

// ---- Declarative choices (stored and rendered by Cockpit; no agent HTML) ----
#[serde(deny_unknown_fields)]
pub struct WidgetChoicesSpec {
    pub prompt: Option<String>,             // ≤ 200 chars after sanitizing
    pub choices: Vec<WidgetChoice>,         // 1..=20, unique ids
}
#[serde(deny_unknown_fields)]
pub struct WidgetChoice {
    pub id: String,                         // [a-z0-9][a-z0-9_-]{0,47}
    pub label: String,                      // 1..=80 chars
    pub detail: Option<String>,             // ≤ 200 chars
}
// Stored selection for a choice: canonical JSON {"id":"<choice id>","label":"<label>"}.
```

Example `choices.json` an agent passes with `--choices-file`:

```json
{
  "prompt": "Which view should I keep working on?",
  "choices": [
    { "id": "latency", "label": "Latency p95", "detail": "per route, last 24 h" },
    { "id": "errors", "label": "Errors by route" }
  ]
}
```

## 2. Core service — `crates/cockpit-core/src/widget.rs` (slice B2)

```rust
pub struct WidgetService { /* adapter, paste adapter, parking_lot::Mutex<Store>, broadcast::Sender<WidgetEvent>, clock */ }

impl WidgetService {
    pub fn new(
        adapter: Arc<dyn crate::browser::BrowserHerdrAdapter>,          // fresh snapshots (browser.rs:61-77)
        paste: Arc<dyn crate::paste_adapter::CommentPasteAdapter>,       // agent label + fingerprint (paste_adapter.rs:10-13)
    ) -> Self;
    #[cfg(test)] pub fn with_clock(self, clock: Arc<dyn Fn() -> u64 + Send + Sync>) -> Self;

    // CLI operations (owner socket)
    pub async fn show(&self, request: WidgetShowRequest) -> Result<WidgetShowResponse, InspectionError>;
    pub async fn close(&self, request: WidgetCloseRequest) -> Result<WidgetCloseResponse, InspectionError>;
    pub async fn list(&self, request: WidgetListRequest) -> Result<WidgetListResponse, InspectionError>;
    pub async fn selection(&self, request: WidgetSelectionRequest) -> Result<WidgetSelectionResponse, InspectionError>;

    // Window operations (gateway / Tauri, via BrowserRuntime)
    pub fn subscribe(&self) -> WidgetSubscription;                      // snapshot event + receiver + window guard
    pub fn report_window(&self, window_id: &str, report: WidgetWindowReport) -> Result<(), InspectionError>;
    pub fn content(&self, request: WidgetContentRequest) -> Result<WidgetContent, InspectionError>;
    pub fn remove(&self, request: WidgetRemoveRequest) -> Result<WidgetRemoveResponse, InspectionError>;   // user removal → tombstone
    pub fn select(&self, request: WidgetSelectRequest) -> Result<WidgetSelectResponse, InspectionError>;

    // Owner loop
    pub async fn reconcile(&self);                                      // retirement + source status; never on stale/failed snapshots
    pub fn shutdown(&self);                                             // wakes waiters with Retired
}

pub struct WidgetSubscription {
    pub window_id: String,                       // uuid v4 minted by the service
    pub snapshot: WidgetEvent,                   // always WidgetEvent::Snapshot
    pub events: tokio::sync::broadcast::Receiver<WidgetEvent>,
    pub guard: WidgetWindowGuard,                // Drop unregisters the window (owner `displayed` state)
}
```

Error codes returned as `InspectionError::new(code, message)`; the CLI maps codes to exit statuses (section 4).

## 3. Owner runtime wire — `crates/cockpit-host/src/browser_runtime.rs` (slice B3)

```rust
enum WireRequest {            // existing variants unchanged (browser_runtime.rs:84-98)
    // …
    WidgetShow(WidgetShowRequest),
    WidgetClose(WidgetCloseRequest),
    WidgetList(WidgetListRequest),
    WidgetSelection(WidgetSelectionRequest),
    WidgetContent(WidgetContentRequest),
    WidgetRemove(WidgetRemoveRequest),
    WidgetSelect(WidgetSelectRequest),
    WidgetWindowReport { window_id: String, report: WidgetWindowReport },
    WidgetEvents,             // streaming, handled in serve_peer like BrowserViewEvents (browser_runtime.rs:847-883)
}
enum WireResponse {           // existing variants unchanged (browser_runtime.rs:102-116)
    // …
    WidgetShown(WidgetShowResponse),
    WidgetClosed(WidgetCloseResponse),
    WidgetListed(WidgetListResponse),
    WidgetSelection(WidgetSelectionResponse),
    WidgetContent(WidgetContent),
    WidgetRemoved(WidgetRemoveResponse),
    WidgetSelected(WidgetSelectResponse),
    WidgetReported,
    WidgetSubscribed { window_id: String },   // first frame of the WidgetEvents stream, before the Snapshot event
    WidgetEvent(WidgetEvent),
}

impl BrowserRuntime {
    pub async fn start(state_root: PathBuf, service: Arc<BrowserService>, widgets: Arc<WidgetService>) -> Result<Self, InspectionError>;
    pub async fn widget_show(&self, r: WidgetShowRequest) -> Result<WidgetShowResponse, InspectionError>;
    pub async fn widget_close(&self, r: WidgetCloseRequest) -> Result<WidgetCloseResponse, InspectionError>;
    pub async fn widget_list(&self, r: WidgetListRequest) -> Result<WidgetListResponse, InspectionError>;
    pub async fn widget_selection(&self, r: WidgetSelectionRequest) -> Result<WidgetSelectionResponse, InspectionError>;
    pub async fn widget_content(&self, r: WidgetContentRequest) -> Result<WidgetContent, InspectionError>;
    pub async fn widget_remove(&self, r: WidgetRemoveRequest) -> Result<WidgetRemoveResponse, InspectionError>;
    pub async fn widget_select(&self, r: WidgetSelectRequest) -> Result<WidgetSelectResponse, InspectionError>;
    pub async fn widget_report(&self, window_id: &str, r: WidgetWindowReport) -> Result<(), InspectionError>;
    pub async fn widget_events(&self) -> Result<WidgetEventStream, InspectionError>; // owner: in-process; observer: forwarded socket stream
}
pub struct WidgetEventStream { pub window_id: String, pub snapshot: WidgetEvent, pub events: tokio::sync::broadcast::Receiver<WidgetEvent>, /* keeps guard or socket alive */ }
```

Read timeouts (`response_read_timeout`, `browser_runtime.rs:56-63`): `WidgetShow`/`WidgetClose`/`WidgetList` 45 s; `WidgetSelection` = `wait_seconds` (or 0) + 45 s; others `IO_TIMEOUT`. An observer whose `WidgetSelection` forward ends in `browser_outcome_unknown` (owner closed the socket) maps to `widget_retired`.

## 4. CLI error code → exit status (slice B4)

| Code | Exit | Raised by |
| --- | --- | --- |
| `widget_usage` (missing `--id`, bad slug, not exactly one content flag, `--stdin` on a TTY, non-regular `--file`, invalid UTF-8, malformed `--choices-file` JSON) | 2 | CLI |
| `widget_target_required`, `--current requires HERDR_ENV=1` wording | 2 | CLI |
| `widget_target_not_found`, `widget_target_mismatch`, `widget_target_no_focused_tab`, `widget_target_changed`, `widget_not_owner` | 2 | owner |
| `selection` status `timeout` (JSON on stdout) | 12 | CLI from owner status |
| `selection` status `retired`, or `widget_retired` | 14 | owner / CLI mapping |
| `widget_dismissed` | 15 | owner |
| `widget_no_owner` (from `browser_owner_unavailable`/`browser_owner_timeout`), `widget_herdr_unavailable`, `widget_busy`, `widget_outcome_unknown` | 20 | CLI mapping / owner |
| `widget_too_large`, `widget_too_complex` | 21 | CLI before connecting / owner |
| `widget_limit`, `widget_rate_limited` | 22 | owner |
| `widget_selection_unavailable` | 23 | owner |
| any other owner code | 20 | CLI mapping, printed verbatim |

Success output: one compact JSON line on stdout (`serde_json::to_string`), timestamps as RFC 3339 UTC seconds (`2026-10-02T14:03:07Z`), `value_json` re-embedded as `value`. Warnings: also one `cockpit: warning: <text>` line each on stderr. Errors: `error: <code>: <message>` on stderr.

## 5. Client contract — `src/client/CockpitClient.ts` (slice B5)

```ts
export interface WidgetStream extends ClosableStream {
  /** Latest window state; adapters coalesce and send at most every 100 ms. */
  report(report: WidgetWindowReport): void;
}
export type WidgetEventHandler = (event: WidgetEvent) => void;

export interface CockpitClient {
  // … existing members (CockpitClient.ts:294-375)
  subscribeWidgets(onEvent: WidgetEventHandler, onError: (error: CockpitClientError) => void, signal?: AbortSignal): Promise<WidgetStream>;
  widgetContent(request: WidgetContentRequest, signal?: AbortSignal): Promise<WidgetContent>;
  widgetRemove(request: WidgetRemoveRequest): Promise<WidgetRemoveResponse>;
  widgetSelect(request: WidgetSelectRequest): Promise<WidgetSelectResponse>;
}
```

Browser adapter: `GET /api/v1/widgets/events` WebSocket (server→client `WidgetEvent` text frames; client→server `WidgetWindowReport` text frames ≤ 4 KiB), `POST /api/v1/widgets/content`, `POST /api/v1/widgets/remove`, `POST /api/v1/widgets/select` (the last two behind `require_origin`, `server.rs:61-67`).
Native adapter: `cockpit_widget_subscribe(channel) -> streamId`, `cockpit_widget_report(streamId, report)`, `cockpit_widget_content`, `cockpit_widget_remove`, `cockpit_widget_select`; cancel with the existing `cockpit_stream_cancel`.

## 6. Frontend layout and arrival (slices B6, B7)

```ts
// splitTree.ts
export type ViewerKind = "files" | "review" | "browser" | "widget";
export function splitLeaf(root, targetId, dir, before, leaf, share = 0.5): LayoutNode; // `share` = newcomer weight

// tabLayoutStore.ts
export type WidgetDockSlot = { currentId: string | null; previousSelectedLeafId: LeafId | null };
// TabLayoutState gains: viewers.widget?: WidgetDockSlot; widgetShare: number  (default 0.4, remembered for the run)
type LayoutAction =
  | { type: "widget/dock"; tabId: string; besideLeafId: LeafId; currentId: string }   // inserts `${tabId}:widget`, never selects
  | { type: "widget/current"; tabId: string; currentId: string }
  | { type: "widget/undock"; tabId: string }                                           // records widgetShare; focus rule of spec 4.6

// src/app/widgets/arrival.ts (pure)
export type ArrivalInput = {
  change: "opened" | "reopened" | "replaced" | "updated";
  arrival: "own_tab" | "cross_source";
  tabDisplayed: boolean;
  blocker: "library" | "zoom" | "drag" | "too_narrow" | null;
  dockPresent: boolean;
  isCurrent: boolean;
  focusInsideFrame: boolean;
};
export type ArrivalDecision =
  | { kind: "dock_now"; makeCurrent: boolean; announce: boolean }       // F1 (+ W-22 rule via makeCurrent)
  | { kind: "defer_until_clear"; indicator: "none" | "widgets_button" } // F2: drag/library → none; zoom/too_narrow → button
  | { kind: "wait_tab"; announce: boolean }                            // F3/F4: tab dot
  | { kind: "wait_click"; announce: boolean }                          // F5/F6: Widgets button (+ tab dot when not displayed)
  | { kind: "replace_in_place"; markUnseen: boolean }                  // F8
  | { kind: "none" };
export function decideArrival(input: ArrivalInput): ArrivalDecision;
```
