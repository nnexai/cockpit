// @vitest-environment jsdom
import "./input/viewerTestLayout";

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../client/CockpitClient";
import type {
  ResourceMutationRequest,
  ResourceMutationResponse,
  PanePresentation,
  SessionSnapshotResponse,
  SessionStreamMessage,
  SessionSummary,
  StatusResponse,
} from "../protocol/generated/v1";
import { App } from "./App";

const terminalReadyCallbacks = vi.hoisted(() => new Map<string, () => void>());
vi.mock("./TerminalPane", () => ({
  TerminalPane: ({ request, onSelect, onReady }: { request: { pane_id: string }; onSelect?: () => void; onReady?: () => void }) => {
    if (onReady) terminalReadyCallbacks.set(request.pane_id, onReady);
    return <button type="button" data-testid={`terminal-${request.pane_id}`} onClick={onSelect}>terminal</button>;
  },
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
    version: "0.8.2",
    protocol: 20,
    focused_space_id: "space-1",
    focused_tab_id: focusedTabId,
    focused_pane_id: focusedPaneId,
    spaces: [{ id: "space-1", label: sessionId === "session-1" ? "Alpha space" : "Beta space", number: 1, tab_count: 2, pane_count: 2, focused: true, agent_status: "idle", git: null }],
    tabs: [
      { id: "tab-1", space_id: "space-1", label: sessionId === "session-1" ? "Alpha tab" : "Beta tab", number: 1, pane_count: 1, focused: !secondTabFocused },
      { id: "tab-2", space_id: "space-1", label: "Second tab", number: 2, pane_count: 1, focused: secondTabFocused },
    ],
    panes: [
      { id: "pane-1", terminal_id: "terminal-1", space_id: "space-1", tab_id: "tab-1", title: "Alpha pane", focused: focusedPaneId === "pane-1", agent: null, agent_status: "idle", revision: 1 },
      { id: "pane-2", terminal_id: "terminal-2", space_id: "space-1", tab_id: "tab-2", title: "Second pane", focused: focusedPaneId === "pane-2", agent: null, agent_status: "idle", revision: 1 },
    ],
    layouts: [],
    agents: [],
  };
}
function manyTabsSnapshot(sessionId: string, focusedTabId = "tab-1", focusedPaneId = "pane-1"): SessionSnapshotResponse {
  const base = snapshot(sessionId, focusedTabId, focusedPaneId);
  return {
    ...base,
    focused_tab_id: focusedTabId,
    focused_pane_id: focusedPaneId,
    spaces: [{ ...base.spaces[0], tab_count: 10, pane_count: 10 }],
    tabs: Array.from({ length: 10 }, (_, index) => ({
      id: `tab-${index + 1}`,
      space_id: "space-1",
      label: index === 0 ? "Alpha tab" : `Tab ${index + 1}`,
      number: index + 1,
      pane_count: 1,
      focused: focusedTabId === `tab-${index + 1}`,
    })),
    panes: Array.from({ length: 10 }, (_, index) => ({
      id: `pane-${index + 1}`,
      terminal_id: `terminal-${index + 1}`,
      space_id: "space-1",
      tab_id: `tab-${index + 1}`,
      title: index === 0 ? "Alpha pane" : `Pane ${index + 1}`,
      focused: focusedPaneId === `pane-${index + 1}`,
      agent: null,
      agent_status: "idle",
      revision: 1,
    })),
  };
}

function panePresentation(sessionId: string, paneId: string, canOpenReview = false): PanePresentation {
  return {
    session_id: sessionId,
    pane_id: paneId,
    terminal_id: `terminal-${paneId}`,
    binding_id: `binding-${paneId}`,
    extension: null,
    renderer: null,
    confidence: "none",
    reason: canOpenReview ? "" : "Review requires a configured repository",
    roots: [{ root_id: "repository", kind: "repository", label: "Repository", path: "/repository", repository_id: "repository", checkout_path: "/repository", companion_id: null }],
    default_root_id: "repository",
    can_open_context: false,
    can_open_files: false,
    files_root_id: null,
    can_open_review: canOpenReview,
    diagnostics: [],
  };
}

function createdSnapshot(sessionId: string): SessionSnapshotResponse {
  const base = snapshot(sessionId);
  return {
    ...base,
    tabs: [...base.tabs, { id: "tab-3", space_id: "space-1", label: "Created tab", number: 3, pane_count: 0, focused: true }],
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
  readonly inspectPane = vi.fn<CockpitClient["inspectPane"]>();
  readonly openReview = vi.fn<CockpitClient["openReview"]>();
  readonly subscriptions: Subscription[] = [];
  readonly mutationResponses: Array<Deferred<ResourceMutationResponse>> = [];
  private readonly snapshotQueues = new Map<string, Array<Promise<SessionSnapshotResponse>>>();
  private sessionResults: Array<Promise<{ sessions: SessionSummary[] }>> = [Promise.resolve({ sessions: sessions() })];

  readonly client: CockpitClient = {
    status: vi.fn(async () => status),
    browserAction: vi.fn(async () => ({ association: null, connection: "absent" as const, message: "No browser is associated with this Space" })),
    browserFeedback: vi.fn(async () => { throw new Error("Unexpected browser feedback in fixture"); }),
    browserDraftRecovery: vi.fn(async () => ({ type: "none" as const })),
    acknowledgeBrowserFeedback: vi.fn(async () => { throw new Error("Unexpected browser feedback acknowledgement in fixture"); }),
    browserFeedbackImage: vi.fn(async () => { throw new Error("Unexpected browser feedback image in fixture"); }),
    sendBrowserFeedback: vi.fn(async () => { throw new Error("Unexpected browser feedback send in fixture"); }),
    resolveWorkspaceDefaults: vi.fn(async () => { throw new Error("Unexpected workspace defaults in terminal fixture"); }),
    projectConfiguration: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
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
    inspectPane: this.inspectPane,
    contextDirectory: vi.fn(async () => { throw new Error("Unexpected Context read in terminal fixture"); }),
    contextDocument: vi.fn(async () => { throw new Error("Unexpected Context read in terminal fixture"); }),
    contextSearch: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    reviewSnapshot: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    reviewFile: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    contextSnapshot: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    contextInvalidate: vi.fn(async () => { throw new Error("Unexpected Context invalidation in terminal fixture"); }),
    contextMedia: vi.fn(), sourceImport: vi.fn(), sourceRefresh: vi.fn(), sourceList: vi.fn(async () => ({ binding_id: "binding", root_id: "root", entries: [], diagnostics: [] })), openReview: this.openReview, openContext: vi.fn(async () => { throw new Error("Unexpected Context launch in terminal fixture"); }),
    commentBatches: vi.fn(async () => { throw new Error("Unexpected comments list in terminal fixture"); }),
    commentBatch: vi.fn(async () => { throw new Error("Unexpected comment batch in terminal fixture"); }),
    commentUpsert: vi.fn(async () => { throw new Error("Unexpected comment upsert in terminal fixture"); }),
    commentDiscard: vi.fn(async () => { throw new Error("Unexpected comment discard in terminal fixture"); }),
    commentRemove: vi.fn(async () => { throw new Error("Unexpected comment remove in terminal fixture"); }),
    commentAttach: vi.fn(async () => { throw new Error("Unexpected comment attach in terminal fixture"); }),
    commentPastePrepare: async () => { throw new Error("unused"); },
    commentPasteSend: async () => { throw new Error("unused"); },
    commentPasteMarkPasted: async () => { throw new Error("unused"); },
    commentPreview: vi.fn(async () => { throw new Error("Unexpected comment preview in terminal fixture"); }),
    sessions: this.sessionsCalls,
    sessionSnapshot: this.snapshotCalls,
    focus: this.focusCalls,
    mutate: this.mutateCalls,
    subscribeSession: vi.fn(async (sessionId, onMessage, onError) => {
      const subscription: Subscription = { sessionId, onMessage, onError, closed: false };
      this.subscriptions.push(subscription);
      return { close: () => { subscription.closed = true; } };
    }),
    openTerminal: vi.fn(),
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
    this.inspectPane.mockImplementation(async (sessionId, paneId) => panePresentation(sessionId, paneId));
    this.openReview.mockImplementation(async (sessionId, request) => panePresentation(sessionId, request.pane_id, true));
  }

  setPanePresentation(value: PanePresentation): void {
    this.inspectPane.mockImplementation(async () => value);
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
  const match = [...container.querySelectorAll<HTMLButtonElement>("button")].find((candidate) => candidate.getAttribute("aria-label") === label || candidate.textContent?.trim() === label);
  if (!match) throw new Error(`Missing button ${label}`);
  return match;
}

function click(element: HTMLElement): void {
  act(() => element.click());
}

function openLocalPaneMenu(): void {
  const pane = container.querySelector<HTMLElement>(".pane-view:not([inert])");
  if (!pane) throw new Error("No visible pane");
  act(() => pane.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 24, clientY: 24 })));
}

function selectedTab(): string {
  const selected = container.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
  if (!selected) throw new Error("No selected tab");
  return selected.getAttribute("aria-label") ?? "";
}
function readyTerminal(paneId: string): void {
  const ready = terminalReadyCallbacks.get(paneId);
  if (!ready) throw new Error(`Missing terminal readiness callback for ${paneId}`);
  act(() => ready());
}

function mutationResponse(sessionId: string, next = snapshot(sessionId)): ResourceMutationResponse {
  return { session_id: sessionId, snapshot: next };
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
  it("opens Review from the command overlay for the selected single pane", async () => {
    const fixture = new AppFixture();
    const presentation = panePresentation("session-1", "pane-1", true);
    presentation.roots.unshift({ ...presentation.roots[0], root_id: "ancestor", kind: "folder", repository_id: "ancestor" });
    fixture.setPanePresentation(presentation);
    await mount(fixture);
    await settle();

    expect(container.querySelector(".pane-border-label")).toBeNull();
    act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "b", ctrlKey: true, bubbles: true, cancelable: true })));
    act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "?", bubbles: true, cancelable: true })));
    await settle();

    expect(button("Open Review right").disabled).toBe(false);
    click(button("All commands"));
    expect(button("Open Review below").disabled).toBe(false);
    click(button("Open Review right"));
    await settle();
    expect(fixture.openReview).toHaveBeenCalledWith("session-1", { pane_id: "pane-1", binding_id: "binding-pane-1", repository_id: "repository", direction: "right" });
  });

  it("opens files from the local pane menu without a task companion", async () => {
    const fixture = new AppFixture();
    const presentation = panePresentation("session-1", "pane-1");
    presentation.can_open_files = true;
    presentation.files_root_id = "folder";
    presentation.roots.push({ root_id: "folder", kind: "folder", label: "Files", path: "/folder", repository_id: "folder", checkout_path: "/folder", companion_id: null });
    fixture.setPanePresentation(presentation);
    vi.mocked(fixture.client.openContext).mockResolvedValue(presentation);
    await mount(fixture);
    openLocalPaneMenu();
    expect(button("Files").disabled).toBe(false);
    expect(button("Context").disabled).toBe(true);
    click(button("Files"));
    await settle();
    expect(fixture.client.openContext).toHaveBeenCalledWith("session-1", { pane_id: "pane-1", binding_id: "binding-pane-1", root_id: "folder", direction: "right" });
  });

  it("keeps local pane actions and Commands available for the selected single pane", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    expect(container.querySelector(".pane-border-label")).toBeNull();
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
    expect(button("Split right").disabled).toBe(false);
    click(button("Create tab"));
    expect(button("Split right").disabled).toBe(true);
  });

  it("paints the requested tab while Herdr confirms focus", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    const tab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-label="Tab 2: Second tab"]');
    if (!tab) throw new Error("Missing second tab");
    click(tab);
    await settle();
    expect(selectedTab()).toBe("Tab 2: Second tab");
    expect(container.querySelector('[aria-label="Waiting for Herdr focus confirmation"]')).not.toBeNull();

    fixture.emitSnapshot("session-1", 1, 2, snapshot("session-1", "tab-2", "pane-2"));
    await settle();
    expect(selectedTab()).toBe("Tab 2: Second tab");
  });
  it("retains an inert painted frame until the newly selected terminal hydrates", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    expect(container.querySelector<HTMLElement>(".pane-canvas")?.style.visibility).toBe("visible");
    const tab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-label="Tab 2: Second tab"]');
    if (!tab) throw new Error("Missing second tab");
    click(tab);
    fixture.emitSnapshot("session-1", 1, 2, snapshot("session-1", "tab-2", "pane-2"));
    await settle();
    expect(selectedTab()).toBe("Tab 2: Second tab");
    expect(container.querySelector<HTMLElement>(".pane-canvas")?.style.visibility).toBe("visible");
    expect(container.querySelector<HTMLElement>('[aria-label="Alpha pane"]')?.style.visibility).toBe("visible");
    expect(container.querySelector<HTMLElement>('[aria-label="Second pane"]')?.style.visibility).toBe("hidden");
    const retained = container.querySelector<HTMLElement>('[aria-label="Alpha pane"]')!;
    expect(retained.hasAttribute("inert")).toBe(true);
    click(retained.querySelector<HTMLButtonElement>(".pane-header-select")!);
    expect(selectedTab()).toBe("Tab 2: Second tab");

    readyTerminal("pane-2");
    await settle();
    expect(container.querySelector<HTMLElement>('[aria-label="Alpha pane"]')).toBeNull();
    expect(container.querySelector<HTMLElement>('[aria-label="Second pane"]')?.style.visibility).toBe("visible");

    const firstTab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-label="Tab 1: Alpha tab"]');
    if (!firstTab) throw new Error("Missing first tab");
    click(firstTab);
    fixture.emitSnapshot("session-1", 1, 3, snapshot("session-1", "tab-1", "pane-1"));
    await settle();
    expect(container.querySelector<HTMLElement>('[aria-label="Second pane"]')?.style.visibility).toBe("visible");
    expect(container.querySelector<HTMLElement>('[aria-label="Alpha pane"]')?.style.visibility).toBe("hidden");
    readyTerminal("pane-1");
    await settle();
    expect(container.querySelector<HTMLElement>('[aria-label="Second pane"]')).toBeNull();
    expect(container.querySelector<HTMLElement>('[aria-label="Alpha pane"]')?.style.visibility).toBe("visible");
  });
  it("reuses mounted panes through a rapid tab reversal", async () => {
    const fixture = new AppFixture();
    await mount(fixture);

    const secondTab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-label="Tab 2: Second tab"]');
    if (!secondTab) throw new Error("Missing second tab");
    click(secondTab);
    fixture.emitSnapshot("session-1", 1, 2, snapshot("session-1", "tab-2", "pane-2"));
    await settle();
    const firstTab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-label="Tab 1: Alpha tab"]');
    if (!firstTab) throw new Error("Missing first tab");
    click(firstTab);
    fixture.emitSnapshot("session-1", 1, 3, snapshot("session-1", "tab-1", "pane-1"));
    await settle();

    expect(container.querySelector<HTMLElement>(".pane-canvas")?.style.visibility).toBe("visible");
    const alphaPanes = [...container.querySelectorAll<HTMLElement>('[aria-label="Alpha pane"]')];
    expect(alphaPanes).toHaveLength(1);
    expect(alphaPanes[0]?.style.visibility).toBe("visible");

    readyTerminal("pane-1");
    await settle();
    expect(container.querySelectorAll('[aria-label="Alpha pane"]')).toHaveLength(1);
    expect(container.querySelector<HTMLElement>('[aria-label="Alpha pane"]')?.style.visibility).toBe("visible");
  });
  it("bounds outgoing terminal hosts while switching through many tabs and reattaches on revisit", async () => {
    const fixture = new AppFixture();
    await mount(fixture);
    const tenTabs = manyTabsSnapshot("session-1");
    fixture.emitSnapshot("session-1", 1, 2, tenTabs);
    await settle();

    for (let index = 2; index <= 10; index += 1) {
      const tab = container.querySelector<HTMLButtonElement>(`[role="tab"][aria-label="Tab ${index}: Tab ${index}"]`);
      if (!tab) throw new Error(`Missing tab ${index}`);
      click(tab);
      fixture.emitSnapshot("session-1", 1, index + 1, manyTabsSnapshot("session-1", `tab-${index}`, `pane-${index}`));
      await settle();
      expect(container.querySelectorAll('[data-testid^="terminal-"]')).toHaveLength(2);
    }

    readyTerminal("pane-10");
    await settle();
    expect(container.querySelectorAll('[data-testid^="terminal-"]')).toHaveLength(1);

    const firstTab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-label="Tab 1: Alpha tab"]');
    if (!firstTab) throw new Error("Missing first tab on revisit");
    click(firstTab);
    fixture.emitSnapshot("session-1", 1, 12, manyTabsSnapshot("session-1", "tab-1", "pane-1"));
    await settle();
    expect(container.querySelectorAll('[data-testid^="terminal-"]')).toHaveLength(2);
    readyTerminal("pane-1");
    await settle();
    expect(container.querySelectorAll('[data-testid^="terminal-"]')).toHaveLength(1);
  });

  it("keeps a terminal mounted when inspection binding identity changes", async () => {
    vi.useFakeTimers();
    try {
      const fixture = new AppFixture();
      await mount(fixture);
      const terminal = container.querySelector<HTMLElement>('[data-testid="terminal-pane-1"]');
      expect(terminal).not.toBeNull();

      const changed = panePresentation("session-1", "pane-1");
      changed.binding_id = "binding-pane-1-after-process-churn";
      fixture.setPanePresentation(changed);
      await advanceTimers(2500);
      await settle();

      expect(container.querySelector<HTMLElement>('[data-testid="terminal-pane-1"]')).toBe(terminal);
    } finally {
      vi.useRealTimers();
    }
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
      expect(fixture.focusCalls).toHaveBeenLastCalledWith("session-1", { kind: "tab", target_id: "tab-2" });

      await advanceTimers(500);
      await settle();
      fixture.emitSnapshot("session-1", 1, 1, snapshot("session-1"));
      await settle();

      expect(fixture.focusCalls).toHaveBeenCalledTimes(2);
      expect(fixture.focusCalls).toHaveBeenLastCalledWith("session-1", { kind: "tab", target_id: "tab-2" });
      expect(selectedTab()).toBe("Tab 1: Alpha tab");
    } finally {
      vi.useRealTimers();
    }
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
    click(button("switch session..."));
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
    click(button("switch session..."));
    click(button("Commands"));
    click(button("switch session..."));
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
      click(button("switch session..."));
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

  it("retains a terminal renderer choice across refresh, stale state, and ordered recovery", async () => {
    vi.useFakeTimers();
    try {
      const fixture = new AppFixture();
      const graphical = { ...panePresentation("session-1", "pane-1", true), extension: "review", renderer: "review", confidence: "verified_launch" } as PanePresentation;
      fixture.setPanePresentation(graphical);
      await mount(fixture);
      const paneMenuItem = (label: string): HTMLButtonElement => {
        const item = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((candidate) => candidate.textContent?.trim() === label);
        if (!item) throw new Error(`Missing pane menu item ${label}`);
        return item;
      };
      openLocalPaneMenu();
      expect(paneMenuItem("Show terminal view")).toBeTruthy();
      click(paneMenuItem("Show terminal view"));
      openLocalPaneMenu();
      click(paneMenuItem("Refresh renderer detection"));
      await settle();
      openLocalPaneMenu();
      expect(paneMenuItem("Render document")).toBeTruthy();
      click(paneMenuItem("Render document"));

      fixture.emitError("session-1");
      await settle();
      openLocalPaneMenu();
      expect(paneMenuItem("Show terminal view")).toBeTruthy();
      click(paneMenuItem("Refresh renderer detection"));
      fixture.queueSnapshot("session-1", Promise.resolve(snapshot("session-1")));
      await advanceTimers(250);
      fixture.emitSnapshot("session-1", 1, 1, snapshot("session-1"));
      await settle();
      openLocalPaneMenu();
      expect(paneMenuItem("Show terminal view")).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });
});
