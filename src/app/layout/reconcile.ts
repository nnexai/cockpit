import type { CreatedPane } from "../../protocol/generated/v1";
import { compareStablePaneId, findNode, firstLoadGrid, insertRootEdge, leaves, removeLeaf, splitLeaf } from "./splitTree";
import { createSessionLayoutState, selectTabLeaf, type FocusTriple, type SessionLayoutState, type TabLayoutState } from "./tabLayoutStore";

export type FocusEcho = { token: number; kind: "space" | "tab" | "pane" | "agent"; targetId: string };
export type FocusClass = "initial" | "unchanged" | "echo" | "external";
export type LayoutEffect =
  | { type: "browser-retire"; tabId: string; associationKey: string | null; serverInstance: string }
  | { type: "viewer-release"; tabId: string; kind: "files" | "review"; viewerId: string }
  | { type: "cancel-transient"; reason: "external-focus" | "leaf-removed" | "tab-hidden" }
  | { type: "consume-echo"; token: number }
  | { type: "announce"; text: string };
export type LayoutSnapshot = {
  session_id?: string; server_instance: string;
  focused_space_id: string | null; focused_tab_id: string | null; focused_pane_id: string | null;
  tabs: { id: string; space_id: string; focused_pane_id: string | null }[];
  panes: { id: string; terminal_id: string; tab_id: string; space_id: string }[];
};
export function sameFocus(a: FocusTriple | null, b: FocusTriple | null): boolean {
  return a === b || !!a && !!b && a.spaceId === b.spaceId && a.tabId === b.tabId && a.paneId === b.paneId;
}
function matches(echo: FocusEcho, next: FocusTriple): boolean {
  return echo.targetId === (echo.kind === "space" ? next.spaceId : echo.kind === "tab" ? next.tabId : next.paneId);
}
export function classifyFocus(prev: FocusTriple | null, next: FocusTriple, echoes: readonly FocusEcho[], _state: SessionLayoutState): FocusClass {
  if (!prev) return "initial";
  if (sameFocus(prev, next)) return "unchanged";
  return echoes.some(echo => matches(echo, next)) ? "echo" : "external";
}

/** Runtime source and placement are deliberately separate. */
export function runtimeSource(tab: TabLayoutState, targetLeafId: string | null = tab.selectedLeafId, focusedPaneId: string | null = tab.focusedPaneId): string | null {
  if (targetLeafId && tab.terminals[targetLeafId]) return targetLeafId;
  if (tab.lastRealLeafId && tab.terminals[tab.lastRealLeafId]) return tab.lastRealLeafId;
  if (focusedPaneId && tab.terminals[focusedPaneId]) return focusedPaneId;
  return leaves(tab.root).find(leaf => leaf.kind === "terminal" && tab.terminals[leaf.id])?.id ?? null;
}

function retirement(tab: TabLayoutState, serverInstance: string): LayoutEffect[] {
  const effects: LayoutEffect[] = [];
  for (const kind of ["files", "review"] as const) {
    const context = tab.viewers[kind]?.context;
    if (context) effects.push({ type: "viewer-release", tabId: tab.tabId, kind, viewerId: context.viewer_id });
  }
  if (tab.viewers.browser) effects.push({ type: "browser-retire", tabId: tab.tabId,
    associationKey: tab.viewers.browser.association?.association_key ?? null, serverInstance });
  return effects;
}

function repair(tab: TabLayoutState): TabLayoutState {
  const member = (id: string | null) => !!id && !!findNode(tab.root, id);
  let next = tab;
  if (!member(next.selectedLeafId)) {
    const first = leaves(next.root)[0]?.id ?? null;
    next = { ...next, selectedLeafId: first };
    if (first) next = selectTabLeaf(next, first);
  }
  if (!member(next.zoomLeafId)) next = { ...next, zoomLeafId: null };
  if (!next.lastRealLeafId || !next.terminals[next.lastRealLeafId]) next = { ...next, lastRealLeafId: null };
  return next;
}

function followExternal(state: SessionLayoutState, focus: FocusTriple, effects: LayoutEffect[]): SessionLayoutState {
  const tab = focus.tabId ? state.tabs[focus.tabId] : undefined;
  let next: SessionLayoutState = { ...state, activeSpaceId: focus.spaceId, activeTabId: focus.tabId, focusIntents: {} };
  if (tab && focus.paneId && tab.terminals[focus.paneId]) {
    const restored = tab.zoomLeafId && tab.zoomLeafId !== focus.paneId;
    next = { ...next, tabs: { ...next.tabs, [tab.tabId]: selectTabLeaf(tab, focus.paneId) } };
    effects.push({ type: "announce", text: restored ? `Layout restored: focus moved to ${focus.paneId} from Herdr.` : `Focus moved to ${focus.paneId} from Herdr.` });
  }
  effects.push({ type: "cancel-transient", reason: "external-focus" });
  return next;
}

export function reconcileSnapshot(state: SessionLayoutState, snapshot: LayoutSnapshot, echoes: readonly FocusEcho[] = []): { state: SessionLayoutState; effects: LayoutEffect[] } {
  const effects: LayoutEffect[] = [];
  if (snapshot.session_id && snapshot.session_id !== state.sessionId) return { state, effects };
  let next = state;
  if (snapshot.server_instance !== state.serverInstance) {
    for (const tab of Object.values(state.tabs)) effects.push(...retirement(tab, state.serverInstance));
    if (Object.keys(state.tabs).length) effects.push({ type: "cancel-transient", reason: "leaf-removed" });
    next = { ...createSessionLayoutState(state.sessionId, snapshot.server_instance), cleanupNotices: state.cleanupNotices };
  }
  const tabs: Record<string, TabLayoutState> = {};
  const grouped: Record<string, LayoutSnapshot["panes"]> = {};
  for (const pane of snapshot.panes) (grouped[pane.tab_id] ??= []).push(pane);
  for (const summary of snapshot.tabs) {
    const members = (grouped[summary.id] ?? []).filter(pane => pane.space_id === summary.space_id)
      .sort((a, b) => compareStablePaneId(a.id, b.id));
    let tab = next.tabs[summary.id];
    if (!members.length) {
      if (tab) { effects.push(...retirement(tab, next.serverInstance), { type: "cancel-transient", reason: "leaf-removed" }); }
      continue;
    }
    const knownFocus = snapshot.focused_tab_id === summary.id && members.some(pane => pane.id === snapshot.focused_pane_id)
      ? snapshot.focused_pane_id : summary.focused_pane_id;
    if (!tab) {
      const terminalLeaves = members.map(pane => ({ t: "leaf" as const, id: pane.id, kind: "terminal" as const, w: 1 }));
      const selected = members.some(pane => pane.id === knownFocus) ? knownFocus : members[0].id;
      tabs[summary.id] = { tabId: summary.id, spaceId: summary.space_id, root: firstLoadGrid(terminalLeaves),
        terminals: Object.fromEntries(members.map(pane => [pane.id, pane.terminal_id])), selectedLeafId: selected, lastRealLeafId: selected,
        zoomLeafId: null, viewers: {}, widgetShare: 0.4, heldMembers: [], heldTerminalIds: {}, bufferedFocus: null,
        focusedPaneId: knownFocus, revision: 0, selectionRevision: 0 };
      continue;
    }
    tab = { ...tab, spaceId: summary.space_id, terminals: { ...tab.terminals }, focusedPaneId: knownFocus,
      heldMembers: [], heldTerminalIds: {} };
    const confirmed = Object.fromEntries(members.map(pane => [pane.id, pane.terminal_id]));
    for (const [id, terminalId] of Object.entries(tab.terminals)) {
      if (confirmed[id] === terminalId) continue;
      if (tab.root) {
        const removed = removeLeaf(tab.root, id);
        tab = { ...tab, root: removed.root, revision: tab.revision + 1 };
        if (tab.selectedLeafId === id) tab.selectedLeafId = removed.absorbedBy;
        if (tab.zoomLeafId === id) tab.zoomLeafId = null;
      }
      delete tab.terminals[id];
      effects.push({ type: "cancel-transient", reason: "leaf-removed" });
    }
    for (const pane of members) {
      if (tab.terminals[pane.id] === pane.terminal_id) continue;
      if (next.pendingCreation?.tabId === tab.tabId) {
        tab.heldMembers.push(pane.id);
        tab.heldTerminalIds[pane.id] = pane.terminal_id;
      } else {
        tab.root = insertRootEdge(tab.root, { t: "leaf", id: pane.id, kind: "terminal", w: 1 }, "row", false);
        tab.terminals[pane.id] = pane.terminal_id;
        tab.revision++;
      }
    }
    tabs[summary.id] = repair(tab);
  }
  for (const tab of Object.values(next.tabs)) {
    if (!snapshot.tabs.some(summary => summary.id === tab.tabId)) effects.push(...retirement(tab, next.serverInstance), { type: "cancel-transient", reason: "leaf-removed" });
  }
  next = { ...next, tabs, pendingCreation: next.pendingCreation && tabs[next.pendingCreation.tabId] ? next.pendingCreation : null };
  const focus: FocusTriple = { spaceId: snapshot.focused_space_id, tabId: snapshot.focused_tab_id, paneId: snapshot.focused_pane_id };
  const focusClass = classifyFocus(next.observedFocus, focus, echoes, next);
  const focusedTab = focus.tabId ? tabs[focus.tabId] : undefined;
  if (focusClass !== "unchanged" && focusedTab && focus.paneId && focusedTab.heldMembers.includes(focus.paneId)) {
    next = { ...next, tabs: { ...tabs, [focusedTab.tabId]: { ...focusedTab, bufferedFocus: focus } } };
  } else if (focusClass === "external") {
    next = followExternal(next, focus, effects);
  } else if (focusClass === "initial") {
    next = { ...next, activeSpaceId: focus.spaceId, activeTabId: focus.tabId };
  } else if (focusClass === "echo") {
    const intents = { ...next.focusIntents };
    for (const echo of echoes.filter(echo => matches(echo, focus))) {
      effects.push({ type: "consume-echo", token: echo.token });
      const intent = intents[echo.token];
      delete intents[echo.token];
      const intendedTab = intent?.tabId ? next.tabs[intent.tabId] : undefined;
      const current = intent && intendedTab && intendedTab.selectionRevision === intent.selectionRevision
        && next.activeTabId === intent.activeTabId && next.activeSpaceId === intent.activeSpaceId;
      if (current) {
        if ((echo.kind === "pane" || echo.kind === "agent") && intendedTab.terminals[echo.targetId]) {
          next = { ...next, tabs: { ...next.tabs, [intendedTab.tabId]: selectTabLeaf(intendedTab, echo.targetId) } };
        }
        next = { ...next, activeTabId: focus.tabId, activeSpaceId: focus.spaceId };
      }
    }
    next = { ...next, focusIntents: intents };
  }
  return { state: { ...next, observedFocus: focus }, effects };
}

export function settleCreation(state: SessionLayoutState, token: number, created: CreatedPane | null): { state: SessionLayoutState; effects: LayoutEffect[] } {
  const pending = state.pendingCreation, effects: LayoutEffect[] = [];
  if (!pending || pending.token !== token) return { state, effects };
  let next: SessionLayoutState = { ...state, pendingCreation: null };
  let tab = next.tabs[pending.tabId];
  if (!tab) return { state: next, effects };
  tab = { ...tab, terminals: { ...tab.terminals } };
  const attributed = created && created.tab_id === tab.tabId && created.space_id === tab.spaceId
    && (tab.heldTerminalIds[created.pane_id] ?? tab.terminals[created.pane_id]) === created.terminal_id ? created : null;
  if (attributed && !findNode(tab.root, attributed.pane_id)) {
    const leaf = { t: "leaf" as const, id: attributed.pane_id, kind: "terminal" as const, w: 1 };
    tab.root = tab.root && findNode(tab.root, pending.placeBeside) ? splitLeaf(tab.root, pending.placeBeside, pending.dir, false, leaf)
      : insertRootEdge(tab.root, leaf, "row", false);
    tab.terminals[attributed.pane_id] = attributed.terminal_id;
    tab.revision++;
  }
  for (const id of [...tab.heldMembers].sort(compareStablePaneId)) {
    if (id === attributed?.pane_id) continue;
    tab.root = insertRootEdge(tab.root, { t: "leaf", id, kind: "terminal", w: 1 }, "row", false);
    tab.terminals[id] = tab.heldTerminalIds[id];
    tab.revision++;
  }
  if (attributed && tab.selectionRevision === pending.selectionRevision) tab = selectTabLeaf(tab, attributed.pane_id);
  const buffered = tab.bufferedFocus;
  tab = repair({ ...tab, heldMembers: [], heldTerminalIds: {}, bufferedFocus: null });
  next = { ...next, tabs: { ...next.tabs, [tab.tabId]: tab } };
  // A later ordered focus must win over an older buffered focus.
  if (buffered && sameFocus(buffered, state.observedFocus) && buffered.paneId !== attributed?.pane_id) next = followExternal(next, buffered, effects);
  return { state: next, effects };
}
