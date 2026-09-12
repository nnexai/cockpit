// @vitest-environment jsdom

import { act, createElement, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ProjectConfiguration, RepositoryListResponse, WorkspaceOperation } from "../../protocol/generated/v1";
import { operationSnapshotIsNewer, operationStatusMessage, SetupDialog, validateArtifactUrl } from "./SetupDialog";

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
  limits: {
    catalog_depth: 4,
    catalog_entries: 100,
    git_timeout_ms: 1_000,
    git_output_bytes: 1_000,
    operation_timeout_ms: 1_000,
    context_preview_bytes: 1_000,
    context_preview_lines: 100,
    context_directory_entries: 100,
    context_tree_depth: 4,
  },
  origins: {},
};

const repositories: RepositoryListResponse = {
  repositories: [{
    repository_id: "repository",
    name: "Repository",
    root: "/repositories/repository",
    checkout_path: "/repositories/repository",
    common_dir: "/repositories/repository/.git",
    branch: "main",
    is_linked_worktree: false,
    is_detached: false,
    provenance: "configured",
  }],
  diagnostics: [],
};

const client = {
  projectConfiguration: async () => configuration,
  repositories: async () => repositories,
} as CockpitClient;

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
});

async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function SetupDialogHarness() {
  const [open, setOpen] = useState(true);
  const [, setTerminalRevision] = useState(1);
  emitTerminalUpdate = () => setTerminalRevision((value) => value + 1);
  return createElement(SetupDialog, { client, sessionId: "session-1", open, onClose: () => setOpen(false), onCompleted: () => undefined });
}

describe("SetupDialog operation snapshot ordering", () => {
  it("validates source provider ownership and issue identity before planning", () => {
    const configured = { ...configuration, providers: [{ id: "github", base_url: "https://github.com", executable: "gh" }] };
    expect(validateArtifactUrl("https://github.com/nnexai/cockpit/issues/4", configured)).toEqual({ valid: true, providerId: "github", identity: "nnexai/cockpit#4" });
    expect(validateArtifactUrl("https://github.com/nnexai/cockpit/pulls/4", configured)).toMatchObject({ valid: false });
    expect(validateArtifactUrl("https://forge.example/acme/repo/issues/4", configured)).toMatchObject({ valid: false });
  });

  it("accepts a newer generation and sequence, but rejects stale or duplicate snapshots", () => {
    const current = operation(4, 12);
    expect(operationSnapshotIsNewer(current, operation(4, 13))).toBe(true);
    expect(operationSnapshotIsNewer(current, operation(5, 0))).toBe(true);
    expect(operationSnapshotIsNewer(current, operation(4, 12))).toBe(false);
    expect(operationSnapshotIsNewer(current, operation(4, 11))).toBe(false);
    expect(operationSnapshotIsNewer(current, operation(3, 99))).toBe(false);
  });

  it("accepts the first authoritative snapshot without inventing an ordering", () => {
    expect(operationSnapshotIsNewer(null, operation(0, 0))).toBe(true);
  });

  it("explains Context checkpoints and offers a source retry for partial imports", () => {
    expect(operationStatusMessage({ state: "running", step: "context_preparing", error: null }))
      .toContain("Preparing the Context");
    expect(operationStatusMessage({ state: "running", step: "context_ready", error: null }))
      .toContain("Context is ready");
    expect(operationStatusMessage({
      state: "partial",
      step: "context_preparing",
      error: { code: "source_provider_unsupported", message: "provider is unavailable" },
    })).toContain("Retry the source step");
  });

  it("preselects the repository associated with the selected Space", async () => {
    container = document.createElement("div");
    document.body.append(container);
    await act(async () => {
      root = createRoot(container!);
      root.render(createElement(SetupDialog, {
        client,
        sessionId: "session-1",
        open: true,
        selectedParent: { label: "Parent", repositoryKey: "/repositories/repository/.git", checkoutPath: "/repositories/repository" },
        onClose: () => undefined,
        onCompleted: () => undefined,
      }));
    });
    await settle();

    expect(container.querySelector<HTMLElement>('[role="option"][aria-selected="true"]')?.textContent).toContain("Repository");
    expect(container.textContent).toContain("Opened from Parent");
  });

  it("keeps the typed field focused across terminal updates and its own rerender", async () => {
    container = document.createElement("div");
    document.body.append(container);
    await act(async () => {
      root = createRoot(container!);
      root.render(createElement(SetupDialogHarness));
    });
    await settle();

    const taskName = container.querySelector<HTMLInputElement>("#setup-task-name")!;
    taskName.focus();
    act(() => emitTerminalUpdate?.());
    expect(document.activeElement).toBe(taskName);

    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(taskName, "retain focus");
      taskName.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await settle();
    expect(document.activeElement).toBe(taskName);
    expect(taskName.value).toBe("retain focus");
  });
});
