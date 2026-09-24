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
  linked_artifacts: [],
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
  const defaults = { artifact: { provider_id: "github", kind: "issue", canonical_id: "acme/repo#2", original_url: "https://github.com/acme/repo/issues/2", canonical_url: "https://github.com/acme/repo/issues/2" }, repositories: [], repository_id: null, branch: null, label: null, checkout_path: null, title: null, linked_artifacts: [] };
  expect(parseWorkspaceDefaults(defaults)).toEqual(defaults);
  expect(() => parseWorkspaceDefaults({ ...defaults, repository_id: "unmatched" })).toThrow();
});

it("carries linked work items in defaults and worktree requests", () => {
  const jira = { provider_id: "jira", kind: "issue", canonical_id: "SCRUM-5", original_url: "https://jira.test/browse/SCRUM-5", canonical_url: "https://jira.test/browse/SCRUM-5" };
  const defaults = { artifact: jira, repositories: [], repository_id: null, branch: null, label: null, checkout_path: null, title: "Login", linked_artifacts: [{ artifact: jira, title: null, error: "not visible" }] };
  expect(parseWorkspaceDefaults(defaults)).toEqual(defaults);
  expect(() => parseWorkspaceDefaults({ ...defaults, linked_artifacts: [{ artifact: jira, title: 5, error: null }] })).toThrow();
  const request = { operation: "create", repository_id: "repo", branch: null, base_ref: null, checkout_path: null, label: null, task_name: null, artifact_url: "https://gitlab.test/a/b/-/merge_requests/1", linked_artifact_urls: [jira.canonical_url], focus: true };
  expect(parseWorkspaceSetupRequest(request)).toEqual(request);
  expect(() => parseWorkspaceSetupRequest({ ...request, linked_artifact_urls: Array(5).fill(jira.canonical_url) })).toThrow();
});
