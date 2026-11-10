// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { InboxThread } from "../../protocol/generated/v1";
import { InboxWork } from "./InboxWork";

it("keeps set-aside tasks separate from done and retains their evidence", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const task = { root_id: "root", task_id: "task", title: "Task", project: null, lane: "ready", run: null, evidence: null, disposition: { kind: "abandoned", reason: "No longer needed", evidence: "Decision", at: "2026-10-07T12:00:00Z", by: { type: "operator" } } };
  const thread = { tasks: [task], timeline: [], questions: [] } as unknown as InboxThread;
  try {
    await act(async () => root.render(<InboxWork thread={thread} view={null} client={{} as CockpitClient} sessionId="session" open={vi.fn()} liveOpen={false} liveStatusFresh={false} diagnostics={null} />));
    expect(host.textContent).toContain("Set aside · abandoned");
    expect(host.textContent).toContain("No longer needed");
    expect(host.textContent).toContain("Decision");
    expect(host.textContent).toContain("does not stop workers");
    expect(host.querySelector('[data-lane="done"]')).toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
