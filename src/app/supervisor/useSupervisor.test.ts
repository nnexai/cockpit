// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient, LocalChangeEvent } from "../../client/CockpitClient";
import type { OrchestrationOverview } from "../../protocol/generated/v1";
import { acceptsSupervisorSnapshot, useSupervisor } from "./useSupervisor";

const snapshot: OrchestrationOverview = {
  state_store_id: "store", session_id: "session-a", revision: 12, tasks_token: "external-edit-a",
  roots: [], board: { root_id: "root-a", path: "/state/tasks/root-a.md", doc_revision: "doc-a", unidentified_items: 0, diagnostics: [], tasks: [] },
  runs: [], questions: [], unreflected_inputs: [], task_ownership: [], directives: [], thread_links: [], intents: [], assignment_intents: [], attention: [], unmanaged_agents: [],
  cleanup: [], restoration: { reattached: 0, total_attachments: 0, cleanup_pending: 0 },
  runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: "2026-10-07T12:00:00Z", runs: [] },
};
const ready: LocalChangeEvent = { stream_id: "stream", sequence: 1, kind: "ready", topics: ["orchestration"] };
let root: Root | null = null, host: HTMLDivElement;
afterEach(() => { if (root) act(() => root!.unmount()); root = null; host?.remove(); vi.useRealTimers(); vi.restoreAllMocks(); });
async function flush(change: () => void = () => {}) { await act(async () => { change(); for (let i = 0; i < 12; i++) await Promise.resolve(); }); }

it("fences session, root and revision while accepting equal-revision task edits", () => {
  expect(acceptsSupervisorSnapshot(snapshot, "session-a", "root-a", 13)).toBe(false);
  expect(acceptsSupervisorSnapshot(snapshot, "session-b", "root-a", 0)).toBe(false);
  expect(acceptsSupervisorSnapshot(snapshot, "session-a", "root-b", 0)).toBe(false);
  expect(acceptsSupervisorSnapshot({ ...snapshot, tasks_token: "new" }, "session-a", "root-a", 12)).toBe(true);
});

it("bootstraps and remains quiet until a scoped event, without a GUI wait", async () => {
  vi.useFakeTimers();
  let emit!: Parameters<CockpitClient["subscribeLocalChanges"]>[1];
  const read = vi.fn(async () => snapshot), wait = vi.fn();
  const subscribe = vi.fn<CockpitClient["subscribeLocalChanges"]>(async (_scope, callback) => { emit = callback; void callback(ready); return { close: vi.fn() }; });
  const client = { orchestrationSnapshot: read, orchestrationWait: wait, subscribeLocalChanges: subscribe } as unknown as CockpitClient;
  function Consumer() { const state = useSupervisor(client, "session-a", "root-a", true, true, "server"); return createElement("output", null, state.connected ? state.snapshot?.tasks_token : "offline"); }
  host = document.createElement("div"); root = createRoot(host);
  await flush(() => root!.render(createElement(Consumer)));
  expect(subscribe).toHaveBeenCalledWith({ kind: "orchestration", session_id: "session-a", server_instance: "server", state_store_id: "store", root_id: "root-a" }, expect.any(Function), expect.any(Function), expect.any(AbortSignal));
  expect(read).toHaveBeenCalledTimes(2); expect(host.textContent).toBe("external-edit-a");
  await act(async () => { await vi.advanceTimersByTimeAsync(100_000); });
  expect(read).toHaveBeenCalledTimes(2); expect(wait).not.toHaveBeenCalled();
  read.mockResolvedValue({ ...snapshot, tasks_token: "external-edit-b" });
  await act(async () => { await emit({ ...ready, kind: "changed", sequence: 2 }); });
  expect(host.textContent).toBe("external-edit-b");
});

it("ignores a bootstrap result from an old session", async () => {
  let resolveFirst!: (value: OrchestrationOverview) => void;
  const first = new Promise<OrchestrationOverview>(resolve => { resolveFirst = resolve; });
  const read = vi.fn().mockReturnValueOnce(first).mockReturnValue(new Promise(() => {}));
  const subscribe = vi.fn();
  const client = { orchestrationSnapshot: read, subscribeLocalChanges: subscribe } as unknown as CockpitClient;
  function Consumer({ sessionId }: { sessionId: string }) { const state = useSupervisor(client, sessionId, "root-a", true, true, "server"); return createElement("output", null, state.snapshot?.session_id); }
  host = document.createElement("div"); root = createRoot(host);
  await flush(() => root!.render(createElement(Consumer, { sessionId: "session-a" })));
  await flush(() => root!.render(createElement(Consumer, { sessionId: "session-b" })));
  await flush(() => resolveFirst(snapshot));
  expect(host.textContent).toBe(""); expect(subscribe).not.toHaveBeenCalled();
});
