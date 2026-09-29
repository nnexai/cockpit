// ILLUSTRATIVE PLANNING EXAMPLE — not product code, not compiled, not imported.
// Shapes fixed up front by 02-implementation-plan.md so parallel slices agree.
// Interaction rules: 01-design.md. Final names live in the product files listed in the plan.

// ---------------------------------------------------------------------------
// src/app/layout/splitTree.ts  (pure, immutable port of demo.html model ops)
// ---------------------------------------------------------------------------
export type ViewerKind = "files" | "review" | "browser";
export type LeafKind = "terminal" | ViewerKind;
/** terminal: Herdr pane id; viewer: `${tabId}:${kind}` (one per kind per tab). */
export type LeafId = string;
export type Leaf = { t: "leaf"; id: LeafId; kind: LeafKind; w: number };
/** `id` is stable for the split's lifetime (divider React keys); the demo has none. */
export type Split = { t: "split"; id: string; dir: "row" | "col"; kids: LayoutNode[]; w: number };
export type LayoutNode = Leaf | Split;
export type Side = "left" | "right" | "top" | "bottom";

// Every op returns a new normalised tree (demo `norm`, demo.html:227-241).
export declare function firstLoadGrid(sortedLeaves: Leaf[]): LayoutNode | null;   // 01-design.md §3.1 (even rows; equals demo buildGrid except N=7,10,13,14,17)
export declare function splitLeaf(root: LayoutNode, targetId: LeafId, dir: "row" | "col", before: boolean, leaf: Leaf): LayoutNode; // 50:50, demo:256-263
export declare function removeLeaf(root: LayoutNode, id: LeafId): { root: LayoutNode | null; absorbedBy: LeafId | null };        // demo:265-273
export declare function insertRootEdge(root: LayoutNode | null, leaf: Leaf, dir: "row" | "col", before: boolean): LayoutNode;    // share 1/(N+1), N incl. zoom-hidden, demo:274-288
export declare function swapLeaves(root: LayoutNode, a: LeafId, b: LeafId): LayoutNode;                                          // slots keep size, demo:289-294
export declare function applyDrop(root: LayoutNode, src: LeafId, drop: DropTarget): LayoutNode;                                  // demo:643-660
export declare function setPairWeights(root: LayoutNode, splitId: string, index: number, wa: number, wb: number): LayoutNode;    // pair total preserved, demo:547-555
export declare function leaves(root: LayoutNode | null): Leaf[];                 // in-order: depth-first, left→right / top→bottom
export declare function compareStablePaneId(a: string, b: string): number;       // digit runs numeric, else UTF-16; tie by whole-string code units

// ---------------------------------------------------------------------------
// src/app/layout/solveLayout.ts  (CSS-flex-equivalent solver, 01-design.md §3.4)
// ---------------------------------------------------------------------------
export const MIN_W = 160, MIN_H = 100, DIVIDER = 8, RIM = 14, EDGE = 0.25, DRAG_PX = 4; // demo.html:174
export type Rect = { x: number; y: number; width: number; height: number };
export type DividerHandle = { splitId: string; index: number; dir: "row" | "col"; rect: Rect };
export declare function solveLayout(root: LayoutNode, area: Rect): { leaves: Map<LeafId, Rect>; dividers: DividerHandle[]; degraded: boolean };
export type DropTarget =
  | { kind: "swap"; target: LeafId; rect: Rect; label: "Swap" | "Swap (pane too small to split)" }
  | { kind: "edge"; target: LeafId; side: Side; rect: Rect }
  | { kind: "root"; side: Side; rect: Rect; sharePct: number };
export declare function computeDrop(point: { x: number; y: number }, srcId: LeafId, solved: Map<LeafId, Rect>, area: Rect, leafCount: number): DropTarget | null; // demo:589-623

// ---------------------------------------------------------------------------
// src/app/layout/tabLayoutStore.ts  (held in App, above Workbench key={epoch})
// ---------------------------------------------------------------------------
export type FocusTriple = { spaceId: string | null; tabId: string | null; paneId: string | null };
export type ViewerSelector = { kind: "files_context" | "files_folder" } | { kind: "review"; repositoryId: string };
export type ViewerSlot<Context, View> = {
  status: "opening" | "open" | "error";
  selector: ViewerSelector;         // current source; reused only by an explicit user Reopen after viewer_not_found
  sourcePaneId: string;             // runtime source terminal at open time
  context: Context | null;          // ViewerContext DTO (null while opening)
  viewsBySource: Record<string, View>; // per source_id view state incl. unsaved commentEditor text (kept for the run)
  error: string | null;
};
export type BrowserSlot = {
  status: "opening" | "open" | "closing" | "close_failed" | "outcome_unknown" | "error";
  association: unknown | null;      // BrowserAssociation DTO
  error: string | null;
};
export type BrowserCleanupNotice = { associationKey: string; tabId: string | null; reason: string }; // in-memory strip, survives tab switches
export type PendingCreation = {
  token: number;                    // mutation coordinator token
  tabId: string;
  placeBeside: LeafId;              // target leaf at request time (viewer or terminal)
  dir: "row" | "col";
  sourcePaneId: string;             // Herdr runtime source (pane_split.pane_id)
};
export type TabLayoutState = {
  tabId: string;
  spaceId: string;                  // last confirmed owner Space
  root: LayoutNode | null;
  terminals: Record<string, string>; // paneId -> terminalId for every terminal leaf
  selectedLeafId: LeafId | null;
  lastRealLeafId: string | null;     // last-selected real terminal
  zoomLeafId: LeafId | null;
  viewers: { files?: ViewerSlot<unknown, unknown>; review?: ViewerSlot<unknown, unknown>; browser?: BrowserSlot };
  heldMembers: string[];             // new members held while a Cockpit creation in this tab is in flight
  bufferedFocus: FocusTriple | null; // changed focus naming a held member; classified on settle (echo iff = created.pane_id)
  revision: number;                  // bumped by every structural change; drag commit checks it
};
export type SessionLayoutState = {
  sessionId: string;
  serverInstance: string;           // SessionSnapshotResponse.server_instance
  tabs: Record<string, TabLayoutState>;
  activeSpaceId: string | null;
  activeTabId: string | null;
  observedFocus: FocusTriple | null; // last applied ordered live snapshot
  pendingCreation: PendingCreation | null;
  cleanupNotices: BrowserCleanupNotice[];
};

// ---------------------------------------------------------------------------
// src/app/layout/reconcile.ts  (pure: snapshot -> next state + side effects)
// ---------------------------------------------------------------------------
/** Exact-identity echoes only: pane/agent target = focused_pane_id, tab target = focused_tab_id, space target = focused_space_id.
 *  A validated created.pane_id is matched separately on creation settle, never via a tab-wide "pending creation" echo. */
export type FocusEcho = { token: number; kind: "space" | "tab" | "pane" | "agent"; targetId: string };
export type FocusClass = "initial" | "unchanged" | "echo" | "external";
export declare function classifyFocus(prev: FocusTriple | null, next: FocusTriple, echoes: readonly FocusEcho[], state: SessionLayoutState): FocusClass;
export type LayoutEffect =
  | { type: "browser-retire"; tabId: string }                     // guard -> Close (stop + remove proven artifacts), tab-scoped target
  | { type: "viewer-release"; tabId: string; kind: "files" | "review"; viewerId: string }
  | { type: "cancel-transient"; reason: "external-focus" | "leaf-removed" | "tab-hidden" }
  | { type: "consume-echo"; token: number }
  | { type: "announce"; text: string };                           // aria-live polite status (01-design.md §6)
export declare function reconcileSnapshot(
  state: SessionLayoutState,
  snapshot: { server_instance: string; focused_space_id: string | null; focused_tab_id: string | null; focused_pane_id: string | null;
              tabs: { id: string; space_id: string; focused_pane_id: string | null }[];
              panes: { id: string; terminal_id: string; tab_id: string; space_id: string }[] },
  echoes: readonly FocusEcho[],
): { state: SessionLayoutState; effects: LayoutEffect[] };
// Order: (1) server_instance change => drop tabs; (2) membership per tab (prune, attributed, held, external);
// (3) retire absent / zero-member tabs; (4) focus classification; (5) selection/zoom/lastReal repair.
