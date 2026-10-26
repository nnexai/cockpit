// @vitest-environment jsdom
import { act } from "react";
import type * as ReactTypes from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { TerminalPaneProps } from "../TerminalPane";
import { herdrBindings, herdrPrefixes, setEffectiveHerdrBindings } from "../input/herdrBindings";

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
  setEffectiveHerdrBindings([]);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function mount() {
  const opener = document.createElement("button");
  const area = document.createElement("div");
  area.className = "workarea-content";
  area.getBoundingClientRect = () => new DOMRect(100, 80, 800, 600);
  const host = document.createElement("div");
  document.body.append(opener, area, host);
  opener.focus();
  root = createRoot(host);
  const onDismiss = vi.fn();
  act(() => root!.render(<QuickTerminal client={{} as CockpitClient} sessionId="source-session" paneId="source-pane" focusEpoch={7} returnFocus={opener} suspended={false} onDismiss={onDismiss} />));
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
  it("opens a scratch pane with the source session and pane IDs", () => {
    const probe = mount();
    expect(mocks.pane).toHaveBeenLastCalledWith(expect.objectContaining({
      request: { session_id: "source-session", pane_id: "source-pane", target_kind: "scratch" },
      selected: true, presented: true, controlAllowed: true, controlPending: false,
      focusEpoch: 7, focusToken: 1, terminalMouseInput: true,
      accessibleLabel: "Quick terminal shell",
    }));
    expect(probe.dialog.style.left).toBe("180px");
    expect(probe.dialog.style.top).toBe("140px");
    expect(probe.dialog.style.width).toBe("640px");
    expect(probe.dialog.style.height).toBe("480px");
  });

  it("keeps the shell open when close is cancelled, then terminates before dismissing on acceptance", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValueOnce(true);
    const probe = mount();
    act(() => probe.close.click());
    expect(confirm).toHaveBeenCalledOnce();
    expect(mocks.terminate).not.toHaveBeenCalled();
    expect(probe.onDismiss).not.toHaveBeenCalled();
    act(() => probe.close.click());
    expect(mocks.terminate).toHaveBeenCalledOnce();
    expect(probe.onDismiss).not.toHaveBeenCalled();
    expect(probe.close.disabled).toBe(true);
    act(() => (mocks.pane.mock.lastCall![0] as TerminalPaneProps).onClosed!());
    expect(probe.onDismiss).toHaveBeenCalledOnce();
  });

  it("requires explicit dismissal when termination cannot be sent", () => {
    mocks.terminate.mockReturnValue(false);
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(true).mockReturnValueOnce(false).mockReturnValueOnce(true);
    const probe = mount();
    act(() => probe.close.click());
    expect(mocks.terminate).toHaveBeenCalledOnce();
    expect(probe.onDismiss).not.toHaveBeenCalled();
    expect(probe.dialog.querySelector('[role="alert"]')).not.toBeNull();
    act(() => probe.close.click());
    expect(probe.onDismiss).not.toHaveBeenCalled();
    act(() => probe.close.click());
    expect(confirm).toHaveBeenCalledTimes(3);
    expect(mocks.terminate).toHaveBeenCalledOnce();
    expect(probe.onDismiss).toHaveBeenCalledOnce();
  });

  it("offers explicit dismissal if an accepted termination loses confirmation", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const probe = mount();
    act(() => probe.close.click());
    expect(probe.close.disabled).toBe(true);
    act(() => (mocks.pane.mock.lastCall![0] as TerminalPaneProps).onStateChange!({ ready: false, error: "connection lost" }));
    expect(probe.close.disabled).toBe(false);
    expect(probe.dialog.querySelector('[role="alert"]')).not.toBeNull();
    expect(probe.onDismiss).not.toHaveBeenCalled();
    act(() => probe.close.click());
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(mocks.terminate).toHaveBeenCalledOnce();
    expect(probe.onDismiss).toHaveBeenCalledOnce();
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

  it("toggles closed with Ctrl-B U and restores focus after unmount", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const probe = mount();
    probe.terminal.focus();
    expect(probe.key(probe.terminal, { key: "b", ctrlKey: true }).defaultPrevented).toBe(true);
    expect(probe.key(probe.terminal, { key: "u" }).defaultPrevented).toBe(true);
    expect(confirm).toHaveBeenCalledOnce();
    expect(mocks.terminate).toHaveBeenCalledOnce();
    expect(probe.onDismiss).not.toHaveBeenCalled();
    act(() => (mocks.pane.mock.lastCall![0] as TerminalPaneProps).onClosed!());
    expect(probe.onDismiss).toHaveBeenCalledOnce();
    await act(async () => { root!.unmount(); root = null; });
    expect(document.activeElement).toBe(probe.opener);
  });

  it("passes a second Ctrl+B to the terminal and disarms the prefix", () => {
    const confirm = vi.spyOn(window, "confirm");
    const probe = mount();
    const received: string[] = [];
    probe.terminal.addEventListener("keydown", event => received.push(`${event.ctrlKey}:${event.key}`));
    expect(probe.key(probe.terminal, { key: "b", ctrlKey: true }).defaultPrevented).toBe(true);
    expect(probe.key(probe.terminal, { key: "b", ctrlKey: true }).defaultPrevented).toBe(false);
    expect(received).toEqual(["true:b"]);
    expect(probe.key(probe.terminal, { key: "u" }).defaultPrevented).toBe(false);
    expect(confirm).not.toHaveBeenCalled();
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

  it("does not intercept the toggle prefix when Herdr owns it", () => {
    const command = { command_id: "prefix:u", binding_labels: ["prefix+u"], action: "shell" as const, description: "Configured action" };
    setEffectiveHerdrBindings(herdrBindings([command]), herdrPrefixes(["ctrl+b"]));
    const confirm = vi.spyOn(window, "confirm");
    const probe = mount();
    expect(probe.key(probe.terminal, { key: "b", ctrlKey: true }).defaultPrevented).toBe(false);
    expect(probe.key(probe.terminal, { key: "u" }).defaultPrevented).toBe(false);
    expect(confirm).not.toHaveBeenCalled();
    expect(probe.onDismiss).not.toHaveBeenCalled();
  });
});
