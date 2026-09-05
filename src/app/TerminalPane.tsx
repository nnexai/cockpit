import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import type { CockpitClient, TerminalStream } from "../client/CockpitClient";
import type { TerminalCommand, TerminalMouseButton, TerminalMouseKind, TerminalOpenRequest, TerminalOwnershipState, TerminalStreamMessage } from "../protocol/generated/v1";


export const MAX_PENDING_CONTROL_COMMANDS = 64;
export function appendPendingControlCommand(queue: TerminalCommand[], command: TerminalCommand): TerminalCommand[] {
  return queue.length >= MAX_PENDING_CONTROL_COMMANDS
    ? [...queue.slice(queue.length - MAX_PENDING_CONTROL_COMMANDS + 1), command]
    : [...queue, command];
}
export type TerminalPaneProps = {
  client: CockpitClient;
  request: Omit<TerminalOpenRequest, "mode" | "takeover" | "cols" | "rows" | "cell_width_px" | "cell_height_px">;
  selected: boolean;
  controlAllowed: boolean;
  controlPending: boolean;
  terminalMouseInput: boolean;
  onRequestControl?: () => void;
  onSelect?: () => void;
  onResync?: () => void;
  onClosed?: () => void;
  onClosePane?: () => void;
  registerStream?: (stream: TerminalStream, active: boolean) => void;
};

type PaneError = { code: string; message: string };
function decodeFrame(bytes: string): Uint8Array {
  const binary = globalThis.atob(bytes);
  const data = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) data[index] = binary.charCodeAt(index);
  return data;
}

function applicationFontSize(): number {
  const fontSize = Number.parseFloat(globalThis.getComputedStyle(document.body).fontSize);
  return Number.isFinite(fontSize) && fontSize > 0 ? fontSize : 16;
}

export function createCockpitTerminal(fontSize = applicationFontSize()): Terminal {
  return new Terminal({
    convertEol: false,
    cursorBlink: false,
    fontFamily: '"IosevkaTerm Nerd Font Mono", "FiraCode Nerd Font Mono", "IBM Plex Mono", "Noto Sans Mono", monospace',
    fontSize,
    theme: { background: "#0c1016", foreground: "#d8dee8" },
    scrollback: 5000,
    vtExtensions: { kittyKeyboard: true },
  });
}


function commandInput(text: string | null, bytes: string | null): TerminalCommand {
  return { type: "terminal.input", text, bytes };
}

export function terminalModifiedEnterInput(event: Pick<KeyboardEvent, "type" | "key" | "shiftKey" | "ctrlKey" | "altKey" | "metaKey">): string | null {
  return event.type === "keydown"
    && event.key === "Enter"
    && event.shiftKey
    && !event.ctrlKey
    && !event.altKey
    && !event.metaKey
    ? "\n"
    : null;
}

type TerminalBounds = Pick<DOMRect, "left" | "top" | "width" | "height">;
export function terminalCellPosition(clientX: number, clientY: number, bounds: TerminalBounds, cols: number, rows: number): { column: number; row: number } {
  const safeCols = Math.max(1, Math.floor(cols));
  const safeRows = Math.max(1, Math.floor(rows));
  const column = bounds.width > 0 ? Math.floor((clientX - bounds.left) / bounds.width * safeCols) : 0;
  const row = bounds.height > 0 ? Math.floor((clientY - bounds.top) / bounds.height * safeRows) : 0;
  return {
    column: Math.max(0, Math.min(safeCols - 1, column)),
    row: Math.max(0, Math.min(safeRows - 1, row)),
  };
}

type TerminalPointer = Pick<PointerEvent, "clientX" | "clientY" | "shiftKey" | "ctrlKey" | "altKey" | "metaKey">;
export function terminalMouseButton(button: number): TerminalMouseButton | null {
  if (button === 0) return "left";
  if (button === 1) return "middle";
  if (button === 2) return "right";
  return null;
}

export function terminalModifierBits(event: Pick<PointerEvent, "shiftKey" | "ctrlKey" | "altKey" | "metaKey">): number {
  return (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
}

export function terminalMouseCommand(
  kind: TerminalMouseKind,
  button: TerminalMouseButton | null,
  event: TerminalPointer,
  bounds: TerminalBounds,
  cols: number,
  rows: number,
): TerminalCommand {
  const position = terminalCellPosition(event.clientX, event.clientY, bounds, cols, rows);
  return {
    type: "terminal.mouse",
    kind,
    button,
    column: position.column,
    row: position.row,
    modifiers: terminalModifierBits(event),
  };
}

export function forwardTerminalMouse(
  enabled: boolean,
  send: (command: TerminalCommand) => void,
  kind: TerminalMouseKind,
  button: TerminalMouseButton | null,
  event: TerminalPointer,
  bounds: TerminalBounds,
  cols: number,
  rows: number,
): boolean {
  if (!enabled) return false;
  send(terminalMouseCommand(kind, button, event, bounds, cols, rows));
  return true;
}


function terminalCellGeometry(terminal: Terminal): { cell_width_px: number; cell_height_px: number } {
  const bounds = terminal.element?.querySelector<HTMLElement>(".xterm-screen")?.getBoundingClientRect();
  return {
    cell_width_px: bounds && terminal.cols > 0 ? Math.max(1, Math.round(bounds.width / terminal.cols)) : 0,
    cell_height_px: bounds && terminal.rows > 0 ? Math.max(1, Math.round(bounds.height / terminal.rows)) : 0,
  };
}

export function TerminalPane({ client, request, selected, controlAllowed, controlPending, terminalMouseInput, onRequestControl, onSelect, onResync, onClosed, onClosePane, registerStream }: TerminalPaneProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const streamRef = useRef<TerminalStream | null>(null);
  const [ownership, setOwnership] = useState<TerminalOwnershipState>("observing");
  const [error, setError] = useState<PaneError | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [closed, setClosed] = useState(false);
  const [terminalReady, setTerminalReady] = useState(false);
  const lastSequence = useRef<bigint | null>(null);
  const ownershipRef = useRef(ownership);
  const pendingCommands = useRef<TerminalCommand[]>([]);
  const [controlRequested, setControlRequested] = useState(controlAllowed);
  const controlRequestedRef = useRef(controlAllowed);
  const controlRequestPendingRef = useRef(false);
  const controlAllowedRef = useRef(controlAllowed);
  controlAllowedRef.current = controlAllowed;
  const activeMouseButton = useRef<TerminalMouseButton | null>(null);
  const lastMouseMotionAt = useRef(0);
  const sendInput = (command: TerminalCommand) => {
    if (!controlAllowedRef.current && !controlRequestPendingRef.current) return;
    if (controlAllowedRef.current && ownershipRef.current === "owned" && streamRef.current) streamRef.current.send(command);
    else pendingCommands.current = appendPendingControlCommand(pendingCommands.current, command);
  };
  const flushPending = () => {
    const stream = streamRef.current;
    if (!controlAllowedRef.current || !controlRequestedRef.current || ownershipRef.current !== "owned" || !stream || pendingCommands.current.length === 0) return;
    pendingCommands.current.forEach((command) => stream.send(command));
    pendingCommands.current = [];
  };
  const requestControl = () => {
    terminalRef.current?.focus();
    onRequestControl?.();
    if (!selected) onSelect?.();
    if (controlAllowedRef.current && controlRequestedRef.current) return;
    controlRequestPendingRef.current = true;
    controlRequestedRef.current = false;
    setControlRequested(false);
  };

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const terminal = createCockpitTerminal();
    const fit = new FitAddon();
    terminal.open(host);
    terminal.loadAddon(fit);
    fit.fit();
    setTerminalReady(true);
    terminalRef.current = terminal;
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => fit.fit());
    observer?.observe(host);
    return () => {
      observer?.disconnect();
      terminal.dispose();
      terminalRef.current = null;
    };
  }, []);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const data = terminal.onData((text) => sendInput(commandInput(text, null)));
    const binary = terminal.onBinary((bytes) => sendInput(commandInput(null, btoa(bytes))));
    terminal.attachCustomKeyEventHandler((event) => {
      const text = terminalModifiedEnterInput(event);
      if (text === null) return true;
      event.preventDefault();
      sendInput(commandInput(text, null));
      return false;
    });
    const resize = terminal.onResize(({ cols, rows }) => {
      const stream = streamRef.current;
      if (!stream) return;
      const bounds = terminal.element?.querySelector<HTMLElement>(".xterm-screen")?.getBoundingClientRect();
      stream.send({
        type: "terminal.resize",
        cols,
        rows,
        cell_width_px: bounds ? Math.max(1, Math.round(bounds.width / Math.max(1, cols))) : 0,
        cell_height_px: bounds ? Math.max(1, Math.round(bounds.height / Math.max(1, rows))) : 0,
      });
    });
    terminal.attachCustomWheelEventHandler((event) => {
      if (event.deltaY === 0) return true;
      event.preventDefault();
      requestControl();
      const bounds = terminal.element?.querySelector<HTMLElement>(".xterm-screen")?.getBoundingClientRect()
        ?? hostRef.current?.getBoundingClientRect();
      const position = bounds ? terminalCellPosition(event.clientX, event.clientY, bounds, terminal.cols, terminal.rows) : { column: 0, row: 0 };
      const cellHeight = bounds && terminal.rows > 0 ? bounds.height / terminal.rows : (typeof terminal.options.fontSize === "number" ? terminal.options.fontSize : applicationFontSize());
      sendInput({
        type: "terminal.scroll",
        direction: event.deltaY < 0 ? "up" : "down",
        lines: Math.min(65535, Math.max(1, Math.ceil(Math.abs(event.deltaY) / Math.max(1, cellHeight)))),
        source: "wheel",
        column: position.column,
        row: position.row,
        modifiers: terminalModifierBits(event),
      });
      return false;
    });
    return () => {
      data.dispose();
      binary.dispose();
      resize.dispose();
      terminal.attachCustomWheelEventHandler(() => true);
      terminal.attachCustomKeyEventHandler(() => true);
    };
  }, [selected, onSelect]);
  useEffect(() => {
    if (controlAllowed) {
      controlRequestPendingRef.current = false;
      if (!controlRequestedRef.current) {
        controlRequestedRef.current = true;
        setControlRequested(true);
      }
      return;
    }
    if (!controlPending) {
      controlRequestPendingRef.current = false;
      pendingCommands.current = [];
      activeMouseButton.current = null;
    }
    if (controlRequestedRef.current) {
      controlRequestedRef.current = false;
      setControlRequested(false);
    }
  }, [controlAllowed, controlPending]);



  useEffect(() => {
    ownershipRef.current = ownership;
  }, [ownership]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal || !terminalReady) return;
    const geometry = terminalCellGeometry(terminal);
    const openRequest: TerminalOpenRequest = {
      ...request,
      mode: controlRequested ? "control" : "observe",
      takeover: false,
      cols: Math.max(1, Math.min(65535, terminal.cols || 80)),
      rows: Math.max(1, Math.min(65535, terminal.rows || 24)),
      cell_width_px: geometry.cell_width_px,
      cell_height_px: geometry.cell_height_px,
    };
    let cancelled = false;
    let stream: TerminalStream | null = null;
    lastSequence.current = null;
    setError(null);
    setClosed(false);
    ownershipRef.current = controlRequested ? "pending" : "observing";
    setOwnership(ownershipRef.current);
    const fail = (code: string, message: string) => {
      pendingCommands.current = [];
      cancelled = true;
      setError({ code, message });
      ownershipRef.current = "released";
      setOwnership("released");
      streamRef.current = null;
      if (stream) registerStream?.(stream, false);
      stream?.close();
    };
    const onMessage = (message: TerminalStreamMessage) => {
      if (cancelled) return;
      if (message.type === "ownership") {
        ownershipRef.current = message.state;
        setOwnership(message.state);
        if (message.state === "owned") flushPending();
        return;
      }
      if (message.type === "frame") {
        let sequence: bigint;
        try {
          sequence = BigInt(message.seq);
        } catch {
          fail("terminal_sequence", "Terminal sent an invalid sequence");
          return;
        }
        if (lastSequence.current === null && !message.full) {
          fail("terminal_sequence", "Terminal stream must begin with a full frame");
          return;
        }
        if (lastSequence.current !== null && sequence !== lastSequence.current + 1n) {
          fail("terminal_sequence", "Terminal output sequence is not consecutive");
          return;
        }
        lastSequence.current = sequence;
        try {
          const text = decodeFrame(message.bytes);
          terminal.write(text);
        } catch {
          fail("terminal_frame", "Terminal sent an invalid frame");
        }
        return;
      }
      if (message.type === "error" || message.type === "disconnected") {
        fail(message.code, message.message);
      } else if (message.type === "closed") {
        cancelled = true;
        setClosed(true);
        setError(null);
        setOwnership("released");
        streamRef.current = null;
        onClosed?.();
        if (stream) registerStream?.(stream, false);
        stream?.close();
      }
    };
    void client.openTerminal(openRequest, onMessage, (cause: unknown) => {
      if (cancelled) return;
      const typed = cause instanceof Error ? cause : new Error("Could not attach terminal");
      fail((typed as Error & { code?: string }).code ?? "terminal_attach_failed", typed.message);
    }).then((opened) => {
      if (cancelled) {
        opened.close();
        return;
      }
      stream = opened;
      streamRef.current = opened;
      flushPending();
      registerStream?.(opened, true);
    }, (cause: unknown) => {
      if (cancelled) return;
      const typed = cause instanceof Error ? cause : new Error("Could not attach terminal");
      fail((typed as Error & { code?: string }).code ?? "terminal_attach_failed", typed.message);
    });
    return () => {
      cancelled = true;
      if (streamRef.current === stream) streamRef.current = null;
      if (stream) registerStream?.(stream, false);
      stream?.close();
    };
  }, [client, request.session_id, request.pane_id, controlRequested, terminalReady, attempt, registerStream]);

  const sendPointerMouse = (kind: TerminalMouseKind, button: TerminalMouseButton | null, event: React.PointerEvent<HTMLDivElement>) => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const bounds = terminal.element?.querySelector<HTMLElement>(".xterm-screen")?.getBoundingClientRect()
      ?? hostRef.current?.getBoundingClientRect();
    if (!bounds) return;
    forwardTerminalMouse(terminalMouseInput, sendInput, kind, button, event, bounds, terminal.cols, terminal.rows);
  };
  return (
    <div
      className="terminal-host"
      ref={hostRef}
      aria-label={`Terminal ${request.pane_id}`}
      onContextMenu={(event) => { event.preventDefault(); event.stopPropagation(); }}
      onPointerDownCapture={(event) => {
        const button = terminalMouseButton(event.button);
        if (!button) return;
        requestControl();
        if (!terminalMouseInput) return;
        event.preventDefault();
        event.stopPropagation();
        lastMouseMotionAt.current = 0;
        activeMouseButton.current = button;
        event.currentTarget.setPointerCapture(event.pointerId);
        sendPointerMouse("down", button, event);
      }}
      onPointerMoveCapture={(event) => {
        if (event.timeStamp - lastMouseMotionAt.current < 16) return;
        lastMouseMotionAt.current = event.timeStamp;
        const button = activeMouseButton.current;
        sendPointerMouse(button ? "drag" : "moved", button, event);
        if (button && terminalMouseInput) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
      onPointerUpCapture={(event) => {
        const button = terminalMouseButton(event.button);
        if (!button) return;
        if (terminalMouseInput) {
          event.preventDefault();
          event.stopPropagation();
          sendPointerMouse("up", button, event);
        }
        activeMouseButton.current = null;
        lastMouseMotionAt.current = 0;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
      }}
      onPointerCancel={(event) => {
        const button = activeMouseButton.current;
        if (button && terminalMouseInput) {
          event.preventDefault();
          event.stopPropagation();
          sendPointerMouse("up", button, event);
        }
        activeMouseButton.current = null;
        lastMouseMotionAt.current = 0;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
      }}
    >
      {closed ? (
        <div className="terminal-overlay" role="status">
          <span>The terminal process has closed.</span>
          {onClosePane ? <button type="button" className="recovery-button" onClick={onClosePane}>Close pane</button> : null}
          <button type="button" className="recovery-button" onClick={onResync}>Resync</button>
        </div>
      ) : error ? (
        <div className="terminal-overlay" role="alert">
          <span>{error.message}</span>
          <button type="button" className="recovery-button" onClick={() => setAttempt((value) => value + 1)}>Retry</button>
          <button type="button" className="recovery-button" onClick={onResync}>Resync</button>
        </div>
      ) : null}
    </div>
  );
}
