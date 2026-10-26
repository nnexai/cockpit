// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient, TerminalStream } from "../client/CockpitClient";
import type { TerminalCommand, TerminalOpenRequest, TerminalOwnershipState, TerminalStreamMessage } from "../protocol/generated/v1";
import { TerminalPane } from "./TerminalPane";
import { copyTerminalSelection, readTerminalClipboard } from "./terminal/terminalClipboard";
import { createCockpitTerminal } from "./terminal/cockpitTerminal";
const mocks = vi.hoisted(() => {
  const terminals: MockTerminal[] = [];
  const fits: MockFitAddon[] = [];
  const observers: MockResizeObserver[] = [];
  const fitDimensions: Array<[number, number]> = [];
  const fitProposals: Array<[number, number]> = [];
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
    readonly attachCustomWheelEventHandler = vi.fn((_handler: (event: WheelEvent) => boolean) => {});

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

    readonly resize = vi.fn((cols: number, rows: number) => {
      this.cols = cols;
      this.rows = rows;
      this.emitResize();
    });
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
    readonly proposeDimensions = vi.fn(() => {
      const proposal = fitProposals.shift();
      return proposal ? { cols: proposal[0], rows: proposal[1] } : this.terminal ? { cols: this.terminal.cols, rows: this.terminal.rows } : undefined;
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

  return { terminals, fits, observers, fitDimensions, fitProposals, MockTerminal, MockFitAddon, MockResizeObserver, native: { enabled: false, invoke: vi.fn() } };
});

vi.mock("@xterm/xterm", () => ({ Terminal: mocks.MockTerminal }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: mocks.MockFitAddon }));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class {} }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => mocks.native.enabled, invoke: mocks.native.invoke }));
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
  mocks.fitProposals.length = 0;
  mocks.native.enabled = false;
  mocks.native.invoke.mockReset();
});

describe("scratch terminal", () => {
  it("renders incremental ANSI, uses xterm input and local sizing, and terminates", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client, openTerminal } = makeClient(sent, messages);
    const terminateRef: { current: (() => boolean) | null } = { current: null };
    const onClosed = vi.fn();
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, true, {
          request: { session_id: "session", pane_id: "pane", target_kind: "scratch" },
          terminateRef, onClosed,
        })} />);
        await settle();
      });
      const terminal = mocks.terminals.at(-1)!;
      await act(async () => {
        messages[0]!(message("owned"));
        messages[0]!({ type: "frame", session_id: "session", pane_id: "pane", stream_id: "stream",
          seq: "1", encoding: "ansi", width: 80, height: 24, full: false, bytes: btoa("\x1b[?1000hhello") });
      });
      expect(terminal.write).toHaveBeenCalledWith(
        Uint8Array.from("\x1b[?1000hhello", (character) => character.charCodeAt(0)), expect.any(Function),
      );
      expect(terminal.resize).not.toHaveBeenCalled();
      expect(host.querySelector('[role="alert"]')).toBeNull();

      const down = pointer("pointerdown", 1);
      host.querySelector(".terminal-host")!.dispatchEvent(down);
      expect(down.defaultPrevented).toBe(false);
      const wheel = new WheelEvent("wheel", { deltaY: 10, cancelable: true });
      expect(terminal.attachCustomWheelEventHandler.mock.calls[0]![0](wheel)).toBe(true);
      terminal.dataHandler?.("\x1b[<0;2;3M");
      expect(sent).toContainEqual({ type: "terminal.input", text: "\x1b[<0;2;3M", bytes: null });
      expect(sent.some((command) => command.type === "terminal.mouse" || command.type === "terminal.scroll")).toBe(false);

      mocks.fitProposals.push([120, 40]);
      mocks.fitDimensions.push([120, 40]);
      await act(async () => {
        mocks.observers[0]!.callback([], mocks.observers[0] as unknown as ResizeObserver);
        await vi.advanceTimersByTimeAsync(100);
      });
      expect(terminal.cols).toBe(120);
      expect(terminal.rows).toBe(40);
      expect(sent).toContainEqual(expect.objectContaining({ type: "terminal.resize", cols: 120, rows: 40 }));
      await act(async () => { expect(terminateRef.current?.()).toBe(true); });
      expect(sent).toContainEqual({ type: "terminal.terminate" });
      await act(async () => {
        messages[0]!({ type: "closed", session_id: "session", pane_id: "pane", stream_id: "stream", reason: "exited" });
      });
      expect(onClosed).toHaveBeenCalledOnce();
      expect(openTerminal).toHaveBeenCalledOnce();
    } finally {
      await act(async () => root.unmount());
      expect(terminateRef.current).toBeNull();
      host.remove();
    }
  });

  it("reports unavailable streams and synchronous termination failures", async () => {
    const terminateRef: { current: (() => boolean) | null } = { current: null };
    const onStateChange = vi.fn();
    const send = vi.fn((command: TerminalCommand) => {
      if (command.type === "terminal.terminate") throw new Error("Termination send failed");
    });
    const client = { openTerminal: vi.fn(() => Promise.resolve({ send, close: vi.fn() })) } as unknown as CockpitClient;
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    const props = paneProps(client, false, {
      request: { session_id: "session", pane_id: "pane", target_kind: "scratch" },
      terminateRef, onStateChange,
    });
    try {
      await act(async () => { root.render(<TerminalPane {...props} deferAttachment />); });
      expect(terminateRef.current?.()).toBe(false);
      expect(send).not.toHaveBeenCalled();
      await act(async () => { root.render(<TerminalPane {...props} />); await settle(); });
      await act(async () => { expect(terminateRef.current?.()).toBe(false); });
      expect(send).toHaveBeenCalledWith({ type: "terminal.terminate" });
      expect(onStateChange).toHaveBeenLastCalledWith({ ready: false, error: "Termination send failed" });
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("does not retry a disconnected scratch process", async () => {
    vi.useFakeTimers();
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client, openTerminal } = makeClient(sent, messages);
    const terminateRef: { current: (() => boolean) | null } = { current: null };
    const onStateChange = vi.fn();
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false, {
          request: { session_id: "session", pane_id: "pane", target_kind: "scratch" },
          terminateRef, onStateChange,
        })} />);
        await settle();
      });
      await act(async () => {
        messages[0]!({ type: "disconnected", session_id: "session", pane_id: "pane", stream_id: "stream",
          code: "disconnected", message: "Terminal websocket closed" });
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(openTerminal).toHaveBeenCalledOnce();
      expect(host.textContent).toContain("Terminal websocket closed");
      expect(host.querySelector("button")).toBeNull();
      expect(terminateRef.current?.()).toBe(false);
      expect(onStateChange).toHaveBeenLastCalledWith({ ready: false, error: "Terminal websocket closed" });
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });
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
  it("forwards popup arrow navigation as application cursor keys", async () => {
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => {
        root.render(<TerminalPane {...paneProps(client, false, { request: { session_id: "session", pane_id: "pane", target_kind: "popup" } })} />);
        await settle();
      });
      await act(async () => { messages[0]!(message("owned")); await settle(); });
      const terminal = mocks.terminals.at(-1)!;
      for (const [key, sequence] of [["ArrowUp", "\u001bOA"], ["ArrowDown", "\u001bOB"], ["ArrowRight", "\u001bOC"], ["ArrowLeft", "\u001bOD"]] as const) {
        const event = new KeyboardEvent("keydown", { key, cancelable: true });
        expect(terminal.keyHandler?.(event)).toBe(false);
        expect(event.defaultPrevented).toBe(true);
        expect(sent.at(-1)).toEqual({ type: "terminal.input", text: sequence, bytes: null });
      }
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("moves DOM focus only after ownership confirmation when focusOnAttach allows it", async () => {
    for (const focusOnAttach of [false, true, undefined]) {
      const sent: TerminalCommand[] = [];
      const messages: Array<(value: TerminalStreamMessage) => void> = [];
      const { client, openTerminal } = makeClient(sent, messages);
      const host = document.createElement("div");
      document.body.append(host);
      const root = createRoot(host);
      try {
        await act(async () => {
          root.render(<TerminalPane {...paneProps(client, false, focusOnAttach === undefined ? {} : { focusOnAttach })} />);
          await settle();
        });
        expect(openTerminal).toHaveBeenCalledOnce();
        const terminal = mocks.terminals.at(-1)!;
        expect(terminal.focus).not.toHaveBeenCalled();
        await act(async () => { messages[0]!(message("owned")); });
        if (focusOnAttach === false) expect(terminal.focus).not.toHaveBeenCalled();
        else expect(terminal.focus).toHaveBeenCalled();
      } finally {
        await act(async () => root.unmount());
        host.remove();
        mocks.terminals.length = 0;
      }
    }
  });

  it("focuses a switched-to terminal only once it is painted and control is confirmed", async () => {
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    const render = (overrides: Partial<Parameters<typeof TerminalPane>[0]>) => act(async () => {
      root.render(<TerminalPane {...paneProps(client, false, overrides)} />);
      await settle();
    });
    try {
      await render({ controlAllowed: false, controlPending: true, presented: false });
      const terminal = mocks.terminals.at(-1)!;
      terminal.focus.mockClear();
      await render({ controlAllowed: true, controlPending: false, presented: false });
      expect(terminal.focus).not.toHaveBeenCalled();
      await render({ controlAllowed: true, controlPending: false, presented: true });
      expect(terminal.focus).not.toHaveBeenCalled();
      await act(async () => { messages[0]!(message("owned")); });
      expect(terminal.focus).toHaveBeenCalledOnce();
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it.each(["owned", "lost", "conflict"] as const)("restores focus after an overlay only with owned control (%s)", async ownership => {
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client, openTerminal } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    const render = (controlAllowed: boolean) => act(async () => {
      root.render(<TerminalPane {...paneProps(client, false, { controlAllowed })} />);
      await settle();
    });
    try {
      await render(false);
      await act(async () => { messages[0]!(message(ownership)); });
      const terminal = mocks.terminals.at(-1)!;
      terminal.focus.mockClear();
      await render(true);
      if (ownership === "owned") expect(terminal.focus).toHaveBeenCalledOnce();
      else expect(terminal.focus).not.toHaveBeenCalled();
      expect(openTerminal).toHaveBeenCalledOnce();
      expect(sent.filter(command => command.type === "terminal.input")).toEqual([]);
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
  it("closes an outgoing stream while hidden, then reattaches input on revisit", async () => {
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

  it("applies each authoritative frame grid before its ANSI without echoing a resize", async () => {
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false)} />); await settle(); });
      const terminal = mocks.terminals.at(-1)!;
      const writes: Array<() => void> = [];
      terminal.write.mockImplementation((_data, done) => { if (done) writes.push(done); });
      act(() => messages[0]!({ type: "frame", session_id: "session", pane_id: "pane", stream_id: "stream", seq: "1", encoding: "ansi", width: 60, height: 12, full: true, bytes: btoa("frame") }));
      await act(async () => { await settle(); });
      // Rendering is held before the grid changes, so xterm's reflow of the old buffer never paints.
      expect(terminal.write).toHaveBeenLastCalledWith("\x1b[?2026h", expect.any(Function));
      expect(terminal.resize).not.toHaveBeenCalled();
      await act(async () => {
        writes[0]!();
        // resize() flushes xterm's write queue; it must not run inside a write callback.
        expect(terminal.resize).not.toHaveBeenCalled();
        await settle();
      });
      expect(terminal.resize).toHaveBeenCalledWith(60, 12);
      expect(terminal.cols).toBe(60);
      expect(terminal.rows).toBe(12);
      expect(new TextDecoder().decode(terminal.write.mock.calls.at(-1)![0] as Uint8Array)).toBe("frame");
      expect(sent.filter((command) => command.type === "terminal.resize")).toEqual([]);
      writes[1]!();
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("serializes frame grid changes so queued ANSI never runs against a newer grid", async () => {
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false)} />); await settle(); });
      const terminal = mocks.terminals.at(-1)!;
      const writes: Array<() => void> = [];
      terminal.write.mockImplementation((_data, done) => { if (done) writes.push(done); });
      act(() => {
        messages[0]!({ type: "frame", session_id: "session", pane_id: "pane", stream_id: "stream", seq: "1", encoding: "ansi", width: 60, height: 12, full: true, bytes: btoa("old grid") });
        messages[0]!({ type: "frame", session_id: "session", pane_id: "pane", stream_id: "stream", seq: "2", encoding: "ansi", width: 100, height: 20, full: false, bytes: btoa("new grid") });
      });
      await act(async () => { await settle(); });
      expect(writes).toHaveLength(1);
      await act(async () => { writes[0]!(); await settle(); });
      expect(writes).toHaveLength(2);
      expect(terminal.cols).toBe(60);
      expect(terminal.rows).toBe(12);
      writes[1]!();
      await act(async () => { await settle(); });
      expect(writes).toHaveLength(3);
      expect(terminal.cols).toBe(60);
      await act(async () => { writes[2]!(); await settle(); });
      expect(writes).toHaveLength(4);
      expect(terminal.resize).toHaveBeenLastCalledWith(100, 20);
      expect(terminal.cols).toBe(100);
      expect(terminal.rows).toBe(20);
      writes[3]!();
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("measures viewport changes after a frame without locally reflowing the rendered grid", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    mocks.fitDimensions.push([80, 24]);
    mocks.fitProposals.push([80, 24], [120, 40]);
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false)} />); await settle(); });
      const terminal = mocks.terminals.at(-1)!;
      act(() => {
        messages[0]!(message("owned"));
        messages[0]!({ type: "frame", session_id: "session", pane_id: "pane", stream_id: "stream", seq: "1", encoding: "ansi", width: 60, height: 12, full: true, bytes: btoa("frame") });
      });
      await act(async () => { await settle(); });
      const fit = mocks.fits.at(-1)!;
      const fitCount = fit.fit.mock.calls.length;
      mocks.observers[0]!.callback([], mocks.observers[0] as unknown as ResizeObserver);
      await vi.advanceTimersByTimeAsync(100);
      expect(fit.fit).toHaveBeenCalledTimes(fitCount);
      expect(terminal.cols).toBe(60);
      expect(terminal.rows).toBe(12);
      expect(sent.filter((command): command is Extract<TerminalCommand, { type: "terminal.resize" }> => command.type === "terminal.resize")).toContainEqual({
        type: "terminal.resize",
        cols: 120,
        rows: 40,
        cell_width_px: 13,
        cell_height_px: 33,
      });
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("resyncs and releases queued frames when the bounded backlog is exceeded", async () => {
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const onResync = vi.fn();
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false, { onResync })} />); await settle(); });
      const terminal = mocks.terminals.at(-1)!;
      const writes: Array<() => void> = [];
      terminal.write.mockImplementation((_data, done) => { if (done) writes.push(done); });
      act(() => {
        for (let index = 1; index <= 65; index += 1) {
          messages[0]!({
            type: "frame",
            session_id: "session",
            pane_id: "pane",
            stream_id: "stream",
            seq: String(index),
            encoding: "ansi",
            width: 80,
            height: 24,
            full: index === 1,
            bytes: btoa(`frame-${index}`),
          });
        }
      });
      expect(onResync).toHaveBeenCalledOnce();
      expect(writes).toHaveLength(1);
      writes[0]!();
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
      await act(async () => { messages[0]!(message("owned")); });
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
      fontFamily: '"IosevkaTerm Nerd Font Mono", ui-monospace, "FiraCode Nerd Font Mono", "Hack Nerd Font Mono", "IBM Plex Mono", "Noto Sans Mono", monospace',
      fontSize: 16,
      lineHeight: 1,
      scrollbar: { showScrollbar: false, width: 8 },
    });
  });

  it("defers a changed viewport resize until authoritative terminal ownership", async () => {
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
      expect(sent.filter((command) => command.type === "terminal.resize")).toEqual([]);
      act(() => messages[0]!(message("pending")));
      expect(sent.filter((command) => command.type === "terminal.resize")).toEqual([]);
      act(() => messages[0]!(message("owned")));
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

  it("keeps one resize in flight and follows up with the latest viewport once Herdr answers", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    mocks.fitDimensions.push([80, 24]);
    mocks.fitProposals.push([80, 24], [120, 40], [90, 20], [90, 20], [70, 15], [70, 15]);
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    const grids = () => sent.flatMap((command) => command.type === "terminal.resize" ? [`${command.cols}x${command.rows}`] : []);
    const frame = (seq: number, width: number, height: number) => ({ type: "frame" as const, session_id: "session", pane_id: "pane", stream_id: "stream", seq: String(seq), encoding: "ansi" as const, width, height, full: seq === 1, bytes: btoa("frame") });
    const observe = async () => {
      mocks.observers[0]!.callback([], mocks.observers[0] as unknown as ResizeObserver);
      await vi.advanceTimersByTimeAsync(16);
    };
    try {
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false)} />); await settle(); });
      act(() => {
        messages[0]!(message("owned"));
        messages[0]!(frame(1, 80, 24));
      });
      await act(async () => { await settle(); });
      await observe();
      expect(grids()).toEqual(["120x40"]);
      await observe();
      expect(grids()).toEqual(["120x40"]);
      act(() => messages[0]!(frame(2, 120, 40)));
      await act(async () => { await settle(); });
      expect(grids()).toEqual(["120x40", "90x20"]);
      await observe();
      expect(grids()).toEqual(["120x40", "90x20"]);
      await vi.advanceTimersByTimeAsync(150);
      expect(grids()).toEqual(["120x40", "90x20", "70x15"]);
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it("fits once per animation frame and cancels a stale callback across remount", async () => {
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
      firstObserver.callback([], firstObserver as unknown as ResizeObserver);
      expect(vi.getTimerCount()).toBe(1);
      await vi.advanceTimersByTimeAsync(16);
      expect(vi.getTimerCount()).toBe(0);
      expect(sent.filter((command) => command.type === "terminal.resize")).toEqual([]);
      act(() => messages[0]!(message("owned")));
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
          messages[1](message("owned"));
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

  it("keeps unselected terminals control-attached without accepting input or reopening on selection changes", async () => {
    vi.useFakeTimers();
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client, openTerminal } = makeClient(sent, messages);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    const render = async (overrides: Partial<Parameters<typeof TerminalPane>[0]>) => {
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false, overrides)} />); await settle(); });
    };
    try {
      await render({ selected: false, controlAllowed: false });
      expect(openTerminal.mock.calls[0]![0]).toMatchObject({ mode: "control", takeover: false });
      await act(async () => { messages[0]!(message("owned")); });
      const terminal = mocks.terminals.at(-1)!;
      terminal.dataHandler?.("unselected");
      expect(sent).toEqual([]);
      await render({ selected: true, controlAllowed: false, controlPending: true, focusToken: 1 });
      terminal.dataHandler?.("pending");
      expect(sent).toEqual([]);
      await render({ selected: true, controlAllowed: true, focusToken: 1 });
      expect(sent).toEqual([{ type: "terminal.input", text: "pending", bytes: null }]);
      await render({ selected: false, controlAllowed: false, focusToken: 2 });
      await act(async () => { await vi.advanceTimersByTimeAsync(2_000); });
      terminal.dataHandler?.("unselected again");
      expect(sent).toEqual([{ type: "terminal.input", text: "pending", bytes: null }]);
      await render({ selected: true, controlAllowed: true, focusToken: 3 });
      terminal.dataHandler?.("selected");
      expect(sent).toEqual([
        { type: "terminal.input", text: "pending", bytes: null },
        { type: "terminal.input", text: "selected", bytes: null },
      ]);
      expect(openTerminal).toHaveBeenCalledOnce();
      expect((await openTerminal.mock.results[0]!.value).close).not.toHaveBeenCalled();
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });

  it.each([
    ["lost", "Take control", true],
    ["lost", "Retry", false],
    ["conflict", "Take control", true],
    ["conflict", "Retry", false],
  ] as const)("retains the last frame after %s and waits for explicit %s", async (state, action, takeover) => {
    vi.useFakeTimers();
    const sent: TerminalCommand[] = [];
    const messages: Array<(value: TerminalStreamMessage) => void> = [];
    const { client, openTerminal } = makeClient(sent, messages);
    const registerStream = vi.fn();
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    const frame = (text: string, seq = "1"): TerminalStreamMessage => ({
      type: "frame", session_id: "session", pane_id: "pane", stream_id: "stream",
      seq, encoding: "ansi", width: 80, height: 24, full: seq === "1", bytes: btoa(text),
    });
    try {
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false, { registerStream })} />); await settle(); });
      const opened = await openTerminal.mock.results[0]!.value;
      await act(async () => {
        messages[0]!(message("owned"));
        messages[0]!(frame("last owned output"));
        await settle();
      });
      const terminal = mocks.terminals.at(-1)!;
      await act(async () => { messages[0]!(message(state)); });
      expect(opened.close).toHaveBeenCalledOnce();
      expect(registerStream).toHaveBeenLastCalledWith(opened, false);
      expect(host.querySelector('[role="alert"]')?.textContent).toContain("Control taken by another client");
      const writeCount = terminal.write.mock.calls.length;
      await act(async () => {
        messages[0]!(frame("retired output", "2"));
        host.querySelector(".terminal-host")!.dispatchEvent(pointer("pointerdown", 10));
        terminal.dataHandler?.("discarded after ownership loss");
        root.render(<TerminalPane {...paneProps(client, false, { selected: false, controlAllowed: false, registerStream })} />);
        await settle();
        await vi.advanceTimersByTimeAsync(2_000);
      });
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false, { registerStream })} />); await settle(); });
      expect(openTerminal).toHaveBeenCalledOnce();
      expect(terminal.write).toHaveBeenCalledTimes(writeCount);
      expect(sent).toEqual([]);
      const button = Array.from(host.querySelectorAll("button")).find((entry) => entry.textContent === action)!;
      await act(async () => { button.click(); await settle(); });
      expect(openTerminal).toHaveBeenCalledTimes(2);
      expect(openTerminal.mock.calls[1]![0]).toMatchObject({ mode: "control", takeover });
      expect(opened.close).toHaveBeenCalledOnce();
      expect(mocks.terminals.at(-1)).toBe(terminal);
      await act(async () => {
        messages[1]!(message("owned"));
        messages[1]!(frame("fresh baseline"));
        await settle();
      });
      expect(host.querySelector('[role="alert"]')).toBeNull();
      expect(new TextDecoder().decode(terminal.write.mock.calls.at(-1)![0])).toBe("fresh baseline");
      terminal.dataHandler?.("new input");
      expect(sent.filter((command) => command.type === "terminal.input")).toEqual([{ type: "terminal.input", text: "new input", bytes: null }]);
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });
});

function fileDrop(files: File[], types = ["Files"]) {
  const event = new Event("drop", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "dataTransfer", { value: { files, types, items: [], dropEffect: "none" } });
  return event;
}

async function renderFileDropPane(overrides: Partial<Parameters<typeof TerminalPane>[0]> = {}) {
  const sent: TerminalCommand[] = [];
  const messages: Array<(value: TerminalStreamMessage) => void> = [];
  const { client } = makeClient(sent, messages);
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const render = async (next: Partial<Parameters<typeof TerminalPane>[0]> = {}) => {
    await act(async () => {
      root.render(<TerminalPane {...paneProps(client, false, { ...overrides, ...next })} />);
      await settle();
    });
  };
  await render();
  return { host, root, messages, sent, render, terminal: mocks.terminals.at(-1)!, terminalHost: host.querySelector<HTMLElement>(".terminal-host")!, dispose: async () => { await act(async () => root.unmount()); host.remove(); } };
}

describe("TerminalPane native clipboard and file drop", () => {
  it("uses the existing Cmd+C selection handler and rich Cmd+V read without adding Enter", async () => {
    mocks.native.enabled = true;
    vi.stubGlobal("navigator", { platform: "MacIntel", userAgent: "Mac OS X" });
    mocks.native.invoke.mockResolvedValue("'/tmp/clipboard image.png'");
    const pane = await renderFileDropPane();
    try {
      await act(async () => { pane.messages[0]!(message("owned")); });
      pane.terminal.getSelection.mockReturnValue("actual selected text");
      await act(async () => {
        const copy = new KeyboardEvent("keydown", { key: "c", metaKey: true, cancelable: true });
        expect(pane.terminal.keyHandler!(copy)).toBe(false);
        expect(copy.defaultPrevented).toBe(true);
        await settle();
      });
      expect(mocks.native.invoke).toHaveBeenCalledWith("cockpit_clipboard_write", { text: "actual selected text" });
      await act(async () => {
        pane.terminal.keyHandler!(new KeyboardEvent("keydown", { key: "v", metaKey: true, cancelable: true }));
        await settle();
      });
      expect(mocks.native.invoke).toHaveBeenCalledWith("cockpit_clipboard_read_terminal");
      expect(pane.terminal.paste).toHaveBeenCalledExactlyOnceWith("'/tmp/clipboard image.png'");
      expect(pane.sent.filter((command) => command.type === "terminal.input")).toEqual([]);
    } finally { await pane.dispose(); }
  });

  it("stages actual file bytes on the dropped pane and holds the paste until control is granted", async () => {
    mocks.native.enabled = true;
    mocks.native.invoke.mockResolvedValue("'/tmp/staged/image.png'");
    const onRequestControl = vi.fn();
    const onSelect = vi.fn();
    const pane = await renderFileDropPane({ selected: false, controlAllowed: false, onRequestControl, onSelect });
    const bytes = new Uint8Array([0, 128, 255]);
    const file = new File([bytes], "image.png", { type: "image/png" });
    Object.defineProperty(file, "arrayBuffer", { value: async () => bytes.buffer });
    try {
      const event = fileDrop([file]);
      await act(async () => { pane.terminalHost.dispatchEvent(event); await settle(); });
      expect(event.defaultPrevented).toBe(true);
      expect(mocks.native.invoke).toHaveBeenCalledWith("cockpit_terminal_files_stage", { files: [{ name: "image.png", bytes: [0, 128, 255] }] });
      expect(onRequestControl).toHaveBeenCalledOnce();
      expect(onSelect).toHaveBeenCalledOnce();
      expect(pane.terminal.paste).not.toHaveBeenCalled();
      expect(pane.host.querySelector('[role="status"]')?.getAttribute("aria-label")).toBe("Clipboard operation in progress");
      await pane.render({ selected: true, controlPending: true, focusToken: 1 });
      await pane.render({ selected: true, controlAllowed: true, controlPending: false, focusToken: 1 });
      await act(async () => { pane.messages.at(-1)!(message("owned")); });
      expect(pane.terminal.paste).toHaveBeenCalledExactlyOnceWith("'/tmp/staged/image.png'");
      expect(pane.host.querySelector('[aria-label="Clipboard operation in progress"]')).toBeNull();
      expect(pane.sent.filter((command) => command.type === "terminal.input")).toEqual([]);
    } finally { await pane.dispose(); }
  });

  it.each(["refused", "epoch", "ownership"] as const)("discards staged data after %s invalidates the pending drop", async (reason) => {
    mocks.native.enabled = true;
    let finish!: (text: string) => void;
    mocks.native.invoke.mockImplementation(() => new Promise<string>((resolve) => { finish = resolve; }));
    const pane = await renderFileDropPane({ controlAllowed: false, controlPending: true });
    const file = new File(["data"], "data.txt");
    Object.defineProperty(file, "arrayBuffer", { value: async () => new Uint8Array([100]).buffer });
    try {
      await act(async () => { pane.terminalHost.dispatchEvent(fileDrop([file])); await settle(); });
      if (reason === "refused") await pane.render({ controlAllowed: false, controlPending: false });
      if (reason === "epoch") await pane.render({ focusEpoch: 2 });
      if (reason === "ownership") await act(async () => { pane.messages.at(-1)!(message("lost")); });
      await act(async () => { finish("'/tmp/stale.txt'"); await settle(); });
      await pane.render({ controlAllowed: true, controlPending: false, focusEpoch: reason === "epoch" ? 2 : 1 });
      await act(async () => { pane.messages.at(-1)!(message("owned")); });
      expect(pane.terminal.paste).not.toHaveBeenCalled();
      expect(pane.host.querySelector('[aria-label="Clipboard operation in progress"]')).toBeNull();
    } finally { await pane.dispose(); }
  });

  it("ignores internal drags and advertised files with no actual File payload", async () => {
    const onRequestControl = vi.fn();
    const pane = await renderFileDropPane({ onRequestControl });
    try {
      const internal = fileDrop([], ["text/plain"]);
      const empty = fileDrop([]);
      await act(async () => { pane.terminalHost.dispatchEvent(internal); pane.terminalHost.dispatchEvent(empty); await settle(); });
      expect(internal.defaultPrevented).toBe(false);
      expect(empty.defaultPrevented).toBe(true);
      expect(onRequestControl).not.toHaveBeenCalled();
      expect(mocks.native.invoke).not.toHaveBeenCalled();
      expect(pane.host.querySelector('[role="alert"]')).toBeNull();
    } finally { await pane.dispose(); }
  });

  it("ignores a terminal behind a modal but accepts the terminal inside that modal", async () => {
    mocks.native.enabled = true;
    mocks.native.invoke.mockResolvedValue("'/tmp/staged/data.txt'");
    const onRequestControl = vi.fn();
    const pane = await renderFileDropPane({ onRequestControl });
    const modal = document.createElement("div");
    modal.setAttribute("role", "dialog");
    modal.setAttribute("aria-modal", "true");
    document.body.append(modal);
    const file = new File(["data"], "data.txt");
    Object.defineProperty(file, "arrayBuffer", { value: async () => new Uint8Array([100]).buffer });
    try {
      await act(async () => { pane.terminalHost.dispatchEvent(fileDrop([file])); await settle(); });
      expect(onRequestControl).not.toHaveBeenCalled();
      expect(mocks.native.invoke).not.toHaveBeenCalled();
      modal.append(pane.host);
      await act(async () => { pane.messages[0]!(message("owned")); pane.terminalHost.dispatchEvent(fileDrop([file])); await settle(); });
      expect(onRequestControl).toHaveBeenCalledOnce();
      expect(pane.terminal.paste).toHaveBeenCalledExactlyOnceWith("'/tmp/staged/data.txt'");
    } finally { await pane.dispose(); modal.remove(); }
  });

  it("reports browser drop errors without a clipboard retry, retaining browser text paste", async () => {
    vi.stubGlobal("navigator", { clipboard: { readText: vi.fn(async () => "browser text"), writeText: vi.fn() } });
    const pane = await renderFileDropPane();
    try {
      await act(async () => { pane.messages[0]!(message("owned")); pane.terminalHost.dispatchEvent(fileDrop([new File(["data"], "not-a-path.txt")])); await settle(); });
      expect(pane.terminal.paste).not.toHaveBeenCalled();
      expect(pane.host.querySelector('[role="alert"]')?.getAttribute("aria-label")).toContain("requires the native Cockpit app");
      expect(pane.host.querySelector(".terminal-status-retry")).toBeNull();
      await act(async () => {
        pane.terminal.keyHandler!(new KeyboardEvent("keydown", { key: "v", ctrlKey: true, shiftKey: true, cancelable: true }));
        await settle();
      });
      expect(pane.terminal.paste).toHaveBeenCalledExactlyOnceWith("browser text");
      expect(pane.host.querySelector('[role="alert"]')).toBeNull();
    } finally { await pane.dispose(); }
  });

  it("keeps closed or inert terminals from accepting file drops", async () => {
    const onRequestControl = vi.fn();
    const pane = await renderFileDropPane({ onRequestControl });
    try {
      pane.host.setAttribute("inert", "");
      await act(async () => { pane.terminalHost.dispatchEvent(fileDrop([new File(["data"], "data.txt")])); await settle(); });
      pane.host.removeAttribute("inert");
      await act(async () => {
        pane.messages[0]!({ type: "closed", session_id: "session", pane_id: "pane", stream_id: "stream", reason: "exited" });
        await settle();
      });
      expect(pane.terminalHost.dataset.closed).toBe("true");
      await act(async () => { pane.terminalHost.dispatchEvent(fileDrop([new File(["data"], "data.txt")])); await settle(); });
      expect(onRequestControl).not.toHaveBeenCalled();
      expect(mocks.native.invoke).not.toHaveBeenCalled();
      expect(pane.terminal.paste).not.toHaveBeenCalled();
    } finally { await pane.dispose(); }
  });

  it("reports native staging failures on the target badge without re-reading the clipboard", async () => {
    mocks.native.enabled = true;
    mocks.native.invoke.mockRejectedValue("Could not read the dropped file");
    const pane = await renderFileDropPane();
    const file = new File(["data"], "data.txt");
    Object.defineProperty(file, "arrayBuffer", { value: async () => new Uint8Array([100]).buffer });
    try {
      await act(async () => { pane.messages[0]!(message("owned")); pane.terminalHost.dispatchEvent(fileDrop([file])); await settle(); });
      expect(pane.host.querySelector('[role="alert"]')?.getAttribute("aria-label")).toBe("Could not read the dropped file");
      expect(pane.host.querySelector(".terminal-status-retry")).toBeNull();
      expect(pane.terminal.paste).not.toHaveBeenCalled();
      expect(mocks.native.invoke.mock.calls.map(([command]) => command)).toEqual(["cockpit_terminal_files_stage"]);
    } finally { await pane.dispose(); }
  });

  it.each(["clipboard", "drop"] as const)("flushes pending %s when ownership arrives before the stream", async (operation) => {
    const text = operation === "drop" ? "'/tmp/staged/data.txt'" : "text before attach";
    mocks.native.enabled = operation === "drop";
    mocks.native.invoke.mockResolvedValue(text);
    vi.stubGlobal("navigator", { clipboard: { readText: vi.fn(async () => text), writeText: vi.fn() } });
    const sent: TerminalCommand[] = [];
    let receive!: (value: TerminalStreamMessage) => void;
    let resolveStream!: (value: TerminalStream) => void;
    const client = { openTerminal: vi.fn((_request: TerminalOpenRequest, onMessage: (value: TerminalStreamMessage) => void) => {
      receive = onMessage;
      return new Promise<TerminalStream>((resolve) => { resolveStream = resolve; });
    }) } as unknown as CockpitClient;
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => { root.render(<TerminalPane {...paneProps(client, false)} />); await settle(); });
      const terminal = mocks.terminals.at(-1)!;
      await act(async () => { receive(message("owned")); });
      await act(async () => {
        if (operation === "clipboard") {
          terminal.keyHandler!(new KeyboardEvent("keydown", { key: "v", ctrlKey: true, shiftKey: true, cancelable: true }));
        } else {
          const file = new File(["data"], "data.txt");
          Object.defineProperty(file, "arrayBuffer", { value: async () => new Uint8Array([100]).buffer });
          host.querySelector(".terminal-host")!.dispatchEvent(fileDrop([file]));
        }
        await settle();
      });
      expect(terminal.paste).not.toHaveBeenCalled();
      expect(host.querySelector('[aria-label="Clipboard operation in progress"]')).not.toBeNull();
      await act(async () => { resolveStream(stream(sent)); await settle(); });
      expect(terminal.paste).toHaveBeenCalledExactlyOnceWith(text);
      expect(host.querySelector('[aria-label="Clipboard operation in progress"]')).toBeNull();
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });
});
