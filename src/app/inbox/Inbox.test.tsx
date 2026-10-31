// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { InboxView, OrchestrationWaitResponse, SessionSnapshotResponse } from "../../protocol/generated/v1";
import type { AgentAttentionState } from "../session/agentAttention";
import { Inbox } from "./Inbox";

const attention = { availability: "ready", response: null } as AgentAttentionState;
const view = (session_id: string): InboxView => ({ session_id, state_store_id: `store-${session_id}`, revision: 1, tasks_token: "token", observation: { status: "fresh", observed_at: "now" }, owners: [{ root_id: "owner", label: "Liaison", kind: "supervisor", stage: "active", harness: "omp", availability: "available", location: null, undelivered: 0 }], threads: [], unmanaged_agents: [] });
let root: Root | null = null, host: HTMLDivElement;
afterEach(() => { if (root) act(() => root!.unmount()); root = null; host?.remove(); });
async function flush(fn: () => void = () => {}) { await act(async () => { fn(); for (let i = 0; i < 8; i++) await Promise.resolve(); }); }
function setup(client: CockpitClient, sessionId = "a", directSnapshot?: SessionSnapshotResponse) {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const render = (id: string, active = true) => root!.render(createElement(Inbox, { client, sessionId: id, active, currentSpace: null, snapshot: directSnapshot ?? { session_id: id, server_instance: "server", panes: [], agents: [] } as unknown as SessionSnapshotResponse, attention, onOpenLocation: async () => {} }));
  return { render: (id = sessionId, active = true) => render(id, active) };
}
function text(value: string) {
  const field = host.querySelector(".inbox:not([hidden]) textarea") as HTMLTextAreaElement;
  Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(field, value);
  field.dispatchEvent(new Event("input", { bubbles: true }));
}
function submit() { host.querySelector(".inbox:not([hidden]) form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); }
function fixture() {
  const waits: Array<(value: OrchestrationWaitResponse) => void> = [];
  const client = { orchestrationInbox: vi.fn(async ({ session_id }: { session_id: string }) => view(session_id)), orchestrationLiaisonLookup: vi.fn(async () => ({ status: "resolved", preferred_root_id: "owner", lookup_revision: 1, roots: [] })), orchestrationWait: vi.fn((_request: unknown, signal?: AbortSignal) => new Promise<OrchestrationWaitResponse>((resolve, reject) => { waits.push(resolve); signal?.addEventListener("abort", () => reject(new Error("aborted")), { once: true }); })), orchestrationMutate: vi.fn(), orchestrationAgentSend: vi.fn(async () => {}) } as unknown as CockpitClient;
  return { client, waits };
}

describe("Inbox", () => {
  it("waits session-wide and refetches only when changed", async () => {
    const { client, waits } = fixture(), app = setup(client);
    await flush(() => app.render());
    expect(client.orchestrationWait).toHaveBeenCalledWith({ root_id: null, after_revision: 1, after_tasks_token: "token", timeout_ms: 30000 }, expect.any(AbortSignal));
    await flush(() => waits[0]!({ revision: 1, tasks_token: "token", changed: false }));
    expect(client.orchestrationInbox).toHaveBeenCalledTimes(1);
    await flush(() => waits[1]!({ revision: 2, tasks_token: "changed", changed: true }));
    expect(client.orchestrationInbox).toHaveBeenCalledTimes(2);
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
    const question = (id: string, run: string) => ({ run_id: run, question_message_id: id, asked_at: "now", text: { text: `Question ${id}`, source: { kind: "owner" as const, run_id: run, harness: "omp" }, message_id: id, at: "now" }, choices: [], receipt: { status: "unresolved" as const } });
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner", "other"], title: { text: "Work", source: { kind: "recorded" }, message_id: null, at: null }, next: null, project: null, request: null, questions: [question("q1", "owner"), question("q2", "other")], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "message", to_run_id: "other", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Work"))!.click());
    await flush(() => (host.querySelectorAll<HTMLButtonElement>(".inbox-question .inbox-q-head")[1]!).click());
    await flush(() => text("Use the second option")); await flush(submit);
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "message_send", to_run_id: "other", kind: "answer", in_reply_to: "q2", text: "Use the second option" });
  });
  it("selects and answers the right root when questions share a message ID", async () => {
    const { client, waits } = fixture(), app = setup(client);
    const inbox = view("a");
    const question = (run: string) => ({ run_id: run, question_message_id: "shared", asked_at: "now", text: { text: `Question from ${run}`, source: { kind: "owner" as const, run_id: run, harness: "omp" }, message_id: "shared", at: "now" }, choices: [], receipt: { status: "unresolved" as const } });
    const first = question("owner"), second = question("other");
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner", "other"], title: { text: "Work", source: { kind: "recorded" }, message_id: null, at: null }, next: null, project: null, request: null, questions: [first, second], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], updated_at: "now" }];
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
    const question = (run: string) => ({ run_id: run, question_message_id: "shared", asked_at: "now", text: { text: `Question from ${run}`, source: { kind: "owner" as const, run_id: run, harness: "omp" }, message_id: "shared", at: "now" }, choices: [], receipt: { status: "unresolved" as const } });
    const first = question("owner"), second = question("other");
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner", "other"], title: { text: "Work", source: { kind: "recorded" }, message_id: null, at: null }, next: null, project: null, request: null, questions: [first, second], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], updated_at: "now" }];
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
    const question = { run_id: "owner", question_message_id: "old", asked_at: "now", text: { text: "Which one?", source: { kind: "owner" as const, run_id: "owner", harness: "omp" }, message_id: "old", at: "now" }, choices: [], receipt: { status: "unresolved" as const } };
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "decision", owner_root_ids: ["owner"], title: { text: "Work", source: { kind: "recorded" }, message_id: null, at: null }, next: null, project: null, request: null, questions: [question], recommendation: null, reply: null, unread_result_message_id: null, tasks: [], receipts: [], facts: [], updated_at: "now" }];
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
  it("marks only the displayed result message read", async () => {
    const { client } = fixture(), app = setup(client);
    const inbox = view("a");
    inbox.threads = [{ thread_id: "thread", state: "needs_you", need: "answer", owner_root_ids: ["owner"], title: { text: "Result", source: { kind: "recorded" }, message_id: null, at: null }, next: null, project: null, request: null, questions: [], recommendation: null, reply: null, unread_result_message_id: "result-id", tasks: [], receipts: [], facts: [], updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "done" } });
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Result"))!.click());
    expect(client.orchestrationMutate).not.toHaveBeenCalled();
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent === "Mark read")!.click());
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toEqual({ action: "thread_mark_read", thread_id: "thread", through_message_id: "result-id" });
  });
  it("fences a task follow-up with its owner's current task revision", async () => {
    const { client } = fixture(), app = setup(client);
    const inbox = view("a");
    inbox.threads = [{ thread_id: "thread", state: "underway", need: null, owner_root_ids: ["owner"], title: { text: "Task", source: { kind: "recorded" }, message_id: null, at: null }, next: null, project: null, request: null, questions: [], recommendation: null, reply: null, unread_result_message_id: null, tasks: [{ root_id: "owner", task_id: "task-id", title: "Task", project: null, lane: "working", run: null }], receipts: [], facts: [], updated_at: "now" }];
    vi.mocked(client.orchestrationInbox).mockResolvedValue(inbox);
    client.orchestrationSnapshot = vi.fn(async () => ({ session_id: "a", state_store_id: "store-a", board: { root_id: "owner", tasks: [{ task: { task_id: "task-id", task_revision: "revision" } }] } })) as unknown as CockpitClient["orchestrationSnapshot"];
    vi.mocked(client.orchestrationMutate).mockResolvedValue({ revision: 2, result: { result: "message", to_run_id: "owner", seq: 1, duplicate: false, stale: false } });
    await flush(() => app.render());
    await flush(() => Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Task"))!.click());
    await flush(() => { const select = host.querySelector<HTMLSelectElement>('select[aria-label="Follow-up target"]')!; select.value = "task-id"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    await flush(() => text("Please change it")); await flush(submit);
    expect(client.orchestrationSnapshot).toHaveBeenCalledWith({ session_id: "a", root_id: "owner" });
    expect(vi.mocked(client.orchestrationMutate).mock.calls[0]![0].action).toMatchObject({ action: "thread_handoff", thread_id: "thread", to_run_id: "owner", task_id: "task-id", expected_task_revision: "revision" });
  });
  it("sends a direct request only with the captured occupant identity", async () => {
    const { client } = fixture();
    const target = { endpoint_identity: "endpoint", workspace_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", agent_kind: "omp", agent_name: "Agent", native_session_id: null, native_session_file: null, state_changed_at: "now" };
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
    const target = { endpoint_identity: "endpoint", workspace_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", agent_kind: "omp", agent_name: "Agent", native_session_id: null, native_session_file: null, state_changed_at: "now" };
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
    const target = { endpoint_identity: "endpoint", workspace_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", agent_kind: "omp", agent_name: "Agent", native_session_id: null, native_session_file: null, state_changed_at: "now" };
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
});
