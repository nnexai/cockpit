// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { WorkspaceTeardownPreview, WorkspaceTeardownResult } from "../../protocol/generated/v1";
import { TeardownDialog, type TeardownClient } from "./TeardownDialog";
import { TeardownRecoveryPanel } from "./TeardownRecoveryPanel";

let root: Root | null = null;
let host: HTMLElement | null = null;

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  root = null;
  host = null;
});

function mount(node: React.ReactNode): HTMLElement {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  act(() => root!.render(node));
  return host;
}

const escape = () => act(() => { window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });

function removalButton(view: HTMLElement, label: string): HTMLButtonElement {
  return [...view.querySelectorAll<HTMLButtonElement>(".setup-actions button")].find((button) => button.textContent === label)!;
}

function dirtyPreview(overrides: Partial<WorkspaceTeardownPreview> = {}): WorkspaceTeardownPreview {
  return {
    operation_id: "operation", workspace_id: "space", endpoint_identity: "endpoint",
    repository_key: "repo", repository_root: "/repo", checkout_path: "/worktrees/task",
    ownership: "selected_linked", workspace_state: "live", is_linked_worktree: true, dirty_state: "dirty",
    discard_count: 3, discard_paths: ["changed.txt", "new file.txt", "nested/deleted.txt"],
    discard_fingerprint: "a".repeat(64),
    allowed_actions: ["close_space", "remove_owned_worktree"], blockers: [], warnings: [],
    required_confirmation: "DISCARD AND REMOVE",
    ...overrides,
  };
}

it("closes the teardown review with Escape and keeps Tab inside it", async () => {
  const client = { workspaceTeardownPreview: vi.fn(() => new Promise<never>(() => undefined)), workspaceTeardownExecute: vi.fn() };
  const onClose = vi.fn();
  const view = mount(<TeardownDialog client={client} sessionId="s" workspaceId="w" open onClose={onClose} onCompleted={vi.fn()} />);
  const dialog = view.querySelector<HTMLElement>('[role="dialog"]')!;
  const controls = [...dialog.querySelectorAll<HTMLElement>("button:not(:disabled)")];
  expect(document.activeElement).toBe(controls[0]);
  act(() => { controls.at(-1)!.focus(); controls.at(-1)!.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true })); });
  expect(document.activeElement).toBe(controls[0]);
  escape();
  expect(onClose).toHaveBeenCalledOnce();
});

it("closes Pending cleanup with Escape", () => {
  const client = { workspaceTeardownPreview: vi.fn(), workspaceTeardownExecute: vi.fn(), workspaceTeardownRecoveries: vi.fn(() => new Promise<never>(() => undefined)) };
  const onClose = vi.fn();
  mount(<TeardownRecoveryPanel client={client} sessionId="s" open onClose={onClose} />);
  escape();
  expect(onClose).toHaveBeenCalledOnce();
});

it("requires selecting and then confirming clean removal and omits the discard fingerprint", async () => {
  const preview: WorkspaceTeardownPreview = {
    operation_id: "operation", workspace_id: "space", endpoint_identity: "endpoint",
    repository_key: "repo", repository_root: "/repo", checkout_path: "/worktrees/task",
    ownership: "selected_linked", workspace_state: "live", is_linked_worktree: true, dirty_state: "clean",
    discard_count: 0, discard_paths: [], discard_fingerprint: null,
    allowed_actions: ["close_space", "remove_owned_worktree"], blockers: [], warnings: ["obsolete recovery advice"],
    required_confirmation: "REMOVE WORKTREE",
  };
  const result: WorkspaceTeardownResult = {
    operation_id: "operation", workspace_id: "space", action: "remove_owned_worktree",
    outcome: "completed", message: "Task worktree removed",
  };
  const client = {
    workspaceTeardownPreview: vi.fn(async () => preview),
    workspaceTeardownExecute: vi.fn<TeardownClient["workspaceTeardownExecute"]>(async () => result),
  };
  const view = mount(<TeardownDialog client={client} sessionId="session" workspaceId="space" open onClose={vi.fn()} onCompleted={vi.fn()} />);
  await act(async () => { await Promise.resolve(); });
  expect(view.querySelector(".setup-discard-list")).toBeNull();
  expect(removalButton(view, "Choose an action").disabled).toBe(true);
  const action = view.querySelector<HTMLButtonElement>(".setup-progress-actions button:last-child")!;
  act(() => action.click());
  expect(action.getAttribute("aria-pressed")).toBe("true");
  expect(client.workspaceTeardownExecute).not.toHaveBeenCalled();
  const confirm = removalButton(view, "Confirm remove worktree");
  expect(confirm.disabled).toBe(false);
  await act(async () => confirm.click());
  expect(client.workspaceTeardownExecute).toHaveBeenCalledWith("session", {
    operation_id: "operation", workspace_id: "space", expected_endpoint_identity: "endpoint",
    expected_checkout_path: "/worktrees/task", action: "remove_owned_worktree", confirmation: "REMOVE WORKTREE",
  });
  expect(client.workspaceTeardownExecute.mock.calls[0]![1]).not.toHaveProperty("discard_fingerprint");
});

it("reviews dirty paths and sends the server token and fingerprint only after the second button press", async () => {
  const preview = dirtyPreview();
  const result: WorkspaceTeardownResult = {
    operation_id: "operation", workspace_id: "space", action: "remove_owned_worktree",
    outcome: "completed", message: "Worktree removed",
  };
  const client = {
    workspaceTeardownPreview: vi.fn(async () => preview),
    workspaceTeardownExecute: vi.fn(async () => result),
  };
  const view = mount(<TeardownDialog client={client} sessionId="session" workspaceId="space" open onClose={vi.fn()} onCompleted={vi.fn()} />);
  await act(async () => { await Promise.resolve(); });
  expect(view.textContent).toContain("3 uncommitted paths will be permanently discarded");
  const list = view.querySelector<HTMLUListElement>(".setup-discard-list")!;
  expect(list.tabIndex).toBe(0);
  expect(list.getAttribute("aria-label")).toBe("Paths to be discarded");
  expect([...list.querySelectorAll("code")].map((path) => path.textContent)).toEqual(preview.discard_paths);
  act(() => { list.focus(); });
  expect(document.activeElement).toBe(list);
  expect(removalButton(view, "Choose an action").disabled).toBe(true);
  const action = view.querySelector<HTMLButtonElement>(".setup-progress-actions button:last-child")!;
  act(() => action.click());
  expect(action.getAttribute("aria-pressed")).toBe("true");
  expect(client.workspaceTeardownExecute).not.toHaveBeenCalled();
  const confirm = removalButton(view, "Confirm discard 3 paths and remove");
  expect(confirm.disabled).toBe(false);
  expect(confirm.getAttribute("aria-describedby")).toBe("teardown-confirm-hint");
  expect(view.querySelector("#teardown-confirm-hint")!.textContent).toContain(preview.checkout_path);
  await act(async () => confirm.click());
  expect(client.workspaceTeardownExecute).toHaveBeenCalledWith("session", {
    operation_id: "operation", workspace_id: "space", expected_endpoint_identity: "endpoint",
    expected_checkout_path: "/worktrees/task", action: "remove_owned_worktree",
    confirmation: "DISCARD AND REMOVE", discard_fingerprint: preview.discard_fingerprint,
  });
});

it("shows a bounded, focusable discard list and reports paths omitted from the preview", async () => {
  const preview = dirtyPreview({
    discard_count: 200,
    discard_paths: Array.from({ length: 50 }, (_, index) => `nested/path-${index}.txt`),
  });
  const client = {
    workspaceTeardownPreview: vi.fn(async () => preview),
    workspaceTeardownExecute: vi.fn(),
  };
  const view = mount(<TeardownDialog client={client} sessionId="session" workspaceId="space" open onClose={vi.fn()} onCompleted={vi.fn()} />);
  await act(async () => { await Promise.resolve(); });
  expect(view.querySelectorAll(".setup-discard-list li")).toHaveLength(50);
  expect(view.querySelector(".setup-discard-list")!.getAttribute("tabindex")).toBe("0");
  expect(view.textContent).toContain("200 uncommitted paths will be permanently discarded");
  expect(view.textContent).toContain("…and 150 more not shown.");
});

it("omits the dirty fingerprint when closing the Space instead of removing its worktree", async () => {
  const preview = dirtyPreview();
  const result: WorkspaceTeardownResult = {
    operation_id: "operation", workspace_id: "space", action: "close_space",
    outcome: "completed", message: "Space closed",
  };
  const client = {
    workspaceTeardownPreview: vi.fn(async () => preview),
    workspaceTeardownExecute: vi.fn<TeardownClient["workspaceTeardownExecute"]>(async () => result),
  };
  const view = mount(<TeardownDialog client={client} sessionId="session" workspaceId="space" open onClose={vi.fn()} onCompleted={vi.fn()} />);
  await act(async () => { await Promise.resolve(); });
  act(() => { view.querySelector<HTMLButtonElement>(".setup-progress-actions button")!.click(); });
  expect(client.workspaceTeardownExecute).not.toHaveBeenCalled();
  await act(async () => removalButton(view, "Close Space").click());
  expect(client.workspaceTeardownExecute).toHaveBeenCalledWith("session", {
    operation_id: "operation", workspace_id: "space", expected_endpoint_identity: "endpoint",
    expected_checkout_path: "/worktrees/task", action: "close_space", confirmation: "",
  });
  expect(client.workspaceTeardownExecute.mock.calls[0]![1]).not.toHaveProperty("discard_fingerprint");
});

it.each(["stale_identity", "git_failure"])("refreshes a %s removal failure while preserving its error and requiring a fresh selection and confirmation", async (operationCode) => {
  const initial = dirtyPreview();
  const fresh = dirtyPreview({
    operation_id: "fresh-operation", endpoint_identity: "fresh-endpoint", checkout_path: "/worktrees/fresh-task",
    required_confirmation: "FRESH DISCARD CONFIRMATION",
    discard_count: 1, discard_paths: ["new-change.txt"], discard_fingerprint: "b".repeat(64),
  });
  const failure = Object.assign(new Error("The reviewed worktree changed. Review it again."), { operationCode });
  const result: WorkspaceTeardownResult = {
    operation_id: "operation", workspace_id: "space", action: "remove_owned_worktree",
    outcome: "completed", message: "Worktree removed",
  };
  const client = {
    workspaceTeardownPreview: vi.fn().mockResolvedValueOnce(initial).mockResolvedValueOnce(fresh),
    workspaceTeardownExecute: vi.fn().mockRejectedValueOnce(failure).mockResolvedValueOnce(result),
  };
  const onCompleted = vi.fn();
  const view = mount(<TeardownDialog client={client} sessionId="session" workspaceId="space" open onClose={vi.fn()} onCompleted={onCompleted} />);
  await act(async () => { await Promise.resolve(); });
  act(() => { view.querySelector<HTMLButtonElement>('.setup-progress-actions button:last-child')!.click(); });
  expect(client.workspaceTeardownExecute).not.toHaveBeenCalled();
  await act(async () => removalButton(view, "Confirm discard 3 paths and remove").click());
  await act(async () => { await Promise.resolve(); });
  expect(client.workspaceTeardownPreview).toHaveBeenCalledTimes(2);
  expect(client.workspaceTeardownPreview).toHaveBeenLastCalledWith("session", { workspace_id: "space" });
  expect(client.workspaceTeardownExecute).toHaveBeenCalledTimes(1);
  expect(onCompleted).not.toHaveBeenCalled();
  const alert = view.querySelector('[role="alert"]')!;
  if (operationCode === "stale_identity") {
    expect(alert.textContent).not.toBe(failure.message);
    expect(alert.textContent).toMatch(/refresh.*review/i);
  } else {
    expect(alert.textContent).toBe(failure.message);
  }
  expect(alert.getAttribute("title")).toContain(`${operationCode}: ${failure.message}`);
  expect(view.textContent).toContain("1 uncommitted path will be permanently discarded");
  expect(view.textContent).toContain("new-change.txt");
  expect(view.textContent).not.toContain("changed.txt");
  expect(view.querySelector('.setup-progress-actions [aria-pressed="true"]')).toBeNull();
  expect(removalButton(view, "Choose an action").disabled).toBe(true);
  act(() => { view.querySelector<HTMLButtonElement>('.setup-progress-actions button:last-child')!.click(); });
  expect(client.workspaceTeardownExecute).toHaveBeenCalledTimes(1);
  const confirm = removalButton(view, "Confirm discard 1 path and remove");
  expect(confirm.disabled).toBe(false);
  await act(async () => confirm.click());
  expect(client.workspaceTeardownExecute).toHaveBeenCalledTimes(2);
  expect(client.workspaceTeardownExecute).toHaveBeenLastCalledWith("session", {
    operation_id: fresh.operation_id, workspace_id: fresh.workspace_id,
    expected_endpoint_identity: fresh.endpoint_identity, expected_checkout_path: fresh.checkout_path,
    action: "remove_owned_worktree", confirmation: fresh.required_confirmation,
    discard_fingerprint: fresh.discard_fingerprint,
  });
});

it("does not enable removal without a server-issued confirmation token", async () => {
  const client = {
    workspaceTeardownPreview: vi.fn(async () => dirtyPreview({ required_confirmation: null })),
    workspaceTeardownExecute: vi.fn(),
  };
  const view = mount(<TeardownDialog client={client} sessionId="session" workspaceId="space" open onClose={vi.fn()} onCompleted={vi.fn()} />);
  await act(async () => { await Promise.resolve(); });
  act(() => { view.querySelector<HTMLButtonElement>(".setup-progress-actions button:last-child")!.click(); });
  const confirm = removalButton(view, "Confirm discard 3 paths and remove");
  expect(confirm.disabled).toBe(true);
  act(() => confirm.click());
  expect(client.workspaceTeardownExecute).not.toHaveBeenCalled();
});

it("reconciles removal without sending the removal token or discard fingerprint", async () => {
  const preview = dirtyPreview({ allowed_actions: ["reconcile_remove_outcome"] });
  const result: WorkspaceTeardownResult = {
    operation_id: preview.operation_id, workspace_id: preview.workspace_id,
    action: "reconcile_remove_outcome", outcome: "completed", message: "Removal reconciled",
  };
  const client = {
    workspaceTeardownPreview: vi.fn(async () => preview),
    workspaceTeardownExecute: vi.fn<TeardownClient["workspaceTeardownExecute"]>(async () => result),
  };
  const onClose = vi.fn();
  const onCompleted = vi.fn();
  const view = mount(<TeardownDialog client={client} sessionId="session" workspaceId="space" open onClose={onClose} onCompleted={onCompleted} />);
  await act(async () => { await Promise.resolve(); });
  act(() => { view.querySelector<HTMLButtonElement>(".setup-progress-actions button")!.click(); });
  expect(client.workspaceTeardownExecute).not.toHaveBeenCalled();
  await act(async () => removalButton(view, "Reconcile removal").click());
  expect(client.workspaceTeardownExecute).toHaveBeenCalledWith("session", {
    operation_id: preview.operation_id, workspace_id: preview.workspace_id,
    expected_endpoint_identity: preview.endpoint_identity, expected_checkout_path: preview.checkout_path,
    action: "reconcile_remove_outcome", confirmation: "",
  });
  expect(client.workspaceTeardownExecute.mock.calls[0]![1]).not.toHaveProperty("discard_fingerprint");
  expect(onCompleted).toHaveBeenCalledWith(result);
  act(() => removalButton(view, "Done").click());
  expect(onClose).toHaveBeenCalledOnce();
});
