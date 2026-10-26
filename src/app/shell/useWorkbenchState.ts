import { useEffect, useState } from "react";
import type { SpaceGitAction, SpaceGitStatus, SpaceServiceStatus } from "../../protocol/generated/v1";
import { spaceCheckoutKey, useSpaceGitStatus } from "../session/spaceGitStatus";
import { useSpaceServiceStatus } from "../session/spaceServiceStatus";
import { useSpaceTags, type SpaceTagsState } from "../session/spaceTags";
import { useAgentAttention, type AgentAttentionState } from "../session/agentAttention";
import { gitActionReason, gitActionRequest, useSpaceGitActions, type SpaceGitActions } from "../session/spaceGitActions";
import type { SessionSummary } from "../../protocol/generated/v1";
import type { LibraryCommand } from "../context/ContextViewer";
import { byId, workbenchView, type WorkbenchView } from "./model";
import { useWorkarea, type WorkareaController } from "./useWorkarea";
import { useSidebarState, type SidebarState } from "./useSidebarState";
import { useWidgetWindowState, type WidgetWindowState } from "./useWidgetWindowState";
import { useLiaisonNavigation, type LiaisonNavigation } from "./useLiaisonNavigation";
import { useLiaison, type LiaisonController } from "../supervisor/useLiaison";
import { useWorkbenchUI, type WorkbenchUI } from "./useWorkbenchUI";
import { useViewerSources } from "./useViewerSources";
import type { ViewerSourcesState } from "./commands";
import { browserOpenDisabledReason } from "../layout/browserLifecycle";
import type { WorkbenchProps } from "./Workbench";

export interface WorkbenchState extends WorkbenchView, WorkareaController, SidebarState, WidgetWindowState, LiaisonNavigation, WorkbenchUI {
  libraryOpen: boolean; notesOpen: boolean; overviewOpen: boolean; liaison: LiaisonController; libraryCommand: LibraryCommand | null;
  mutationBusy: boolean; modalOpen: boolean; browserOpen: boolean; browserInputActive: boolean; browserReason: string | null;
  viewerSources: ViewerSourcesState; sidebarSession: SessionSummary | undefined;
  gitPoll: { spaces: ReadonlyMap<string, SpaceGitStatus>; error: string | undefined; refresh(): void };
  spaceGit: ReadonlyMap<string, SpaceGitStatus>; gitBlocked: string | undefined; gitActions: SpaceGitActions;
  spaceServices: ReadonlyMap<string, readonly SpaceServiceStatus[]>;
  spaceTags: SpaceTagsState;
  attention: AgentAttentionState;
  tagEditorRequest: { spaceId: string; token: number } | null;
  requestTagEditor(spaceId: string): void;
  gitNotice: { sessionId: string | null; text: string } | null;
  runGitAction(spaceId: string, action: SpaceGitAction): void;
}
export function useWorkbenchState(props: WorkbenchProps): WorkbenchState {
  const { client, state, selection, tabLayout, mutations } = props;
  const view = workbenchView(state, selection, tabLayout, mutations);
  const { snapshot, spaces, allTabs, tabs, selectedTab, sourcePaneId, selectedLeaf, shell, popup } = view;
  const workarea = useWorkarea(state.sessionId, selection.paneId);
  const libraryOpen = workarea.view.kind === "library";
  const notesOpen = workarea.view.kind === "notes";
  const overviewOpen = workarea.view.kind === "overview";
  const ui = useWorkbenchUI(props, workarea);
  const widget = useWidgetWindowState(props, workarea, selectedTab, sourcePaneId, spaces, allTabs, tabs);
  const navigation = useLiaisonNavigation(props, workarea.closeOverview);
  const liaison = useLiaison(props, workarea.closeOverview, navigation.supervisorTerminal, workarea.view.kind === "terminal");
  const sidebar = useSidebarState(props);
  const { sidebarCollapsed, narrowViewport, drawerOpen } = sidebar;
  const { popupPending, dialog, commandsOpen, switcherOpen, sessionChooserOpen, setupOpen, recoveryOpen, teardownSpaceId, libraryAddOpen, askOpen } = ui;
  const mutationBusy = mutations.pending !== null;
  const modalOpen = Boolean(popup || popupPending) || dialog !== null || commandsOpen || switcherOpen || sessionChooserOpen || setupOpen || recoveryOpen || teardownSpaceId !== null || libraryAddOpen || askOpen || liaison.recovery !== null;
  const gitPoll = useSpaceGitStatus(client, state.sync === "live" ? state.sessionId : null, spaceCheckoutKey(spaces, snapshot?.panes ?? []));
  const spaceGit = gitPoll.spaces;
  const spaceServices = useSpaceServiceStatus(client, state.sync === "live" ? state.sessionId : null, snapshot, narrowViewport ? drawerOpen : !sidebarCollapsed);
  const spaceTags = useSpaceTags(client, state.sessionId, snapshot, state.sync === "live");
  const attention = useAgentAttention(client, state, state.sync === "live");
  const [tagEditor, setTagEditor] = useState<{ spaceId: string; token: number; sessionId: string | null; serverInstance: string | undefined } | null>(null);
  const tagEditorRequest = tagEditor?.sessionId === state.sessionId && tagEditor?.serverInstance === snapshot?.server_instance ? tagEditor : null;
  const requestTagEditor = (spaceId: string) => {
    if (spaceTags.editBlocked) return;
    if (narrowViewport) sidebar.openDrawer();
    else if (sidebarCollapsed) sidebar.toggleSidebarCollapsed();
    setTagEditor(previous => ({ spaceId, token: (previous?.token ?? 0) + 1, sessionId: state.sessionId, serverInstance: snapshot?.server_instance }));
  };
  const gitBlocked = state.sync !== "live" ? "Herdr is not live" : mutationBusy ? "Herdr is applying a change" : popup || popupPending ? "The popup owns keyboard input" : gitPoll.error ? `Git status unavailable: ${gitPoll.error}` : undefined;
  const [gitNotice, setGitNotice] = useState<{ sessionId: string | null; text: string } | null>(null);
  const gitActions = useSpaceGitActions(client, state.sessionId, spaceGit, gitPoll.refresh, (spaceId, problem) => {
    const row = [...document.querySelectorAll<HTMLElement>(".space-row-group")].find(element => element.dataset.spaceId === spaceId);
    const box = row?.getBoundingClientRect();
    const listBox = row?.closest(".space-list")?.getBoundingClientRect();
    if (sidebarCollapsed || (narrowViewport && !drawerOpen) || !box || box.height === 0 || !listBox || box.bottom <= listBox.top || box.top >= listBox.bottom) setGitNotice({ sessionId: state.sessionId, text: `${byId(spaces, spaceId)?.label ?? "Space"}: ${problem.summary}` });
  });
  useEffect(() => {
    if (!gitNotice) return;
    const timer = window.setTimeout(() => setGitNotice(null), 6000);
    return () => window.clearTimeout(timer);
  }, [gitNotice]);
  const runGitAction = (spaceId: string, action: SpaceGitAction) => {
    const status = spaceGit.get(spaceId);
    const pending = gitActions.forStatus(status);
    if (gitActionReason(status, action, pending, gitBlocked)) return;
    const request = gitActionRequest(status, action);
    if (request) {
      setGitNotice(null);
      gitActions.run(request);
    }
  };
  const viewerSources = useViewerSources({ client, menu: ui.menu, commandsOpen, sourcePaneId, sessionId: state.sessionId, live: state.sync === "live" });
  const browserOpen = Boolean(tabLayout?.viewers.browser);
  const browserInputActive = selectedLeaf?.kind === "browser" && workarea.view.kind === "terminal";
  const browserReason = !selectedTab ? "Select a tab first" : state.sync !== "live" ? "Herdr is not live" : browserOpenDisabledReason(props.ctx, selectedTab.id);
  return { ...view, ...workarea, ...ui, ...widget, ...navigation, ...sidebar, libraryOpen, notesOpen, overviewOpen, liaison,
    libraryCommand: workarea.view.kind === "library" ? workarea.view.command : null,
    mutationBusy, modalOpen, viewerSources, browserOpen, browserInputActive, browserReason,
    sidebarSession: props.sessions.find(session => session.id === state.sessionId), gitPoll, spaceGit, spaceServices, spaceTags, attention, tagEditorRequest, requestTagEditor, gitBlocked, gitActions, gitNotice, runGitAction };
}
