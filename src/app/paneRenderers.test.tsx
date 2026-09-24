// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { CockpitClientError, type CockpitClient } from "../client/CockpitClient";
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
      <button type="button" onClick={() => renderers.updateView("pane", "binding", { rootId: "root", path: "notes.md", files: {}, commentEditor: null, review: null })}>view</button>
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

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const reviewWithRoot = {
  ...presentation,
  can_open_review: true,
  default_root_id: "repository",
  roots: [{
    root_id: "repository",
    kind: "repository",
    label: "cockpit",
    path: "/repo",
    repository_id: "repository",
    checkout_path: "/repo",
    companion_id: null,
  }],
} as PanePresentation;
it.each([
  { source: "terminal", extension: null, renderer: null, authorized: "child" },
  { source: "verified Context", extension: "context", renderer: "context", authorized: "parent" },
] as const)("opens Review for the authorized $source checkout rather than an unrelated root", async ({ extension, renderer, authorized }) => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const roots = [
    { ...reviewWithRoot.roots[0], root_id: "parent", repository_id: "parent" },
    { ...reviewWithRoot.roots[0], root_id: "child", path: "/worktrees/child", checkout_path: "/worktrees/child", repository_id: "child" },
    { ...reviewWithRoot.roots[0], root_id: "companion", kind: "companion" as const, path: "/companions/task", checkout_path: "/worktrees/child", repository_id: "parent" },
  ];
  const linked = { ...reviewWithRoot, extension, renderer, default_root_id: "companion", roots } as PanePresentation;
  const inspectPane = vi.fn().mockResolvedValue(structuredClone(linked));
  const openReview = vi.fn<CockpitClient["openReview"]>().mockImplementation(async (_session, request) => {
    if (request.repository_id !== authorized) throw new CockpitClientError("native_error", "selected repository is not the source pane's current checkout", { operationCode: "context_root_not_authorized" });
    return linked;
  });
  const onResync = vi.fn();
  const client = { inspectPane, openReview } as unknown as CockpitClient;
  function Probe() {
    const renderers = usePaneRenderers(client, "session", ["pane"], ["pane"], true, 0, onResync);
    return <div><button type="button" onClick={() => { void renderers.open("pane", "right", "review"); }}>Open Review</button><span role="alert">{renderers.panes.pane?.actionError}</span></div>;
  }
  try {
    await act(async () => { mounted.render(<Probe />); await settle(); });
    await act(async () => { host.querySelector<HTMLButtonElement>("button")!.click(); await settle(); });
    expect(host.querySelector("[role=alert]")?.textContent).toBe("");
    expect(onResync).toHaveBeenCalledOnce();
    expect(openReview).toHaveBeenCalledOnce();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it.each(["request_outcome_unknown", "mutation_applied_snapshot_failed"] as const)(
  "refreshes authoritative renderer state after %s without relaunching",
  async (operationCode) => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    const host = document.createElement("div");
    document.body.append(host);
    const mounted = createRoot(host);
    const refreshed = {
      ...reviewWithRoot,
      renderer: "context",
      extension: "context",
      reason: "Context is ready",
      can_open_context: true,
      can_open_review: false,
    } as PanePresentation;
    const inspectPane = vi.fn()
      .mockResolvedValueOnce(structuredClone(reviewWithRoot))
      .mockResolvedValueOnce(structuredClone(reviewWithRoot))
      .mockResolvedValueOnce(structuredClone(refreshed));
    const openReview = vi.fn().mockRejectedValue(
      new CockpitClientError("native_error", "launch confirmation failed", { operationCode }),
    );
    const onResync = vi.fn();
    const client = { inspectPane, openReview } as unknown as CockpitClient;
    function Probe() {
      const renderers = usePaneRenderers(client, "session", ["pane"], ["pane"], true, 0, onResync);
      const pane = renderers.panes.pane;
      return <div>
        <span data-testid="renderer">{pane?.presentation.renderer ?? "none"}</span>
        <span data-testid="error">{pane?.actionError ?? ""}</span>
        <span data-testid="unknown">{pane?.outcomeUnknown ? "unknown" : "known"}</span>
        <button type="button" onClick={() => { void renderers.open("pane", "right", "review"); }}>open</button>
      </div>;
    }
    try {
      await act(async () => { mounted.render(<Probe />); await settle(); });
      await act(async () => { host.querySelector<HTMLButtonElement>("button")?.click(); await settle(); });

      expect(openReview).toHaveBeenCalledOnce();
      expect(onResync).toHaveBeenCalledOnce();
      expect(host.querySelector("[data-testid=renderer]")?.textContent).toBe("context");
      expect(host.querySelector("[data-testid=error]")?.textContent).toBe("launch confirmation failed");
      expect(host.querySelector("[data-testid=unknown]")?.textContent).toBe("unknown");
    } finally {
      await act(async () => mounted.unmount());
      host.remove();
    }
  },
);

it("resyncs after a successful launch and reflects the refreshed renderer", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const refreshed = {
    ...reviewWithRoot,
    renderer: "context",
    extension: "context",
    reason: "Context is ready",
    can_open_context: true,
    can_open_review: false,
  } as PanePresentation;
  const inspectPane = vi.fn()
    .mockResolvedValueOnce(structuredClone(reviewWithRoot))
    .mockResolvedValueOnce(structuredClone(reviewWithRoot))
    .mockResolvedValueOnce(structuredClone(refreshed));
  const openReview = vi.fn().mockResolvedValue(undefined);
  const onResync = vi.fn();
  const client = { inspectPane, openReview } as unknown as CockpitClient;
  function Probe() {
    const renderers = usePaneRenderers(client, "session", ["pane"], ["pane"], true, 0, onResync);
    const pane = renderers.panes.pane;
    return <div>
      <span data-testid="renderer">{pane?.presentation.renderer ?? "none"}</span>
      <span data-testid="error">{pane?.actionError ?? ""}</span>
      <button type="button" onClick={() => { void renderers.open("pane", "right", "review"); }}>open</button>
    </div>;
  }
  try {
    await act(async () => { mounted.render(<Probe />); await settle(); });
    await act(async () => { host.querySelector<HTMLButtonElement>("button")?.click(); await settle(); });

    expect(openReview).toHaveBeenCalledOnce();
    expect(onResync).toHaveBeenCalledOnce();
    expect(host.querySelector("[data-testid=renderer]")?.textContent).toBe("context");
    expect(host.querySelector("[data-testid=error]")?.textContent).toBe("");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("ignores an obsolete uncertain launch completion without refreshing the current session", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const outcome = deferred<void>();
  const sessionA = { ...structuredClone(reviewWithRoot), session_id: "session-a" };
  const sessionB = { ...structuredClone(reviewWithRoot), session_id: "session-b", binding_id: "binding-b" };
  const inspectPane = vi.fn()
    .mockResolvedValueOnce(sessionA)
    .mockResolvedValueOnce(sessionA)
    .mockResolvedValue(sessionB);
  const openReview = vi.fn().mockImplementation(() => outcome.promise);
  const onResync = vi.fn();
  const client = { inspectPane, openReview } as unknown as CockpitClient;
  function Probe({ sessionId }: { sessionId: string }) {
    const renderers = usePaneRenderers(client, sessionId, ["pane"], ["pane"], true, 0, onResync);
    const pane = renderers.panes.pane;
    return <div>
      <span data-testid="session">{pane?.presentation.session_id ?? "none"}</span>
      <button type="button" onClick={() => { void renderers.open("pane", "right", "review"); }}>open</button>
    </div>;
  }
  try {
    await act(async () => { mounted.render(<Probe sessionId="session-a" />); await settle(); });
    await act(async () => { host.querySelector<HTMLButtonElement>("button")?.click(); await settle(); });
    expect(openReview).toHaveBeenCalledOnce();

    await act(async () => { mounted.render(<Probe sessionId="session-b" />); await settle(); });
    outcome.reject(new CockpitClientError("native_error", "launch confirmation failed", { operationCode: "request_outcome_unknown" }));
    await act(async () => { await settle(); });

    expect(host.querySelector("[data-testid=session]")?.textContent).toBe("session-b");
    expect(onResync).not.toHaveBeenCalled();
    expect(inspectPane.mock.calls.filter(([sessionId]) => sessionId === "session-b")).toHaveLength(1);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});
