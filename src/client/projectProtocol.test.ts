import { describe, expect, it } from "vitest";
import { parseWorkspaceOperation } from "./projectProtocol";

const repository = {
  repository_id: "repo-1",
  name: "repository",
  root: "/repositories/repository",
  checkout_path: "/repositories/repository",
  common_dir: "/repositories/repository/.git",
  branch: "main",
  is_linked_worktree: false,
  is_detached: false,
  provenance: "configured",
};

const plan = {
  operation_id: "operation-1",
  generation: 1,
  endpoint_identity: "endpoint-1",
  session_id: "session-1",
  repository,
  mode: "create",
  branch: "task",
  base: "main",
  checkout_path: "/worktrees/task",
  companion_path: "/companions/operation-1",
  companion_id: "operation-1",
  companion_created_by_operation: true,
  label: "Task",
  focus: true,
  trust_repository: true,
  artifact: null,
  effects: [],
  warnings: [],
};

function operation(step: "context_preparing" | "context_ready") {
  return {
    operation_id: "operation-1",
    generation: 1,
    sequence: 1,
    session_id: "session-1",
    plan,
    state: "running",
    step,
    workspace_id: "workspace-1",
    tab_id: null,
    pane_id: null,
    companion_id: "operation-1",
    owned_resources: [],
    error: null,
    resume_allowed: false,
    cancel_requested: false,
    updated_at: "2026-09-12T00:00:00Z",
  };
}

describe("workspace operation protocol", () => {
  it("accepts the generated Context preparation checkpoints", () => {
    expect(parseWorkspaceOperation(operation("context_preparing")).step).toBe("context_preparing");
    expect(parseWorkspaceOperation(operation("context_ready")).step).toBe("context_ready");
  });
});
