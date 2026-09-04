import { useCallback, useEffect, useReducer, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent, type ReactNode, type RefObject } from "react";
import {
  parseResourceMutationResponse,
  type CockpitClient,
  type CockpitStatus,
  type TerminalStream,
} from "../client/CockpitClient";
import type {
  FocusRequest,
  LayoutRect,
  ResourceMutationRequest,
  ResourceMutationResponse,
  SessionSnapshotResponse,
  SessionStreamMessage,
  SessionSummary,
  TabLayout,
  TerminalOpenRequest,
} from "../protocol/generated/v1";
import { initialSessionState, sessionReducer, type SessionState } from "./sessionReducer";
import { TerminalPane } from "./TerminalPane";

type StatusError = { message: string; code?: string };
type SessionSnapshot = SessionSnapshotResponse;
type Space = SessionSnapshot["spaces"][number];
type Tab = SessionSnapshot["tabs"][number];
type Pane = SessionSnapshot["panes"][number];
type Agent = SessionSnapshot["agents"][number];
type Selection = { spaceId: string | null; tabId: string | null; paneId: string | null };

type MutationOperation = {
  epoch: number;
  token: number;
  key: string;
  request: ResourceMutationRequest;
  focusFromSnapshot: boolean;
};
type MutationFailure = StatusError & { operation: MutationOperation };
export type MutationCoordinatorState = {
  token: number;
  pending: MutationOperation | null;
  errors: Record<string, MutationFailure>;
};
export type MutationCoordinatorAction =
  | { type: "begin"; operation: MutationOperation }
  | { type: "succeed"; epoch: number; token: number }
  | { type: "fail"; epoch: number; token: number; error: StatusError }
  | { type: "clear"; key: string }
  | { type: "reset" };

export const initialMutationCoordinatorState: MutationCoordinatorState = { token: 0, pending: null, errors: {} };
export function mutationCoordinatorReducer(state: MutationCoordinatorState, action: MutationCoordinatorAction): MutationCoordinatorState {
  if (action.type === "reset") return initialMutationCoordinatorState;
  if (action.type === "begin") {
    if (state.pending) return state;
    const errors = { ...state.errors };
    delete errors[action.operation.key];
    return { token: action.operation.token, pending: action.operation, errors };
  }
  if (action.type === "clear") {
    const errors = { ...state.errors };
    delete errors[action.key];
    return { ...state, errors };
  }
  const pending = state.pending;
  if (!pending || pending.epoch !== action.epoch || pending.token !== action.token) return state;
  if (action.type === "succeed") return { ...state, pending: null };
  return { ...state, pending: null, errors: { ...state.errors, [pending.key]: { ...action.error, operation: pending } } };
}

export const FOCUS_FALLBACK_MS = 500;
export function scheduleFocusFallback(isCurrent: () => boolean, onDelayed: () => void, onResync: () => void): () => void {
  const timer = globalThis.setTimeout(() => {
    if (!isCurrent()) return;
    onDelayed();
    onResync();
  }, FOCUS_FALLBACK_MS);
  return () => globalThis.clearTimeout(timer);
}

export type ResizeHandle = {
  id: string;
  paneId: string;
  axis: "x" | "y";
  negativeDirection: "left" | "up";
  positiveDirection: "right" | "down";
  coordinate: number;
  start: number;
  length: number;
};
const EPSILON = 0.000001;
const edge = (rect: LayoutRect, axis: "x" | "y") => axis === "x" ? rect.x + rect.width : rect.y + rect.height;
const overlap = (aStart: number, aLength: number, bStart: number, bLength: number) => {
  const start = Math.max(aStart, bStart);
  return { start, length: Math.min(aStart + aLength, bStart + bLength) - start };
};
export function deriveResizeHandles(layout: TabLayout | undefined): ResizeHandle[] {
  if (!layout || layout.zoomed) return [];
  const handles: ResizeHandle[] = [];
  for (let leftIndex = 0; leftIndex < layout.panes.length; leftIndex += 1) {
    for (let rightIndex = leftIndex + 1; rightIndex < layout.panes.length; rightIndex += 1) {
      const first = layout.panes[leftIndex];
      const second = layout.panes[rightIndex];
      const verticalOverlap = overlap(first.rect.y, first.rect.height, second.rect.y, second.rect.height);
      if (verticalOverlap.length > EPSILON) {
        const firstBefore = Math.abs(edge(first.rect, "x") - second.rect.x) <= EPSILON;
        const secondBefore = Math.abs(edge(second.rect, "x") - first.rect.x) <= EPSILON;
        if (firstBefore || secondBefore) {
          const pane = firstBefore ? first : second;
          handles.push({ id: `x:${pane.pane_id}:${firstBefore ? second.pane_id : first.pane_id}`, paneId: pane.pane_id, axis: "x", negativeDirection: "left", positiveDirection: "right", coordinate: edge(pane.rect, "x"), start: verticalOverlap.start, length: verticalOverlap.length });
        }
      }
      const horizontalOverlap = overlap(first.rect.x, first.rect.width, second.rect.x, second.rect.width);
      if (horizontalOverlap.length > EPSILON) {
        const firstBefore = Math.abs(edge(first.rect, "y") - second.rect.y) <= EPSILON;
        const secondBefore = Math.abs(edge(second.rect, "y") - first.rect.y) <= EPSILON;
        if (firstBefore || secondBefore) {
          const pane = firstBefore ? first : second;
          handles.push({ id: `y:${pane.pane_id}:${firstBefore ? second.pane_id : first.pane_id}`, paneId: pane.pane_id, axis: "y", negativeDirection: "up", positiveDirection: "down", coordinate: edge(pane.rect, "y"), start: horizontalOverlap.start, length: horizontalOverlap.length });
        }
      }
    }
  }
  return handles;
}
export function resizeRequest(handle: ResizeHandle, pixelDelta: number, canvasPixels: number): ResourceMutationRequest | null {
  if (!Number.isFinite(pixelDelta) || !Number.isFinite(canvasPixels) || canvasPixels <= 0 || Math.abs(pixelDelta) < 1) return null;
  const amount = Math.abs(pixelDelta) / canvasPixels;
  if (!Number.isFinite(amount) || amount <= 0) return null;
  return { type: "pane_resize", pane_id: handle.paneId, direction: pixelDelta < 0 ? handle.negativeDirection : handle.positiveDirection, amount };
}

export function projectedPaneIds(paneIds: string[], layout: TabLayout | undefined, fallbackPaneId: string | null): string[] {
  if (!layout?.zoomed) return paneIds;
  const focusedPaneId = paneIds.includes(layout.focused_pane_id ?? "")
    ? layout.focused_pane_id
    : paneIds.includes(fallbackPaneId ?? "") ? fallbackPaneId : paneIds[0] ?? null;
  return focusedPaneId ? [focusedPaneId] : [];
}

export function projectedPaneRect(layout: TabLayout | undefined, paneId: string): LayoutRect | undefined {
  if (!layout) return undefined;
  if (layout.zoomed) return layout.area;
  return layout.panes.find((candidate) => candidate.pane_id === paneId)?.rect;
}

export function tabDropInsertionIndex(sourceIndex: number, targetIndex: number, afterTarget: boolean): number | null {
  if (sourceIndex < 0 || targetIndex < 0 || sourceIndex === targetIndex) return null;
  return targetIndex + (afterTarget ? 1 : 0);
}

function describeError(error: unknown, fallback: string): StatusError {
  if (error instanceof Error) {
    const typed = error as Error & { code?: unknown; operationCode?: unknown };
    return { message: typed.message || fallback, code: typeof typed.operationCode === "string" ? typed.operationCode : typeof typed.code === "string" ? typed.code : undefined };
  }
  return { message: fallback };
}
function stateGlyph(status: string): string {
  switch (status.toLowerCase()) {
    case "blocked": case "error": return "●";
    case "working": case "running": return "◐";
    case "done": case "complete": return "●";
    case "idle": return "○";
    default: return "·";
  }
}
function stateClass(status: string): string {
  switch (status.toLowerCase()) {
    case "blocked": case "error": return "blocked";
    case "working": case "running": return "working";
    case "done": case "complete": return "done";
    case "idle": return "idle";
    default: return "unknown";
  }
}

export type SpaceTreeRow = {
  kind: "top-level" | "parent" | "child";
  space: Space;
  label: string;
  branch: string | null;
  repositoryKey: string | null;
  expanded: boolean;
  connector: "├─" | "└─" | null;
};

export function spaceStatus(status: string): { glyph: string; className: string } {
  return { glyph: stateGlyph(status), className: stateClass(status) };
}

function worktreeLabel(space: Space): string {
  const branch = space.git?.branch;
  return branch ? branch.replace(/^worktree\//, "") : space.label;
}

export function projectSpaceTree(
  spaces: Space[],
  collapsedRepositoryKeys: ReadonlySet<string> = new Set(),
  selectedSpaceId: string | null = null,
): SpaceTreeRow[] {
  const membersByRepository = new Map<string, Space[]>();
  for (const space of spaces) {
    const repositoryKey = space.git?.repository_key;
    if (!repositoryKey) continue;
    const members = membersByRepository.get(repositoryKey);
    if (members) members.push(space);
    else membersByRepository.set(repositoryKey, [space]);
  }

  const groups = new Map<string, { parent: Space; members: Space[] }>();
  for (const [repositoryKey, members] of membersByRepository) {
    const parent = members.find((space) => !space.git?.is_linked_worktree);
    if (members.length >= 2 && parent) groups.set(repositoryKey, { parent, members });
  }

  const rows: SpaceTreeRow[] = [];
  const emittedRepositoryKeys = new Set<string>();
  for (const space of spaces) {
    const repositoryKey = space.git?.repository_key ?? null;
    const group = repositoryKey === null ? undefined : groups.get(repositoryKey);
    if (repositoryKey === null || !group) {
      rows.push({ kind: "top-level", space, label: space.label, branch: space.git?.branch ?? null, repositoryKey: null, expanded: true, connector: null });
      continue;
    }
    if (emittedRepositoryKeys.has(repositoryKey)) continue;
    emittedRepositoryKeys.add(repositoryKey);

    const expanded = !collapsedRepositoryKeys.has(repositoryKey);
    rows.push({ kind: "parent", space: group.parent, label: group.parent.label, branch: group.parent.git?.branch ?? null, repositoryKey, expanded, connector: null });
    const children = group.members.filter((member) => member.id !== group.parent.id);
    const visibleChildren = expanded ? children : children.filter((child) => child.id === selectedSpaceId);
    visibleChildren.forEach((child, index) => rows.push({
      kind: "child",
      label: worktreeLabel(child),
      space: child,
      branch: child.git?.branch ?? null,
      repositoryKey,
      expanded,
      connector: index === visibleChildren.length - 1 ? "└─" : "├─",
    }));
  }
  return rows;
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
function focusRequestForSnapshot(snapshot: SessionSnapshot): FocusRequest | null {
  if (snapshot.focused_pane_id) return { kind: "pane", target_id: snapshot.focused_pane_id };
  if (snapshot.focused_tab_id) return { kind: "tab", target_id: snapshot.focused_tab_id };
  if (snapshot.focused_space_id) return { kind: "space", target_id: snapshot.focused_space_id };
  return null;
}

function CompatibilityNotice({ status, error, retry }: { status: CockpitStatus | null; error: StatusError | null; retry: () => void }) {
  const herdr = status?.herdr;
  const title = error ? "Cockpit unavailable" : herdr?.status === "incompatible" ? "Herdr is incompatible" : "Herdr is unavailable";
  const message = error?.message ?? (herdr && herdr.status !== "compatible" ? herdr.message : undefined);
  const code = error?.code ?? (herdr && herdr.status !== "compatible" ? herdr.code : undefined);
  return <main className="compatibility-main" aria-live="polite"><section className="notice notice-error" role="alert"><p className="eyebrow">Cockpit</p><h1>{title}</h1><p>{message ?? "Could not read the Herdr compatibility status."}</p>{code ? <code>{code}</code> : null}<button type="button" className="action-button" onClick={retry}>Retry status</button></section></main>;
}

type Mutate = (key: string, request: ResourceMutationRequest, focusFromSnapshot?: boolean) => boolean;
type ContextTarget =
  | { kind: "space"; id: string }
  | { kind: "tab"; id: string }
  | { kind: "pane"; id: string };
type ContextMenuState = { target: ContextTarget; x: number; y: number };
type PaneDialog =
  | { kind: "rename"; paneId: string }
  | { kind: "swap"; paneId: string }
  | { kind: "move"; paneId: string };
export type PrefixCommand =
  | "help" | "new-space" | "rename-space" | "close-space"
  | "new-tab" | "rename-tab" | "previous-tab" | "next-tab" | "close-tab"
  | "rename-pane" | "split-right" | "split-down" | "close-pane" | "zoom-pane" | "resize";

export function prefixCommandForKey(key: string, shiftKey: boolean): PrefixCommand | null {
  if (key === "?") return "help";
  if (shiftKey && key.toLowerCase() === "n") return "new-space";
  if (shiftKey && key.toLowerCase() === "w") return "rename-space";
  if (shiftKey && key.toLowerCase() === "d") return "close-space";
  if (!shiftKey && key === "c") return "new-tab";
  if (shiftKey && key.toLowerCase() === "t") return "rename-tab";
  if (!shiftKey && key === "p") return "previous-tab";
  if (!shiftKey && key === "n") return "next-tab";
  if (shiftKey && key.toLowerCase() === "x") return "close-tab";
  if (shiftKey && key.toLowerCase() === "p") return "rename-pane";
  if (!shiftKey && key === "v") return "split-right";
  if (!shiftKey && key === "-") return "split-down";
  if (!shiftKey && key === "x") return "close-pane";
  if (!shiftKey && key === "z") return "zoom-pane";
  if (!shiftKey && key === "r") return "resize";
  return null;
}

export function canSwitchSessions(sessionCount: number): boolean {
  return sessionCount > 1;
}

export function tabLabelIsRedundant(label: string, displayedNumber: number): boolean {
  return label.trim() === String(displayedNumber);
}

export function contextMenuPosition(
  x: number,
  y: number,
  viewportWidth: number,
  viewportHeight: number,
  menuWidth = 208,
  menuHeight = 320,
  gutter = 8,
): { x: number; y: number } {
  return {
    x: Math.max(gutter, Math.min(x, viewportWidth - menuWidth - gutter)),
    y: Math.max(gutter, Math.min(y, viewportHeight - menuHeight - gutter)),
  };
}

export function nextModalFocusIndex(current: number, count: number, shiftKey: boolean): number {
  if (count <= 0) return -1;
  if (current < 0) return shiftKey ? count - 1 : 0;
  return (current + (shiftKey ? count - 1 : 1)) % count;
}

function useModalFocus<T extends HTMLElement>(onDismiss: () => void): RefObject<T | null> {
  const ref = useRef<T | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const dismissRef = useRef(onDismiss);
  dismissRef.current = onDismiss;
  useEffect(() => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const focusable = ref.current?.querySelector<HTMLElement>("button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex='-1'])");
    focusable?.focus();
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        dismissRef.current();
      }
    };
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("keydown", escape);
      opener.current?.focus();
    };
  }, []);
  return ref;
}

function trapModalTab(event: ReactKeyboardEvent<HTMLElement>, root: HTMLElement | null): void {
  if (event.key !== "Tab") return;
  const controls = [...(root?.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex='-1'])") ?? [])];
  if (controls.length === 0) return;
  event.preventDefault();
  const next = nextModalFocusIndex(controls.indexOf(document.activeElement as HTMLElement), controls.length, event.shiftKey);
  controls[next]?.focus();
}

function editableTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));
}

function InlineRename({ label, ariaLabel, onCommit, onCancel }: { label: string; ariaLabel: string; onCommit: (label: string) => boolean; onCancel: () => void }) {
  const [value, setValue] = useState(label);
  return <input className="inline-rename" aria-label={ariaLabel} autoFocus value={value} onChange={(event) => setValue(event.target.value)} onBlur={onCancel} onKeyDown={(event) => {
    if (event.key === "Escape") { event.preventDefault(); onCancel(); }
    if (event.key === "Enter") { event.preventDefault(); const next = value.trim(); if (!next) onCancel(); else onCommit(next); }
  }} />;
}

function ContextMenu({ menu, children, onDismiss }: { menu: ContextMenuState; children: ReactNode; onDismiss: () => void }) {
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const dismiss = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) onDismiss(); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") onDismiss(); };
    window.addEventListener("pointerdown", dismiss);
    window.addEventListener("keydown", escape);
    ref.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    return () => { window.removeEventListener("pointerdown", dismiss); window.removeEventListener("keydown", escape); };
  }, [onDismiss, menu]);
  const position = contextMenuPosition(menu.x, menu.y, window.innerWidth, window.innerHeight);
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

function Spaces({ spaces, selectedSpaceId, editingId, busy, onEdit, onSelect, onContext, mutate }: {
  spaces: Space[];
  selectedSpaceId: string | null;
  editingId: string | null;
  busy: boolean;
  onEdit: (id: string | null) => void;
  onSelect: (space: Space) => void;
  onContext: (event: MouseEvent, target: ContextTarget) => void;
  mutate: Mutate;
}) {
  const [collapsedRepositoryKeys, setCollapsedRepositoryKeys] = useState<Set<string>>(() => new Set());
  const rows = projectSpaceTree(spaces, collapsedRepositoryKeys, selectedSpaceId);
  const toggleRepository = (repositoryKey: string) => {
    setCollapsedRepositoryKeys((current) => {
      const next = new Set(current);
      if (next.has(repositoryKey)) next.delete(repositoryKey);
      else next.add(repositoryKey);
      return next;
    });
  };
  return <section className="sidebar-section spaces-section" aria-labelledby="spaces-heading">
    <div className="sidebar-section-heading"><h2 id="spaces-heading">spaces</h2></div>
    <div className="space-list">{spaces.length === 0 ? <p className="empty-row">No spaces</p> : rows.map((row) => {
      const space = row.space;
      const status = spaceStatus(space.agent_status);
      const displayLabel = row.label;
      return <div className={`resource-row space-tree-row space-tree-${row.kind}${row.branch && row.kind !== "child" ? " has-branch" : ""} state-${status.className}${space.id === selectedSpaceId ? " is-selected" : ""}`} key={space.id} draggable={!busy && editingId !== space.id}
        onDragStart={(event) => { if (!busy) event.dataTransfer.setData("application/x-cockpit-space", space.id); }}
        onDragOver={(event) => { if (!busy) event.preventDefault(); }}
        onDrop={(event) => { if (busy) return; event.preventDefault(); const id = event.dataTransfer.getData("application/x-cockpit-space"); if (id && id !== space.id) mutate(`space:${id}`, { type: "space_move_block", space_ids: [id], before_space_id: space.id }); }}
        onContextMenu={(event) => onContext(event, { kind: "space", id: space.id })}>
        {row.kind === "parent" && row.repositoryKey
          ? <button type="button" className="space-chevron" disabled={busy} aria-label={`${row.expanded ? "Collapse" : "Expand"} ${space.label}`} aria-expanded={row.expanded} onClick={() => toggleRepository(row.repositoryKey!)}>{row.expanded ? "⌄" : "›"}</button>
          : row.kind === "child" ? <span className="space-connector" aria-hidden="true">{row.connector}</span> : null}
        {editingId === space.id
          ? <InlineRename label={space.label} ariaLabel={`Rename Space ${space.label}`} onCancel={() => onEdit(null)} onCommit={(label) => { const accepted = mutate(`space:${space.id}`, { type: "space_rename", space_id: space.id, label }); if (accepted) onEdit(null); return accepted; }} />
          : <button type="button" disabled={busy} className="resource-select" title={displayLabel} onClick={() => onSelect(space)} onDoubleClick={() => onEdit(space.id)}>
            <span className="resource-icon" aria-hidden="true">{status.glyph}</span>
            <span className="space-details"><span className="resource-label">{displayLabel}</span>{row.kind !== "child" && row.branch ? <span className="space-branch">{row.branch}</span> : null}</span>
          </button>}
      </div>;
    })}</div>
  </section>;
}

function Agents({ agents, spaces, tabs, selection, onSelect }: { agents: Agent[]; spaces: Space[]; tabs: Tab[]; selection: Selection; onSelect: (agent: Agent) => void }) {
  return <section className="sidebar-section agents-section" aria-labelledby="agents-heading"><div className="sidebar-section-heading"><h2 id="agents-heading">agents</h2></div><div className="agent-list">{agents.length === 0 ? <p className="empty-row">Inbox empty</p> : agents.map((agent) => {
    const location = [spaces.find((space) => space.id === agent.space_id)?.label, tabs.find((tab) => tab.id === agent.tab_id)?.label].filter(Boolean).join(" · ");
    return <button type="button" className={`agent-row${agent.pane_id === selection.paneId ? " is-selected" : ""} state-${stateClass(agent.status)}`} key={`${agent.pane_id}:${agent.name}`} onClick={() => onSelect(agent)} title={[location, agent.name].filter(Boolean).join(" · ")}><span className="agent-state" aria-hidden="true">{stateGlyph(agent.status)}</span><span className="agent-details">{location ? <span className="agent-location">{location}</span> : null}<span className="agent-name">{agent.name}</span></span></button>;
  })}</div></section>;
}

function TabStrip({ tabs, selectedTabId, editingId, busy, onEdit, onSelect, onContext, onCreate, mutate }: {
  tabs: Tab[];
  selectedTabId: string | null;
  editingId: string | null;
  busy: boolean;
  onEdit: (id: string | null) => void;
  onSelect: (tab: Tab) => void;
  onContext: (event: MouseEvent, target: ContextTarget) => void;
  onCreate: () => void;
  mutate: Mutate;
}) {
  return <nav className="tab-strip" role="tablist" aria-label="Tabs">{tabs.map((tab, index) => {
    const displayedNumber = tab.number || index + 1;
    return <div className={`tab-item${tab.id === selectedTabId ? " is-selected" : ""}`} key={tab.id} draggable={!busy && editingId !== tab.id}
      onDragStart={(event) => { if (!busy) event.dataTransfer.setData("application/x-cockpit-tab", tab.id); }}
      onDragOver={(event) => { if (!busy) event.preventDefault(); }}
      onDrop={(event) => { if (busy) return; event.preventDefault(); const id = event.dataTransfer.getData("application/x-cockpit-tab"); if (!id || id === tab.id) return; const insertion = tabDropInsertionIndex(tabs.findIndex((candidate) => candidate.id === id), index, event.clientX >= event.currentTarget.getBoundingClientRect().left + event.currentTarget.getBoundingClientRect().width / 2); if (insertion !== null) mutate(`tab:${id}`, { type: "tab_move", tab_id: id, insert_index: insertion }); }}
      onContextMenu={(event) => onContext(event, { kind: "tab", id: tab.id })}>
      {editingId === tab.id
        ? <InlineRename label={tab.label} ariaLabel={`Rename tab ${tab.label}`} onCancel={() => onEdit(null)} onCommit={(label) => { const accepted = mutate(`tab:${tab.id}`, { type: "tab_rename", tab_id: tab.id, label }); if (accepted) onEdit(null); return accepted; }} />
        : <button type="button" disabled={busy} role="tab" aria-selected={tab.id === selectedTabId} aria-label={`Tab ${tab.label}`} className="tab-button" title={tab.label} onClick={() => onSelect(tab)} onDoubleClick={() => onEdit(tab.id)}><span className="tab-number">{displayedNumber}</span>{tabLabelIsRedundant(tab.label, displayedNumber) ? null : <span className="tab-label">{tab.label}</span>}</button>}
    </div>;
  })}
    <button type="button" disabled={busy} className="tab-add" aria-label="Create tab" title="New tab (Ctrl+B c)" onClick={onCreate}>+</button>
  </nav>;
}

function PaneView({ pane, label, selected, showLabel, controlAllowed, pendingControl, onSelect, onContext, onRelease, request, client, registerStream, onRetry, onResync, mutate, style }: {
  pane: Pane;
  label: string;
  selected: boolean;
  showLabel: boolean;
  controlAllowed: boolean;
  pendingControl: boolean;
  onSelect: () => void;
  onContext: (event: MouseEvent, target: ContextTarget) => void;
  onRelease: () => void;
  request: Omit<TerminalOpenRequest, "mode" | "takeover" | "cols" | "rows">;
  client: CockpitClient;
  registerStream: (stream: TerminalStream, active: boolean) => void;
  onRetry: () => void;
  onResync: () => void;
  mutate: Mutate;
  style: { left: string; top: string; width: string; height: string };
}) {
  const title = pane.title || label;
  const closePane = () => { if (window.confirm(`Close ${title}?`)) mutate(`pane:${pane.id}`, { type: "pane_close", pane_id: pane.id }); };
  return <section className={`pane-view${selected ? " is-selected" : ""}`} style={style} aria-label={title}
    onPointerDownCapture={(event) => { if (event.button === 0 && !controlAllowed && !pendingControl) onSelect(); }}
    onContextMenu={(event) => onContext(event, { kind: "pane", id: pane.id })}>
    {showLabel ? <div className="pane-border-label" title={title}>{title}</div> : null}
    <div className="terminal-surface"><TerminalPane client={client} request={request} controlAllowed={controlAllowed} pendingControl={pendingControl} onRelease={onRelease} onRetry={onRetry} onResync={onResync} onClosed={onResync} onClosePane={closePane} registerStream={registerStream} /></div>
  </section>;
}

function ResizeHandles({ layout, mutate }: { layout: TabLayout | undefined; mutate: Mutate }) {
  const canvasRef = useRef<HTMLDivElement | null>(null);
  const [preview, setPreview] = useState<{ id: string; delta: number } | null>(null);
  const drag = useRef<{ handle: ResizeHandle; pointerId: number; start: number } | null>(null);
  const area = layout?.area;
  const pctX = (value: number) => area && area.width ? `${((value - area.x) / area.width) * 100}%` : "0%";
  const pctY = (value: number) => area && area.height ? `${((value - area.y) / area.height) * 100}%` : "0%";
  return <div ref={canvasRef} className="resize-layer">{deriveResizeHandles(layout).map((handle) => {
    const delta = preview?.id === handle.id ? preview.delta : 0;
    const style = handle.axis === "x" ? { left: pctX(handle.coordinate), top: pctY(handle.start), height: area ? `${handle.length / area.height * 100}%` : "0%", transform: `translateX(${delta}px)` } : { top: pctY(handle.coordinate), left: pctX(handle.start), width: area ? `${handle.length / area.width * 100}%` : "0%", transform: `translateY(${delta}px)` };
    return <div key={handle.id} role="separator" aria-label={`Resize pane ${handle.paneId} ${handle.axis === "x" ? "horizontally" : "vertically"}`} aria-orientation={handle.axis === "x" ? "vertical" : "horizontal"} tabIndex={0} className={`resize-handle resize-${handle.axis}`} style={style}
      onKeyDown={(event) => { const step = event.shiftKey ? 20 : 5; const deltaValue = handle.axis === "x" ? event.key === "ArrowLeft" ? -step : event.key === "ArrowRight" ? step : 0 : event.key === "ArrowUp" ? -step : event.key === "ArrowDown" ? step : 0; if (!deltaValue) return; event.preventDefault(); const bounds = canvasRef.current?.getBoundingClientRect(); const request = resizeRequest(handle, deltaValue, handle.axis === "x" ? bounds?.width ?? 0 : bounds?.height ?? 0); if (request) mutate(`pane:${handle.paneId}`, request); }}
      onPointerDown={(event) => { if (event.button !== 0) return; event.currentTarget.setPointerCapture(event.pointerId); drag.current = { handle, pointerId: event.pointerId, start: handle.axis === "x" ? event.clientX : event.clientY }; setPreview({ id: handle.id, delta: 0 }); }}
      onPointerMove={(event) => { const active = drag.current; if (!active || active.pointerId !== event.pointerId) return; setPreview({ id: active.handle.id, delta: (active.handle.axis === "x" ? event.clientX : event.clientY) - active.start }); }}
      onPointerUp={(event) => { const active = drag.current; if (!active || active.pointerId !== event.pointerId) return; const deltaValue = (active.handle.axis === "x" ? event.clientX : event.clientY) - active.start; const bounds = canvasRef.current?.getBoundingClientRect(); const request = resizeRequest(active.handle, deltaValue, active.handle.axis === "x" ? bounds?.width ?? 0 : bounds?.height ?? 0); drag.current = null; setPreview(null); if (request) mutate(`pane:${active.handle.paneId}`, request); }}
      onPointerCancel={() => { drag.current = null; setPreview(null); }} />;
  })}</div>;
}

const shortcutRows: Array<[string, string]> = [
  ["Ctrl+B ?", "commands"], ["Ctrl+B Shift+N", "new space"], ["Ctrl+B Shift+W", "rename space"], ["Ctrl+B Shift+D", "close space"],
  ["Ctrl+B c", "new tab"], ["Ctrl+B Shift+T", "rename tab"], ["Ctrl+B p", "previous tab"], ["Ctrl+B n", "next tab"], ["Ctrl+B Shift+X", "close tab"],
  ["Ctrl+B Shift+P", "rename pane"], ["Ctrl+B v", "split right"], ["Ctrl+B -", "split down"], ["Ctrl+B z", "toggle zoom"], ["Ctrl+B x", "close pane"],
  ["Ctrl+B r", "focus a resize border"], ["drag border / arrows", "resize"], ["right-click", "resource commands"],
];

function CommandOverlay({ run, onSwitchSession, onDismiss }: { run: (command: PrefixCommand) => void; onSwitchSession: () => void; onDismiss: () => void }) {
  const ref = useModalFocus<HTMLElement>(onDismiss);
  const clickable: Partial<Record<string, PrefixCommand>> = {
    "new space": "new-space", "rename space": "rename-space", "close space": "close-space", "new tab": "new-tab",
    "rename tab": "rename-tab", "previous tab": "previous-tab", "next tab": "next-tab", "close tab": "close-tab",
    "rename pane": "rename-pane", "split right": "split-right", "split down": "split-down", "toggle zoom": "zoom-pane",
    "close pane": "close-pane", "focus a resize border": "resize", resize: "resize",
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><section ref={ref} className="command-overlay" role="dialog" aria-modal="true" aria-labelledby="commands-title" onKeyDown={(event) => trapModalTab(event, ref.current)}><header><h2 id="commands-title">commands</h2><button type="button" onClick={onDismiss} aria-label="Close commands">Esc</button></header><button type="button" className="session-command" onClick={onSwitchSession}><span>switch session...</span></button><div className="shortcut-list">{shortcutRows.map(([keys, label]) => clickable[label] ? <button type="button" key={keys} onClick={() => run(clickable[label]!)}><kbd>{keys}</kbd><span>{label}</span></button> : <div key={keys}><kbd>{keys}</kbd><span>{label}</span></div>)}</div></section></div>;
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
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><form ref={ref} className="chooser-overlay" role="dialog" aria-modal="true" aria-labelledby="chooser-title" onSubmit={(event) => { event.preventDefault(); submit(); }} onKeyDown={(event) => trapModalTab(event, ref.current)}>
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
  const ref = useModalFocus<HTMLFormElement>(onDismiss);
  useEffect(() => setSessionId((selected) => reconcileSessionChoice(sessions, selected, currentSessionId)), [sessions, currentSessionId]);
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><form ref={ref} className="chooser-overlay" role="dialog" aria-modal="true" aria-labelledby="session-chooser-title" onSubmit={(event) => { event.preventDefault(); if (sessionId && sessionId !== currentSessionId) onSession(sessionId); onDismiss(); }} onKeyDown={(event) => trapModalTab(event, ref.current)}>
    <h2 id="session-chooser-title">switch session</h2>
    {sessions.length === 0 ? <div className="empty-choice" role="status">No sessions are available.</div> : <select aria-label="Session" value={sessionId} onChange={(event) => setSessionId(event.target.value)}>{sessions.map((session) => <option key={session.id} value={session.id}>{session.label}{session.running ? "" : " (stopped)"}</option>)}</select>}
    <footer>{sessions.length === 0 ? <button type="button" onClick={() => { void onRefresh().catch(() => undefined); }}>Refresh</button> : null}<button type="button" onClick={onDismiss}>Cancel</button><button type="submit" disabled={!sessionId || sessionId === currentSessionId}>Switch</button></footer>
  </form></div>;
}

export function mutationFailureCanRetry(request: ResourceMutationRequest, code: string | undefined): boolean {
  if (code === "mutation_applied_snapshot_failed") return false;
  if (request.type === "space_rename" || request.type === "space_move_block" || request.type === "tab_rename" || request.type === "tab_move" || request.type === "pane_rename") return true;
  return request.type === "pane_zoom" && request.mode !== "toggle";
}

function RecoveryPanel({ state, mutations, onReconnect, onRetry, onRetryMutation }: { state: SessionState; mutations: MutationCoordinatorState; onReconnect: () => void; onRetry: () => void; onRetryMutation: (operation: MutationOperation) => void }) {
  const failures = Object.values(mutations.errors);
  if (!state.syncError && !state.focusError && failures.length === 0) return null;
  return <aside className="recovery-panel" aria-label="Recovery" role="alert">
    {state.syncError ? <div><span>{state.syncError.message}</span><button type="button" onClick={onReconnect}>Resync</button></div> : null}
    {state.focusError ? <div><span>{state.focusError.message}</span><button type="button" onClick={onRetry}>Retry focus</button></div> : null}
    {failures.map((failure) => <div key={`${failure.operation.key}:${failure.operation.token}`}><span>{failure.message}</span>{failure.code ? <code>{failure.code}</code> : null}{mutationFailureCanRetry(failure.operation.request, failure.code) ? <button type="button" onClick={() => onRetryMutation(failure.operation)}>Retry</button> : null}<button type="button" onClick={onReconnect}>Resync</button></div>)}
  </aside>;
}

function Workbench({ client, state, sessions, selection, confirmedPaneId: confirmedPaneIdProp, mutations, onSession, onFocus, onRelease, onReconnect, onRetry, onRefreshSessions, onMutate, onRetryMutation }: {
  client: CockpitClient; state: SessionState; sessions: SessionSummary[]; selection: Selection; confirmedPaneId: string | null; mutations: MutationCoordinatorState;
  onSession: (id: string) => void; onFocus: (request: FocusRequest, location: Selection) => void; onRelease: () => void; onReconnect: () => void; onRetry: () => void; onRefreshSessions: () => Promise<void>; onMutate: Mutate; onRetryMutation: (operation: MutationOperation) => void;
}) {
  const snapshot = state.snapshot;
  const confirmedPaneId = state.sync === "live" ? confirmedPaneIdProp : null;
  const spaces = snapshot?.spaces ?? [];
  const allTabs = snapshot?.tabs ?? [];
  const tabs = tabsForSpace(allTabs, selection.spaceId);
  const selectedTab = byId(tabs, selection.tabId);
  const layout = snapshot?.layouts.find((candidate) => candidate.tab_id === selectedTab?.id && candidate.space_id === selection.spaceId);
  const panes = panesForTab(snapshot?.panes ?? [], selectedTab?.id ?? null);
  const visiblePaneIds = projectedPaneIds(panes.map((pane) => pane.id), layout, snapshot?.focused_pane_id ?? selection.paneId);
  const visiblePanes = panes.filter((pane) => visiblePaneIds.includes(pane.id));
  const [menu, setMenu] = useState<ContextMenuState | null>(null);
  const [editing, setEditing] = useState<ContextTarget | null>(null);
  const [dialog, setDialog] = useState<PaneDialog | null>(null);
  const [commandsOpen, setCommandsOpen] = useState(false);
  const [sessionChooserOpen, setSessionChooserOpen] = useState(false);
  const [prefixActive, setPrefixActive] = useState(false);
  const mutationBusy = mutations.pending !== null;
  const modalOpen = dialog !== null || commandsOpen || sessionChooserOpen;
  const streamRegistry = useRef(new Set<TerminalStream>());
  const registerStream = useCallback((stream: TerminalStream, active: boolean) => { if (active) streamRegistry.current.add(stream); else streamRegistry.current.delete(stream); }, []);
  useEffect(() => () => { streamRegistry.current.forEach((stream) => stream.close()); streamRegistry.current.clear(); }, []);
  const focusSpace = (space: Space) => { if (!modalOpen) onFocus({ kind: "space", target_id: space.id }, { spaceId: space.id, tabId: allTabs.find((tab) => tab.space_id === space.id && tab.focused)?.id ?? null, paneId: null }); };
  const focusTab = (tab: Tab) => { if (!modalOpen) onFocus({ kind: "tab", target_id: tab.id }, { spaceId: tab.space_id, tabId: tab.id, paneId: snapshot?.panes.find((pane) => pane.tab_id === tab.id && pane.focused)?.id ?? null }); };
  const focusPane = (pane: Pane) => { if (!modalOpen) onFocus({ kind: "pane", target_id: pane.id }, { spaceId: pane.space_id, tabId: pane.tab_id, paneId: pane.id }); };
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
  const runCommand = useCallback((command: PrefixCommand) => {
    const space = byId(spaces, selection.spaceId);
    const tab = byId(tabs, selection.tabId);
    const pane = byId(panes, selection.paneId);
    if (command === "help") { setCommandsOpen(true); return; }
    if (mutationBusy && !["previous-tab", "next-tab", "resize"].includes(command)) return;
    if (command === "new-space") onMutate("space:new", { type: "space_create", label: null, cwd: null }, true);
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
    if (command === "resize" && !mutationBusy) document.querySelector<HTMLElement>(".resize-handle")?.focus();
  }, [spaces, tabs, panes, selection.spaceId, selection.tabId, selection.paneId, mutationBusy, modalOpen]);
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (modalOpen) return;
      if (event.key === "Escape" && prefixActive) { event.preventDefault(); setPrefixActive(false); return; }
      if (!prefixActive) {
        const target = event.target instanceof HTMLElement ? event.target : null;
        const prefixSafe = !editableTarget(target) || Boolean(target?.closest(".terminal-host"));
        if (event.ctrlKey && !event.shiftKey && event.key.toLowerCase() === "b" && prefixSafe) { event.preventDefault(); setPrefixActive(true); return; }
        if (event.key === "?" && !editableTarget(target)) { event.preventDefault(); setCommandsOpen(true); }
        return;
      }
      event.preventDefault();
      setPrefixActive(false);
      const command = prefixCommandForKey(event.key, event.shiftKey);
      if (command) runCommand(command);
    };
    window.addEventListener("keydown", keydown, true);
    return () => window.removeEventListener("keydown", keydown, true);
  }, [prefixActive, runCommand, modalOpen]);
  const openContext = (event: MouseEvent, target: ContextTarget) => { event.preventDefault(); event.stopPropagation(); if (!mutationBusy && !modalOpen) setMenu({ target, x: event.clientX, y: event.clientY }); };
  const dismissMenu = useCallback(() => setMenu(null), []);
  const menuAction = (action: () => boolean | void) => { if (action() !== false) dismissMenu(); };
  const renderMenu = () => {
    if (!menu) return null;
    const disabled = mutationBusy;
    if (menu.target.kind === "space") {
      const space = spaces.find((candidate) => candidate.id === menu.target.id);
      if (!space) return null;
      const index = spaces.indexOf(space);
      return <ContextMenu menu={menu} onDismiss={dismissMenu}><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}>Rename</button><button role="menuitem" type="button" disabled={disabled || index === 0} onClick={() => menuAction(() => onMutate(`space:${space.id}`, { type: "space_move_block", space_ids: [space.id], before_space_id: spaces[index - 1]?.id ?? null }))}>Move up</button><button role="menuitem" type="button" disabled={disabled || index === spaces.length - 1} onClick={() => menuAction(() => onMutate(`space:${space.id}`, { type: "space_move_block", space_ids: [space.id], before_space_id: spaces[index + 2]?.id ?? null }))}>Move down</button><button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeSpace(space))}>Close</button></ContextMenu>;
    }
    if (menu.target.kind === "tab") {
      const tab = allTabs.find((candidate) => candidate.id === menu.target.id);
      if (!tab) return null;
      const siblings = tabsForSpace(allTabs, tab.space_id);
      const index = siblings.indexOf(tab);
      return <ContextMenu menu={menu} onDismiss={dismissMenu}><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}>Rename</button><button role="menuitem" type="button" disabled={disabled || index === 0} onClick={() => menuAction(() => onMutate(`tab:${tab.id}`, { type: "tab_move", tab_id: tab.id, insert_index: index - 1 }))}>Move left</button><button role="menuitem" type="button" disabled={disabled || index === siblings.length - 1} onClick={() => menuAction(() => onMutate(`tab:${tab.id}`, { type: "tab_move", tab_id: tab.id, insert_index: index + 2 }))}>Move right</button><button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeTab(tab))}>Close</button></ContextMenu>;
    }
    const pane = snapshot?.panes.find((candidate) => candidate.id === menu.target.id);
    if (!pane) return null;
    return <ContextMenu menu={menu} onDismiss={dismissMenu}><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}>Rename</button><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => onMutate(`pane:${pane.id}`, { type: "pane_split", pane_id: pane.id, direction: "right", ratio: null }, true))}>Split right</button><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => onMutate(`pane:${pane.id}`, { type: "pane_split", pane_id: pane.id, direction: "down", ratio: null }, true))}>Split down</button><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => onMutate(`pane:${pane.id}`, { type: "pane_zoom", pane_id: pane.id, mode: "toggle" }))}>Toggle zoom</button><button role="menuitem" type="button" disabled={disabled || panes.length < 2} onClick={() => menuAction(() => { setDialog({ kind: "swap", paneId: pane.id }); return true; })}>Swap...</button><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => { setDialog({ kind: "move", paneId: pane.id }); return true; })}>Move...</button><button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closePane(pane))}>Close</button></ContextMenu>;
  };
  return <div className="workbench">
    <aside className="sidebar"><Spaces spaces={spaces} selectedSpaceId={selection.spaceId} editingId={editing?.kind === "space" ? editing.id : null} busy={mutationBusy} onEdit={(id) => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "space", id } : null); }} onSelect={focusSpace} onContext={openContext} mutate={onMutate} /><div className="sidebar-divider"><button type="button" disabled={mutationBusy || modalOpen} onClick={() => onMutate("space:new", { type: "space_create", label: null, cwd: null }, true)}>new</button><button type="button" disabled={modalOpen} onClick={() => setCommandsOpen(true)}>menu</button></div><Agents agents={snapshot?.agents ?? []} spaces={spaces} tabs={allTabs} selection={selection} onSelect={(agent) => { if (!modalOpen) onFocus({ kind: "agent", target_id: agent.pane_id }, { spaceId: agent.space_id, tabId: agent.tab_id, paneId: agent.pane_id }); }} /></aside>
    <main className="main-workarea">
      {selection.spaceId ? <TabStrip tabs={tabs} selectedTabId={selection.tabId} editingId={editing?.kind === "tab" ? editing.id : null} busy={mutationBusy} onEdit={(id) => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "tab", id } : null); }} onSelect={focusTab} onContext={openContext} onCreate={() => { if (selection.spaceId) onMutate("tab:new", { type: "tab_create", space_id: selection.spaceId, label: null }, true); }} mutate={onMutate} /> : null}
      <div className="pane-canvas">{panes.length === 0 ? <div className="empty-main"><strong>No panes</strong><span>Create a tab or select another space.</span></div> : visiblePanes.map((pane, index) => {
        const rectangle = projectedPaneRect(layout, pane.id);
        const area = layout?.area;
        const style = rectangle && area && area.width > 0 && area.height > 0 ? { left: `${(rectangle.x - area.x) / area.width * 100}%`, top: `${(rectangle.y - area.y) / area.height * 100}%`, width: `${rectangle.width / area.width * 100}%`, height: `${rectangle.height / area.height * 100}%` } : { left: `${index / visiblePanes.length * 100}%`, top: "0%", width: `${100 / visiblePanes.length}%`, height: "100%" };
        return <PaneView key={pane.id} pane={pane} label={pane.title ?? `Pane ${panes.indexOf(pane) + 1}`} selected={pane.id === selection.paneId} showLabel={panes.length > 1} controlAllowed={pane.id === confirmedPaneId && pane.id === snapshot?.focused_pane_id && !state.focusPending} pendingControl={state.focusPending?.kind === "pane" && state.focusPending.target_id === pane.id} onSelect={() => focusPane(pane)} onContext={openContext} onRelease={onRelease} request={{ session_id: state.sessionId!, pane_id: pane.id }} client={client} registerStream={registerStream} onRetry={() => focusPane(pane)} onResync={onReconnect} mutate={onMutate} style={style} />
      })}{mutationBusy ? null : <ResizeHandles layout={layout} mutate={onMutate} />}</div>
    </main>
    {renderMenu()}
    {dialog ? <PaneDialogOverlay dialog={dialog} panes={panes} tabs={allTabs} spaces={spaces} busy={mutationBusy} onDismiss={() => setDialog(null)} mutate={onMutate} /> : null}
    {commandsOpen ? <CommandOverlay run={(command) => { setCommandsOpen(false); runCommand(command); }} onSwitchSession={() => { setCommandsOpen(false); void onRefreshSessions().catch(() => undefined).finally(() => setSessionChooserOpen(true)); }} onDismiss={() => setCommandsOpen(false)} /> : null}
    {sessionChooserOpen ? <SessionDialogOverlay sessions={sessions} currentSessionId={state.sessionId} onRefresh={onRefreshSessions} onSession={onSession} onDismiss={() => setSessionChooserOpen(false)} /> : null}
    {prefixActive ? <div className="prefix-indicator" role="status">Ctrl+B</div> : null}
    <RecoveryPanel state={state} mutations={mutations} onReconnect={onReconnect} onRetry={onRetry} onRetryMutation={onRetryMutation} />
  </div>;
}

export function App({ client }: { client: CockpitClient }) {
  const [status, setStatus] = useState<CockpitStatus | null>(null);
  const [statusError, setStatusError] = useState<StatusError | null>(null);
  const [statusAttempt, setStatusAttempt] = useState(0);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [sessionsAttempt, setSessionsAttempt] = useState(0);
  const [sessionsLoaded, setSessionsLoaded] = useState(false);
  const [sessionsError, setSessionsError] = useState<StatusError | null>(null);
  const [state, dispatch] = useReducer(sessionReducer, initialSessionState);
  const [mutations, dispatchMutation] = useReducer(mutationCoordinatorReducer, initialMutationCoordinatorState);
  const [selection, setSelection] = useState<Selection>({ spaceId: null, tabId: null, paneId: null });
  const [confirmedPaneId, setConfirmedPaneId] = useState<string | null>(null);
  const sessionStream = useRef<{ close(): void } | null>(null);
  const focusTokenRef = useRef(0);
  const focusIntent = useRef<{ epoch: number; token: number; request: FocusRequest; location: Selection } | null>(null);
  const focusFallbackCancel = useRef<(() => void) | null>(null);
  const [focusDelayed, setFocusDelayed] = useState(false);
  const mutationTokenRef = useRef(0);
  const mutationPendingRef = useRef(false);
  const [resyncAttempt, setResyncAttempt] = useState(0);
  const recoveryResyncRef = useRef(false);
  const stateRef = useRef(state);
  stateRef.current = state;
  const autoResyncTimer = useRef<number | null>(null);
  const mountedRef = useRef(true);
  useEffect(() => { mountedRef.current = true; return () => { mountedRef.current = false; focusFallbackCancel.current?.(); }; }, []);
  useEffect(() => { let active = true; setStatus(null); setStatusError(null); void client.status().then((next) => { if (active) setStatus(next); }, (error: unknown) => { if (active) setStatusError(describeError(error, "Could not read Cockpit status")); }); return () => { active = false; }; }, [client, statusAttempt]);
  const compatible = status?.herdr.status === "compatible";
  useEffect(() => { if (!compatible) return; let active = true; setSessionsError(null); void client.sessions().then((response) => { if (active) { setSessions(response.sessions); setSessionsLoaded(true); } }, (error: unknown) => { if (active) { setSessionsError(describeError(error, "Could not list Herdr sessions")); setSessionsLoaded(true); } }); return () => { active = false; }; }, [client, compatible, statusAttempt, sessionsAttempt]);
  useEffect(() => {
    if (!compatible || !sessionsLoaded) return;
    if (sessions.length === 0) {
      sessionStream.current?.close();
      sessionStream.current = null;
      setSelection({ spaceId: null, tabId: null, paneId: null });
      setConfirmedPaneId(null);
      return;
    }
    if (!state.sessionId || !sessions.some((session) => session.id === state.sessionId)) {
      const preferred = sessions.find((session) => session.is_default) ?? sessions[0];
      dispatch({ type: "switch", sessionId: preferred.id });
      setSelection({ spaceId: null, tabId: null, paneId: null });
      setConfirmedPaneId(null);
    }
  }, [compatible, sessionsLoaded, sessions, state.sessionId]);
  useEffect(() => {
    const sessionId = state.sessionId;
    if (!compatible || !sessionId) return;
    const epoch = state.epoch;
    let active = true;
    sessionStream.current?.close();
    sessionStream.current = null;
    dispatch({ type: "snapshot/request", epoch, sessionId });
    void (async () => {
      try {
        const snapshot = await client.sessionSnapshot(sessionId);
        if (!active) return;
        const recovering = recoveryResyncRef.current;
        const confirmedFocus = recovering && stateRef.current.focusError ? focusRequestForSnapshot(snapshot) : null;
        if (confirmedFocus) {
          const clearToken = focusTokenRef.current + 1;
          focusTokenRef.current = clearToken;
          dispatch({ type: "focus/request", epoch, sessionId, request: confirmedFocus, token: clearToken });
          focusIntent.current = null;
        }
        dispatch({ type: "snapshot/received", epoch, sessionId, snapshot });
        if (recovering) {
          dispatchMutation({ type: "reset" });
          mutationPendingRef.current = false;
          recoveryResyncRef.current = false;
        }
        setConfirmedPaneId(snapshot.focused_pane_id);
        const stream = await client.subscribeSession(sessionId, (message: SessionStreamMessage) => dispatch({ type: "stream/message", epoch, sessionId, message }), (error: unknown) => { if (!active) return; const described = describeError(error, "Session stream disconnected"); dispatch({ type: "stream/error", epoch, sessionId, code: described.code ?? "stream_disconnected", message: described.message }); });
        if (active) sessionStream.current = stream; else stream.close();
      } catch (error: unknown) {
        if (!active) return;
        const described = describeError(error, "Could not read the session snapshot");
        dispatch({ type: "stream/error", epoch, sessionId, code: described.code ?? "snapshot_error", message: described.message });
      }
    })();
    return () => { active = false; sessionStream.current?.close(); sessionStream.current = null; };
  }, [client, compatible, state.sessionId, state.epoch, resyncAttempt]);
  useEffect(() => {
    if (state.sync !== "stale" && state.sync !== "disconnected") return;
    if (autoResyncTimer.current !== null) return;
    autoResyncTimer.current = window.setTimeout(() => { autoResyncTimer.current = null; setResyncAttempt((value) => value + 1); }, 250);
    return () => { if (autoResyncTimer.current !== null) { window.clearTimeout(autoResyncTimer.current); autoResyncTimer.current = null; } };
  }, [state.sync, state.epoch]);
  useEffect(() => {
    if (!state.snapshot) return;
    const next = authoritativeSelection(state.snapshot);
    setSelection(next);
    const intent = focusIntent.current;
    if (intent && intent.epoch === state.epoch && intent.token === state.focusToken && !state.focusPending) {
      focusFallbackCancel.current?.();
      focusFallbackCancel.current = null;
      setFocusDelayed(false);
      setConfirmedPaneId(next.paneId);
      focusIntent.current = null;
    }
  }, [state.snapshot, state.epoch, state.focusPending, state.focusToken]);
  const switchSession = (id: string) => { sessionStream.current?.close(); sessionStream.current = null; focusFallbackCancel.current?.(); focusFallbackCancel.current = null; setFocusDelayed(false); focusIntent.current = null; mutationTokenRef.current += 1; mutationPendingRef.current = false; dispatchMutation({ type: "reset" }); setSelection({ spaceId: null, tabId: null, paneId: null }); setConfirmedPaneId(null); dispatch({ type: "switch", sessionId: id }); };
  const focus = (request: FocusRequest, location: Selection) => {
    const sessionId = state.sessionId;
    if (!sessionId) return;
    const epoch = state.epoch;
    const token = focusTokenRef.current + 1;
    focusTokenRef.current = token;
    focusFallbackCancel.current?.();
    focusFallbackCancel.current = null;
    setFocusDelayed(false);
    focusIntent.current = { epoch, token, request, location };
    setConfirmedPaneId(null);
    dispatch({ type: "focus/request", epoch, sessionId, request, token });
    void client.focus(sessionId, request).then((response) => {
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId || focusTokenRef.current !== token) return;
      if (!response.accepted) { dispatch({ type: "focus/error", epoch, sessionId, token, code: "focus_rejected", message: "Herdr did not accept this focus request" }); return; }
      if (!stateRef.current.focusPending) return;
      focusFallbackCancel.current = scheduleFocusFallback(
        () => mountedRef.current && stateRef.current.epoch === epoch && stateRef.current.sessionId === sessionId && focusTokenRef.current === token && stateRef.current.focusPending !== null,
        () => setFocusDelayed(true),
        () => { recoveryResyncRef.current = true; setResyncAttempt((value) => value + 1); },
      );
    }, (error: unknown) => {
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId || focusTokenRef.current !== token) return;
      const described = describeError(error, "Could not focus resource");
      focusIntent.current = { epoch, token, request, location };
      focusFallbackCancel.current?.();
      focusFallbackCancel.current = null;
      setFocusDelayed(false);
      dispatch({ type: "focus/error", epoch, sessionId, token, code: described.code ?? "focus_error", message: described.message });
    });
  };
  const mutate = useCallback<Mutate>((key, request, focusFromSnapshot = false) => {
    const current = stateRef.current;
    const sessionId = current.sessionId;
    if (!sessionId || mutationPendingRef.current) return false;
    const epoch = current.epoch;
    const token = mutationTokenRef.current + 1;
    mutationTokenRef.current = token;
    mutationPendingRef.current = true;
    const operation: MutationOperation = { epoch, token, key, request, focusFromSnapshot };
    dispatchMutation({ type: "begin", operation });
    const accepted = true;
    void client.mutate(sessionId, request).then((response) => {
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId || mutationTokenRef.current !== token) return;
      const snapshot = authoritativeMutationSnapshot(sessionId, response);
      mutationPendingRef.current = false;
      dispatchMutation({ type: "succeed", epoch, token });
      dispatch({ type: "snapshot/received", epoch, sessionId, snapshot, preserveStream: true });
      if (focusFromSnapshot && snapshot.focused_pane_id) {
        setSelection(authoritativeSelection(snapshot));
        setConfirmedPaneId(snapshot.focused_pane_id);
      }
    }).catch((error: unknown) => {
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId || mutationTokenRef.current !== token) return;
      mutationPendingRef.current = false;
      dispatchMutation({ type: "fail", epoch, token, error: describeError(error, "Could not update Herdr resource") });
    });
    return accepted;
  }, [client]);
  const retryMutation = (operation: MutationOperation) => { mutate(operation.key, operation.request, operation.focusFromSnapshot); };
  const retryFocus = () => {
    const intent = focusIntent.current;
    if (intent) focus(intent.request, intent.location);
  };
  const releaseControl = () => setConfirmedPaneId(null);
  const refreshSessions = useCallback(async () => {
    const response = await client.sessions();
    if (mountedRef.current) {
      setSessions(response.sessions);
      setSessionsError(null);
      setSessionsLoaded(true);
    }
  }, [client]);
  const explicitResync = () => {
    recoveryResyncRef.current = true;
    void refreshSessions().catch((error: unknown) => setSessionsError(describeError(error, "Could not list Herdr sessions")));
    setResyncAttempt((value) => value + 1);
  };
  if (!status || !compatible) return <div className="app-shell">{statusError || (status && !compatible) ? <CompatibilityNotice status={status} error={statusError} retry={() => setStatusAttempt((value) => value + 1)} /> : <main className="compatibility-main" aria-live="polite"><section className="notice notice-loading" role="status"><p className="eyebrow">Cockpit</p><h1>Connecting to Herdr</h1><p>Reading compatibility status...</p></section></main>}</div>;
  if (sessionsError && sessions.length === 0) return <div className="app-shell"><CompatibilityNotice status={status} error={sessionsError} retry={() => setSessionsAttempt((value) => value + 1)} /></div>;
  if (sessionsLoaded && sessions.length === 0) return <div className="app-shell"><main className="compatibility-main"><section className="notice"><h1>No Herdr sessions</h1><p>Create or start a session, then refresh the list.</p><button type="button" className="action-button" onClick={() => setSessionsAttempt((value) => value + 1)}>Refresh sessions</button></section></main></div>;
  return <div className="app-shell"><Workbench key={state.epoch} client={client} state={state} sessions={sessions} selection={selection} confirmedPaneId={confirmedPaneId} mutations={mutations} onSession={switchSession} onFocus={focus} onRelease={releaseControl} onReconnect={explicitResync} onRetry={retryFocus} onRefreshSessions={refreshSessions} onMutate={mutate} onRetryMutation={retryMutation} />{focusDelayed ? <div className="focus-feedback" role="status">Waiting for Herdr focus confirmation...</div> : null}</div>;
}
