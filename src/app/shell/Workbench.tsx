import type { CockpitClient } from "../../client/CockpitClient";
import type { FocusRequest, SessionSummary, WidgetSummary } from "../../protocol/generated/v1";
import type { SessionState } from "../session/sessionStore";
import type { MutationCoordinatorState, MutationOperation } from "../session/mutationCoordinator";
import type { LeafCtx, TabLayoutState } from "../layout/tabLayoutStore";
import { closeBrowserLeaf, retryBrowserCleanup } from "../layout/browserLifecycle";
import { byId, type Selection, type Mutate } from "./model";
import { buildCommands } from "./commands";
import { useWorkbenchState } from "./useWorkbenchState";
import { useWorkbenchActions } from "./useWorkbenchActions";
import { useWorkbenchInput } from "./useWorkbenchInput";
import { WorkbenchContent } from "./WorkbenchContent";

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
  const runtime = useWorkbenchState(props);
  const actions = useWorkbenchActions(props, runtime);
  const input = useWorkbenchInput(props, runtime, actions);
  const { ctx, tabLayout, mutations, onFocus } = props;
  const { shell, popup, spaces, allTabs, selectedTab, confirmedSpaceId, customPrefixes, libraryOpen, notesOpen, browserOpen, browserReason, spaceGit, gitActions, gitBlocked, widgets, widgetsPending, viewerSources, limits, openSupervisor, runGitAction, setRecoveryOpen, closeNotes, openNotes, showWidgets, setLibraryAddOpen, openLibrary, setLimitsOpen, commandNotice, lifecycleError, leave } = runtime;
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
  const commandActions = buildCommands({
    herdr: { commands: shell?.commands ?? [], prefixes: customPrefixes, reason: input.customCommandReason, popupOpen: Boolean(popup) },
    view: runtime, libraryOpen, notesOpen, browser: { open: browserOpen, reason: browserReason },
    git: { target, status, pending: gitActions.forStatus(status), blocked: gitBlocked },
    widgets: { pending: widgetsPending, dockTabId: tabLayout?.viewers.widget ? tabLayout.tabId : null, list: widgets.widgets(ctx.sessionId) },
    viewerSources, local: limits.absent ? {} : { "subscription-limits": () => setLimitsOpen(true) },
    run: {
      herdr: input.runHerdrCommand, prefix: input.runCommand, supervisor: openSupervisor, git: runGitAction,
      recovery: () => setRecoveryOpen(true), toggleNotes: notesOpen ? closeNotes : openNotes, openBrowser: actions.openBrowser,
      closeBrowser: () => { if (selectedTab) actions.perform(closeBrowserLeaf(ctx, selectedTab.id)); },
      retryBrowserCleanup: () => actions.perform(retryBrowserCleanup(ctx)), showWidgets,
      cycleWidget: (tabId, step) => widgets.cycle(tabId, step), removeWidget: tabId => { actions.closeLeaf(`${tabId}:widget`); }, goToWidget,
      libraryAdd: () => setLibraryAddOpen(true), library: openLibrary, openViewer: actions.openViewer,
    },
  });
  const commandFailures = Object.values(mutations.errors).filter(failure => failure.operation.request.type === "command_invoke");
  const commandFailureMessage = commandFailures.map(failure => `${failure.code === "request_outcome_unknown" ? "Herdr did not confirm this command. Check the session before running it again." : "Could not run Herdr command:"} ${failure.message}`).join(" ");
  const commandStatus = commandNotice || commandFailureMessage || lifecycleError ? <p role="alert">{commandNotice || commandFailureMessage || lifecycleError}</p> : null;
  return <WorkbenchContent {...props} {...runtime} {...actions} commandActions={commandActions} commandFailureMessage={commandFailureMessage} commandStatus={commandStatus} />;
}
