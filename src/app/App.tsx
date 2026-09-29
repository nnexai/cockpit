import { UiIcon } from "./UiIcon";
import { ReviewViewer } from "./review/ReviewViewer";
import { useCallback, useEffect, useLayoutEffect, useReducer, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent, type PointerEvent as ReactPointerEvent, type ReactNode, type RefObject } from "react";
import {
  parseResourceMutationResponse,
  type CockpitClient,
  type CockpitStatus,
  type TerminalStream,
} from "../client/CockpitClient";
import type {
  BrowserTarget,
  BrowserViewPresentation,
  BrowserViewViewportRequest,
  FocusRequest,
  ResourceMutationRequest,
  ResourceMutationResponse,
  SessionSnapshotResponse,
  SessionStreamMessage,
  SessionSummary,
  SpaceGitStatus,
  TabLayout,
  TerminalOpenRequest,
} from "../protocol/generated/v1";
import { initialSessionState, sessionReducer, type SessionState } from "./session/sessionStore";
import { useFocusCoordinator } from "./session/focusCoordinator";
import { spaceCheckoutKey, useSpaceGitStatus } from "./session/spaceGitStatus";
import { type MutationCoordinatorState, type MutationOperation, useMutationCoordinator } from "./session/mutationCoordinator";
import { deriveResizeHandles, projectedPaneIds, projectedPaneRect, resizeRequest, tabDropInsertionIndex, type ResizeHandle } from "./layout/layoutProjection";
import { routeWorkbenchKeydown } from "./input/keymap";
import { SHORTCUTS, SHORTCUT_SEPARATOR, armedPrefixHint, focusSidebarList, formatShortcut, shortcutEntry, withShortcut, type PrefixCommand } from "./input/shortcuts";
import { trapModalTab, useModalFocus } from "./input/modal";
import { flushSync } from "react-dom";
import { dispatchFileNavigation, rankFuzzyMatches } from "./input/fileNavigation";
import { TerminalPane } from "./TerminalPane";
import { SetupDialog, setupParentFor } from "./projects/SetupDialog";
import { TeardownDialog } from "./projects/TeardownDialog";
import { TeardownRecoveryPanel } from "./projects/TeardownRecoveryPanel";
import { ContextViewer, type ContextViewState, type LibraryCommand } from "./context/ContextViewer";
import { AddContextDialog } from "./library/AddContextDialog";
import { LibraryView } from "./library/LibraryView";
import type { LibrarySpace } from "./library/libraryState";
import { isGraphicalContext, isGraphicalReview, usePaneRenderers, type PaneRendererState } from "./paneRenderers";
import { BrowserPane, type BrowserPaneRecoveryRegistration } from "./browser/BrowserPane";
import { InlineRename } from "./InlineRename";
import { Sidebar } from "./sidebar/Sidebar";
import { spaceNotesFromFailures, type ContextAnchor } from "./sidebar/Spaces";
import type { Agent, Space } from "./sidebar/spaceTree";

type StatusError = { message: string; code?: string };
type SessionSnapshot = SessionSnapshotResponse;
type Tab = SessionSnapshot["tabs"][number];
type Pane = SessionSnapshot["panes"][number];
type Selection = { spaceId: string | null; tabId: string | null; paneId: string | null };

function describeError(error: unknown, fallback: string): StatusError {
  if (error instanceof Error) {
    const typed = error as Error & { code?: unknown; operationCode?: unknown };
    return { message: typed.message || fallback, code: typeof typed.operationCode === "string" ? typed.operationCode : typeof typed.code === "string" ? typed.code : undefined };
  }
  return { message: fallback };
}
function byId<T extends { id: string }>(items: T[], id: string | null): T | undefined { return id ? items.find((item) => item.id === id) : undefined; }
function tabsForSpace(tabs: Tab[], spaceId: string | null): Tab[] { return spaceId ? tabs.filter((tab) => tab.space_id === spaceId) : []; }
function panesForTab(panes: Pane[], tabId: string | null): Pane[] { return tabId ? panes.filter((pane) => pane.tab_id === tabId) : []; }
export function authoritativeSelection(snapshot: SessionSnapshot): Selection {
  const spaceId = snapshot.spaces.some((space) => space.id === snapshot.focused_space_id) ? snapshot.focused_space_id : snapshot.spaces[0]?.id ?? null;
  const tabs = tabsForSpace(snapshot.tabs, spaceId);
  const tabId = tabs.some((tab) => tab.id === snapshot.focused_tab_id) ? snapshot.focused_tab_id : tabs[0]?.id ?? null;
  const panes = panesForTab(snapshot.panes, tabId);
  const paneId = panes.some((pane) => pane.id === snapshot.focused_pane_id) ? snapshot.focused_pane_id : panes[0]?.id ?? null;
  return { spaceId, tabId, paneId };
}
export function authoritativeMutationSnapshot(expectedSessionId: string, response: ResourceMutationResponse): SessionSnapshot {
  const parsed = parseResourceMutationResponse(response);
  if (parsed.session_id !== expectedSessionId) {
    throw new Error("Mutation response belongs to another session");
  }
  return parsed.snapshot;
}
function CompatibilityNotice({ status, error, retry, onOpenLibrary }: { status: CockpitStatus | null; error: StatusError | null; retry: () => void; onOpenLibrary: () => void }) {
  const herdr = status?.herdr;
  const title = error ? "Cockpit unavailable" : herdr?.status === "incompatible" ? "Herdr is incompatible" : "Herdr is unavailable";
  const message = error?.message ?? (herdr && herdr.status !== "compatible" ? herdr.message : undefined);
  const code = error?.code ?? (herdr && herdr.status !== "compatible" ? herdr.code : undefined);
  return <main className="compatibility-main" aria-live="polite"><section className="notice notice-error" role="alert"><p className="eyebrow">Cockpit</p><h1>{title}</h1><p>{message ?? "Could not read the Herdr compatibility status."}</p>{code ? <code>{code}</code> : null}<div className="notice-actions"><button type="button" className="action-button" onClick={retry}>Retry status</button><OpenLibraryButton onOpen={onOpenLibrary} /></div></section></main>;
}

/** The Library needs no Herdr session, so every no-session screen can open it. */
function OpenLibraryButton({ onOpen }: { onOpen: () => void }) {
  return <button type="button" className="action-button" data-library-opener onClick={onOpen}>Open Library</button>;
}

type Mutate = (key: string, request: ResourceMutationRequest, focusFromSnapshot?: boolean) => boolean;
type ContextTarget =
  | { kind: "space"; id: string }
  | { kind: "tab"; id: string }
  | { kind: "pane"; id: string };
type ContextMenuState = { target: ContextTarget; x: number; y: number };
type DragIntent = { kind: "space" | "tab"; sourceId: string; order: string[] };
type DropMark = { targetId: string; side: "before" | "after" } | null;
type PaneDialog =
  | { kind: "rename"; paneId: string }
  | { kind: "swap"; paneId: string }
  | { kind: "move"; paneId: string };
type PaneCanvasProjection = {
  key: string | null;
  panes: Pane[];
  layout: TabLayout | undefined;
  visiblePaneIds: string[];
  selectedPaneId: string | null;
};

export function canSwitchSessions(sessionCount: number): boolean {
  return sessionCount > 1;
}

export function tabLabelIsRedundant(label: string, displayedNumber: number): boolean {
  const trimmed = label.trim();
  return trimmed === String(displayedNumber) || /^\d+$/.test(trimmed);
}

export type PaneFocusDirection = "left" | "right" | "up" | "down";

const LAYOUT_EPSILON = 0.000001;

function overlapLength(firstStart: number, firstLength: number, secondStart: number, secondLength: number): number {
  return Math.min(firstStart + firstLength, secondStart + secondLength) - Math.max(firstStart, secondStart);
}

export function paneIdInDirection(layout: TabLayout | undefined, paneId: string | null, direction: PaneFocusDirection): string | null {
  if (!layout || layout.zoomed || !paneId) return null;
  const current = layout.panes.find((candidate) => candidate.pane_id === paneId);
  if (!current) return null;
  const candidates = layout.panes.flatMap((candidate, index) => {
    if (candidate.pane_id === paneId) return [];
    const horizontal = overlapLength(current.rect.x, current.rect.width, candidate.rect.x, candidate.rect.width);
    const vertical = overlapLength(current.rect.y, current.rect.height, candidate.rect.y, candidate.rect.height);
    const overlaps = direction === "left" || direction === "right" ? vertical : horizontal;
    const touches = direction === "left"
      ? Math.abs(candidate.rect.x + candidate.rect.width - current.rect.x) <= LAYOUT_EPSILON
      : direction === "right"
        ? Math.abs(current.rect.x + current.rect.width - candidate.rect.x) <= LAYOUT_EPSILON
        : direction === "up"
          ? Math.abs(candidate.rect.y + candidate.rect.height - current.rect.y) <= LAYOUT_EPSILON
          : Math.abs(current.rect.y + current.rect.height - candidate.rect.y) <= LAYOUT_EPSILON;
    return touches && overlaps > LAYOUT_EPSILON ? [{ paneId: candidate.pane_id, overlap: overlaps, index }] : [];
  });
  candidates.sort((left, right) => right.overlap - left.overlap || left.index - right.index);
  return candidates[0]?.paneId ?? null;
}

export function contextMenuPosition(
  x: number,
  y: number,
  viewportWidth: number,
  viewportHeight: number,
  menuWidth = 286,
  menuHeight = 320,
  gutter = 8,
): { x: number; y: number } {
  return {
    x: Math.max(gutter, Math.min(x, viewportWidth - menuWidth - gutter)),
    y: Math.max(gutter, Math.min(y, viewportHeight - menuHeight - gutter)),
  };
}

function ContextMenu({ menu, children, onDismiss }: { menu: ContextMenuState; children: ReactNode; onDismiss: () => void }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const dismissAndRestore = useCallback(() => {
    onDismiss();
    window.setTimeout(() => opener.current?.focus({ preventScroll: true }), 0);
  }, [onDismiss]);
  useEffect(() => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dismiss = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) dismissAndRestore(); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") dismissAndRestore(); };
    window.addEventListener("pointerdown", dismiss);
    window.addEventListener("keydown", escape);
    ref.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    return () => { window.removeEventListener("pointerdown", dismiss); window.removeEventListener("keydown", escape); };
  }, [dismissAndRestore, onDismiss, menu]);
  const [menuSize, setMenuSize] = useState({ width: 286, height: 320 });
  useLayoutEffect(() => {
    const root = ref.current;
    if (!root) return;
    const measure = () => {
      const { width, height } = root.getBoundingClientRect();
      setMenuSize((current) => current.width === width && current.height === height ? current : { width, height });
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(root);
    return () => observer?.disconnect();
  }, [menu]);
  const position = contextMenuPosition(menu.x, menu.y, window.innerWidth, window.innerHeight, menuSize.width, menuSize.height);
  return <div ref={ref} className="context-menu" role="menu" aria-label={`${menu.target.kind} actions`} style={{ left: position.x, top: position.y }} onContextMenu={(event) => event.preventDefault()} onKeyDown={(event) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const items = [...(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
    if (items.length === 0) return;
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : event.key === "ArrowDown" ? (current + 1) % items.length : (current - 1 + items.length) % items.length;
    items[next].focus();
  }}>{children}</div>;
}

function TabStrip({ tabs, selectedTabId, editingId, busy, browserOpen, libraryOpen, onEdit, onSelect, onContext, onCreate, onBrowserToggle, onLibraryToggle, onCommands, sidebarOpen, onToggleSidebar, mutate }: {
  tabs: Tab[];
  selectedTabId: string | null;
  editingId: string | null;
  busy: boolean;
  browserOpen: boolean;
  libraryOpen: boolean;
  onEdit: (id: string | null) => void;
  onSelect: (tab: Tab) => void;
  onContext: (event: MouseEvent, target: ContextTarget) => void;
  onCreate: () => void;
  onBrowserToggle: () => void;
  onLibraryToggle: () => void;
  onCommands: () => void;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  mutate: Mutate;
}) {
  const [dragIntent, setDragIntent] = useState<DragIntent | null>(null);
  const [dropMark, setDropMark] = useState<DropMark>(null);
  const [dragMessage, setDragMessage] = useState<string | null>(null);
  return <nav className="tab-toolbar" aria-label="Tabs"><button type="button" className="tab-icon-button" aria-label={sidebarOpen ? "Hide sidebar" : "Show sidebar"} title={withShortcut(sidebarOpen ? "Hide sidebar" : "Show sidebar", "toggle-sidebar")} aria-expanded={sidebarOpen} aria-controls="cockpit-sidebar" onClick={onToggleSidebar}><UiIcon name={sidebarOpen ? "sidebar-open" : "sidebar"} /></button><div className="tab-strip" role="tablist">{tabs.map((tab, index) => {
    const displayedNumber = index + 1;
    const redundantLabel = tabLabelIsRedundant(tab.label, displayedNumber);
    const accessibleLabel = redundantLabel ? `Tab ${displayedNumber}` : `Tab ${displayedNumber}: ${tab.label}`;
    const side = dropMark?.targetId === tab.id ? dropMark.side : null;
    return <div className={`tab-item${tab.id === selectedTabId ? " is-selected" : ""}${side ? ` drop-${side}` : ""}`} key={tab.id} draggable={!busy && editingId !== tab.id}
      onDragStart={(event) => { if (!busy) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("application/x-cockpit-tab", tab.id); event.dataTransfer.setData("text/plain", `tab:${tab.id}`); setDragIntent({ kind: "tab", sourceId: tab.id, order: tabs.map((candidate) => candidate.id) }); setDragMessage(null); } }}
      onDragEnd={() => { setDragIntent(null); setDropMark(null); }}
      onDragEnter={(event) => { if (!busy) event.preventDefault(); }}
      onDragOver={(event) => { if (!busy) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; if (dragIntent && dragIntent.sourceId !== tab.id) setDropMark({ targetId: tab.id, side: event.clientX >= event.currentTarget.getBoundingClientRect().left + event.currentTarget.getBoundingClientRect().width / 2 ? "after" : "before" }); } }}
      onDrop={(event) => {
        if (busy) return;
        event.preventDefault();
        const fallback = event.dataTransfer.getData("text/plain");
        const id = event.dataTransfer.getData("application/x-cockpit-tab") || (fallback.startsWith("tab:") ? fallback.slice(4) : "");
        if (!id || id === tab.id) return;
        const afterTarget = event.clientX >= event.currentTarget.getBoundingClientRect().left + event.currentTarget.getBoundingClientRect().width / 2;
        const intent = dragIntent?.sourceId === id ? dragIntent : { kind: "tab" as const, sourceId: id, order: tabs.map((candidate) => candidate.id) };
        const unchanged = intent.order.length === tabs.length && intent.order.every((candidate, position) => candidate === tabs[position]?.id);
        const insertion = unchanged ? tabDropInsertionIndex(tabs.findIndex((candidate) => candidate.id === id), index, afterTarget) : null;
        setDropMark(null);
        if (!unchanged) setDragMessage("Tab order changed while dragging. Start again.");
        else if (insertion !== null) mutate(`tab:${id}`, { type: "tab_move", tab_id: id, insert_index: insertion });
      }}
      onContextMenu={(event) => onContext(event, { kind: "tab", id: tab.id })}>
      {editingId === tab.id
        ? <InlineRename label={tab.label} ariaLabel={`Rename tab ${tab.label}`} onCancel={() => onEdit(null)} onCommit={(label) => { const accepted = mutate(`tab:${tab.id}`, { type: "tab_rename", tab_id: tab.id, label }); if (accepted) onEdit(null); return accepted; }} />
        : <button type="button" disabled={busy} draggable={!busy} role="tab" aria-selected={tab.id === selectedTabId} aria-label={accessibleLabel} className="tab-button" title={displayedNumber <= 9 ? withShortcut(redundantLabel ? `Tab ${displayedNumber}` : tab.label, `select-tab-${displayedNumber as 1}`) : redundantLabel ? `Tab ${displayedNumber}` : tab.label} onDragStart={(event) => { if (!busy) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("application/x-cockpit-tab", tab.id); event.dataTransfer.setData("text/plain", `tab:${tab.id}`); setDragIntent({ kind: "tab", sourceId: tab.id, order: tabs.map((candidate) => candidate.id) }); setDragMessage(null); } }} onClick={() => onSelect(tab)} onDoubleClick={() => onEdit(tab.id)}><span className="tab-number">{displayedNumber}</span>{redundantLabel ? null : <span className="tab-label">{tab.label}</span>}</button>}
    </div>;
  })}
    <button type="button" disabled={busy} className="tab-add" aria-label="Create tab" title={withShortcut("New tab", "new-tab")} onClick={onCreate}><UiIcon name="plus" /></button></div>{dragMessage ? <span className="resource-inline-status tab-drag-status" role="status">{dragMessage}</span> : null}<div className="tab-strip-actions"><span className="tab-strip-separator" aria-hidden="true" /><button type="button" className="tab-icon-button" disabled={busy} aria-label="Browser" aria-pressed={browserOpen} title={withShortcut(browserOpen ? "Close browser" : "Open browser", "toggle-browser")} onClick={onBrowserToggle}><UiIcon name="browser" /></button><button type="button" className="tab-icon-button" aria-label="Library" aria-pressed={libraryOpen} title={withShortcut(libraryOpen ? "Close Library" : "Open Library", "toggle-library")} onClick={onLibraryToggle}><UiIcon name="library" /></button><button type="button" className="tab-strip-action" title={withShortcut("Commands", "help")} onClick={onCommands}>Commands</button></div>
  </nav>;
}

/** Header of a graphical pane: what it shows and, for files, which folder. */
function viewerTitle(renderer: PaneRendererState): { title: string; subtitle: string | null; path?: string } {
  if (isGraphicalReview(renderer)) return { title: "Review", subtitle: null };
  const { roots, default_root_id } = renderer.presentation;
  const root = roots.find(candidate => candidate.root_id === (renderer.view.rootId ?? default_root_id)) ?? roots[0];
  if (root?.kind === "companion") return { title: "Context", subtitle: null, path: root.path };
  // Task worktrees are named `<repository>-<operation id>`; the id is noise here.
  const label = root?.label.replace(/-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/, "") ?? null;
  return { title: "Files", subtitle: label, path: root?.path };
}

function PaneView({ pane, label, solo = false, selected, paintedSelected, retained, busy, controlAllowed, controlPending, focusError, focusEpoch, focusToken, terminalMouseInput, onRequestControl, onSelect, onContext, onRetryFocus, request, client, registerStream, onResync, mutate, style, renderer, rendererReady, deferTerminal, focusOnAttach, onRendererViewChange, onTerminalView, onRefreshRenderer, onReady, onPrepared, space }: {
  pane: Pane;
  label: string;
  /** The only Herdr pane in its tab; like Herdr, a lone terminal needs no header row. */
  solo?: boolean;
  selected: boolean;
  paintedSelected: boolean;
  retained: boolean;
  busy: boolean;
  controlAllowed: boolean;
  controlPending: boolean;
  focusError: { code: string; message: string } | null;
  focusEpoch: number;
  focusToken: number;
  terminalMouseInput: boolean;
  onRequestControl: () => void;
  onSelect: () => void;
  onContext: (event: MouseEvent, target: ContextTarget) => void;
  onRetryFocus: () => void;
  request: Omit<TerminalOpenRequest, "mode" | "takeover" | "cols" | "rows" | "cell_width_px" | "cell_height_px">;
  client: CockpitClient;
  registerStream: (stream: TerminalStream, active: boolean) => void;
  onResync: () => void;
  mutate: Mutate;
  style: { left: string; top: string; width: string; height: string; visibility?: CSSProperties["visibility"]; pointerEvents?: CSSProperties["pointerEvents"] };
  renderer: PaneRendererState | undefined;
  rendererReady: boolean;
  deferTerminal: boolean;
  /** The terminal painted its first frame, even if Herdr has not confirmed focus yet. */
  onPrepared: () => void;
  /** False after the Library view closes, until an explicit local pane action (D13). */
  focusOnAttach: boolean;
  onRendererViewChange: (bindingId: string, value: ContextViewState) => void;
  onTerminalView: () => void;
  onRefreshRenderer: () => void;
  onReady: () => void;
  /** The pane's own Space, which its Context viewer's `Resources` and `Add to <Space>` act on. */
  space: LibrarySpace | null;
}) {
  const title = pane.title || label;
  const closePane = () => { if (window.confirm(`Close ${title}?`)) mutate(`pane:${pane.id}`, { type: "pane_close", pane_id: pane.id }); };
  const graphical = isGraphicalContext(renderer) || isGraphicalReview(renderer);
  const graphicalRef = useRef<HTMLDivElement>(null);
  const paneRef = useRef<HTMLElement>(null);
  const [terminalReadiness, setTerminalReadiness] = useState({ deferred: deferTerminal, ready: false });
  useLayoutEffect(() => {
    setTerminalReadiness({ deferred: deferTerminal, ready: false });
  }, [deferTerminal]);
  const intendedControl = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (!graphical) { intendedControl.current = null; return; }
    if (controlAllowed) {
      intendedControl.current?.focus({ preventScroll: true });
      intendedControl.current = null;
    } else if (!controlPending) {
      intendedControl.current = null;
      const active = document.activeElement;
      if (active instanceof HTMLElement && graphicalRef.current?.contains(active)) active.blur();
    }
  }, [graphical, controlAllowed, controlPending]);
  const reportTerminalReady = () => {
    setTerminalReadiness({ deferred: deferTerminal, ready: true });
  };
  useEffect(() => {
    if (!graphical && !deferTerminal && terminalReadiness.deferred === deferTerminal && terminalReadiness.ready) onPrepared();
  }, [deferTerminal, graphical, onPrepared, terminalReadiness]);
  useEffect(() => {
    if (controlPending) return;
    if (graphical ? rendererReady : !deferTerminal && terminalReadiness.deferred === deferTerminal && terminalReadiness.ready) onReady();
  }, [controlPending, deferTerminal, graphical, onReady, rendererReady, terminalReadiness]);
  useEffect(() => {
    if (selected) return;
    const active = document.activeElement;
    if (active instanceof HTMLElement && paneRef.current?.contains(active)) active.blur();
  }, [selected]);
  const focusStatus = controlPending ? <span className="pane-focus-status" role="status" aria-label="Waiting for Herdr focus confirmation" title="Waiting for Herdr focus confirmation">⟳</span> : focusError ? <span className="pane-focus-status pane-focus-status-error" role="alert" aria-label={focusError.message} title={`${focusError.code}: ${focusError.message}`}><span aria-hidden="true">!</span><button type="button" className="pane-focus-retry" aria-label="Retry focus" onClick={onRetryFocus}>↻</button></span> : null;
  return <section ref={paneRef} className={`pane-view${selected || paintedSelected ? " is-selected" : ""}`} style={style} aria-label={title} inert={retained}
    onContextMenu={(event) => onContext(event, { kind: "pane", id: pane.id })}>
    <header className={`pane-header${solo && !graphical ? " is-hidden" : ""}`}>
      <button type="button" className="pane-header-select" onClick={onSelect} title={title}>
        <UiIcon name={graphical ? "file" : "terminal"} /><span className="pane-title">{graphical && renderer ? viewerTitle(renderer).title : title}</span>{graphical && renderer && viewerTitle(renderer).subtitle ? <span className="pane-subtitle" title={viewerTitle(renderer).path}>{viewerTitle(renderer).subtitle}</span> : null}
      </button>
      {!solo || graphical ? focusStatus : null}
      <button type="button" className="pane-header-expand" aria-label="Expand or restore pane" title={withShortcut("Expand / restore pane", "zoom-pane")} onClick={() => mutate(`pane:${pane.id}`, { type: "pane_zoom", pane_id: pane.id, mode: "toggle" })}><UiIcon name="expand" /></button>
    </header>
    {solo && !graphical ? <div className="pane-focus-overlay">{focusStatus}</div> : null}
    {graphical && renderer ? <div ref={graphicalRef} className="graphical-pane"
      onPointerDownCapture={(event) => {
        if (!controlAllowed) {
          event.preventDefault();
          intendedControl.current = event.target instanceof HTMLElement ? event.target.closest<HTMLElement>("button,input,select,textarea,[tabindex]") : null;
        }
      }}
      onFocusCapture={(event) => {
        if (!controlAllowed) {
          intendedControl.current = event.target;
          event.target.blur();
          onRequestControl();
        }
      }}>
      {isGraphicalReview(renderer) ? <ReviewViewer client={client} presentation={renderer.presentation} value={renderer.view} onChange={(value) => onRendererViewChange(renderer.presentation.binding_id, value)} onRequestControl={onRequestControl} onTerminalView={onTerminalView} /> : <ContextViewer client={client} presentation={renderer.presentation} value={renderer.view}
        onChange={(value) => onRendererViewChange(renderer.presentation.binding_id, value)}
        controlAllowed={controlAllowed} onRequestControl={onRequestControl} onTerminalView={onTerminalView} space={space} />}
    </div> : <div className="terminal-surface"><TerminalPane client={client} request={request} selected={selected} presented={paintedSelected} controlAllowed={controlAllowed} controlPending={controlPending} focusEpoch={focusEpoch} focusToken={focusToken} terminalMouseInput={terminalMouseInput} deferAttachment={deferTerminal} focusOnAttach={focusOnAttach} onRequestControl={onRequestControl} onSelect={onSelect} onReady={reportTerminalReady} onResync={onResync} onClosed={onResync} onClosePane={closePane} registerStream={registerStream} /></div>}
    {renderer?.actionError || (graphical && renderer?.inspectionError) ? <div className="pane-presentation-error" role="status"><span>{renderer.actionError ?? renderer.inspectionError}</span><button type="button" onClick={onRefreshRenderer}>Refresh</button>{graphical ? <button type="button" onClick={onTerminalView}>Terminal</button> : null}</div> : null}
  </section>;
}

function ResizeHandles({ layout, mutate }: { layout: TabLayout | undefined; mutate: Mutate }) {
  const canvasRef = useRef<HTMLDivElement | null>(null);
  const [preview, setPreview] = useState<{ id: string; delta: number } | null>(null);
  const drag = useRef<{ handle: ResizeHandle; pointerId: number; start: number } | null>(null);
  useEffect(() => () => document.body.classList.remove("is-resizing-panes"), []);
  const area = layout?.area;
  const pctX = (value: number) => area && area.width ? `${((value - area.x) / area.width) * 100}%` : "0%";
  const pctY = (value: number) => area && area.height ? `${((value - area.y) / area.height) * 100}%` : "0%";
  return <div ref={canvasRef} className="resize-layer">{deriveResizeHandles(layout).map((handle) => {
    const delta = preview?.id === handle.id ? preview.delta : 0;
    const style = handle.axis === "x" ? { left: pctX(handle.coordinate), top: pctY(handle.start), height: area ? `${handle.length / area.height * 100}%` : "0%", transform: `translateX(${delta}px)` } : { top: pctY(handle.coordinate), left: pctX(handle.start), width: area ? `${handle.length / area.width * 100}%` : "0%", transform: `translateY(${delta}px)` };
    return <div key={handle.id} role="separator" aria-label={`Resize pane ${handle.paneId} ${handle.axis === "x" ? "horizontally" : "vertically"}`} aria-orientation={handle.axis === "x" ? "vertical" : "horizontal"} tabIndex={0} className={`resize-handle resize-${handle.axis}`} style={style}
      onKeyDown={(event) => { const step = event.shiftKey ? 20 : 5; const deltaValue = handle.axis === "x" ? event.key === "ArrowLeft" ? -step : event.key === "ArrowRight" ? step : 0 : event.key === "ArrowUp" ? -step : event.key === "ArrowDown" ? step : 0; if (!deltaValue) return; event.preventDefault(); const bounds = canvasRef.current?.getBoundingClientRect(); const request = resizeRequest(handle, deltaValue, handle.axis === "x" ? bounds?.width ?? 0 : bounds?.height ?? 0); if (request) mutate(`pane:${handle.paneId}`, request); }}
      onPointerDown={(event) => { if (event.button !== 0) return; event.preventDefault(); event.currentTarget.setPointerCapture(event.pointerId); document.body.classList.add("is-resizing-panes"); drag.current = { handle, pointerId: event.pointerId, start: handle.axis === "x" ? event.clientX : event.clientY }; setPreview({ id: handle.id, delta: 0 }); }}
      onPointerMove={(event) => { const active = drag.current; if (!active || active.pointerId !== event.pointerId) return; setPreview({ id: active.handle.id, delta: (active.handle.axis === "x" ? event.clientX : event.clientY) - active.start }); }}
      onPointerUp={(event) => { const active = drag.current; if (!active || active.pointerId !== event.pointerId) return; const deltaValue = (active.handle.axis === "x" ? event.clientX : event.clientY) - active.start; const bounds = canvasRef.current?.getBoundingClientRect(); const request = resizeRequest(active.handle, deltaValue, active.handle.axis === "x" ? bounds?.width ?? 0 : bounds?.height ?? 0); drag.current = null; setPreview(null); document.body.classList.remove("is-resizing-panes"); if (request) mutate(`pane:${active.handle.paneId}`, request); }}
      onPointerCancel={() => { drag.current = null; setPreview(null); document.body.classList.remove("is-resizing-panes"); }} />;
  })}</div>;
}

type CommandAction = { id: string; label: string; shortcut?: string; group: "Navigate" | "Space" | "Tab" | "Pane" | "Browser" | "Library"; disabled?: boolean; reason?: string; reasonDetail?: string; run: () => void };

/** Picks the clause of a renderer diagnostic chain that explains one action, e.g. "Requires a safe source pane directory". */
export function rendererReasonFor(kind: RendererActionDefinition["kind"], reason: string): string | undefined {
  const prefix = kind === "review" ? "Open Review " : kind === "files" ? "Open files " : "Open Context ";
  const clause = reason.split(";").map((part) => part.trim()).find((part) => part.startsWith(prefix));
  if (!clause) return undefined;
  const rest = clause.slice(prefix.length);
  return rest.charAt(0).toLocaleUpperCase() + rest.slice(1);
}
type RendererActionDefinition = { id: string; label: string; direction: "right" | "down"; kind: "review" | "files" | "context" };

const rendererActionDefinitions: RendererActionDefinition[] = [
  { id: "review-right", label: "Open Review right", direction: "right", kind: "review" },
  { id: "review-down", label: "Open Review below", direction: "down", kind: "review" },
  { id: "files-right", label: "Open files right", direction: "right", kind: "files" },
  { id: "files-down", label: "Open files below", direction: "down", kind: "files" },
  { id: "context-right", label: "Open Context right", direction: "right", kind: "context" },
  { id: "context-down", label: "Open Context below", direction: "down", kind: "context" },
];

/** Prefix commands that only move focus or open a surface, so they stay available while a Herdr mutation is pending. */
const COMMANDS_ALLOWED_WHILE_BUSY: readonly PrefixCommand[] = ["previous-tab", "next-tab", "previous-pane", "next-pane", "focus-left", "focus-right", "focus-up", "focus-down", "resize", "toggle-sidebar", "focus-spaces", "focus-agents", "switch-session", "toggle-library", "open-file-picker"];

/** A shortcut as `kbd` chips: `Ctrl+B` `i` for a sequence, one chip for a chord, alternatives separated by "or". */
function ShortcutKeys({ text }: { text: string }) {
  return <span className="command-keys">{text.split(SHORTCUT_SEPARATOR).map((form, index) => <span className="command-key-form" key={form}>
    {index > 0 ? <span className="command-key-or">or</span> : null}
    {(form.startsWith("Ctrl+B ") ? ["Ctrl+B", form.slice("Ctrl+B ".length)] : [form]).map((chip) => <kbd key={chip}>{chip}</kbd>)}
  </span>)}</span>;
}

function CommandOverlay({ actions, statusContent, onSwitchSession, onDismiss }: { actions: CommandAction[]; statusContent?: ReactNode; onSwitchSession: () => void; onDismiss: () => void }) {
  const ref = useModalFocus<HTMLElement>(onDismiss);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const activeRowRef = useRef<HTMLButtonElement | null>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [showAll, setShowAll] = useState(false);
  const normalized = query.trim().toLocaleLowerCase();
  const primaryIds = ["prefix:zoom-pane", "prefix:toggle-library", "browser:open", "prefix:new-tab", "prefix:split-right", "prefix:switch-session", "renderer:review-right", "prefix:setup-space"];
  const ranked = normalized
    ? rankFuzzyMatches(query, actions, (action) => `${action.label} ${action.shortcut ?? ""} ${action.group}`)
    : actions.map((action, index) => ({ ...action, score: index, matchedIndices: [] as number[] }));
  const groups = ["Navigate", "Space", "Tab", "Pane", "Browser", "Library"] as const;
  // Rows render grouped, so keep `filtered` in that order for the highlight and arrow keys. With a query, groups follow their best match.
  const groupOrder: readonly CommandAction["group"][] = normalized ? [...new Set(ranked.map((action) => action.group))] : groups;
  const filtered = normalized || showAll
    ? groupOrder.flatMap((group) => ranked.filter((action) => action.group === group))
    : primaryIds.flatMap((id) => ranked.filter((action) => action.id === id));
  useEffect(() => setActive((current) => Math.min(current, Math.max(0, filtered.length - 1))), [filtered.length]);
  useEffect(() => { searchRef.current?.focus(); }, []);
  useEffect(() => { activeRowRef.current?.scrollIntoView?.({ block: "nearest" }); }, [active, normalized]);
  const runActive = () => {
    const action = filtered[active];
    if (action && !action.disabled) action.run();
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><section ref={ref} className="command-overlay" role="dialog" aria-modal="true" aria-labelledby="commands-title" onKeyDown={(event) => {
    const listStep = event.key === "ArrowDown" || (event.ctrlKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "n") ? 1 : event.key === "ArrowUp" || (event.ctrlKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "p") ? -1 : 0;
    if (listStep !== 0) { event.preventDefault(); setActive((current) => filtered.length === 0 ? 0 : (current + (listStep === 1 ? 1 : filtered.length - 1)) % filtered.length); return; }
    if (event.key === "Enter" && document.activeElement instanceof HTMLInputElement) { event.preventDefault(); runActive(); return; }
    trapModalTab(event, ref.current);
  }}><header><h2 id="commands-title">Commands</h2><button type="button" onClick={onDismiss} aria-label="Close commands"><UiIcon name="close" /></button></header><div className="command-search-box"><UiIcon name="search" /><input ref={searchRef} className="command-search" aria-label="Find a command" placeholder="Find a command…" autoComplete="off" value={query} onChange={(event) => { setQuery(event.target.value); setActive(0); }} /></div>{statusContent ? <div className="command-status">{statusContent}</div> : null}<div className="command-list" role="listbox" aria-label="Available commands">{filtered.length === 0 ? <p className="command-empty">No matching commands.</p> : groupOrder.map((group) => {
    const groupActions = !normalized && !showAll ? (group === "Navigate" ? filtered : []) : filtered.filter((action) => action.group === group);
    if (groupActions.length === 0) return null;
    return <section className="command-group" key={group}><h3>{group}</h3>{groupActions.map((action) => {
      const index = filtered.indexOf(action);
      return <button ref={index === active ? activeRowRef : null} type="button" role="option" aria-selected={index === active} className={`command-row${index === active ? " is-active" : ""}`} key={action.id} disabled={action.disabled} onMouseMove={() => { if (index !== active) setActive(index); }} onClick={() => action.run()}><UiIcon name={action.group === "Pane" ? "terminal" : action.group === "Navigate" ? "grid" : "right"} /><span className="command-row-label"><span>{Array.from(action.label, (character, characterIndex) => action.matchedIndices.includes(characterIndex) ? <mark key={characterIndex}>{character}</mark> : character)}</span>{action.disabled && action.reason ? <small title={action.reasonDetail ?? action.reason}>{action.reason}</small> : null}</span>{action.shortcut ? <ShortcutKeys text={action.shortcut} /> : null}</button>;
    })}</section>;
  })}</div><footer className="command-footer"><span>↑↓ or Ctrl+N/P navigate · Enter choose · Esc close · type a name or a key</span><button type="button" onClick={() => { setShowAll((value) => !value); setActive(0); }}>{showAll ? "Quick commands" : "All commands"}</button></footer></section></div>;
}

export function moveDestinationLabel(tab: Tab, spaces: Space[]): string {
  const space = spaces.find((candidate) => candidate.id === tab.space_id);
  return `${space?.label ?? `Space ${space?.number ?? "?"}`} / ${tab.label || `Tab ${tab.number}`}`;
}

function PaneDialogOverlay({ dialog, panes, tabs, spaces, busy, onDismiss, mutate }: { dialog: PaneDialog; panes: Pane[]; tabs: Tab[]; spaces: Space[]; busy: boolean; onDismiss: () => void; mutate: Mutate }) {
  const pane = panes.find((candidate) => candidate.id === dialog.paneId);
  const [value, setValue] = useState(dialog.kind === "rename" ? pane?.title ?? "" : "");
  const ref = useModalFocus<HTMLFormElement>(onDismiss);
  if (!pane) return null;
  const submit = () => {
    const key = `pane:${pane.id}`;
    let accepted = false;
    if (dialog.kind === "rename") accepted = mutate(key, { type: "pane_rename", pane_id: pane.id, label: value.trim() || null });
    if (dialog.kind === "swap" && value) accepted = mutate(key, { type: "pane_swap", source_pane_id: pane.id, target_pane_id: value });
    if (dialog.kind === "move" && value === "new-tab") accepted = mutate(key, { type: "pane_move", pane_id: pane.id, destination: { type: "new_tab", space_id: pane.space_id, label: null } }, true);
    if (dialog.kind === "move" && value === "new-space") accepted = mutate(key, { type: "pane_move", pane_id: pane.id, destination: { type: "new_space", label: null, tab_label: null } }, true);
    if (dialog.kind === "move" && value.startsWith("tab:")) accepted = mutate(key, { type: "pane_move", pane_id: pane.id, destination: { type: "existing_tab", tab_id: value.slice(4), direction: "right", target_pane_id: null, ratio: null } }, true);
    if (accepted) onDismiss();
  };
  const onChooserKeyDown = (event: ReactKeyboardEvent<HTMLFormElement>) => {
    if (event.target instanceof HTMLSelectElement && event.ctrlKey && !event.altKey && !event.metaKey
      && (event.key.toLowerCase() === "n" || event.key.toLowerCase() === "p")) {
      const choices = [...event.target.options].filter((option) => option.value !== "");
      if (choices.length > 0) {
        event.preventDefault();
        const current = choices.findIndex((option) => option.value === value);
        const direction = event.key.toLowerCase() === "n" ? 1 : -1;
        const next = current < 0 ? (direction > 0 ? 0 : choices.length - 1) : (current + direction + choices.length) % choices.length;
        setValue(choices[next].value);
      }
      return;
    }
    trapModalTab(event, ref.current);
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><form ref={ref} className="chooser-overlay" role="dialog" aria-modal="true" aria-labelledby="chooser-title" onSubmit={(event) => { event.preventDefault(); submit(); }} onKeyDown={onChooserKeyDown}>
    <h2 id="chooser-title">{dialog.kind} pane</h2>
    {dialog.kind === "rename" ? <input aria-label="Pane name" value={value} onChange={(event) => setValue(event.target.value)} /> : <select aria-label={dialog.kind === "swap" ? "Swap target" : "Move destination"} value={value} onChange={(event) => setValue(event.target.value)}><option value="">Choose...</option>{dialog.kind === "swap" ? panes.filter((candidate) => candidate.id !== pane.id).map((candidate) => <option key={candidate.id} value={candidate.id}>{candidate.title ?? `Pane ${panes.findIndex((item) => item.id === candidate.id) + 1}`}</option>) : <><option value="new-tab">New tab in this space</option><option value="new-space">New space</option>{tabs.filter((tab) => tab.id !== pane.tab_id).map((tab) => <option key={tab.id} value={`tab:${tab.id}`}>{moveDestinationLabel(tab, spaces)}</option>)}</>}</select>}
    <footer><button type="button" onClick={onDismiss}>Cancel</button><button type="submit" disabled={busy || (dialog.kind !== "rename" && !value)}>{dialog.kind}</button></footer>
  </form></div>;
}

export function reconcileSessionChoice(sessions: SessionSummary[], selected: string, currentSessionId: string | null): string {
  if (sessions.some((session) => session.id === selected)) return selected;
  if (currentSessionId && sessions.some((session) => session.id === currentSessionId)) return currentSessionId;
  return sessions[0]?.id ?? "";
}

function SessionDialogOverlay({ sessions, currentSessionId, onRefresh, onDismiss, onSession }: { sessions: SessionSummary[]; currentSessionId: string | null; onRefresh: () => Promise<void>; onDismiss: () => void; onSession: (sessionId: string) => void }) {
  const [sessionId, setSessionId] = useState(() => reconcileSessionChoice(sessions, "", currentSessionId));
  const [query, setQuery] = useState("");
  const selectedSessionRef = useRef<HTMLButtonElement | null>(null);
  const ref = useModalFocus<HTMLFormElement>(onDismiss);
  useEffect(() => setSessionId((selected) => reconcileSessionChoice(sessions, selected, currentSessionId)), [sessions, currentSessionId]);
  const normalized = query.trim().toLocaleLowerCase();
  const filteredSessions = sessions.filter((session) => !normalized || `${session.label} ${session.id} ${session.running ? "running" : "stopped"}`.toLocaleLowerCase().includes(normalized));
  useEffect(() => { selectedSessionRef.current?.scrollIntoView?.({ block: "nearest" }); }, [sessionId, normalized]);
  const selectRelativeSession = (direction: 1 | -1) => {
    if (filteredSessions.length === 0) return;
    const current = filteredSessions.findIndex((session) => session.id === sessionId);
    const next = current < 0 ? (direction === 1 ? 0 : filteredSessions.length - 1) : (current + direction + filteredSessions.length) % filteredSessions.length;
    setSessionId(filteredSessions[next].id);
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><form ref={ref} className="chooser-overlay session-chooser" role="dialog" aria-modal="true" aria-labelledby="session-chooser-title" onSubmit={(event) => { event.preventDefault(); if (sessionId && sessionId !== currentSessionId) onSession(sessionId); onDismiss(); }} onKeyDown={(event) => {
    if (event.ctrlKey && !event.altKey && !event.metaKey && (event.key.toLowerCase() === "n" || event.key.toLowerCase() === "p")) { event.preventDefault(); selectRelativeSession(event.key.toLowerCase() === "n" ? 1 : -1); return; }
    trapModalTab(event, ref.current);
  }}>
    <h2 id="session-chooser-title">Switch session</h2>
    {sessions.length === 0 ? <div className="empty-choice" role="status">No sessions are available.</div> : <><input className="session-search" aria-label="Find a session" placeholder="Find a session…" autoComplete="off" value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={(event) => { if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); selectRelativeSession(event.key === "ArrowDown" ? 1 : -1); } }} /><div className="session-list" role="listbox" aria-label="Session">{filteredSessions.length === 0 ? <p className="empty-choice" role="status">No sessions match.</p> : filteredSessions.map((session) => <button ref={session.id === sessionId ? selectedSessionRef : null} key={session.id} type="button" role="option" aria-selected={session.id === sessionId} data-session-id={session.id} className={`session-choice${session.id === sessionId ? " is-selected" : ""}`} onClick={() => setSessionId(session.id)}><span>{session.label}</span><small>{session.running ? "running" : "stopped"}</small></button>)}</div></>}
    <footer>{sessions.length === 0 ? <button type="button" onClick={() => { void onRefresh().catch(() => undefined); }}>Refresh</button> : null}<button type="button" onClick={onDismiss}>Cancel</button><button type="submit" disabled={!sessionId || sessionId === currentSessionId}>Switch</button></footer>
  </form></div>;
}

export function mutationFailureCanRetry(request: ResourceMutationRequest, code: string | undefined): boolean {
  if (code === "mutation_applied_snapshot_failed" || code === "request_outcome_unknown") return false;
  if (request.type === "space_rename" || request.type === "space_move_block" || request.type === "tab_rename" || request.type === "tab_move" || request.type === "pane_rename") return true;
  return request.type === "pane_zoom" && request.mode !== "toggle";
}

function RecoveryPanel({ state, mutations, onReconnect, onRetryMutation }: { state: SessionState; mutations: MutationCoordinatorState; onReconnect: () => void; onRetryMutation: (operation: MutationOperation) => void }) {
  const failures = Object.values(mutations.errors);
  if (!state.syncError && failures.length === 0) return null;
  return <aside className="recovery-panel" aria-label="Recovery" role="alert">
    {state.syncError ? <div><span>{state.syncError.message}</span><button type="button" onClick={onReconnect}>Resync</button></div> : null}
    {failures.map((failure) => <div key={`${failure.operation.key}:${failure.operation.token}`}><span>{failure.message}</span>{failure.code ? <code>{failure.code}</code> : null}{mutationFailureCanRetry(failure.operation.request, failure.code) ? <button type="button" onClick={() => onRetryMutation(failure.operation)}>Retry</button> : null}<button type="button" onClick={onReconnect}>Resync</button></div>)}

  </aside>;
}
type BrowserPresentationState = { associationOpen: boolean; visible: boolean; presentation: BrowserViewPresentation };
type BrowserGuardEntry = { key: string; recovery: BrowserPaneRecoveryRegistration; settling?: Promise<void> };
type BrowserGuardHandoff = {
  current: BrowserGuardEntry | null;
  outgoing: BrowserGuardEntry | null;
  incoming: BrowserGuardEntry | null;
  notify?: () => void;
};
const BROWSER_FALLBACK_VIEWPORT: BrowserViewViewportRequest = { css_width: 800, css_height: 600, device_pixel_ratio: 1 };
const BROWSER_SPLIT_MIN_RATIO = 0.25;
const BROWSER_SPLIT_MAX_RATIO = 0.65;
const BROWSER_SPLIT_DEFAULT_RATIO = 0.42;
const BROWSER_SPLIT_KEY = "cockpit.browser.split-ratio";

function boundedBrowserViewport(width: number, height: number, devicePixelRatio: number): BrowserViewViewportRequest {
  // CSS dimensions remain logical viewport coordinates; DPR controls capture
  // density and is kept separate from annotation and input coordinates.
  return {
    css_width: Math.max(1, Math.min(2560, Math.round(Number.isFinite(width) && width > 0 ? width : BROWSER_FALLBACK_VIEWPORT.css_width))),
    css_height: Math.max(1, Math.min(1600, Math.round(Number.isFinite(height) && height > 0 ? height : BROWSER_FALLBACK_VIEWPORT.css_height))),
    device_pixel_ratio: Math.max(0.1, Math.min(16, Number.isFinite(devicePixelRatio) && devicePixelRatio > 0 ? devicePixelRatio : 1)),
  };
}

function boundedBrowserSplitRatio(value: number): number {
  return Math.max(BROWSER_SPLIT_MIN_RATIO, Math.min(BROWSER_SPLIT_MAX_RATIO, Number.isFinite(value) ? value : BROWSER_SPLIT_DEFAULT_RATIO));
}

function readBrowserSplitRatio(key: string): number {
  if (typeof window === "undefined") return BROWSER_SPLIT_DEFAULT_RATIO;
  try {
    const stored = window.sessionStorage.getItem(`${BROWSER_SPLIT_KEY}:${key}`);
    return stored === null ? BROWSER_SPLIT_DEFAULT_RATIO : boundedBrowserSplitRatio(Number(stored));
  } catch {
    return BROWSER_SPLIT_DEFAULT_RATIO;
  }
}

const SIDEBAR_MIN_WIDTH = 224;
const SIDEBAR_MAX_WIDTH = 360;
const SIDEBAR_DEFAULT_WIDTH = 240;
const SIDEBAR_WIDTH_KEY = "cockpit.sidebar.width";
const SIDEBAR_COLLAPSED_KEY = "cockpit.sidebar.collapsed";

function isNarrowViewport(): boolean {
  return typeof window !== "undefined" && window.innerWidth <= 800;
}

function readSidebarWidth(): number {
  if (typeof window === "undefined") return SIDEBAR_DEFAULT_WIDTH;
  try {
    const stored = window.localStorage.getItem(SIDEBAR_WIDTH_KEY);
    const value = stored === null ? NaN : Number(stored);
    return Number.isFinite(value) ? Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, value)) : SIDEBAR_DEFAULT_WIDTH;
  } catch {
    return SIDEBAR_DEFAULT_WIDTH;
  }
}

function readSidebarCollapsed(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "true";
  } catch {
    return false;
  }
}

function Workbench({ client, state, sessions, selection, controlPaneId, terminalMouseInput, mutations, browserHandoff, onSession, onFocus, onPanePrepared, onRequestControl, onReconnect, onRetry, onRefreshSessions, onOpenSession, onMutate, onRetryMutation }: {
  client: CockpitClient; state: SessionState; sessions: SessionSummary[]; selection: Selection; controlPaneId: string | null; terminalMouseInput: boolean; mutations: MutationCoordinatorState; browserHandoff: BrowserGuardHandoff;
  onSession: (id: string) => void; onFocus: (request: FocusRequest, location: Selection, prepare?: { paneId: string }) => void; onPanePrepared: (paneId: string) => void; onRequestControl: (paneId: string) => void; onReconnect: () => void; onRetry: () => void; onRefreshSessions: () => Promise<void>; onOpenSession: () => void; onMutate: Mutate; onRetryMutation: (operation: MutationOperation) => void;
}) {
  const snapshot = state.snapshot;
  const spaces = snapshot?.spaces ?? [];
  const selectedSpace = byId(spaces, selection.spaceId);
  // Herdr's selected Space is the Library view's and palette add's target; no focus or selection request follows from it.
  const librarySpace: LibrarySpace | null = state.sessionId && selectedSpace
    ? { target: { session_id: state.sessionId, space_id: selectedSpace.id }, label: selectedSpace.label, live: state.sync === "live" }
    : null;
  const setupParent = setupParentFor(selectedSpace, snapshot?.panes ?? [], snapshot?.focused_pane_id ?? null);
  const allTabs = snapshot?.tabs ?? [];
  const tabs = tabsForSpace(allTabs, selection.spaceId);
  const selectedTab = byId(tabs, selection.tabId);
  const layout = snapshot?.layouts.find((candidate) => candidate.tab_id === selectedTab?.id && candidate.space_id === selection.spaceId);
  const panes = panesForTab(snapshot?.panes ?? [], selectedTab?.id ?? null);
  const visiblePaneIds = projectedPaneIds(panes.map((pane) => pane.id), layout, snapshot?.focused_pane_id ?? selection.paneId);
  const visiblePanes = panes.filter((pane) => visiblePaneIds.includes(pane.id));
  const allPaneIds = (snapshot?.panes ?? []).map((pane) => pane.id);
  const spaceGit = useSpaceGitStatus(client, state.sync === "live" ? state.sessionId : null, spaceCheckoutKey(spaces, snapshot?.panes ?? []));
  // While the Library view covers the work area no pane renderer is polled or mounted (D13).
  const [libraryOpen, setLibraryOpen] = useState(false);
  const renderers = usePaneRenderers(client, state.sessionId, libraryOpen ? [] : visiblePaneIds, (snapshot?.panes ?? []).map((pane) => pane.id), state.sync === "live", state.epoch, onReconnect);
  // Keep incoming panes mounted for fitting and first-frame rendering, but out
  // of the painted frame until each visible pane reports readiness.
  // Herdr still owns selection and layout.
  // Inspection binding IDs include volatile foreground-process identity. They
  // authorize renderer requests, but must not reset a live terminal renderer.
  const rendererKey = visiblePaneIds.map((paneId) => {
    const renderer = renderers.panes[paneId];
    const graphical = renderer && (isGraphicalContext(renderer) || isGraphicalReview(renderer));
    return `${paneId}:${graphical ? `${renderer.presentation.renderer}:${renderer.choice ?? ""}` : "terminal"}`;
  }).join("\0");
  const paneRenderKey = selectedTab && visiblePanes.length > 0 ? `${selectedTab.id}\0${visiblePaneIds.join("\0")}\0${rendererKey}` : null;
  const paneProjection: PaneCanvasProjection = { key: paneRenderKey, panes: visiblePanes, layout, visiblePaneIds, selectedPaneId: selection.paneId && visiblePaneIds.includes(selection.paneId) ? selection.paneId : null };
  const committedProjection = useRef<PaneCanvasProjection | null>(null);
  // Terminals attached at the last commit stay attached while the session
  // resyncs after a mutation, so a split drag or rename does not reattach them.
  const attachedTerminals = useRef<{ epoch: number; paneIds: ReadonlySet<string> }>({ epoch: -1, paneIds: new Set() });
  const nextAttachedTerminals = new Set<string>();
  useLayoutEffect(() => {
    attachedTerminals.current = { epoch: state.epoch, paneIds: nextAttachedTerminals };
  });
  const sessionResyncing = state.sync === "loading" && state.snapshot !== null && attachedTerminals.current.epoch === state.epoch;
  const [paneReadiness, setPaneReadiness] = useState<{ key: string | null; paneIds: ReadonlySet<string> }>({ key: null, paneIds: new Set() });
  useLayoutEffect(() => {
    setPaneReadiness({ key: paneRenderKey, paneIds: new Set() });
  }, [paneRenderKey]);
  const markPaneReady = useCallback((paneId: string) => {
    setPaneReadiness((current) => {
      if (current.key !== paneRenderKey || current.paneIds.has(paneId)) return current;
      const paneIds = new Set(current.paneIds);
      paneIds.add(paneId);
      return { key: current.key, paneIds };
    });
  }, [paneRenderKey]);
  const paneCanvasReady = paneRenderKey === null
    || (paneReadiness.key === paneRenderKey && visiblePaneIds.every((paneId) => paneReadiness.paneIds.has(paneId)));
  useLayoutEffect(() => {
    if (paneCanvasReady) committedProjection.current = paneProjection;
  });
  const retainedProjection = !paneCanvasReady ? committedProjection.current : null;
  const currentPaneIds = new Set(paneProjection.visiblePaneIds);
  const outgoingProjection = retainedProjection
    && retainedProjection.key !== paneProjection.key
    && !retainedProjection.visiblePaneIds.some((paneId) => currentPaneIds.has(paneId))
    ? retainedProjection
    : null;
  const paneCanvasVisible = paneCanvasReady || outgoingProjection !== null || committedProjection.current === null;
  const [menu, setMenu] = useState<ContextMenuState | null>(null);
  const [editing, setEditing] = useState<ContextTarget | null>(null);
  const [dialog, setDialog] = useState<PaneDialog | null>(null);
  const [commandsOpen, setCommandsOpen] = useState(false);
  const [sessionChooserOpen, setSessionChooserOpen] = useState(false);
  const [setupOpen, setSetupOpen] = useState(false);
  const [recoveryOpen, setRecoveryOpen] = useState(false);
  const [teardownSpaceId, setTeardownSpaceId] = useState<string | null>(null);
  const [libraryAddOpen, setLibraryAddOpen] = useState(false);
  const [libraryCommand, setLibraryCommand] = useState<LibraryCommand | null>(null);
  const libraryCommandToken = useRef(0);
  // Set when the Library closes; cleared by an explicit local pane, tab or Space action.
  const [attachFocusSuppressed, setAttachFocusSuppressed] = useState(false);
  // The pane the Library was opened from, so closing it with the toggle key returns focus there (design R2).
  const libraryOrigin = useRef<{ paneId: string; graphical: boolean } | null>(null);
  const selectedPaneIdRef = useRef(selection.paneId);
  selectedPaneIdRef.current = selection.paneId;
  const openLibrary = useCallback((command?: { kind: "refresh" } | { kind: "tokens" } | { kind: "open"; itemId: string }) => {
    const active = document.activeElement;
    libraryOrigin.current = active instanceof HTMLElement && active.closest(".pane-view") && selectedPaneIdRef.current
      ? { paneId: selectedPaneIdRef.current, graphical: Boolean(active.closest(".graphical-pane")) }
      : null;
    setLibraryOpen(true);
    // A command belongs to this opening only; reopening must not replay it.
    setLibraryCommand(command ? { ...command, token: ++libraryCommandToken.current } : null);
  }, []);
  const closeLibrary = useCallback(() => {
    libraryOrigin.current = null;
    setLibraryOpen(false);
    setAttachFocusSuppressed(true);
  }, []);
  // Closing with the toggle key: the originating pane, if still selected, takes DOM focus back (a terminal once it attaches).
  const closeLibraryToOrigin = useCallback(() => {
    const origin = libraryOrigin.current;
    libraryOrigin.current = null;
    const returnToPane = origin !== null && origin.paneId === selectedPaneIdRef.current;
    setLibraryOpen(false);
    setAttachFocusSuppressed(!returnToPane);
    if (returnToPane && origin.graphical) {
      const focusDocument = (attempts: number) => {
        const document_ = document.querySelector<HTMLElement>(".pane-view.is-selected .context-document, .pane-view.is-selected .review-diff");
        if (document_) document_.focus({ preventScroll: true });
        else if (attempts > 0) requestAnimationFrame(() => focusDocument(attempts - 1));
      };
      requestAnimationFrame(() => focusDocument(60));
    }
  }, []);
  const [prefixActive, setPrefixActive] = useState(false);
  const [prefixHint, setPrefixHint] = useState<string | null>(null);
  useEffect(() => {
    if (prefixHint === null) return;
    const timer = window.setTimeout(() => setPrefixHint(null), 2000);
    return () => window.clearTimeout(timer);
  }, [prefixHint]);
  const [sidebarWidth, setSidebarWidth] = useState(readSidebarWidth);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readSidebarCollapsed);
  const browserSizeKey = `${state.sessionId}:${selection.spaceId}`;
  const [browserSplitRatio, setBrowserSplitRatio] = useState(() => readBrowserSplitRatio(browserSizeKey));
  useEffect(() => setBrowserSplitRatio(readBrowserSplitRatio(browserSizeKey)), [browserSizeKey]);
  const [narrowViewport, setNarrowViewport] = useState(isNarrowViewport);
  const [drawerOpen, setDrawerOpen] = useState(() => !isNarrowViewport());
  const sidebarReturnFocus = useRef<HTMLElement | null>(null);
  const sidebarCloseRef = useRef<HTMLButtonElement | null>(null);
  const drawerFocusTarget = useRef<{ spaceId: string; paneId: string | null } | null>(null);
  const mutationBusy = mutations.pending !== null || renderers.busy;
  const modalOpen = dialog !== null || commandsOpen || sessionChooserOpen || setupOpen || recoveryOpen || teardownSpaceId !== null || libraryAddOpen;
  const sidebarSession = sessions.find((session) => session.id === state.sessionId);
  const openSessionChooser = useCallback(() => {
    setSessionChooserOpen(true);
    onOpenSession();
  }, [onOpenSession]);
  const closeDrawer = useCallback((restoreFocus = true) => {
    setDrawerOpen(false);
    drawerFocusTarget.current = null;
    if (restoreFocus) window.setTimeout(() => {
      const target = sidebarReturnFocus.current ?? document.querySelector<HTMLElement>(".drawer-toggle");
      target?.focus({ preventScroll: true });
      sidebarReturnFocus.current = null;
    }, 0);
  }, []);
  const openDrawer = useCallback(() => {
    if (!narrowViewport) return;
    sidebarReturnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setDrawerOpen(true);
  }, [narrowViewport]);
  const updateSidebarWidth = useCallback((next: number) => {
    const width = Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, next));
    setSidebarWidth(width);
    try { window.localStorage.setItem(SIDEBAR_WIDTH_KEY, String(width)); } catch { /* local preferences are optional */ }
  }, []);
  const updateBrowserSplitRatio = useCallback((next: number) => {
    const ratio = boundedBrowserSplitRatio(next);
    setBrowserSplitRatio(ratio);
    try { window.sessionStorage.setItem(`${BROWSER_SPLIT_KEY}:${browserSizeKey}`, String(ratio)); } catch { /* local preferences are optional */ }
  }, [browserSizeKey]);
  const toggleSidebarCollapsed = useCallback(() => {
    setSidebarCollapsed((collapsed) => {
      const next = !collapsed;
      try { window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(next)); } catch { /* local preferences are optional */ }
      return next;
    });
  }, []);
  useEffect(() => {
    let previous = isNarrowViewport();
    const update = () => {
      const narrow = isNarrowViewport();
      setNarrowViewport(narrow);
      if (narrow !== previous) setDrawerOpen(!narrow);
      previous = narrow;
    };
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);
  useEffect(() => {
    if (!narrowViewport || !drawerOpen) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closeDrawer();
        return;
      }
      if (event.key !== "Tab") return;
      const controls = [...document.querySelectorAll<HTMLElement>(".sidebar:not([hidden]) button:not(:disabled), .sidebar:not([hidden]) input:not(:disabled), .sidebar:not([hidden]) select:not(:disabled), .sidebar:not([hidden]) [tabindex]:not([tabindex='-1'])")];
      if (controls.length === 0) return;
      event.preventDefault();
      const current = controls.indexOf(document.activeElement as HTMLElement);
      controls[(current + (event.shiftKey ? controls.length - 1 : 1)) % controls.length]?.focus();
    };
    window.addEventListener("keydown", handleKeyDown, true);
    window.setTimeout(() => sidebarCloseRef.current?.focus({ preventScroll: true }), 0);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, [closeDrawer, drawerOpen, narrowViewport]);
  useEffect(() => {
    const target = drawerFocusTarget.current;
    if (!target || !narrowViewport || !drawerOpen || state.focusPending || state.focusError) return;
    if (selection.spaceId !== target.spaceId) return;
    if (target.paneId && selection.paneId !== target.paneId) return;
    closeDrawer();
  }, [closeDrawer, drawerOpen, narrowViewport, selection.paneId, selection.spaceId, state.focusError, state.focusPending]);
  const [browserError, setBrowserError] = useState<(StatusError & { action: string }) | null>(null);
  const [browserBusy, setBrowserBusy] = useState(false);
  const [browserInputActive, setBrowserInputActive] = useState(false);
  const [browserPresentation, setBrowserPresentation] = useState<Record<string, BrowserPresentationState>>({});
  const [browserViewport, setBrowserViewport] = useState<BrowserViewViewportRequest>(BROWSER_FALLBACK_VIEWPORT);
  const browserRegionRef = useRef<HTMLDivElement | null>(null);
  const browserTarget: BrowserTarget | null = state.sessionId && selection.spaceId ? {
    session_id: state.sessionId,
    space_id: selection.spaceId,
    pane_id: null,
    endpoint_path: null,
  } : null;
  const browserKey = state.sessionId && selection.spaceId ? `${state.sessionId}:${selection.spaceId}` : null;
  const selectedBrowserPresentation = browserKey ? browserPresentation[browserKey] : undefined;
  const previousBrowserKeyRef = useRef<string | null>(browserKey);
  const browserKeyChanged = previousBrowserKeyRef.current !== browserKey;
  const browserPresentationRef = useRef(browserPresentation);
  browserPresentationRef.current = browserPresentation;
  const browserClientIdRef = useRef<string | null>(null);
  if (browserClientIdRef.current === null) {
    browserClientIdRef.current = `cockpit-browser-${globalThis.crypto?.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`}`;
  }
  const browserClientId = browserClientIdRef.current;
  const browserCloseGuardRef = browserHandoff;
  const settleBrowserOutgoing = useCallback((outgoing: BrowserGuardEntry): void => {
    const settle = () => {
      if (browserCloseGuardRef.outgoing !== outgoing) return;
      browserCloseGuardRef.outgoing = null;
      const incoming = browserCloseGuardRef.incoming;
      browserCloseGuardRef.incoming = null;
      if (incoming) browserCloseGuardRef.current = incoming;
      browserCloseGuardRef.notify?.();
    };
    const settling = outgoing.recovery.guard().then(settle);
    outgoing.settling = settling;
    void settling.catch(() => {
      if (browserCloseGuardRef.outgoing === outgoing) {
        outgoing.settling = undefined;
        browserCloseGuardRef.notify?.();
      }
    });
  }, [browserCloseGuardRef]);
  const registerBrowserCloseGuard = useCallback((recovery: BrowserPaneRecoveryRegistration | null) => {
    if (!browserKey) return;
    if (recovery) {
      if (browserCloseGuardRef.outgoing) {
        browserCloseGuardRef.incoming = { key: browserKey, recovery };
        browserCloseGuardRef.notify?.();
        return;
      }
      browserCloseGuardRef.current = { key: browserKey, recovery };
    } else if (browserCloseGuardRef.current?.key === browserKey) {
      const outgoing = browserCloseGuardRef.current;
      browserCloseGuardRef.outgoing = outgoing;
      browserCloseGuardRef.current = null;
      settleBrowserOutgoing(outgoing);
    } else if (browserCloseGuardRef.incoming?.key === browserKey) {
      browserCloseGuardRef.incoming = null;
      browserCloseGuardRef.notify?.();
    }
  }, [browserCloseGuardRef, browserKey, settleBrowserOutgoing]);
  const browserBlockedByOutgoing = Boolean(browserHandoff.outgoing);
  const retryBrowserHandoff = useCallback(() => {
    const outgoing = browserHandoff.outgoing;
    if (!outgoing || outgoing.settling) return;
    const settling = outgoing.recovery.retry().then(() => outgoing.recovery.guard()).then(() => {
      if (browserHandoff.outgoing === outgoing) {
        browserHandoff.outgoing = null;
        const incoming = browserHandoff.incoming;
        browserHandoff.incoming = null;
        if (incoming) browserHandoff.current = incoming;
        browserHandoff.notify?.();
      }
    });
    outgoing.settling = settling;
    void settling.catch(() => {
      if (browserHandoff.outgoing === outgoing) {
        outgoing.settling = undefined;
        browserHandoff.notify?.();
      }
    });
  }, [browserHandoff]);
  const discardBrowserHandoff = useCallback(() => {
    const outgoing = browserHandoff.outgoing;
    if (!outgoing || outgoing.settling) return;
    const settling = outgoing.recovery.discard().then(() => outgoing.recovery.guard()).then(() => {
      if (browserHandoff.outgoing === outgoing) {
        browserHandoff.outgoing = null;
        const incoming = browserHandoff.incoming;
        browserHandoff.incoming = null;
        if (incoming) browserHandoff.current = incoming;
        browserHandoff.notify?.();
      }
    });
    outgoing.settling = settling;
    void settling.catch(() => {
      if (browserHandoff.outgoing === outgoing) {
        outgoing.settling = undefined;
        browserHandoff.notify?.();
      }
    });
  }, [browserHandoff]);
  // Presentation state is intentionally independent from Herdr sync. A stale
  // session can still safely display the last browser frame for this exact
  // session/Space, while browser actions remain live-state guarded below.
  const browserOnly = !browserKeyChanged && selectedBrowserPresentation?.presentation === "browser_only";
  const browserAssociationOpen = Boolean(browserTarget && selectedBrowserPresentation?.associationOpen) && !browserBlockedByOutgoing;
  const browserVisible = Boolean(browserAssociationOpen && selectedBrowserPresentation?.visible);
  const browserSyncUnavailable = state.sync !== "live";
  const browserSyncMessage = state.sync === "disconnected"
    ? "Herdr session disconnected; showing the last confirmed browser frame."
    : state.sync === "stale"
      ? "Herdr session is stale; showing the last confirmed browser frame."
      : "Herdr session is resyncing; showing the last confirmed browser frame.";
  useEffect(() => {
    const previousKey = previousBrowserKeyRef.current;
    previousBrowserKeyRef.current = browserKey;
    if (previousKey === browserKey) return;
    setBrowserInputActive(false);
    setBrowserPresentation((current) => {
      let changed = false;
      const next = { ...current };
      for (const key of [previousKey, browserKey]) {
        if (key && next[key]?.presentation === "browser_only") {
          next[key] = { ...next[key], presentation: "split" };
          changed = true;
        }
      }
      return changed ? next : current;
    });
  }, [browserKey]);
  const setBrowserOnlyPresentation = useCallback((presentation: BrowserViewPresentation) => {
    if (!browserKey) return;
    setBrowserPresentation((current) => {
      const existing = current[browserKey];
      if (!existing || existing.presentation === presentation) return current;
      return { ...current, [browserKey]: { ...existing, presentation } };
    });
  }, [browserKey]);
  const enterBrowserOnly = useCallback(() => {
    if (!browserVisible) return;
    setBrowserOnlyPresentation("browser_only");
  }, [browserVisible, setBrowserOnlyPresentation]);
  const backToTerminals = useCallback(() => {
    setBrowserInputActive(false);
    setBrowserOnlyPresentation("split");
  }, [setBrowserOnlyPresentation]);
  const hideBrowser = useCallback(() => {
    setBrowserInputActive(false);
    if (!browserKey) return;
    setBrowserPresentation((current) => {
      const existing = current[browserKey];
      return existing && !existing.visible && existing.presentation === "split" ? current : {
        ...current,
        [browserKey]: { associationOpen: existing?.associationOpen ?? false, visible: false, presentation: "split" },
      };
    });
  }, [browserKey]);
  useEffect(() => {
    if (!browserVisible) return;
    const region = browserRegionRef.current;
    if (!region) return;
    const updateViewport = () => {
      const surface = region.querySelector<HTMLElement>(".browser-surface");
      if (!surface) return;
      const bounds = surface.getBoundingClientRect();
      if (!(bounds.width > 0 && bounds.height > 0)) return;
      setBrowserViewport(boundedBrowserViewport(bounds.width, bounds.height, window.devicePixelRatio));
    };
    updateViewport();
    window.addEventListener("resize", updateViewport);
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(updateViewport);
    observer?.observe(region);
    const surface = region.querySelector<HTMLElement>(".browser-surface");
    if (surface) observer?.observe(surface);
    return () => {
      window.removeEventListener("resize", updateViewport);
      observer?.disconnect();
    };
  }, [browserVisible, browserKey, browserSplitRatio, narrowViewport, browserSyncUnavailable]);
  const browserSplitterKeyDown = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Home") {
      event.preventDefault();
      updateBrowserSplitRatio(BROWSER_SPLIT_MIN_RATIO);
    } else if (event.key === "End") {
      event.preventDefault();
      updateBrowserSplitRatio(BROWSER_SPLIT_MAX_RATIO);
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      event.preventDefault();
      updateBrowserSplitRatio(browserSplitRatio + 0.02);
    } else if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      event.preventDefault();
      updateBrowserSplitRatio(browserSplitRatio - 0.02);
    }
  }, [browserSplitRatio, updateBrowserSplitRatio]);
  const browserSplitterPointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const content = event.currentTarget.parentElement;
    if (!content) return;
    event.preventDefault();
    const bounds = content.getBoundingClientRect();
    const size = narrowViewport ? bounds.height : bounds.width;
    if (!(size > 0)) return;
    const move = (next: PointerEvent) => {
      const coordinate = narrowViewport ? next.clientY : next.clientX;
      const ratio = narrowViewport ? (bounds.bottom - coordinate) / size : (bounds.right - coordinate) / size;
      updateBrowserSplitRatio(ratio);
    };
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
  }, [narrowViewport, updateBrowserSplitRatio]);
  const browserBusyRef = useRef(false);
  const browserRequest = useRef(0);
  const browserTargetRef = useRef<{ sessionId: string; spaceId: string } | null>(null);
  browserTargetRef.current = state.sessionId && selection.spaceId ? { sessionId: state.sessionId, spaceId: selection.spaceId } : null;
  const streamRegistry = useRef(new Set<TerminalStream>());
  const registerStream = useCallback((stream: TerminalStream, active: boolean) => { if (active) streamRegistry.current.add(stream); else streamRegistry.current.delete(stream); }, []);
  useEffect(() => () => { streamRegistry.current.forEach((stream) => stream.close()); streamRegistry.current.clear(); }, []);
  // Selecting a Space keeps the Library open; selecting a tab, pane or agent closes it to show that pane.
  const focusSpace = (space: Space) => {
    if (modalOpen) return;
    setAttachFocusSuppressed(false);
    setBrowserInputActive(false);
    const tabId = allTabs.find((tab) => tab.space_id === space.id && tab.focused)?.id ?? null;
    const paneId = snapshot?.panes.find((pane) => pane.space_id === space.id && pane.focused)?.id ?? null;
    if (narrowViewport) drawerFocusTarget.current = { spaceId: space.id, paneId };
    onFocus({ kind: "space", target_id: space.id }, { spaceId: space.id, tabId, paneId });
  };
  const focusTab = (tab: Tab) => {
    if (modalOpen) return;
    setLibraryOpen(false);
    setAttachFocusSuppressed(false);
    setBrowserInputActive(false);
    const paneId = snapshot?.panes.find((pane) => pane.tab_id === tab.id && pane.focused)?.id
      ?? snapshot?.layouts.find((candidate) => candidate.tab_id === tab.id)?.focused_pane_id ?? null;
    // A terminal pane attaches at Cockpit's grid before Herdr shows its tab, so
    // its resize is done by the time other Herdr clients see the tab.
    const paneRenderer = paneId ? renderers.panes[paneId] : undefined;
    const prepare = paneId && state.sync === "live" && snapshot?.focused_tab_id !== tab.id
      && !attachedTerminals.current.paneIds.has(paneId)
      && !(paneRenderer && (isGraphicalContext(paneRenderer) || isGraphicalReview(paneRenderer)))
      ? { paneId } : undefined;
    onFocus({ kind: "tab", target_id: tab.id }, { spaceId: tab.space_id, tabId: tab.id, paneId }, prepare);
  };
  const focusPane = (pane: Pane) => { if (!modalOpen) { setLibraryOpen(false); setAttachFocusSuppressed(false); setBrowserInputActive(false); onFocus({ kind: "pane", target_id: pane.id }, { spaceId: pane.space_id, tabId: pane.tab_id, paneId: pane.id }); } };
  const focusAgent = (agent: Agent) => {
    if (modalOpen) return;
    setLibraryOpen(false);
    setAttachFocusSuppressed(false);
    setBrowserInputActive(false);
    if (narrowViewport) drawerFocusTarget.current = { spaceId: agent.space_id, paneId: agent.pane_id };
    onFocus({ kind: "agent", target_id: agent.pane_id }, { spaceId: agent.space_id, tabId: agent.tab_id, paneId: agent.pane_id });
  };
  const beginRename = (target: ContextTarget | null): boolean => {
    if (!target || mutationBusy) return false;
    if (target.kind === "pane") setDialog({ kind: "rename", paneId: target.id });
    else setEditing(target);
    return true;
  };
  const closeSpace = (space: Space | undefined): boolean => Boolean(space && window.confirm(`Close Space "${space.label}" and all of its tabs and panes?`) && onMutate(`space:${space.id}`, { type: "space_close", space_id: space.id }));
  const closeTab = (tab: Tab | undefined): boolean => Boolean(tab && window.confirm(`Close tab "${tab.label}" and all of its panes?`) && onMutate(`tab:${tab.id}`, { type: "tab_close", tab_id: tab.id }));
  const closePane = (pane: Pane | undefined): boolean => {
    if (!pane) return false;
    const label = pane.title ?? `Pane ${panes.findIndex((candidate) => candidate.id === pane.id) + 1}`;
    return Boolean(window.confirm(`Close ${label}?`) && onMutate(`pane:${pane.id}`, { type: "pane_close", pane_id: pane.id }));
  };
  const browserAction = useCallback(async (spaceId: string, action: "open" | "show" | "close" | "reconnect", url?: string) => {
    const sessionId = state.sessionId;
    const key = sessionId ? `${sessionId}:${spaceId}` : null;
    if (!sessionId || !key || state.sync !== "live" || browserBusyRef.current || browserTargetRef.current?.sessionId !== sessionId || browserTargetRef.current.spaceId !== spaceId) return;
    const existing = browserPresentationRef.current[key];
    if ((action === "open" || action === "show") && existing?.associationOpen && !url) {
      setBrowserPresentation((current) => ({ ...current, [key]: { associationOpen: true, visible: true, presentation: "split" } }));
      setBrowserError(null);
      setCommandsOpen(false);
      return;
    }
    const token = ++browserRequest.current;
    browserBusyRef.current = true;
    setBrowserBusy(true);
    if (action === "close") {
      const registered = browserCloseGuardRef.current?.key === key ? browserCloseGuardRef.current : browserCloseGuardRef.outgoing;
      if (registered) {
        try { await (registered.settling ?? registered.recovery.guard()); 
        } catch (error) {
          browserBusyRef.current = false;
          setBrowserBusy(false);
          const described = describeError(error, "Browser close was refused because draft work is not durable yet");
          setBrowserError({ ...described, action });
          setCommandsOpen(true);
          return;
        }
      }
      if (browserRequest.current !== token || browserTargetRef.current?.sessionId !== sessionId || browserTargetRef.current.spaceId !== spaceId) { browserBusyRef.current = false; setBrowserBusy(false); return; }
      // A browser-only view hides the pane canvas. Restore it before awaiting
      // the remote close so closing the browser never leaves an empty workarea.
      setBrowserInputActive(false);
      setBrowserPresentation((currentState) => {
        const presentation = currentState[key];
        return presentation?.presentation === "browser_only"
          ? { ...currentState, [key]: { ...presentation, presentation: "split" } }
          : currentState;
      });
    }
    setBrowserError(null);
    const target: BrowserTarget = { session_id: sessionId, space_id: spaceId, pane_id: null, endpoint_path: null };
    const current = () => browserRequest.current === token && browserTargetRef.current?.sessionId === sessionId && browserTargetRef.current.spaceId === spaceId;
    try {
      const response = await client.browserAction({ target, action: action === "close" ? { kind: "close" } : { kind: "open", url: url ?? null } });
      if (!current()) return;
      if (response.association && (response.association.session_id !== sessionId || response.association.space_id !== spaceId)) throw new Error("Browser response belongs to another Space");
      if (action === "open" || action === "show" || action === "reconnect") {
        if (response.connection !== "open" && response.association?.connection !== "open") throw new Error(response.message || "Browser association did not open");
        setBrowserPresentation((currentState) => ({ ...currentState, [key]: { associationOpen: true, visible: true, presentation: "split" } }));
      } else {
        setBrowserPresentation((currentState) => ({ ...currentState, [key]: { associationOpen: false, visible: false, presentation: "split" } }));
        const focusedPane = panes.find((pane) => pane.id === selection.paneId && pane.space_id === spaceId)
          ?? snapshot?.panes.find((pane) => pane.space_id === spaceId && pane.focused);
        if (focusedPane) focusPane(focusedPane);
      }
      setBrowserError(null);
      setCommandsOpen(false);
    } catch (error) {
      if (!current()) return;
      setBrowserError({ ...describeError(error, "Browser action failed"), action });
      setCommandsOpen(true);
    } finally {
      if (current()) {
        browserBusyRef.current = false;
        setBrowserBusy(false);
      }
    }
  }, [client, state.sessionId, state.sync, panes, selection.paneId, snapshot, focusPane]);
  useEffect(() => {
    browserRequest.current += 1;
    setBrowserError(null);
    setBrowserBusy(false);
    browserBusyRef.current = false;
    return () => { browserRequest.current += 1; };
  // A transient Herdr sync loss does not change the selected association: keep
  // its in-flight open response, then render the view read-only until sync resumes.
  }, [selection.spaceId, state.sessionId, state.epoch]);
  const feedbackBusyRef = useRef(false);
  const feedbackRequest = useRef(0);
  const sendFeedback = useCallback(async (ids: string[], operationId: string, acknowledgeDuplicateRisk: boolean) => {
    const sessionId = state.sessionId; const spaceId = selection.spaceId;
    if (!sessionId || !spaceId || ids.length === 0 || state.sync !== "live") throw new Error("Annotation delivery is unavailable for this Space.");
    if (feedbackBusyRef.current) throw new Error("Annotation delivery is already in progress.");
    const token = ++feedbackRequest.current;
    feedbackBusyRef.current = true;
    try {
      const response = await client.sendBrowserFeedback({
        target: { session_id: sessionId, space_id: spaceId, pane_id: null, endpoint_path: null },
        ids, operation_id: operationId,
        acknowledge_duplicate_risk: acknowledgeDuplicateRisk,
      });
      if (feedbackRequest.current !== token) throw new Error("Annotation delivery was superseded.");
      if (response.operation_id !== operationId) throw new Error("Annotation delivery response did not match the retained operation.");
      return response;
    } finally {
      if (feedbackRequest.current === token) feedbackBusyRef.current = false;
    }
  }, [client, selection.spaceId, state.sessionId, state.sync]);
  const sendCapturedFeedback = useCallback((ids: string[], operationId: string, acknowledgeDuplicateRisk: boolean) => sendFeedback(ids, operationId, acknowledgeDuplicateRisk), [sendFeedback]);
  useEffect(() => {
    feedbackRequest.current += 1; feedbackBusyRef.current = false;
  }, [selection.spaceId, state.sessionId, state.sync]);
  const runCommand = useCallback((command: PrefixCommand) => {
    setBrowserInputActive(false);
    setPrefixHint(null);
    const space = byId(spaces, selection.spaceId);
    const tab = byId(tabs, selection.tabId);
    const pane = byId(panes, selection.paneId);
    const execute = () => {
      if (command === "new-space") onMutate("space:new", { type: "space_create", label: null, cwd: null }, true);
      if (command === "setup-space" && state.sync === "live") setSetupOpen(true);
      if (command === "rename-space" && space) beginRename({ kind: "space", id: space.id });
      if (command === "close-space") closeSpace(space);
      if (command === "new-tab" && selection.spaceId) onMutate("tab:new", { type: "tab_create", space_id: selection.spaceId, label: null }, true);
      if (command === "rename-tab" && tab) beginRename({ kind: "tab", id: tab.id });
      if (command === "close-tab") closeTab(tab);
      if (command === "rename-pane" && pane) beginRename({ kind: "pane", id: pane.id });
      if (command === "split-right" && pane) onMutate(`pane:${pane.id}`, { type: "pane_split", pane_id: pane.id, direction: "right", ratio: null }, true);
      if (command === "split-down" && pane) onMutate(`pane:${pane.id}`, { type: "pane_split", pane_id: pane.id, direction: "down", ratio: null }, true);
      if (command === "close-pane") closePane(pane);
      if (command === "zoom-pane" && pane) onMutate(`pane:${pane.id}`, { type: "pane_zoom", pane_id: pane.id, mode: "toggle" });
      if (command === "previous-tab" && tab) { const index = tabs.indexOf(tab); if (index > 0) focusTab(tabs[index - 1]); }
      if (command === "next-tab" && tab) { const index = tabs.indexOf(tab); if (index >= 0 && index < tabs.length - 1) focusTab(tabs[index + 1]); }
      if (command.startsWith("select-tab-")) { const target = tabs[Number(command.slice("select-tab-".length)) - 1]; if (target) focusTab(target); }
      if (command === "previous-pane" && pane) { const index = panes.indexOf(pane); focusPane(panes[(index - 1 + panes.length) % panes.length]); }
      if (command === "next-pane" && pane) { const index = panes.indexOf(pane); focusPane(panes[(index + 1) % panes.length]); }
      if (["focus-left", "focus-right", "focus-up", "focus-down"].includes(command)) {
        const direction = command.slice("focus-".length) as PaneFocusDirection;
        const target = byId(panes, paneIdInDirection(layout, selection.paneId, direction));
        if (target) focusPane(target);
      }
      if (["swap-left", "swap-right", "swap-up", "swap-down"].includes(command) && pane) {
        const target = byId(panes, paneIdInDirection(layout, pane.id, command.slice("swap-".length) as PaneFocusDirection));
        if (target) onMutate(`pane:${pane.id}`, { type: "pane_swap", source_pane_id: pane.id, target_pane_id: target.id });
      }
      if (command === "open-file-picker") dispatchFileNavigation("open-picker");
      if (command === "resize" && !mutationBusy) document.querySelector<HTMLElement>(".resize-handle")?.focus();
      if (command === "switch-session") openSessionChooser();
      if (command === "toggle-library") { if (libraryOpen) closeLibraryToOrigin(); else openLibrary(); }
      if (command === "toggle-browser" && selection.spaceId) void browserAction(selection.spaceId, selectedBrowserPresentation?.associationOpen ? "close" : "open");
      if (command === "toggle-sidebar") {
        const visible = narrowViewport ? drawerOpen : !sidebarCollapsed;
        if (narrowViewport) { if (drawerOpen) closeDrawer(); else openDrawer(); } else toggleSidebarCollapsed();
        // A collapsed sidebar cannot keep focus; the selected tab is a safe target that sends nothing to Herdr.
        if (visible && !narrowViewport && document.activeElement?.closest("#cockpit-sidebar")) {
          window.setTimeout(() => document.querySelector<HTMLElement>('.tab-button[aria-selected="true"], .drawer-toggle')?.focus({ preventScroll: true }), 0);
        }
      }
      if (command === "focus-spaces" || command === "focus-agents") {
        const list = command === "focus-spaces" ? "spaces" : "agents";
        const visible = narrowViewport ? drawerOpen : !sidebarCollapsed;
        if (!visible) { if (narrowViewport) openDrawer(); else toggleSidebarCollapsed(); }
        // The sidebar may still be mounting (and the drawer focuses its close button first): retry until a row holds focus.
        const attempt = (remaining: number) => {
          focusSidebarList(list);
          const focused = document.activeElement;
          if (remaining > 0 && !(focused?.closest("#cockpit-sidebar") && !focused.matches(".sidebar-close"))) window.setTimeout(() => attempt(remaining - 1), 30);
        };
        window.setTimeout(() => attempt(20), visible ? 0 : 60);
      }
    };
    if (command === "help") { setCommandsOpen(true); return; }
    if (mutationBusy && !COMMANDS_ALLOWED_WHILE_BUSY.includes(command)) return;
    if (shortcutEntry(command).paneScoped && libraryOpen) {
      // The command acts on a pane the Library covers: show the pane first, then run, so a confirmation names a visible target.
      libraryOrigin.current = null;
      setAttachFocusSuppressed(false);
      flushSync(() => setLibraryOpen(false));
      requestAnimationFrame(() => window.setTimeout(execute, 0));
      return;
    }
    execute();
  }, [spaces, tabs, panes, layout, snapshot, selection.spaceId, selection.tabId, selection.paneId, mutationBusy, modalOpen, libraryOpen, narrowViewport, drawerOpen, sidebarCollapsed, state.sync, browserAction, selectedBrowserPresentation?.associationOpen, closeLibraryToOrigin, openLibrary, openSessionChooser, closeDrawer, openDrawer, toggleSidebarCollapsed]);
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      routeWorkbenchKeydown(event, { modalOpen, prefixActive, runCommand, setPrefixActive, setCommandsOpen, onUnboundPrefixKey: setPrefixHint });
    };
    window.addEventListener("keydown", keydown, true);
    return () => window.removeEventListener("keydown", keydown, true);
  }, [prefixActive, runCommand, modalOpen]);
  const openContext = (event: ContextAnchor, target: ContextTarget) => { event.preventDefault(); event.stopPropagation(); if (!mutationBusy && !modalOpen) setMenu({ target, x: event.clientX, y: event.clientY }); };
  const dismissMenu = useCallback(() => setMenu(null), []);
  const menuAction = (action: () => boolean | void) => { if (action() !== false) dismissMenu(); };
  const renderMenu = () => {
    if (!menu) return null;
    const disabled = mutationBusy;
    if (menu.target.kind === "space") {
      const space = spaces.find((candidate) => candidate.id === menu.target.id);
      if (!space) return null;
      const browserReason = space.id === selection.spaceId ? undefined : "Select this Space first";
      const spaceBrowser = state.sessionId ? browserPresentation[`${state.sessionId}:${space.id}`] : undefined;
      return <ContextMenu menu={menu} onDismiss={dismissMenu}>
        <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}><UiIcon name="edit" />Rename</button>
        <button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeSpace(space))}><UiIcon name="close" />Close</button>
        <button role="menuitem" type="button" disabled={disabled || browserBusy || Boolean(browserReason)} title={browserReason} onClick={() => menuAction(() => { void browserAction(space.id, "open"); })}><UiIcon name="grid" />Open browser</button>
        <button role="menuitem" type="button" disabled={disabled || browserBusy || !spaceBrowser?.associationOpen || spaceBrowser.visible || Boolean(browserReason)} title={browserReason} onClick={() => menuAction(() => { void browserAction(space.id, "show"); })}><UiIcon name="grid" />Show browser</button>
        <button role="menuitem" type="button" disabled={disabled || !spaceBrowser?.visible || Boolean(browserReason)} title={browserReason} onClick={() => menuAction(hideBrowser)}><UiIcon name="grid" />Hide browser</button>
        <button role="menuitem" type="button" disabled={disabled || browserBusy || !spaceBrowser?.associationOpen || Boolean(browserReason)} className="destructive" title={browserReason} onClick={() => menuAction(() => { void browserAction(space.id, "close"); })}><UiIcon name="close" />Close browser</button>
        <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => setTeardownSpaceId(space.id))}><UiIcon name="trash" />Review task cleanup…</button>
      </ContextMenu>;
    }
    if (menu.target.kind === "tab") {
      const tab = allTabs.find((candidate) => candidate.id === menu.target.id);
      if (!tab) return null;
      return <ContextMenu menu={menu} onDismiss={dismissMenu}><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}><UiIcon name="edit" />Rename</button><button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeTab(tab))}><UiIcon name="close" />Close</button></ContextMenu>;
    }
    const pane = snapshot?.panes.find((candidate) => candidate.id === menu.target.id);
    if (!pane) return null;
    const renderer = renderers.panes[pane.id];
    return <ContextMenu menu={menu} onDismiss={dismissMenu}>
      <p className="context-menu-heading" role="presentation">Selected pane · {isGraphicalReview(renderer) ? "Review" : isGraphicalContext(renderer) ? "Files" : "Terminal"}</p>
      <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => onMutate(`pane:${pane.id}`, { type: "pane_zoom", pane_id: pane.id, mode: "toggle" }))}><UiIcon name="expand" />Expand / restore pane</button>
      {isGraphicalContext(renderer) || isGraphicalReview(renderer) ? <button role="menuitem" type="button" onClick={() => menuAction(() => { document.querySelector<HTMLElement>(".pane-view.is-selected .context-document, .pane-view.is-selected .review-diff")?.focus({ preventScroll: true }); dispatchFileNavigation("open-picker"); })}><UiIcon name="search" />Go to file…</button> : null}
      <p className="context-menu-heading" role="presentation">Open view</p>
      {rendererActionDefinitions.filter(action => action.direction === "right").map(({ id, label, direction, kind }) => {
        const capability = kind === "review" ? renderer?.presentation.can_open_review : kind === "files" ? renderer?.presentation.can_open_files : renderer?.presentation.can_open_context;
        return <button key={id} role="menuitem" type="button" disabled={disabled || !capability} title={renderer?.presentation.reason ?? label} onClick={() => menuAction(() => { void renderers.open(pane.id, direction, kind === "context" ? undefined : kind); })}><UiIcon name="file" />{kind === "review" ? "Review" : kind === "files" ? "Files" : "Context"}</button>;
      })}
      <details className="context-menu-advanced"><summary>Advanced</summary>
        <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}><UiIcon name="edit" />Rename pane</button>
        {(["right", "down"] as const).map(direction => <button key={direction} role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => onMutate(`pane:${pane.id}`, { type: "pane_split", pane_id: pane.id, direction, ratio: null }, true))}><UiIcon name="sidebar" />Split {direction}</button>)}
        {rendererActionDefinitions.filter(action => action.direction === "down").map(({ id, label, direction, kind }) => <button key={id} role="menuitem" type="button" disabled={disabled || !(kind === "review" ? renderer?.presentation.can_open_review : kind === "files" ? renderer?.presentation.can_open_files : renderer?.presentation.can_open_context)} onClick={() => menuAction(() => { void renderers.open(pane.id, direction, kind === "context" ? undefined : kind); })}><UiIcon name="file" />{label}</button>)}
        <button role="menuitem" type="button" disabled={!renderer?.presentation.renderer} onClick={() => menuAction(() => renderers.choose(pane.id, (isGraphicalContext(renderer) || isGraphicalReview(renderer)) ? "terminal" : renderer?.presentation.renderer ?? "context"))}><UiIcon name="terminal" />{isGraphicalContext(renderer) || isGraphicalReview(renderer) ? "Show terminal view" : "Render document"}</button>
        <button role="menuitem" type="button" onClick={() => menuAction(renderers.refresh)}><UiIcon name="refresh" />Refresh renderer detection</button>
        <button role="menuitem" type="button" disabled={disabled || panes.length < 2} onClick={() => menuAction(() => { setDialog({ kind: "swap", paneId: pane.id }); })}><UiIcon name="right" />Swap…</button>
        <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => { setDialog({ kind: "move", paneId: pane.id }); })}><UiIcon name="right" />Move…</button>
      </details>
      <div className="context-menu-separator" role="presentation" />
      <button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closePane(pane))}><UiIcon name="close" />Close pane</button>
    </ContextMenu>;
  };
  const selectedRenderer = selection.paneId ? renderers.panes[selection.paneId] : undefined;
  const selectedPane = byId(panes, selection.paneId);
  const commandActionRows: CommandAction[] = [
    ...SHORTCUTS.filter((entry) => entry.prefix && entry.palette !== false).map((entry): CommandAction => {
      const command = entry.id as PrefixCommand;
      const reason = entry.needs === "space" && !selectedSpace ? "Select a Space first"
        : entry.needs === "tab" && !selectedTab ? "Select a tab first"
        : entry.needs === "pane" && !selectedPane ? "Select a pane first"
        : command === "setup-space" && state.sync !== "live" ? "Herdr is not live"
        : undefined;
      return {
        id: `prefix:${command}`, label: command === "toggle-library" && libraryOpen ? "Close Library" : entry.label, shortcut: formatShortcut(command), group: entry.group,
        disabled: reason !== undefined, reason, run: () => runCommand(command),
      };
    }),
    { id: "recovery:cleanup", label: "Recover task cleanup…", group: "Navigate", run: () => setRecoveryOpen(true) },
    { id: "browser:open", label: "Open browser for Space", group: "Browser", disabled: !selection.spaceId || browserBusy || state.sync !== "live", reason: !selection.spaceId ? "Select a Space first" : state.sync !== "live" ? "Herdr is not live" : undefined, run: () => { if (selection.spaceId) void browserAction(selection.spaceId, "open"); } },
    { id: "browser:show", label: "Show browser view", group: "Browser", disabled: !selection.spaceId || browserBusy || !selectedBrowserPresentation?.associationOpen || Boolean(selectedBrowserPresentation.visible) || state.sync !== "live", reason: !selection.spaceId ? "Select a Space first" : !selectedBrowserPresentation?.associationOpen ? "No browser association is open" : state.sync !== "live" ? "Herdr is not live" : undefined, run: () => { if (selection.spaceId) void browserAction(selection.spaceId, "show"); } },
    { id: "browser:hide", label: "Hide browser view", group: "Browser", disabled: !browserVisible, reason: !browserVisible ? "Open the browser view first" : undefined, run: hideBrowser },
    { id: "browser:close", label: "Close browser for Space", group: "Browser", disabled: !selection.spaceId || browserBusy || !selectedBrowserPresentation?.associationOpen || state.sync !== "live", reason: !selection.spaceId ? "Select a Space first" : !selectedBrowserPresentation?.associationOpen ? "No browser association is open" : state.sync !== "live" ? "Herdr is not live" : undefined, run: () => { if (selection.spaceId) void browserAction(selection.spaceId, "close"); } },
    { id: "library:add", label: "Add to Library…", group: "Library", run: () => setLibraryAddOpen(true) },
    { id: "library:refresh", label: "Refresh Library", group: "Library", run: () => openLibrary({ kind: "refresh" }) },
    { id: "library:tokens", label: "Provider tokens…", group: "Library", run: () => openLibrary({ kind: "tokens" }) },
    ...rendererActionDefinitions.map(({ id, label, direction, kind }) => {
      const capability = kind === "review" ? selectedRenderer?.presentation.can_open_review : kind === "files" ? selectedRenderer?.presentation.can_open_files : selectedRenderer?.presentation.can_open_context;
      const fallback = kind === "context" ? "Context requires a configured companion directory" : "Select a pane with a configured repository";
      const detail = selectedRenderer?.presentation.reason;
      const reason = detail ? rendererReasonFor(kind, detail) ?? fallback : fallback;
      return { id: `renderer:${id}`, label, group: "Pane" as const, disabled: mutationBusy || !capability, reason, reasonDetail: detail, run: () => { if (selection.paneId) void renderers.open(selection.paneId, direction, kind === "context" ? undefined : kind); } };
    }),
  ];
  const browserToggleShortcut = formatShortcut("toggle-browser");
  const commandActions: CommandAction[] = commandActionRows.map((action) => action.id === "browser:open" || action.id === "browser:close" ? { ...action, shortcut: browserToggleShortcut } : action);
  const commandStatus = <>{browserBusy ? <p role="status">Working on the Space browser…</p> : null}{browserError ? <p role="alert">{browserError.message}</p> : null}</>;
  const renderPaneLayer = (projection: PaneCanvasProjection, incoming: boolean, painted: boolean) => projection.visiblePaneIds.map((paneId, index) => {
    const pane = projection.panes.find((candidate) => candidate.id === paneId);
    if (!pane) return null;
    const rectangle = projectedPaneRect(projection.layout, pane.id);
    const area = projection.layout?.area;
    const bounds = rectangle && area && area.width > 0 && area.height > 0 ? { left: `${(rectangle.x - area.x) / area.width * 100}%`, top: `${(rectangle.y - area.y) / area.height * 100}%`, width: `${rectangle.width / area.width * 100}%`, height: `${rectangle.height / area.height * 100}%` } : { left: `${index / projection.visiblePaneIds.length * 100}%`, top: "0%", width: `${100 / projection.visiblePaneIds.length}%`, height: "100%" };
    const style = { ...bounds, visibility: painted ? "visible" as const : "hidden" as const, pointerEvents: painted && incoming ? "auto" as const : "none" as const };
    const renderer = renderers.panes[pane.id];
    const paneRendererKey = renderer && (isGraphicalContext(renderer) || isGraphicalReview(renderer)) ? `${renderer.presentation.renderer}:${renderer.choice ?? ""}` : "terminal";
    const paintedSelected = painted && pane.id === projection.selectedPaneId;
    const controlPendingForPane = incoming && state.focusPending !== null
      && (state.focusPending.kind === "pane" || state.focusPending.kind === "agent"
        ? state.focusPending.target_id === pane.id
        : state.focusPending.kind === "tab"
          ? state.focusPending.target_id === pane.tab_id
          : state.focusPending.kind === "space" && state.focusPending.target_id === pane.space_id);
    const currentPaneStatus = incoming && pane.id === selection.paneId && state.focusPending !== null && !controlPendingForPane;
    const paneFocusError = incoming && state.focusError && (pane.id === controlPaneId || pane.id === selection.paneId) ? state.focusError : null;
    const sessionAttachable = state.sync === "live" || (sessionResyncing && attachedTerminals.current.paneIds.has(pane.id));
    // A tab switch attaches the target's terminals while Herdr still shows the old
    // tab, and the old tab's terminals stay attached until the swap is painted, so
    // the old pane's size only reverts after Herdr's focus has moved on.
    const preparingTab = state.focusPending?.kind === "tab" && state.focusPending.target_id === pane.tab_id;
    const deferTerminal = incoming
      ? !sessionAttachable || (snapshot?.focused_tab_id !== pane.tab_id && !preparingTab)
        || (state.focusPending?.kind === "tab" && !preparingTab) || state.focusPending?.kind === "space"
      : !(sessionAttachable && attachedTerminals.current.paneIds.has(pane.id));
    if (!deferTerminal) nextAttachedTerminals.add(pane.id);
    const paneSpace = state.sessionId ? spaces.find((space) => space.id === pane.space_id) : undefined;
    return <PaneView key={`${pane.id}:${paneRendererKey}`} pane={pane} space={state.sessionId && paneSpace ? { target: { session_id: state.sessionId, space_id: paneSpace.id }, label: paneSpace.label, live: state.sync === "live" } : null} label={pane.title ?? `Pane ${projection.panes.indexOf(pane) + 1}`} solo={projection.panes.length === 1} selected={incoming && pane.id === selection.paneId} paintedSelected={paintedSelected} retained={!incoming} busy={mutationBusy} controlAllowed={!browserInputActive && incoming && state.sync === "live" && pane.id === controlPaneId && pane.id === snapshot?.focused_pane_id && !state.focusPending && !state.focusError} controlPending={Boolean(controlPendingForPane || currentPaneStatus)} focusError={paneFocusError} focusEpoch={state.epoch} focusToken={state.focusToken} terminalMouseInput={terminalMouseInput} deferTerminal={deferTerminal} onPrepared={() => onPanePrepared(pane.id)} focusOnAttach={!attachFocusSuppressed} onRequestControl={() => { if (incoming && !modalOpen && state.sync === "live") { setAttachFocusSuppressed(false); setBrowserInputActive(false); if (pane.id !== snapshot?.focused_pane_id || (state.focusError && pane.id === selection.paneId)) focusPane(pane); else onRequestControl(pane.id); } }} onSelect={() => { if (incoming) focusPane(pane); }} onContext={openContext} onRetryFocus={onRetry} request={{ session_id: state.sessionId!, pane_id: pane.id }} client={client} registerStream={registerStream} onResync={onReconnect} mutate={onMutate} style={style} renderer={renderer} rendererReady={renderers.inspectedPaneIds.includes(pane.id)} onRendererViewChange={(bindingId, value) => renderers.updateView(pane.id, bindingId, value)} onTerminalView={() => renderers.choose(pane.id, "terminal")} onReady={incoming ? () => markPaneReady(pane.id) : () => undefined} onRefreshRenderer={renderers.refresh} />;
  });
  const workbenchStyle: CSSProperties & { "--sidebar-width": string; "--browser-ratio": string } = {
    "--sidebar-width": `${sidebarWidth}px`,
    "--browser-ratio": `${browserSplitRatio * 100}%`,
  };
  const sidebarClass = "sidebar";
  const projections = paneProjection.key === null
    ? []
    : paneCanvasReady || outgoingProjection === null
      ? [paneProjection]
      : [outgoingProjection, paneProjection];
  const paintedProjectionKey = paneCanvasReady ? paneProjection.key : outgoingProjection?.key ?? paneProjection.key;
  const paneInstances = libraryOpen ? [] : projections.flatMap((projection) => renderPaneLayer(projection, projection.key === paneProjection.key, projection.key === paintedProjectionKey));
  // The inline browser stays associated but hidden while the Library covers the work area.
  const browserPresented = browserVisible && !libraryOpen;
  // Selection chrome follows Herdr's acknowledgement: while a focus request is in flight the sidebar keeps the confirmed row and marks the target as pending.
  const pendingSpaceId = state.focusPending?.kind === "space" ? state.focusPending.target_id : null;
  const pendingPaneId = state.focusPending?.kind === "pane" || state.focusPending?.kind === "agent" ? state.focusPending.target_id : null;
  const sidebarSelectedSpaceId = state.focusPending ? snapshot?.focused_space_id ?? null : selection.spaceId;
  const sidebarSelectedPaneId = state.focusPending ? snapshot?.focused_pane_id ?? null : selection.paneId;
  return <div className={`workbench${sidebarCollapsed ? " sidebar-collapsed" : ""}${narrowViewport && drawerOpen ? " drawer-open" : ""}`} style={workbenchStyle}>
    {narrowViewport && drawerOpen ? <button type="button" className="drawer-scrim" aria-label="Close sidebar" onClick={() => closeDrawer()} /> : null}
    <aside id="cockpit-sidebar" className={sidebarClass} aria-label="Spaces and agents" role={narrowViewport && drawerOpen ? "dialog" : undefined} aria-modal={narrowViewport && drawerOpen ? "true" : undefined} aria-hidden={narrowViewport && !drawerOpen ? "true" : undefined} hidden={narrowViewport ? !drawerOpen : sidebarCollapsed}>
      <Sidebar session={sidebarSession} sync={state.sync} narrow={narrowViewport} onSession={openSessionChooser} onClose={() => closeDrawer()} closeRef={sidebarCloseRef} hasSession={state.sessionId !== null} hasSnapshot={snapshot !== null}
        spaces={{ spaces, gitStatus: spaceGit, selectedSpaceId: sidebarSelectedSpaceId, pendingSpaceId, editingId: editing?.kind === "space" ? editing.id : null, busy: mutationBusy, notes: spaceNotesFromFailures(Object.values(mutations.errors)), onEdit: (id) => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "space", id } : null); }, onSelect: focusSpace, onContext: openContext, onSetup: () => setSetupOpen(true), setupEnabled: state.sync === "live" && !modalOpen, mutate: onMutate }}
        agents={{ agents: snapshot?.agents ?? [], spaces, tabs: allTabs, selectedPaneId: sidebarSelectedPaneId, pendingPaneId, onSelect: focusAgent }} />
    </aside>
    {!narrowViewport && !sidebarCollapsed ? <div className="sidebar-resizer" role="separator" tabIndex={sidebarCollapsed ? -1 : 0} aria-label="Resize sidebar" aria-orientation="vertical" aria-valuemin={SIDEBAR_MIN_WIDTH} aria-valuemax={SIDEBAR_MAX_WIDTH} aria-valuenow={sidebarWidth}
      onKeyDown={(event) => { if (sidebarCollapsed) return; if (event.key === "Home") { event.preventDefault(); updateSidebarWidth(SIDEBAR_DEFAULT_WIDTH); } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") { event.preventDefault(); updateSidebarWidth(sidebarWidth + (event.key === "ArrowLeft" ? -8 : 8)); } }}
      onPointerDown={(event) => { if (sidebarCollapsed || event.button !== 0) return; event.preventDefault(); const start = event.clientX; const width = sidebarWidth; const move = (next: PointerEvent) => updateSidebarWidth(width + next.clientX - start); const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); }; window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop); }} /> : null}
    <main className="main-workarea">
      {!selection.spaceId ? <button type="button" className="drawer-toggle" aria-expanded={drawerOpen} aria-controls="cockpit-sidebar" aria-label="Open sidebar" onClick={narrowViewport ? openDrawer : toggleSidebarCollapsed}><UiIcon name="sidebar" /> <span>Sidebar</span></button> : null}
      {selection.spaceId ? <TabStrip sidebarOpen={narrowViewport ? drawerOpen : !sidebarCollapsed} onToggleSidebar={narrowViewport ? (drawerOpen ? () => closeDrawer() : openDrawer) : toggleSidebarCollapsed} tabs={tabs} selectedTabId={selection.tabId} editingId={editing?.kind === "tab" ? editing.id : null} busy={mutationBusy} browserOpen={Boolean(selectedBrowserPresentation?.associationOpen)} libraryOpen={libraryOpen} onEdit={(id) => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "tab", id } : null); }} onSelect={focusTab} onContext={openContext} onCreate={() => { if (selection.spaceId) onMutate("tab:new", { type: "tab_create", space_id: selection.spaceId, label: null }, true); }} onBrowserToggle={() => {
        if (!selection.spaceId) return;
        if (libraryOpen) { closeLibrary(); void browserAction(selection.spaceId, "open"); return; }
        void browserAction(selection.spaceId, selectedBrowserPresentation?.associationOpen ? "close" : "open");
      }} onLibraryToggle={() => { if (libraryOpen) closeLibrary(); else openLibrary(); }} onCommands={() => setCommandsOpen(true)} mutate={onMutate} /> : null}
      <div className="workarea-content">
        {libraryOpen ? <LibraryView client={client} onClose={closeLibrary} command={libraryCommand} space={librarySpace} /> : <div className="pane-canvas" style={{ visibility: !browserVisible || paneCanvasVisible ? "visible" : "hidden", display: browserVisible && browserOnly ? "none" : undefined }} onPointerDownCapture={() => setAttachFocusSuppressed(false)}>
          {paneProjection.panes.length === 0 ? <div className="empty-main"><strong>No panes</strong><span>Create a tab or select another space.</span></div> : paneInstances}
          {outgoingProjection || mutationBusy ? null : <ResizeHandles layout={layout} mutate={onMutate} />}
        </div>}
        {browserPresented && !browserOnly ? <div className={`browser-splitter${narrowViewport ? " is-horizontal" : ""}`} role="separator" tabIndex={0} aria-label="Resize browser region" aria-orientation={narrowViewport ? "horizontal" : "vertical"} aria-valuemin={BROWSER_SPLIT_MIN_RATIO * 100} aria-valuemax={BROWSER_SPLIT_MAX_RATIO * 100} aria-valuenow={Math.round(browserSplitRatio * 100)} aria-valuetext={`${Math.round(browserSplitRatio * 100)}% browser region`} onKeyDown={browserSplitterKeyDown} onPointerDown={browserSplitterPointerDown} onDoubleClick={() => updateBrowserSplitRatio(BROWSER_SPLIT_DEFAULT_RATIO)} /> : null}
        {browserBlockedByOutgoing && browserHandoff.outgoing ? <div className="browser-recovery-strip" role="status"><details><summary>Review retained browser work</summary><p>{browserHandoff.outgoing.recovery.describe()}</p><p>Discard only clears the local retry intent; an in-flight or unknown remote write is not undone.</p></details><button type="button" onClick={retryBrowserHandoff}>Retry retained work</button><button type="button" onClick={discardBrowserHandoff}>Discard retry intent</button></div> : null}
        {browserAssociationOpen && browserTarget ? <div ref={browserRegionRef} className={`browser-region${browserSyncUnavailable ? " is-session-stale" : ""}`} aria-label="Inline browser region" style={!browserPresented ? { display: "none" } : browserOnly ? { flex: "1 1 0", minHeight: 0 } : undefined}>
          {browserPresented && browserSyncUnavailable ? <div className="browser-recovery-strip" role="status"><span>{browserSyncMessage}</span><button type="button" onClick={onReconnect} aria-label="Resync Herdr session for browser view">{state.sync === "disconnected" ? "Reconnect" : "Resync"}</button></div> : null}
          <BrowserPane key={browserKey ?? "browser-none"} client={client} target={browserTarget} viewport={browserViewport} visible={browserPresented} presentation={browserOnly ? "browser_only" : "split"} clientId={browserClientId} inputActive={browserInputActive && !browserSyncUnavailable && !modalOpen} liveInputEnabled={browserPresented && !browserKeyChanged && !browserSyncUnavailable && state.sync === "live" && !modalOpen} onInteractionFocus={() => { if (!browserSyncUnavailable && !modalOpen) setBrowserInputActive(true); }} onReconnect={() => selection.spaceId ? browserAction(selection.spaceId, "reconnect") : undefined} onCloseBrowser={() => selection.spaceId ? browserAction(selection.spaceId, "close") : undefined} onFeedback={sendCapturedFeedback} onExpand={enterBrowserOnly} onBackToTerminals={browserOnly ? backToTerminals : undefined} registerCloseGuard={registerBrowserCloseGuard} />
        </div> : null}
      </div>
    </main>
    {renderMenu()}
    {dialog ? <PaneDialogOverlay dialog={dialog} panes={panes} tabs={allTabs} spaces={spaces} busy={mutationBusy} onDismiss={() => setDialog(null)} mutate={onMutate} /> : null}
    {commandsOpen ? <CommandOverlay actions={commandActions.map((action) => ({ ...action, run: () => { setCommandsOpen(false); action.run(); } }))} statusContent={commandStatus} onSwitchSession={() => { setCommandsOpen(false); void onRefreshSessions().catch(() => undefined).finally(() => setSessionChooserOpen(true)); }} onDismiss={() => setCommandsOpen(false)} /> : null}
    {libraryAddOpen ? <AddContextDialog client={client} onClose={() => setLibraryAddOpen(false)} onOpenItem={(itemId) => openLibrary({ kind: "open", itemId })} space={librarySpace} /> : null}
    {sessionChooserOpen ? <SessionDialogOverlay sessions={sessions} currentSessionId={state.sessionId} onRefresh={onRefreshSessions} onSession={onSession} onDismiss={() => setSessionChooserOpen(false)} /> : null}
    {state.sessionId ? <SetupDialog client={client} sessionId={state.sessionId} open={setupOpen} selectedParent={setupParent} parentSpaceId={selection.spaceId} onClose={() => setSetupOpen(false)} onCompleted={onReconnect} /> : null}
    {state.sessionId ? <TeardownRecoveryPanel client={client} sessionId={state.sessionId} open={recoveryOpen} onClose={() => setRecoveryOpen(false)} /> : null}
    {state.sessionId && teardownSpaceId ? <TeardownDialog client={client} sessionId={state.sessionId} workspaceId={teardownSpaceId} open onClose={() => setTeardownSpaceId(null)} onCompleted={onReconnect} /> : null}
    {prefixActive ? <div className="prefix-indicator" role="status"><span>Ctrl+B · {armedPrefixHint()}</span></div> : prefixHint ? <div className="prefix-indicator is-notice" role="status">{prefixHint}</div> : null}
    <RecoveryPanel state={state} mutations={mutations} onReconnect={onReconnect} onRetryMutation={onRetryMutation} />
  </div>;
}
export function App({ client }: { client: CockpitClient }) {
  const browserHandoffRef = useRef<BrowserGuardHandoff>({ current: null, outgoing: null, incoming: null });
  const [, setBrowserHandoffRevision] = useState(0);
  browserHandoffRef.current.notify = () => setBrowserHandoffRevision((revision) => revision + 1);
  const [status, setStatus] = useState<CockpitStatus | null>(null);
  const [statusError, setStatusError] = useState<StatusError | null>(null);
  const [statusAttempt, setStatusAttempt] = useState(0);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [sessionsAttempt, setSessionsAttempt] = useState(0);
  const [sessionsLoaded, setSessionsLoaded] = useState(false);
  const [sessionsError, setSessionsError] = useState<StatusError | null>(null);
  const [state, dispatch] = useReducer(sessionReducer, initialSessionState);
  const [selection, setSelection] = useState<Selection>({ spaceId: null, tabId: null, paneId: null });
  const [controlPaneId, setControlPaneId] = useState<string | null>(null);
  useEffect(() => {
    const outgoing = browserHandoffRef.current.outgoing;
    if (!outgoing) return;
    const settling = outgoing.settling ?? outgoing.recovery.guard().then(() => {
      if (browserHandoffRef.current.outgoing === outgoing) {
        browserHandoffRef.current.outgoing = null;
        const incoming = browserHandoffRef.current.incoming;
        browserHandoffRef.current.incoming = null;
        if (incoming) browserHandoffRef.current.current = incoming;
        browserHandoffRef.current.notify?.();
      }
    });
    outgoing.settling = settling;
    void settling.catch(() => {
      if (browserHandoffRef.current.outgoing === outgoing) {
        outgoing.settling = undefined;
        browserHandoffRef.current.notify?.();
      }
    });
  }, [selection.spaceId, state.epoch, state.sessionId]);
  const sessionStream = useRef<{ close(): void } | null>(null);
  const controlInitializedEpoch = useRef<number | null>(null);
  const [resyncAttempt, setResyncAttempt] = useState(0);
  const recoveryResyncRef = useRef(false);
  const sessionObservation = useRef(0);
  const sessionListRequest = useRef(0);
  const stateRef = useRef(state);
  stateRef.current = state;
  const autoResyncTimer = useRef<number | null>(null);
  const autoResyncAttempts = useRef(0);
  const healthyLiveTimer = useRef<number | null>(null);
  const mountedRef = useRef(true);
  const clearRecoveryTimers = useCallback(() => {
    if (autoResyncTimer.current !== null) window.clearTimeout(autoResyncTimer.current);
    if (healthyLiveTimer.current !== null) window.clearTimeout(healthyLiveTimer.current);
    autoResyncTimer.current = null;
    healthyLiveTimer.current = null;
  }, []);
  const requestResync = useCallback(() => {
    recoveryResyncRef.current = true;
    setResyncAttempt((value) => value + 1);
  }, []);
  const { focus, panePrepared, reconcile: reconcileFocus, reset: resetFocus, retryFocus, tokenRef: focusTokenRef } = useFocusCoordinator({
    client,
    stateRef,
    mountedRef,
    dispatch,
    describeError,
    onTimeout: requestResync,
  });
  const focusAndSelect = useCallback((request: FocusRequest, location: Selection, prepare?: { paneId: string }) => {
    setSelection(location);
    setControlPaneId(location.paneId);
    focus(request, location, prepare);
  }, [focus]);
  const { consumeFocusedPane, mutate, reset: resetMutations, retry: retryMutation, state: mutations, tokenRef: mutationTokenRef } = useMutationCoordinator({
    client,
    stateRef,
    mountedRef,
    sessionObservationRef: sessionObservation,
    focusTokenRef,
    dispatchSession: dispatch,
    describeError,
    mutationSnapshot: authoritativeMutationSnapshot,
    onResync: requestResync,
  });
  const resetSessionRuntime = useCallback(() => {
    sessionObservation.current += 1;
    sessionStream.current?.close();
    sessionStream.current = null;
    resetFocus();
    clearRecoveryTimers();
    autoResyncAttempts.current = 0;
    recoveryResyncRef.current = false;
    resetMutations();
    setSelection({ spaceId: null, tabId: null, paneId: null });
    setControlPaneId(null);
    controlInitializedEpoch.current = null;
  }, [clearRecoveryTimers, resetFocus, resetMutations]);
  const switchSession = useCallback((id: string) => {
    resetSessionRuntime();
    dispatch({ type: "switch", sessionId: id });
  }, [resetSessionRuntime]);
  const refreshSessions = useCallback(async () => {
    const request = ++sessionListRequest.current;
    setSessionsError(null);
    try {
      const response = await client.sessions();
      if (!mountedRef.current || sessionListRequest.current !== request) return;
      setSessions(response.sessions);
      setSessionsLoaded(true);
    } catch (error: unknown) {
      if (!mountedRef.current || sessionListRequest.current !== request) return;
      setSessionsError(describeError(error, "Could not list Herdr sessions"));
      setSessionsLoaded(true);
    }
  }, [client]);
  useEffect(() => { mountedRef.current = true; return () => { mountedRef.current = false; clearRecoveryTimers(); resetFocus(); }; }, [clearRecoveryTimers, resetFocus]);
  useEffect(() => { let active = true; setStatus(null); setStatusError(null); void client.status().then((next) => { if (active) setStatus(next); }, (error: unknown) => { if (active) setStatusError(describeError(error, "Could not read Cockpit status")); }); return () => { active = false; }; }, [client, statusAttempt]);
  const compatible = status?.herdr.status === "compatible";
  const sessionAvailable = sessionsLoaded && sessions.some((session) => session.id === state.sessionId);
  useEffect(() => {
    if (!compatible) return;
    void refreshSessions();
    return () => { sessionListRequest.current += 1; };
  }, [refreshSessions, compatible, statusAttempt, sessionsAttempt]);
  useEffect(() => {
    if (!compatible || !sessionsLoaded) return;
    if (sessions.length === 0) {
      resetSessionRuntime();
      return;
    }
    if (!state.sessionId || !sessions.some((session) => session.id === state.sessionId)) {
      const preferred = sessions.find((session) => session.is_default) ?? sessions[0];
      switchSession(preferred.id);
    }
  }, [compatible, sessionsLoaded, sessions, state.sessionId, resetSessionRuntime, switchSession]);
  useEffect(() => {
    const sessionId = state.sessionId;
    if (!compatible || !sessionId || !sessionAvailable) return;
    const epoch = state.epoch;
    const observation = ++sessionObservation.current;
    const recovering = recoveryResyncRef.current;
    const recoveryFocusToken = focusTokenRef.current;
    const recoveryMutationToken = mutationTokenRef.current;
    const controller = new AbortController();
    let active = true;
    sessionStream.current?.close();
    sessionStream.current = null;
    dispatch({ type: "snapshot/request", epoch, sessionId });
    void (async () => {
      try {
        const snapshot = await client.sessionSnapshot(sessionId, controller.signal);
        if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
        dispatch({ type: "snapshot/received", epoch, sessionId, snapshot });
        const stream = await client.subscribeSession(sessionId, (message: SessionStreamMessage) => {
          if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
          if (recovering && message.type === "snapshot" && message.sequence === 1 && recoveryFocusToken === focusTokenRef.current && stateRef.current.focusError) {
            // Reissue the coordinator's retained intent. The bootstrap snapshot
            // is a stale observation and must not become a new user request.
            retryFocus();
          }
          dispatch({ type: "stream/message", epoch, sessionId, message });
          if (message.type !== "snapshot" || message.sequence !== 1) return;
          if (controlInitializedEpoch.current !== epoch) {
            controlInitializedEpoch.current = epoch;
            setControlPaneId(message.snapshot.focused_pane_id);
          }
          const mutationFocusPane = consumeFocusedPane(epoch, message.snapshot.focused_pane_id);
          if (mutationFocusPane) setControlPaneId(mutationFocusPane);
          if (recovering && recoveryMutationToken === mutationTokenRef.current) {
            recoveryResyncRef.current = false;
          }
        }, (error: unknown) => {
          if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
          const described = describeError(error, "Session stream disconnected");
          dispatch({ type: "stream/error", epoch, sessionId, code: described.code ?? "stream_disconnected", message: described.message });
        }, controller.signal);
        if (active && !controller.signal.aborted && sessionObservation.current === observation) sessionStream.current = stream; else stream.close();
      } catch (error: unknown) {
        if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
        const described = describeError(error, "Could not read the session snapshot");
        dispatch({ type: "stream/error", epoch, sessionId, code: described.code ?? "snapshot_error", message: described.message });
      }
    })();
    return () => {
      active = false;
      controller.abort();
      sessionStream.current?.close();
      sessionStream.current = null;
    };
  }, [client, compatible, sessionAvailable, state.sessionId, state.epoch, resyncAttempt]);
  useEffect(() => {
    if (state.sync !== "live") {
      if (healthyLiveTimer.current !== null) {
        window.clearTimeout(healthyLiveTimer.current);
        healthyLiveTimer.current = null;
      }
      return;
    }
    if (healthyLiveTimer.current !== null) return;
    const epoch = state.epoch;
    const generation = state.generation;
    // A stream snapshot is only a bootstrap boundary; require one second of
    // ordered live traffic so an immediate snapshot/disconnect cannot renew
    // the outage budget indefinitely.
    healthyLiveTimer.current = window.setTimeout(() => {
      healthyLiveTimer.current = null;
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.generation !== generation || stateRef.current.sync !== "live") return;
      autoResyncAttempts.current = 0;
    }, 1000);
    return () => {
      if (healthyLiveTimer.current !== null) {
        window.clearTimeout(healthyLiveTimer.current);
        healthyLiveTimer.current = null;
      }
    };
  }, [state.sync, state.epoch, state.generation]);
  useEffect(() => {
    if (state.sync !== "stale" && state.sync !== "disconnected") return;
    if (autoResyncAttempts.current >= 3 || autoResyncTimer.current !== null) return;
    const epoch = state.epoch;
    const sessionId = state.sessionId;
    const attempt = autoResyncAttempts.current;
    autoResyncTimer.current = window.setTimeout(() => {
      autoResyncTimer.current = null;
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId
        || (stateRef.current.sync !== "stale" && stateRef.current.sync !== "disconnected")) return;
      autoResyncAttempts.current += 1;
      setResyncAttempt((value) => value + 1);
    }, [250, 500, 1000][attempt] ?? 1000);
    return () => {
      if (autoResyncTimer.current !== null) {
        window.clearTimeout(autoResyncTimer.current);
        autoResyncTimer.current = null;
      }
    };
  }, [state.sync, state.epoch]);
  useEffect(() => {
    if (!state.snapshot) return;
    const next = authoritativeSelection(state.snapshot);
    if (!state.focusPending) setSelection(next);
    const nextControlPaneId = reconcileFocus(state, next, controlPaneId);
    if (nextControlPaneId !== undefined) setControlPaneId(nextControlPaneId);
  }, [state.snapshot, state.sync, state.epoch, state.focusPending, state.focusToken, state.focusError, controlPaneId, reconcileFocus]);

  // With no session the Library opens full-screen in place of the notice screens.
  const [noSessionLibraryOpen, setNoSessionLibraryOpen] = useState(false);
  const returnToLibraryOpener = useRef(false);
  useEffect(() => {
    if (noSessionLibraryOpen || !returnToLibraryOpener.current) return;
    returnToLibraryOpener.current = false;
    document.querySelector<HTMLElement>("[data-library-opener]")?.focus({ preventScroll: true });
  }, [noSessionLibraryOpen]);
  const openNoSessionLibrary = () => setNoSessionLibraryOpen(true);
  const noSessionLibrary = noSessionLibraryOpen
    ? <div className="app-shell"><LibraryView client={client} fullScreen onClose={() => { returnToLibraryOpener.current = true; setNoSessionLibraryOpen(false); }} /></div>
    : null;
  const explicitResync = () => {
    clearRecoveryTimers();
    autoResyncAttempts.current = 0;
    void refreshSessions();
    requestResync();
  };
  if ((!status || !compatible) && (statusError || status) && noSessionLibrary) return noSessionLibrary;
  if (!status || !compatible) return <div className="app-shell">{statusError || (status && !compatible) ? <CompatibilityNotice status={status} error={statusError} retry={() => setStatusAttempt((value) => value + 1)} onOpenLibrary={openNoSessionLibrary} /> : <main className="compatibility-main" aria-live="polite"><section className="notice notice-loading" role="status"><p className="eyebrow">Cockpit</p><h1>Connecting to Herdr</h1><p>Reading compatibility status...</p></section></main>}</div>;
  if (((sessionsError && sessions.length === 0) || (sessionsLoaded && sessions.length === 0)) && noSessionLibrary) return noSessionLibrary;
  if (sessionsError && sessions.length === 0) return <div className="app-shell"><CompatibilityNotice status={status} error={sessionsError} retry={() => setSessionsAttempt((value) => value + 1)} onOpenLibrary={openNoSessionLibrary} /></div>;
  if (sessionsLoaded && sessions.length === 0) return <div className="app-shell"><main className="compatibility-main"><section className="notice"><h1>No Herdr sessions</h1><p>Create or start a session, then refresh the list.</p><div className="notice-actions"><button type="button" className="action-button" onClick={() => setSessionsAttempt((value) => value + 1)}>Refresh sessions</button><OpenLibraryButton onOpen={openNoSessionLibrary} /></div></section></main></div>;
  return <div className="app-shell"><Workbench key={state.epoch} client={client} state={state} sessions={sessions} selection={selection} controlPaneId={controlPaneId} terminalMouseInput={status.capabilities.terminal_mouse_input} mutations={mutations} browserHandoff={browserHandoffRef.current} onSession={switchSession} onFocus={focusAndSelect} onPanePrepared={panePrepared} onRequestControl={setControlPaneId} onReconnect={explicitResync} onRetry={retryFocus} onRefreshSessions={refreshSessions} onOpenSession={() => { void refreshSessions(); }} onMutate={mutate} onRetryMutation={retryMutation} /></div>;
}
