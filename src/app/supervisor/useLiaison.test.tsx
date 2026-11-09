// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { WorkbenchProps } from "../shell/Workbench";
import { useLiaison } from "./useLiaison";
import { useTaskHistoryMessages } from "./useTaskHistoryMessages";

const mockObserver = vi.hoisted(() => ({ snapshot: null as unknown, connected: false, busy: false, error: null, mutationFailure: { current: null } }));
vi.mock("./useSupervisor", () => ({ useSupervisor: () => mockObserver }));

it("backs off unavailable liaison lookups and resets after recovery", async () => {
  vi.useFakeTimers();
  const lookup = vi.fn().mockResolvedValue({ status: "unavailable", code: "offline", message: "offline" });
  let props = { client: { orchestrationLiaisonLookup: lookup }, state: { sessionId: "session", sync: "live", snapshot: null }, selection: { spaceId: null, paneId: null }, onReconnect: () => {} } as unknown as WorkbenchProps;
  const root = createRoot(document.createElement("div"));
  function Consumer() { useLiaison(props, () => {}, async () => {}, false); return null; }
  try {
    await act(async () => root.render(<Consumer />));
    for (const delay of [1500, 3000, 6000, 12000, 24000, 30000, 30000]) {
      const before = lookup.mock.calls.length;
      await act(async () => { await vi.advanceTimersByTimeAsync(delay - 1); });
      expect(lookup).toHaveBeenCalledTimes(before);
      await act(async () => { await vi.advanceTimersByTimeAsync(1); });
      expect(lookup).toHaveBeenCalledTimes(before + 1);
    }
    lookup.mockResolvedValueOnce({ status: "resolved", preferred_root_id: null, roots: [] });
    await act(async () => { await vi.advanceTimersByTimeAsync(30000); });
    expect(lookup).toHaveBeenCalledTimes(9);
    props = { ...props, state: { ...props.state, sessionId: "other-session" } };
    await act(async () => root.render(<Consumer />));
    expect(lookup).toHaveBeenCalledTimes(10);
    await act(async () => { await vi.advanceTimersByTimeAsync(1499); });
    expect(lookup).toHaveBeenCalledTimes(10);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(lookup).toHaveBeenCalledTimes(11);
  } finally {
    await act(async () => root.unmount());
    vi.useRealTimers();
  }
});

it("loads history for a disconnected liaison only while the observer is healthy", async () => {
  const history = vi.fn(() => new Promise<never>(() => {}));
  const mutate = vi.fn();
  const client = {
    orchestrationTaskHistory: history,
    orchestrationMutate: mutate,
    orchestrationLiaisonLookup: vi.fn(async () => ({ status: "resolved", preferred_root_id: "root", roots: [{ root_id: "root", connection: "disconnected", resume: "available" }] })),
  };
  mockObserver.snapshot = {
    session_id: "session", state_store_id: "store", revision: 1, tasks_token: "token",
    runs: [{ run_id: "root", root_id: "root", stage: "active", brief: "work", attachment: null }],
    board: { root_id: "root", tasks: [] }, questions: [],
    runtime: { status: "fresh", runs: [] },
  };
  mockObserver.connected = true;
  const props = { client, state: { sessionId: "session", sync: "live", snapshot: { server_instance: "server", panes: [] } }, selection: { spaceId: null, paneId: null }, onReconnect: () => {} } as unknown as WorkbenchProps;
  const host = document.createElement("div"), root = createRoot(host);
  let retry = () => {};
  let start = async () => {};
  function Consumer() {
    const liaison = useLiaison(props, () => {}, async () => {}, false);
    const messages = useTaskHistoryMessages(props.client, liaison.root && liaison.snapshot ? { sessionId: "session", stateStoreId: "store", rootId: "root", taskId: "task" } : null, 1, liaison.observationConnected);
    retry = messages.retry;
    start = liaison.start;
    return <output>{`${liaison.connected}:${liaison.observationConnected}:${messages.disconnected}:${liaison.actionEnabled}`}</output>;
  }
  try {
    await act(async () => { root.render(<Consumer />); for (let i = 0; i < 8; i++) await Promise.resolve(); });
    expect(host.textContent).toBe("false:true:false:true");
    expect(history).toHaveBeenCalledOnce();
    mockObserver.connected = false;
    await act(async () => { root.render(<Consumer />); });
    expect(host.textContent).toBe("false:false:true:false");
    await act(async () => retry());
    await act(async () => start());
    expect(history).toHaveBeenCalledOnce();
    expect(mutate).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    mockObserver.snapshot = null;
    mockObserver.connected = false;
  }
});
