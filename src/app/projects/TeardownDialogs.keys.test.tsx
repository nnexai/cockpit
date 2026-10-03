// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { WorkspaceTeardownPreview, WorkspaceTeardownResult } from "../../protocol/generated/v1";
import { TeardownDialog } from "./TeardownDialog";
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

it("requires the exact reviewed confirmation before removing an owned worktree", async () => {
  const preview: WorkspaceTeardownPreview = {
    operation_id: "operation", workspace_id: "space", endpoint_identity: "endpoint",
    repository_key: "repo", repository_root: "/repo", checkout_path: "/worktrees/task",
    ownership: "owned_created", workspace_state: "live", is_linked_worktree: true, dirty_state: "clean",
    allowed_actions: ["close_space", "remove_owned_worktree"], blockers: [], warnings: [],
    required_confirmation: "REMOVE /worktrees/task",
  };
  const result: WorkspaceTeardownResult = {
    operation_id: "operation", workspace_id: "space", action: "remove_owned_worktree",
    outcome: "completed", message: "Task worktree removed",
  };
  const client = {
    workspaceTeardownPreview: vi.fn(async () => preview),
    workspaceTeardownExecute: vi.fn(async () => result),
  };
  const view = mount(<TeardownDialog client={client} sessionId="session" workspaceId="space" open onClose={vi.fn()} onCompleted={vi.fn()} />);
  await act(async () => { await Promise.resolve(); });
  const action = [...view.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Remove task worktree")!;
  act(() => action.click());
  const confirm = [...view.querySelectorAll<HTMLButtonElement>("button")].filter((button) => button.textContent === "Remove task worktree").at(-1)!;
  expect(confirm.disabled).toBe(true);
  const input = view.querySelector<HTMLInputElement>("#teardown-confirmation")!;
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "REMOVE");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(confirm.disabled).toBe(true);
  expect(client.workspaceTeardownExecute).not.toHaveBeenCalled();
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, preview.required_confirmation!);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(confirm.disabled).toBe(false);
  await act(async () => confirm.click());
  expect(client.workspaceTeardownExecute).toHaveBeenCalledWith("session", {
    operation_id: "operation", workspace_id: "space", expected_endpoint_identity: "endpoint",
    expected_checkout_path: "/worktrees/task", action: "remove_owned_worktree", confirmation: "REMOVE /worktrees/task",
  });
});
