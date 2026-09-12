// @vitest-environment jsdom

import { act, createElement, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProjectConfiguration, RepositoryListResponse, WorkspaceDefaults, WorkspaceOperation, WorkspaceSetupPlan } from "../../protocol/generated/v1";
import { operationSnapshotIsNewer, operationStatusMessage, SetupDialog, type SetupClient } from "./SetupDialog";

function operation(generation: number, sequence: number): WorkspaceOperation {
  return { generation, sequence } as WorkspaceOperation;
}

const configuration: ProjectConfiguration = {
  version: 1,
  repository_roots: ["/repositories"],
  worktree_root: "/worktrees",
  companion_root: "/companions",
  state_root: "/state",
  branch_template: "{task}",
  checkout_template: "{task}",
  providers: [],
  limits: { catalog_depth: 4, catalog_entries: 100, git_timeout_ms: 1_000, git_output_bytes: 1_000, operation_timeout_ms: 1_000, context_preview_bytes: 1_000, context_preview_lines: 100, context_directory_entries: 100, context_tree_depth: 4 },
  origins: {},
};

const repository = { repository_id: "repository", name: "Repository", root: "/repositories/repository", checkout_path: "/repositories/repository", common_dir: "/repositories/repository/.git", branch: "main", is_linked_worktree: false, is_detached: false, provenance: "configured" };
const repositories: RepositoryListResponse = { repositories: [repository], diagnostics: [] };
const client: SetupClient = {
  projectConfiguration: async () => configuration,
  repositories: async () => repositories,
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

afterEach(() => {
  act(() => root?.unmount());
  root = null;
  container?.remove();
  container = null;
  emitTerminalUpdate = null;
  vi.useRealTimers();
});

async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function writeInput(input: HTMLInputElement, value: string): void {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  const promise = new Promise<T>((next) => { resolve = next; });
  return { promise, resolve };
}

function sourceDefaults(branch: string, canonicalId: string): WorkspaceDefaults {
  return { artifact: { provider_id: "github", kind: "issue", canonical_id: canonicalId, original_url: `https://github.com/nnexai/${canonicalId.replace("#", "/issues/")}`, canonical_url: `https://github.com/nnexai/${canonicalId.replace("#", "/issues/")}` }, repositories: [repository], repository_id: repository.repository_id, branch, label: branch, checkout_path: null };
}

function openPlan(): WorkspaceSetupPlan {
  return { operation_id: "operation-1", generation: 1, endpoint_identity: "endpoint-1", session_id: "session-1", repository: null, mode: "open", ownership: "borrowed_directory", branch: null, base: null, checkout_path: "/tmp/borrowed", companion_path: "/companions/operation-1", companion_id: "operation-1", companion_created_by_operation: true, label: "borrowed", focus: true, artifact: null, effects: [], warnings: [] };
}

function createPlan(): WorkspaceSetupPlan {
  return { ...openPlan(), repository, mode: "create", ownership: "owned_worktree", branch: "task", checkout_path: "/worktrees/task" };
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

function SetupDialogHarness() {
  const [open, setOpen] = useState(true);
  const [, setTerminalRevision] = useState(1);
  emitTerminalUpdate = () => setTerminalRevision((value) => value + 1);
  return createElement(SetupDialog, { client, sessionId: "session-1", open, onClose: () => setOpen(false), onCompleted: () => undefined });
}

function ReopenHarness({ dialogClient }: { dialogClient: SetupClient }) {
  const [open, setOpen] = useState(true);
  return createElement("div", undefined,
    createElement("button", { type: "button", onClick: () => setOpen(true) }, "Reopen setup"),
    createElement(SetupDialog, { client: dialogClient, sessionId: "session-1", open, onClose: () => setOpen(false), onCompleted: () => undefined }),
  );
}

async function renderDialog(dialogClient: SetupClient = client): Promise<void> {
  container = document.createElement("div");
  document.body.append(container);
  await act(async () => {
    root = createRoot(container!);
    root.render(createElement(SetupDialog, { client: dialogClient, sessionId: "session-1", open: true, onClose: () => undefined, onCompleted: () => undefined }));
  });
  await settle();
}

describe("SetupDialog", () => {
  it("accepts a newer generation and sequence, but rejects stale or duplicate snapshots", () => {
    const current = operation(4, 12);
    expect(operationSnapshotIsNewer(current, operation(4, 13))).toBe(true);
    expect(operationSnapshotIsNewer(current, operation(5, 0))).toBe(true);
    expect(operationSnapshotIsNewer(current, operation(4, 12))).toBe(false);
    expect(operationSnapshotIsNewer(current, operation(3, 99))).toBe(false);
  });

  it("describes Context checkpoints and source retry", () => {
    expect(operationStatusMessage({ state: "running", step: "context_preparing", error: null })).toContain("Preparing Context");
    expect(operationStatusMessage({ state: "running", step: "context_ready", error: null })).toContain("Context is ready");
    expect(operationStatusMessage({ state: "partial", step: "context_preparing", error: { code: "source_provider_unsupported", message: "provider is unavailable" } })).toContain("Retry the source step");
  });

  it("keeps the typed branch focused across unrelated rerenders", async () => {
    container = document.createElement("div");
    document.body.append(container);
    await act(async () => {
      root = createRoot(container!);
      root.render(createElement(SetupDialogHarness));
    });
    await settle();

    const branch = container.querySelector<HTMLInputElement>("#setup-branch")!;
    branch.focus();
    act(() => emitTerminalUpdate?.());
    expect(document.activeElement).toBe(branch);
    writeInput(branch, "retain-focus");
    await settle();
    expect(document.activeElement).toBe(branch);
    expect(branch.value).toBe("retain-focus");
  });

  it("ignores a late source lookup and preserves manual branch edits", async () => {
    vi.useFakeTimers();
    const first = deferred<WorkspaceDefaults>();
    const second = deferred<WorkspaceDefaults>();
    const resolveWorkspaceDefaults = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const dialogClient: SetupClient = { ...client, resolveWorkspaceDefaults };
    await renderDialog(dialogClient);

    const source = container!.querySelector<HTMLInputElement>("#setup-artifact-url")!;
    const branch = container!.querySelector<HTMLInputElement>("#setup-branch")!;
    await act(async () => { writeInput(source, "https://github.com/nnexai/cockpit/issues/4"); });
    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    await act(async () => {
      writeInput(branch, "manual-branch");
      writeInput(source, "https://github.com/nnexai/cockpit/issues/5");
    });
    await act(async () => { await vi.advanceTimersByTimeAsync(300); });

    await act(async () => { second.resolve(sourceDefaults("new-branch", "cockpit#5")); await Promise.resolve(); });
    expect(branch.value).toBe("manual-branch");
    await act(async () => { first.resolve(sourceDefaults("old-branch", "cockpit#4")); await Promise.resolve(); });
    expect(branch.value).toBe("manual-branch");
    expect(container!.textContent).toContain("cockpit#5");
  });

  it("opens an exact path without repository, source, or branch inputs", async () => {
    const planWorkspace = vi.fn(async () => openPlan());
    const startWorkspace = vi.fn(async () => workspaceOperation(openPlan(), "completed"));
    const dialogClient: SetupClient = { ...client, planWorkspace, startWorkspace };
    await renderDialog(dialogClient);

    const existing = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("Existing directory"))!;
    act(() => existing.click());
    await settle();
    expect(container!.querySelector("#setup-repository")).toBeNull();
    expect(container!.querySelector("#setup-artifact-url")).toBeNull();
    expect(container!.querySelector("#setup-branch")).toBeNull();

    const path = container!.querySelector<HTMLInputElement>("#setup-checkout")!;
    writeInput(path, "/tmp/borrowed");
    await settle();
    const submit = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Open Space")!;
    await act(async () => { submit.click(); await Promise.resolve(); });
    expect(planWorkspace).toHaveBeenCalledWith("session-1", expect.objectContaining({ operation: "open", path: "/tmp/borrowed" }));
  });

  it("does not reuse an opened directory as a worktree destination", async () => {
    const planWorkspace = vi.fn(async () => createPlan());
    const startWorkspace = vi.fn(async () => workspaceOperation(createPlan(), "completed"));
    const dialogClient: SetupClient = { ...client, planWorkspace, startWorkspace };
    await renderDialog(dialogClient);

    const buttons = () => [...container!.querySelectorAll<HTMLButtonElement>("button")];
    act(() => buttons().find((button) => button.textContent?.includes("Existing directory"))?.click());
    await settle();
    await act(async () => { writeInput(container!.querySelector<HTMLInputElement>("#setup-checkout")!, "/tmp/borrowed"); });

    act(() => buttons().find((button) => button.textContent?.includes("New worktree"))?.click());
    await settle();
    act(() => buttons().find((button) => button.textContent?.includes("Existing directory"))?.click());
    await settle();
    expect(container!.querySelector<HTMLInputElement>("#setup-checkout")?.value).toBe("/tmp/borrowed");

    act(() => buttons().find((button) => button.textContent?.includes("New worktree"))?.click());
    await settle();
    const submit = buttons().find((button) => button.textContent === "Create Space")!;
    await act(async () => { submit.click(); await Promise.resolve(); });
    expect(planWorkspace).toHaveBeenCalledWith("session-1", expect.objectContaining({ operation: "create", checkout_path: null }));
  });

  it("does not execute a plan that became stale while it was loading", async () => {
    const planned = deferred<WorkspaceSetupPlan>();
    const planWorkspace = vi.fn(() => planned.promise);
    const startWorkspace = vi.fn();
    const dialogClient: SetupClient = { ...client, planWorkspace, startWorkspace };
    await renderDialog(dialogClient);

    const submit = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Create Space")!;
    act(() => submit.click());
    writeInput(container!.querySelector<HTMLInputElement>("#setup-branch")!, "new-task");
    await act(async () => { planned.resolve(createPlan()); await Promise.resolve(); });
    expect(startWorkspace).not.toHaveBeenCalled();
  });

  it("retains a failed start receipt across close and reopens it for inspection", async () => {
    const plan = createPlan();
    const planWorkspace = vi.fn(async () => plan);
    const startWorkspace = vi.fn(async () => { throw new Error("connection lost"); });
    const readOperation = vi.fn()
      .mockRejectedValueOnce(new Error("status unavailable"))
      .mockResolvedValueOnce(workspaceOperation(plan, "outcome_unknown"));
    const dialogClient: SetupClient = { ...client, planWorkspace, startWorkspace, workspaceOperation: readOperation };
    container = document.createElement("div");
    document.body.append(container);
    await act(async () => {
      root = createRoot(container!);
      root.render(createElement(ReopenHarness, { dialogClient }));
    });
    await settle();

    const submit = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Create Space")!;
    await act(async () => { submit.click(); await Promise.resolve(); });
    await settle();
    expect(planWorkspace).toHaveBeenCalledTimes(1);
    expect(container!.querySelector<HTMLInputElement>("#setup-branch")?.disabled).toBe(true);
    expect(container!.textContent).toContain("Check operation");

    act(() => container!.querySelector<HTMLButtonElement>("[aria-label='Close setup dialog']")?.click());
    act(() => [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Reopen setup")?.click());
    await settle();
    const inspect = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Check operation")!;
    await act(async () => { inspect.click(); await Promise.resolve(); });
    expect(planWorkspace).toHaveBeenCalledTimes(1);
    expect(container!.textContent).toContain("Recover existing checkout");
  });
});
