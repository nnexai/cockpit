// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { TaskHistoryMessages } from "../../protocol/generated/v1";
import { useTaskHistoryMessages } from "./useTaskHistoryMessages";

it("keeps the error visible during a retry and rejects an older revision", async () => {
  let resolveRetry!: (value: TaskHistoryMessages) => void;
  const retry = new Promise<TaskHistoryMessages>((resolve) => { resolveRetry = resolve; });
  const read = vi.fn().mockRejectedValueOnce(new Error("offline")).mockReturnValueOnce(retry).mockResolvedValueOnce({ session_id: "s", state_store_id: "store", root_id: "r", task_id: "t", revision: 4, digest: "d", not_modified: false, messages: [], runs: [], cleanup: [], task_ownership: [], unreflected_inputs: [] });
  const client = { orchestrationTaskHistory: read } as unknown as CockpitClient;
  function Consumer() {
    const history = useTaskHistoryMessages(client, { sessionId: "s", stateStoreId: "store", rootId: "r", taskId: "t" }, 4);
    return <button onClick={history.retry}>{history.error ? history.retrying ? "retrying" : "error" : history.data ? "loaded" : "loading"}</button>;
  }
  const host = document.createElement("div"), root = createRoot(host);
  try {
    await act(async () => root.render(<Consumer />));
    expect(host.textContent).toBe("error");
    await act(async () => host.querySelector("button")!.click());
    expect(host.textContent).toBe("retrying");
    await act(async () => { resolveRetry({ session_id: "s", state_store_id: "store", root_id: "r", task_id: "t", revision: 3, digest: "d", not_modified: false, messages: [], runs: [], cleanup: [], task_ownership: [], unreflected_inputs: [] }); await retry; });
    expect(read).toHaveBeenCalledTimes(2);
    expect(host.textContent).toBe("error");
    await act(async () => host.querySelector("button")!.click());
    expect(read).toHaveBeenCalledTimes(3);
    expect(host.textContent).toBe("loaded");
  } finally { act(() => root.unmount()); }
});
