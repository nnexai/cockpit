// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
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
