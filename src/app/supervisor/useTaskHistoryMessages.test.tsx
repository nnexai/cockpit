// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { TaskHistoryMessages } from "../../protocol/generated/v1";
import { useTaskHistoryMessages, type HistoryScope } from "./useTaskHistoryMessages";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((nextResolve) => { resolve = nextResolve; });
  return { promise, resolve };
}

let root: Root | null = null, host: HTMLDivElement;
afterEach(() => { if (root) act(() => root!.unmount()); root = null; host?.remove(); });
const scope: HistoryScope = { sessionId: "session", stateStoreId: "store", rootId: "root", taskId: "task" };
function response(overrides: Partial<TaskHistoryMessages> = {}): TaskHistoryMessages {
  return { session_id: "session", state_store_id: "store", root_id: "root", task_id: "task", revision: 1, digest: "digest", not_modified: false, messages: [], runs: [], cleanup: [], task_ownership: [], unreflected_inputs: [], ...overrides };
}
it("loads only selected history, coalesces revisions and retains data on digest hits", async () => {
  const first = deferred<TaskHistoryMessages>(), second = deferred<TaskHistoryMessages>();
  const read = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  const client = { orchestrationTaskHistory: read } as unknown as CockpitClient;
  function Consumer({ selected, revision }: { selected: HistoryScope | null; revision: number }) {
    const history = useTaskHistoryMessages(client, selected, revision);
    return <output>{history.data ? history.pending ? "updating" : "loaded" : history.error ? "error" : "loading"}</output>;
  }
  host = document.createElement("div"); root = createRoot(host);
  await act(async () => root!.render(<Consumer selected={null} revision={1} />));
  expect(read).not.toHaveBeenCalled();
  await act(async () => root!.render(<Consumer selected={scope} revision={1} />));
  expect(read).toHaveBeenCalledTimes(1);
  await act(async () => { first.resolve(response()); await first.promise; });
  expect(host.textContent).toBe("loaded");
  await act(async () => root!.render(<Consumer selected={scope} revision={2} />));
  expect(host.textContent).toBe("updating");
  expect(read.mock.calls[1][0]).toMatchObject({ state_store_id: "store", known_digest: "digest" });
  await act(async () => root!.render(<Consumer selected={scope} revision={3} />));
  await act(async () => { second.resolve(response({ revision: 3, not_modified: true })); await second.promise; });
  expect(read).toHaveBeenCalledTimes(2);
  expect(host.textContent).toBe("loaded");
  await act(async () => root!.render(<Consumer selected={null} revision={4} />));
  expect(read).toHaveBeenCalledTimes(2);
});
it("ignores a late response after changing identity and starts reopening empty", async () => {
  const first = deferred<TaskHistoryMessages>(), second = deferred<TaskHistoryMessages>();
  const read = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  const client = { orchestrationTaskHistory: read } as unknown as CockpitClient;
  function Consumer({ selected }: { selected: HistoryScope | null }) {
    const history = useTaskHistoryMessages(client, selected, 1);
    return <output>{history.data?.task_id ?? "loading"}</output>;
  }
  host = document.createElement("div"); root = createRoot(host);
  await act(async () => root!.render(<Consumer selected={scope} />));
  await act(async () => root!.render(<Consumer selected={{ ...scope, taskId: "other" }} />));
  await act(async () => { first.resolve(response()); await first.promise; });
  expect(host.textContent).toBe("loading");
  await act(async () => { second.resolve(response({ task_id: "other" })); await second.promise; });
  expect(host.textContent).toBe("other");
  await act(async () => root!.render(<Consumer selected={null} />));
  await act(async () => root!.render(<Consumer selected={scope} />));
  expect(host.textContent).toBe("loading");
});
it("advances the retained revision on digest hits and rejects later regressions", async () => {
  const read = vi.fn()
    .mockResolvedValueOnce(response({ revision: 5 }))
    .mockResolvedValueOnce(response({ revision: 8, not_modified: true }))
    .mockResolvedValueOnce(response({ revision: 7, digest: "older" }));
  const client = { orchestrationTaskHistory: read } as unknown as CockpitClient;
  function Consumer({ token }: { token: string }) {
    const history = useTaskHistoryMessages(client, scope, 1, true, token);
    return <output>{history.data?.revision}:{history.error ? "error" : "ok"}</output>;
  }
  host = document.createElement("div"); root = createRoot(host);
  await act(async () => root!.render(<Consumer token="a" />));
  expect(host.textContent).toBe("5:ok");
  await act(async () => root!.render(<Consumer token="b" />));
  expect(host.textContent).toBe("8:ok");
  await act(async () => root!.render(<Consumer token="c" />));
  expect(read).toHaveBeenCalledTimes(3);
  expect(host.textContent).toBe("8:error");
});

it("reuses selected evidence and its digest after a transient disconnect", async () => {
  const reconnect = deferred<TaskHistoryMessages>();
  const messages = [{ message_id: "evidence" }] as TaskHistoryMessages["messages"];
  const read = vi.fn().mockResolvedValueOnce(response({ messages })).mockReturnValueOnce(reconnect.promise);
  const client = { orchestrationTaskHistory: read } as unknown as CockpitClient;
  function Consumer({ connected }: { connected: boolean }) {
    const history = useTaskHistoryMessages(client, scope, 1, connected);
    return <output>{`${history.disconnected ? "disconnected" : history.pending ? "updating" : history.error ? "error" : "loaded"}:${history.data?.messages.length ?? 0}`}</output>;
  }
  host = document.createElement("div"); root = createRoot(host);
  await act(async () => root!.render(<Consumer connected />));
  expect(host.textContent).toBe("loaded:1");
  await act(async () => root!.render(<Consumer connected={false} />));
  expect(host.textContent).toBe("disconnected:1");
  await act(async () => root!.render(<Consumer connected />));
  expect(read).toHaveBeenCalledTimes(2);
  expect(read.mock.calls[1][0]).toMatchObject({ known_digest: "digest" });
  expect(host.textContent).toBe("updating:1");
  await act(async () => { reconnect.resolve(response({ not_modified: true })); await reconnect.promise; });
  expect(host.textContent).toBe("loaded:1");
});

it.each(["sessionId", "stateStoreId", "rootId", "taskId"] as const)("does not reuse evidence after disconnected %s changes", async field => {
  const read = vi.fn().mockResolvedValue(response());
  const client = { orchestrationTaskHistory: read } as unknown as CockpitClient;
  function Consumer({ selected, connected }: { selected: HistoryScope; connected: boolean }) {
    const history = useTaskHistoryMessages(client, selected, 1, connected);
    return <output>{history.data ? "loaded" : "empty"}</output>;
  }
  host = document.createElement("div"); root = createRoot(host);
  await act(async () => root!.render(<Consumer selected={scope} connected />));
  await act(async () => root!.render(<Consumer selected={{ ...scope, [field]: "other" }} connected={false} />));
  await act(async () => root!.render(<Consumer selected={scope} connected={false} />));
  expect(host.textContent).toBe("empty");
  await act(async () => root!.render(<Consumer selected={scope} connected />));
  expect(read.mock.calls[1][0]).not.toHaveProperty("known_digest");
});

it("ignores an aborted response on reconnect and fences cached evidence by client", async () => {
  const late = deferred<TaskHistoryMessages>(), reconnect = deferred<TaskHistoryMessages>();
  const read = vi.fn().mockResolvedValueOnce(response()).mockReturnValueOnce(late.promise).mockReturnValueOnce(reconnect.promise);
  const otherRead = vi.fn().mockResolvedValue(response());
  const client = { orchestrationTaskHistory: read } as unknown as CockpitClient;
  const otherClient = { orchestrationTaskHistory: otherRead } as unknown as CockpitClient;
  function Consumer({ activeClient, connected, revision }: { activeClient: CockpitClient; connected: boolean; revision: number }) {
    const history = useTaskHistoryMessages(activeClient, scope, revision, connected);
    return <output>{`${history.data?.revision ?? 0}:${history.error ? "error" : "ok"}`}</output>;
  }
  host = document.createElement("div"); root = createRoot(host);
  await act(async () => root!.render(<Consumer activeClient={client} connected revision={1} />));
  await act(async () => root!.render(<Consumer activeClient={client} connected revision={2} />));
  await act(async () => root!.render(<Consumer activeClient={client} connected={false} revision={2} />));
  await act(async () => root!.render(<Consumer activeClient={client} connected revision={2} />));
  expect(read.mock.calls[2][0]).toHaveProperty("known_digest", "digest");
  await act(async () => { late.resolve(response({ revision: 9, digest: "stale" })); await late.promise; });
  await act(async () => { reconnect.resolve(response({ revision: 2, not_modified: true })); await reconnect.promise; });
  expect(host.textContent).toBe("2:ok");
  await act(async () => root!.render(<Consumer activeClient={otherClient} connected revision={1} />));
  expect(otherRead.mock.calls[0][0]).not.toHaveProperty("known_digest");
});
