// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../client/CockpitClient";
import type { PanePresentation } from "../protocol/generated/v1";
import { isGraphicalReview, usePaneRenderers } from "./paneRenderers";

const presentation = {
  session_id: "session",
  pane_id: "pane",
  terminal_id: "terminal",
  binding_id: "binding",
  extension: "review",
  renderer: "review",
  confidence: "verified_launch",
  reason: "Review is ready",
  roots: [],
  default_root_id: null,
  can_open_files: false,
  files_root_id: null,
  can_open_context: false,
  can_open_review: false,
  diagnostics: [],
} as PanePresentation;

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
}

it("retains renderer choice and view for a stable binding across polls, refresh, and stale/live recovery", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const inspectPane = vi.fn().mockResolvedValue(structuredClone(presentation));
  const client = { inspectPane } as unknown as CockpitClient;
  function Probe({ sessionId = "session", live = true }: { sessionId?: string; live?: boolean }) {
    const renderers = usePaneRenderers(client, sessionId, ["pane"], ["pane"], live, 0, vi.fn());
    const pane = renderers.panes.pane;
    return <div>
      <span data-testid="renderer">{isGraphicalReview(pane) ? "review" : "terminal"}</span>
      <span data-testid="view">{pane?.view.path ?? ""}</span>
      <button type="button" onClick={() => renderers.choose("pane", "terminal")}>terminal</button>
      <button type="button" onClick={() => renderers.updateView("pane", "binding", { rootId: "root", path: "notes.md", files: {}, commentEditor: null })}>view</button>
      <button type="button" onClick={renderers.refresh}>refresh</button>
    </div>;
  }
  const poll = async () => {
    await act(async () => { await vi.advanceTimersByTimeAsync(2_500); await settle(); });
  };
  try {
    await act(async () => { mounted.render(<Probe />); await settle(); });
    expect(host.querySelector("[data-testid=renderer]")?.textContent).toBe("review");
    act(() => host.querySelector<HTMLButtonElement>("button")?.click());
    act(() => host.querySelectorAll<HTMLButtonElement>("button")[1]?.click());
    expect(host.querySelector("[data-testid=renderer]")?.textContent).toBe("terminal");
    expect(host.querySelector("[data-testid=view]")?.textContent).toBe("notes.md");

    await poll();
    expect(host.querySelector("[data-testid=renderer]")?.textContent).toBe("terminal");
    expect(host.querySelector("[data-testid=view]")?.textContent).toBe("notes.md");
    await act(async () => { host.querySelectorAll<HTMLButtonElement>("button")[2]?.click(); await settle(); });
    expect(host.querySelector("[data-testid=renderer]")?.textContent).toBe("terminal");
    expect(host.querySelector("[data-testid=view]")?.textContent).toBe("notes.md");

    await act(async () => { mounted.render(<Probe live={false} />); await settle(); });
    await act(async () => { mounted.render(<Probe />); await settle(); });
    expect(host.querySelector("[data-testid=renderer]")?.textContent).toBe("terminal");
    expect(host.querySelector("[data-testid=view]")?.textContent).toBe("notes.md");

    inspectPane.mockResolvedValueOnce({ ...structuredClone(presentation), binding_id: "new-binding" });
    await act(async () => { host.querySelectorAll<HTMLButtonElement>("button")[2]?.click(); await settle(); });
    expect(host.querySelector("[data-testid=renderer]")?.textContent).toBe("review");
    expect(host.querySelector("[data-testid=view]")?.textContent).toBe("");

    inspectPane.mockResolvedValueOnce({ ...structuredClone(presentation), session_id: "other", binding_id: "other-binding" });
    await act(async () => { mounted.render(<Probe sessionId="other" />); await settle(); });
    expect(host.querySelector("[data-testid=renderer]")?.textContent).toBe("review");
    expect(host.querySelector("[data-testid=view]")?.textContent).toBe("");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
    vi.useRealTimers();
  }
});

it("retains the last usable presentation while inspection errors repeat", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const inspectPane = vi.fn()
    .mockResolvedValueOnce(structuredClone(presentation))
    .mockResolvedValueOnce({ ...structuredClone(presentation), reason: "Review changed" })
    .mockRejectedValue(new Error("Presentation unavailable"));
  const client = { inspectPane } as unknown as CockpitClient;
  function Probe() {
    const { panes } = usePaneRenderers(client, "session", ["pane"], ["pane"], true, 0, vi.fn());
    const pane = panes.pane;
    return <div><span data-testid="reason">{pane?.presentation.reason}</span><span data-testid="error">{pane?.inspectionError ?? ""}</span></div>;
  }
  const poll = async () => {
    await act(async () => { await vi.advanceTimersByTimeAsync(2_500); await settle(); });
  };
  try {
    await act(async () => { mounted.render(<Probe />); await settle(); });
    await poll();
    expect(host.querySelector("[data-testid=reason]")?.textContent).toBe("Review changed");
    await poll();
    expect(host.querySelector("[data-testid=reason]")?.textContent).toBe("Review changed");
    expect(host.querySelector("[data-testid=error]")?.textContent).toBe("Presentation unavailable");
    await poll();
    expect(host.querySelector("[data-testid=error]")?.textContent).toBe("Presentation unavailable");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
    vi.useRealTimers();
  }
});
