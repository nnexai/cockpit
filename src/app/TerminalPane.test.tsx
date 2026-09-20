// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient, TerminalStream } from "../client/CockpitClient";
import type { TerminalCommand, TerminalOpenRequest, TerminalOwnershipState, TerminalStreamMessage } from "../protocol/generated/v1";
import { copyTerminalSelection, createCockpitTerminal, readTerminalClipboard, TerminalPane } from "./TerminalPane";
const mocks = vi.hoisted(() => {
  const terminals: MockTerminal[] = [];
  const fits: MockFitAddon[] = [];
  const observers: MockResizeObserver[] = [];
  const fitDimensions: Array<[number, number]> = [];

  class MockTerminal {
    cols = 80;
    rows = 24;
    options: Record<string, unknown>;
    element: HTMLElement | null = null;
    private resizeListeners: Array<(size: { cols: number; rows: number }) => void> = [];
    private renderListeners: Array<() => void> = [];
    dataHandler: ((data: string) => void) | null = null;
    readonly focus = vi.fn();
    readonly dispose = vi.fn();
    readonly write = vi.fn((_data: Uint8Array, done?: () => void) => { done?.(); });
    readonly onRender = vi.fn((listener: () => void) => {
      this.renderListeners.push(listener);
      return { dispose: () => { this.renderListeners = this.renderListeners.filter((entry) => entry !== listener); } };
    });
    readonly refresh = vi.fn((_start: number, _end: number) => this.emitRender());
    emitRender() { this.renderListeners.forEach((listener) => listener()); }
    readonly paste = vi.fn((_data: string) => {});
    readonly hasSelection = vi.fn(() => true);
    readonly getSelection = vi.fn(() => "selected");
    readonly onData = vi.fn((listener: (data: string) => void) => {
      this.dataHandler = listener;
      return { dispose: vi.fn() };
    });
    readonly onBinary = vi.fn(() => ({ dispose: vi.fn() }));
    readonly onResize = vi.fn((listener: (size: { cols: number; rows: number }) => void) => {
      this.resizeListeners.push(listener);
      return { dispose: vi.fn(() => { this.resizeListeners = this.resizeListeners.filter((entry) => entry !== listener); }) };
    });
    keyHandler: ((event: KeyboardEvent) => boolean) | null = null;
    readonly attachCustomKeyEventHandler = vi.fn((handler: (event: KeyboardEvent) => boolean) => { this.keyHandler = handler; });
    readonly attachCustomWheelEventHandler = vi.fn();

    constructor(options: Record<string, unknown> = {}) {
      this.options = { fontSize: 14, ...options };
      terminals.push(this);
    }

    open(host: HTMLElement) {
      this.element = document.createElement("div");
      const screen = document.createElement("div");
      screen.className = "xterm-screen";
      screen.getBoundingClientRect = () => ({ left: 0, top: 0, width: 800, height: 400, right: 800, bottom: 400, x: 0, y: 0, toJSON: () => ({}) });
      this.element.append(screen);
      host.append(this.element);
    }

    loadAddon(addon: MockFitAddon) {
      addon.terminal = this;
    }

    emitResize() {
      this.resizeListeners.forEach((listener) => listener({ cols: this.cols, rows: this.rows }));
    }
  }

  class MockFitAddon {
    terminal: MockTerminal | null = null;
    readonly fit = vi.fn(() => {
      const [cols, rows] = fitDimensions.shift() ?? [this.terminal?.cols ?? 80, this.terminal?.rows ?? 24];
      if (!this.terminal) return;
      this.terminal.cols = cols;
      this.terminal.rows = rows;
      this.terminal.emitResize();
    });

    constructor() {
      fits.push(this);
    }
  }

  class MockResizeObserver {
    readonly callback: ResizeObserverCallback;
    readonly observe = vi.fn();
    readonly disconnect = vi.fn();

    constructor(callback: ResizeObserverCallback) {
      this.callback = callback;
      observers.push(this);
    }
  }

  return { terminals, fits, observers, fitDimensions, MockTerminal, MockFitAddon, MockResizeObserver };
});

vi.mock("@xterm/xterm", () => ({ Terminal: mocks.MockTerminal }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: mocks.MockFitAddon }));
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const request = {
  session_id: "session",
  pane_id: "pane",
  mode: "control",
  takeover: false,
  cols: 80,
  rows: 24,
  cell_width_px: 10,
  cell_height_px: 16,
} as TerminalOpenRequest;

function stream(sent: TerminalCommand[]): TerminalStream {
  return { send: (command) => sent.push(command), close: vi.fn() };
}

function message(state: TerminalOwnershipState): TerminalStreamMessage {
  return { type: "ownership", session_id: "session", pane_id: "pane", stream_id: "stream", state, message: null };
}

function mouseMode(enabled: boolean, streamId = "stream"): TerminalStreamMessage {
  return { type: "mouse_mode", session_id: "session", pane_id: "pane", stream_id: streamId, enabled };
}

function makeClient(sent: TerminalCommand[], onMessage: Array<(value: TerminalStreamMessage) => void>) {
  const openTerminal = vi.fn((_request: TerminalOpenRequest, receive: (value: TerminalStreamMessage) => void) => {
    onMessage.push(receive);
    return Promise.resolve(stream(sent));
  });
  return {
    client: { openTerminal } as unknown as CockpitClient,
    openTerminal,
  };
}

async function settle() {
  await Promise.resolve();
  await Promise.resolve();
}

function pointer(type: string, timeStamp: number, button = 0, options: { pointerId?: number; shiftKey?: boolean } = {}) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperties(event, {
    button: { value: button },
    clientX: { value: 120 },
    clientY: { value: 80 },
    pointerId: { value: options.pointerId ?? 1 },
    shiftKey: { value: options.shiftKey ?? false },
    ctrlKey: { value: false },
    altKey: { value: false },
    metaKey: { value: false },
    timeStamp: { value: timeStamp },
  });
  return event;
}

function paneProps(client: CockpitClient, terminalMouseInput: boolean, overrides: Partial<Parameters<typeof TerminalPane>[0]> = {}) {
  return {
    client,
    request,
    selected: true,
    controlAllowed: true,
    controlPending: false,
    focusTransitionPending: false,
    focusEpoch: 1,
    focusToken: 0,
    terminalMouseInput,
    ...overrides,
  };
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  mocks.terminals.length = 0;
  mocks.fits.length = 0;
  mocks.observers.length = 0;
  mocks.fitDimensions.length = 0;
});

describe("TerminalPane fitting and pointer ownership", () => {
  it("opens the required terminal stream without waiting for focus confirmation", async () => {
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client, openTerminal } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false)} />);
        await settle();
      });
      expect(openTerminal).toHaveBeenCalledOnce();
      expect(openTerminal.mock.calls[0]?.[0].mode).toBe("control");
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("waits to attach a hidden incoming tab terminal", async () => {
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client, openTerminal } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false, { deferAttachment: true })} />);
        await settle();
      });
      expect(openTerminal).not.toHaveBeenCalled();

      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false, { deferAttachment: false })} />);
        await settle();
      });
      expect(openTerminal).toHaveBeenCalledOnce();
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });
  it("closes an outgoing stream without reopening a hidden observer, then reattaches input on revisit", async () => {
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client, openTerminal } = makeClient(sent, messages);
    const opened: Array<{ commands: TerminalCommand[]; terminal: TerminalStream }> = [];
    openTerminal.mockImplementation((_request: TerminalOpenRequest, receive: (value: TerminalStreamMessage) => void) => {
      const commands: TerminalCommand[] = [];
      const terminal = stream(commands);
      opened.push({ commands, terminal });
      messages.push(receive);
      return Promise.resolve(terminal);
    });
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false)} />);
        await settle();
      });
      expect(openTerminal).toHaveBeenCalledOnce();

      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false, { selected: false, controlAllowed: false, deferAttachment: true })} />);
        await settle();
      });
      expect(opened[0].terminal.close).toHaveBeenCalledOnce();
      expect(openTerminal).toHaveBeenCalledOnce();
      mocks.terminals.at(-1)?.dataHandler?.("hidden input");
      expect(opened[0].commands).toEqual([]);

      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false, { deferAttachment: false })} />);
        await settle();
      });
      expect(openTerminal.mock.calls.at(-1)?.[0].mode).toBe("control");
      for (const retired of opened.slice(0, -1)) expect(retired.terminal.close).toHaveBeenCalledOnce();
      act(() => messages.at(-1)?.(message("owned")));
      mocks.terminals.at(-1)?.dataHandler?.("revisited");
      expect(opened.at(-1)?.commands).toEqual([{ type: "terminal.input", text: "revisited", bytes: null }]);
      for (const retired of opened.slice(0, -1)) expect(retired.commands).toEqual([]);
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });
  it("waits for a rendered full frame and ignores retired write completion", async () => {
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient([], messages);
    const ready = vi.fn();
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false, { onReady: ready })} />); await settle(); });
      expect(ready).not.toHaveBeenCalled();
      const writes: Array<() => void> = [];
      mocks.terminals.at(-1)!.refresh.mockImplementation(() => undefined);
      mocks.terminals.at(-1)!.write.mockImplementation((_text, done) => { if (done) writes.push(done); });
      const frame: TerminalStreamMessage = { type: "frame", session_id: "session", pane_id: "pane", stream_id: "stream", seq: "1", encoding: "ansi", width: 80, height: 24, full: true, bytes: btoa("current contents") };
      act(() => messages[0](frame));
      expect(ready).not.toHaveBeenCalled();
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false, { onReady: ready, deferAttachment: true })} />); });
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false, { onReady: ready })} />); await settle(); });
      act(() => writes[0]());
      expect(ready).not.toHaveBeenCalled();
      act(() => messages.at(-1)!(frame));
      act(() => writes[1]());
      expect(ready).not.toHaveBeenCalled();
      act(() => mocks.terminals.at(-1)!.emitRender());
      expect(ready).toHaveBeenCalledOnce();
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });
  it("restores terminal focus only when a frame leaves focus on the document", async () => {
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false)} />);
        await settle();
      });
      const terminal = mocks.terminals[0]!;
      terminal.focus.mockClear();
      const input = document.createElement("textarea");
      terminal.element!.append(input);
      input.focus();
      terminal.write.mockImplementationOnce((_data, done) => { input.remove(); done?.(); });
      act(() => messages[0]!({ type: "frame", session_id: "session", pane_id: "pane", stream_id: "stream", seq: "1", encoding: "utf8", width: 80, height: 24, full: true, bytes: btoa("frame") }));
      expect(terminal.focus).toHaveBeenCalledOnce();
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("copies the selected terminal text through the user clipboard gesture", async () => {
    const writeText = vi.fn(async (_text: string) => undefined);
    expect(await copyTerminalSelection({ getSelection: () => "α\tline\n二" }, { readText: vi.fn(), writeText })).toBe(true);
    expect(writeText).toHaveBeenCalledWith("α\tline\n二");
    expect(await copyTerminalSelection({ getSelection: () => "" }, { readText: vi.fn(), writeText })).toBe(false);
  });

  it("reads paste text only through the explicit clipboard operation", async () => {
    const readText = vi.fn(async () => "line one\nline two\n✓");
    expect(await readTerminalClipboard({ readText, writeText: vi.fn() })).toBe("line one\nline two\n✓");
    expect(readText).toHaveBeenCalledTimes(1);
  });

  it("anchors the clipboard menu at the right-click coordinates", async () => {
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false)} />);
        await settle();
      });
      const terminalHost = host.querySelector<HTMLElement>(".terminal-host")!;
      const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 123, clientY: 87 });
      await act(async () => {
        terminalHost.dispatchEvent(event);
        await settle();
      });
      const menu = host.querySelector<HTMLElement>(".terminal-context-menu");
      expect(event.defaultPrevented).toBe(true);
      expect(menu?.style.left).toBe("123px");
      expect(menu?.style.top).toBe("87px");
      expect(menu?.querySelector<HTMLButtonElement>('button[role="menuitem"]')?.disabled).toBe(false);
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("holds paste until the requested pane owns control", async () => {
    const readText = vi.fn(async () => "line one\nline two\n✓");
    vi.stubGlobal("navigator", { clipboard: { readText, writeText: vi.fn() } });
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false, { selected: false, controlAllowed: false })} />);
        await settle();
      });
      const terminal = mocks.terminals[0];
      expect(terminal?.keyHandler).not.toBeNull();
      await act(async () => {
        terminal?.keyHandler?.(new KeyboardEvent("keydown", { key: "v", ctrlKey: true, shiftKey: true, cancelable: true }));
        await settle();
      });
      expect(terminal?.paste).not.toHaveBeenCalled();

      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false, { selected: true, controlAllowed: false, controlPending: true, focusToken: 1 })} />);
        await settle();
      });
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false, { selected: true, controlAllowed: true, controlPending: false, focusToken: 1 })} />);
        await settle();
      });
      await act(async () => {
        messages.at(-1)?.(message("owned"));
        await settle();
      });
      expect(terminal?.paste).toHaveBeenCalledOnce();
      expect(terminal?.paste).toHaveBeenCalledWith("line one\nline two\n✓");
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("uses the final DOM renderer metrics and visible scrollbar options", () => {
    const terminal = createCockpitTerminal(16);
    expect(terminal.options).toMatchObject({
      fontFamily: 'ui-monospace, "FiraCode Nerd Font Mono", "Hack Nerd Font Mono", "IBM Plex Mono", "Noto Sans Mono", monospace',
      fontSize: 16,
      lineHeight: 1,
      scrollbar: { showScrollbar: false, width: 8 },
    });
  });

  it("re-fits after delayed attachment and sends one changed authoritative resize", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    mocks.fitDimensions.push([100, 30], [120, 40]);
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client, openTerminal } = makeClient(sent, messages);
    let resolveOpened: ((value: TerminalStream) => void) | null = null;
    const delayedOpen = new Promise<TerminalStream>((resolve) => { resolveOpened = resolve; });
    openTerminal.mockImplementationOnce((_request: TerminalOpenRequest, receive: (value: TerminalStreamMessage) => void) => {
      messages.push(receive);
      return delayedOpen;
    });
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false)} />);
        await settle();
      });
      expect(openTerminal.mock.calls[0]?.[0]).toMatchObject({ cols: 100, rows: 30, cell_width_px: 8, cell_height_px: 13 });
      expect(sent.filter((command) => command.type === "terminal.resize")).toEqual([]);

      await act(async () => {
        resolveOpened!(stream(sent));
        await settle();
      });
      expect(sent.filter((command): command is Extract<TerminalCommand, { type: "terminal.resize" }> => command.type === "terminal.resize")).toEqual([
        { type: "terminal.resize", cols: 120, rows: 40, cell_width_px: 7, cell_height_px: 10 },
      ]);

      mocks.observers[0].callback([], mocks.observers[0] as unknown as ResizeObserver);
      await vi.advanceTimersByTimeAsync(100);
      expect(sent.filter((command) => command.type === "terminal.resize")).toHaveLength(1);
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("debounces resize fitting and cancels a stale callback across remount", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    mocks.fitDimensions.push([80, 24], [80, 24], [120, 40], [100, 30]);
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client, openTerminal } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    let rootUnmounted = false;
    let secondRoot: Root | null = null;
    let secondRootUnmounted = false;
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false)} />);
        await settle();
      });
      expect(openTerminal.mock.calls[0]?.[0]).toMatchObject({ cols: 80, rows: 24 });
      const firstObserver = mocks.observers[0];
      firstObserver.callback([], firstObserver as unknown as ResizeObserver);
      await vi.advanceTimersByTimeAsync(50);
      firstObserver.callback([], firstObserver as unknown as ResizeObserver);
      expect(sent.filter((command) => command.type === "terminal.resize")).toEqual([]);
      await vi.advanceTimersByTimeAsync(99);
      expect(sent.filter((command) => command.type === "terminal.resize")).toEqual([]);
      await vi.advanceTimersByTimeAsync(1);
      expect(sent.filter((command): command is Extract<TerminalCommand, { type: "terminal.resize" }> => command.type === "terminal.resize")).toEqual([
        { type: "terminal.resize", cols: 120, rows: 40, cell_width_px: 7, cell_height_px: 10 },
      ]);

      firstObserver.callback([], firstObserver as unknown as ResizeObserver);
      await act(async () => root.unmount());
      rootUnmounted = true;
      expect(vi.getTimerCount()).toBe(0);
      await vi.advanceTimersByTimeAsync(100);
      expect(sent.filter((command) => command.type === "terminal.resize")).toHaveLength(1);

      secondRoot = createRoot(host);
      await act(async () => {
        secondRoot!.render(<TerminalPane {...paneProps(client, false)} />);
        await settle();
      });
      expect(openTerminal).toHaveBeenCalledTimes(2);
      expect(openTerminal.mock.calls[1]?.[0]).toMatchObject({ cols: 100, rows: 30 });
    } finally {
      if (!rootUnmounted) await act(async () => root.unmount());
      if (secondRoot && !secondRootUnmounted) {
        await act(async () => secondRoot!.unmount());
        secondRootUnmounted = true;
      }
      host.remove();
    }
  });

  it("does not read geometry for idle motion, but preserves controlled drag and release", async () => {
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    const pointerCaptureNames = ["setPointerCapture", "releasePointerCapture", "hasPointerCapture"] as const;
    const originalPointerCapture = Object.fromEntries(pointerCaptureNames.map((name) => [name, Object.getOwnPropertyDescriptor(HTMLElement.prototype, name)]));
    Object.assign(HTMLElement.prototype, {
      setPointerCapture: vi.fn(),
      releasePointerCapture: vi.fn(),
      hasPointerCapture: vi.fn(() => true),
    });
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    let rootUnmounted = false;
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, true)} />);
        await settle();
      });
      await act(async () => {
        messages[0](mouseMode(true));
        messages[0](message("owned"));
      });
      const screen = host.querySelector(".xterm-screen")!;
      const geometry = vi.spyOn(screen, "getBoundingClientRect");
      const terminalHost = host.querySelector<HTMLElement>(".terminal-host")!;
      terminalHost.dispatchEvent(pointer("pointermove", 100));
      expect(geometry).not.toHaveBeenCalled();
      expect(sent).toEqual([]);
      await act(async () => {
        terminalHost.dispatchEvent(pointer("pointerdown", 10));
        terminalHost.dispatchEvent(pointer("pointermove", 20));
        terminalHost.dispatchEvent(pointer("pointermove", 25));
        terminalHost.dispatchEvent(pointer("pointermove", 40));
        terminalHost.dispatchEvent(pointer("pointerup", 50));
        terminalHost.dispatchEvent(pointer("pointerdown", 60));
        terminalHost.dispatchEvent(pointer("pointercancel", 70));
      });
      expect(sent.filter((command): command is Extract<TerminalCommand, { type: "terminal.mouse" }> => command.type === "terminal.mouse").map((command) => command.kind)).toEqual(["down", "drag", "drag", "up", "down", "up"]);
      await act(async () => root.unmount());
      rootUnmounted = true;
    } finally {
      if (!rootUnmounted) await act(async () => root.unmount());
      for (const name of pointerCaptureNames) {
        const descriptor = originalPointerCapture[name];
        if (descriptor) Object.defineProperty(HTMLElement.prototype, name, descriptor);
        else delete (HTMLElement.prototype as Partial<Record<typeof name, unknown>>)[name];
      }
      host.remove();
    }
  });
  it("requires authoritative current-stream mode and restores xterm selection when disabled", async () => {
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    const pointerCaptureNames = ["setPointerCapture", "releasePointerCapture", "hasPointerCapture"] as const;
    const originalPointerCapture = Object.fromEntries(pointerCaptureNames.map((name) => [name, Object.getOwnPropertyDescriptor(HTMLElement.prototype, name)]));
    Object.assign(HTMLElement.prototype, {
      setPointerCapture: vi.fn(),
      releasePointerCapture: vi.fn(),
      hasPointerCapture: vi.fn(() => true),
    });
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    let rootUnmounted = false;
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, true)} />);
        await settle();
      });
      await act(async () => {
        messages[0](message("owned"));
      });
      const terminalHost = host.querySelector<HTMLElement>(".terminal-host")!;
      const shellSelection = pointer("pointerdown", 10);
      terminalHost.dispatchEvent(shellSelection);
      expect(shellSelection.defaultPrevented).toBe(false);
      expect(sent).toEqual([]);

      await act(async () => { messages[0](mouseMode(true)); });
      const shiftSelection = pointer("pointerdown", 20, 0, { shiftKey: true });
      terminalHost.dispatchEvent(shiftSelection);
      expect(shiftSelection.defaultPrevented).toBe(false);
      expect(sent).toEqual([]);

      const appPointer = pointer("pointerdown", 30);
      terminalHost.dispatchEvent(appPointer);
      expect(appPointer.defaultPrevented).toBe(true);
      expect(sent.filter((command) => command.type === "terminal.mouse").map((command) => command.kind)).toEqual(["down"]);

      await act(async () => { messages[0](mouseMode(false)); });
      expect(HTMLElement.prototype.releasePointerCapture).toHaveBeenCalledWith(1);
      const restoredSelection = pointer("pointerdown", 40);
      terminalHost.dispatchEvent(restoredSelection);
      expect(restoredSelection.defaultPrevented).toBe(false);
      expect(sent.filter((command) => command.type === "terminal.mouse").map((command) => command.kind)).toEqual(["down"]);
    } finally {
      if (!rootUnmounted) await act(async () => root.unmount());
      for (const name of pointerCaptureNames) {
        const descriptor = originalPointerCapture[name];
        if (descriptor) Object.defineProperty(HTMLElement.prototype, name, descriptor);
        else delete (HTMLElement.prototype as Partial<Record<typeof name, unknown>>)[name];
      }
      host.remove();
    }
  });

  it("ignores a stale mode event after reconnect and clears pending gestures on ownership loss", async () => {
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    const pointerCaptureNames = ["setPointerCapture", "releasePointerCapture", "hasPointerCapture"] as const;
    const originalPointerCapture = Object.fromEntries(pointerCaptureNames.map((name) => [name, Object.getOwnPropertyDescriptor(HTMLElement.prototype, name)]));
    Object.assign(HTMLElement.prototype, {
      setPointerCapture: vi.fn(),
      releasePointerCapture: vi.fn(),
      hasPointerCapture: vi.fn(() => true),
    });
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const firstRoot = createRoot(host);
    try {
      await act(async () => {
        firstRoot.render(<TerminalPane {...paneProps(client, true, { controlAllowed: false, controlPending: true })} />);
        await settle();
      });
      const staleMessage = messages[0];
      await act(async () => firstRoot.unmount());

      const secondRoot = createRoot(host);
      try {
        await act(async () => {
          secondRoot.render(<TerminalPane {...paneProps(client, true, { controlAllowed: false, controlPending: true })} />);
          await settle();
        });
        staleMessage(mouseMode(true));
        const stalePointer = pointer("pointerdown", 10);
        host.querySelector<HTMLElement>(".terminal-host")!.dispatchEvent(stalePointer);
        expect(stalePointer.defaultPrevented).toBe(false);
        expect(sent).toEqual([]);

        await act(async () => {
          messages[1](mouseMode(true));
          messages[1](message("observing"));
        });
        const pendingPointer = pointer("pointerdown", 20);
        host.querySelector<HTMLElement>(".terminal-host")!.dispatchEvent(pendingPointer);
        expect(pendingPointer.defaultPrevented).toBe(true);
        await act(async () => { messages[1](message("lost")); });
        expect(HTMLElement.prototype.releasePointerCapture).toHaveBeenCalledWith(1);
        await act(async () => { messages[1](message("owned")); });
        expect(sent.filter((command) => command.type === "terminal.mouse")).toEqual([]);
      } finally {
        await act(async () => secondRoot.unmount());
      }
    } finally {
      for (const name of pointerCaptureNames) {
        const descriptor = originalPointerCapture[name];
        if (descriptor) Object.defineProperty(HTMLElement.prototype, name, descriptor);
        else delete (HTMLElement.prototype as Partial<Record<typeof name, unknown>>)[name];
      }
      host.remove();
    }
  });
  it("observes after a control conflict and only takes over on a new local click", async () => {
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client, openTerminal } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => { root.render(<TerminalPane {...paneProps(client, true)} />); });
      await act(async () => { messages[0](message("conflict")); });
      expect(openTerminal.mock.calls.map(([request]) => request.mode)).toEqual(["control", "observe"]);
      await act(async () => { messages[1](message("observing")); });
      expect(openTerminal.mock.calls.map(([request]) => request.mode)).toEqual(["control", "observe"]);
      await act(async () => {
        messages[1]({ type: "frame", session_id: "session", pane_id: "pane", stream_id: "stream", seq: "1", encoding: "ansi", width: 80, height: 24, full: true, bytes: btoa("observer output") });
      });
      expect(new TextDecoder().decode(mocks.terminals.at(-1)!.write.mock.calls[0][0])).toBe("observer output");
      await act(async () => { host.querySelector(".terminal-host")!.dispatchEvent(pointer("pointerdown", 10)); });
      expect(openTerminal.mock.calls.at(-1)![0]).toMatchObject({ mode: "control", takeover: true });
      expect(openTerminal.mock.calls.map(([request]) => request.mode)).toEqual(["control", "observe", "control"]);
      expect(sent).toEqual([]);
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("keeps the first character typed during focus confirmation for that pane intent", async () => {
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, true, { controlAllowed: false, controlPending: true, focusToken: 7 })} />);
        await settle();
      });
      const onData = mocks.terminals.at(-1)!.onData.mock.calls[0][0] as (data: string) => void;
      onData("A");

      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, true, { controlAllowed: true, controlPending: false, focusToken: 7 })} />);
        await settle();
      });
      await act(async () => { messages.at(-1)!(message("owned")); });
      expect(sent.filter((command) => command.type === "terminal.input")).toEqual([{ type: "terminal.input", text: "A", bytes: null }]);
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("discards input queued during a rejected control request", async () => {
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => { root.render(<TerminalPane {...paneProps(client, true, { controlAllowed: false, controlPending: true })} />); });
      await act(async () => { host.querySelector(".terminal-host")!.dispatchEvent(pointer("pointerdown", 10)); });
      const onData = mocks.terminals.at(-1)!.onData.mock.calls[0][0] as (data: string) => void;
      onData("stale input");
      await act(async () => { root.render(<TerminalPane {...paneProps(client, true, { controlAllowed: false, controlPending: false })} />); });
      await act(async () => { root.render(<TerminalPane {...paneProps(client, true)} />); });
      await act(async () => { messages.at(-1)!(message("owned")); });
      expect(sent).toEqual([]);
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });
});
