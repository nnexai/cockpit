import { ReviewViewer } from "./review/ReviewViewer";
import { useCallback, useEffect, useReducer, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent, type ReactNode, type RefObject } from "react";
import {
  parseResourceMutationResponse,
  type CockpitClient,
  type CockpitStatus,
  type TerminalStream,
} from "../client/CockpitClient";
import type {
  BrowserFeedbackImage,
  BrowserFeedbackLookup,
  BrowserFeedbackSendResponse,
  FocusRequest,
  ResourceMutationRequest,
  ResourceMutationResponse,
  SessionSnapshotResponse,
  SessionStreamMessage,
  SessionSummary,
  TabLayout,
  TerminalOpenRequest,
} from "../protocol/generated/v1";
import { initialSessionState, sessionReducer, type SessionState } from "./session/sessionStore";
import { focusRequestForSnapshot, useFocusCoordinator } from "./session/focusCoordinator";
import { type MutationCoordinatorState, type MutationOperation, useMutationCoordinator } from "./session/mutationCoordinator";
import { deriveResizeHandles, projectedPaneIds, projectedPaneRect, resizeRequest, tabDropInsertionIndex, type ResizeHandle } from "./layout/layoutProjection";
import { type PrefixCommand, routeWorkbenchKeydown } from "./input/keymap";
import { dispatchFileNavigation } from "./input/fileNavigation";
import { TerminalPane } from "./TerminalPane";
import { SetupDialog } from "./projects/SetupDialog";
import { TeardownDialog } from "./projects/TeardownDialog";
import { TeardownRecoveryPanel } from "./projects/TeardownRecoveryPanel";
import { ContextViewer, type ContextViewState } from "./context/ContextViewer";
import { isGraphicalContext, isGraphicalReview, usePaneRenderers, type PaneRendererState } from "./paneRenderers";

type StatusError = { message: string; code?: string };
type SessionSnapshot = SessionSnapshotResponse;
type Space = SessionSnapshot["spaces"][number];
type Tab = SessionSnapshot["tabs"][number];
type Pane = SessionSnapshot["panes"][number];
type Agent = SessionSnapshot["agents"][number];
type Selection = { spaceId: string | null; tabId: string | null; paneId: string | null };

export function spaceDropBeforeId(spaces: Space[], sourceId: string, targetId: string, afterTarget: boolean): string | null | undefined {
  const sourceIndex = spaces.findIndex((space) => space.id === sourceId);
  if (sourceIndex < 0 || sourceId === targetId) return undefined;
  const remaining = spaces.filter((space) => space.id !== sourceId);
  const targetIndex = remaining.findIndex((space) => space.id === targetId);
  if (targetIndex < 0) return undefined;
  const insertionIndex = targetIndex + (afterTarget ? 1 : 0);
  if (insertionIndex === sourceIndex) return undefined;
  return remaining[insertionIndex]?.id ?? null;
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

function agentStatusPriority(status: string): number {
  switch (stateClass(status)) {
    case "blocked": return 4;
    case "done": return 3;
    case "working": return 2;
    case "idle": return 1;
    default: return 0;
  }
}

export function orderAgentsByHerdrPriority(agents: Agent[]): Agent[] {
  return agents
    .map((agent, index) => ({ agent, index }))
    .sort((left, right) => agentStatusPriority(right.agent.status) - agentStatusPriority(left.agent.status)
      || right.agent.state_change_seq - left.agent.state_change_seq
      || Number(right.agent.focused) - Number(left.agent.focused)
      || left.index - right.index)
    .map(({ agent }) => agent);
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

function Spaces({ spaces, selectedSpaceId, editingId, busy, onEdit, onSelect, onContext, onSetup, setupEnabled, mutate }: {
  spaces: Space[];
  selectedSpaceId: string | null;
  editingId: string | null;
  busy: boolean;
  onEdit: (id: string | null) => void;
  onSelect: (space: Space) => void;
  onContext: (event: MouseEvent, target: ContextTarget) => void;
  onSetup: () => void;
  setupEnabled: boolean;
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
    <div className="sidebar-section-heading"><h2 id="spaces-heading">spaces</h2><button type="button" className="space-setup" aria-label="Set up a task Space" title="Set up a task Space" disabled={busy || !setupEnabled} onClick={onSetup}>+</button></div>
    <div className="space-list">{spaces.length === 0 ? <p className="empty-row">No spaces</p> : rows.map((row, index) => {
      const space = row.space;
      const status = spaceStatus(space.agent_status);
      const displayLabel = row.label;
      return <div className={`resource-row space-tree-row space-tree-${row.kind}${row.branch && row.kind !== "child" ? " has-branch" : ""} state-${status.className}${space.id === selectedSpaceId ? " is-selected" : ""}`} key={space.id} draggable={!busy && editingId !== space.id}
        onDragStart={(event) => { if (!busy) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("application/x-cockpit-space", space.id); event.dataTransfer.setData("text/plain", `space:${space.id}`); } }}
        onDragEnter={(event) => { if (!busy) event.preventDefault(); }}
        onDragOver={(event) => { if (!busy) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; } }}
        onDrop={(event) => {
          if (busy) return;
          event.preventDefault();
          const fallback = event.dataTransfer.getData("text/plain");
          const id = event.dataTransfer.getData("application/x-cockpit-space") || (fallback.startsWith("space:") ? fallback.slice(6) : "");
          const beforeSpaceId = spaceDropBeforeId(spaces, id, space.id, event.clientY >= event.currentTarget.getBoundingClientRect().top + event.currentTarget.getBoundingClientRect().height / 2);
          if (beforeSpaceId !== undefined) mutate(`space:${id}`, { type: "space_move_block", space_ids: [id], before_space_id: beforeSpaceId });
        }}
        onContextMenu={(event) => onContext(event, { kind: "space", id: space.id })}>
        {row.kind === "child" ? <span className={`space-connector${rows[index - 1]?.kind === "parent" ? " is-first" : ""}${row.connector === "└─" ? " is-last" : ""}`} aria-hidden="true" /> : null}
        {editingId === space.id
          ? <InlineRename label={space.label} ariaLabel={`Rename Space ${space.label}`} onCancel={() => onEdit(null)} onCommit={(label) => { const accepted = mutate(`space:${space.id}`, { type: "space_rename", space_id: space.id, label }); if (accepted) onEdit(null); return accepted; }} />
          : <button type="button" disabled={busy} className="resource-select" title={displayLabel} onClick={() => onSelect(space)} onDoubleClick={() => onEdit(space.id)}>
            <span className="resource-icon" aria-hidden="true">{status.glyph}</span>
            <span className="space-details"><span className="resource-label">{displayLabel}</span>{row.kind !== "child" && row.branch ? <span className="space-branch">{row.branch}</span> : null}</span>
          </button>}
        {row.kind === "parent" && row.repositoryKey
          ? <button type="button" className="space-chevron" disabled={busy} aria-label={`${row.expanded ? "Collapse" : "Expand"} ${space.label}`} aria-expanded={row.expanded} onClick={() => toggleRepository(row.repositoryKey!)}>{row.expanded ? "⌄" : "›"}</button>
          : null}
      </div>;
    })}</div>
  </section>;
}
function Agents({ agents, spaces, tabs, selection, onSelect }: { agents: Agent[]; spaces: Space[]; tabs: Tab[]; selection: Selection; onSelect: (agent: Agent) => void }) {
  const orderedAgents = orderAgentsByHerdrPriority(agents);
  return <section className="sidebar-section agents-section" aria-labelledby="agents-heading"><div className="sidebar-section-heading"><h2 id="agents-heading">agents</h2></div><div className="agent-list">{orderedAgents.length === 0 ? <p className="empty-row">Inbox empty</p> : orderedAgents.map((agent) => {
    const location = [spaces.find((space) => space.id === agent.space_id)?.label, tabs.find((tab) => tab.id === agent.tab_id)?.label].filter(Boolean).join(" · ");
    return <button type="button" className={`agent-row${agent.pane_id === selection.paneId ? " is-selected" : ""} state-${stateClass(agent.status)}`} key={`${agent.pane_id}:${agent.name}`} onClick={() => onSelect(agent)} title={[location, agent.name].filter(Boolean).join(" · ")}><span className="agent-state" aria-hidden="true">{stateGlyph(agent.status)}</span><span className="agent-details">{location ? <span className="agent-location">{location}</span> : null}<span className="agent-name">{agent.name}</span></span></button>;
  })}</div></section>;
}

function TabStrip({ tabs, selectedTabId, editingId, busy, hasSelectedPane, onEdit, onSelect, onContext, onCreate, onPaneMenu, onCommands, mutate }: {
  tabs: Tab[];
  selectedTabId: string | null;
  editingId: string | null;
  busy: boolean;
  hasSelectedPane: boolean;
  onEdit: (id: string | null) => void;
  onSelect: (tab: Tab) => void;
  onContext: (event: MouseEvent, target: ContextTarget) => void;
  onCreate: () => void;
  onPaneMenu: (event: MouseEvent<HTMLButtonElement>) => void;
  onCommands: () => void;
  mutate: Mutate;
}) {
  return <nav className="tab-toolbar" aria-label="Tabs"><div className="tab-strip" role="tablist">{tabs.map((tab, index) => {
    const displayedNumber = index + 1;
    const redundantLabel = tabLabelIsRedundant(tab.label, displayedNumber);
    const accessibleLabel = redundantLabel ? `Tab ${displayedNumber}` : `Tab ${displayedNumber}: ${tab.label}`;
    return <div className={`tab-item${tab.id === selectedTabId ? " is-selected" : ""}`} key={tab.id} draggable={!busy && editingId !== tab.id}
      onDragStart={(event) => { if (!busy) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("application/x-cockpit-tab", tab.id); event.dataTransfer.setData("text/plain", `tab:${tab.id}`); } }}
      onDragEnter={(event) => { if (!busy) event.preventDefault(); }}
      onDragOver={(event) => { if (!busy) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; } }}
      onDrop={(event) => {
        if (busy) return;
        event.preventDefault();
        const fallback = event.dataTransfer.getData("text/plain");
        const id = event.dataTransfer.getData("application/x-cockpit-tab") || (fallback.startsWith("tab:") ? fallback.slice(4) : "");
        if (!id || id === tab.id) return;
        const insertion = tabDropInsertionIndex(tabs.findIndex((candidate) => candidate.id === id), index, event.clientX >= event.currentTarget.getBoundingClientRect().left + event.currentTarget.getBoundingClientRect().width / 2);
        if (insertion !== null) mutate(`tab:${id}`, { type: "tab_move", tab_id: id, insert_index: insertion });
      }}
      onContextMenu={(event) => onContext(event, { kind: "tab", id: tab.id })}>
      {editingId === tab.id
        ? <InlineRename label={tab.label} ariaLabel={`Rename tab ${tab.label}`} onCancel={() => onEdit(null)} onCommit={(label) => { const accepted = mutate(`tab:${tab.id}`, { type: "tab_rename", tab_id: tab.id, label }); if (accepted) onEdit(null); return accepted; }} />
        : <button type="button" disabled={busy} role="tab" aria-selected={tab.id === selectedTabId} aria-label={accessibleLabel} className="tab-button" title={redundantLabel ? `Tab ${displayedNumber}` : tab.label} onClick={() => onSelect(tab)} onDoubleClick={() => onEdit(tab.id)}><span className="tab-number">{displayedNumber}</span>{redundantLabel ? null : <span className="tab-label">{tab.label}</span>}</button>}
    </div>;
  })}
    <button type="button" disabled={busy} className="tab-add" aria-label="Create tab" title="New tab (Ctrl+B c)" onClick={onCreate}>+</button></div><div className="tab-strip-actions"><button type="button" className="tab-strip-action" disabled={busy || !hasSelectedPane} onClick={onPaneMenu}>Pane</button><button type="button" className="tab-strip-action" onClick={onCommands}>Commands</button></div>
  </nav>;
}

function PaneView({ pane, label, selected, showLabel, controlAllowed, controlPending, focusEpoch, focusToken, terminalMouseInput, onRequestControl, onSelect, onContext, request, client, registerStream, onResync, mutate, style, renderer, onRendererViewChange, onTerminalView, onRefreshRenderer }: {
  pane: Pane;
  label: string;
  selected: boolean;
  showLabel: boolean;
  controlAllowed: boolean;
  controlPending: boolean;
  focusEpoch: number;
  focusToken: number;
  terminalMouseInput: boolean;
  onRequestControl: () => void;
  onSelect: () => void;
  onContext: (event: MouseEvent, target: ContextTarget) => void;
  request: Omit<TerminalOpenRequest, "mode" | "takeover" | "cols" | "rows" | "cell_width_px" | "cell_height_px">;
  client: CockpitClient;
  registerStream: (stream: TerminalStream, active: boolean) => void;
  onResync: () => void;
  mutate: Mutate;
  style: { left: string; top: string; width: string; height: string };
  renderer: PaneRendererState | undefined;
  onRendererViewChange: (bindingId: string, value: ContextViewState) => void;
  onTerminalView: () => void;
  onRefreshRenderer: () => void;
}) {
  const title = pane.title || label;
  const closePane = () => { if (window.confirm(`Close ${title}?`)) mutate(`pane:${pane.id}`, { type: "pane_close", pane_id: pane.id }); };
  const graphical = isGraphicalContext(renderer) || isGraphicalReview(renderer);
  const graphicalRef = useRef<HTMLDivElement>(null);
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
  return <section className={`pane-view${selected ? " is-selected" : ""}`} style={style} aria-label={title}
    onContextMenu={(event) => onContext(event, { kind: "pane", id: pane.id })}>
    {controlPending ? <span className="pane-focus-pending" role="status" aria-label="Waiting for Herdr focus confirmation">Waiting for focus…</span> : null}
    {showLabel ? <div className="pane-border-label" title={title}>{title}</div> : null}
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
        controlAllowed={controlAllowed} onRequestControl={onRequestControl} onTerminalView={onTerminalView} />}
    </div> : <div className="terminal-surface"><TerminalPane client={client} request={request} selected={selected} controlAllowed={controlAllowed} controlPending={controlPending} focusEpoch={focusEpoch} focusToken={focusToken} terminalMouseInput={terminalMouseInput} onRequestControl={onRequestControl} onSelect={onSelect} onResync={onResync} onClosed={onResync} onClosePane={closePane} registerStream={registerStream} /></div>}
    {renderer?.actionError || (graphical && renderer?.inspectionError) ? <div className="pane-presentation-error" role="status"><span>{renderer.actionError ?? renderer.inspectionError}</span><button type="button" onClick={onRefreshRenderer}>Refresh</button>{graphical ? <button type="button" onClick={onTerminalView}>Terminal</button> : null}</div> : null}
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
  ["Ctrl+B c", "new tab"], ["Ctrl+B Shift+T", "rename tab"], ["Ctrl+B p", "previous tab"], ["Ctrl+B n", "next tab"], ["Ctrl+B 1…9", "select tab"], ["Ctrl+B Shift+X", "close tab"],
  ["Ctrl+B Shift+P", "rename pane"], ["Ctrl+B v", "split right"], ["Ctrl+B -", "split down"], ["Ctrl+B z", "toggle zoom"], ["Ctrl+B x", "close pane"],
  ["Ctrl+B o / Shift+O", "next / previous pane"], ["Ctrl+B h j k l", "focus pane left down up right"], ["Ctrl+B f / Ctrl+P", "open file picker"], ["Ctrl+B [ / ] or Alt+1 / 2", "focus file tree / content"],
  ["Ctrl+B r", "focus a resize border"], ["drag border / arrows", "resize"], ["right-click", "resource commands"],
];

function CommandOverlay({ run, onSwitchSession, onDismiss, contextActions }: { run: (command: PrefixCommand) => void; onSwitchSession: () => void; onDismiss: () => void; contextActions: ReactNode }) {
  const ref = useModalFocus<HTMLElement>(onDismiss);
  const clickable: Partial<Record<string, PrefixCommand>> = {
    "new space": "new-space", "rename space": "rename-space", "close space": "close-space", "new tab": "new-tab",
    "rename tab": "rename-tab", "previous tab": "previous-tab", "next tab": "next-tab", "close tab": "close-tab",
    "rename pane": "rename-pane", "split right": "split-right", "split down": "split-down", "toggle zoom": "zoom-pane",
    "close pane": "close-pane", "focus a resize border": "resize", resize: "resize",
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><section ref={ref} className="command-overlay" role="dialog" aria-modal="true" aria-labelledby="commands-title" onKeyDown={(event) => trapModalTab(event, ref.current)}><header><h2 id="commands-title">commands</h2><button type="button" onClick={onDismiss} aria-label="Close commands">Esc</button></header><div className="command-actions"><button type="button" className="session-command" onClick={onSwitchSession}><span>switch session...</span></button>{contextActions}</div><div className="shortcut-list">{shortcutRows.map(([keys, label]) => clickable[label] ? <button type="button" key={keys} onClick={() => run(clickable[label]!)}><kbd>{keys}</kbd><span>{label}</span></button> : <div className="shortcut-row" key={keys}><kbd>{keys}</kbd><span>{label}</span></div>)}</div></section></div>;
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
  if (code === "mutation_applied_snapshot_failed" || code === "request_outcome_unknown") return false;
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
type FeedbackImageState = Record<string, BrowserFeedbackImage | null>;

function FeedbackPanel({ lookup, images, busy, error, sendResult, riskPending, onRefresh, onAcknowledge, onSend, onRetryRisk, onDismiss }: {
  lookup: BrowserFeedbackLookup | null;
  images: FeedbackImageState;
  busy: boolean;
  error: StatusError | null;
  sendResult: BrowserFeedbackSendResponse | null;
  riskPending: boolean;
  onRefresh: () => void;
  onAcknowledge: () => void;
  onSend: () => void;
  onRetryRisk: () => void;
  onDismiss: () => void;
}) {
  const captures = lookup?.feedback.captures.filter(capture => capture.pending_ids.length > 0) ?? [];
  const pendingIds = Array.from(new Set(captures.flatMap((capture) => capture.pending_ids)));
  return <div className="feedback-panel" role="dialog" aria-modal="false" aria-labelledby="feedback-panel-title">
    <header>
      <div>
        <span className="heading-kicker">SPACE FEEDBACK</span>
        <h2 id="feedback-panel-title">Browser annotations</h2>
      </div>
      <div className="feedback-panel-actions">
        <button type="button" onClick={onRefresh} disabled={busy} aria-label="Refresh browser feedback">Refresh</button>
        <button type="button" onClick={onDismiss} aria-label="Close browser feedback">Close</button>
      </div>
    </header>
    {lookup ? <p className="feedback-browser-state">Browser: <strong>{lookup.browser.connection.replaceAll("_", " ")}</strong>{lookup.browser.message ? ` · ${lookup.browser.message}` : ""}</p> : null}
    {busy && !lookup ? <p className="feedback-state">Reading pending feedback…</p> : null}
    {error ? <div className="feedback-state feedback-state-error" role="alert"><span>{error.message}</span><button type="button" onClick={onRefresh} disabled={busy}>Retry</button></div> : null}
    {!error && lookup && captures.length === 0 ? <p className="feedback-state">No pending browser annotations.</p> : null}
    {lookup && captures.length > 0 ? <div className="feedback-captures">
      {captures.map((capture) => <article className="feedback-capture" key={capture.id}>
        <header>
          <div className="feedback-capture-heading"><strong>{capture.page.title || "Untitled page"}</strong><a href={capture.page.url} target="_blank" rel="noreferrer">{capture.page.url}</a></div>
          <time dateTime={capture.page.captured_at}>{new Date(capture.page.captured_at).toLocaleString()}</time>
        </header>
        {images[capture.id] ? <img className="feedback-image" src={`data:${images[capture.id]!.mime_type};base64,${images[capture.id]!.data_base64}`} alt={`Combined capture of ${capture.page.title || capture.page.url}`} /> : <div className="feedback-image-placeholder">{images[capture.id] === null ? "Image unavailable" : "Loading capture image…"}</div>}
        <div className="feedback-annotations">
          {capture.annotations.filter(annotation => capture.pending_ids.includes(annotation.id)).map((annotation) => <div className="feedback-annotation" key={annotation.id}>
            <span className="feedback-annotation-kind">{annotation.kind}</span>{annotation.comment ? <p>{annotation.comment}</p> : null}
            {annotation.element ? <details><summary>Element evidence</summary><span>{annotation.element.tag}{annotation.element.role ? ` · ${annotation.element.role}` : ""}{annotation.element.name ? ` · ${annotation.element.name}` : ""}</span><p>{annotation.element.excerpt || annotation.element.text}</p><code>{annotation.element.locators.join(" | ")}</code></details> : null}
          </div>)}
          {capture.pending_ids.filter((id) => !capture.annotations.some((annotation) => annotation.id === id)).map((id) => <div className="feedback-annotation" key={id}><code>{id}</code><span>pending annotation</span></div>)}
        </div>
        <footer><span>{capture.pending_ids.length} pending annotation{capture.pending_ids.length === 1 ? "" : "s"}</span></footer>
      </article>)}
    </div> : null}
    {pendingIds.length > 0 ? <footer className="feedback-footer">
      <span>{pendingIds.length} pending annotation{pendingIds.length === 1 ? "" : "s"}</span>
      <button type="button" onClick={onAcknowledge} disabled={busy} title="Mark these annotations handled without sending them to an agent. Images remain available for the configured retention period.">Acknowledge</button>
      <button type="button" onClick={onSend} disabled={busy}>Send to agent</button>
    </footer> : null}
    {sendResult ? <div className={`feedback-result feedback-result-${sendResult.state}`} role={sendResult.state === "rejected" ? "alert" : "status"}><strong>{sendResult.state.replaceAll("_", " ")}</strong><span>{sendResult.message}</span>{sendResult.target ? <code>{sendResult.target.agent_label}</code> : null}</div> : null}
    {riskPending ? <div className="feedback-risk" role="alert"><strong>Delivery outcome is unknown.</strong><span>Retrying may paste the same browser context twice.</span><button type="button" onClick={onRetryRisk} disabled={busy}>Retry and acknowledge duplicate risk</button></div> : null}
  </div>;
}

function Workbench({ client, state, sessions, selection, controlPaneId, terminalMouseInput, mutations, onSession, onFocus, onRequestControl, onReconnect, onRetry, onRefreshSessions, onMutate, onRetryMutation }: {
  client: CockpitClient; state: SessionState; sessions: SessionSummary[]; selection: Selection; controlPaneId: string | null; terminalMouseInput: boolean; mutations: MutationCoordinatorState;
  onSession: (id: string) => void; onFocus: (request: FocusRequest, location: Selection) => void; onRequestControl: (paneId: string) => void; onReconnect: () => void; onRetry: () => void; onRefreshSessions: () => Promise<void>; onMutate: Mutate; onRetryMutation: (operation: MutationOperation) => void;
}) {
  const snapshot = state.snapshot;
  const spaces = snapshot?.spaces ?? [];
  const selectedSpace = byId(spaces, selection.spaceId);
  const setupParent = selectedSpace?.git ? {
    label: selectedSpace.label,
    repositoryKey: selectedSpace.git.repository_key,
    checkoutPath: selectedSpace.git.checkout_path,
  } : null;
  const allTabs = snapshot?.tabs ?? [];
  const tabs = tabsForSpace(allTabs, selection.spaceId);
  const selectedTab = byId(tabs, selection.tabId);
  const layout = snapshot?.layouts.find((candidate) => candidate.tab_id === selectedTab?.id && candidate.space_id === selection.spaceId);
  const panes = panesForTab(snapshot?.panes ?? [], selectedTab?.id ?? null);
  const visiblePaneIds = projectedPaneIds(panes.map((pane) => pane.id), layout, snapshot?.focused_pane_id ?? selection.paneId);
  const visiblePanes = panes.filter((pane) => visiblePaneIds.includes(pane.id));
  const renderers = usePaneRenderers(client, state.sessionId, visiblePaneIds, (snapshot?.panes ?? []).map((pane) => pane.id), state.sync === "live", state.epoch, onReconnect);
  const [menu, setMenu] = useState<ContextMenuState | null>(null);
  const [editing, setEditing] = useState<ContextTarget | null>(null);
  const [dialog, setDialog] = useState<PaneDialog | null>(null);
  const [commandsOpen, setCommandsOpen] = useState(false);
  const [sessionChooserOpen, setSessionChooserOpen] = useState(false);
  const [setupOpen, setSetupOpen] = useState(false);
  const [recoveryOpen, setRecoveryOpen] = useState(false);
  const [teardownSpaceId, setTeardownSpaceId] = useState<string | null>(null);
  const [prefixActive, setPrefixActive] = useState(false);
  const mutationBusy = mutations.pending !== null || renderers.busy;
  const modalOpen = dialog !== null || commandsOpen || sessionChooserOpen || setupOpen || recoveryOpen || teardownSpaceId !== null;
  const [browserError, setBrowserError] = useState<(StatusError & { action: string }) | null>(null);
  const [browserBusy, setBrowserBusy] = useState(false);
  const browserBusyRef = useRef(false);
  const browserRequest = useRef(0);
  const browserTargetRef = useRef<{ sessionId: string; spaceId: string } | null>(null);
  browserTargetRef.current = state.sessionId && selection.spaceId ? { sessionId: state.sessionId, spaceId: selection.spaceId } : null;
  const [feedbackOpen, setFeedbackOpen] = useState(false);
  const [feedbackLookup, setFeedbackLookup] = useState<BrowserFeedbackLookup | null>(null);
  const [feedbackImages, setFeedbackImages] = useState<FeedbackImageState>({});
  const [feedbackSpaceId, setFeedbackSpaceId] = useState<string | null>(null);
  const feedbackImagesRef = useRef<FeedbackImageState>({});
  feedbackImagesRef.current = feedbackImages;
  const [feedbackError, setFeedbackError] = useState<StatusError | null>(null);
  const [feedbackBusy, setFeedbackBusy] = useState(false);
  const [feedbackResult, setFeedbackResult] = useState<BrowserFeedbackSendResponse | null>(null);
  const [feedbackRisk, setFeedbackRisk] = useState<{ ids: string[] } | null>(null);
  const feedbackBusyRef = useRef(false);
  const feedbackRequest = useRef(0);
  const feedbackTargetRef = useRef<{ sessionId: string; spaceId: string } | null>(null);
  feedbackTargetRef.current = state.sessionId && selection.spaceId ? { sessionId: state.sessionId, spaceId: selection.spaceId } : null;
  const streamRegistry = useRef(new Set<TerminalStream>());
  const registerStream = useCallback((stream: TerminalStream, active: boolean) => { if (active) streamRegistry.current.add(stream); else streamRegistry.current.delete(stream); }, []);
  useEffect(() => () => { streamRegistry.current.forEach((stream) => stream.close()); streamRegistry.current.clear(); }, []);
  const focusSpace = (space: Space) => {
    if (modalOpen) return;
    const tabId = allTabs.find((tab) => tab.space_id === space.id && tab.focused)?.id ?? null;
    const paneId = snapshot?.panes.find((pane) => pane.space_id === space.id && pane.focused)?.id ?? null;
    onFocus({ kind: "space", target_id: space.id }, { spaceId: space.id, tabId, paneId });
  };
  const focusTab = (tab: Tab) => {
    if (modalOpen) return;
    const paneId = snapshot?.panes.find((pane) => pane.tab_id === tab.id && pane.focused)?.id ?? null;
    onFocus({ kind: "tab", target_id: tab.id }, { spaceId: tab.space_id, tabId: tab.id, paneId });
  };
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
  const browserAction = useCallback(async (spaceId: string, action: "open" | "show" | "close", url?: string) => {
    const sessionId = state.sessionId;
    if (!sessionId || state.sync !== "live" || browserBusyRef.current || browserTargetRef.current?.sessionId !== sessionId || browserTargetRef.current.spaceId !== spaceId) return;
    const token = ++browserRequest.current;
    browserBusyRef.current = true;
    setBrowserBusy(true);
    setBrowserError(null);
    const target = { session_id: sessionId, space_id: spaceId, pane_id: null, endpoint_path: null };
    const current = () => browserRequest.current === token && browserTargetRef.current?.sessionId === sessionId && browserTargetRef.current.spaceId === spaceId;
    try {
      let response = await client.browserAction({ target, action: action === "open" ? { kind: "open", url: url ?? null } : { kind: action } });
      if (!current()) return;
      if (response.association && (response.association.session_id !== sessionId || response.association.space_id !== spaceId)) throw new Error("Browser response belongs to another Space");
      if (action === "open" && response.connection === "open") {
        response = await client.browserAction({ target, action: { kind: "show" } });
        if (!current()) return;
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
  }, [client, state.sessionId, state.sync]);
  useEffect(() => {
    browserRequest.current += 1;
    setBrowserError(null);
    setBrowserBusy(false);
    browserBusyRef.current = false;
    return () => { browserRequest.current += 1; };
  }, [browserAction, selection.spaceId, state.sessionId, state.sync]);
  const loadFeedback = useCallback(async (spaceId: string, quiet = false) => {
    const sessionId = state.sessionId;
    if (!sessionId || state.sync !== "live" || feedbackBusyRef.current
      || feedbackTargetRef.current?.sessionId !== sessionId || feedbackTargetRef.current.spaceId !== spaceId) return;
    const token = ++feedbackRequest.current;
    feedbackBusyRef.current = true;
    if (!quiet) { setFeedbackBusy(true); setFeedbackError(null); }
    const target = { session_id: sessionId, space_id: spaceId, pane_id: null, endpoint_path: null };
    const current = () => feedbackRequest.current === token && feedbackTargetRef.current?.sessionId === sessionId && feedbackTargetRef.current.spaceId === spaceId;
    try {
      const lookup = await client.browserFeedback({ target });
      if (!current()) return;
      if (lookup.browser.association && (lookup.browser.association.session_id !== sessionId || lookup.browser.association.space_id !== spaceId)) throw new Error("Feedback response belongs to another Space");
      setFeedbackSpaceId(spaceId);
      setFeedbackLookup(lookup);
      setFeedbackError(null);
      await Promise.all(lookup.feedback.captures.map(async (capture) => {
        if (feedbackImagesRef.current[capture.id]) return;
        try {
          const image = await client.browserFeedbackImage({ target, capture_id: capture.id });
          if (current()) setFeedbackImages((previous) => ({ ...previous, [capture.id]: image }));
        } catch {
          if (current()) setFeedbackImages((previous) => ({ ...previous, [capture.id]: null }));
        }
      }));
    } catch (error) {
      if (current()) {
        setFeedbackSpaceId(spaceId);
        setFeedbackError(describeError(error, "Could not read browser feedback"));
      }
    } finally {
      if (current()) {
        feedbackBusyRef.current = false;
        setFeedbackBusy(false);
      }
    }
  }, [client, state.sessionId, state.sync]);
  const openFeedback = useCallback(() => {
    if (selection.spaceId && state.sync === "live") {
      setFeedbackOpen(true);
      void loadFeedback(selection.spaceId);
    }
  }, [loadFeedback, selection.spaceId, state.sync]);
  const refreshFeedback = useCallback(() => {
    if (selection.spaceId) void loadFeedback(selection.spaceId);
  }, [loadFeedback, selection.spaceId]);
  const displayedFeedbackIds = feedbackLookup && feedbackSpaceId === selection.spaceId
    ? Array.from(new Set(feedbackLookup.feedback.captures.flatMap((capture) => capture.pending_ids))) : [];
  const sendFeedback = useCallback(async (ids: string[], operationId: string, acknowledgeDuplicateRisk: boolean) => {
    const sessionId = state.sessionId;
    const spaceId = selection.spaceId;
    if (!sessionId || !spaceId || state.sync !== "live" || feedbackBusyRef.current
      || feedbackTargetRef.current?.sessionId !== sessionId || feedbackTargetRef.current.spaceId !== spaceId) return;
    const token = ++feedbackRequest.current;
    feedbackBusyRef.current = true;
    setFeedbackBusy(true);
    setFeedbackError(null);
    const target = { session_id: sessionId, space_id: spaceId, pane_id: null, endpoint_path: null };
    try {
      const response = await client.sendBrowserFeedback({ target, ids, operation_id: operationId, acknowledge_duplicate_risk: acknowledgeDuplicateRisk });
      if (feedbackRequest.current !== token || feedbackTargetRef.current?.sessionId !== sessionId || feedbackTargetRef.current.spaceId !== spaceId) return;
      setFeedbackResult(response);
      setFeedbackRisk(response.state === "outcome_unknown" ? { ids } : null);
    } catch (error) {
      if (feedbackRequest.current === token) setFeedbackError(describeError(error, "Could not send browser feedback"));
    } finally {
      if (feedbackRequest.current === token) {
        feedbackBusyRef.current = false;
        setFeedbackBusy(false);
        void loadFeedback(spaceId);
      }
    }
  }, [client, loadFeedback, selection.spaceId, state.sessionId, state.sync]);
  const acknowledgeFeedback = useCallback(async () => {
    const sessionId = state.sessionId;
    const spaceId = selection.spaceId;
    const ids = displayedFeedbackIds;
    if (!sessionId || !spaceId || ids.length === 0 || state.sync !== "live" || feedbackBusyRef.current) return;
    const token = ++feedbackRequest.current;
    feedbackBusyRef.current = true;
    setFeedbackBusy(true);
    setFeedbackError(null);
    try {
      await client.acknowledgeBrowserFeedback({ target: { session_id: sessionId, space_id: spaceId, pane_id: null, endpoint_path: null }, ids });
    } catch (error) {
      if (feedbackRequest.current === token) setFeedbackError(describeError(error, "Could not acknowledge browser feedback"));
    } finally {
      if (feedbackRequest.current === token) {
        feedbackBusyRef.current = false;
        setFeedbackBusy(false);
        void loadFeedback(spaceId);
      }
    }
  }, [client, displayedFeedbackIds, loadFeedback, selection.spaceId, state.sessionId, state.sync]);
  const sendDisplayedFeedback = useCallback(() => {
    if (displayedFeedbackIds.length === 0) return;
    const operationId = globalThis.crypto?.randomUUID?.() ?? `browser-feedback-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    void sendFeedback(displayedFeedbackIds, operationId, false);
  }, [displayedFeedbackIds, sendFeedback]);
  const retryUnknownFeedback = useCallback(() => {
    if (!feedbackRisk) return;
    const operationId = globalThis.crypto?.randomUUID?.() ?? `browser-feedback-retry-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    void sendFeedback(feedbackRisk.ids, operationId, true);
  }, [feedbackRisk, sendFeedback]);
  useEffect(() => {
    feedbackRequest.current += 1;
    feedbackBusyRef.current = false;
    setFeedbackOpen(false);
    setFeedbackLookup(null);
    setFeedbackImages({});
    setFeedbackSpaceId(null);
    setFeedbackError(null);
    setFeedbackResult(null);
    setFeedbackRisk(null);
  }, [selection.spaceId, state.sessionId, state.sync]);
  useEffect(() => {
    if (!feedbackOpen || !selection.spaceId || state.sync !== "live") return;
    const timer = window.setInterval(() => { if (document.visibilityState === "visible") void loadFeedback(selection.spaceId!, true); }, 5000);
    return () => window.clearInterval(timer);
  }, [feedbackOpen, loadFeedback, selection.spaceId, state.sync]);
  const runCommand = useCallback((command: PrefixCommand) => {
    const space = byId(spaces, selection.spaceId);
    const tab = byId(tabs, selection.tabId);
    const pane = byId(panes, selection.paneId);
    if (command === "help") { setCommandsOpen(true); return; }
    if (mutationBusy && !["previous-tab", "next-tab", "previous-pane", "next-pane", "focus-left", "focus-right", "focus-up", "focus-down", "resize"].includes(command)) return;
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
    if (command.startsWith("select-tab-")) { const target = tabs[Number(command.slice("select-tab-".length)) - 1]; if (target) focusTab(target); }
    if (command === "previous-pane" && pane) { const index = panes.indexOf(pane); focusPane(panes[(index - 1 + panes.length) % panes.length]); }
    if (command === "next-pane" && pane) { const index = panes.indexOf(pane); focusPane(panes[(index + 1) % panes.length]); }
    if (["focus-left", "focus-right", "focus-up", "focus-down"].includes(command)) {
      const direction = command.slice("focus-".length) as PaneFocusDirection;
      const target = byId(panes, paneIdInDirection(layout, selection.paneId, direction));
      if (target) focusPane(target);
    }
    if (command === "open-file-picker") dispatchFileNavigation("open-picker");
    if (command === "focus-file-tree") dispatchFileNavigation("focus-tree");
    if (command === "focus-file-content") dispatchFileNavigation("focus-content");
    if (command === "resize" && !mutationBusy) document.querySelector<HTMLElement>(".resize-handle")?.focus();
  }, [spaces, tabs, panes, layout, selection.spaceId, selection.tabId, selection.paneId, mutationBusy, modalOpen]);
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      routeWorkbenchKeydown(event, { modalOpen, prefixActive, runCommand, setPrefixActive, setCommandsOpen });
    };
    window.addEventListener("keydown", keydown, true);
    return () => window.removeEventListener("keydown", keydown, true);
  }, [prefixActive, runCommand, modalOpen]);
  const openContext = (event: MouseEvent, target: ContextTarget) => { event.preventDefault(); event.stopPropagation(); if (!mutationBusy && !modalOpen) setMenu({ target, x: event.clientX, y: event.clientY }); };
  const openSelectedPaneMenu = (event: MouseEvent<HTMLButtonElement>) => {
    const pane = byId(panes, selection.paneId);
    if (!pane || mutationBusy || modalOpen) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    setMenu({ target: { kind: "pane", id: pane.id }, x: bounds.left, y: bounds.bottom });
  };
  const dismissMenu = useCallback(() => setMenu(null), []);
  const menuAction = (action: () => boolean | void) => { if (action() !== false) dismissMenu(); };
  const renderMenu = () => {
    if (!menu) return null;
    const disabled = mutationBusy;
    if (menu.target.kind === "space") {
      const space = spaces.find((candidate) => candidate.id === menu.target.id);
      if (!space) return null;
      return <ContextMenu menu={menu} onDismiss={dismissMenu}><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}>Rename</button><button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeSpace(space))}>Close</button>{space.id === selection.spaceId ? <><button role="menuitem" type="button" disabled={disabled || browserBusy} onClick={() => menuAction(() => { void browserAction(space.id, "open"); })}>Open browser</button><button role="menuitem" type="button" disabled={disabled || browserBusy} onClick={() => menuAction(() => { void browserAction(space.id, "show"); })}>Show browser</button><button role="menuitem" type="button" disabled={disabled || feedbackBusy} onClick={() => menuAction(openFeedback)}>Browser feedback</button><button role="menuitem" type="button" disabled={disabled || browserBusy} className="destructive" onClick={() => menuAction(() => { void browserAction(space.id, "close"); })}>Close browser</button></> : null}<button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => setTeardownSpaceId(space.id))}>Review task cleanup…</button></ContextMenu>;
    }
    if (menu.target.kind === "tab") {
      const tab = allTabs.find((candidate) => candidate.id === menu.target.id);
      if (!tab) return null;
      return <ContextMenu menu={menu} onDismiss={dismissMenu}><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}>Rename</button><button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeTab(tab))}>Close</button></ContextMenu>;
    }
    const pane = snapshot?.panes.find((candidate) => candidate.id === menu.target.id);
    if (!pane) return null;
    const renderer = renderers.panes[pane.id];
    return <ContextMenu menu={menu} onDismiss={dismissMenu}>
      <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}>Rename</button>
      <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => onMutate(`pane:${pane.id}`, { type: "pane_split", pane_id: pane.id, direction: "right", ratio: null }, true))}>Split right</button>
      <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => onMutate(`pane:${pane.id}`, { type: "pane_split", pane_id: pane.id, direction: "down", ratio: null }, true))}>Split down</button>
      <button role="menuitem" type="button" disabled={disabled || !renderer?.presentation.can_open_review} title={renderer?.presentation.reason} onClick={() => menuAction(() => { void renderers.open(pane.id, "right", "review"); })}>Open Review right</button>
      <button role="menuitem" type="button" disabled={disabled || !renderer?.presentation.can_open_review} title={renderer?.presentation.reason} onClick={() => menuAction(() => { void renderers.open(pane.id, "down", "review"); })}>Open Review below</button>
      <button role="menuitem" type="button" disabled={disabled || !renderer?.presentation.can_open_files} title={renderer?.presentation.reason} onClick={() => menuAction(() => { void renderers.open(pane.id, "right", "files"); })}>Open files right</button>
      <button role="menuitem" type="button" disabled={disabled || !renderer?.presentation.can_open_files} title={renderer?.presentation.reason} onClick={() => menuAction(() => { void renderers.open(pane.id, "down", "files"); })}>Open files below</button>
      <button role="menuitem" type="button" disabled={disabled || !renderer?.presentation.can_open_context} title={renderer?.presentation.reason} onClick={() => menuAction(() => { void renderers.open(pane.id, "right"); })}>Open Context right</button>
      <button role="menuitem" type="button" disabled={disabled || !renderer?.presentation.can_open_context} title={renderer?.presentation.reason} onClick={() => menuAction(() => { void renderers.open(pane.id, "down"); })}>Open Context below</button>
      <button role="menuitem" type="button" disabled={!renderer?.presentation.renderer} title={renderer?.presentation.reason} onClick={() => menuAction(() => renderers.choose(pane.id, (isGraphicalContext(renderer) || isGraphicalReview(renderer)) ? "terminal" : renderer?.presentation.renderer ?? "context"))}>{isGraphicalContext(renderer) || isGraphicalReview(renderer) ? "Show terminal view" : renderer?.presentation.renderer === "review" ? "Render as Review" : "Render as Context"}</button>
      <button role="menuitem" type="button" onClick={() => menuAction(renderers.refresh)}>Refresh renderer detection</button>
      <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => onMutate(`pane:${pane.id}`, { type: "pane_zoom", pane_id: pane.id, mode: "toggle" }))}>Toggle zoom</button>
      <button role="menuitem" type="button" disabled={disabled || panes.length < 2} onClick={() => menuAction(() => { setDialog({ kind: "swap", paneId: pane.id }); return true; })}>Swap...</button>
      <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => { setDialog({ kind: "move", paneId: pane.id }); return true; })}>Move...</button>
      <button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closePane(pane))}>Close</button>
    </ContextMenu>;
  };
  return <div className="workbench">
    <aside className="sidebar"><Spaces spaces={spaces} selectedSpaceId={selection.spaceId} editingId={editing?.kind === "space" ? editing.id : null} busy={mutationBusy} onEdit={(id) => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "space", id } : null); }} onSelect={focusSpace} onContext={openContext} onSetup={() => setSetupOpen(true)} setupEnabled={state.sync === "live" && !modalOpen} mutate={onMutate} /><Agents agents={snapshot?.agents ?? []} spaces={spaces} tabs={allTabs} selection={selection} onSelect={(agent) => { if (!modalOpen) onFocus({ kind: "agent", target_id: agent.pane_id }, { spaceId: agent.space_id, tabId: agent.tab_id, paneId: agent.pane_id }); }} /></aside>
    <main className="main-workarea">
      {selection.spaceId ? <TabStrip tabs={tabs} selectedTabId={selection.tabId} editingId={editing?.kind === "tab" ? editing.id : null} busy={mutationBusy} hasSelectedPane={Boolean(byId(panes, selection.paneId))} onEdit={(id) => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "tab", id } : null); }} onSelect={focusTab} onContext={openContext} onCreate={() => { if (selection.spaceId) onMutate("tab:new", { type: "tab_create", space_id: selection.spaceId, label: null }, true); }} onPaneMenu={openSelectedPaneMenu} onCommands={() => setCommandsOpen(true)} mutate={onMutate} /> : null}
      <div className="pane-canvas">{panes.length === 0 ? <div className="empty-main"><strong>No panes</strong><span>Create a tab or select another space.</span></div> : visiblePanes.map((pane, index) => {
        const rectangle = projectedPaneRect(layout, pane.id);
        const area = layout?.area;
        const style = rectangle && area && area.width > 0 && area.height > 0 ? { left: `${(rectangle.x - area.x) / area.width * 100}%`, top: `${(rectangle.y - area.y) / area.height * 100}%`, width: `${rectangle.width / area.width * 100}%`, height: `${rectangle.height / area.height * 100}%` } : { left: `${index / visiblePanes.length * 100}%`, top: "0%", width: `${100 / visiblePanes.length}%`, height: "100%" };
        const renderer = renderers.panes[pane.id];
        const controlPendingForPane = state.focusPending !== null
          && (state.focusPending.kind === "pane" || state.focusPending.kind === "agent"
            ? state.focusPending.target_id === pane.id
            : state.focusPending.kind === "tab"
              ? state.focusPending.target_id === pane.tab_id
              : state.focusPending.kind === "space" && state.focusPending.target_id === pane.space_id);
        return <PaneView key={pane.id} pane={pane} label={pane.title ?? `Pane ${panes.indexOf(pane) + 1}`} selected={pane.id === selection.paneId} showLabel={panes.length > 1} controlAllowed={state.sync === "live" && pane.id === controlPaneId && pane.id === snapshot?.focused_pane_id && !state.focusPending && !state.focusError} controlPending={controlPendingForPane} focusEpoch={state.epoch} focusToken={state.focusToken} terminalMouseInput={terminalMouseInput} onRequestControl={() => { if (!modalOpen && state.sync === "live") { if (pane.id !== snapshot?.focused_pane_id || (state.focusError && pane.id === selection.paneId)) focusPane(pane); else onRequestControl(pane.id); } }} onSelect={() => focusPane(pane)} onContext={openContext} request={{ session_id: state.sessionId!, pane_id: pane.id }} client={client} registerStream={registerStream} onResync={onReconnect} mutate={onMutate} style={style} renderer={renderer} onRendererViewChange={(bindingId, value) => renderers.updateView(pane.id, bindingId, value)} onTerminalView={() => renderers.choose(pane.id, "terminal")} onRefreshRenderer={renderers.refresh} />;
      })}{mutationBusy ? null : <ResizeHandles layout={layout} mutate={onMutate} />}</div>
    </main>
    {renderMenu()}
    {dialog ? <PaneDialogOverlay dialog={dialog} panes={panes} tabs={allTabs} spaces={spaces} busy={mutationBusy} onDismiss={() => setDialog(null)} mutate={onMutate} /> : null}
    {commandsOpen ? <CommandOverlay run={(command) => { setCommandsOpen(false); runCommand(command); }} onSwitchSession={() => { setCommandsOpen(false); void onRefreshSessions().catch(() => undefined).finally(() => setSessionChooserOpen(true)); }} onDismiss={() => setCommandsOpen(false)} contextActions={<>
      <button type="button" className="session-command" onClick={() => { setCommandsOpen(false); setRecoveryOpen(true); }}>Recover task cleanup…</button>
      {browserBusy ? <p role="status">Working on the Space browser…</p> : null}
      {browserError ? <p role="alert">{browserError.message}</p> : null}
      <button type="button" className="session-command" disabled={!selection.spaceId || browserBusy || state.sync !== "live"} onClick={() => { if (selection.spaceId) void browserAction(selection.spaceId, "open"); }}>Open browser for Space</button>
      <button type="button" className="session-command" disabled={!selection.spaceId || browserBusy || state.sync !== "live"} onClick={() => { if (selection.spaceId) void browserAction(selection.spaceId, "show"); }}>Show browser for Space</button>
      <button type="button" className="session-command" disabled={!selection.spaceId || browserBusy || state.sync !== "live"} onClick={() => { if (selection.spaceId) void browserAction(selection.spaceId, "close"); }}>Close browser for Space</button>
      <button type="button" className="session-command" disabled={!selection.spaceId || feedbackBusy || state.sync !== "live"} onClick={() => { setCommandsOpen(false); openFeedback(); }}>Browser feedback{feedbackLookup && feedbackSpaceId === selection.spaceId && feedbackLookup.feedback.pending_count > 0 ? ` · ${feedbackLookup.feedback.pending_count}` : ""}</button>
      <button type="button" className="session-command" disabled={!selection.spaceId || feedbackBusy || state.sync !== "live"} onClick={() => { setCommandsOpen(false); setFeedbackOpen(true); void sendFeedback([], globalThis.crypto?.randomUUID?.() ?? `browser-context-${Date.now()}-${Math.random().toString(36).slice(2)}`, false); }}>Send browser context</button>
      <button type="button" className="session-command" disabled={mutationBusy || !renderers.panes[selection.paneId ?? ""]?.presentation.can_open_review} title={renderers.panes[selection.paneId ?? ""]?.presentation.reason} onClick={() => { setCommandsOpen(false); if (selection.paneId) void renderers.open(selection.paneId, "right", "review"); }}>Open Review right</button>
      <button type="button" className="session-command" disabled={mutationBusy || !renderers.panes[selection.paneId ?? ""]?.presentation.can_open_review} title={renderers.panes[selection.paneId ?? ""]?.presentation.reason} onClick={() => { setCommandsOpen(false); if (selection.paneId) void renderers.open(selection.paneId, "down", "review"); }}>Open Review below</button>
      <button type="button" className="session-command" disabled={mutationBusy || !renderers.panes[selection.paneId ?? ""]?.presentation.can_open_files} title={renderers.panes[selection.paneId ?? ""]?.presentation.reason} onClick={() => { setCommandsOpen(false); if (selection.paneId) void renderers.open(selection.paneId, "right", "files"); }}>Open files right</button>
      <button type="button" className="session-command" disabled={mutationBusy || !renderers.panes[selection.paneId ?? ""]?.presentation.can_open_files} title={renderers.panes[selection.paneId ?? ""]?.presentation.reason} onClick={() => { setCommandsOpen(false); if (selection.paneId) void renderers.open(selection.paneId, "down", "files"); }}>Open files below</button>
      <button type="button" className="session-command" disabled={mutationBusy || !renderers.panes[selection.paneId ?? ""]?.presentation.can_open_context} onClick={() => { setCommandsOpen(false); if (selection.paneId) void renderers.open(selection.paneId, "right"); }}>Open Context right</button>
      <button type="button" className="session-command" disabled={mutationBusy || !renderers.panes[selection.paneId ?? ""]?.presentation.can_open_context} onClick={() => { setCommandsOpen(false); if (selection.paneId) void renderers.open(selection.paneId, "down"); }}>Open Context below</button>
      <button type="button" className="session-command" disabled={!renderers.panes[selection.paneId ?? ""]?.presentation.renderer} title={renderers.panes[selection.paneId ?? ""]?.presentation.reason} onClick={() => { setCommandsOpen(false); if (selection.paneId) renderers.choose(selection.paneId, (isGraphicalContext(renderers.panes[selection.paneId]) || isGraphicalReview(renderers.panes[selection.paneId])) ? "terminal" : renderers.panes[selection.paneId]?.presentation.renderer ?? "context"); }}>{isGraphicalContext(renderers.panes[selection.paneId ?? ""]) || isGraphicalReview(renderers.panes[selection.paneId ?? ""]) ? "Show terminal view" : renderers.panes[selection.paneId ?? ""]?.presentation.renderer === "review" ? "Render as Review" : "Render as Context"}</button>
    </>} /> : null}
    {sessionChooserOpen ? <SessionDialogOverlay sessions={sessions} currentSessionId={state.sessionId} onRefresh={onRefreshSessions} onSession={onSession} onDismiss={() => setSessionChooserOpen(false)} /> : null}
    {state.sessionId ? <SetupDialog client={client} sessionId={state.sessionId} open={setupOpen} selectedParent={setupParent} onClose={() => setSetupOpen(false)} onCompleted={onReconnect} /> : null}
    {state.sessionId ? <TeardownRecoveryPanel client={client} sessionId={state.sessionId} open={recoveryOpen} onClose={() => setRecoveryOpen(false)} /> : null}
    {state.sessionId && teardownSpaceId ? <TeardownDialog client={client} sessionId={state.sessionId} workspaceId={teardownSpaceId} open onClose={() => setTeardownSpaceId(null)} onCompleted={onReconnect} /> : null}
    {feedbackOpen && (feedbackSpaceId === selection.spaceId || feedbackLookup === null) ? <FeedbackPanel lookup={feedbackLookup} images={feedbackImages} busy={feedbackBusy} error={feedbackError} sendResult={feedbackResult} riskPending={feedbackRisk !== null} onRefresh={refreshFeedback} onAcknowledge={() => { void acknowledgeFeedback(); }} onSend={sendDisplayedFeedback} onRetryRisk={retryUnknownFeedback} onDismiss={() => setFeedbackOpen(false)} /> : null}
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
  const [selection, setSelection] = useState<Selection>({ spaceId: null, tabId: null, paneId: null });
  const [controlPaneId, setControlPaneId] = useState<string | null>(null);
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
  const { focus, reconcile: reconcileFocus, reset: resetFocus, retryFocus, tokenRef: focusTokenRef } = useFocusCoordinator({
    client,
    stateRef,
    mountedRef,
    dispatch,
    describeError,
    onTimeout: requestResync,
  });
  const focusAndSelect = useCallback((request: FocusRequest, location: Selection) => {
    setSelection(location);
    setControlPaneId(location.paneId);
    focus(request, location);
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
    let active = true;
    sessionStream.current?.close();
    sessionStream.current = null;
    dispatch({ type: "snapshot/request", epoch, sessionId });
    void (async () => {
      try {
        const snapshot = await client.sessionSnapshot(sessionId);
        if (!active || sessionObservation.current !== observation) return;
        dispatch({ type: "snapshot/received", epoch, sessionId, snapshot });
        const stream = await client.subscribeSession(sessionId, (message: SessionStreamMessage) => {
          if (!active || sessionObservation.current !== observation) return;
          if (recovering && message.type === "snapshot" && message.sequence === 1 && recoveryFocusToken === focusTokenRef.current && stateRef.current.focusError) {
            const confirmedFocus = focusRequestForSnapshot(message.snapshot);
            if (confirmedFocus) {
              const token = ++focusTokenRef.current;
              dispatch({ type: "focus/request", epoch, sessionId, request: confirmedFocus, token });
            }
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
          if (!active || sessionObservation.current !== observation) return;
          const described = describeError(error, "Session stream disconnected");
          dispatch({ type: "stream/error", epoch, sessionId, code: described.code ?? "stream_disconnected", message: described.message });
        });
        if (active && sessionObservation.current === observation) sessionStream.current = stream; else stream.close();
      } catch (error: unknown) {
        if (!active || sessionObservation.current !== observation) return;
        const described = describeError(error, "Could not read the session snapshot");
        dispatch({ type: "stream/error", epoch, sessionId, code: described.code ?? "snapshot_error", message: described.message });
      }
    })();
    return () => { active = false; sessionStream.current?.close(); sessionStream.current = null; };
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

  const explicitResync = () => {
    clearRecoveryTimers();
    autoResyncAttempts.current = 0;
    void refreshSessions();
    requestResync();
  };
  if (!status || !compatible) return <div className="app-shell">{statusError || (status && !compatible) ? <CompatibilityNotice status={status} error={statusError} retry={() => setStatusAttempt((value) => value + 1)} /> : <main className="compatibility-main" aria-live="polite"><section className="notice notice-loading" role="status"><p className="eyebrow">Cockpit</p><h1>Connecting to Herdr</h1><p>Reading compatibility status...</p></section></main>}</div>;
  if (sessionsError && sessions.length === 0) return <div className="app-shell"><CompatibilityNotice status={status} error={sessionsError} retry={() => setSessionsAttempt((value) => value + 1)} /></div>;
  if (sessionsLoaded && sessions.length === 0) return <div className="app-shell"><main className="compatibility-main"><section className="notice"><h1>No Herdr sessions</h1><p>Create or start a session, then refresh the list.</p><button type="button" className="action-button" onClick={() => setSessionsAttempt((value) => value + 1)}>Refresh sessions</button></section></main></div>;
  return <div className="app-shell"><Workbench key={state.epoch} client={client} state={state} sessions={sessions} selection={selection} controlPaneId={controlPaneId} terminalMouseInput={status.capabilities.terminal_mouse_input} mutations={mutations} onSession={switchSession} onFocus={focusAndSelect} onRequestControl={setControlPaneId} onReconnect={explicitResync} onRetry={retryFocus} onRefreshSessions={refreshSessions} onMutate={mutate} onRetryMutation={retryMutation} /></div>;
}
