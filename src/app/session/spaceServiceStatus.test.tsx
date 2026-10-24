// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { SessionSnapshotResponse, SpaceServiceStatus, SpaceServiceStatusRequest, SpaceServiceStatusResponse } from "../../protocol/generated/v1";
import { SPACE_SERVICE_DEBOUNCE_MS, SPACE_SERVICE_POLL_MS, useSpaceServiceStatus } from "./spaceServiceStatus";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(next => { resolve = next; });
  return { promise, resolve };
}

const service: SpaceServiceStatus = { key: "dev", pane_id: "p1", tab_id: null, title: "dev server", status: "healthy", port: 5173 };
function snapshot(sessionId = "session", serverInstance = "server"): SessionSnapshotResponse {
  return {
    session_id: sessionId, server_instance: serverInstance, version: "1", protocol: 1,
    focused_space_id: "s1", focused_tab_id: "t1", focused_pane_id: "p1",
    spaces: [{ id: "s1", label: "app", number: 1, tab_count: 1, pane_count: 1, focused: true, agent_status: "idle", git: null }],
    tabs: [{ id: "t1", space_id: "s1", label: "terminal", number: 1, pane_count: 1, focused: true, focused_pane_id: "p1" }],
    panes: [{ id: "p1", terminal_id: "terminal", space_id: "s1", tab_id: "t1", title: null, focused: true, agent: null, agent_status: "idle", revision: 0 }], agents: [],
  };
}

it("reads once on open without overlapping or polling an empty registry", async () => {
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const pending = deferred<SpaceServiceStatusResponse>();
  const spaceServiceStatus = vi.fn().mockReturnValueOnce(pending.promise).mockResolvedValue({ session_id: "session", services: [service] });
  const client = { spaceServiceStatus } as unknown as CockpitClient;
  const currentSnapshot = snapshot();
  let services: ReadonlyMap<string, readonly SpaceServiceStatus[]> = new Map();
  function Probe() { services = useSpaceServiceStatus(client, "session", currentSnapshot, true); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS * 3); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(1);
    await act(async () => pending.resolve({ session_id: "session", services: [] }));
    expect(services.size).toBe(0);
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS * 10); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(1);
    expect(spaceServiceStatus).toHaveBeenCalledWith("session", { server_instance: "server", pane_ids: ["p1"] }, expect.any(AbortSignal));
  } finally { await act(async () => root.unmount()); }
});

it("suspends and aborts reads while either the sidebar or page is hidden", async () => {
  vi.useFakeTimers();
  let pageVisible = true;
  vi.spyOn(document, "visibilityState", "get").mockImplementation(() => pageVisible ? "visible" : "hidden");
  const spaceServiceStatus = vi.fn((_session: string, _request: SpaceServiceStatusRequest, _signal: AbortSignal) => new Promise<SpaceServiceStatusResponse>(() => {}));
  const client = { spaceServiceStatus } as unknown as CockpitClient;
  const currentSnapshot = snapshot();
  let shown = false;
  function Probe() { useSpaceServiceStatus(client, "session", currentSnapshot, shown); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS * 3); });
    expect(spaceServiceStatus).not.toHaveBeenCalled();
    shown = true;
    await act(async () => root.render(createElement(Probe)));
    const first = spaceServiceStatus.mock.calls[0][2];
    pageVisible = false;
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(first.aborted).toBe(true);
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS * 3); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(1);
    pageVisible = true;
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(spaceServiceStatus).toHaveBeenCalledTimes(2);
    const second = spaceServiceStatus.mock.calls[1][2];
    shown = false;
    await act(async () => root.render(createElement(Probe)));
    expect(second.aborted).toBe(true);
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS * 3); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(2);
  } finally { await act(async () => root.unmount()); }
});

it.each(["session", "server", "client"] as const)("aborts and discards late results after the %s identity changes", async identity => {
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const pending = deferred<SpaceServiceStatusResponse>();
  const read = vi.fn((_session: string, _request: SpaceServiceStatusRequest, _signal: AbortSignal) => pending.promise);
  let client = { spaceServiceStatus: read } as unknown as CockpitClient;
  let sessionId = "session";
  let currentSnapshot = snapshot();
  let services: ReadonlyMap<string, readonly SpaceServiceStatus[]> = new Map();
  function Probe() { services = useSpaceServiceStatus(client, sessionId, currentSnapshot, true); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    const signal = read.mock.calls[0][2];
    if (identity === "session") { sessionId = "other"; currentSnapshot = snapshot(sessionId); }
    if (identity === "server") currentSnapshot = snapshot(sessionId, "replacement");
    const replacement = vi.fn().mockResolvedValue({ session_id: sessionId, services: [] });
    if (identity === "client") client = { spaceServiceStatus: replacement } as unknown as CockpitClient;
    else read.mockImplementation(() => Promise.resolve({ session_id: sessionId, services: [] }));
    await act(async () => root.render(createElement(Probe)));
    expect(signal.aborted).toBe(true);
    await act(async () => pending.resolve({ session_id: "session", services: [service] }));
    expect(services.size).toBe(0);
  } finally { await act(async () => root.unmount()); }
});

it("joins pane, tab and Space against each latest snapshot, never trusting registry ownership", async () => {
  const spaceServiceStatus = vi.fn().mockResolvedValue({ session_id: "session", services: [service, { ...service, key: "tab", tab_id: "t1" }, { ...service, key: "wrong-tab", tab_id: "missing" }, { ...service, key: "dead", pane_id: "missing" }] });
  const client = { spaceServiceStatus } as unknown as CockpitClient;
  let currentSnapshot = snapshot();
  let services: ReadonlyMap<string, readonly SpaceServiceStatus[]> = new Map();
  function Probe() { services = useSpaceServiceStatus(client, "session", currentSnapshot, true); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    expect(services.get("s1")?.map(item => item.key)).toEqual(["dev", "tab"]);
    currentSnapshot = { ...currentSnapshot, tabs: [{ ...currentSnapshot.tabs[0], space_id: "missing" }] };
    await act(async () => root.render(createElement(Probe)));
    expect(services.size).toBe(0);
    currentSnapshot = { ...currentSnapshot, tabs: [{ ...currentSnapshot.tabs[0], space_id: "s1" }], spaces: [] };
    await act(async () => root.render(createElement(Probe)));
    expect(services.size).toBe(0);
    currentSnapshot = snapshot();
    currentSnapshot = {
      ...currentSnapshot, spaces: [{ ...currentSnapshot.spaces[0], id: "moved" }],
      panes: [{ ...currentSnapshot.panes[0], space_id: "moved" }],
      tabs: [{ ...currentSnapshot.tabs[0], space_id: "moved" }],
    };
    await act(async () => root.render(createElement(Probe)));
    expect(services.has("s1")).toBe(false);
    expect(services.get("moved")?.map(item => item.key)).toEqual(["dev", "tab"]);
    expect(spaceServiceStatus).toHaveBeenCalledTimes(1);
  } finally { await act(async () => root.unmount()); }
});

it("retains the last successful read quietly on errors and retries only at the normal cadence", async () => {
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const spaceServiceStatus = vi.fn().mockResolvedValueOnce({ session_id: "session", services: [service] }).mockRejectedValueOnce(new Error("offline")).mockResolvedValue({ session_id: "session", services: [] });
  const client = { spaceServiceStatus } as unknown as CockpitClient;
  const currentSnapshot = snapshot();
  let services: ReadonlyMap<string, readonly SpaceServiceStatus[]> = new Map();
  function Probe() { services = useSpaceServiceStatus(client, "session", currentSnapshot, true); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(2);
    expect(services.get("s1")).toEqual([service]);
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS - 1); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(2);
    await act(async () => { vi.advanceTimersByTime(1); });
    expect(services.size).toBe(0);
  } finally { await act(async () => root.unmount()); }
});

it("polls only health-port rows, deduplicates unchanged results, and stops when none remain", async () => {
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const pending = deferred<SpaceServiceStatusResponse>();
  const spaceServiceStatus = vi.fn()
    .mockResolvedValueOnce({ session_id: "session", services: [service] })
    .mockReturnValueOnce(pending.promise)
    .mockResolvedValue({ session_id: "session", services: [{ ...service, port: null }] });
  const client = { spaceServiceStatus } as unknown as CockpitClient;
  const currentSnapshot = snapshot();
  let services: ReadonlyMap<string, readonly SpaceServiceStatus[]> = new Map();
  function Probe() { services = useSpaceServiceStatus(client, "session", currentSnapshot, true); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    const initial = services;
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS); });
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS * 3); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(2);
    await act(async () => pending.resolve({ session_id: "session", services: [{ ...service }] }));
    expect(services).toBe(initial);
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS); });
    expect(services.get("s1")?.[0].port).toBeNull();
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS * 10); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(3);
  } finally { await act(async () => root.unmount()); }
});

it("debounces sorted unique pane sets across all Spaces, ignoring unrelated snapshot updates", async () => {
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const spaceServiceStatus = vi.fn().mockResolvedValue({ session_id: "session", services: [] });
  const client = { spaceServiceStatus } as unknown as CockpitClient;
  let currentSnapshot = snapshot();
  const firstPane = currentSnapshot.panes[0];
  currentSnapshot = { ...currentSnapshot, panes: [{ ...firstPane, id: "p3", space_id: "other" }, firstPane, firstPane] };
  function Probe() { useSpaceServiceStatus(client, "session", currentSnapshot, true); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    expect(spaceServiceStatus.mock.calls[0][1]).toEqual({ server_instance: "server", pane_ids: ["p1", "p3"] });
    currentSnapshot = { ...currentSnapshot, focused_pane_id: "p3", panes: [...currentSnapshot.panes].reverse() };
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS * 2); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(1);
    currentSnapshot = { ...currentSnapshot, panes: [...currentSnapshot.panes, { ...firstPane, id: "p2" }] };
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_DEBOUNCE_MS - 1); });
    currentSnapshot = { ...currentSnapshot, panes: currentSnapshot.panes.filter(pane => pane.id !== "p3") };
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_DEBOUNCE_MS - 1); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(1);
    await act(async () => { vi.advanceTimersByTime(1); });
    expect(spaceServiceStatus.mock.calls[1][1]).toEqual({ server_instance: "server", pane_ids: ["p1", "p2"] });
    currentSnapshot = { ...currentSnapshot, server_instance: "replacement" };
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_DEBOUNCE_MS); });
    expect(spaceServiceStatus.mock.calls[2][1]).toEqual({ server_instance: "replacement", pane_ids: ["p1", "p2"] });
  } finally { await act(async () => root.unmount()); }
});

it("aborts obsolete pane-set reads and discovers again when shown after an idle result", async () => {
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const pending = deferred<SpaceServiceStatusResponse>();
  const spaceServiceStatus = vi.fn().mockReturnValueOnce(pending.promise).mockResolvedValue({ session_id: "session", services: [] });
  const client = { spaceServiceStatus } as unknown as CockpitClient;
  let currentSnapshot = snapshot();
  let shown = true;
  let services: ReadonlyMap<string, readonly SpaceServiceStatus[]> = new Map();
  function Probe() { services = useSpaceServiceStatus(client, "session", currentSnapshot, shown); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    const first = spaceServiceStatus.mock.calls[0][2] as AbortSignal;
    currentSnapshot = { ...currentSnapshot, panes: [] };
    await act(async () => root.render(createElement(Probe)));
    expect(first.aborted).toBe(true);
    await act(async () => pending.resolve({ session_id: "session", services: [service] }));
    expect(services.size).toBe(0);
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_DEBOUNCE_MS); });
    expect(spaceServiceStatus.mock.calls[1][1]).toEqual({ server_instance: "server", pane_ids: [] });
    shown = false;
    await act(async () => root.render(createElement(Probe)));
    shown = true;
    await act(async () => root.render(createElement(Probe)));
    expect(spaceServiceStatus).toHaveBeenCalledTimes(3);
  } finally { await act(async () => root.unmount()); }
});

it("settles newly added panes at one and three seconds, then stops without health ports", async () => {
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const spaceServiceStatus = vi.fn().mockResolvedValue({ session_id: "session", services: [] });
  const client = { spaceServiceStatus } as unknown as CockpitClient;
  let currentSnapshot = snapshot();
  function Probe() { useSpaceServiceStatus(client, "session", currentSnapshot, true); return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    currentSnapshot = { ...currentSnapshot, panes: [...currentSnapshot.panes, { ...currentSnapshot.panes[0], id: "p2" }] };
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_DEBOUNCE_MS); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(2);
    await act(async () => { vi.advanceTimersByTime(999); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(2);
    await act(async () => { vi.advanceTimersByTime(1); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(3);
    await act(async () => { vi.advanceTimersByTime(1_999); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(3);
    await act(async () => { vi.advanceTimersByTime(1); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(4);
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS * 10); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(4);
  } finally { await act(async () => root.unmount()); }
});

it.each(["pane-set", "hidden", "unmount"] as const)("cancels bounded settlement reads after %s", async change => {
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const spaceServiceStatus = vi.fn().mockResolvedValue({ session_id: "session", services: [] });
  const client = { spaceServiceStatus } as unknown as CockpitClient;
  let currentSnapshot = snapshot();
  let shown = true;
  function Probe() { useSpaceServiceStatus(client, "session", currentSnapshot, shown); return null; }
  const root = createRoot(document.createElement("div"));
  let mounted = true;
  try {
    await act(async () => root.render(createElement(Probe)));
    currentSnapshot = { ...currentSnapshot, panes: [...currentSnapshot.panes, { ...currentSnapshot.panes[0], id: "p2" }] };
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_DEBOUNCE_MS); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(2);
    if (change === "pane-set") currentSnapshot = snapshot();
    if (change === "hidden") shown = false;
    if (change === "unmount") { await act(async () => root.unmount()); mounted = false; }
    else await act(async () => root.render(createElement(Probe)));
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_DEBOUNCE_MS); });
    const reads = change === "pane-set" ? 3 : 2;
    expect(spaceServiceStatus).toHaveBeenCalledTimes(reads);
    await act(async () => { vi.advanceTimersByTime(SPACE_SERVICE_POLL_MS * 10); });
    expect(spaceServiceStatus).toHaveBeenCalledTimes(reads);
  } finally { if (mounted) await act(async () => root.unmount()); }
});
