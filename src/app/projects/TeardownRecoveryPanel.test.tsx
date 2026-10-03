// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  WorkspaceTeardownPreview,
  WorkspaceTeardownResult,
} from "../../protocol/generated/v1";
import { TeardownRecoveryPanel } from "./TeardownRecoveryPanel";

let container: HTMLDivElement;
let root: Root;

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function settle(): Promise<void> {
  await act(async () => { await Promise.resolve(); });
}

function preview(actions: WorkspaceTeardownPreview["allowed_actions"], confirmation: string | null): WorkspaceTeardownPreview {
  return {
    operation_id: "operation-1", workspace_id: "space-1", endpoint_identity: "endpoint-1",
    repository_key: "repo", repository_root: "/repo", checkout_path: "/worktrees/task",
    ownership: "owned_created", workspace_state: "missing",
    is_linked_worktree: true, dirty_state: "unknown",
    allowed_actions: actions, blockers: ["the exact worktree is not live in the requested workspace"],
    warnings: [], required_confirmation: confirmation,
  };
}

function result(action: WorkspaceTeardownResult["action"], outcome: WorkspaceTeardownResult["outcome"]): WorkspaceTeardownResult {
  return { operation_id: "operation-1", workspace_id: "space-1", action, outcome, message: outcome };
}

describe("TeardownRecoveryPanel", () => {
  it("reopens an absent-Space record and reconciles an unknown removal without additional cleanup", async () => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    const workspaceTeardownPreview = vi.fn()
      .mockResolvedValueOnce(preview(["reconcile_remove_outcome"], null));
    const workspaceTeardownExecute = vi.fn()
      .mockResolvedValueOnce(result("reconcile_remove_outcome", "completed"));
    const client = {
      workspaceTeardownRecoveries: vi.fn(async () => ({
        recoveries: [{ operation_id: "operation-1", workspace_id: "space-1", checkout_path: "/worktrees/task", state: "outcome_unknown" as const }],
      })),
      workspaceTeardownPreview,
      workspaceTeardownExecute,
    };
    act(() => {
      root.render(<TeardownRecoveryPanel client={client} sessionId="session-1" open onClose={vi.fn()} />);
    });
    await settle();
    expect(container.textContent).toContain("/worktrees/task");
    const review = [...container.querySelectorAll("button")].find((button) => button.textContent === "Review recovery")!;
    act(() => review.click());
    await settle();

    let buttons = [...container.querySelectorAll("button")];
    act(() => buttons.find((button) => button.textContent === "Reconcile removal")!.click());
    buttons = [...container.querySelectorAll("button")];
    act(() => buttons.filter((button) => button.textContent === "Reconcile removal")[1]!.click());
    await settle();
    await settle();

    expect(workspaceTeardownExecute).toHaveBeenNthCalledWith(1, "session-1", expect.objectContaining({ action: "reconcile_remove_outcome" }));
    expect(workspaceTeardownExecute).toHaveBeenCalledTimes(1);
    expect(container.querySelector("#teardown-confirmation")).toBeNull();
    expect(container.textContent).toContain("completed");
  });
});
