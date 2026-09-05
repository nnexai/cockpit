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
    ownership: "owned_created", workspace_state: "missing", companion_state: "owned",
    is_linked_worktree: true, dirty_state: "unknown", companion_path: "/companions/operation-1",
    allowed_actions: actions, blockers: ["the exact worktree is not live in the requested workspace"],
    warnings: [], required_confirmation: confirmation,
  };
}

function result(action: WorkspaceTeardownResult["action"], outcome: WorkspaceTeardownResult["outcome"]): WorkspaceTeardownResult {
  return { operation_id: "operation-1", workspace_id: "space-1", action, outcome, message: outcome };
}

describe("TeardownRecoveryPanel", () => {
  it("reopens an absent-Space record, reconciles an unknown removal, then explicitly cleans the orphan", async () => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    const workspaceTeardownPreview = vi.fn()
      .mockResolvedValueOnce(preview(["reconcile_remove_outcome"], null))
      .mockResolvedValueOnce(preview(["remove_orphaned_companion"], "REMOVE COMPANION"))
      .mockResolvedValueOnce(preview([], null));
    const workspaceTeardownExecute = vi.fn()
      .mockResolvedValueOnce(result("reconcile_remove_outcome", "orphaned_companion"))
      .mockResolvedValueOnce(result("remove_orphaned_companion", "completed"));
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

    buttons = [...container.querySelectorAll("button")];
    act(() => buttons.find((button) => button.textContent === "Remove orphaned companion")!.click());
    const confirmation = container.querySelector<HTMLInputElement>("#teardown-confirmation")!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(confirmation, "REMOVE COMPANION");
      confirmation.dispatchEvent(new Event("input", { bubbles: true }));
    });
    buttons = [...container.querySelectorAll("button")];
    act(() => buttons.filter((button) => button.textContent === "Remove orphaned companion")[1]!.click());
    await settle();

    expect(workspaceTeardownExecute).toHaveBeenNthCalledWith(1, "session-1", expect.objectContaining({ action: "reconcile_remove_outcome" }));
    expect(workspaceTeardownExecute).toHaveBeenNthCalledWith(2, "session-1", expect.objectContaining({ action: "remove_orphaned_companion", confirmation: "REMOVE COMPANION" }));
  });
});
