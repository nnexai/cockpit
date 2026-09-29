// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { BrowserCleanupStatus } from "../../protocol/generated/v1";
import { BrowserCleanupStrip } from "./BrowserCleanupStrip";
import { createSessionLayoutState, layoutReducer, type LeafCtx } from "./tabLayoutStore";

vi.mock("../browser/SavedBrowserWork", () => ({ SavedBrowserWork: () => null }));

it("keeps saved browser work discoverable under its original source after the tab is gone", async () => {
  Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
  const status: BrowserCleanupStatus = { cutover: "done", failures: [], saved_tabs: [{ association_key: "old-browser", session_id: "original-session", tab_id: "retired-tab", tab_label: "Original tab", space_id: "retired-space", space_label: "Original Space", saved_capture_count: 2, draft_count: 1, pending_capture: true }] };
  const client = { browserCleanupStatus: vi.fn(async () => status), browserLegacyList: vi.fn(async () => ({ archives: [] })) } as unknown as CockpitClient;
  let state = createSessionLayoutState("replacement-session", "replacement-server");
  const ctx: LeafCtx = { client, sessionId: "replacement-session", serverInstance: "replacement-server", clientId: "window", getState: () => state, dispatch: (action) => { state = layoutReducer(state, action).state; } };
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => { root.render(<BrowserCleanupStrip ctx={ctx} activeTabId={null} />); });
    expect(host.textContent).toContain("Saved browser work: Tab Original tab");
    expect(host.textContent).toContain("Original tab retired-tab");
    expect(host.textContent).toContain("Original Space (retired-space)");
    expect(host.textContent).toContain("Session original-session");
    expect(host.textContent).toContain("2 saved captures · 1 drafts · capture pending");
    expect(state.tabs).toEqual({});
  } finally { await act(async () => root.unmount()); host.remove(); }
});
