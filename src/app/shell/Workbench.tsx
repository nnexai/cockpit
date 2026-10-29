import { useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { FocusRequest, SessionSummary, WidgetSummary } from "../../protocol/generated/v1";
import type { SessionState } from "../session/sessionStore";
import type { MutationCoordinatorState, MutationOperation } from "../session/mutationCoordinator";
import type { LeafCtx, TabLayoutState } from "../layout/tabLayoutStore";
import { closeBrowserLeaf, retryBrowserCleanup } from "../layout/browserLifecycle";
import { leaves } from "../layout/splitTree";
import { byId, type Selection, type Mutate } from "./model";
import { buildCommands } from "./commands";
import { useWorkbenchState } from "./useWorkbenchState";
import { useWorkbenchActions } from "./useWorkbenchActions";
import { useWorkbenchInput } from "./useWorkbenchInput";
import { WorkbenchContent } from "./WorkbenchContent";
import { useQuickAsk } from "../quickAsk/useQuickAsk";
import { QuickTerminal } from "./QuickTerminal";
import { plainError } from "./plainErrors";
import { widgetKey } from "../widgets/widgetStore";
import { stateClass } from "../sidebar/spaceTree";
import { useAgentPower } from "./useAgentPower";
import { SendAgentDialog } from "../quickAsk/SendAgentDialog";

export interface WorkbenchProps {
  client: CockpitClient; state: SessionState; sessions: SessionSummary[]; selection: Selection; terminalMouseInput: boolean; mutations: MutationCoordinatorState;
  ctx: LeafCtx; tabLayout: TabLayoutState | null; registerTransient(cancel: () => void): () => void;
  layoutError: string | null; onDismissLayoutError(): void;
  onWidgetAnnouncement(text: string): void;
  onSession(id: string): void; onFocus(request: FocusRequest, location: Selection, prepare?: { paneId: string }): void;
  onSelectLeaf(tabId: string, leafId: string): void; onSplit(tabId: string, leafId: string, direction: "right" | "down"): void;
  onPanePrepared(paneId: string): void; onReconnect(): void; onRetry(): void; onRefreshSessions(): Promise<void>; onOpenSession(): void; onMutate: Mutate; onRetryMutation(operation: MutationOperation): void;
}
export function Workbench(props: WorkbenchProps) {
  const [sendAgentsOpen, setSendAgentsOpen] = useState(false);
  useEffect(() => { setSendAgentsOpen(false); }, [props.state.sessionId]);
  const runtime = useWorkbenchState(props, sendAgentsOpen);
  const actions = useWorkbenchActions(props, runtime);
  const power = useAgentPower((runtime.snapshot?.agents ?? []).some(agent => stateClass(agent.status) === "working"), runtime.setPrefixHint);
  const [quickTerminal, setQuickTerminal] = useState<{ sessionId: string; paneId: string; cwd: string | null; returnFocus: HTMLElement | null } | null>(null);
  const [closedWidgets, setClosedWidgets] = useState<WidgetSummary[]>([]);
  const [reopenedWidget, setReopenedWidget] = useState<string | null>(null);
  const updatePending = useRef(false);
  const quickTerminalRef = useRef(quickTerminal);
  quickTerminalRef.current = quickTerminal;
  const openQuickTerminal = () => {
    if (quickTerminalRef.current) return;
    if (!props.state.sessionId || !runtime.sourcePaneId || props.state.sync !== "live") { runtime.setPrefixHint("A live terminal in this tab is required"); return; }
    const focused = runtime.commandsOpen ? runtime.commandsOpener.current : document.activeElement;
    setQuickTerminal({ sessionId: props.state.sessionId, paneId: runtime.sourcePaneId, cwd: runtime.snapshot?.panes.find(pane => pane.id === runtime.sourcePaneId)?.cwd ?? null, returnFocus: focused instanceof HTMLElement ? focused : null });
  };
  const origin = useRef<{ sessionId: string | null; selection: Selection } | null>(null);
  const { liaison, overviewOpen, closeOverview } = runtime;
  useEffect(() => {
    if (origin.current?.sessionId !== props.state.sessionId) origin.current = null;
    if (props.selection.spaceId && props.selection.paneId && props.selection.paneId !== liaison.paneId) {
      origin.current = { sessionId: props.state.sessionId, selection: { ...props.selection } };
    }
  }, [props.state.sessionId, props.selection.spaceId, props.selection.tabId, props.selection.paneId, liaison.paneId]);
  const running = liaison.connected && !!liaison.paneId;
  const inLiaisonPane = running && props.selection.paneId === liaison.paneId;
  const saved = origin.current?.sessionId === props.state.sessionId && origin.current.selection.paneId !== liaison.paneId ? origin.current.selection : null;
  const previousSpace = saved ? byId(runtime.spaces, saved.spaceId) : undefined;
  const liaisonLabel = !running ? liaison.actionLabel ?? "Start supervisor"
    : inLiaisonPane && previousSpace && !overviewOpen ? `Back to ${previousSpace.label}` : "Go to liaison";
  const liaisonReason = !running ? liaison.actionEnabled ? null : liaison.actionReason ?? liaison.status ?? "Supervisor unavailable"
    : !inLiaisonPane && !liaison.startAvailable ? liaison.status ?? "Supervisor unavailable"
    : inLiaisonPane && !overviewOpen && !previousSpace ? "Nothing to return to" : null;
  const toggleLiaison = () => {
    if (liaisonReason) return;
    if (!running || !inLiaisonPane) { void liaison.activate(); return; }
    if (overviewOpen) { closeOverview(); return; }
    if (!saved || !previousSpace) return;
    const tab = runtime.allTabs.find(item => item.id === saved.tabId && item.space_id === previousSpace.id);
    const layout = tab ? props.ctx.getState().tabs[tab.id] : undefined;
    const leaf = saved.paneId && layout ? leaves(layout.root).find(item => item.id === saved.paneId) : undefined;
    const pane = saved.paneId && leaf?.kind === "terminal" ? runtime.snapshot?.panes.find(item => item.id === saved.paneId && item.tab_id === tab?.id && item.space_id === previousSpace.id) : undefined;
    runtime.leave("focus");
    if (tab && leaf && saved.paneId && (leaf.kind !== "terminal" || pane)) {
      if (layout?.zoomLeafId && layout.zoomLeafId !== saved.paneId) props.ctx.dispatch({ type: "zoom-toggle", tabId: tab.id, leafId: layout.zoomLeafId });
      if (leaf.kind === "terminal") props.onFocus({ kind: "pane", target_id: saved.paneId }, saved);
      else {
        props.onSelectLeaf(tab.id, saved.paneId);
        props.onFocus({ kind: "tab", target_id: tab.id }, saved);
      }
    } else {
      const tabId = runtime.allTabs.find(item => item.space_id === previousSpace.id && item.focused)?.id ?? null;
      const paneId = runtime.snapshot?.panes.find(item => item.space_id === previousSpace.id && item.focused)?.id ?? null;
      props.onFocus({ kind: "space", target_id: previousSpace.id }, { spaceId: previousSpace.id, tabId, paneId });
    }
  };
  const input = useWorkbenchInput(props, runtime, actions, openQuickTerminal, toggleLiaison);
  const ask = useQuickAsk({
    client: props.client, sessionId: props.state.sessionId, serverInstance: runtime.snapshot?.server_instance ?? null,
    spaceId: runtime.confirmedSpaceId, initialCwd: runtime.selectedSpace?.git?.checkout_path ?? runtime.selectedPane?.cwd ?? "",
    open: runtime.askOpen, setOpen: runtime.setAskOpen,
    onOpenTab: location => {
      runtime.leave("focus");
      props.onFocus({ kind: "pane", target_id: location.pane_id }, { spaceId: location.space_id, tabId: location.tab_id, paneId: location.pane_id });
    },
  });
  useEffect(() => {
    if (!runtime.switcherOpen || !props.state.sessionId || props.state.sync !== "live") return;
    const abort = new AbortController();
    setClosedWidgets([]);
    void props.client.widgetShelfList({}, abort.signal).then(result => {
      if (!abort.signal.aborted) setClosedWidgets(result.widgets.map(entry => entry.summary).filter(widget => widget.key.session_id === props.state.sessionId));
    }).catch(() => { if (!abort.signal.aborted) runtime.setPrefixHint("Could not load closed widgets"); });
    return () => abort.abort();
  }, [runtime.switcherOpen, props.state.sessionId, props.state.sync, props.client, runtime.setPrefixHint]);
  useEffect(() => {
    const target = runtime.switchTarget;
    if (!target || runtime.switcherOpen) return;
    runtime.setSwitchTarget(null);
    // Re-resolve after the modal has unmounted; never focus from a stale row or while another modal owns input.
    if (runtime.modalOpen || props.state.sync !== "live" || (target.kind === "space" && runtime.mutationBusy)) return;
    if (target.kind === "space") {
      const space = byId(runtime.spaces, target.id);
      if (space) actions.focusSpace(space);
      else runtime.setPrefixHint("That Space no longer exists");
    } else if (target.kind === "agent") {
      const agent = runtime.snapshot?.agents.find(candidate => candidate.pane_id === target.id);
      if (agent) actions.focusAgent(agent);
      else runtime.setPrefixHint("That agent no longer exists");
    } else if (target.kind === "widget") {
      const widget = runtime.widgets.byKey.get(target.id);
      if (widget) goToWidget(widget);
      else runtime.setPrefixHint("That widget no longer exists");
    } else {
      const widget = closedWidgets.find(candidate => widgetKey(candidate.key) === target.id && candidate.key.session_id === props.state.sessionId);
      if (!widget) { runtime.setPrefixHint("That closed widget no longer exists"); return; }
      void props.client.widgetShelfReopen({ key: widget.key }).then(reopened => {
        setReopenedWidget(widgetKey(reopened.key));
      }).catch(() => runtime.setPrefixHint("Could not reopen widget"));
    }
  }, [runtime.switchTarget, runtime.switcherOpen, runtime.modalOpen, runtime.spaces, runtime.snapshot, runtime.mutationBusy, props.state.sync, actions.focusSpace, actions.focusAgent, runtime.setSwitchTarget, runtime.setPrefixHint]);
  const { ctx, tabLayout, mutations, onFocus } = props;
  const { shell, popup, spaces, allTabs, selectedTab, confirmedSpaceId, customPrefixes, libraryOpen, notesOpen, openOverview, browserOpen, browserReason, spaceGit, gitActions, gitBlocked, widgets, widgetsPending, viewerSources, limits, runGitAction, setRecoveryOpen, closeNotes, openNotes, showWidgets, setLibraryAddOpen, openLibrary, setLimitsOpen, setSwitcherOpen, commandNotice, lifecycleError, leave } = runtime;
  const target = byId(spaces, confirmedSpaceId);
  const status = target ? spaceGit.get(target.id) : undefined;
  const goToWidget = (widget: WidgetSummary) => {
    const tab = allTabs.find(candidate => candidate.id === widget.key.tab_id);
    if (!tab) return;
    // A widget explicitly requested from Commands returns to the terminal workarea.
    leave("keep");
    const local = ctx.getState().tabs[tab.id];
    if (local?.zoomLeafId) ctx.dispatch({ type: "zoom-toggle", tabId: tab.id, leafId: local.zoomLeafId });
    widgets.show(tab.id, widget.key.id);
    onFocus({ kind: "tab", target_id: tab.id }, { spaceId: tab.space_id, tabId: tab.id, paneId: local?.selectedLeafId ?? tab.focused_pane_id });
  };
  useEffect(() => {
    if (!reopenedWidget) return;
    const widget = widgets.byKey.get(reopenedWidget);
    if (!widget || widget.key.session_id !== props.state.sessionId) return;
    setReopenedWidget(null);
    goToWidget(widget);
  }, [reopenedWidget, widgets.getSnapshot(), props.state.sessionId]);
  const installUpdate = (sourcePath: string) => {
    const sessionId = props.state.sessionId;
    const spaceId = spaces.find(space => space.git?.checkout_path === sourcePath)?.id
      ?? runtime.snapshot?.panes.find(pane => pane.cwd === sourcePath)?.space_id
      ?? props.selection.spaceId;
    if (!sessionId || !spaceId || props.state.sync !== "live" || updatePending.current) return;
    updatePending.current = true;
    void props.client.updateInstall(sessionId, spaceId).then(location => {
      runtime.leave("focus");
      props.onFocus({ kind: "pane", target_id: location.pane_id }, { spaceId: location.space_id, tabId: location.tab_id, paneId: location.pane_id });
    }).catch(error => runtime.setPrefixHint(`Could not start install: ${String(error)}. Check for an install tab before retrying.`))
      .finally(() => { updatePending.current = false; });
  };
  const commandActions = buildCommands({
    herdr: { commands: shell?.commands ?? [], prefixes: customPrefixes, reason: input.customCommandReason, popupOpen: Boolean(popup) },
    view: runtime, libraryOpen, notesOpen, browser: { open: browserOpen, reason: browserReason },
    liaison: { label: liaisonLabel, reason: liaisonReason, overviewAvailable: liaison.tasks.some(task => !task.task.checked), overviewOpen },
    git: { target, status, pending: gitActions.forStatus(status), blocked: gitBlocked },
    tags: { blocked: runtime.spaceTags.editBlocked },
    widgets: { pending: widgetsPending, dockTabId: tabLayout?.viewers.widget ? tabLayout.tabId : null, list: widgets.widgets(ctx.sessionId) },
    viewerSources, local: { "quick-switch": () => setSwitcherOpen(true), ...(!limits.absent ? { "subscription-limits": () => setLimitsOpen(true) } : {}) },
    power: { stayAwake: power.enabled, toggleStayAwake: power.available ? power.toggle : undefined, turnScreenOff: power.screenOffAvailable ? power.screenOff : undefined },
    sendToIdleAgents: () => setSendAgentsOpen(true),
    run: {
      herdr: input.runHerdrCommand, prefix: input.runCommand, liaison: toggleLiaison, overview: () => { if (!overviewOpen) openOverview(); else { closeOverview(); if (props.selection.paneId !== liaison.paneId && liaison.startAvailable) void liaison.start(); } }, git: runGitAction,
      recovery: () => setRecoveryOpen(true), toggleNotes: notesOpen ? closeNotes : openNotes, openBrowser: actions.openBrowser,
      askAgent: () => runtime.setAskOpen(true),
      editSpaceTags: () => { if (runtime.selectedSpace) runtime.requestTagEditor(runtime.selectedSpace.id); },
      closeBrowser: () => { if (selectedTab) actions.perform(closeBrowserLeaf(ctx, selectedTab.id)); },
      retryBrowserCleanup: () => actions.perform(retryBrowserCleanup(ctx)), showWidgets,
      cycleWidget: (tabId, step) => widgets.cycle(tabId, step), removeWidget: tabId => { actions.closeLeaf(`${tabId}:widget`); }, goToWidget,
      libraryAdd: () => setLibraryAddOpen(true), library: openLibrary, openViewer: actions.openViewer,
    },
  });
  const commandFailures = Object.values(mutations.errors).filter(failure => failure.operation.request.type === "command_invoke");
  const commandFailureMessage = commandFailures.map(failure => plainError(failure.code, failure.message).text).join(" ");
  const commandFailureDetail = commandFailures.map(failure => failure.detail ?? plainError(failure.code, failure.message).detail).filter(Boolean).join("\n");
  const commandStatus = commandNotice || commandFailureMessage || lifecycleError ? <p role="alert" title={commandNotice || lifecycleError ? undefined : commandFailureDetail || undefined}>{commandNotice || commandFailureMessage || lifecycleError}</p> : null;
  return <><WorkbenchContent {...props} {...runtime} {...actions} onUpdate={installUpdate} closedWidgets={closedWidgets} liaisonAction={{ label: liaisonLabel, reason: liaisonReason, running, inPane: inLiaisonPane, toggle: toggleLiaison }} ask={ask} quickTerminalOpen={Boolean(quickTerminal)} commandActions={commandActions} commandFailureMessage={commandFailureMessage} commandStatus={commandStatus} />
    {quickTerminal ? <QuickTerminal client={props.client} sessionId={quickTerminal.sessionId} paneId={quickTerminal.paneId} cwd={quickTerminal.cwd} focusEpoch={props.state.epoch} returnFocus={quickTerminal.returnFocus} suspended={Boolean(popup)} onDismiss={() => setQuickTerminal(null)} /> : null}
    {sendAgentsOpen && props.state.sessionId ? <SendAgentDialog key={props.state.sessionId} targets={(props.state.sync === "live" ? runtime.snapshot?.agents ?? [] : []).filter(agent => stateClass(agent.status) === "idle").map(agent => ({
      paneId: agent.pane_id, label: agent.title || agent.name, space: `${spaces.find(space => space.id === agent.space_id)?.label ?? "Space"} · ${allTabs.find(tab => tab.id === agent.tab_id)?.label ?? "tab"}`,
    }))} onSend={async (paneId, text) => { await props.client.orchestrationAgentSend({ session_id: props.state.sessionId!, reference: paneId, text }); }} onClose={() => setSendAgentsOpen(false)} /> : null}</>;
}
