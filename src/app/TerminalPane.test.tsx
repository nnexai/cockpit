// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient, TerminalStream } from "../client/CockpitClient";
import type { TerminalCommand, TerminalOpenRequest, TerminalStreamMessage } from "../protocol/generated/v1";
import { TerminalPane } from "./TerminalPane";
const mocks = vi.hoisted(() => {
  const terminals: MockTerminal[] = [];
  const fits: MockFitAddon[] = [];
  const observers: MockResizeObserver[] = [];
  const fitDimensions: Array<[number, number]> = [];

  class MockTerminal {
    cols = 80;
    rows = 24;
    options = { fontSize: 14 };
    element: HTMLElement | null = null;
    private resizeListeners: Array<(size: { cols: number; rows: number }) => void> = [];
    readonly focus = vi.fn();
    readonly dispose = vi.fn();
    readonly write = vi.fn();
    readonly onData = vi.fn(() => ({ dispose: vi.fn() }));
    readonly onBinary = vi.fn(() => ({ dispose: vi.fn() }));
    readonly onResize = vi.fn((listener: (size: { cols: number; rows: number }) => void) => {
      this.resizeListeners.push(listener);
      return { dispose: vi.fn(() => { this.resizeListeners = this.resizeListeners.filter((entry) => entry !== listener); }) };
    });
    readonly attachCustomKeyEventHandler = vi.fn();
    readonly attachCustomWheelEventHandler = vi.fn();

    constructor() {
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

function message(state: "owned" | "observing"): TerminalStreamMessage {
  return { type: "ownership", session_id: "session", pane_id: "pane", stream_id: "stream", state, message: null };
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

function pointer(type: string, timeStamp: number, button = 0) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperties(event, {
    button: { value: button },
    clientX: { value: 120 },
    clientY: { value: 80 },
    pointerId: { value: 1 },
    shiftKey: { value: false },
    ctrlKey: { value: false },
    altKey: { value: false },
    metaKey: { value: false },
    timeStamp: { value: timeStamp },
  });
  return event;
}

function paneProps(client: CockpitClient, terminalMouseInput: boolean) {
  return {
    client,
    request,
    selected: true,
    controlAllowed: true,
    controlPending: false,
    terminalMouseInput,
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
  it("debounces resize fitting and cancels a stale callback across remount", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("ResizeObserver", mocks.MockResizeObserver);
    mocks.fitDimensions.push([80, 24], [120, 40], [100, 30]);
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
      await act(async () => { messages[0](message("owned")); });
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
});
