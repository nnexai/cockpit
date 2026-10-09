import { parseResourceMutationResponse } from "../../client/CockpitClient";
import type { ResourceMutationRequest, SessionSnapshotResponse } from "../../protocol/generated/v1";
import type { SessionState } from "../session/sessionStore";
import type { MutationCoordinatorState } from "../session/mutationCoordinator";
import type { TabLayoutState } from "../layout/tabLayoutStore";
import { leaves, type Leaf } from "../layout/splitTree";
import { runtimeSource } from "../layout/reconcile";
import { setupParentFor, type SetupParent } from "../projects/SetupDialog";
import type { LibrarySpace } from "../library/libraryState";
import type { Space } from "../sidebar/spaceTree";

export type StatusError = { message: string; code?: string };
export type SessionSnapshot = SessionSnapshotResponse;
export type Tab = SessionSnapshot["tabs"][number];
export type Pane = SessionSnapshot["panes"][number];
export type Selection = { spaceId: string | null; tabId: string | null; paneId: string | null };

export function describeError(error: unknown, fallback: string): StatusError {
  if (error instanceof Error) {
    const typed = error as Error & { code?: unknown; operationCode?: unknown };
    return { message: typed.message || fallback, code: typeof typed.operationCode === "string" ? typed.operationCode : typeof typed.code === "string" ? typed.code : undefined };
  }
  return { message: fallback };
}
export function byId<T extends { id: string }>(items: readonly T[], id: string | null): T | undefined { return id ? items.find((item) => item.id === id) : undefined; }
export function tabsForSpace(tabs: Tab[], spaceId: string | null): Tab[] { return spaceId ? tabs.filter((tab) => tab.space_id === spaceId) : []; }
export function panesForTab(panes: Pane[], tabId: string | null): Pane[] { return tabId ? panes.filter((pane) => pane.tab_id === tabId) : []; }
export function authoritativeMutationSnapshot(expectedSessionId: string, response: unknown): SessionSnapshot {
  const parsed = parseResourceMutationResponse(response);
  if (parsed.session_id !== expectedSessionId) {
    throw new Error("Mutation response belongs to another session");
  }
  return parsed.snapshot;
}

export type Mutate = (key: string, request: ResourceMutationRequest, focusFromSnapshot?: boolean) => boolean;

export interface WorkbenchView {
  snapshot: SessionSnapshot | null; shell: SessionSnapshot["herdr_shell"] | null;
  popup: NonNullable<SessionSnapshot["herdr_shell"]>["popup"] | null;
  spaces: Space[]; allTabs: Tab[]; tabs: Tab[]; panes: Pane[];
  selectedSpace: Space | undefined; selectedTab: Tab | undefined; selectedPane: Pane | undefined;
  localLeaves: Leaf[]; selectedLeaf: Leaf | undefined; sourcePaneId: string | null;
  librarySpace: LibrarySpace | null; setupParent: SetupParent | null;
  live: boolean; busy: boolean; confirmedSpaceId: string | null; confirmedPaneId: string | null;
}
export function workbenchView(state: SessionState, selection: Selection, tabLayout: TabLayoutState | null, mutations: MutationCoordinatorState): WorkbenchView {
  const snapshot = state.snapshot;
  const shell = snapshot?.herdr_shell ?? null;
  const popup = shell?.popup ?? null;
  const spaces = snapshot?.spaces ?? [];
  const allTabs = snapshot?.tabs ?? [];
  const tabs = tabsForSpace(allTabs, selection.spaceId);
  const selectedSpace = byId(spaces, selection.spaceId);
  const selectedTab = byId(tabs, selection.tabId);
  const panes = panesForTab(snapshot?.panes ?? [], selectedTab?.id ?? null);
  const localLeaves = leaves(tabLayout?.root ?? null);
  const selectedLeaf = localLeaves.find(leaf => leaf.id === selection.paneId);
  const selectedPane = byId(panes, selection.paneId);
  const sourcePaneId = tabLayout ? runtimeSource(tabLayout, selection.paneId, snapshot?.focused_pane_id) : null;
  const librarySpace: LibrarySpace | null = state.sessionId && selectedSpace
    ? { target: { session_id: state.sessionId, space_id: selectedSpace.id }, label: selectedSpace.label, live: state.sync === "live" } : null;
  const setupParent = setupParentFor(selectedSpace, snapshot?.panes ?? [], snapshot?.focused_pane_id ?? null);
  return { snapshot, shell, popup, spaces, allTabs, tabs, panes, selectedSpace, selectedTab, selectedPane, localLeaves, selectedLeaf, sourcePaneId, librarySpace, setupParent, live: state.sync === "live", busy: mutations.pending !== null,
    confirmedSpaceId: state.focusPending ? snapshot?.focused_space_id ?? null : selection.spaceId,
    confirmedPaneId: state.focusPending ? snapshot?.focused_pane_id ?? null : selection.paneId };
}
