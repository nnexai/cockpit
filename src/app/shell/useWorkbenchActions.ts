import { useCallback } from "react";
import type { ContextAnchor } from "../sidebar/Spaces";
import { solveLayout } from "../layout/solveLayout";
import { openViewerLeaf, closeViewerLeaf } from "../layout/viewerLifecycle";
import { openBrowserLeaf, closeBrowserLeaf } from "../layout/browserLifecycle";
import type { Agent, Space } from "../sidebar/spaceTree";
import type { ContextTarget } from "./ContextMenu";
import { byId, describeError, type Tab, type Pane } from "./model";
import { viewerCapability as hasViewerCapability, type RendererKind } from "./commands";
import type { WorkbenchProps } from "./Workbench";
import type { WorkbenchState } from "./useWorkbenchState";

export type PaneFocusDirection = "left" | "right" | "up" | "down";
export interface WorkbenchActions {
  perform(operation: Promise<void>): void; openBrowser(): void; toggleBrowser(): void;
  focusSpace(space: Space): void; focusTab(tab: Tab): void; selectLeaf(leafId: string): void; focusAgent(agent: Agent): void;
  beginRename(target: ContextTarget | null): boolean; closeSpace(space: Space | undefined): boolean; closeTab(tab: Tab | undefined): boolean;
  lastTerminalMessage(pane: Pane, operation: "Closing" | "Moving"): string | null;
  closeLeaf(leafId: string): boolean; swap(source: string, target: string): void; zoom(leafId: string): void;
  neighbour(direction: PaneFocusDirection): string | null; viewerCapability(kind: RendererKind): boolean; openViewer(kind: RendererKind): void;
  openContext(event: ContextAnchor, target: ContextTarget): void; dismissMenu(): void; menuAction(action: () => boolean | void): void;
}
export function useWorkbenchActions(props: WorkbenchProps, runtime: WorkbenchState): WorkbenchActions {
  const { state, selection, tabLayout, ctx, onFocus, onSelectLeaf, onMutate } = props;
  const { snapshot, allTabs, panes, selectedTab, localLeaves, sourcePaneId, viewerSources, modalOpen, mutationBusy, narrowViewport, drawerFocusTarget, attachedPaneIds, widgets, area, browserOpen, browserReason, setAttachFocusSuppressed, setLifecycleError, setDialog, setEditing, setMenu, leave } = runtime;
  const perform = (operation: Promise<void>) => { setLifecycleError(null); void operation.catch(error => setLifecycleError(describeError(error, "Could not change this pane").message)); };
  const openBrowser = () => { if (selectedTab && !browserReason) { leave("focus"); perform(openBrowserLeaf(ctx, selectedTab.id, "row")); } };
  const toggleBrowser = () => { if (!selectedTab) return; leave("focus"); if (browserOpen) perform(closeBrowserLeaf(ctx, selectedTab.id)); else openBrowser(); };
  // Selecting a Space keeps the Library open; selecting a tab, pane or agent closes it to show that pane.
  const focusSpace = (space: Space) => {
    if (modalOpen) return;
    setAttachFocusSuppressed(false);
    const tabId = allTabs.find((tab) => tab.space_id === space.id && tab.focused)?.id ?? null;
    const paneId = snapshot?.panes.find((pane) => pane.space_id === space.id && pane.focused)?.id ?? null;
    if (narrowViewport) drawerFocusTarget.current = { spaceId: space.id, paneId };
    onFocus({ kind: "space", target_id: space.id }, { spaceId: space.id, tabId, paneId });
  };
  const focusTab = (tab: Tab) => {
    if (modalOpen) return;
    leave("focus");
    const remembered = ctx.getState().tabs[tab.id];
    const paneId = remembered?.selectedLeafId ?? tab.focused_pane_id ?? null;
    const focusedTerminal = tab.focused_pane_id;
    const painted = remembered?.zoomLeafId ? remembered.zoomLeafId === focusedTerminal : true;
    const prepare = focusedTerminal && painted && state.sync === "live" && snapshot?.focused_tab_id !== tab.id && !attachedPaneIds.current.has(focusedTerminal) ? { paneId: focusedTerminal } : undefined;
    onFocus({ kind: "tab", target_id: tab.id }, { spaceId: tab.space_id, tabId: tab.id, paneId }, prepare);
  };
  const selectLeaf = (leafId: string) => {
    if (modalOpen || !tabLayout) return;
    leave("focus");
    onSelectLeaf(tabLayout.tabId, leafId);
  };
  const focusAgent = (agent: Agent) => {
    if (modalOpen) return;
    leave("focus");
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
  const lastTerminalMessage = (pane: Pane, operation: "Closing" | "Moving") => {
    const tab = ctx.getState().tabs[pane.tab_id];
    if (!tab || Object.keys(tab.terminals).length !== 1 || !Object.values(tab.viewers).some(Boolean)) return null;
    const browser = tab.viewers.browser ? ", and deletes the browser's profile (cookies, logins, site data)" : "";
    const label = allTabs.find(candidate => candidate.id === pane.tab_id)?.label || pane.tab_id;
    return `This is the last terminal in ${label}. ${operation} it also closes Files, Review, Browser and widgets in this tab${browser}. Drafts and comments you saved stay.`;
  };
  const closeLeaf = (leafId: string): boolean => {
    if (!tabLayout) return false;
    const leaf = localLeaves.find(candidate => candidate.id === leafId);
    if (!leaf) return false;
    if (leaf.kind === "widget") { const current = widgets.widgets(ctx.sessionId, tabLayout.tabId).find(widget => widget.key.id === tabLayout.viewers.widget?.currentId); if (current) perform(widgets.remove(current.key)); return Boolean(current); }
    if (leaf.kind === "browser") { perform(closeBrowserLeaf(ctx, tabLayout.tabId)); return true; }
    if (leaf.kind !== "terminal") { perform(closeViewerLeaf(ctx, tabLayout.tabId, leaf.kind)); return true; }
    const pane = byId(panes, leafId);
    if (!pane) return false;
    const message = lastTerminalMessage(pane, "Closing") ?? `Close ${pane.title || "Terminal"}?`;
    return window.confirm(message) && onMutate(`pane:${pane.id}`, { type: "pane_close", pane_id: pane.id });
  };
  const swap = (source: string, target: string) => {
    if (!tabLayout?.root) return;
    const rect = solveLayout(tabLayout.root, area).leaves.get(target);
    if (rect) ctx.dispatch({ type: "drop", tabId: tabLayout.tabId, src: source, target: { kind: "swap", target, rect, label: "Swap" }, revision: tabLayout.revision });
  };
  const zoom = (leafId: string) => { if (tabLayout) ctx.dispatch({ type: "zoom-toggle", tabId: tabLayout.tabId, leafId }); };
  const neighbour = (direction: PaneFocusDirection) => {
    if (!tabLayout?.root || !selection.paneId) return null;
    const solved = solveLayout(tabLayout.root, area).leaves;
    const current = solved.get(selection.paneId);
    if (!current) return null;
    const cx = current.x + current.width / 2, cy = current.y + current.height / 2;
    const candidates = localLeaves.flatMap(leaf => {
      const rect = solved.get(leaf.id);
      if (!rect || leaf.id === selection.paneId) return [];
      const dx = rect.x + rect.width / 2 - cx, dy = rect.y + rect.height / 2 - cy;
      const primary = direction === "left" ? -dx : direction === "right" ? dx : direction === "up" ? -dy : dy;
      const secondary = direction === "left" || direction === "right" ? Math.abs(dy) : Math.abs(dx);
      return primary > 0 ? [{ id: leaf.id, score: primary + secondary * 2 }] : [];
    });
    candidates.sort((a, b) => a.score - b.score);
    return candidates[0]?.id ?? null;
  };
  const viewerCapability = (kind: RendererKind) => hasViewerCapability(kind, viewerSources);
  const openViewer = (kind: RendererKind) => {
    if (!tabLayout || !sourcePaneId || viewerSources.status !== "ready" || !viewerCapability(kind) || mutationBusy || state.sync !== "live") return;
    const selector = kind === "review" ? { kind: "review" as const, repositoryId: viewerSources.options.review_repository_ids[0] } : { kind: kind === "files" ? "files_folder" as const : "files_context" as const };
    leave("focus");
    perform(openViewerLeaf(ctx, tabLayout.tabId, kind === "review" ? "review" : "files", selector, "row", sourcePaneId));
  };
  const openContext = (event: ContextAnchor, target: ContextTarget) => { event.preventDefault(); event.stopPropagation(); if (!mutationBusy && !modalOpen) setMenu({ target, x: event.clientX, y: event.clientY }); };
  const dismissMenu = useCallback(() => setMenu(null), []);
  const menuAction = (action: () => boolean | void) => { if (action() !== false) dismissMenu(); };
  return { perform, openBrowser, toggleBrowser, focusSpace, focusTab, selectLeaf, focusAgent, beginRename, closeSpace, closeTab, lastTerminalMessage, closeLeaf, swap, zoom, neighbour, viewerCapability, openViewer, openContext, dismissMenu, menuAction };
}
