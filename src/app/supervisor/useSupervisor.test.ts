// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { OrchestrationSnapshot, OrchestrationWaitResponse } from "../../protocol/generated/v1";
import { acceptsSupervisorSnapshot, useSupervisor } from "./useSupervisor";

const snapshot: OrchestrationSnapshot = {
  state_store_id: "store", session_id: "session-a", revision: 12, tasks_token: "external-edit-a",
  roots: [], board: { root_id: "root-a", path: "/state/tasks/root-a.md", doc_revision: "doc-a", unidentified_items: 0, diagnostics: [], tasks: [] },
  runs: [], messages: [], questions: [], task_ownership: [], intents: [], assignment_intents: [], attention: [], unmanaged_agents: [],
  cleanup: [], restoration: { reattached: 0, total_attachments: 0, cleanup_pending: 0 },
  runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: "2026-10-07T12:00:00Z", runs: [] },
};
interface Deferred<T> { promise: Promise<T>; resolve(value: T): void }
function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
let root: Root | null = null, host: HTMLDivElement;
afterEach(() => { if (root) act(() => root!.unmount()); root = null; host?.remove(); vi.restoreAllMocks(); });
async function flush(change: () => void = () => {}) {
  await act(async () => { change(); for (let index = 0; index < 8; index++) await Promise.resolve(); });
}

describe("supervisor snapshot fences", () => {
  it("rejects stale machine revisions and wrong session/root responses", () => {
    expect(acceptsSupervisorSnapshot(snapshot, "session-a", "root-a", 13)).toBe(false);
    expect(acceptsSupervisorSnapshot(snapshot, "session-b", "root-a", 0)).toBe(false);
    expect(acceptsSupervisorSnapshot(snapshot, "session-a", "root-b", 0)).toBe(false);
  });
  it("accepts external Markdown changes and missing canonical boards", () => {
    expect(acceptsSupervisorSnapshot({ ...snapshot, tasks_token: "external-edit-b" }, "session-a", "root-a", 12)).toBe(true);
    expect(acceptsSupervisorSnapshot({ ...snapshot, board: null }, "session-a", "root-a", 12)).toBe(true);
  });
  it("renews silent root-scoped waits without rebuilding snapshots", async () => {
    const waits = [deferred<OrchestrationWaitResponse>(), deferred<OrchestrationWaitResponse>()];
    const snapshotRead = vi.fn(async () => snapshot), wait = vi.fn().mockReturnValueOnce(waits[0].promise).mockReturnValueOnce(waits[1].promise);
    const client = { orchestrationSnapshot: snapshotRead, orchestrationWait: wait } as unknown as CockpitClient;
    function Consumer() { useSupervisor(client, "session-a", "root-a", true, true); return null; }
    host = document.createElement("div"); root = createRoot(host);
    await flush(() => root!.render(createElement(Consumer)));
    expect(wait).toHaveBeenCalledWith({ root_id: "root-a", after_revision: 12, after_tasks_token: "external-edit-a", timeout_ms: 30_000 });
    await flush(() => waits[0].resolve({ revision: 14, tasks_token: "silent-token", changed: false }));
    expect(snapshotRead).toHaveBeenCalledOnce();
    expect(wait).toHaveBeenLastCalledWith({ root_id: "root-a", after_revision: 14, after_tasks_token: "silent-token", timeout_ms: 30_000 });
    await flush(() => root!.unmount()); root = null;
    await flush(() => waits[1].resolve({ revision: 15, tasks_token: "late", changed: false }));
    expect(wait).toHaveBeenCalledTimes(2);
  });
  it("ignores a late snapshot after changing session identity", async () => {
    const first = deferred<OrchestrationSnapshot>(), second = deferred<OrchestrationSnapshot>(), wait = vi.fn();
    const client = { orchestrationSnapshot: vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise), orchestrationWait: wait } as unknown as CockpitClient;
    function Consumer({ sessionId }: { sessionId: string }) { const state = useSupervisor(client, sessionId, "root-a", true, true); return createElement("output", null, state.snapshot?.session_id); }
    host = document.createElement("div"); root = createRoot(host);
    await flush(() => root!.render(createElement(Consumer, { sessionId: "session-a" })));
    await flush(() => root!.render(createElement(Consumer, { sessionId: "session-b" })));
    await flush(() => first.resolve(snapshot));
    expect(host.textContent).toBe(""); expect(wait).not.toHaveBeenCalled();
  });
});
