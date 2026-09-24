// @vitest-environment jsdom

import { act, createElement, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RepositoryListResponse, WorkspaceDefaults, WorkspaceOperation, WorkspaceSetupPlan, WorkspaceSetupRequest } from "../../protocol/generated/v1";
import { operationSnapshotIsNewer, operationStatusMessage, SetupDialog, type SetupClient } from "./SetupDialog";

function operation(generation: number, sequence: number): WorkspaceOperation {
  return { generation, sequence } as WorkspaceOperation;
}

const repository = { repository_id: "repository", name: "Repository", root: "/repositories/repository", checkout_path: "/repositories/repository", common_dir: "/repositories/repository/.git", branch: "main", is_linked_worktree: false, is_detached: false, provenance: "configured" };
const other = { ...repository, repository_id: "other", name: "Other", root: "/repositories/other", checkout_path: "/repositories/other", common_dir: "/repositories/other/.git" };
const repositories: RepositoryListResponse = { repositories: [repository, other], diagnostics: [] };
const client: SetupClient = {
  repositories: async () => repositories,
  sessionSnapshot: async () => { throw new Error("snapshot should not run"); },
  resolveWorkspaceDefaults: async () => { throw new Error("lookup should not run"); },
  planWorkspace: async () => { throw new Error("plan should not run"); },
  startWorkspace: async () => { throw new Error("start should not run"); },
  workspaceOperation: async () => { throw new Error("operation should not run"); },
  cancelWorkspace: async () => { throw new Error("cancel should not run"); },
  resumeWorkspace: async () => { throw new Error("resume should not run"); },
  reconcileWorkspace: async () => { throw new Error("reconcile should not run"); },
};

let root: Root | null = null;
let container: HTMLDivElement | null = null;
let emitTerminalUpdate: (() => void) | null = null;

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => {
  act(() => root?.unmount());
  root = null;
  container?.remove();
  container = null;
  emitTerminalUpdate = null;
  vi.useRealTimers();
});

async function settle(ms = 0): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
    await Promise.resolve();
    await Promise.resolve();
  });
}

function writeInput(input: HTMLInputElement, value: string): void {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function key(target: Element, name: string): void {
  target.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true }));
}

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  let reject: (reason: unknown) => void = () => undefined;
  const promise = new Promise<T>((next, fail) => { resolve = next; reject = fail; });
  return { promise, resolve, reject };
}

function sourceDefaults(branch: string, canonicalId: string, extra: Partial<WorkspaceDefaults> = {}): WorkspaceDefaults {
  const url = `https://gitlab.test/acme/${canonicalId.replace("!", "/-/merge_requests/")}`;
  return { artifact: { provider_id: "gitlab", kind: "review", canonical_id: canonicalId, original_url: url, canonical_url: url }, repositories: [repository], repository_id: repository.repository_id, branch, label: branch, checkout_path: null, title: "Fix login", linked_artifacts: [], ...extra };
}

function openPlan(): WorkspaceSetupPlan {
  return { operation_id: "operation-1", generation: 1, endpoint_identity: "endpoint-1", session_id: "session-1", repository: null, mode: "open", ownership: "borrowed_directory", branch: null, base: null, checkout_path: "/tmp/borrowed", companion_path: "/companions/operation-1", companion_id: "operation-1", companion_created_by_operation: true, label: "borrowed", focus: true, artifact: null, linked_artifacts: [], effects: [], warnings: [] };
}

function createPlan(overrides: Partial<WorkspaceSetupPlan> = {}): WorkspaceSetupPlan {
  return { ...openPlan(), repository, mode: "create", ownership: "owned_worktree", branch: "task", checkout_path: "/worktrees/task", ...overrides };
}

/** Plans echo the request, one fresh operation per call. */
function planner() {
  let count = 0;
  return vi.fn(async (_session: string, request: WorkspaceSetupRequest) => {
    count += 1;
    if (request.operation === "open") return { ...openPlan(), operation_id: `operation-${count}`, checkout_path: request.path };
    const chosen = request.repository_id === other.repository_id ? other : repository;
    return createPlan({ operation_id: `operation-${count}`, repository: chosen, branch: request.branch ?? "task", artifact: request.artifact_url ? sourceDefaults("task", "acme/app!7").artifact : null });
  });
}

function workspaceOperation(plan: WorkspaceSetupPlan, state: WorkspaceOperation["state"]): WorkspaceOperation {
  return {
    operation_id: plan.operation_id,
    generation: plan.generation,
    sequence: 1,
    session_id: plan.session_id,
    plan,
    state,
    step: state === "completed" ? "completed" : "herdr_requested",
    workspace_id: "workspace-1",
    tab_id: null,
    pane_id: null,
    companion_id: plan.companion_id,
    owned_resources: [],
    error: null,
    resume_allowed: false,
    cancel_requested: false,
    updated_at: "2026-09-12T00:00:00Z",
  };
}

function Harness({ dialogClient, onCompleted = () => undefined, parent = true }: { dialogClient: SetupClient; onCompleted?: (operation: WorkspaceOperation) => void; parent?: boolean }) {
  const [open, setOpen] = useState(true);
  const [, setTerminalRevision] = useState(1);
  emitTerminalUpdate = () => setTerminalRevision((value) => value + 1);
  return createElement("div", undefined,
    createElement("button", { type: "button", onClick: () => setOpen(true) }, "Reopen setup"),
    createElement(SetupDialog, { client: dialogClient, sessionId: "session-1", open, selectedParent: parent ? { label: "Repository", repositoryKey: repository.common_dir, checkoutPath: repository.checkout_path } : null, onClose: () => setOpen(false), onCompleted }),
  );
}

async function renderDialog(dialogClient: SetupClient = client, options: { onCompleted?: (operation: WorkspaceOperation) => void; parent?: boolean } = {}): Promise<void> {
  container = document.createElement("div");
  document.body.append(container);
  await act(async () => {
    root = createRoot(container!);
    root.render(createElement(Harness, { dialogClient, ...options }));
  });
  await settle();
}

const $ = <T extends Element = HTMLInputElement>(selector: string) => container!.querySelector<T>(selector as never) as T | null;
const button = (label: string) => [...container!.querySelectorAll<HTMLButtonElement>("button")].find((candidate) => candidate.textContent === label);

describe("SetupDialog", () => {
  it("accepts a newer generation and sequence, but rejects stale or duplicate snapshots", () => {
    const current = operation(4, 12);
    expect(operationSnapshotIsNewer(current, operation(4, 13))).toBe(true);
    expect(operationSnapshotIsNewer(current, operation(5, 0))).toBe(true);
    expect(operationSnapshotIsNewer(current, operation(4, 12))).toBe(false);
    expect(operationSnapshotIsNewer(current, operation(3, 99))).toBe(false);
  });

  it("names the current step and source retry in plain words", () => {
    expect(operationStatusMessage({ state: "running", step: "herdr_requested", error: null })).toBe("Creating the worktree and Space…");
    expect(operationStatusMessage({ state: "running", step: "context_preparing", error: null })).toBe("Preparing Context…");
    expect(operationStatusMessage({ state: "partial", step: "context_preparing", error: { code: "source_provider_unsupported", message: "provider is unavailable" } })).toContain("Retry the source step");
  });

  it("starts with the link focused and the parent Space's repository chosen", async () => {
    await renderDialog({ ...client, planWorkspace: planner() });
    expect(document.activeElement).toBe($("#setup-artifact-url"));
    expect($("#setup-repository")!.value).toBe("Repository");
    expect($("#setup-repository-results")).toBeNull();
  });

  it("keeps the typed branch focused across unrelated rerenders", async () => {
    await renderDialog({ ...client, planWorkspace: planner() });
    const branch = $("#setup-branch")!;
    branch.focus();
    act(() => emitTerminalUpdate?.());
    expect(document.activeElement).toBe(branch);
    await act(async () => writeInput(branch, "retain-focus"));
    await settle(400);
    expect(document.activeElement).toBe(branch);
    expect(branch.value).toBe("retain-focus");
  });

  it("picks the best fuzzy match on Enter, shows it and closes the list", async () => {
    await renderDialog({ ...client, planWorkspace: planner() }, { parent: false });
    const input = $("#setup-repository")!;
    act(() => input.focus());
    await act(async () => writeInput(input, "oth"));
    expect([...container!.querySelectorAll("#setup-repository-results mark")].map((mark) => mark.textContent).join("")).toBe("Oth");
    await act(async () => key(input, "Enter"));
    expect($("#setup-repository-results")).toBeNull();
    expect(input.value).toBe("Other");
  });

  it("chooses a highlighted match with the arrows and closes the list on Escape without closing setup", async () => {
    await renderDialog({ ...client, planWorkspace: planner() });
    const input = $("#setup-repository")!;
    act(() => input.focus());
    await act(async () => key(input, "ArrowDown"));
    expect($("#setup-repository-results")).not.toBeNull();
    await act(async () => key(input, "ArrowDown"));
    await act(async () => key(input, "Enter"));
    expect(input.value).toBe("Other");
    await act(async () => writeInput(input, "rep"));
    await act(async () => key(input, "Escape"));
    expect($("#setup-repository-results")).toBeNull();
    expect(input.value).toBe("Other");
    expect($("[role='dialog']")).not.toBeNull();
  });

  it("prepares the setup while typing and starts exactly that plan with one click", async () => {
    const planWorkspace = planner();
    const startWorkspace = vi.fn(async (_session: string, request: { operation_id: string }) => workspaceOperation(createPlan({ operation_id: request.operation_id }), "running"));
    await renderDialog({ ...client, planWorkspace, startWorkspace, workspaceOperation: vi.fn(() => new Promise<WorkspaceOperation>(() => undefined)) });
    await act(async () => writeInput($("#setup-branch")!, "feature"));
    await settle(400);
    expect(planWorkspace).toHaveBeenCalledTimes(1);
    expect(container!.textContent).toContain("New worktree feature");
    await act(async () => button("Create Space")!.click());
    await settle();
    expect(startWorkspace).toHaveBeenCalledTimes(1);
    expect(startWorkspace).toHaveBeenCalledWith("session-1", { operation_id: "operation-1", expected_generation: 1 });
    expect(planWorkspace).toHaveBeenCalledTimes(1);
    expect($("#setup-branch")!.disabled).toBe(true);
  });

  it("waits for the plan of the latest input when Enter comes before it", async () => {
    const planWorkspace = planner();
    const startWorkspace = vi.fn(async (_session: string, request: { operation_id: string }) => workspaceOperation(createPlan({ operation_id: request.operation_id }), "running"));
    await renderDialog({ ...client, planWorkspace, startWorkspace, workspaceOperation: vi.fn(() => new Promise<WorkspaceOperation>(() => undefined)) });
    await settle(400);
    const branch = $("#setup-branch")!;
    await act(async () => writeInput(branch, "changed"));
    await act(async () => key(branch, "Enter"));
    expect(startWorkspace).not.toHaveBeenCalled();
    await settle(400);
    expect(startWorkspace).toHaveBeenCalledTimes(1);
    const started = startWorkspace.mock.calls[0][1].operation_id;
    const lastPlan = await planWorkspace.mock.results.at(-1)!.value;
    expect(started).toBe(lastPlan.operation_id);
    expect(planWorkspace.mock.calls.at(-1)![1]).toMatchObject({ branch: "changed" });
  });

  it("ignores a late link lookup and preserves manual branch edits", async () => {
    const first = deferred<WorkspaceDefaults>();
    const second = deferred<WorkspaceDefaults>();
    const resolveWorkspaceDefaults = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    await renderDialog({ ...client, resolveWorkspaceDefaults, planWorkspace: planner() });
    const source = $("#setup-artifact-url")!;
    const branch = $("#setup-branch")!;
    await act(async () => writeInput(source, "https://gitlab.test/acme/app/-/merge_requests/4"));
    await settle(300);
    await act(async () => {
      writeInput(branch, "manual-branch");
      writeInput(source, "https://gitlab.test/acme/app/-/merge_requests/5");
    });
    await settle(300);
    await act(async () => { second.resolve(sourceDefaults("new-branch", "acme/app!5")); await Promise.resolve(); });
    expect(branch.value).toBe("manual-branch");
    await act(async () => { first.resolve(sourceDefaults("old-branch", "acme/app!4")); await Promise.resolve(); });
    expect(branch.value).toBe("manual-branch");
    expect(container!.textContent).toContain("MR !5 · Fix login");
  });

  it("imports a linked work item unless it is unticked", async () => {
    const linked = { artifact: { provider_id: "jira", kind: "issue", canonical_id: "SCRUM-5", original_url: "https://jira.test/browse/SCRUM-5", canonical_url: "https://jira.test/browse/SCRUM-5" }, title: "Login times out", error: null };
    const planWorkspace = planner();
    await renderDialog({ ...client, planWorkspace, resolveWorkspaceDefaults: async () => sourceDefaults("fix", "acme/app!7", { linked_artifacts: [linked] }) });
    await act(async () => writeInput($("#setup-artifact-url")!, "https://gitlab.test/acme/app/-/merge_requests/7"));
    await settle(300);
    await settle(400);
    expect(container!.textContent).toContain("Also import SCRUM-5 · Login times out");
    expect(planWorkspace.mock.calls.at(-1)![1]).toMatchObject({ artifact_url: "https://gitlab.test/acme/app/-/merge_requests/7", linked_artifact_urls: ["https://jira.test/browse/SCRUM-5"], branch: "fix" });
    await act(async () => container!.querySelector<HTMLInputElement>(".task-setup-linked input")!.click());
    await settle(400);
    expect(planWorkspace.mock.calls.at(-1)![1]).toMatchObject({ linked_artifact_urls: [] });
  });

  it("opens an existing folder with only a path and name", async () => {
    const planWorkspace = planner();
    await renderDialog({ ...client, planWorkspace });
    await act(async () => button("Open an existing folder instead")!.click());
    await settle();
    expect($("#setup-repository")).toBeNull();
    expect($("#setup-artifact-url")).toBeNull();
    expect($("#setup-branch")).toBeNull();
    await act(async () => writeInput($("#setup-checkout")!, "/tmp/borrowed"));
    await settle(400);
    expect(planWorkspace.mock.calls.at(-1)![1]).toEqual({ operation: "open", path: "/tmp/borrowed", label: "borrowed", task_name: null, focus: true });
    expect(button("Open Space")).toBeDefined();
    expect(container!.textContent).toContain("as it is; Cockpit never deletes it");
  });

  it("asks for a fresh confirmation when the backend says the plan went stale", async () => {
    const failure = Object.assign(new Error("The reviewed endpoint changed"), { code: "http_error", operationCode: "stale_plan" });
    const planWorkspace = planner();
    const startWorkspace = vi.fn().mockRejectedValueOnce(failure).mockImplementation(async (_session: string, request: { operation_id: string }) => workspaceOperation(createPlan({ operation_id: request.operation_id }), "running"));
    await renderDialog({ ...client, planWorkspace, startWorkspace, workspaceOperation: vi.fn(() => new Promise<WorkspaceOperation>(() => undefined)) });
    await settle(400);
    await act(async () => button("Create Space")!.click());
    await settle(400);
    expect(container!.querySelector("[role='alert']")?.textContent).toContain(failure.message);
    expect($("#setup-branch")!.disabled).toBe(false);
    expect(planWorkspace).toHaveBeenCalledTimes(2);
    expect(startWorkspace).toHaveBeenCalledTimes(1);
    await act(async () => button("Create Space")!.click());
    await settle();
    expect(startWorkspace).toHaveBeenCalledTimes(2);
    expect(startWorkspace.mock.calls[1][1]).toEqual({ operation_id: "operation-2", expected_generation: 1 });
  });

  it("keeps a started setup with an unknown outcome across close and reopen", async () => {
    const planWorkspace = planner();
    const startWorkspace = vi.fn(async () => { throw new Error("connection lost"); });
    const readOperation = vi.fn()
      .mockRejectedValueOnce(new Error("status unavailable"))
      .mockResolvedValueOnce(workspaceOperation(createPlan({ operation_id: "operation-1" }), "outcome_unknown"));
    await renderDialog({ ...client, planWorkspace, startWorkspace, workspaceOperation: readOperation });
    await settle(400);
    await act(async () => button("Create Space")!.click());
    await settle();
    expect($("#setup-branch")!.disabled).toBe(true);
    expect(button("Check setup")).toBeDefined();

    act(() => $<HTMLButtonElement>("[aria-label='Close setup dialog']")!.click());
    act(() => button("Reopen setup")!.click());
    await settle(400);
    expect(planWorkspace).toHaveBeenCalledTimes(1);
    await act(async () => button("Check setup")!.click());
    await settle();
    expect(planWorkspace).toHaveBeenCalledTimes(1);
    expect(startWorkspace).toHaveBeenCalledTimes(1);
    expect(button("Recover existing checkout")).toBeDefined();
  });

  it("closes itself once the Space is ready", async () => {
    const planWorkspace = planner();
    const onCompleted = vi.fn();
    const startWorkspace = vi.fn(async (_session: string, request: { operation_id: string }) => workspaceOperation(createPlan({ operation_id: request.operation_id }), "completed"));
    await renderDialog({ ...client, planWorkspace, startWorkspace }, { onCompleted });
    await settle(400);
    await act(async () => button("Create Space")!.click());
    await settle();
    expect(onCompleted).toHaveBeenCalledTimes(1);
    expect($("[role='dialog']")).toBeNull();
  });
});
