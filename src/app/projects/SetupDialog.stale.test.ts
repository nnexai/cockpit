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
  it("selects a repository through subsequence search and requires explicit approval", async () => {
    const planWorkspace = vi.fn(async () => createPlan());
    const startWorkspace = vi.fn(async () => workspaceOperation(createPlan(), "completed"));
    await renderDialog({ ...client, planWorkspace, startWorkspace });

    const repositoryInput = container!.querySelector<HTMLInputElement>("#setup-repository")!;
    await act(async () => { repositoryInput.focus(); writeInput(repositoryInput, "rps"); });
    expect([...container!.querySelectorAll(".setup-repository mark")].map((mark) => mark.textContent).join("")).toBe("Rps");
    await act(async () => { repositoryInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
    const review = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Review setup")!;
    await act(async () => { review.click(); await Promise.resolve(); });
    expect(planWorkspace).toHaveBeenCalledWith("session-1", expect.objectContaining({ repository_id: "repository" }));
    expect(startWorkspace).not.toHaveBeenCalled();
    const approve = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Start setup")!;
    await act(async () => { approve.click(); await Promise.resolve(); });
    expect(startWorkspace).toHaveBeenCalledTimes(1);
  });
  it("opens repository matches as a focused overlay without adding form height", async () => {
    await renderDialog();
    const input = container!.querySelector<HTMLInputElement>("#setup-repository")!;
    expect(container!.querySelector("#setup-repository-results")).toBeNull();
    expect(container!.querySelector("#setup-branch")).not.toBeNull();

    act(() => input.focus());
    expect(container!.querySelector("#setup-repository-results")).not.toBeNull();
    expect(container!.querySelector("#setup-branch")).not.toBeNull();

    act(() => input.blur());
    expect(container!.querySelector("#setup-repository-results")).toBeNull();
  });
  it("keeps a source-selected nested repository active in the picker", async () => {
    vi.useFakeTimers();
    const nestedRepository = { ...repository, repository_id: "nested", name: "Nested Repository", root: "/repositories/nested", checkout_path: "/repositories/nested", common_dir: "/repositories/nested/.git" };
    const repositoryResponse: RepositoryListResponse = { repositories: [repository, nestedRepository], diagnostics: [] };
    const planWorkspace = vi.fn(async () => ({ ...createPlan(), repository: nestedRepository }));
    const startWorkspace = vi.fn(async () => workspaceOperation({ ...createPlan(), repository: nestedRepository }, "completed"));
    const dialogClient: SetupClient = {
      ...client,
      repositories: async () => repositoryResponse,
      resolveWorkspaceDefaults: async () => ({ ...sourceDefaults("main", "cockpit#5"), repositories: repositoryResponse.repositories, repository_id: nestedRepository.repository_id }),
      planWorkspace,
      startWorkspace,
    };
    try {
      await renderDialog(dialogClient);
      const source = container!.querySelector<HTMLInputElement>("#setup-artifact-url")!;
      await act(async () => {
        writeInput(source, "https://github.com/nnexai/cockpit/issues/5");
        await vi.advanceTimersByTimeAsync(300);
      });
      await settle();
      const input = container!.querySelector<HTMLInputElement>("#setup-repository")!;
      act(() => input.focus());
      const review = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Review setup")!;
      await act(async () => { review.click(); await Promise.resolve(); });
      expect(planWorkspace).toHaveBeenCalledWith("session-1", expect.objectContaining({ repository_id: "nested" }));
    } finally {
      vi.useRealTimers();
    }
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
    const review = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Review setup")!;
    await act(async () => { review.click(); await Promise.resolve(); });
    expect(planWorkspace).toHaveBeenCalledWith("session-1", expect.objectContaining({ operation: "open", path: "/tmp/borrowed" }));
    expect(startWorkspace).not.toHaveBeenCalled();
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
    const review = buttons().find((button) => button.textContent === "Review setup")!;
    await act(async () => { review.click(); await Promise.resolve(); });
    expect(planWorkspace).toHaveBeenCalledWith("session-1", expect.objectContaining({ operation: "create", checkout_path: null }));
  });

  it("does not execute a plan that became stale while it was loading", async () => {
    const planned = deferred<WorkspaceSetupPlan>();
    const planWorkspace = vi.fn(() => planned.promise);
    const startWorkspace = vi.fn();
    const dialogClient: SetupClient = { ...client, planWorkspace, startWorkspace };
    await renderDialog(dialogClient);

    const review = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Review setup")!;
    act(() => review.click());
    writeInput(container!.querySelector<HTMLInputElement>("#setup-branch")!, "new-task");
    await act(async () => { planned.resolve(createPlan()); await Promise.resolve(); });
    expect(startWorkspace).not.toHaveBeenCalled();
  });
  it("invalidates the reviewed plan when an input changes before approval", async () => {
    const planWorkspace = vi.fn(async () => createPlan());
    const startWorkspace = vi.fn(async () => workspaceOperation(createPlan(), "completed"));
    await renderDialog({ ...client, planWorkspace, startWorkspace });

    const review = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Review setup")!;
    await act(async () => { review.click(); await Promise.resolve(); });
    expect(container!.textContent).toContain("Reviewed setup effects");
    await act(async () => { writeInput(container!.querySelector<HTMLInputElement>("#setup-branch")!, "deliberate-edit"); });
    expect(container!.textContent).not.toContain("Reviewed setup effects");
    expect(startWorkspace).not.toHaveBeenCalled();
  });

  it("returns a rejected approval to editable review using the backend operation code", async () => {
    const failure = Object.assign(new Error("The reviewed endpoint changed"), {
      code: "http_error", operationCode: "stale_plan",
    });
    const planWorkspace = vi.fn(async () => createPlan());
    const startWorkspace = vi.fn(async () => { throw failure; });
    await renderDialog({ ...client, planWorkspace, startWorkspace });
    const button = (label: string) => [...container!.querySelectorAll<HTMLButtonElement>("button")].find((candidate) => candidate.textContent === label)!;
    await act(async () => { button("Review setup").click(); await Promise.resolve(); });
    await act(async () => { button("Start setup").click(); await Promise.resolve(); });
    await settle();

    expect(container!.querySelector<HTMLInputElement>("#setup-branch")?.disabled).toBe(false);
    expect(container!.querySelector("[role='alert']")?.textContent).toContain(failure.message);
    expect(button("Review setup").disabled).toBe(false);
    await act(async () => { button("Review setup").click(); await Promise.resolve(); });
    expect(button("Start setup").disabled).toBe(false);
    expect(startWorkspace).toHaveBeenCalledTimes(1);
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

    const review = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Review setup")!;
    await act(async () => { review.click(); await Promise.resolve(); });
    const approve = [...container!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Start setup")!;
    await act(async () => { approve.click(); await Promise.resolve(); });
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
