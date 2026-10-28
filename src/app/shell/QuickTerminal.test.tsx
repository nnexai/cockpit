// @vitest-environment jsdom
import { act } from "react";
import type * as ReactTypes from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { TerminalPaneProps } from "../TerminalPane";

const mocks = vi.hoisted(() => ({
  pane: vi.fn(),
  terminate: vi.fn(),
}));

vi.mock("../TerminalPane", async () => {
  const React = await vi.importActual<typeof ReactTypes>("react");
  return {
    TerminalPane: (props: TerminalPaneProps) => {
      mocks.pane(props);
      React.useEffect(() => {
        if (props.terminateRef) props.terminateRef.current = mocks.terminate;
        return () => { if (props.terminateRef) props.terminateRef.current = null; };
      }, [props.terminateRef]);
      return React.createElement("textarea", { className: "xterm-helper-textarea", "aria-label": props.accessibleLabel });
    },
  };
});

import { QuickTerminal } from "./QuickTerminal";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
let root: Root | null = null;

beforeEach(() => {
  mocks.terminate.mockReturnValue(true);
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    disconnect() {}
  });
});

afterEach(async () => {
  if (root) await act(async () => root?.unmount());
  root = null;
  document.body.replaceChildren();
  mocks.pane.mockReset();
  mocks.terminate.mockReset();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function mount({ cwd, rect = new DOMRect(100, 80, 800, 600) }: { cwd?: string; rect?: DOMRect } = {}) {
  const opener = document.createElement("button");
  const area = document.createElement("div");
  area.className = "workarea-content";
  area.getBoundingClientRect = () => rect;
  const host = document.createElement("div");
  document.body.append(opener, area, host);
  opener.focus();
  root = createRoot(host);
  const onDismiss = vi.fn();
  act(() => root!.render(<QuickTerminal client={{} as CockpitClient} sessionId="source-session" paneId="source-pane" cwd={cwd} focusEpoch={7} returnFocus={opener} suspended={false} onDismiss={onDismiss} />));
  const dialog = host.querySelector<HTMLElement>('[role="dialog"]')!;
  const terminal = host.querySelector<HTMLTextAreaElement>(".xterm-helper-textarea")!;
  const close = host.querySelector<HTMLButtonElement>('button[aria-label="Close quick terminal"]')!;
  const key = (target: HTMLElement, init: KeyboardEventInit) => {
    const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
    act(() => { target.dispatchEvent(event); });
    return event;
  };
  return { opener, dialog, terminal, close, onDismiss, key };
}

describe("QuickTerminal", () => {
  it("shows the source cwd and available shortcut without guessing an absent cwd", () => {
    const probe = mount({ cwd: "/home/user/project" });
    expect(probe.dialog.querySelector(".quick-terminal-cwd")?.getAttribute("title")).toBe("/home/user/project");
    expect(probe.dialog.querySelector(".quick-terminal-key")?.textContent).toBe("Ctrl+`");
  });

  it("uses a generic label when the source does not report a cwd", () => {
    const label = mount().dialog.querySelector(".quick-terminal-cwd");
    expect(label?.textContent).toBe("Quick terminal");
    expect(label?.hasAttribute("title")).toBe(false);
  });

  it.each([true, false])("immediately dismisses on close after attempting termination (sent: %s)", (sent) => {
    mocks.terminate.mockReturnValue(sent);
    const confirm = vi.spyOn(window, "confirm");
    const probe = mount();
    act(() => probe.close.click());
    expect(confirm).not.toHaveBeenCalled();
    expect(mocks.terminate).toHaveBeenCalledOnce();
    expect(probe.onDismiss).toHaveBeenCalledOnce();
    expect(mocks.terminate.mock.invocationCallOrder[0]).toBeLessThan(probe.onDismiss.mock.invocationCallOrder[0]);
  });

  it("dismisses on shell exit without confirming or terminating again", () => {
    const confirm = vi.spyOn(window, "confirm");
    const probe = mount();
    const props = mocks.pane.mock.lastCall![0] as TerminalPaneProps;
    act(() => props.onClosed!());
    expect(probe.onDismiss).toHaveBeenCalledOnce();
    expect(confirm).not.toHaveBeenCalled();
    expect(mocks.terminate).not.toHaveBeenCalled();
  });

  it("toggles closed with Ctrl+Backquote and restores focus after unmount", async () => {
    const confirm = vi.spyOn(window, "confirm");
    const probe = mount();
    probe.terminal.focus();
    expect(probe.key(probe.terminal, { key: "`", code: "Backquote", ctrlKey: true }).defaultPrevented).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
    expect(mocks.terminate).toHaveBeenCalledOnce();
    expect(probe.onDismiss).toHaveBeenCalledOnce();
    await act(async () => { root!.unmount(); root = null; });
    expect(document.activeElement).toBe(probe.opener);
  });

  it("leaves Tab from the terminal available to the shell while cycling focus from the close button", () => {
    const probe = mount();
    probe.terminal.focus();
    const tab = probe.key(probe.terminal, { key: "Tab" });
    expect(tab.defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(probe.terminal);
    probe.close.focus();
    expect(probe.key(probe.close, { key: "Tab" }).defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(probe.terminal);
  });

});
