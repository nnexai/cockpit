// @vitest-environment jsdom
import "./input/viewerTestLayout";

import { act, useEffect, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient, TerminalStream } from "../client/CockpitClient";
import { CockpitClientError } from "../client/CockpitClient";
import type { TerminalPaneProps } from "./TerminalPane";
import type * as ContextViewerModule from "./context/ContextViewer";
import type { ContextViewerProps } from "./context/ContextViewer";
import type {
  BrowserAssociation,
  ResourceMutationRequest,
  ResourceMutationResponse,
  ViewerContext,
  ViewerSourceOptions,
  ViewerOpenRequest,
  SessionSnapshotResponse,
  SessionStreamMessage,
  SessionSummary,
  StatusResponse,
  SpaceGitActionResponse,
} from "../protocol/generated/v1";
import { App } from "./App";

const terminalReadyCallbacks = vi.hoisted(() => new Map<string, () => void>());
vi.mock("./TerminalPane", () => ({
  TerminalPane: ({ client, request, selected, presented = true, controlAllowed, deferAttachment, focusOnAttach = true, onSelect, onReady, registerStream }: TerminalPaneProps) => {
    const input = useRef<HTMLButtonElement>(null);
    useEffect(() => {
      if (selected && presented && controlAllowed && !deferAttachment && focusOnAttach) input.current?.focus();
    }, [selected, presented, controlAllowed, deferAttachment, focusOnAttach]);
    if (onReady) terminalReadyCallbacks.set(request.pane_id, onReady);
    const registration = useRef(registerStream);
    registration.current = registerStream;
    useEffect(() => {
      if (deferAttachment) return;
      let disposed = false;
      let stream: TerminalStream | undefined;
      void client.openTerminal({ ...request, mode: "control", takeover: false, cols: 80, rows: 24, cell_width_px: 8, cell_height_px: 16 }, () => undefined, () => undefined).then(next => {
        if (disposed) next.close();
        else {
          stream = next;
          registration.current?.(next, true);
        }
      });
      return () => {
        disposed = true;
        if (stream) registration.current?.(stream, false);
        stream?.close();
      };
    }, [client, request.session_id, request.pane_id, deferAttachment]);
    return <button ref={input} type="button" data-testid={`terminal-${request.pane_id}`} data-deferred={String(Boolean(deferAttachment))} onClick={onSelect}>terminal</button>;
  },
}));

vi.mock("./context/ContextViewer", async importOriginal => {
  const actual = await importOriginal<typeof ContextViewerModule>();
  return {
    ...actual,
    ContextViewer: (props: ContextViewerProps) => props.context
      ? <button type="button" data-testid="files-content">Files content</button>
      : <actual.ContextViewer {...props} />,
  };
});
vi.mock("./review/ReviewViewer", () => ({
  ReviewViewer: () => <button type="button" data-testid="review-content">Review content</button>,
}));
vi.mock("./browser/BrowserPane", () => ({
  BrowserPane: ({ target, onInteractionFocus }: { target: { tab_id: string | null }; onInteractionFocus?: () => void }) =>
    <button type="button" data-testid={`browser-${target.tab_id}`} onFocus={onInteractionFocus} onClick={onInteractionFocus}>Browser content</button>,
}));

type Deferred<T> = {
  promise: Promise<T>;
  resolve(value: T): void;
  reject(error: unknown): void;
};

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, resolve, reject };
}

const status: StatusResponse = {
  protocol_version: "20",
  cockpit_version: "test",
  mode: "test",
  capabilities: { terminal_mouse_input: false },
  herdr: { status: "compatible", identity: { version: "0.8.2", protocol: 20, schema_version: 1 } },
};

function sessions(includeSecond = true): SessionSummary[] {
  return [
    { id: "session-1", label: "Alpha", is_default: true, running: true },
    ...(includeSecond ? [{ id: "session-2", label: "Beta", is_default: false, running: true }] : []),
  ];
}

function snapshot(sessionId: string, focusedTabId = "tab-1", focusedPaneId = "pane-1"): SessionSnapshotResponse {
  const secondTabFocused = focusedTabId === "tab-2";
  return {
    session_id: sessionId,
    server_instance: "0123456789abcdef",
    version: "0.8.2",
    protocol: 20,
    focused_space_id: "space-1",
    focused_tab_id: focusedTabId,
    focused_pane_id: focusedPaneId,
    spaces: [{ id: "space-1", label: sessionId === "session-1" ? "Alpha space" : "Beta space", number: 1, tab_count: 2, pane_count: 2, focused: true, agent_status: "idle", git: null }],
    tabs: [
      { id: "tab-1", space_id: "space-1", label: sessionId === "session-1" ? "Alpha tab" : "Beta tab", number: 1, pane_count: 1, focused: !secondTabFocused, focused_pane_id: "pane-1" },
      { id: "tab-2", space_id: "space-1", label: "Second tab", number: 2, pane_count: 1, focused: secondTabFocused, focused_pane_id: "pane-2" },
    ],
    panes: [
      { id: "pane-1", terminal_id: "terminal-1", space_id: "space-1", tab_id: "tab-1", title: "Alpha pane", focused: focusedPaneId === "pane-1", agent: null, agent_status: "idle", revision: 1 },
      { id: "pane-2", terminal_id: "terminal-2", space_id: "space-1", tab_id: "tab-2", title: "Second pane", focused: focusedPaneId === "pane-2", agent: null, agent_status: "idle", revision: 1 },
    ],
    agents: [],
  };
}
function twoTerminalSnapshot(sessionId: string, focusedPaneId = "pane-1"): SessionSnapshotResponse {
  const base = snapshot(sessionId, "tab-1", focusedPaneId);
  return {
    ...base,
    spaces: [{ ...base.spaces[0], pane_count: 3 }],
    tabs: base.tabs.map(tab => tab.id === "tab-1" ? { ...tab, pane_count: 2, focused_pane_id: focusedPaneId } : tab),
    panes: [...base.panes, { ...base.panes[0], id: "pane-3", terminal_id: "terminal-3", title: "Third pane", focused: focusedPaneId === "pane-3" }],
  };
}

function viewerSources(sessionId: string, paneId: string): ViewerSourceOptions {
  return {
    session_id: sessionId, pane_id: paneId, tab_id: paneId === "pane-2" ? "tab-2" : "tab-1", space_id: "space-1",
    files_context_root_id: null, files_folder_root_id: "folder", review_repository_ids: ["repository"],
    roots: [{ root_id: "repository", kind: "repository", label: "Repository", path: "/repository", repository_id: "repository", checkout_path: "/repository", companion_id: null },
      { root_id: "folder", kind: "folder", label: "Files", path: "/folder", repository_id: "folder", checkout_path: "/folder", companion_id: null }],
    reason: "Context requires a configured companion directory", diagnostics: [],
  };
}

function viewerContext(sessionId: string, request: ViewerOpenRequest): ViewerContext {
  const roots = viewerSources(sessionId, request.source_pane_id).roots;
  return {
    session_id: sessionId, viewer_id: `${request.tab_id}:${request.kind}`, binding_id: `binding-${request.source_pane_id}`,
    tab_id: request.tab_id, space_id: "space-1", kind: request.kind,
    source_kind: request.kind === "review" ? "review" : "context", source_id: request.source_pane_id,
    roots, default_root_id: request.kind === "review" ? "repository" : "folder", diagnostics: [],
  };
}

function browserAssociation(tabId: string): BrowserAssociation {
  return {
    association_key: `fixture-${tabId}`, owner_id: "fixture", session_id: "session-1",
    space_id: "space-1", space_label: "Alpha space", tab_id: tabId, tab_label: tabId,
    playwright_session: `cockpit-${tabId}`, working_directory: `/browser/${tabId}`,
    profile_path: `/browser/${tabId}/profile`, invocation: "fixture",
    connection: "open", incarnation: `incarnation-${tabId}`, opened_tab: null,
  };
}

function createdSnapshot(sessionId: string): SessionSnapshotResponse {
  const base = snapshot(sessionId);
  return {
    ...base,
    tabs: [...base.tabs, { id: "tab-3", space_id: "space-1", label: "Created tab", number: 3, pane_count: 0, focused: true, focused_pane_id: null }],
    spaces: [{ ...base.spaces[0], tab_count: 3 }],
    focused_tab_id: "tab-3",
    focused_pane_id: null,
  };
}

type Subscription = {
  sessionId: string;
  onMessage: (message: SessionStreamMessage) => void;
  onError: (error: { code?: string; message?: string }) => void;
  closed: boolean;
};

class AppFixture {
  readonly snapshotCalls = vi.fn<(sessionId: string) => Promise<SessionSnapshotResponse>>();
  readonly mutateCalls = vi.fn<(sessionId: string, request: ResourceMutationRequest) => Promise<ResourceMutationResponse>>();
  readonly focusCalls = vi.fn<CockpitClient["focus"]>();
  readonly sessionsCalls = vi.fn<() => Promise<{ sessions: SessionSummary[] }>>();
  readonly viewerSources = vi.fn<CockpitClient["viewerSources"]>();
  readonly viewerOpen = vi.fn<CockpitClient["viewerOpen"]>();
  readonly subscriptions: Subscription[] = [];
  readonly mutationResponses: Array<Deferred<ResourceMutationResponse>> = [];
  private readonly snapshotQueues = new Map<string, Array<Promise<SessionSnapshotResponse>>>();
  private sessionResults: Array<Promise<{ sessions: SessionSummary[] }>> = [Promise.resolve({ sessions: sessions() })];

  readonly client: CockpitClient = {
    status: vi.fn(async () => status),
    quotaStatus: vi.fn(async () => { throw new CockpitClientError("http_error", "Subscription quota is not configured in this fixture", { status: 503, operationCode: "quota_unavailable" }); }),
    browserAction: vi.fn(async () => ({ association: null, connection: "absent" as const, message: "No browser is associated with this tab", cleanup: "none" as const, cleanup_reason: null })),
    browserCleanupStatus: vi.fn(async () => ({ failures: [] })),
    browserCleanupRetry: vi.fn(async () => ({ failures: [] })),
    browserFeedback: vi.fn(async () => { throw new Error("Unexpected browser feedback in fixture"); }),
    browserDraftRecovery: vi.fn(async () => ({ type: "none" as const })),
    acknowledgeBrowserFeedback: vi.fn(async () => { throw new Error("Unexpected browser feedback acknowledgement in fixture"); }),
    browserFeedbackImage: vi.fn(async () => { throw new Error("Unexpected browser feedback image in fixture"); }),
    sendBrowserFeedback: vi.fn(async () => { throw new Error("Unexpected browser feedback send in fixture"); }),
    resolveWorkspaceDefaults: vi.fn(async () => { throw new Error("Unexpected workspace defaults in terminal fixture"); }),
    projectConfiguration: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    providerCredentials: vi.fn(async () => { throw new Error("Unexpected provider credentials in terminal fixture"); }),
    setProviderCredential: vi.fn(async () => { throw new Error("Unexpected provider credentials in terminal fixture"); }),
    clearProviderCredential: vi.fn(async () => { throw new Error("Unexpected provider credentials in terminal fixture"); }),
    repositories: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    planWorkspace: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    startWorkspace: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    workspaceOperation: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    resumeWorkspace: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    cancelWorkspace: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    reconcileWorkspace: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    workspaceTeardownPreview: vi.fn(async () => { throw new Error("Unexpected workspace teardown in terminal fixture"); }),
    workspaceTeardownExecute: vi.fn(async () => { throw new Error("Unexpected workspace teardown in terminal fixture"); }),
    workspaceTeardownRecoveries: vi.fn(async () => { throw new Error("Unexpected workspace teardown in terminal fixture"); }),
    viewerSources: this.viewerSources,
    viewerOpen: this.viewerOpen,
    viewerRelease: vi.fn(async () => undefined),
    contextDirectory: vi.fn(async () => { throw new Error("Unexpected Context read in terminal fixture"); }),
    contextFileIndex: vi.fn(async () => { throw new Error("Unexpected Context file index in terminal fixture"); }),
    contextDocument: vi.fn(async () => { throw new Error("Unexpected Context read in terminal fixture"); }),
    contextSearch: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    reviewSnapshot: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    reviewFile: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    contextInvalidate: vi.fn(async () => { throw new Error("Unexpected Context invalidation in terminal fixture"); }),
    contextMedia: vi.fn(),
    librarySpaceList: vi.fn(async (request: { target: { session_id: string; space_id: string } }) => ({ target: request.target, companion: { status: "available" as const, companion_root_id: "companion:fixture", companion_label: "Context" }, attempts: [], rows: [], behind: 0, diagnostics: [] })),
    librarySpaceAdd: vi.fn(), librarySpaceAttemptsDismiss: vi.fn(), librarySpaceUpdate: vi.fn(), librarySpaceRemove: vi.fn(),
    commentBatches: vi.fn(async () => { throw new Error("Unexpected comments list in terminal fixture"); }),
    commentBatch: vi.fn(async () => { throw new Error("Unexpected comment batch in terminal fixture"); }),
    commentUpsert: vi.fn(async () => { throw new Error("Unexpected comment upsert in terminal fixture"); }),
    commentDiscard: vi.fn(async () => { throw new Error("Unexpected comment discard in terminal fixture"); }),
    commentRemove: vi.fn(async () => { throw new Error("Unexpected comment remove in terminal fixture"); }),
    commentAttach: vi.fn(async () => { throw new Error("Unexpected comment attach in terminal fixture"); }),
    commentPastePrepare: async () => { throw new Error("unused"); },
    commentPasteSend: async () => { throw new Error("unused"); },
    commentPasteMarkPasted: async () => { throw new Error("unused"); },
    libraryListing: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryResolve: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryConfluenceSpaces: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryAdd: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryRefresh: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryAttachments: vi.fn(async () => { throw new Error("Unexpected Library attachment operation in terminal fixture"); }),
    libraryOperation: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryOperationCancel: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryReplace: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryRemove: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryDirectory: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryFileIndex: vi.fn(async () => { throw new Error("Unexpected Library file index in terminal fixture"); }),
    libraryDocument: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryMedia: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    commentPreview: vi.fn(async () => { throw new Error("Unexpected comment preview in terminal fixture"); }),
    sessions: this.sessionsCalls,
    sessionSnapshot: this.snapshotCalls,
    spaceGitStatus: vi.fn(async (sessionId: string) => ({ session_id: sessionId, spaces: [] })),
    spaceGitAction: vi.fn(async () => { throw new Error("Unexpected Git action in terminal fixture"); }),
    focus: this.focusCalls,
    mutate: this.mutateCalls,
    subscribeSession: vi.fn(async (sessionId, onMessage, onError) => {
      const subscription: Subscription = { sessionId, onMessage, onError, closed: false };
      this.subscriptions.push(subscription);
      return { close: () => { subscription.closed = true; } };
    }),
    openTerminal: vi.fn(async () => ({ close: vi.fn(), send: vi.fn() })),
    openBrowserView: vi.fn(),
  };

  constructor() {
    this.sessionsCalls.mockImplementation(() => this.sessionResults.shift() ?? Promise.resolve({ sessions: sessions() }));
    this.snapshotCalls.mockImplementation((sessionId) => this.snapshotQueues.get(sessionId)?.shift() ?? Promise.resolve(snapshot(sessionId)));
    this.mutateCalls.mockImplementation((sessionId, request) => {
      void sessionId;
      void request;
      const response = deferred<ResourceMutationResponse>();
      this.mutationResponses.push(response);
      return response.promise;
    });
    this.focusCalls.mockImplementation(async (sessionId, request) => ({ session_id: sessionId, kind: request.kind, target_id: request.target_id, accepted: true }));
    this.viewerSources.mockImplementation(async (sessionId, paneId) => viewerSources(sessionId, paneId));
    this.viewerOpen.mockImplementation(async (sessionId, request) => viewerContext(sessionId, request));
  }

  queueSnapshot(sessionId: string, result: Promise<SessionSnapshotResponse>): void {
    const queue = this.snapshotQueues.get(sessionId) ?? [];
    queue.push(result);
    this.snapshotQueues.set(sessionId, queue);
  }

  queueSessions(result: Promise<{ sessions: SessionSummary[] }>): void {
    this.sessionResults.push(result);
  }

  resolveMutation(index: number, value: ResourceMutationResponse): void {
    this.mutationResponses[index].resolve(value);
  }

  latestSubscription(sessionId: string): Subscription {
    const current = [...this.subscriptions].reverse().find((candidate) => candidate.sessionId === sessionId && !candidate.closed);
    if (!current) throw new Error(`No active subscription for ${sessionId}`);
    return current;
  }

  emitSnapshot(sessionId: string, generation: number, sequence: number, next: SessionSnapshotResponse): void {
    this.latestSubscription(sessionId).onMessage({ type: "snapshot", session_id: sessionId, generation, sequence, snapshot: next });
  }

  emitError(sessionId: string, code = "stream_disconnected", message = "stream disconnected"): void {
    this.latestSubscription(sessionId).onError({ code, message });
  }
}

let root: Root | null = null;
let container: HTMLDivElement;

afterEach(() => {
  if (root) {
    act(() => root?.unmount());
    root = null;
  }
  terminalReadyCallbacks.clear();
  container.remove();
  vi.restoreAllMocks();
});

async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}


async function mount(fixture: AppFixture): Promise<void> {
  container = document.createElement("div");
  document.body.append(container);
  await act(async () => {
    root = createRoot(container);
    root.render(<App client={fixture.client} />);
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
  await settle();
  fixture.emitSnapshot("session-1", 1, 1, snapshot("session-1"));
  await settle();
  readyTerminal("pane-1");
  await settle();
}

function button(label: string): HTMLButtonElement {
  // A Commands row also holds its key chips, so it is named by its label span.
  const match = [...container.querySelectorAll<HTMLButtonElement>("button")].find((candidate) => candidate.getAttribute("aria-label") === label || candidate.textContent?.trim() === label || candidate.querySelector(".command-row-label > span")?.textContent === label);
  if (!match) throw new Error(`Missing button ${label}`);
  return match;
}

function click(element: HTMLElement): void {
  act(() => element.click());
}

function openLocalPaneMenu(): void {
  const pane = container.querySelector<HTMLElement>('[data-leaf-id][data-selected="true"]');
  if (!pane) throw new Error("No visible pane");
  act(() => pane.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 24, clientY: 24 })));
}

function selectedTab(): string {
  const selected = container.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
  if (!selected) throw new Error("No selected tab");
  return selected.getAttribute("aria-label") ?? "";
}

function leaf(id: string): HTMLElement {
  const host = container.querySelector<HTMLElement>(`[data-leaf-id="${id}"]`);
  if (!host) throw new Error(`Missing leaf ${id}`);
  return host;
}

function selectedLeaf(): string {
  const host = container.querySelector<HTMLElement>('[data-leaf-id][data-selected="true"]');
  if (!host?.dataset.leafId) throw new Error("No selected leaf");
  return host.dataset.leafId;
}

function leafButton(id: string, label: string): HTMLButtonElement {
  const match = leaf(id).querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
  if (!match) throw new Error(`Missing ${label} in ${id}`);
  return match;
}

async function openCommand(label: string): Promise<void> {
  click(button("Commands"));
  await settle();
  click(button("All commands"));
  click(button(label));
  await settle();
}
function readyTerminal(paneId: string): void {
  const ready = terminalReadyCallbacks.get(paneId);
  if (!ready) throw new Error(`Missing terminal readiness callback for ${paneId}`);
  act(() => ready());
}

function mutationResponse(sessionId: string, next = snapshot(sessionId)): ResourceMutationResponse {
  return { session_id: sessionId, snapshot: next, created: null };
}

function selectSession(sessionId: string): void {
  const choice = container.querySelector<HTMLButtonElement>(`[data-session-id="${sessionId}"]`);
  if (!choice) throw new Error("Missing session choice");
  click(choice);
}
async function advanceTimers(milliseconds: number): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(milliseconds);
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function exhaustAutomaticRecovery(fixture: AppFixture): Promise<void> {
  for (const delay of [250, 500, 1000]) {
    fixture.snapshotCalls.mockRejectedValueOnce(Object.assign(new Error("snapshot unavailable"), { code: "snapshot_error" }));
    await advanceTimers(delay);
  }
}

describe("mounted App mutation and session ordering", () => {
  it("prefers Git's current branch and upstream position over outdated Herdr metadata", async () => {
    const fixture = new AppFixture();
    vi.mocked(fixture.client.spaceGitStatus).mockResolvedValue({ session_id: "session-1", spaces: [{ space_id: "space-1", source: "pane_folder", checkout: { state: "branch", root: "/repo", branch: "main", upstream: { state: "tracked", name: "origin/main", ahead: 2, behind: 1 } } }] });
    await mount(fixture);
    const next = snapshot("session-1");
    next.spaces[0].git = { repository_key: "/repo", repository: "repo", branch: "outdated", checkout_path: "/repo", is_linked_worktree: false };
    act(() => fixture.emitSnapshot("session-1", 1, 2, next));
    await settle();

    expect(fixture.client.spaceGitStatus).toHaveBeenCalledWith("session-1", expect.any(AbortSignal));
    const line = container.querySelector(".space-branch");
    expect(line?.querySelector(".space-branch-name")?.textContent).toBe("main");
    expect(line?.querySelector(".space-ahead-behind")?.textContent).toBe("↑2 ↓1");
    expect(container.querySelector('[data-row-id="space-1"]')?.getAttribute("aria-label")).toContain("branch main");
  });

  it("runs a nonselected row's push without changing Herdr focus and refreshes its result", async () => {
    const fixture = new AppFixture();
    const git = { space_id: "space-2", source: "pane_folder" as const, checkout: { state: "branch" as const, root: "/child", branch: "feature", upstream: { state: "tracked" as const, name: "origin/main", ahead: 1, behind: 0 } } };
    vi.mocked(fixture.client.spaceGitStatus).mockResolvedValue({ session_id: "session-1", spaces: [git] });
    const action = deferred<SpaceGitActionResponse>();
    vi.mocked(fixture.client.spaceGitAction).mockReturnValue(action.promise);
    await mount(fixture);
    const next = snapshot("session-1");
    next.spaces.push({ ...next.spaces[0], id: "space-2", label: "Child checkout", focused: false });
    act(() => fixture.emitSnapshot("session-1", 1, 2, next));
    await settle();
    const selectBefore = fixture.focusCalls.mock.calls.length;
    const readsBefore = vi.mocked(fixture.client.spaceGitStatus).mock.calls.length;
    click(container.querySelector<HTMLButtonElement>('[data-space-id="space-2"] [data-git-action="push"]')!);
    expect(fixture.client.spaceGitAction).toHaveBeenCalledWith("session-1", { space_id: "space-2", action: "push", expected_root: "/child", expected_branch: "feature", expected_upstream: "origin/main" });
    expect(fixture.focusCalls).toHaveBeenCalledTimes(selectBefore);
    expect(container.querySelector('[data-row-id="space-1"]')?.getAttribute("aria-current")).toBe("true");
    expect(container.querySelector('[data-space-id="space-2"] .space-git-action')?.getAttribute("aria-busy")).toBe("true");
    await act(async () => action.resolve({ session_id: "session-1", space_id: "space-2", action: "push", root: "/child", branch: "feature", upstream: "origin/main", outcome: { result: "updated", commits: 1 } }));
    await settle();
    expect(vi.mocked(fixture.client.spaceGitStatus).mock.calls.length).toBe(readsBefore + 1);
    expect(container.querySelector('[data-space-id="space-2"] .git-row-note')?.textContent).toContain("feature → origin/main");
    expect(fixture.focusCalls).toHaveBeenCalledTimes(selectBefore);
  });

  it("Commands pulls the selected checkout while a row menu targets its own checkout", async () => {
    const fixture = new AppFixture();
    const status = (space_id: string, root: string, branch: string) => ({ space_id, source: "pane_folder" as const, checkout: { state: "branch" as const, root, branch, upstream: { state: "tracked" as const, name: `origin/${branch}`, ahead: 1, behind: 0 } } });
    vi.mocked(fixture.client.spaceGitStatus).mockResolvedValue({ session_id: "session-1", spaces: [status("space-1", "/parent", "main"), status("space-2", "/child", "feature")] });
    vi.mocked(fixture.client.spaceGitAction).mockImplementation(async (sessionId, request) => ({ session_id: sessionId, space_id: request.space_id, action: request.action, root: request.expected_root, branch: request.expected_branch, upstream: request.expected_upstream, outcome: { result: "up_to_date" } }));
    await mount(fixture);
    const next = snapshot("session-1");
    next.spaces.push({ ...next.spaces[0], id: "space-2", label: "Child checkout", focused: false });
    act(() => fixture.emitSnapshot("session-1", 1, 2, next));
    await settle();
    await openCommand("Pull Space (fast-forward only)");
    expect(fixture.client.spaceGitAction).toHaveBeenLastCalledWith("session-1", expect.objectContaining({ space_id: "space-1", action: "pull", expected_root: "/parent" }));
    act(() => container.querySelector('[data-row-id="space-2"]')!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 24, clientY: 24 })));
    await settle();
    const push = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(element => element.querySelector("span")?.textContent === "Push");
    click(push!);
    await settle();
    expect(fixture.client.spaceGitAction).toHaveBeenLastCalledWith("session-1", expect.objectContaining({ space_id: "space-2", action: "push", expected_root: "/child" }));
  });

  it("opens Review as a local tab viewer from the command overlay", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    await openCommand("Open Review");

    expect(selectedLeaf()).toBe("tab-1:review");
    expect(container.querySelector('[data-testid="review-content"]')).not.toBeNull();
    expect(terminal("pane-1")).not.toBeNull();
    expect(fixture.viewerOpen).toHaveBeenCalledWith("session-1", {
      tab_id: "tab-1", kind: "review", source_pane_id: "pane-1",
      source: { kind: "review", repository_id: "repository" }, client_id: expect.any(String),
    });
    click(leafButton("tab-1:review", "Close Review"));
    await settle();
    expect(container.querySelector('[data-leaf-id="tab-1:review"]')).toBeNull();
    expect(selectedLeaf()).toBe("pane-1");
    expect(fixture.client.viewerRelease).toHaveBeenCalledWith("session-1", "tab-1:review");
    expect(fixture.mutateCalls).not.toHaveBeenCalled();
  });

  it("opens files from the local pane menu without a task companion", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    openLocalPaneMenu();
    await settle();
    expect(button("Files").disabled).toBe(false);
    expect(container.querySelector<HTMLButtonElement>('.context-menu button:has(.context-menu-reason)')?.disabled).toBe(true);
    click(button("Files"));
    await settle();

    expect(selectedLeaf()).toBe("tab-1:files");
    expect(container.querySelector('[data-testid="files-content"]')).not.toBeNull();
    expect(terminal("pane-1")).not.toBeNull();
    expect(fixture.mutateCalls).not.toHaveBeenCalled();
  });

  it("keeps local pane actions and Commands available for the selected single pane", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    expect(button("Set up a task Space").closest(".spaces-section .sidebar-section-heading")).not.toBeNull();
    expect(container.querySelector(".sidebar-divider")).toBeNull();
    expect(button("Commands").closest(".tab-strip")).toBeNull();
    expect(button("Commands").closest(".tab-strip-actions")).not.toBeNull();
    expect([...container.querySelectorAll("button")].some((candidate) => candidate.textContent?.trim() === "Pane")).toBe(false);
    openLocalPaneMenu();
    expect(container.querySelector('[role="menu"][aria-label="pane actions"]')).not.toBeNull();
    click(button("Commands"));
    expect(container.querySelector(".command-overlay")).not.toBeNull();
  });

  it("disables local pane actions while a mutation is pending", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    openLocalPaneMenu();
    expect(button("Rename pane…").disabled).toBe(false);
    click(button("Create tab"));
    expect(button("Rename pane…").disabled).toBe(true);
  });

  it("paints the requested tab while Herdr confirms focus", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    const tab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-label="Tab 2: Second tab"]');
    if (!tab) throw new Error("Missing second tab");
    click(tab);
    await settle();
    expect(selectedTab()).toBe("Tab 2: Second tab");

    fixture.emitSnapshot("session-1", 1, 2, snapshot("session-1", "tab-2", "pane-2"));
    await settle();
    expect(selectedTab()).toBe("Tab 2: Second tab");
  });
  it("retries the retained focus intent after a recovery snapshot", async () => {
    vi.useFakeTimers();
    try {
      const fixture = new AppFixture();
      await mount(fixture);

      const tab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-label="Tab 2: Second tab"]');
      if (!tab) throw new Error("Missing second tab");
      click(tab);
      await settle();
      readyTerminal("pane-2");
      await settle();
      expect(fixture.focusCalls).toHaveBeenLastCalledWith("session-1", { kind: "tab", target_id: "tab-2" });

      await advanceTimers(500);
      await settle();
      fixture.emitSnapshot("session-1", 1, 1, snapshot("session-1"));
      await settle();

      expect(fixture.focusCalls).toHaveBeenCalledTimes(2);
      expect(fixture.focusCalls).toHaveBeenLastCalledWith("session-1", { kind: "tab", target_id: "tab-2" });
      expect(selectedTab()).toBe("Tab 2: Second tab");
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps attached terminals attached while the session resyncs after a mutation", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    const terminal = () => container.querySelector<HTMLElement>('[data-testid="terminal-pane-1"]');
    expect(terminal()?.dataset.deferred).toBe("false");

    click(button("Create tab"));
    const followUp = deferred<SessionSnapshotResponse>();
    fixture.queueSnapshot("session-1", followUp.promise);
    fixture.resolveMutation(0, mutationResponse("session-1", snapshot("session-1")));
    await settle();

    expect(container.querySelector(".session-chip")?.textContent).toBe("Resyncing");
    expect(terminal()?.dataset.deferred).toBe("false");
  });

  it("keeps a newer stream event when the delayed mutation response arrives after it", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    click(button("Create tab"));
    expect(fixture.mutateCalls).toHaveBeenCalledTimes(1);
    fixture.emitSnapshot("session-1", 1, 2, snapshot("session-1", "tab-2", "pane-2"));
    await settle();
    expect(selectedTab()).toBe("Tab 2: Second tab");

    const followUp = deferred<SessionSnapshotResponse>();
    fixture.queueSnapshot("session-1", followUp.promise);
    fixture.resolveMutation(0, mutationResponse("session-1", snapshot("session-1")));
    await settle();

    expect(selectedTab()).toBe("Tab 2: Second tab");
    expect(fixture.snapshotCalls).toHaveBeenCalledWith("session-1", expect.any(AbortSignal));
  });

  it("lets an ordered event after the mutation response win over the old response snapshot", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    click(button("Create tab"));
    const followUp = deferred<SessionSnapshotResponse>();
    fixture.queueSnapshot("session-1", followUp.promise);
    fixture.resolveMutation(0, mutationResponse("session-1", snapshot("session-1")));
    await settle();

    followUp.resolve(snapshot("session-1"));
    await settle();
    fixture.emitSnapshot("session-1", 1, 1, snapshot("session-1", "tab-2", "pane-2"));
    await settle();

    expect(selectedTab()).toBe("Tab 2: Second tab");
    expect(fixture.mutateCalls).toHaveBeenCalledTimes(1);
  });

  it("ignores a mutation response from the session that the user left", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    click(button("Create tab"));
    click(button("Commands"));
    click(button("Switch session…"));
    await settle();
    selectSession("session-2");
    click(button("Switch"));
    await settle();
    fixture.emitSnapshot("session-2", 1, 1, snapshot("session-2"));
    await settle();

    fixture.resolveMutation(0, mutationResponse("session-1", createdSnapshot("session-1")));
    await settle();

    expect(container.textContent).toContain("Beta space");
    expect(container.textContent).toContain("Beta tab");
    expect(container.textContent).not.toContain("Created tab");
  });

  it("preserves a newer local focus over an external focus and stale mutation response", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    click(button("Create tab"));
    fixture.emitSnapshot("session-1", 1, 2, snapshot("session-1", "tab-2", "pane-2"));
    await settle();
    act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "b", ctrlKey: true, bubbles: true, cancelable: true })));
    act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "p", bubbles: true, cancelable: true })));
    await settle();
    readyTerminal("pane-1");
    await settle();
    expect(fixture.focusCalls).toHaveBeenCalledWith("session-1", { kind: "tab", target_id: "tab-1" });
    fixture.emitSnapshot("session-1", 1, 3, snapshot("session-1", "tab-1", "pane-1"));
    await settle();

    const followUp = deferred<SessionSnapshotResponse>();
    fixture.queueSnapshot("session-1", followUp.promise);
    fixture.resolveMutation(0, mutationResponse("session-1", snapshot("session-1", "tab-2", "pane-2")));
    await settle();

    expect(selectedTab()).toBe("Tab 1: Alpha tab");
    expect(fixture.focusCalls).toHaveBeenCalledWith("session-1", { kind: "tab", target_id: "tab-1" });
  });

  it("keeps the last known resources and exposes resync without retry after a successful mutation whose snapshot fails", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    click(button("Create tab"));
    const followUpError = Object.assign(new Error("follow-up snapshot failed"), { code: "snapshot_error" });
    fixture.queueSnapshot("session-1", Promise.reject(followUpError));
    fixture.resolveMutation(0, mutationResponse("session-1", createdSnapshot("session-1")));
    await settle();

    const recovery = container.querySelector<HTMLElement>('[aria-label="Recovery"]');
    expect(recovery?.textContent).toContain("follow-up snapshot failed");
    expect(recovery?.querySelector("button")?.textContent).toBe("Resync");
    expect(recovery?.textContent).not.toContain("Retry");
    expect(container.textContent).toContain("Alpha tab");
  });

  it("does not let reconnect during a mutation replace the current selected resource", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    click(button("Create tab"));
    fixture.emitSnapshot("session-1", 1, 2, snapshot("session-1", "tab-2", "pane-2"));
    await settle();
    fixture.emitError("session-1");
    await settle();
    click(button("Resync"));
    const reconnectSnapshot = deferred<SessionSnapshotResponse>();
    fixture.queueSnapshot("session-1", reconnectSnapshot.promise);
    await settle();

    fixture.resolveMutation(0, mutationResponse("session-1", snapshot("session-1")));
    await settle();

    expect(selectedTab()).toBe("Tab 2: Second tab");
    expect(fixture.mutateCalls).toHaveBeenCalledTimes(1);
  });

  it("ignores an older session list after a newer refresh removed that session", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    const older = deferred<{ sessions: SessionSummary[] }>();
    const newer = deferred<{ sessions: SessionSummary[] }>();
    fixture.queueSessions(older.promise);
    fixture.queueSessions(newer.promise);

    click(button("Commands"));
    click(button("Switch session…"));
    click(button("Commands"));
    click(button("Switch session…"));
    newer.resolve({ sessions: [sessions()[1]] });
    await settle();
    fixture.emitSnapshot("session-2", 1, 1, snapshot("session-2"));
    await settle();

    older.resolve({ sessions: [sessions()[0]] });
    await settle();
    expect(container.textContent).toContain("Beta space");
    expect(selectedTab()).toBe("Tab 1: Beta tab");
  });

  it("falls back from a removed session and permits a new mutation immediately", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    click(button("Create tab"));
    fixture.queueSessions(Promise.resolve({ sessions: [sessions()[1]] }));
    fixture.emitError("session-1");
    await settle();
    click(button("Resync"));
    await settle();
    fixture.emitSnapshot("session-2", 1, 1, snapshot("session-2"));
    await settle();

    click(button("Create tab"));
    expect(fixture.mutateCalls).toHaveBeenCalledTimes(2);
    expect(fixture.mutateCalls.mock.calls[1]?.[0]).toBe("session-2");
  });
  it("stops automatic recovery after three fired attempts and keeps explicit Resync available", async () => {
    vi.useFakeTimers();
    try {
      const fixture = new AppFixture();
      await mount(fixture);
      const initialCalls = fixture.snapshotCalls.mock.calls.length;
      fixture.emitError("session-1");
      await settle();
      await exhaustAutomaticRecovery(fixture);
      expect(fixture.snapshotCalls.mock.calls.length).toBe(initialCalls + 3);
      await advanceTimers(5_000);
      expect(fixture.snapshotCalls.mock.calls.length).toBe(initialCalls + 3);
      expect(button("Resync")).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });

  it("renews the automatic recovery budget only after a stable ordered live stream", async () => {
    vi.useFakeTimers();
    try {
      const fixture = new AppFixture();
      await mount(fixture);
      const initialCalls = fixture.snapshotCalls.mock.calls.length;
      fixture.emitError("session-1");
      await settle();
      fixture.queueSnapshot("session-1", Promise.resolve(snapshot("session-1")));
      await advanceTimers(250);
      fixture.emitSnapshot("session-1", 1, 1, snapshot("session-1"));
      await settle();
      await advanceTimers(900);
      fixture.emitSnapshot("session-1", 2, 1, snapshot("session-1"));
      await settle();
      await advanceTimers(200);

      fixture.emitError("session-1");
      await settle();
      fixture.queueSnapshot("session-1", Promise.resolve(snapshot("session-1")));
      await advanceTimers(250);
      expect(fixture.snapshotCalls.mock.calls.length).toBe(initialCalls + 1);
      await advanceTimers(250);
      expect(fixture.snapshotCalls.mock.calls.length).toBe(initialCalls + 2);
      fixture.emitSnapshot("session-1", 1, 1, snapshot("session-1"));
      await settle();
      await advanceTimers(1_000);
      fixture.emitError("session-1");
      await settle();
      await advanceTimers(250);
      expect(fixture.snapshotCalls.mock.calls.length).toBe(initialCalls + 3);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not renew retries for streams that disconnect just after their initial snapshot", async () => {
    vi.useFakeTimers();
    try {
      const fixture = new AppFixture();
      await mount(fixture);
      const initialCalls = fixture.snapshotCalls.mock.calls.length;
      fixture.emitError("session-1");
      await settle();
      for (const delay of [250, 500, 1000]) {
        await advanceTimers(delay);
        fixture.emitSnapshot("session-1", 1, 1, snapshot("session-1"));
        await settle();
        await advanceTimers(100);
        fixture.emitError("session-1");
        await settle();
      }
      expect(fixture.snapshotCalls.mock.calls.length).toBe(initialCalls + 3);
      await advanceTimers(5_000);
      expect(fixture.snapshotCalls.mock.calls.length).toBe(initialCalls + 3);
      expect(button("Resync").disabled).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("allows manual Resync to recover after automatic attempts are exhausted", async () => {
    vi.useFakeTimers();
    try {
      const fixture = new AppFixture();
      await mount(fixture);
      const initialCalls = fixture.snapshotCalls.mock.calls.length;
      fixture.emitError("session-1");
      await settle();
      await exhaustAutomaticRecovery(fixture);
      expect(fixture.snapshotCalls.mock.calls.length).toBe(initialCalls + 3);

      click(button("Resync"));
      await settle();
      expect(fixture.snapshotCalls.mock.calls.length).toBe(initialCalls + 4);
      fixture.emitSnapshot("session-1", 1, 1, snapshot("session-1"));
      await settle();
      expect(container.querySelector('[aria-label="Recovery"]')).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("cancels recovery timers when switching sessions or unmounting", async () => {
    vi.useFakeTimers();
    try {
      const fixture = new AppFixture();
      await mount(fixture);
      const initialCalls = fixture.snapshotCalls.mock.calls.length;
      fixture.emitError("session-1");
      await settle();

      click(button("Commands"));
      click(button("Switch session…"));
      await settle();
      selectSession("session-2");
      click(button("Switch"));
      await settle();
      await advanceTimers(2_000);
      expect(fixture.snapshotCalls.mock.calls.filter(([sessionId]) => sessionId === "session-1")).toHaveLength(1);
      expect(fixture.snapshotCalls.mock.calls.length).toBe(initialCalls + 1);

      fixture.emitError("session-2");
      await settle();
      const beforeUnmount = fixture.snapshotCalls.mock.calls.length;
      act(() => root?.unmount());
      root = null;
      await advanceTimers(2_000);
      expect(fixture.snapshotCalls.mock.calls.length).toBe(beforeUnmount);
    } finally {
      vi.useRealTimers();
    }
  });

});

function emptyLibrary(fixture: AppFixture): void {
  vi.mocked(fixture.client.libraryListing).mockResolvedValue({
    root: { root_id: "library:fixture", kind: "library", label: "Library", path: "/data/cockpit/library", repository_id: "", checkout_path: "", companion_id: null },
    generation: "1", items: [], follows: [], next_offset: null, diagnostics: [],
  });
}

async function openLibraryFromPalette(): Promise<void> {
  act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "b", ctrlKey: true, bubbles: true, cancelable: true })));
  act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "?", bubbles: true, cancelable: true })));
  await settle();
  click(button("All commands"));
  click(button("Open Library"));
  await settle();
}

function terminal(paneId: string): HTMLElement | null {
  return container.querySelector<HTMLElement>(`[data-testid="terminal-${paneId}"]`);
}

describe("Library view presentation lifecycle", () => {
  it("unmounts leaf content while open and returns focus to the still-mounted sidebar invoker without terminal focus", async () => {
    const fixture = new AppFixture();
    emptyLibrary(fixture);
    await mount(fixture);
    fixture.focusCalls.mockClear();
    const invoker = container.querySelector<HTMLButtonElement>(".space-tree-row .resource-select")!;
    act(() => invoker.focus());

    await openLibraryFromPalette();
    expect(container.querySelector('section[aria-label="Library"]')).not.toBeNull();
    expect(terminal("pane-1")).toBeNull();
    // The selected Space is the view's `Add to <Space>` target; reading its copies asks Herdr for nothing.
    expect(fixture.client.librarySpaceList).toHaveBeenCalledWith({ target: { session_id: "session-1", space_id: "space-1" } }, expect.any(AbortSignal));
    expect(container.contains(document.activeElement)).toBe(true);
    expect(document.activeElement).not.toBe(invoker);

    click(button("Close Library"));
    await settle();
    expect(document.activeElement).toBe(invoker);
    readyTerminal("pane-1");
    await settle();
    expect(document.activeElement).toBe(invoker);
    expect(fixture.focusCalls).not.toHaveBeenCalled();
  });

  it.each(["pointer", "Escape", "toggle"])("returns keyboard focus to the selected terminal after a %s close", async (close) => {
    const fixture = new AppFixture();
    emptyLibrary(fixture);
    await mount(fixture);
    fixture.focusCalls.mockClear();
    act(() => terminal("pane-1")!.focus());

    await openLibraryFromPalette();
    expect(terminal("pane-1")).toBeNull();
    if (close === "pointer") click(button("Close Library"));
    else if (close === "Escape") press(document.activeElement!, "Escape");
    else prefix(document.activeElement!, "i");
    await settle();
    readyTerminal("pane-1");
    await settle();
    expect(document.activeElement).toBe(terminal("pane-1"));
    expect(fixture.focusCalls).not.toHaveBeenCalled();
  });

  it("returns keyboard focus to the selected terminal when the toolbar opened the Library", async () => {
    const fixture = new AppFixture();
    emptyLibrary(fixture);
    await mount(fixture);
    fixture.focusCalls.mockClear();
    const invoker = container.querySelector<HTMLButtonElement>(".tab-icon-button[aria-label*='Library']")!;
    act(() => invoker.focus());
    click(invoker);
    await settle();
    click(button("Close Library"));
    await settle();
    expect(document.activeElement).toBe(terminal("pane-1"));
    expect(fixture.focusCalls).not.toHaveBeenCalled();
  });

  it("returns to the selected terminal after a popup closes following a Library close", async () => {
    const frames: FrameRequestCallback[] = [];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => { frames.push(callback); return frames.length; });
    const fixture = new AppFixture();
    emptyLibrary(fixture);
    await mount(fixture);
    fixture.focusCalls.mockClear();
    const invoker = container.querySelector<HTMLButtonElement>(".space-tree-row .resource-select")!;
    act(() => invoker.focus());
    await openLibraryFromPalette();
    click(button("Close Library"));
    await settle();
    expect(document.activeElement).toBe(invoker);

    const next = snapshot("session-1");
    next.herdr_shell = { status: "live", prefix_bindings: ["ctrl+b"], commands: [], popup: { terminal_id: "popup", title: "Inbox", width: null, height: null }, error: null };
    act(() => fixture.emitSnapshot("session-1", 1, 2, next));
    await settle();
    expect(container.querySelector("[data-server-modal]")).not.toBeNull();
    act(() => fixture.emitSnapshot("session-1", 1, 3, snapshot("session-1")));
    await settle();
    frames.splice(0).forEach(callback => callback(0));
    expect(document.activeElement).toBe(terminal("pane-1"));
    expect(fixture.focusCalls).not.toHaveBeenCalled();
  });

  it("opens the Library from the no-session screen without any Space action", async () => {
    const fixture = new AppFixture();
    emptyLibrary(fixture);
    fixture.sessionsCalls.mockImplementation(async () => ({ sessions: [] }));
    container = document.createElement("div");
    document.body.append(container);
    await act(async () => {
      root = createRoot(container);
      root.render(<App client={fixture.client} />);
    });
    await settle();
    await settle();
    expect(container.textContent).toContain("No Herdr sessions");

    click(button("Open Library"));
    await settle();
    expect(container.querySelector('section[aria-label="Library"]')).not.toBeNull();
    expect(container.textContent).toContain("The Library is empty");
    expect(container.textContent).toContain("Confluence page");
    expect([...container.querySelectorAll("button")].some((candidate) => candidate.textContent?.startsWith("Add to "))).toBe(false);
    expect(fixture.client.libraryListing).toHaveBeenCalled();

    click(button("Close Library"));
    await settle();
    expect(document.activeElement).toBe(button("Open Library"));
    expect(fixture.snapshotCalls).not.toHaveBeenCalled();
  });
});

function press(target: Element | Window, key: string, init: KeyboardEventInit = {}): void {
  act(() => { target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init })); });
}

function prefix(target: Element | Window, key: string, init: KeyboardEventInit = {}): void {
  press(target, "b", { ctrlKey: true });
  press(target, key, init);
}

describe("keyboard prefix in the workbench", () => {
  it("opens the Library from a focused terminal with Ctrl+B i and returns focus to that terminal on the same key", async () => {
    const fixture = new AppFixture();
    emptyLibrary(fixture);
    await mount(fixture);
    act(() => terminal("pane-1")!.focus());

    prefix(terminal("pane-1")!, "i");
    await settle();
    expect(container.querySelector('section[aria-label="Library"]')).not.toBeNull();
    expect(terminal("pane-1")).toBeNull();

    prefix(document.activeElement!, "i");
    await settle();
    expect(container.querySelector('section[aria-label="Library"]')).toBeNull();
    expect(document.activeElement).toBe(terminal("pane-1"));
  });


  it("invokes advertised commands with confirmed Herdr focus while keeping the Library open", async () => {
    const fixture = new AppFixture();
    emptyLibrary(fixture);
    await mount(fixture);
    const next = snapshot("session-1");
    next.herdr_shell = { status: "live", prefix_bindings: ["ctrl+b"], commands: [{ command_id: "opaque-command", binding_labels: ["prefix+alt+a"], action: "plugin_action", description: "Configured action" }], popup: null, error: null };
    act(() => fixture.emitSnapshot("session-1", 1, 2, next));
    await settle();
    prefix(window, "i");
    await settle();
    expect(container.querySelector('section[aria-label="Library"]')).not.toBeNull();
    prefix(window, "å", { altKey: true, code: "KeyA" });
    await settle();
    expect(fixture.mutateCalls).toHaveBeenCalledExactlyOnceWith("session-1", { type: "command_invoke", command_id: "opaque-command", space_id: "space-1", tab_id: "tab-1", pane_id: "pane-1" });
    expect(fixture.focusCalls).not.toHaveBeenCalled();
    expect(container.querySelector('section[aria-label="Library"]')).not.toBeNull();
  });

  it("shows a not-bound hint for a key after the prefix that Cockpit does not use", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    prefix(window, "o");
    expect(container.querySelector(".prefix-indicator")?.textContent).toBe("Ctrl+B o is not bound in Cockpit");
    expect(fixture.mutateCalls).not.toHaveBeenCalled();
  });

  it("lists every Commands row with its key as chips and finds a row by its key", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    prefix(window, "?");
    await settle();
    click(button("All commands"));
    const row = [...container.querySelectorAll<HTMLElement>(".command-row")].find((candidate) => candidate.textContent?.includes("Open Library"))!;
    expect([...row.querySelectorAll("kbd")].map((chip) => chip.textContent)).toEqual(["Ctrl+B", "i"]);
    expect(container.querySelector(".command-row")).not.toBeNull();
    const search = container.querySelector<HTMLInputElement>(".command-search")!;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => { setValue.call(search, "swap"); search.dispatchEvent(new Event("input", { bubbles: true })); });
    const labels = [...container.querySelectorAll(".command-row-label")].map((label) => label.textContent);
    expect(labels).toEqual(expect.arrayContaining([expect.stringContaining("Swap pane left")]));
  });

  it("moves a pane chooser selection with Ctrl+N/P", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    openLocalPaneMenu();
    click(button("Move to…"));
    const destination = container.querySelector<HTMLSelectElement>('select[aria-label="Move destination"]')!;
    expect(destination.value).toBe("");
    press(destination, "n", { ctrlKey: true });
    expect(destination.value).toBe("new-tab");
    press(destination, "n", { ctrlKey: true });
    expect(destination.value).toBe("new-space");
    press(destination, "p", { ctrlKey: true });
    expect(destination.value).toBe("new-tab");
  });

  it("toggles the sidebar with Ctrl+B b", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    const workbench = container.querySelector(".workbench")!;
    expect(workbench.classList.contains("sidebar-collapsed")).toBe(false);
    prefix(window, "b");
    expect(workbench.classList.contains("sidebar-collapsed")).toBe(true);
    prefix(window, "b");
    expect(workbench.classList.contains("sidebar-collapsed")).toBe(false);
  });
});

describe("pane-scoped prefix commands over the Library", () => {
  it("shows the pane before a destructive confirmation names it", async () => {
    const fixture = new AppFixture();
    emptyLibrary(fixture);
    await mount(fixture);
    act(() => terminal("pane-1")!.focus());
    const libraryOpenAtConfirm: boolean[] = [];
    const confirm = vi.spyOn(window, "confirm").mockImplementation((message) => {
      libraryOpenAtConfirm.push(container.querySelector('section[aria-label="Library"]') !== null && message !== undefined);
      return false;
    });
    try {
      prefix(terminal("pane-1")!, "i");
      await settle();
      expect(container.querySelector('section[aria-label="Library"]')).not.toBeNull();
      prefix(document.activeElement!, "x");
      await act(async () => { await new Promise((resolve) => setTimeout(resolve, 80)); });
      expect(confirm).toHaveBeenCalledExactlyOnceWith("Close Alpha pane?");
      expect(libraryOpenAtConfirm).toEqual([false]);
      expect(fixture.mutateCalls).not.toHaveBeenCalled();
    } finally {
      confirm.mockRestore();
    }
  });
});

describe("tab-local viewers, focus and placement", () => {
  it("keeps a viewer selected through unchanged ordered snapshots without requesting Herdr focus", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    fixture.focusCalls.mockClear();
    await openCommand("Open Files");
    const content = container.querySelector('[data-testid="files-content"]');
    expect(selectedLeaf()).toBe("tab-1:files");

    fixture.emitSnapshot("session-1", 1, 2, snapshot("session-1"));
    await settle();
    fixture.emitSnapshot("session-1", 1, 3, snapshot("session-1"));
    await settle();

    expect(selectedLeaf()).toBe("tab-1:files");
    expect(container.querySelector('[data-testid="files-content"]')).toBe(content);
    expect(fixture.focusCalls).not.toHaveBeenCalled();
  });

  it("follows external terminal focus and restores a viewer's local zoom", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    fixture.emitSnapshot("session-1", 1, 2, twoTerminalSnapshot("session-1"));
    await settle();
    await openCommand("Open Files");
    click(leafButton("tab-1:files", "Zoom this pane"));
    expect(terminal("pane-3")).toBeNull();

    fixture.emitSnapshot("session-1", 1, 3, twoTerminalSnapshot("session-1", "pane-3"));
    await settle();

    expect(selectedLeaf()).toBe("pane-3");
    expect(terminal("pane-1")).not.toBeNull();
    expect(terminal("pane-3")).not.toBeNull();
    expect(container.querySelector(".pane-zoom-bar")).toBeNull();
    expect(fixture.mutateCalls).not.toHaveBeenCalled();
  });

  it("does not replace a later viewer selection with an already-issued terminal focus echo", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    fixture.emitSnapshot("session-1", 1, 2, twoTerminalSnapshot("session-1"));
    await settle();
    click(terminal("pane-3")!);
    await settle();
    expect(fixture.focusCalls).toHaveBeenCalledWith("session-1", { kind: "pane", target_id: "pane-3" });
    await openCommand("Open Files");

    fixture.emitSnapshot("session-1", 1, 3, twoTerminalSnapshot("session-1", "pane-3"));
    await settle();

    expect(selectedLeaf()).toBe("tab-1:files");
    expect(container.querySelector('[data-testid="files-content"]')).not.toBeNull();
  });

  it("splits from a viewer using the last real terminal and places only the receipted pane beside that viewer", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    fixture.emitSnapshot("session-1", 1, 2, twoTerminalSnapshot("session-1"));
    await settle();
    click(terminal("pane-3")!);
    await settle();
    await openCommand("Open Files");
    expect(fixture.viewerOpen).toHaveBeenLastCalledWith("session-1", expect.objectContaining({ source_pane_id: "pane-3" }));
    prefix(window, "-");
    expect(fixture.mutateCalls).toHaveBeenCalledWith("session-1", { type: "pane_split", pane_id: "pane-3", direction: "down", ratio: null });

    const base = twoTerminalSnapshot("session-1");
    const next: SessionSnapshotResponse = {
      ...base, focused_pane_id: "pane-4",
      spaces: [{ ...base.spaces[0], pane_count: 5 }],
      tabs: base.tabs.map(tab => tab.id === "tab-1" ? { ...tab, pane_count: 4, focused_pane_id: "pane-4" } : tab),
      panes: [...base.panes.map(pane => ({ ...pane, focused: false })),
        { ...base.panes[0], id: "pane-4", terminal_id: "terminal-4", title: "Created pane", focused: true },
        { ...base.panes[0], id: "pane-5", terminal_id: "terminal-5", title: "External pane", focused: false }],
    };
    fixture.emitSnapshot("session-1", 1, 3, next);
    await settle();
    expect(container.querySelector('[data-leaf-id="pane-4"]')).toBeNull();
    expect(selectedLeaf()).toBe("tab-1:files");
    fixture.queueSnapshot("session-1", deferred<SessionSnapshotResponse>().promise);
    fixture.resolveMutation(0, {
      session_id: "session-1", snapshot: next,
      created: { pane_id: "pane-4", terminal_id: "terminal-4", space_id: "space-1", tab_id: "tab-1" },
    });
    await settle();

    expect(selectedLeaf()).toBe("pane-4");
    const viewer = leaf("tab-1:files");
    const created = leaf("pane-4");
    expect(created.style.left).toBe(viewer.style.left);
    expect(created.style.width).toBe(viewer.style.width);
    expect(Number.parseFloat(created.style.top)).toBeGreaterThan(Number.parseFloat(viewer.style.top));
    expect(Number.parseFloat(leaf("pane-5").style.left)).toBeGreaterThan(Number.parseFloat(created.style.left));
  });

  it("applies swap, divider resize and zoom locally while selection keeps painted control streams attached", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    fixture.emitSnapshot("session-1", 1, 2, twoTerminalSnapshot("session-1"));
    await settle();
    await openCommand("Open Files");
    const first = terminal("pane-1");
    const third = terminal("pane-3");
    const streams = vi.mocked(fixture.client.openTerminal).mock.results.map(result => result.value);
    const attachmentCount = vi.mocked(fixture.client.openTerminal).mock.calls.length;
    const before = leaf("tab-1:files").style.left;

    prefix(window, "H", { shiftKey: true });
    await settle();
    expect(leaf("tab-1:files").style.left).not.toBe(before);
    const divider = container.querySelector<HTMLElement>("[data-layout-divider]")!;
    const sizes = [leaf("pane-1").style.width, leaf("pane-3").style.width, leaf("tab-1:files").style.width];
    press(divider, "ArrowRight");
    await settle();
    expect([leaf("pane-1").style.width, leaf("pane-3").style.width, leaf("tab-1:files").style.width]).not.toEqual(sizes);
    click(terminal("pane-1")!);
    await settle();
    act(() => container.querySelector<HTMLButtonElement>('[data-testid="files-content"]')!.focus());
    await settle();
    expect(selectedLeaf()).toBe("tab-1:files");
    expect(terminal("pane-1")).toBe(first);
    expect(terminal("pane-3")).toBe(third);
    expect(fixture.client.openTerminal).toHaveBeenCalledTimes(attachmentCount);
    for (const stream of await Promise.all(streams)) expect(stream.close).not.toHaveBeenCalled();

    click(leafButton("tab-1:files", "Zoom this pane"));
    expect(container.querySelector(".pane-zoom-bar")).toBeNull();
    click(leafButton("tab-1:files", "Restore layout"));
    await settle();
    expect(terminal("pane-1")).not.toBeNull();
    expect(terminal("pane-3")).not.toBeNull();
    expect(fixture.mutateCalls).not.toHaveBeenCalled();
  });
});

describe("tab-local Browser flow", () => {
  it("keeps separate browsers in two tabs and closes only the selected tab's browser", async () => {
    const fixture = new AppFixture();
    vi.mocked(fixture.client.browserAction).mockImplementation(async request => ({
      association: browserAssociation(request.target.tab_id!), connection: request.action.kind === "close" ? "closed" : "open",
      message: "", cleanup: request.action.kind === "close" ? "done" : "none", cleanup_reason: null,
    }));
    await mount(fixture);
    click(button("Browser"));
    await settle();
    expect(selectedLeaf()).toBe("tab-1:browser");
    expect(container.querySelector('[data-testid="browser-tab-1"]')).not.toBeNull();
    expect(terminal("pane-1")).not.toBeNull();

    click(button("Tab 2: Second tab"));
    await settle();
    readyTerminal("pane-2");
    fixture.emitSnapshot("session-1", 1, 2, snapshot("session-1", "tab-2", "pane-2"));
    await settle();
    click(button("Browser"));
    await settle();
    expect(selectedLeaf()).toBe("tab-2:browser");
    expect(container.querySelector('[data-testid="browser-tab-2"]')).not.toBeNull();

    click(button("Tab 1: Alpha tab"));
    await settle();
    readyTerminal("pane-1");
    fixture.emitSnapshot("session-1", 1, 3, snapshot("session-1"));
    await settle();
    expect(selectedLeaf()).toBe("tab-1:browser");
    click(button("Browser"));
    await settle();
    expect(container.querySelector('[data-leaf-id="tab-1:browser"]')).toBeNull();
    expect(selectedLeaf()).toBe("pane-1");
    expect(fixture.client.browserAction).toHaveBeenCalledWith({ target: { session_id: "session-1", tab_id: "tab-1", pane_id: null, endpoint_path: null }, action: { kind: "close" } });
    expect(vi.mocked(fixture.client.browserAction).mock.calls.filter(([request]) => request.action.kind === "open_fresh").map(([request]) => request.target.tab_id)).toEqual(["tab-1", "tab-2"]);

    click(button("Tab 2: Second tab"));
    await settle();
    readyTerminal("pane-2");
    fixture.emitSnapshot("session-1", 1, 4, snapshot("session-1", "tab-2", "pane-2"));
    await settle();
    expect(selectedLeaf()).toBe("tab-2:browser");
    expect(container.querySelector('[data-testid="browser-tab-2"]')).not.toBeNull();
    expect(fixture.mutateCalls).not.toHaveBeenCalled();
  });
});

describe("external focus during local terminal creation", () => {
  it("follows an existing terminal immediately and does not let the created receipt steal that later selection", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    fixture.emitSnapshot("session-1", 1, 2, twoTerminalSnapshot("session-1"));
    await settle();
    await openCommand("Open Files");
    prefix(window, "-");

    fixture.emitSnapshot("session-1", 1, 3, twoTerminalSnapshot("session-1", "pane-3"));
    await settle();
    expect(selectedLeaf()).toBe("pane-3");

    const base = twoTerminalSnapshot("session-1", "pane-3");
    const next: SessionSnapshotResponse = {
      ...base,
      spaces: [{ ...base.spaces[0], pane_count: 4 }],
      tabs: base.tabs.map(tab => tab.id === "tab-1" ? { ...tab, pane_count: 3 } : tab),
      panes: [...base.panes, { ...base.panes[0], id: "pane-4", terminal_id: "terminal-4", title: "Created pane", focused: false }],
    };
    fixture.emitSnapshot("session-1", 1, 4, next);
    await settle();
    fixture.queueSnapshot("session-1", deferred<SessionSnapshotResponse>().promise);
    fixture.resolveMutation(0, {
      session_id: "session-1", snapshot: next,
      created: { pane_id: "pane-4", terminal_id: "terminal-4", space_id: "space-1", tab_id: "tab-1" },
    });
    await settle();

    expect(selectedLeaf()).toBe("pane-3");
    expect(leaf("pane-4").style.left).toBe(leaf("tab-1:files").style.left);
    expect(Number.parseFloat(leaf("pane-4").style.top)).toBeGreaterThan(Number.parseFloat(leaf("tab-1:files").style.top));
  });
});
