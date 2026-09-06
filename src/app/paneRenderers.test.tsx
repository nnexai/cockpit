// @vitest-environment jsdom
import { act, memo, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../client/CockpitClient";
import type { PanePresentation } from "../protocol/generated/v1";
import { usePaneRenderers } from "./paneRenderers";

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

it("keeps pane presentation state stable for cloned polls while applying real changes and errors", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const observed: Array<{ state: object; reason: string | undefined; error: string | null }> = [];
  const inspectPane = vi.fn()
    .mockResolvedValueOnce(structuredClone(presentation))
    .mockResolvedValueOnce(structuredClone(presentation))
    .mockResolvedValueOnce({ ...structuredClone(presentation), reason: "Review changed" })
    .mockRejectedValueOnce(new Error("Presentation unavailable"))
    .mockRejectedValueOnce(new Error("Presentation unavailable"));
  const client = { inspectPane } as unknown as CockpitClient;
  const resync = vi.fn();
  let latestState: object | null = null;

  const CommittedPresentation = memo(function CommittedPresentation({ panes }: { panes: Record<string, { presentation: PanePresentation; inspectionError: string | null }> }) {
    useEffect(() => {
      observed.push({ state: panes, reason: panes.pane?.presentation.reason, error: panes.pane?.inspectionError ?? null });
    }, [panes]);
    return null;
  });

  function Probe() {
    const { panes } = usePaneRenderers(client, "session", ["pane"], ["pane"], true, 0, resync);
    latestState = panes;
    return <CommittedPresentation panes={panes} />;
  }

  const poll = async () => {
    await act(async () => { await vi.advanceTimersByTimeAsync(2_500); await settle(); });
  };
  try {
    await act(async () => { mounted.render(<Probe />); await settle(); });
    expect(observed).toHaveLength(2);
    const initialPaneState = observed.at(-1)!.state;
    await poll();
    expect(observed).toHaveLength(2);
    await act(async () => { mounted.render(<Probe />); });
    expect(latestState).toBe(initialPaneState);
    expect(observed).toHaveLength(2);
    await poll();
    expect(observed).toHaveLength(3);
    expect(observed.at(-1)).toMatchObject({ reason: "Review changed", error: null });
    await poll();
    expect(observed).toHaveLength(4);
    expect(observed.at(-1)).toMatchObject({ reason: "Review changed", error: "Presentation unavailable" });
    await poll();
    expect(observed).toHaveLength(4);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
    vi.useRealTimers();
  }
});
