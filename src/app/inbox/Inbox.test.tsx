// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { InboxLocation, InboxView, LiaisonLookupResponse, OrchestrationMutationResponse, OrchestrationWaitResponse, SessionSnapshotResponse } from "../../protocol/generated/v1";
import type { AgentAttentionState } from "../session/agentAttention";
import type { AttentionRow } from "../sidebar/agentAttentionModel";
import { Inbox } from "./Inbox";

const attention = { availability: "ready", response: null } as AgentAttentionState;
const view = (session_id: string): InboxView => ({ session_id, state_store_id: `store-${session_id}`, revision: 1, tasks_token: "token", observation: { status: "fresh", observed_at: "now" }, owners: [{ root_id: "owner", run_id: "owner", attempt: 1, session_id, attachment_id: null, attachment_epoch: null, label: "Liaison", kind: "supervisor", stage: "active", harness: "omp", availability: "available", location: null, undelivered: 0 }], threads: [], image_limits: { max_bytes: 1024 * 1024, max_count: 4, mime_types: ["image/png", "image/jpeg"] }, unmanaged_agents: [] });
let root: Root | null = null, host: HTMLDivElement;
afterEach(() => { if (root) act(() => root!.unmount()); root = null; host?.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function flush(fn: () => void = () => {}) { await act(async () => { fn(); for (let i = 0; i < 8; i++) await Promise.resolve(); }); }
function setup(client: CockpitClient, sessionId = "a", directSnapshot?: SessionSnapshotResponse, options?: { attention: AgentAttentionState; onOpenLocation: (location: InboxLocation) => Promise<void> }) {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const render = (id: string, active = true, snapshot = directSnapshot) => root!.render(createElement(Inbox, { client, sessionId: id, active, currentSpace: null, snapshot: snapshot ?? { session_id: id, server_instance: "server", panes: [], agents: [] } as unknown as SessionSnapshotResponse, attention: options?.attention ?? attention, onOpenLocation: options?.onOpenLocation ?? (async () => {}) }));
  return { render: (id = sessionId, active = true, snapshot = directSnapshot) => render(id, active, snapshot) };
}
function text(value: string) {
  const field = host.querySelector(".inbox:not([hidden]) textarea") as HTMLTextAreaElement;
  Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(field, value);
  field.dispatchEvent(new Event("input", { bubbles: true }));
}
function submit() { host.querySelector(".inbox:not([hidden]) form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); }
const image = { image_id: "image-1", mime_type: "image/png", width: 1, height: 1, bytes: 3 };
function imageFixture() {
  const { client, waits } = fixture();
  client.orchestrationImagePut = vi.fn(async () => image);
  vi.spyOn(FileReader.prototype, "readAsDataURL").mockImplementation(function (this: FileReader) {
    Object.defineProperty(this, "result", { configurable: true, value: "data:image/png;base64,YWJj" });
    this.dispatchEvent(new Event("load"));
  });
  class TestURL extends URL {
    static createObjectURL = vi.fn(() => "blob:test-image");
    static revokeObjectURL = vi.fn();
  }
  vi.stubGlobal("URL", TestURL);
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  return { client, waits };
}
function pickImages(...files: File[]) {
  const input = host.querySelector<HTMLInputElement>('input[type="file"]')!;
  Object.defineProperty(input, "files", { configurable: true, value: files });
  input.dispatchEvent(new Event("change", { bubbles: true }));
}
function pasteImage(file: File, withText: boolean) {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: {
    types: withText ? ["text/plain", "Files"] : ["Files"],
    getData: (type: string) => withText && type === "text/plain" ? "pasted words" : "",
    files: [file],
    items: [{ kind: "file", type: file.type, getAsFile: () => file }],
  } });
  host.querySelector("textarea")!.dispatchEvent(event);
  return event;
}
function fixture() {
  const waits: Array<(value: OrchestrationWaitResponse) => void> = [];
  const client = { orchestrationInbox: vi.fn(async ({ session_id }: { session_id: string }) => view(session_id)), orchestrationLiaisonLookup: vi.fn(async () => ({ status: "resolved", preferred_root_id: "owner", lookup_revision: 1, roots: [] })), orchestrationWait: vi.fn((_request: unknown, signal?: AbortSignal) => new Promise<OrchestrationWaitResponse>((resolve, reject) => { waits.push(resolve); signal?.addEventListener("abort", () => reject(new Error("aborted")), { once: true }); })), orchestrationMutate: vi.fn(), orchestrationAgentSend: vi.fn(async () => {}) } as unknown as CockpitClient;
  return { client, waits };
}
function directFixture(options?: { attention: AgentAttentionState; onOpenLocation: (location: InboxLocation) => Promise<void> }) {
  const target = (pane: string, name: string, stamp = "now") => ({ endpoint_identity: "endpoint", workspace_id: "space", tab_id: "tab", pane_id: pane, terminal_id: `terminal-${pane}`, agent_kind: name, agent_name: name, native_session_id: null, native_session_file: null, state_changed_at: stamp });
  const agent = (pane: string, name: string, stamp = "now") => ({ workspace_id: "space", workspace_label: "Space", tab_id: "tab", tab_label: "Tab", pane_id: pane, agent_name: name, agent_status: "idle", state_changed_at: stamp, expected_target: target(pane, name, stamp) });
  const snapshot = (status: "idle" | "working" | "done" = "idle", seq = 1) => ({ session_id: "a", server_instance: "server", panes: ["a", "b"].map(id => ({ id, terminal_id: `terminal-${id}` })), agents: ["a", "b"].map((id, i) => ({ pane_id: id, space_id: "space", tab_id: "tab", name: i ? "Beta" : "Alpha", status: i ? "idle" : status, state_change_seq: seq, focused: false })) } as unknown as SessionSnapshotResponse);
  const inbox = view("a"); inbox.unmanaged_agents = [agent("a", "Alpha"), agent("b", "Beta")];
  const { client, waits } = fixture();
  vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
  const app = setup(client, "a", snapshot(), options);
  const choose = (name: string) => Array.from(host.querySelectorAll(".inbox-row")).find(b => b.textContent?.includes(name))!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  return { client, waits, inbox, agent, snapshot, app, choose };
}

function completionAttention(seq = 1) {
  const seen = vi.fn(async (_row: AttentionRow) => true);
  const state = { ...attention, seen, response: { session_id: "a", server_instance: "server", epoch: 1, refused: null, records: [{ pane_id: "a", terminal_id: "terminal-a", agent: "Alpha", last: { epoch: 1, seq, status: "idle", observed_at: "now" }, completion: { epoch: 1, seq, at: "now", inferred: true }, acked: null }] } } as AgentAttentionState;
  return { state, seen };
}
function deferredFocus() {
  let resolve!: () => void, reject!: (error: Error) => void;
  const promise = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function deferredLookup() {
  let resolve!: (value: LiaisonLookupResponse) => void;
  const promise = new Promise<LiaisonLookupResponse>(yes => { resolve = yes; });
  return { promise, resolve };
}
function openSession() {
  Array.from(host.querySelectorAll<HTMLButtonElement>(".inbox-detail button")).find(button => button.textContent === "Open session")!.click();
}

describe("Inbox", () => {
  it("shows the chronological conversation and linked task evidence", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    const owner = { kind: "owner" as const, run_id: "owner", harness: "omp" };
    const entry = (message_id: string, minute: string, kind: "request" | "follow_up" | "update" | "result", text: string) => ({ message_id, at: `2026-11-04T10:${minute}:00Z`, kind, text, source: kind === "request" || kind === "follow_up" ? { kind: "user" as const } : owner, outcome: kind === "result" ? "succeeded" as const : null, images: [], in_reply_to: null });
    inbox.threads = [{ thread_id: "thread", state: "handled", need: null, owner_root_ids: ["owner"], title: { text: "Timeline work", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [], recommendation: null, reply: null, unread_result_message_id: null, receipts: [], facts: [], updated_at: "2026-11-04T10:04:00Z", timeline_omitted: 0, timeline: [entry("1", "00", "request", "Original request"), entry("2", "01", "update", "First owner update"), entry("3", "02", "update", "Second owner update"), entry("4", "03", "follow_up", "More detail please"), entry("5", "04", "result", "Final result")], tasks: [{ root_id: "owner", task_id: "task", title: "Linked task", project: null, lane: "accepted", run: null, evidence: { run_id: "run", attempt: 1, attempts: 1, result: null, checks: [{ contract: "Desktop check", command_or_scenario: "desktop smoke", outcome: "failed", evidence: "Mismatch", limitation: "No native coverage" }], commits: [], head_commit: null, integration: null, artifacts: [], images: [], decisions: [] } }] }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    await flush(() => app.render());
    await flush(() => host.querySelector<HTMLButtonElement>(".inbox-row")!.click());
    const timeline = host.querySelector('[aria-label="Timeline"]')!.textContent!;
    const positions = ["Original request", "First owner update", "Second owner update", "More detail please", "Final result"].map(value => timeline.indexOf(value));
    expect(positions.every((position, index) => position >= 0 && (index === 0 || position > positions[index - 1]!))).toBe(true);
    const work = host.querySelector('[aria-label="Work"]')!;
    expect(work.textContent).toContain("Desktop check");
    expect(work.textContent).toContain("No native coverage");
  });
  it("shows distinct diagnostic message IDs once without duplicating labelled request and question IDs", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    const value = (text: string, message_id: string) => ({ text, source: { kind: "recorded" as const }, message_id, at: "now", images: [] });
    const question = { run_id: "owner", question_message_id: "question-id", asked_at: "now", text: value("A question", "question-id"), choices: [], receipt: { status: "unresolved" as const } };
    const receipt = (message_id: string, attempt_run_id: string | null) => ({ message_id, kind: "instruction" as const, to_run_id: "owner", attempt_run_id, stage: "stored" as const, created_at: "now", acked_at: null, stale: false });
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner"], title: value("Diagnostics work", "request-id"), next: value("Next", "shared-id"), project: null, request: { message_id: "request-id", text: "Request", created_at: "now", to_run_id: "owner", handoff: { task_id: null, task_revision: null, attempt_run_id: null, binding: null, origin: "browser" }, images: [] }, follow_ups: [], questions: [question], recommendation: value("Recommendation", "question-id"), reply: value("Reply", "shared-id"), unread_result_message_id: null, tasks: [], receipts: [receipt("request-id", "attempt-id"), receipt("shared-id", null), receipt("receipt-only-id", null)], facts: [value("Fact", "shared-id"), value("Distinct fact", "fact-id")], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Diagnostics work"))!.click());
    const disclosure = host.querySelector<HTMLDetailsElement>(".inbox-diag")!;
    expect(disclosure.open).toBe(false);
    await flush(() => disclosure.querySelector("summary")!.click());
    expect(disclosure.open).toBe(true);
    const diagnostics = host.querySelector(".inbox-evidence")!;
    const labels = Array.from(diagnostics.querySelectorAll<HTMLButtonElement>("button")).map(button => button.getAttribute("aria-label"));
    expect(labels.filter(label => label === "Copy request ID request-id")).toHaveLength(1);
    expect(labels.filter(label => label === "Copy question ID question-id")).toHaveLength(1);
    expect(labels.filter(label => label === "Copy message ID shared-id")).toHaveLength(1);
    expect(labels.filter(label => label === "Copy message ID fact-id")).toHaveLength(1);
    expect(labels.filter(label => label === "Copy message ID receipt-only-id")).toHaveLength(1);
    expect(labels).not.toContain("Copy message ID request-id");
    expect(labels).not.toContain("Copy message ID question-id");
    expect(labels).toContain("Copy attempt run ID attempt-id");
  });
  it("acknowledges the selected completion without opening the terminal and keeps its detail when it leaves attention", async () => {
    const completion = completionAttention();
    const options = { attention: completion.state, onOpenLocation: vi.fn(async () => {}) };
    const f = directFixture(options);
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha"));
    expect(completion.seen).toHaveBeenCalledTimes(1);
    expect(completion.seen.mock.calls[0]![0]).toMatchObject({ terminalId: "terminal-a", completion: { epoch: 1, seq: 1 } });
    expect(options.onOpenLocation).not.toHaveBeenCalled();
    expect(host.querySelector(".inbox")?.getAttribute("data-detail")).toBe("true");
    options.attention = { ...completion.state, response: null };
    await flush(() => f.app.render());
    expect(host.querySelector(".inbox-detail")?.textContent).toContain("Alpha");
    expect(host.querySelector(".inbox-others")?.textContent).toContain("Alpha");
    await flush(() => f.choose("Alpha"));
    expect(completion.seen).toHaveBeenCalledTimes(1);
  });
  it("captures the selected completion even if a newer completion arrives while acknowledgement is pending", async () => {
    const completion = completionAttention();
    let resolve!: (value: boolean) => void;
    completion.seen.mockImplementationOnce(() => new Promise<boolean>(done => { resolve = done; }));
    const options = { attention: completion.state, onOpenLocation: vi.fn(async () => {}) };
    const f = directFixture(options);
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha"));
    const captured = completion.seen.mock.calls[0]![0];
    options.attention = completionAttention(2).state;
    await flush(() => f.app.render("a", true, f.snapshot("done", 2)));
    await flush(() => resolve(true));
    expect(captured.completion).toMatchObject({ epoch: 1, seq: 1 });
    expect(options.attention.seen).not.toHaveBeenCalled();
    expect(host.querySelector(".inbox-detail")?.textContent).toContain("Alpha");
  });
  it("selects without acknowledging an unsafe direct target", async () => {
    const completion = completionAttention();
    const f = directFixture({ attention: completion.state, onOpenLocation: vi.fn(async () => {}) });
    f.inbox.unmanaged_agents[0]!.expected_target!.native_session_id = "different-session";
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha"));
    expect(completion.seen).not.toHaveBeenCalled();
    expect(host.querySelector(".inbox-detail")?.textContent).toContain("Alpha");
  });
  it("does not acknowledge selection when the inbox observation is unavailable", async () => {
    const completion = completionAttention();
    const f = directFixture({ attention: completion.state, onOpenLocation: vi.fn(async () => {}) });
    f.inbox.observation = { status: "unavailable", error: { code: "unavailable", message: "Unavailable" } };
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha"));
    expect(completion.seen).not.toHaveBeenCalled();
    expect(host.querySelector(".inbox-detail")?.textContent).toContain("Alpha");
  });

  it("acknowledges only the completion captured when a direct session is opened", async () => {
    const completion = completionAttention(), focus = deferredFocus();
    const options = { attention: completion.state, onOpenLocation: vi.fn(() => focus.promise) };
    const f = directFixture(options);
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha"));
    expect(completion.seen).toHaveBeenCalledTimes(1);
    completion.seen.mockClear();
    await flush(openSession);
    expect(options.onOpenLocation).toHaveBeenCalledTimes(1);
    expect(completion.seen).not.toHaveBeenCalled();
    const newer = completionAttention(2);
    options.attention = newer.state;
    await flush(() => f.app.render("a", true, f.snapshot("done", 2)));
    await flush(() => focus.resolve());
    expect(newer.seen).toHaveBeenCalledTimes(1);
    expect(newer.seen.mock.calls[0]![0]).toMatchObject({ terminalId: "terminal-a", completion: { epoch: 1, seq: 1 } });
  });
  it("does not acknowledge rejected focus or a stale selected target", async () => {
    const completion = completionAttention(), focus = deferredFocus();
    const options = { attention: completion.state, onOpenLocation: vi.fn(() => focus.promise) };
    const f = directFixture(options);
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha")); completion.seen.mockClear(); await flush(openSession);
    await flush(() => focus.reject(new Error("focus rejected")));
    expect(completion.seen).not.toHaveBeenCalled();
    expect(host.textContent).toContain("focus rejected");
    await flush(() => f.app.render("a", true, { ...f.snapshot(), panes: [{ ...f.snapshot().panes[0]!, terminal_id: "replacement" }, f.snapshot().panes[1]!] }));
    await flush(openSession);
    expect(options.onOpenLocation).toHaveBeenCalledTimes(1);
    expect(completion.seen).not.toHaveBeenCalled();
  });
  it("does not acknowledge if the terminal identity changes while focus is pending", async () => {
    const completion = completionAttention(), focus = deferredFocus();
    const f = directFixture({ attention: completion.state, onOpenLocation: () => focus.promise });
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha")); completion.seen.mockClear(); await flush(openSession);
    await flush(() => f.app.render("a", true, { ...f.snapshot(), panes: [{ ...f.snapshot().panes[0]!, terminal_id: "replacement" }, f.snapshot().panes[1]!] }));
    await flush(() => focus.resolve());
    expect(completion.seen).not.toHaveBeenCalled();
  });
  it("does not acknowledge after the selection changes while focus is pending", async () => {
    const completion = completionAttention(), focus = deferredFocus();
    const f = directFixture({ attention: completion.state, onOpenLocation: () => focus.promise });
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha")); completion.seen.mockClear(); await flush(openSession);
    await flush(() => f.choose("Beta")); await flush(() => focus.resolve());
    expect(completion.seen).not.toHaveBeenCalled();
  });
  it("waits session-wide and refetches only when changed", async () => {
    const { client, waits } = fixture(), app = setup(client);
    await flush(() => app.render());
    expect(client.orchestrationWait).toHaveBeenCalledWith({ root_id: null, after_revision: 1, after_tasks_token: "token", timeout_ms: 30000 }, expect.any(AbortSignal));
    await flush(() => waits[0]!({ revision: 1, tasks_token: "token", changed: false }));
    expect(client.orchestrationInbox).toHaveBeenCalledTimes(1);
    await flush(() => waits[1]!({ revision: 2, tasks_token: "changed", changed: true }));
    expect(client.orchestrationInbox).toHaveBeenCalledTimes(2);
  });
  it("uses full projected titles and sizes the restored draft", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    const fullTitle = `Historic ${"😀".repeat(140)}`;
    const request = "Short heading\n" + "Request body ".repeat(80);
    const base = { state: "underway" as const, need: null, owner_root_ids: ["owner"], next: null, project: null, follow_ups: [], questions: [], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" };
    inbox.threads = [
      { ...base, thread_id: "short", title: { text: "Short heading", source: { kind: "user" }, message_id: null, at: null, images: [] }, request: { message_id: "m", text: request, created_at: "now", to_run_id: "owner", handoff: { task_id: null, task_revision: null, attempt_run_id: null, binding: null, origin: "browser" }, images: [] }, timeline: [{ message_id: "m", at: "2026-11-04T10:00:00Z", kind: "request", text: request, source: { kind: "user" }, outcome: null, images: [], in_reply_to: null }] },
      { ...base, thread_id: "historic", title: { text: fullTitle, source: { kind: "recorded" }, message_id: null, at: null, images: [] }, request: null },
    ];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    vi.spyOn(HTMLTextAreaElement.prototype, "scrollHeight", "get").mockImplementation(function (this: HTMLTextAreaElement) { return this.value ? 180 : 40; });
    await flush(() => app.render());
    const rows = host.querySelectorAll<HTMLButtonElement>(".inbox-row");
    expect(rows[0]!.querySelector("strong")!.textContent).toBe("Short heading");
    expect(rows[1]!.querySelector("strong")!.textContent).toBe(fullTitle);
    expect(rows[1]!.title).toBe(fullTitle);
    await flush(() => text("Long draft"));
    expect(host.querySelector<HTMLTextAreaElement>("textarea")!.style.height).toBe("180px");
    await flush(() => rows[0]!.click());
    expect(host.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe("");
    expect(host.querySelector<HTMLTextAreaElement>("textarea")!.style.height).toBe("40px");
    expect(host.querySelector(".inbox-thread-header h2")!.textContent).toBe("Short heading");
    await flush(() => rows[1]!.click());
    expect(host.querySelector(".inbox-thread-header h2")!.textContent).toBe(fullTitle);
    await flush(() => host.querySelector<HTMLButtonElement>(".inbox-new")!.click());
    expect(host.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe("Long draft");
    expect(host.querySelector<HTMLTextAreaElement>("textarea")!.style.height).toBe("180px");
  });
  it("keeps session drafts while switching and does not focus the composer", async () => {
    const { client } = fixture(), app = setup(client);
    await flush(() => app.render("a"));
    expect(document.activeElement?.tagName).not.toBe("TEXTAREA");
    await flush(() => text("first draft"));
    await flush(() => app.render("b"));
    await flush(() => text("second draft"));
    await flush(() => app.render("a", false));
    await flush(() => app.render("a"));
    expect((host.querySelector(".inbox:not([hidden]) textarea") as HTMLTextAreaElement).value).toBe("first draft");
    await flush(() => app.render("b"));
    expect((host.querySelector(".inbox:not([hidden]) textarea") as HTMLTextAreaElement).value).toBe("second draft");
  });
  it("distinguishes identically named owners and routes to the selected root", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    const first = { ...inbox.owners[0]!, root_id: "live-same", label: "Supervisor", location: { workspace_id: "space", workspace_label: "Project", tab_id: "tab", tab_label: "Work", pane_id: "pane" } };
    inbox.owners = [
      first,
      { ...first, root_id: "old-same", availability: "unknown", location: null },
      { ...first, root_id: "starting", label: "Starting owner", availability: "starting" },
      { ...first, root_id: "absent", label: "Absent owner", availability: "absent" },
    ];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "message", to_run_id: "old-same", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    const select = host.querySelector<HTMLSelectElement>('select[aria-label="Owner"]')!;
    expect(Array.from(select.options).map(option => [option.value, option.textContent])).toEqual([
      ["", "Choose owner"],
      ["live-same", "Supervisor · Project / Work · live-same · Available"],
      ["old-same", "Supervisor · old-same · Status unknown"],
      ["starting", "Starting owner · Starting"],
    ]);
    await flush(() => { select.value = "old-same"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(select.value).toBe("old-same");
    await flush(() => text("Send to the selected supervisor")); await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "thread_handoff", to_run_id: "old-same", text: "Send to the selected supervisor" });
  });
  it("routes new work to an available owner even when unknown and starting owners appear first", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    inbox.owners = [
      { ...inbox.owners[0]!, root_id: "unknown", availability: "unknown" },
      { ...inbox.owners[0]!, root_id: "starting", availability: "starting" },
      { ...inbox.owners[0]!, root_id: "available" },
    ];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "message", to_run_id: "available", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    expect(host.querySelector<HTMLSelectElement>('select[aria-label="Owner"]')!.value).toBe("available");
    await flush(() => text("New work")); await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "thread_handoff", to_run_id: "available", text: "New work" });
  });
  it("keeps the available default when a delayed lookup prefers an unknown owner", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    inbox.owners = [
      { ...inbox.owners[0]!, root_id: "unknown", availability: "unknown" },
      { ...inbox.owners[0]!, root_id: "available" },
    ];
    const lookup = deferredLookup();
    vi.mocked(client.orchestrationLiaisonLookup).mockReturnValue(lookup.promise);
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "message", to_run_id: "available", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    const select = host.querySelector<HTMLSelectElement>('select[aria-label="Owner"]')!;
    expect(select.value).toBe("available");
    await flush(() => lookup.resolve({ status: "resolved", preferred_root_id: "unknown", lookup_revision: 1, roots: [] }));
    expect(select.value).toBe("available");
    await flush(() => text("New work")); await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "thread_handoff", to_run_id: "available", text: "New work" });
  });
  it("prefers an available new-work owner without overriding an explicit unknown selection", async () => {
    const { client, waits } = fixture(), app = setup(client), inbox = view("a");
    inbox.owners = [
      { ...inbox.owners[0]!, root_id: "unknown", availability: "unknown" },
      { ...inbox.owners[0]!, root_id: "starting", availability: "starting" },
      { ...inbox.owners[0]!, root_id: "available" },
    ];
    const lookup = deferredLookup();
    vi.mocked(client.orchestrationLiaisonLookup).mockReturnValue(lookup.promise);
    vi.mocked(client.orchestrationInbox).mockResolvedValueOnce(inbox).mockResolvedValue({ ...inbox, revision: 2 });
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 3, result: { result: "message", to_run_id: "unknown", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    const select = host.querySelector<HTMLSelectElement>('select[aria-label="Owner"]')!;
    expect(select.value).toBe("available");
    await flush(() => { select.value = "unknown"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    await flush(() => lookup.resolve({ status: "resolved", preferred_root_id: "starting", lookup_revision: 1, roots: [] }));
    expect(select.value).toBe("unknown");
    await flush(() => waits[0]!({ revision: 2, tasks_token: "changed", changed: true }));
    expect(select.value).toBe("unknown");
    await flush(() => text("Send to my chosen owner")); await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "thread_handoff", to_run_id: "unknown", text: "Send to my chosen owner" });
  });
  it("falls back from an ineligible preferred owner and keeps drafts as availability changes", async () => {
    const { client, waits } = fixture(), app = setup(client), inbox = view("a");
    inbox.owners = [
      { ...inbox.owners[0]!, stage: "closed" },
      { ...inbox.owners[0]!, root_id: "absent", availability: "absent" },
      { ...inbox.owners[0]!, root_id: "unknown", kind: "adopted", availability: "unknown" },
      { ...inbox.owners[0]!, root_id: "starting", availability: "starting" },
      { ...inbox.owners[0]!, root_id: "worker", kind: "worker" },
    ];
    vi.mocked(client.orchestrationInbox).mockResolvedValueOnce(inbox).mockResolvedValue({ ...inbox, revision: 2, owners: inbox.owners.map(o => o.root_id === "unknown" ? { ...o, availability: "absent" as const } : o) });
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 3, result: { result: "message", to_run_id: "starting", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    const select = host.querySelector<HTMLSelectElement>('select[aria-label="Owner"]')!;
    expect(Array.from(select.options).map(o => o.value)).toEqual(["", "unknown", "starting"]);
    expect(select.value).toBe("unknown");
    await flush(() => text("Keep this draft"));
    await flush(() => waits[0]!({ revision: 2, tasks_token: "changed", changed: true }));
    expect(select.value).toBe("starting");
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("Keep this draft");
    await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "thread_handoff", to_run_id: "starting", text: "Keep this draft" });
  });
  it("preserves follow-up routing to closed and absent historical owners", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    inbox.owners = [{ ...inbox.owners[0]!, stage: "closed", availability: "absent" }, { ...inbox.owners[0]!, root_id: "other", availability: "absent" }];
    inbox.threads = [{ thread_id: "thread", state: "underway", need: null, owner_root_ids: ["owner", "other"], title: { text: "Historic work", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "message", to_run_id: "other", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Historic work"))!.click());
    const select = host.querySelector<HTMLSelectElement>('select[aria-label="Owner"]')!;
    expect(Array.from(select.options).map(o => o.value)).toEqual(["", "owner", "other"]);
    await flush(() => { select.value = "other"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    await flush(() => text("Follow up")); await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "thread_handoff", thread_id: "thread", to_run_id: "other" });
  });
  it("keeps the draft and does not send when no eligible new-work owner exists", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    inbox.owners = [{ ...inbox.owners[0]!, availability: "absent" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    await flush(() => app.render()); await flush(() => text("Unsent draft")); await flush(submit);
    expect((host.querySelector<HTMLSelectElement>('select[aria-label="Owner"]')!).value).toBe("");
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("Unsent draft");
    expect(client.orchestrationMutate).not.toHaveBeenCalled();
  });
  it("uploads a picked image and removes it before sending", async () => {
    const { client } = imageFixture(), app = setup(client);
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "message", to_run_id: "owner", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    expect(host.querySelector<HTMLButtonElement>('[aria-label="Attach images"]')?.disabled).toBe(false);
    expect(host.textContent).not.toContain("Image metadata may be retained.");
    await flush(() => pickImages(new File(["abc"], "photo.png", { type: "image/png" })));
    expect(client.orchestrationImagePut).toHaveBeenCalledWith({ session_id: "a", data_base64: "YWJj" }, expect.any(AbortSignal));
    expect(host.querySelector<HTMLImageElement>(".inbox-image-tray img")?.src).toBe("blob:test-image");
    expect(host.querySelector(".inbox-compose-hint")?.textContent).toContain("Images are shared with the owner and workers on this request. Image metadata may be retained.");
    const attach = host.querySelector<HTMLButtonElement>('[aria-label="Attach images"]')!;
    expect(document.getElementById(attach.getAttribute("aria-describedby")!)?.textContent).toContain("Image metadata may be retained.");
    await flush(() => host.querySelector<HTMLButtonElement>('[aria-label="Remove photo.png"]')!.click());
    expect(host.querySelector(".inbox-image-tray")).toBeNull();
    expect(host.textContent).not.toContain("Image metadata may be retained.");
    expect(attach.hasAttribute("aria-describedby")).toBe(false);
    await flush(() => text("Only text")); await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ text: "Only text" });
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).not.toMatchObject({ images: [image.image_id] });
  });
  it("sends an image-only request and retries the same image and handoff identity", async () => {
    const { client } = imageFixture(), app = setup(client);
    vi.mocked(client.orchestrationMutate).mockRejectedValueOnce(new Error("connection lost")).mockResolvedValueOnce({ revision: 2, result: { result: "message", to_run_id: "owner", seq: 1, duplicate: true, stale: false } });
    await flush(() => app.render());
    await flush(() => pickImages(new File(["abc"], "photo.png", { type: "image/png" })));
    await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "thread_handoff", text: "", images: [image.image_id] });
    expect(host.querySelector('[aria-label="Remove photo.png"]')).not.toBeNull();
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent === "Retry original request")!.click());
    expect(vi.mocked(client.orchestrationMutate).mock.calls[1]![0]).toEqual(vi.mocked(client.orchestrationMutate).mock.calls[0]![0]);
    expect(client.orchestrationImagePut).toHaveBeenCalledTimes(1);
  });
  it("uploads pasted images while allowing mixed text paste through", async () => {
    const { client } = imageFixture(), app = setup(client);
    const file = new File(["abc"], "pasted.png", { type: "image/png" });
    await flush(() => app.render());
    let mixed!: Event;
    await flush(() => { mixed = pasteImage(file, true); });
    expect(mixed.defaultPrevented).toBe(false);
    expect(client.orchestrationImagePut).toHaveBeenCalledWith({ session_id: "a", data_base64: "YWJj" }, expect.any(AbortSignal));
    expect(host.querySelector('[aria-label="Remove pasted.png"]')).not.toBeNull();
    await flush(() => host.querySelector<HTMLButtonElement>('[aria-label="Remove pasted.png"]')!.click());
    let imagePaste!: Event;
    await flush(() => { imagePaste = pasteImage(file, false); });
    expect(imagePaste.defaultPrevented).toBe(true);
    expect(client.orchestrationImagePut).toHaveBeenCalledTimes(2);
    expect(host.querySelector('[aria-label="Remove pasted.png"]')).not.toBeNull();
  });
  it("uploads dropped files without intercepting text drags", async () => {
    const { client } = imageFixture(), app = setup(client);
    await flush(() => app.render());
    const form = host.querySelector<HTMLFormElement>(".inbox-compose")!;
    const drag = (type: string, files: File[], types: string[]) => {
      const event = new Event(type, { bubbles: true, cancelable: true });
      Object.defineProperty(event, "dataTransfer", { value: { files, types, dropEffect: "none" } });
      form.dispatchEvent(event);
      return event;
    };
    let textDrag!: Event, textDrop!: Event;
    await flush(() => { textDrag = drag("dragover", [], ["text/plain"]); textDrop = drag("drop", [], ["text/plain"]); });
    expect(textDrag.defaultPrevented).toBe(false);
    expect((textDrag as DragEvent).dataTransfer?.dropEffect).toBe("none");
    expect(textDrop.defaultPrevented).toBe(false);
    expect(client.orchestrationImagePut).not.toHaveBeenCalled();
    const file = new File(["abc"], "drop.png", { type: "image/png" });
    let fileDrag!: Event, fileDrop!: Event;
    await flush(() => { fileDrag = drag("dragover", [file], ["Files"]); });
    expect(fileDrag.defaultPrevented).toBe(true);
    expect((fileDrag as DragEvent).dataTransfer?.dropEffect).toBe("copy");
    expect(form.getAttribute("data-dragging")).toBe("true");
    await flush(() => { fileDrop = drag("drop", [file], ["Files"]); });
    expect(fileDrop.defaultPrevented).toBe(true);
    expect(form.hasAttribute("data-dragging")).toBe(false);
    expect(client.orchestrationImagePut).toHaveBeenCalledWith({ session_id: "a", data_base64: "YWJj" }, expect.any(AbortSignal));
    expect(host.querySelector('[aria-label="Remove drop.png"]')).not.toBeNull();
    vi.mocked(client.orchestrationMutate).mockRejectedValueOnce(new Error("connection lost"));
    await flush(submit);
    let pendingDrag!: Event;
    await flush(() => { pendingDrag = drag("dragover", [file], ["Files"]); });
    expect(pendingDrag.defaultPrevented).toBe(false);
    expect((pendingDrag as DragEvent).dataTransfer?.dropEffect).toBe("none");
  });
  it("retries the same handoff identity after uncertain delivery", async () => {
    const { client } = fixture(), app = setup(client);
    vi.mocked(client.orchestrationMutate).mockRejectedValueOnce(new Error("connection lost")).mockResolvedValueOnce({ revision: 2, result: { result: "message", to_run_id: "owner", seq: 1, duplicate: true, stale: false } });
    await flush(() => app.render()); await flush(() => text("Please investigate")); await flush(submit);
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("Please investigate");
    const retry = Array.from(host.querySelectorAll("button")).find(b => b.textContent === "Retry original request")!;
    await flush(() => retry.click());
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0]).toEqual(vi.mocked(client.orchestrationMutate).mock.calls[1]![0]);
  });
  it("submits Enter once but not Shift+Enter or an IME composition", async () => {
    const { client } = fixture(), app = setup(client);
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "message", to_run_id: "owner", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render()); await flush(() => text("Please investigate"));
    const field = host.querySelector("textarea")!;
    await flush(() => field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true, cancelable: true })));
    await flush(() => field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true, cancelable: true })));
    await flush(() => field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 229, bubbles: true, cancelable: true })));
    expect(client.orchestrationMutate).not.toHaveBeenCalled();
    await flush(() => field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(client.orchestrationMutate).toHaveBeenCalledTimes(1);
  });
  it("answers only the explicitly selected current question", async () => {
    const { client } = fixture(), app = setup(client);
    const inbox = view("a");
    const question = (id: string, run: string) => ({ run_id: run, question_message_id: id, asked_at: "now", text: { text: `Question ${id}`, source: { kind: "owner" as const, run_id: run, harness: "omp" }, message_id: id, at: "now", images: [] }, choices: [], receipt: { status: "unresolved" as const } });
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner", "other"], title: { text: "Work", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [question("q1", "owner"), question("q2", "other")], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "message", to_run_id: "other", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Work"))!.click());
    await flush(() => (host.querySelectorAll<HTMLButtonElement>(".inbox-question .inbox-q-head")[1]!).click());
    await flush(() => text("Use the second option")); await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "message_send", to_run_id: "other", kind: "answer", in_reply_to: "q2", text: "Use the second option" });
  });
  it("sends a follow-up after a confirmed answer and resolved question", async () => {
    const { client, waits } = fixture(), app = setup(client), inbox = view("a");
    const question = { run_id: "owner", question_message_id: "q", asked_at: "now", text: { text: "Choose", source: { kind: "owner" as const, run_id: "owner", harness: "omp" }, message_id: "q", at: "now", images: [] }, choices: ["Blue"], receipt: { status: "unresolved" as const } };
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner"], title: { text: "Work", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [question], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValueOnce(inbox).mockResolvedValue({ ...inbox, revision: 2, threads: [{ ...inbox.threads[0]!, state: "handled", need: null, questions: [] }] });
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "message", to_run_id: "owner", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    await flush(() => host.querySelector<HTMLButtonElement>(".inbox-row")!.click());
    await flush(() => host.querySelector<HTMLButtonElement>(".inbox-choice")!.click());
    await flush(submit);
    await flush(() => waits[0]!({ revision: 2, tasks_token: "updated", changed: true }));
    await flush(() => text("One more thing")); await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[1]![0].action).toMatchObject({ action: "thread_handoff", thread_id: "thread", to_run_id: "owner", text: "One more thing" });
  });
  it("selects and answers the right root when questions share a message ID", async () => {
    const { client, waits } = fixture(), app = setup(client);
    const inbox = view("a");
    const question = (run: string) => ({ run_id: run, question_message_id: "shared", asked_at: "now", text: { text: `Question from ${run}`, source: { kind: "owner" as const, run_id: run, harness: "omp" }, message_id: "shared", at: "now", images: [] }, choices: [], receipt: { status: "unresolved" as const } });
    const first = question("owner"), second = question("other");
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner", "other"], title: { text: "Work", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [first, second], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValueOnce(inbox).mockResolvedValue({ ...inbox, revision: 2, threads: [{ ...inbox.threads[0]!, questions: [second, first] }] });
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 3, result: { result: "message", to_run_id: "other", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Work"))!.click());
    const ownerQuestion = host.querySelectorAll<HTMLButtonElement>(".inbox-question .inbox-q-head")[0]!;
    const otherQuestion = host.querySelectorAll<HTMLButtonElement>(".inbox-question .inbox-q-head")[1]!;
    await flush(() => otherQuestion.click());
    expect(ownerQuestion.getAttribute("aria-pressed")).toBe("false");
    expect(otherQuestion.getAttribute("aria-pressed")).toBe("true");
    const target = Array.from(host.querySelectorAll<HTMLButtonElement>(".inbox-seg button")).find(button => button.textContent === "Answer")!;
    expect(target.getAttribute("aria-pressed")).toBe("true");
    await flush(() => text("Answer for the other root"));
    await flush(() => waits[0]!({ revision: 2, tasks_token: "updated", changed: true }));
    expect(otherQuestion.getAttribute("aria-pressed")).toBe("true");
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("Answer for the other root");
    await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "message_send", to_run_id: "other", kind: "answer", in_reply_to: "shared", text: "Answer for the other root" });
  });
  it("does not retarget a vanished question to another root with the same message ID", async () => {
    const { client, waits } = fixture(), app = setup(client);
    const inbox = view("a");
    const question = (run: string) => ({ run_id: run, question_message_id: "shared", asked_at: "now", text: { text: `Question from ${run}`, source: { kind: "owner" as const, run_id: run, harness: "omp" }, message_id: "shared", at: "now", images: [] }, choices: [], receipt: { status: "unresolved" as const } });
    const first = question("owner"), second = question("other");
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner", "other"], title: { text: "Work", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [first, second], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValueOnce(inbox).mockResolvedValue({ ...inbox, revision: 2, threads: [{ ...inbox.threads[0]!, questions: [first] }] });
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Work"))!.click());
    await flush(() => host.querySelectorAll<HTMLButtonElement>(".inbox-question .inbox-q-head")[1]!.click());
    await flush(() => text("My answer"));
    await flush(() => waits[0]!({ revision: 2, tasks_token: "updated", changed: true }));
    await flush(submit);
    expect(client.orchestrationMutate).not.toHaveBeenCalled();
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("My answer");
    expect(host.textContent).toContain("selected question changed");
  });
  it("does not send a vanished question as an owner follow-up", async () => {
    const { client, waits } = fixture(), app = setup(client);
    const inbox = view("a");
    const question = { run_id: "owner", question_message_id: "old", asked_at: "now", text: { text: "Which one?", source: { kind: "owner" as const, run_id: "owner", harness: "omp" }, message_id: "old", at: "now", images: [] }, choices: [], receipt: { status: "unresolved" as const } };
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner"], title: { text: "Work", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [question], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValueOnce(inbox).mockResolvedValueOnce({ ...inbox, revision: 2, threads: [{ ...inbox.threads[0]!, questions: [] }] });
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Work"))!.click());
    await flush(() => host.querySelector<HTMLButtonElement>(".inbox-question .inbox-q-head")!.click());
    await flush(() => text("My answer"));
    await flush(() => waits[0]!({ revision: 2, tasks_token: "updated", changed: true }));
    await flush(submit);
    expect(client.orchestrationMutate).not.toHaveBeenCalled();
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("My answer");
    expect(host.textContent).toContain("selected question changed");
  });
  it("shows the selected owner report as rich feedback with its provenance", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "answer", owner_root_ids: ["owner"], title: { text: "Result", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [], recommendation: null, reply: { text: "## Report\n\n- **Completed**\n- Verified", source: { kind: "owner", run_id: "owner", harness: "omp" }, message_id: "result-id", at: "now", images: [] }, unread_result_message_id: "result-id", tasks: [], receipts: [], facts: [], timeline: [{ message_id: "result-id", at: "2026-11-04T10:00:00Z", kind: "result", text: "## Report\n\n- **Completed**\n- Verified", source: { kind: "owner", run_id: "owner", harness: "omp" }, outcome: "succeeded", images: [], in_reply_to: null }], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Result"))!.click());
    const update = host.querySelector(".inbox-result")!;
    expect(update.textContent).toContain("Result");
    expect(update.querySelector("h2")?.textContent).toBe("Report");
    expect(update.querySelectorAll("li")).toHaveLength(2);
    expect(update.querySelector(".inbox-feedback strong")?.textContent).toBe("Completed");
  });
  it("shows handled generically and keeps follow-up separate from acknowledgement", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    inbox.threads = [{ thread_id: "thread", state: "handled", need: null, owner_root_ids: ["owner"], title: { text: "Earlier request", source: { kind: "user" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [], recommendation: null, reply: { text: "The work was reported.", source: { kind: "owner", run_id: "owner", harness: "omp" }, message_id: "result", at: "now", images: [] }, unread_result_message_id: null, tasks: [], receipts: [], facts: [], timeline: [{ message_id: "result", at: "2026-11-04T10:00:00Z", kind: "result", text: "The work was reported.", source: { kind: "owner", run_id: "owner", harness: "omp" }, outcome: "succeeded", images: [], in_reply_to: null }], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    await flush(() => app.render());
    const row = host.querySelector<HTMLButtonElement>(".inbox-row")!;
    expect(row.textContent).toContain("The work was reported.");
    expect(row.textContent).toContain("Handled");
    await flush(() => row.click());
    expect(host.querySelector(".inbox-state")?.textContent).toBe("Handled");
    expect(host.querySelector('textarea[aria-label="Follow up with owner"]')).not.toBeNull();
    expect(client.orchestrationMutate).not.toHaveBeenCalled();
  });
  it("shows a NeedsInput report as rich question text without changing its choices", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    const question = { run_id: "owner", question_message_id: "q", asked_at: "now", text: { text: "## Decision\n\nPlease review **both** options.", source: { kind: "owner" as const, run_id: "owner", harness: "omp" }, message_id: "q", at: "now", images: [] }, choices: ["Yes", "No"], receipt: { status: "unresolved" as const } };
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner"], title: { text: "Question", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [question], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Question"))!.click());
    expect(host.querySelector(".inbox-q-text h2")?.textContent).toBe("Decision");
    expect(host.querySelector(".inbox-q-text strong")?.textContent).toBe("both");
    await flush(() => Array.from(host.querySelectorAll<HTMLButtonElement>(".inbox-choice")).find(b => b.textContent === "Yes")!.click());
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("Yes");
  });
  it("marks only the displayed result message read", async () => {
    const { client } = fixture(), app = setup(client);
    const inbox = view("a");
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner"], title: { text: "Result", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [{ run_id: "owner", question_message_id: "question-id", asked_at: "now", text: { text: "Choose a direction", source: { kind: "owner", run_id: "owner", harness: "omp" }, message_id: "question-id", at: "now", images: [] }, choices: ["Yes"], receipt: { status: "unresolved" } }], recommendation: null, reply: { text: "The result", source: { kind: "owner", run_id: "owner", harness: "omp" }, message_id: "result-id", at: "now", images: [] }, unread_result_message_id: "result-id", tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "done" } });
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Result"))!.click());
    expect(client.orchestrationMutate).not.toHaveBeenCalled();
    await flush(() => host.querySelector<HTMLButtonElement>(".inbox-choice")!.click());
    expect(host.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe("Yes");
    expect(client.orchestrationMutate).not.toHaveBeenCalled();
    const acknowledge = Array.from(host.querySelectorAll("button")).find(b => b.textContent === "Acknowledge")!;
    const description = document.getElementById(acknowledge.getAttribute("aria-describedby")!);
    expect(description?.classList.contains("inbox-sr")).toBe(true);
    expect(description?.textContent).toContain("does not answer questions");
    expect(acknowledge.title).toBe(description?.textContent);
    expect(host.querySelector(".inbox-result-actions > p")).toBeNull();
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent === "Acknowledge")!.click());
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toEqual({ action: "thread_mark_read", thread_id: "thread", through_message_id: "result-id" });
  });
  it("shows acknowledgement progress only for the result being marked read, not a follow-up or another selection", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    const result = { thread_id: "first", state: "needs_you" as const, need: "answer" as const, owner_root_ids: ["owner"], title: { text: "First result", source: { kind: "recorded" as const }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [], recommendation: null, reply: { text: "Report", source: { kind: "owner" as const, run_id: "owner", harness: "omp" }, message_id: "result-id", at: "now", images: [] }, unread_result_message_id: "result-id", tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" };
    inbox.threads = [result, { ...result, thread_id: "second", title: { ...result.title, text: "Second result" } }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    let finishFollowup!: (response: OrchestrationMutationResponse) => void;
    let finishRead!: (response: OrchestrationMutationResponse) => void;
    const followup = new Promise<OrchestrationMutationResponse>(resolve => { finishFollowup = resolve; });
    const read = new Promise<OrchestrationMutationResponse>(resolve => { finishRead = resolve; });
    vi.mocked(client.orchestrationMutate).mockReturnValueOnce(followup).mockReturnValueOnce(read);
    const choose = (title: string) => Array.from(host.querySelectorAll<HTMLButtonElement>(".inbox-row")).find(button => button.textContent?.includes(title))!.click();
    const acknowledge = () => host.querySelector<HTMLButtonElement>(".inbox-result-actions .is-primary")!;
    await flush(() => app.render());
    await flush(() => choose("First result"));
    await flush(() => text("Please follow up"));
    await flush(submit);
    expect(acknowledge().textContent).toBe("Acknowledge");
    expect(acknowledge().disabled).toBe(true);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "thread_handoff", thread_id: "first" });
    await flush(() => finishFollowup({ revision: 2, result: { result: "message", to_run_id: "owner", seq: 1, duplicate: false, stale: false } }));
    await flush(() => acknowledge().click());
    expect(acknowledge().textContent).toBe("Acknowledging…");
    expect(vi.mocked(client.orchestrationMutate).mock.calls[1]![0].action).toEqual({ action: "thread_mark_read", thread_id: "first", through_message_id: "result-id" });
    await flush(() => choose("Second result"));
    expect(acknowledge().textContent).toBe("Acknowledge");
    expect(acknowledge().disabled).toBe(true);
    await flush(() => choose("First result"));
    expect(acknowledge().textContent).toBe("Acknowledging…");
    await flush(() => finishRead({ revision: 2, result: { result: "done" } }));
    expect(acknowledge().textContent).toBe("Acknowledge");
  });
  it("does not acknowledge a result absent from the displayed report", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "answer", owner_root_ids: ["owner"], title: { text: "Result", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [], recommendation: null, reply: { text: "Earlier report", source: { kind: "owner", run_id: "owner", harness: "omp" }, message_id: "old-result", at: "now", images: [] }, unread_result_message_id: "new-result", tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    await flush(() => app.render());
    await flush(() => host.querySelector<HTMLButtonElement>(".inbox-row")!.click());
    const button = Array.from(host.querySelectorAll("button")).find(b => b.textContent === "Acknowledge")!;
    expect(button.disabled).toBe(true);
    expect(host.querySelector(".inbox-result-actions")?.textContent).toContain("not available");
    await flush(() => button.click());
    expect(client.orchestrationMutate).not.toHaveBeenCalled();
  });
  it("does not steal focus when acknowledgement finishes after the user moves away", async () => {
    const { client } = fixture(), app = setup(client), inbox = view("a");
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "answer", owner_root_ids: ["owner"], title: { text: "Result", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [], recommendation: null, reply: { text: "Report", source: { kind: "owner", run_id: "owner", harness: "omp" }, message_id: "result-id", at: "now", images: [] }, unread_result_message_id: "result-id", tasks: [], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    let finish!: () => void;
    vi.mocked(client.orchestrationMutate).mockImplementation(() => new Promise(resolve => { finish = () => resolve({ revision: 2, result: { result: "done" } }); }));
    await flush(() => app.render());
    await flush(() => host.querySelector<HTMLButtonElement>(".inbox-row")!.click());
    const button = Array.from(host.querySelectorAll("button")).find(b => b.textContent === "Acknowledge")!;
    await flush(() => { button.focus(); button.click(); });
    const other = host.querySelector<HTMLButtonElement>(".inbox-new")!;
    await flush(() => other.focus());
    expect(document.activeElement).toBe(other);
    const handled = { ...inbox, revision: 2, threads: [{ ...inbox.threads[0]!, state: "handled" as const, need: null, unread_result_message_id: null }] };
    vi.mocked(client.orchestrationInbox).mockResolvedValue(handled);
    await flush(() => finish());
    expect(document.activeElement).toBe(other);
  });
  it("fences a task follow-up with its owner's current task revision", async () => {
    const { client } = fixture(), app = setup(client);
    const inbox = view("a");
    inbox.threads = [{ thread_id: "thread", state: "underway", need: null, owner_root_ids: ["owner"], title: { text: "Task", source: { kind: "recorded" }, message_id: null, at: null, images: [] }, next: null, project: null, request: null, follow_ups: [], questions: [], recommendation: null, reply: null, unread_result_message_id: null, tasks: [{ root_id: "owner", task_id: "task-id", title: "Task", project: null, lane: "working", run: null, evidence: null }], receipts: [], facts: [], timeline: [], timeline_omitted: 0, updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    client.orchestrationSnapshot = vi.fn(async () => ({ session_id: "a", state_store_id: "store-a", unreflected_inputs: [], board: { root_id: "owner", tasks: [{ task: { task_id: "task-id", task_revision: "revision" } }] } })) as unknown as CockpitClient["orchestrationSnapshot"];
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "message", to_run_id: "owner", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Task"))!.click());
    await flush(() => { const select = host.querySelector<HTMLSelectElement>('select[aria-label="Follow-up target"]')!; select.value = "task-id"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    await flush(() => text("Please change it")); await flush(submit);
    expect(client.orchestrationSnapshot).toHaveBeenCalledWith({ session_id: "a", root_id: "owner" });
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "thread_handoff", thread_id: "thread", to_run_id: "owner", task_id: "task-id", expected_task_revision: "revision" });
  });
  it("keeps the original quiet-agent status fallback when no context is available", async () => {
    const f = directFixture();
    await flush(() => f.app.render());
    const row = Array.from(host.querySelectorAll<HTMLButtonElement>(".inbox-row")).find(b => b.textContent?.includes("Alpha"))!;
    expect(row.classList.contains("is-quiet")).toBe(true);
    expect(row.querySelector(".inbox-meta")).toBeNull();
    expect(row.querySelector(".inbox-row-next")?.textContent).toBe("Space · idle");
    await flush(() => f.choose("Alpha"));
    expect(host.querySelector('[aria-label="Session context"]')).toBeNull();
  });
  it("shows direct session context only for the matching live occupant", async () => {
    const f = directFixture();
    const target = { ...f.inbox.unmanaged_agents[0]!.expected_target!, agent_kind: "omp", native_session_id: "session-one" };
    f.inbox.unmanaged_agents[0] = { ...f.inbox.unmanaged_agents[0]!, expected_target: target };
    const base = f.snapshot();
    const withContext = { ...base, agents: [{ ...base.agents[0]!, name: "omp", terminal_id: "terminal-a", native_session_id: "session-one", session_title: "Repair index", idle_summary: "Index repaired" }, base.agents[1]!] };
    await flush(() => f.app.render("a", true, withContext));
    const row = Array.from(host.querySelectorAll<HTMLButtonElement>(".inbox-row")).find(b => b.textContent?.includes("Alpha"))!;
    expect(row.textContent).toContain("Index repaired");
    expect(row.textContent).toContain("Repair index");
    await flush(() => f.choose("Alpha"));
    expect(host.querySelector(".inbox-content")?.textContent).toContain("Repair index");
    expect(host.querySelector(".inbox-head")?.textContent).toContain("Direct agent · idle");
    expect(host.querySelector('[aria-label="Session context"]')?.textContent).toContain("Session title");
    expect(host.querySelector('[aria-label="Session context"]')?.textContent).toContain("Session recap · OMP session");
    expect(host.querySelector(".inbox-content")?.textContent).toContain("Index repaired");
    const working = { ...withContext, agents: [{ ...withContext.agents[0]!, status: "working", working_description: "Rebuilding index" }, withContext.agents[1]!] };
    await flush(() => f.app.render("a", true, working));
    expect(host.querySelector(".inbox-content")?.textContent).toContain("Rebuilding index");
    expect(host.querySelector(".inbox-content")?.textContent).not.toContain("Index repaired");
    await flush(() => f.app.render("a", true, { ...withContext, agents: [{ ...withContext.agents[0]!, native_session_id: "replacement", session_title: "Different session", idle_summary: "Different update" }, withContext.agents[1]!] }));
    expect(host.textContent).not.toContain("Different session");
    await flush(() => f.app.render("a", true, { ...withContext, panes: [{ ...withContext.panes[0]!, terminal_id: "replacement-terminal" }, withContext.panes[1]!] }));
    expect(host.querySelector(".inbox-content")?.textContent).not.toContain("Repair index");
    expect(host.querySelector(".inbox-content")?.textContent).not.toContain("Index repaired");
    await flush(() => f.app.render("a", true, { ...withContext, agents: [{ ...withContext.agents[0]!, name: "other-harness" }, withContext.agents[1]!] }));
    expect(host.querySelector(".inbox-head")?.textContent).toContain("status unavailable");
    expect(host.querySelector(".inbox-content")?.textContent).not.toContain("Repair index");
  });
  it("sends a direct request only with the captured occupant identity", async () => {
    const { client } = fixture();
    const target = { endpoint_identity: "endpoint", workspace_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", agent_kind: "Agent", agent_name: "Agent", native_session_id: null, native_session_file: null, state_changed_at: "now" };
    const inbox = view("a");
    inbox.unmanaged_agents = [{ workspace_id: "space", workspace_label: "Space", tab_id: "tab", tab_label: "Tab", pane_id: "pane", agent_name: "Agent", agent_status: "idle", state_changed_at: "now", expected_target: target }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    const snapshot = { session_id: "a", server_instance: "server", panes: [{ id: "pane", terminal_id: "terminal" }], agents: [{ pane_id: "pane", space_id: "space", tab_id: "tab", name: "Agent", status: "idle", state_change_seq: 1, focused: false }] } as unknown as SessionSnapshotResponse;
    const app = setup(client, "a", snapshot);
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Agent"))!.click());
    await flush(() => text("Hello")); await flush(submit);
    expect(client.orchestrationAgentSend).toHaveBeenCalledWith({ session_id: "a", reference: "pane", text: "Hello", expected_target: target });
  });
  it("does not retarget a retained draft when the pane occupant changes", async () => {
    const { client, waits } = fixture();
    const target = { endpoint_identity: "endpoint", workspace_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", agent_kind: "Agent", agent_name: "Agent", native_session_id: null, native_session_file: null, state_changed_at: "now" };
    const agent = { workspace_id: "space", workspace_label: "Space", tab_id: "tab", tab_label: "Tab", pane_id: "pane", agent_name: "Agent", agent_status: "idle", state_changed_at: "now", expected_target: target };
    const inbox = view("a"); inbox.unmanaged_agents = [agent];
    vi.mocked(client.orchestrationInbox).mockResolvedValueOnce(inbox).mockResolvedValueOnce({ ...inbox, revision: 2, unmanaged_agents: [{ ...agent, expected_target: { ...target, native_session_id: "replacement" } }] });
    const snapshot = { session_id: "a", server_instance: "server", panes: [{ id: "pane", terminal_id: "terminal" }], agents: [{ pane_id: "pane", space_id: "space", tab_id: "tab", name: "Agent", status: "idle", state_change_seq: 1, focused: false }] } as unknown as SessionSnapshotResponse;
    const app = setup(client, "a", snapshot);
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Agent"))!.click());
    await flush(() => text("Keep this"));
    await flush(() => waits[0]!({ revision: 2, tasks_token: "changed", changed: true }));
    await flush(submit);
    expect(client.orchestrationAgentSend).not.toHaveBeenCalled();
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("Keep this");
    expect(host.textContent).toContain("reselect the agent");
  });
  it("retains a draft on stale identity and requires explicit reselection", async () => {
    const { client } = fixture();
    const target = { endpoint_identity: "endpoint", workspace_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", agent_kind: "Agent", agent_name: "Agent", native_session_id: null, native_session_file: null, state_changed_at: "now" };
    const inbox = view("a"); inbox.unmanaged_agents = [{ workspace_id: "space", workspace_label: "Space", tab_id: "tab", tab_label: "Tab", pane_id: "pane", agent_name: "Agent", agent_status: "idle", state_changed_at: "now", expected_target: target }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    vi.mocked(client.orchestrationAgentSend).mockRejectedValueOnce(new CockpitClientError("http_error", "Changed", { operationCode: "stale_identity" }));
    const snapshot = { session_id: "a", server_instance: "server", panes: [{ id: "pane", terminal_id: "terminal" }], agents: [{ pane_id: "pane", space_id: "space", tab_id: "tab", name: "Agent", status: "idle", state_change_seq: 1, focused: false }] } as unknown as SessionSnapshotResponse;
    const app = setup(client, "a", snapshot);
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Agent"))!.click());
    await flush(() => text("Keep this")); await flush(submit);
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("Keep this");
    await flush(submit);
    expect(client.orchestrationAgentSend).toHaveBeenCalledTimes(1);
    expect(host.textContent).toContain("reselect the agent");
  });
  it("keeps the submission receipt through working and idle while preserving the target fence", async () => {
    const f = directFixture();
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha"));
    await flush(() => text("Hello")); await flush(submit);
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("");
    expect(host.textContent).toContain("Submitted to Alpha’s terminal. Not confirmed read.");
    vi.mocked(f.client.orchestrationInbox).mockResolvedValue({ ...f.inbox, revision: 2, unmanaged_agents: [f.agent("a", "Alpha", "later"), f.agent("b", "Beta")] });
    await flush(() => f.app.render("a", true, f.snapshot("working", 2)));
    expect(host.querySelector(".inbox-head")?.textContent).toContain("working");
    expect(host.textContent).toContain("Submitted to Alpha’s terminal");
    expect(host.textContent).not.toContain("Direct messages are available only");
    expect(host.textContent).not.toContain("Agent identity changed");
    await flush(() => f.app.render("a", true, f.snapshot("idle", 3)));
    expect(host.textContent).toContain("Submitted to Alpha’s terminal");
    await flush(() => text("Another"));
    await flush(submit);
    expect(f.client.orchestrationAgentSend).toHaveBeenCalledTimes(1);
    expect(host.textContent).toContain("reselect the agent");
  });

  it("requires reselection after submission and allows a completed agent to receive another message", async () => {
    const f = directFixture();
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha"));
    await flush(() => text("First")); await flush(submit);
    expect(host.textContent).toContain("Submitted to Alpha’s terminal. Not confirmed read.");
    await flush(() => f.app.render("a", true, f.snapshot("done", 2)));
    expect(host.querySelector(".inbox-head")?.textContent).toContain("done");
    expect(host.textContent).toContain("Submitted to Alpha’s terminal. Not confirmed read.");
    await flush(() => text("Second")); await flush(submit);
    expect(f.client.orchestrationAgentSend).toHaveBeenCalledTimes(1);
    expect(host.textContent).toContain("Submitted to Alpha’s terminal. Not confirmed read.");
    await flush(() => f.choose("Alpha")); await flush(submit);
    expect(f.client.orchestrationAgentSend).toHaveBeenCalledTimes(2);
    expect(vi.mocked(f.client.orchestrationAgentSend).mock.calls[1]![0]).toMatchObject({ reference: "a", text: "Second", expected_target: f.agent("a", "Alpha").expected_target });
    expect(host.textContent).toContain("Submitted to Alpha’s terminal. Not confirmed read.");
  });

  it("does not dispatch to a working agent", async () => {
    const f = directFixture();
    await flush(() => f.app.render("a", true, f.snapshot("working", 2)));
    await flush(() => f.choose("Alpha")); await flush(() => text("Wait")); await flush(submit);
    expect(f.client.orchestrationAgentSend).not.toHaveBeenCalled();
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("Wait");
  });

  it.each(["agent_not_idle", "herdr_prompt_deferred", "agent_blocked", "agent_not_ready"])("keeps the draft and requires reselection after %s", async code => {
    const f = directFixture();
    vi.mocked(f.client.orchestrationAgentSend).mockRejectedValueOnce(new CockpitClientError("http_error", "Busy", { operationCode: code }));
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha"));
    await flush(() => text("Keep this")); await flush(submit);
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("Keep this");
    expect(host.textContent).toContain("Not sent. Alpha is busy.");
    expect(host.textContent).not.toContain("Delivery is uncertain");
    await flush(() => f.app.render("a", true, f.snapshot("working", 2)));
    await flush(() => f.app.render("a", true, f.snapshot("idle", 3)));
    await flush(submit);
    expect(f.client.orchestrationAgentSend).toHaveBeenCalledTimes(1);
    await flush(() => f.choose("Alpha")); await flush(submit);
    expect(f.client.orchestrationAgentSend).toHaveBeenCalledTimes(2);
  });

  it.each(["herdr_outcome_unknown", "transport_error", "request_not_dispatched", "toString"])("never replays an uncertain %s outcome after reselection", async code => {
    const f = directFixture();
    vi.mocked(f.client.orchestrationAgentSend).mockRejectedValueOnce(new CockpitClientError("http_error", "Unknown", { operationCode: code }));
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha"));
    await flush(() => text("Keep this")); await flush(submit);
    expect(host.textContent).toContain("Delivery is uncertain");
    expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe("Keep this");
    await flush(() => f.app.render("a", true, f.snapshot("working", 2)));
    await flush(() => f.app.render("a", true, f.snapshot("idle", 3)));
    await flush(() => f.choose("Alpha")); await flush(submit);
    expect(host.textContent).toContain("Delivery is uncertain");
    expect(f.client.orchestrationAgentSend).toHaveBeenCalledTimes(1);
  });

  it("treats a network failure without a code as uncertain", async () => {
    const f = directFixture();
    vi.mocked(f.client.orchestrationAgentSend).mockRejectedValueOnce(new Error("Network unavailable"));
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha"));
    await flush(() => text("Keep this")); await flush(submit);
    await flush(() => f.choose("Alpha")); await flush(submit);
    expect(host.textContent).toContain("Delivery is uncertain");
    expect(f.client.orchestrationAgentSend).toHaveBeenCalledTimes(1);
  });

  it.each(["accepted", "busy", "uncertain", "stale"])("does not attribute a late %s outcome to a different recipient", async outcome => {
    const f = directFixture();
    let resolve!: () => void, reject!: (error: Error) => void;
    vi.mocked(f.client.orchestrationAgentSend).mockImplementationOnce(() => new Promise<void>((yes, no) => { resolve = yes; reject = no; }));
    await flush(() => f.app.render()); await flush(() => f.choose("Alpha"));
    await flush(() => text("Hello")); await flush(submit);
    await flush(() => f.choose("Beta"));
    await flush(() => {
      if (outcome === "accepted") resolve();
      else reject(new CockpitClientError("http_error", "Alpha failure", { operationCode: outcome === "busy" ? "agent_not_idle" : outcome === "stale" ? "stale_identity" : "herdr_outcome_unknown" }));
    });
    expect(host.querySelector(".inbox-head")?.textContent).toContain("Beta");
    expect(host.textContent).not.toContain("Submitted to Alpha");
    expect(host.textContent).not.toContain("Not sent.");
    expect(host.textContent).not.toContain("Alpha failure");
    expect(host.textContent).not.toContain("Delivery is uncertain");
    await flush(() => f.choose("Alpha"));
    expect(host.textContent).not.toContain("Submitted to Alpha");
    if (outcome === "uncertain") {
      expect(host.textContent).toContain("Delivery is uncertain");
      await flush(submit);
      expect(f.client.orchestrationAgentSend).toHaveBeenCalledTimes(1);
    }
  });
});
