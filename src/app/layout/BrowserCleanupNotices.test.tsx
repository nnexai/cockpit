// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { BrowserCleanupStatus } from "../../protocol/generated/v1";
import { BrowserCleanupNotices } from "./BrowserCleanupNotices";
import { createSessionLayoutState, layoutReducer, type LeafCtx } from "./tabLayoutStore";
import { browserOpenDisabledReason } from "./browserLifecycle";

function fixture(status: BrowserCleanupStatus) {
  let state = createSessionLayoutState("session", "server");
  const browserCleanupRetry = vi.fn(async () => ({ failures: [] }));
  const client = { browserCleanupStatus: vi.fn(async () => status), browserCleanupRetry } as unknown as CockpitClient;
  const ctx: LeafCtx = { client, sessionId: "session", serverInstance: "server", clientId: "window", getState: () => state, dispatch: (action) => { state = layoutReducer(state, action).state; } };
  return { ctx, browserCleanupRetry };
}


it("retries an incomplete tab cleanup and removes its transient notice on success", async () => {
  Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
  const f = fixture({ failures: [{ association_key: "browser:tab", scope: { kind: "tab", session_id: "session", tab_id: "tab" }, reason: "Profile is locked", unproven_paths: [] }] });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<BrowserCleanupNotices ctx={f.ctx} activeTabId="tab" />));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Profile is locked");
    expect(browserOpenDisabledReason(f.ctx, "tab")).toContain("cleanup");
    expect(browserOpenDisabledReason(f.ctx, "other-tab")).toBeNull();
    const retry = [...host.querySelectorAll("button")].find((button) => button.textContent === "Retry cleanup")!;
    await act(async () => retry.click());
    expect(f.browserCleanupRetry).toHaveBeenCalledExactlyOnceWith({ association_key: "browser:tab" });
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(browserOpenDisabledReason(f.ctx, "tab")).toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("dismisses a cleanup error without retrying or blocking another tab", async () => {
  Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
  const f = fixture({ failures: [{ association_key: "browser:tab", scope: { kind: "tab", session_id: "session", tab_id: "tab" }, reason: "Profile is locked", unproven_paths: [] }] });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<BrowserCleanupNotices ctx={f.ctx} activeTabId="tab" />));
    const dismiss = [...host.querySelectorAll("button")].find((button) => button.textContent === "Dismiss")!;
    await act(async () => dismiss.click());
    expect(f.browserCleanupRetry).not.toHaveBeenCalled();
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(browserOpenDisabledReason(f.ctx, "tab")).toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); }
});
