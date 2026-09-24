// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { PaneSummary, SpaceSummary } from "../../protocol/generated/v1";
import { aheadBehindLabel, spaceCheckoutKey, useSpaceGitStatus } from "./spaceGitStatus";

afterEach(() => { vi.useRealTimers(); });

it("formats Herdr's compact upstream position", () => {
  expect(aheadBehindLabel({ space_id: "w1", branch: "main", upstream: "origin/main", ahead: 14, behind: 0 })).toBe("↑14");
  expect(aheadBehindLabel({ space_id: "w1", branch: "main", upstream: "origin/main", ahead: 2, behind: 1 })).toBe("↑2 ↓1");
  expect(aheadBehindLabel({ space_id: "w1", branch: "main", upstream: null, ahead: null, behind: null })).toBe("");
  expect(aheadBehindLabel(undefined)).toBe("");
});

it("polls while visible and skips polls while the page is hidden", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  const spaceGitStatus = vi.fn(async (sessionId: string) => ({ session_id: sessionId, spaces: [{ space_id: "w1", branch: "main", upstream: "origin/main", ahead: 1, behind: 0 }] }));
  const client = { spaceGitStatus } as unknown as CockpitClient;
  const seen: string[] = [];
  function Probe() {
    const status = useSpaceGitStatus(client, "session", "w1", 1000);
    seen.push(aheadBehindLabel(status.get("w1")));
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
