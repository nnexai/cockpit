import { useCallback, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { BrowserAssociation, CreatedPane, ViewerContext } from "../../protocol/generated/v1";
import { applyDrop, findNode, removeLeaf, setPairWeights, splitLeaf, type Direction, type LayoutNode, type LeafId } from "./splitTree";
import type { DropTarget } from "./solveLayout";
import { reconcileSnapshot, settleCreation, type FocusEcho, type LayoutEffect, type LayoutSnapshot } from "./reconcile";
export type { LayoutEffect } from "./reconcile";

export type FocusTriple = { spaceId: string | null; tabId: string | null; paneId: string | null };
export type ViewerSelector = { kind: "files_context" | "files_folder" } | { kind: "files_repository"; rootId: string } | { kind: "review"; repositoryId: string };
export type ViewerSlot<Context = ViewerContext, View = unknown> = {
  status: "opening" | "open" | "error"; selector: ViewerSelector; sourcePaneId: string;
  context: Context | null; viewsBySource: Record<string, View>; error: string | null; requestId?: string;
};
export type BrowserSlot = {
  status: "opening" | "open" | "closing" | "close_failed" | "outcome_unknown" | "error";
  association: BrowserAssociation | null; error: string | null; requestId?: string;
};
export type BrowserCleanupNotice = { associationKey: string; tabId: string | null; reason: string };
export type PendingCreation = { token: number; tabId: string; placeBeside: LeafId; dir: Direction; sourcePaneId: string; selectionRevision?: number };
export type TabLayoutState = {
  tabId: string; spaceId: string; root: LayoutNode | null; terminals: Record<string, string>;
  selectedLeafId: LeafId | null; lastRealLeafId: string | null; zoomLeafId: LeafId | null;
  viewers: { files?: ViewerSlot; review?: ViewerSlot; browser?: BrowserSlot };
  heldMembers: string[]; heldTerminalIds: Record<string, string>; bufferedFocus: FocusTriple | null;
  focusedPaneId: string | null; revision: number; selectionRevision: number;
};
export type FocusIntent = FocusEcho & { tabId: string | null; selectionRevision: number; activeTabId: string | null; activeSpaceId: string | null };
export type SessionLayoutState = {
  sessionId: string; serverInstance: string; tabs: Record<string, TabLayoutState>;
  activeSpaceId: string | null; activeTabId: string | null; observedFocus: FocusTriple | null;
  pendingCreation: PendingCreation | null; cleanupNotices: BrowserCleanupNotice[];
  focusIntents: Record<number, FocusIntent>;
};
export type LeafCtx = { client: CockpitClient; sessionId: string; serverInstance: string; clientId: string; getState(): SessionLayoutState; dispatch(action: LayoutAction): void };
export type LayoutAction =
  | { type: "snapshot"; snapshot: LayoutSnapshot; echoes?: readonly FocusEcho[]; sync?: string }
  | { type: "select-leaf"; tabId: string; leafId: LeafId }
  | { type: "activate-tab"; tabId: string }
  | { type: "focus/register"; token: number; kind: FocusEcho["kind"]; targetId: string; tabId?: string }
  | { type: "drop"; tabId: string; src: LeafId; target: DropTarget; revision: number }
  | { type: "resize-commit"; tabId: string; splitId: string; index: number; wa: number; wb: number }
  | { type: "zoom-toggle"; tabId: string; leafId?: LeafId }
  | { type: "creation/begin"; creation: PendingCreation }
  | { type: "creation/settled"; token: number; created: CreatedPane | null }
  | { type: "viewer/open-begin"; tabId: string; kind: "files" | "review"; selector: ViewerSelector; sourcePaneId: string; dir: Direction; placeBeside?: LeafId; requestId?: string }
  | { type: "viewer/opened"; tabId: string; kind: "files" | "review"; context: ViewerContext; requestId?: string }
  | { type: "viewer/failed"; tabId: string; kind: "files" | "review"; error: string; requestId?: string }
  | { type: "viewer/closed"; tabId: string; kind: "files" | "review"; requestId?: string }
  | { type: "viewer/source-view"; tabId: string; kind: "files" | "review"; sourceId: string; view: unknown }
  | { type: "browser/state"; tabId: string; slot: BrowserSlot; dir?: Direction; placeBeside?: LeafId; requestId?: string }
  | { type: "cleanup/notice"; notice: BrowserCleanupNotice }
  | { type: "cleanup/dismiss"; associationKey: string }
  | { type: "leaf/close-local"; tabId: string; leafId: LeafId };
export type LayoutResult = { state: SessionLayoutState; effects: LayoutEffect[] };
export type TabLayouts = {
  state: SessionLayoutState;
  getState(): SessionLayoutState;
  dispatch(action: LayoutAction): void;
  takeEffects(): LayoutEffect[];
};

export function createSessionLayoutState(sessionId: string, serverInstance = ""): SessionLayoutState {
  return { sessionId, serverInstance, tabs: {}, activeSpaceId: null, activeTabId: null, observedFocus: null,
    pendingCreation: null, cleanupNotices: [], focusIntents: {} };
}

export function selectTabLeaf(tab: TabLayoutState, id: string): TabLayoutState {
  const leaf = findNode(tab.root, id);
  if (!leaf || leaf.t !== "leaf") return tab;
  return { ...tab, selectedLeafId: id, lastRealLeafId: leaf.kind === "terminal" ? id : tab.lastRealLeafId,
    zoomLeafId: tab.zoomLeafId && tab.zoomLeafId !== id ? null : tab.zoomLeafId, selectionRevision: tab.selectionRevision + 1 };
}

function closeLocal(tab: TabLayoutState, id: string): TabLayoutState {
  const leaf = findNode(tab.root, id);
  if (!leaf || leaf.t !== "leaf" || leaf.kind === "terminal" || !tab.root) return tab;
  const removed = removeLeaf(tab.root, id), viewers = { ...tab.viewers };
  delete viewers[leaf.kind];
  let next = { ...tab, root: removed.root, viewers, revision: tab.revision + 1,
    zoomLeafId: tab.zoomLeafId === id ? null : tab.zoomLeafId };
  if (next.selectedLeafId === id) {
    next = { ...next, selectedLeafId: null };
    if (removed.absorbedBy) next = selectTabLeaf(next, removed.absorbedBy);
  }
  return next;
}

function insertViewer(tab: TabLayoutState, kind: "files" | "review" | "browser", dir: Direction, target?: LeafId): TabLayoutState {
  const id = `${tab.tabId}:${kind}`;
  if (findNode(tab.root, id)) return tab;
  const beside = target ?? tab.selectedLeafId;
  if (!tab.root || !beside || !findNode(tab.root, beside)) return tab;
  return { ...tab, root: splitLeaf(tab.root, beside, dir, false, { t: "leaf", id, kind, w: 1 }), revision: tab.revision + 1 };
}

export function layoutReducer(state: SessionLayoutState, action: LayoutAction): LayoutResult {
  const effects: LayoutEffect[] = [];
  const result = (next: SessionLayoutState = state): LayoutResult => ({ state: next, effects });
  if (action.type === "snapshot") return action.sync && action.sync !== "live" ? result() : reconcileSnapshot(state, action.snapshot, action.echoes ?? []);
  if (action.type === "creation/settled") return settleCreation(state, action.token, action.created);
  if (action.type === "creation/begin") {
    const tab = state.tabs[action.creation.tabId];
    if (!tab || state.pendingCreation) return result();
    return result({ ...state, pendingCreation: { ...action.creation, selectionRevision: tab.selectionRevision },
      tabs: { ...state.tabs, [tab.tabId]: { ...tab, zoomLeafId: null } } });
  }
  if (action.type === "cleanup/notice") return result({ ...state, cleanupNotices: [...state.cleanupNotices.filter(item => item.associationKey !== action.notice.associationKey), action.notice] });
  if (action.type === "cleanup/dismiss") return result({ ...state, cleanupNotices: state.cleanupNotices.filter(item => item.associationKey !== action.associationKey) });
  if (action.type === "focus/register") {
    const tabId = action.tabId ?? (action.kind === "tab" ? action.targetId : state.activeTabId);
    const intent: FocusIntent = { ...action, tabId, selectionRevision: tabId ? state.tabs[tabId]?.selectionRevision ?? 0 : 0,
      activeTabId: state.activeTabId, activeSpaceId: state.activeSpaceId };
    return result({ ...state, focusIntents: { ...state.focusIntents, [action.token]: intent } });
  }
  const tab = state.tabs[action.tabId];
  if (!tab) return result();
  let next = tab;
  switch (action.type) {
    case "activate-tab":
      if (state.activeTabId !== tab.tabId) effects.push({ type: "cancel-transient", reason: "tab-hidden" });
      return result({ ...state, activeTabId: tab.tabId, activeSpaceId: tab.spaceId });
    case "select-leaf": next = selectTabLeaf(tab, action.leafId); break;
    case "drop":
      if (!tab.root || action.revision !== tab.revision || tab.zoomLeafId) return result();
      next = { ...tab, root: applyDrop(tab.root, action.src, action.target) };
      if (next.root === tab.root) return result();
      next = selectTabLeaf({ ...next, revision: tab.revision + 1 }, action.src);
      break;
    case "resize-commit":
      if (!tab.root) return result();
      next = { ...tab, root: setPairWeights(tab.root, action.splitId, action.index, action.wa, action.wb) };
      if (next.root !== tab.root) next.revision++;
      break;
    case "zoom-toggle": {
      const id = action.leafId ?? tab.selectedLeafId;
      if (!id || !findNode(tab.root, id)) return result();
      next = { ...selectTabLeaf(tab, id), zoomLeafId: tab.zoomLeafId === id ? null : id };
      effects.push({ type: "announce", text: next.zoomLeafId ? `Zoomed: ${id}. Other panes are hidden, not closed.` : "Layout restored." });
      break;
    }
    case "viewer/open-begin": {
      const previous = tab.viewers[action.kind];
      const slot: ViewerSlot = { status: "opening", selector: action.selector, sourcePaneId: action.sourcePaneId,
        context: previous?.context ?? null, viewsBySource: previous?.viewsBySource ?? {}, error: null, requestId: action.requestId };
      next = insertViewer(tab, action.kind, action.dir, action.placeBeside);
      if (!findNode(next.root, `${tab.tabId}:${action.kind}`)) return result();
      next = selectTabLeaf({ ...next, viewers: { ...next.viewers, [action.kind]: slot } }, `${tab.tabId}:${action.kind}`);
      break;
    }
    case "viewer/opened":
    case "viewer/failed":
    case "viewer/source-view": {
      const slot = tab.viewers[action.kind];
      if (!slot || (action.type !== "viewer/source-view" && action.requestId !== undefined && action.requestId !== slot.requestId)) return result();
      const updated = action.type === "viewer/opened" ? { ...slot, context: action.context, status: "open" as const, error: null }
        : action.type === "viewer/failed" ? { ...slot, status: "error" as const, error: action.error }
        : { ...slot, viewsBySource: { ...slot.viewsBySource, [action.sourceId]: action.view } };
      next = { ...tab, viewers: { ...tab.viewers, [action.kind]: updated } };
      break;
    }
    case "viewer/closed":
      if (action.requestId !== undefined && action.requestId !== tab.viewers[action.kind]?.requestId) return result();
      next = closeLocal(tab, `${tab.tabId}:${action.kind}`); break;
    case "browser/state": {
      const slot = tab.viewers.browser;
      const requestId = action.requestId ?? action.slot.requestId;
      const starts = action.slot.status === "opening" || action.slot.status === "closing";
      if (!starts && requestId !== undefined && requestId !== slot?.requestId) return result();
      next = insertViewer(tab, "browser", action.dir ?? "row", action.placeBeside);
      if (!findNode(next.root, `${tab.tabId}:browser`)) return result();
      next = { ...next, viewers: { ...next.viewers, browser: { ...action.slot, requestId } } };
      if (!slot) next = selectTabLeaf(next, `${tab.tabId}:browser`);
      break;
    }
    case "leaf/close-local": next = closeLocal(tab, action.leafId); break;
  }
  return result({ ...state, tabs: { ...state.tabs, [tab.tabId]: next } });
}

// Dispatch is synchronous for overlapping network intents; React only observes the resulting state.
// Session layouts are retained in this App-owned hook for the run, not persisted to storage.
export function useTabLayouts(sessionId: string, serverInstance = ""): TabLayouts {
  const sessions = useRef<Record<string, SessionLayoutState>>({});
  if (!sessions.current[sessionId]) sessions.current[sessionId] = createSessionLayoutState(sessionId, serverInstance);
  const [, render] = useState(0);
  const pendingEffects = useRef<Record<string, LayoutEffect[]>>({});
  const getState = useCallback(() => sessions.current[sessionId], [sessionId]);
  const dispatch = useCallback((action: LayoutAction) => {
    const next = layoutReducer(sessions.current[sessionId], action);
    sessions.current[sessionId] = next.state;
    (pendingEffects.current[sessionId] ??= []).push(...next.effects);
    render(n => n + 1);
  }, [sessionId]);
  const takeEffects = useCallback(() => pendingEffects.current[sessionId]?.splice(0) ?? [], [sessionId]);
  return { state: sessions.current[sessionId], getState, dispatch, takeEffects };
}
