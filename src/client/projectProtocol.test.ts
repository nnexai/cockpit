import { describe, expect, it } from "vitest";
import { parseWorkspaceOperation, parseWorkspaceSetupRequest, parseWorkspaceDefaults } from "./projectProtocol";

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
  ownership: "owned_worktree",
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

it("opens a directory without repository fields and rejects worktree fields on that operation", () => {
  const request = { operation: "open", path: "/notes/nested", label: "Notes", task_name: null, focus: true };
  expect(parseWorkspaceSetupRequest(request)).toEqual(request);
  expect(() => parseWorkspaceSetupRequest({ ...request, repository_id: "repo", branch: "main" })).toThrow();
});

it("keeps unmatched provider defaults unselected and rejects a repository outside the returned matches", () => {
  const defaults = { artifact: { provider_id: "github", kind: "issue", canonical_id: "acme/repo#2", original_url: "https://github.com/acme/repo/issues/2", canonical_url: "https://github.com/acme/repo/issues/2" }, repositories: [], repository_id: null, branch: null, label: null, checkout_path: null };
  expect(parseWorkspaceDefaults(defaults)).toEqual(defaults);
  expect(() => parseWorkspaceDefaults({ ...defaults, repository_id: "unmatched" })).toThrow();
});
