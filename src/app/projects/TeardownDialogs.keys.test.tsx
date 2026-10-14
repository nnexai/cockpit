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

function setConfirmation(view: HTMLElement, value: string): void {
  const input = view.querySelector<HTMLInputElement>("#teardown-confirmation")!;
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

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

it("requires the exact server confirmation and omits a fingerprint for clean removal", async () => {
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
  expect(view.textContent).toContain("the selected Space's linked worktree");
  expect(view.textContent).not.toContain("Cockpit-owned");
  expect(view.textContent).not.toContain("Recovery notes");
  expect(view.textContent).not.toContain("obsolete recovery advice");
  expect(view.querySelector(".setup-discard-list")).toBeNull();
  const action = [...view.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Remove worktree")!;
  act(() => action.click());
  const confirm = removalButton(view, "Remove worktree");
  expect(confirm.disabled).toBe(true);
  for (const wrong of ["REMOVE", "REMOVE /worktrees/task", "REMOVE WORKTREE ", "remove worktree"]) {
    setConfirmation(view, wrong);
    expect(confirm.disabled).toBe(true);
    expect(client.workspaceTeardownExecute).not.toHaveBeenCalled();
  }
  setConfirmation(view, preview.required_confirmation!);
  expect(confirm.disabled).toBe(false);
  await act(async () => confirm.click());
  expect(client.workspaceTeardownExecute).toHaveBeenCalledWith("session", {
    operation_id: "operation", workspace_id: "space", expected_endpoint_identity: "endpoint",
    expected_checkout_path: "/worktrees/task", action: "remove_owned_worktree", confirmation: "REMOVE WORKTREE",
  });
  expect(client.workspaceTeardownExecute.mock.calls[0]![1]).not.toHaveProperty("discard_fingerprint");
});

it("reviews dirty paths and sends the fingerprint only after the server's discard confirmation", async () => {
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
  const action = [...view.querySelectorAll<HTMLButtonElement>(".setup-progress-actions button")].find((button) => button.textContent === "Discard and remove worktree")!;
  act(() => action.click());
  const confirm = removalButton(view, "Discard and remove worktree");
  expect(confirm.disabled).toBe(true);
  expect(view.querySelector(".setup-label")!.textContent).toContain("discard 3 uncommitted paths and remove this exact reviewed worktree");
  for (const wrong of ["REMOVE WORKTREE", "DISCARD AND REMOVE ", "discard and remove"]) {
    setConfirmation(view, wrong);
    expect(confirm.disabled).toBe(true);
    expect(client.workspaceTeardownExecute).not.toHaveBeenCalled();
  }
  setConfirmation(view, preview.required_confirmation!);
  expect(confirm.disabled).toBe(false);
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
  expect(view.querySelector("#teardown-confirmation")).toBeNull();
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
  setConfirmation(view, initial.required_confirmation!);
  await act(async () => removalButton(view, "Discard and remove worktree").click());
  await act(async () => { await Promise.resolve(); });
  expect(client.workspaceTeardownPreview).toHaveBeenCalledTimes(2);
  expect(client.workspaceTeardownPreview).toHaveBeenLastCalledWith("session", { workspace_id: "space" });
  expect(client.workspaceTeardownExecute).toHaveBeenCalledTimes(1);
  expect(onCompleted).not.toHaveBeenCalled();
  expect(view.querySelector('[role="alert"]')!.textContent).toBe(failure.message);
  expect(view.textContent).toContain("1 uncommitted path will be permanently discarded");
  expect(view.textContent).toContain("new-change.txt");
  expect(view.textContent).not.toContain("changed.txt");
  expect(view.querySelector('.setup-progress-actions [aria-pressed="true"]')).toBeNull();
  expect(view.querySelector("#teardown-confirmation")).toBeNull();
  expect(removalButton(view, "Choose an action").disabled).toBe(true);
  act(() => { view.querySelector<HTMLButtonElement>('.setup-progress-actions button:last-child')!.click(); });
  expect(view.querySelector<HTMLInputElement>("#teardown-confirmation")!.value).toBe("");
  expect(removalButton(view, "Discard and remove worktree").disabled).toBe(true);
  setConfirmation(view, fresh.required_confirmation!);
  await act(async () => removalButton(view, "Discard and remove worktree").click());
  expect(client.workspaceTeardownExecute).toHaveBeenCalledTimes(2);
  expect(client.workspaceTeardownExecute).toHaveBeenLastCalledWith("session", expect.objectContaining({
    confirmation: fresh.required_confirmation, discard_fingerprint: fresh.discard_fingerprint,
  }));
});
