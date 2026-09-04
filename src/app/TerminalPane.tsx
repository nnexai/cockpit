import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import type { CockpitClient, TerminalStream } from "../client/CockpitClient";
import type { TerminalCommand, TerminalOpenRequest, TerminalOwnershipState, TerminalStreamMessage } from "../protocol/generated/v1";


export const MAX_PENDING_CONTROL_COMMANDS = 64;
export function appendPendingControlCommand(queue: TerminalCommand[], command: TerminalCommand): TerminalCommand[] {
  return queue.length >= MAX_PENDING_CONTROL_COMMANDS
    ? [...queue.slice(queue.length - MAX_PENDING_CONTROL_COMMANDS + 1), command]
    : [...queue, command];
}
export type TerminalPaneProps = {
  client: CockpitClient;
  request: Omit<TerminalOpenRequest, "mode" | "takeover" | "cols" | "rows">;
  selected: boolean;
  controlAllowed: boolean;
  onRequestControl?: () => void;
  onControlLost?: () => void;
  onSelect?: () => void;
  onRetry?: () => void;
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

export function shouldObserveAfterControlLoss(
  controlRequested: boolean,
  ownership: TerminalOwnershipState,
): boolean {
  return controlRequested && (ownership === "conflict" || ownership === "lost");
}

export function TerminalPane({ client, request, selected, controlAllowed, onRequestControl, onControlLost, onSelect, onRetry, onResync, onClosed, onClosePane, registerStream }: TerminalPaneProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const streamRef = useRef<TerminalStream | null>(null);
  const [ownership, setOwnership] = useState<"pending" | "observing" | "owned" | "conflict" | "released" | "lost">("observing");
  const [error, setError] = useState<PaneError | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [closed, setClosed] = useState(false);
  const [terminalReady, setTerminalReady] = useState(false);
  const lastSequence = useRef<bigint | null>(null);
  const ownershipRef = useRef(ownership);
  const pendingCommands = useRef<TerminalCommand[]>([]);
  const [controlRequested, setControlRequested] = useState(controlAllowed);
  const controlRequestedRef = useRef(controlAllowed);
  const flushPending = () => {
    const stream = streamRef.current;
    if (ownershipRef.current !== "owned" || !stream || pendingCommands.current.length === 0) return;
    pendingCommands.current.forEach((command) => stream.send(command));
    pendingCommands.current = [];
  };
  const requestControl = () => {
    terminalRef.current?.focus();
    onRequestControl?.();
    if (!selected) onSelect?.();
    if (controlRequestedRef.current) return;
    controlRequestedRef.current = true;
    setControlRequested(true);
  };

  useEffect(() => {
    if (!hostRef.current) return;
    const terminal = new Terminal({
      convertEol: false,
      customGlyphs: true,
      cursorBlink: false,
      fontFamily: '"IosevkaTerm Nerd Font Mono", "FiraCode Nerd Font Mono", "IBM Plex Mono", "Noto Sans Mono", monospace',
      fontSize: applicationFontSize(),
      theme: { background: "#0c1016", foreground: "#d8dee8" },
      scrollback: 5000,
    });
    terminal.open(hostRef.current);
    let disposed = false;
    let fit: { fit(): void } | null = null;
    void Promise.all([import("@xterm/addon-fit"), import("@xterm/addon-canvas")]).then(([{ FitAddon }, { CanvasAddon }]) => {
      if (disposed) return;
      const addon = new FitAddon();
      fit = addon;
      terminal.loadAddon(addon);
      terminal.loadAddon(new CanvasAddon());
      addon.fit();
      setTerminalReady(true);
    }).catch(() => undefined);
    terminalRef.current = terminal;
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => fit?.fit());
    observer?.observe(hostRef.current);
    return () => {
      disposed = true;
      observer?.disconnect();
      terminal.dispose();
      terminalRef.current = null;
    };
  }, []);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const sendInput = (command: TerminalCommand) => {
      if (ownershipRef.current === "owned" && streamRef.current) streamRef.current.send(command);
      else if (controlRequestedRef.current) pendingCommands.current = appendPendingControlCommand(pendingCommands.current, command);
    };
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
      if (ownershipRef.current !== "owned" || !streamRef.current) return;
      const bounds = terminal.element?.querySelector<HTMLElement>(".xterm-screen")?.getBoundingClientRect();
      streamRef.current.send({
        type: "terminal.resize",
        cols,
        rows,
        cell_width_px: bounds ? Math.max(0, Math.round(bounds.width / Math.max(1, cols))) : 0,
        cell_height_px: bounds ? Math.max(0, Math.round(bounds.height / Math.max(1, rows))) : 0,
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
        modifiers: (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0),
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
    if (controlAllowed && !controlRequestedRef.current) {
      controlRequestedRef.current = true;
      setControlRequested(true);
    }
  }, [controlAllowed]);



  useEffect(() => {
    ownershipRef.current = ownership;
  }, [ownership]);

  useEffect(() => {
    if (ownership === "owned") flushPending();
  }, [ownership]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal || !terminalReady) return;
    const mode = controlRequested ? "control" : "observe";
    const openRequest: TerminalOpenRequest = {
      ...request,
      mode,
      takeover: controlRequested,
      cols: Math.max(1, Math.min(65535, terminal.cols || 80)),
      rows: Math.max(1, Math.min(65535, terminal.rows || 24)),
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
    const observeAfterControlLoss = () => {
      pendingCommands.current = [];
      cancelled = true;
      setError(null);
      setClosed(false);
      ownershipRef.current = "observing";
      setOwnership("observing");
      streamRef.current = null;
      if (stream) registerStream?.(stream, false);
      stream?.close();
      controlRequestedRef.current = false;
      setControlRequested(false);
      onControlLost?.();
    };
    const onMessage = (message: TerminalStreamMessage) => {
      if (cancelled) return;
      if (message.type === "ownership") {
        if (message.state === "conflict" || message.state === "lost") {
          if (shouldObserveAfterControlLoss(controlRequested, message.state)) observeAfterControlLoss();
          else fail(message.state, message.message ?? "Terminal control is unavailable");
        } else {
          ownershipRef.current = message.state;
          setOwnership(message.state);
          if (message.state === "owned") flushPending();
        }
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
        if (!message.full && sequence !== lastSequence.current! + 1n) {
          fail("terminal_sequence", "Terminal output sequence is not consecutive");
          return;
        }
        lastSequence.current = sequence;
        try {
          const text = decodeFrame(message.bytes);
          if (message.full) {
            terminal.reset();
          }
          terminal.write(text);
          if (controlRequested) { ownershipRef.current = "owned"; setOwnership("owned"); flushPending(); }
        } catch {
          fail("terminal_frame", "Terminal sent an invalid frame");
        }
        return;
      }
      if (message.type === "error") {
        fail(message.code, message.message);
      } else if (message.type === "disconnected") {
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
      if (cancelled) { opened.close(); return; }
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

  return (
    <div className="terminal-host" ref={hostRef} aria-label={`Terminal ${request.pane_id}`} onPointerDownCapture={(event) => { if (event.button === 0) requestControl(); }}>
      {closed ? (
        <div className="terminal-overlay" role="status">
          <span>The terminal process has closed.</span>
          {onClosePane ? <button type="button" className="recovery-button" onClick={onClosePane}>Close pane</button> : null}
          <button type="button" className="recovery-button" onClick={onResync}>Resync</button>
        </div>
      ) : error ? (
        <div className="terminal-overlay" role="alert">
          <span>{error.message}</span>
          <button type="button" className="recovery-button" onClick={() => {
            setAttempt((value) => value + 1);
            if (error.code === "conflict" || error.code === "lost") onRetry?.();
          }}>{error.code === "conflict" || error.code === "lost" ? "Retry control" : "Retry"}</button>
          <button type="button" className="recovery-button" onClick={onResync}>Resync</button>
        </div>
      ) : null}
    </div>
  );
}
