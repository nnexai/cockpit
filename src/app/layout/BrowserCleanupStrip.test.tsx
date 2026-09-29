// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { BrowserLegacyArchive } from "../../protocol/generated/v1";
import type { CockpitClient } from "../../client/CockpitClient";
import { BrowserCleanupStrip } from "./BrowserCleanupStrip";
import { createSessionLayoutState, layoutReducer, type LeafCtx } from "./tabLayoutStore";

vi.mock("../browser/SavedBrowserWork", () => ({ SavedBrowserWork: () => null }));

it("authorizes only the manifest the operator reviewed and reports replaced artifacts as preserved", async () => {
  Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
  const manifest: BrowserLegacyArchive["candidates"] = [
    { path: "/fixture/old-profile", kind: "directory", dev: "2", inode: "41", entry_count: 2, total_bytes: 500, captured_at: "2026-09-30T10:00:00Z", state: "pending" },
    { path: "/fixture/launch.json", kind: "file", dev: "2", inode: "42", entry_count: 1, total_bytes: 50, captured_at: "2026-09-30T10:00:00Z", state: "pending" },
  ];
  const archive: BrowserLegacyArchive = { association_key: "legacy", session_id: "old-session", space_id: "old-space", space_label: "Original", archived_at: "2026-09-30T10:00:00Z", session_stopped: true, saved_capture_count: 1, draft_count: 1, pending_capture: false, candidates: manifest, not_candidates: ["/fixture/unowned-symlink"] };
  const remove = vi.fn(async () => ({ archives: [{ ...archive, candidates: manifest.map((candidate, index) => ({ ...candidate, state: index === 0 ? "removed" as const : "changed" as const })) }] }));
  const client = { browserCleanupStatus: vi.fn(async () => ({ cutover: "done", failures: [], saved_tabs: [] })), browserLegacyList: vi.fn(async () => ({ archives: [archive] })), browserLegacyRemove: remove } as unknown as CockpitClient;
  let state = createSessionLayoutState("session", "server");
  const ctx: LeafCtx = { client, sessionId: "session", serverInstance: "server", clientId: "window", getState: () => state, dispatch: (action) => { state = layoutReducer(state, action).state; } };
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => { root.render(<BrowserCleanupStrip ctx={ctx} activeTabId={null} />); });
    expect(host.textContent).toContain("Saved before tabs: Space Original");
    expect(host.textContent).toContain("device 2, inode 42");
    expect(host.textContent).toContain("Cockpit cannot prove these items are unchanged");
    const authorize = [...host.querySelectorAll("button")].find((button) => button.textContent === "Remove these items")!;
    await act(async () => { authorize.click(); });
    expect(remove).toHaveBeenCalledExactlyOnceWith({ association_key: "legacy", candidates: manifest });
    expect(host.textContent).toContain("Changed and preserved");
    expect(host.textContent).toContain("/fixture/unowned-symlink");
  } finally { await act(async () => root.unmount()); host.remove(); }
});
