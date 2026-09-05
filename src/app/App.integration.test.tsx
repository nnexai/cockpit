// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../client/CockpitClient";
import type {
  ResourceMutationRequest,
  ResourceMutationResponse,
  SessionSnapshotResponse,
  SessionStreamMessage,
  SessionSummary,
  StatusResponse,
} from "../protocol/generated/v1";
import { App } from "./App";

vi.mock("./TerminalPane", () => ({
  TerminalPane: ({ request, onSelect }: { request: { pane_id: string }; onSelect?: () => void }) => (
    <button type="button" data-testid={`terminal-${request.pane_id}`} onClick={onSelect}>terminal</button>
  ),
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
  readonly subscriptions: Subscription[] = [];
  readonly mutationResponses: Array<Deferred<ResourceMutationResponse>> = [];
  private readonly snapshotQueues = new Map<string, Array<Promise<SessionSnapshotResponse>>>();
  private sessionResults: Array<Promise<{ sessions: SessionSummary[] }>> = [Promise.resolve({ sessions: sessions() })];

  readonly client: CockpitClient = {
    status: vi.fn(async () => status),
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
    inspectPane: vi.fn(async () => { throw new Error("Context inspection is unavailable in this terminal fixture"); }),
    contextDirectory: vi.fn(async () => { throw new Error("Unexpected Context read in terminal fixture"); }),
    contextDocument: vi.fn(async () => { throw new Error("Unexpected Context read in terminal fixture"); }),
    contextSearch: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    reviewSnapshot: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    reviewFile: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    contextSnapshot: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    contextInvalidate: vi.fn(async () => { throw new Error("Unexpected Context invalidation in terminal fixture"); }),
    contextMedia: vi.fn(), sourceImport: vi.fn(), sourceRefresh: vi.fn(), sourceList: vi.fn(async () => ({ binding_id: "binding", root_id: "root", entries: [], diagnostics: [] })), openReview: vi.fn(), openContext: vi.fn(async () => { throw new Error("Unexpected Context launch in terminal fixture"); }),
    commentBatches: vi.fn(async () => { throw new Error("Unexpected comments list in terminal fixture"); }),
    commentBatch: vi.fn(async () => { throw new Error("Unexpected comment batch in terminal fixture"); }),
    commentUpsert: vi.fn(async () => { throw new Error("Unexpected comment upsert in terminal fixture"); }),
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
}

function button(label: string): HTMLButtonElement {
  const match = [...container.querySelectorAll<HTMLButtonElement>("button")].find((candidate) => candidate.getAttribute("aria-label") === label || candidate.textContent?.trim() === label);
  if (!match) throw new Error(`Missing button ${label}`);
  return match;
}

function click(element: HTMLElement): void {
  act(() => element.click());
}

function selectedTab(): string {
  const selected = container.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
  if (!selected) throw new Error("No selected tab");
  return selected.getAttribute("aria-label") ?? "";
}

function mutationResponse(sessionId: string, next = snapshot(sessionId)): ResourceMutationResponse {
  return { session_id: sessionId, snapshot: next };
}

function selectSession(sessionId: string): void {
  const select = container.querySelector<HTMLSelectElement>('select[aria-label="Session"]');
  if (!select) throw new Error("Missing session selector");
  act(() => {
    select.value = sessionId;
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

describe("mounted App mutation and session ordering", () => {
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
    expect(fixture.snapshotCalls).toHaveBeenCalledWith("session-1");
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
    click(button("menu"));
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

    click(button("menu"));
    click(button("switch session..."));
    click(button("menu"));
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
});
