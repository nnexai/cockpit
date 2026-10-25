// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { PaneSummary, SpaceGitStatus, SpaceSummary } from "../../protocol/generated/v1";
import { aheadBehindLabel, spaceCheckoutKey, useSpaceGitStatus } from "./spaceGitStatus";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(nextResolve => { resolve = nextResolve; });
  return { promise, resolve };
}

afterEach(() => { vi.useRealTimers(); });


it("polls while visible and skips polls while the page is hidden", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  const spaceGitStatus = vi.fn(async (sessionId: string) => ({ session_id: sessionId, spaces: [{ space_id: "w1", source: "herdr_checkout" as const, checkout: { state: "branch" as const, root: "/repo", branch: "main", upstream: { state: "tracked" as const, name: "origin/main", ahead: 1, behind: 0 } } }] }));
  const client = { spaceGitStatus } as unknown as CockpitClient;
  const seen: string[] = [];
  function Probe() {
    const status = useSpaceGitStatus(client, "session", "w1", 1000);
    seen.push(aheadBehindLabel(status.spaces.get("w1")));
    return null;
  }
  const host = document.createElement("div");
  const root = createRoot(host);
  let visibility: DocumentVisibilityState = "visible";
  vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visibility);
  try {
    await act(async () => root.render(createElement(Probe)));
    await act(async () => { await Promise.resolve(); });
    expect(spaceGitStatus).toHaveBeenCalledTimes(1);
    expect(seen.at(-1)).toBe("↑1");

    visibility = "hidden";
    await act(async () => { vi.advanceTimersByTime(3000); });
    expect(spaceGitStatus).toHaveBeenCalledTimes(1);

    visibility = "visible";
    await act(async () => { document.dispatchEvent(new Event("visibilitychange")); await Promise.resolve(); });
    expect(spaceGitStatus).toHaveBeenCalledTimes(2);
  } finally {
    await act(async () => root.unmount());
  }
});

it("rereads a plain Space when its first pane moves to another folder, and ignores other panes", () => {
  const plain: SpaceSummary = { id: "w1", label: "main", number: 1, tab_count: 1, pane_count: 2, focused: true, agent_status: "idle", git: null };
  const pane = (id: string, cwd?: string): PaneSummary => ({ id, terminal_id: `t-${id}`, space_id: "w1", tab_id: "w1:t1", title: null, focused: false, agent: null, agent_status: "idle", revision: 0, cwd });
  const key = spaceCheckoutKey([plain], [pane("w1:p1", "/src/app"), pane("w1:p2", "/tmp")]);
  expect(spaceCheckoutKey([plain], [pane("w1:p1", "/src/app"), pane("w1:p2", "/elsewhere")])).toBe(key);
  expect(spaceCheckoutKey([plain], [pane("w1:p1", "/src/other"), pane("w1:p2", "/tmp")])).not.toBe(key);
  expect(spaceCheckoutKey([plain], [pane("w1:p1"), pane("w1:p2", "/tmp")])).toBe(spaceCheckoutKey([plain], [pane("w1:p2", "/tmp")]));
});

it.each(["session", "checkout"] as const)("retains last-known status on a failed read, refreshes while hidden, and never carries it across %s changes", async identity => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const status: SpaceGitStatus = { space_id: "w1", source: "herdr_checkout", checkout: { state: "detached", root: "/repo" } };
  const spaceGitStatus = vi.fn().mockResolvedValueOnce({ session_id: "session", spaces: [status] }).mockRejectedValueOnce(new Error("ACL denied"));
  const client = { spaceGitStatus } as unknown as CockpitClient;
  let refresh!: () => void;
  let error: string | undefined;
  let spaces: ReadonlyMap<string, SpaceGitStatus> = new Map();
  let sessionId = "session";
  let checkoutKey = "w1";
  function Probe() {
    const poll = useSpaceGitStatus(client, sessionId, checkoutKey);
    refresh = poll.refresh; error = poll.error; spaces = poll.spaces;
    return null;
  }
  const root = createRoot(document.createElement("div"));
  const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  try {
    await act(async () => root.render(createElement(Probe)));
    expect(spaces.get("w1")).toEqual(status);
    visibility.mockReturnValue("hidden");
    await act(async () => refresh());
    expect(error).toBe("ACL denied");
    expect(spaces.get("w1")).toEqual(status);
    expect(spaceGitStatus).toHaveBeenCalledTimes(2);
    spaceGitStatus.mockResolvedValueOnce({ session_id: "session", spaces: [status] });
    await act(async () => refresh());
    expect(error).toBeUndefined();
    expect(spaces.get("w1")).toEqual(status);
    spaceGitStatus.mockRejectedValueOnce(new Error("new target unreadable"));
    if (identity === "session") sessionId = "other-session";
    else checkoutKey = "other-checkout";
    await act(async () => root.render(createElement(Probe)));
    await act(async () => refresh());
    expect(error).toBe("new target unreadable");
    expect(spaces.has("w1")).toBe(false);
  } finally { await act(async () => root.unmount()); visibility.mockRestore(); }
});

it("discards stale status when the same session's Space moves to another checkout", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const pending = deferred<{ session_id: string; spaces: SpaceGitStatus[] }>();
  const spaceGitStatus = vi.fn().mockReturnValueOnce(pending.promise).mockResolvedValue({ session_id: "session", spaces: [{ space_id: "w1", source: "pane_folder", checkout: { state: "detached", root: "/new" } }] });
  const client = { spaceGitStatus } as unknown as CockpitClient;
  let checkoutKey = "old";
  let spaces: ReadonlyMap<string, SpaceGitStatus> = new Map();
  function Probe() { spaces = useSpaceGitStatus(client, "session", checkoutKey).spaces; return null; }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(createElement(Probe)));
    checkoutKey = "new";
    await act(async () => root.render(createElement(Probe)));
    await act(async () => pending.resolve({ session_id: "session", spaces: [{ space_id: "w1", source: "pane_folder", checkout: { state: "detached", root: "/old" } }] }));
    expect(spaces.get("w1")?.checkout).toEqual({ state: "detached", root: "/new" });
  } finally { await act(async () => root.unmount()); }
});

it("reuses unchanged poll results without rendering, but publishes failures and recovery", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  const value: SpaceGitStatus = { space_id: "w1", source: "herdr_checkout", checkout: { state: "detached", root: "/repo" } };
  const spaceGitStatus = vi.fn(async () => ({ session_id: "session", spaces: [structuredClone(value)] }));
  const client = { spaceGitStatus } as unknown as CockpitClient;
  let current!: { spaces: ReadonlyMap<string, SpaceGitStatus>; error?: string; refresh(): void };
  let renders = 0;
  function Probe() { current = useSpaceGitStatus(client, "session", "w1", 1000); ++renders; return null; }
  const root = createRoot(document.createElement("div"));
  const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  try {
    await act(async () => root.render(createElement(Probe)));
    const spaces = current.spaces;
    const loadedRenders = renders;
    for (let poll = 0; poll < 3; ++poll) await act(async () => vi.advanceTimersByTime(1000));
    expect(spaceGitStatus).toHaveBeenCalledTimes(4);
    expect(renders).toBe(loadedRenders);
    expect(current.spaces).toBe(spaces);
    await act(async () => current.refresh());
    expect(spaceGitStatus).toHaveBeenCalledTimes(5);
    expect(renders).toBe(loadedRenders);
    spaceGitStatus.mockRejectedValueOnce(new Error("offline"));
    await act(async () => current.refresh());
    expect(renders).toBe(loadedRenders + 1);
    expect(current.error).toBe("offline");
    expect(current.spaces).toBe(spaces);
    spaceGitStatus.mockRejectedValueOnce(new Error("offline"));
    await act(async () => current.refresh());
    expect(renders).toBe(loadedRenders + 1);
    spaceGitStatus.mockRejectedValueOnce(new Error("denied"));
    await act(async () => current.refresh());
    expect(renders).toBe(loadedRenders + 2);
    expect(current.error).toBe("denied");
    await act(async () => current.refresh());
    expect(renders).toBe(loadedRenders + 3);
    expect(current.error).toBeUndefined();
    expect(current.spaces).toBe(spaces);
    await act(async () => current.refresh());
    expect(renders).toBe(loadedRenders + 3);
  } finally { await act(async () => root.unmount()); visibility.mockRestore(); }
});

it("publishes every Git status field, union transition and Space order change", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const branch = { state: "branch" as const, root: "/repo", branch: "main", upstream: { state: "tracked" as const, name: "origin/main", ahead: 0, behind: 0 } };
  const base: SpaceGitStatus = { space_id: "w1", source: "herdr_checkout", checkout: branch };
  const checkouts: SpaceGitStatus["checkout"][] = [
    { ...branch, upstream: { ...branch.upstream, ahead: 1 } },
    { ...branch, upstream: { ...branch.upstream, behind: 1 } },
    { ...branch, upstream: { ...branch.upstream, name: "origin/other" } },
    { ...branch, branch: "other" },
    { ...branch, root: "/other" },
    { state: "detached", root: "/repo" },
    { state: "detached", root: "/other" },
    { state: "unavailable", root: "/repo", code: "missing", message: "Missing" },
    { state: "unavailable", root: "/repo", code: "denied", message: "Missing" },
    { state: "unavailable", root: "/repo", code: "denied", message: "Denied" },
    { state: "unavailable", root: null, code: "denied", message: "Denied" },
    { ...branch, upstream: { state: "none" } },
    { ...branch, upstream: { state: "gone", name: "origin/main" } },
    { ...branch, upstream: { state: "gone", name: "origin/other" } },
    { ...branch, upstream: { state: "local", name: "origin/other" } },
    { ...branch, upstream: { state: "local", name: "main" } },
    { ...branch, upstream: { state: "unavailable", name: "main", code: "missing", message: "Missing" } },
    { ...branch, upstream: { state: "unavailable", name: "other", code: "missing", message: "Missing" } },
    { ...branch, upstream: { state: "unavailable", name: "other", code: "denied", message: "Missing" } },
    { ...branch, upstream: { state: "unavailable", name: "other", code: "denied", message: "Denied" } },
  ];
  const next: SpaceGitStatus = { ...base, space_id: "w2" };
  const changes: SpaceGitStatus[][] = [
    ...checkouts.map(checkout => [{ ...base, checkout }]),
    [{ ...base, source: "pane_folder" }],
    [next], [base, next], [next, base], [base], [],
  ];
  let response = [base];
  const spaceGitStatus = vi.fn(async () => ({ session_id: "session", spaces: structuredClone(response) }));
  const client = { spaceGitStatus } as unknown as CockpitClient;
  let current!: { spaces: ReadonlyMap<string, SpaceGitStatus>; error?: string; refresh(): void };
  let renders = 0;
  function Probe() { current = useSpaceGitStatus(client, "session", "w1"); ++renders; return null; }
  const root = createRoot(document.createElement("div"));
  const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  try {
    await act(async () => root.render(createElement(Probe)));
    for (response of changes) {
      const before = renders;
      await act(async () => current.refresh());
      expect([...current.spaces.values()]).toEqual(response);
      expect(renders).toBe(before + 1);
      const spaces = current.spaces;
      await act(async () => current.refresh());
      expect(renders).toBe(before + 1);
      expect(current.spaces).toBe(spaces);
    }
  } finally { await act(async () => root.unmount()); visibility.mockRestore(); }
});
