// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { SessionSnapshotResponse, SpaceCpuStatusRequest, SpaceCpuStatusResponse } from "../../protocol/generated/v1";
import { SPACE_CPU_POLL_MS, useSpaceCpu } from "./spaceCpu";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

it("stops and aborts CPU reads when the sidebar or page is hidden", async () => {
  vi.useFakeTimers();
  let visible = true;
  vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visible ? "visible" : "hidden");
  const spaceCpuStatus = vi.fn((_session: string, _request: SpaceCpuStatusRequest, _signal: AbortSignal) => new Promise<SpaceCpuStatusResponse>(() => {}));
  const client = { spaceCpuStatus } as unknown as CockpitClient;
  const snapshot = { session_id: "session", server_instance: "server", panes: [{ id: "p1", space_id: "s1" }] } as SessionSnapshotResponse;
  let shown = false;
  function Probe() { useSpaceCpu(client, "session", snapshot, shown); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { vi.advanceTimersByTime(SPACE_CPU_POLL_MS * 3); });
    expect(spaceCpuStatus).not.toHaveBeenCalled();
    shown = true;
    await act(async () => root.render(createElement(Probe)));
    const first = spaceCpuStatus.mock.calls[0]![2];
    visible = false;
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(first.aborted).toBe(true);
    await act(async () => { vi.advanceTimersByTime(SPACE_CPU_POLL_MS * 3); });
    expect(spaceCpuStatus).toHaveBeenCalledTimes(1);
    visible = true;
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(spaceCpuStatus).toHaveBeenCalledTimes(2);
    const second = spaceCpuStatus.mock.calls[1]![2];
    shown = false;
    await act(async () => root.render(createElement(Probe)));
    expect(second.aborted).toBe(true);
  } finally { await act(async () => root.unmount()); }
});
