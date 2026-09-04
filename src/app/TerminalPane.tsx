import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import type { CockpitClient, TerminalStream } from "../client/CockpitClient";
import type { TerminalCommand, TerminalOpenRequest, TerminalStreamMessage } from "../protocol/generated/v1";


export const MAX_PENDING_CONTROL_COMMANDS = 64;
export function appendPendingControlCommand(queue: TerminalCommand[], command: TerminalCommand): TerminalCommand[] {
  return queue.length >= MAX_PENDING_CONTROL_COMMANDS
    ? [...queue.slice(queue.length - MAX_PENDING_CONTROL_COMMANDS + 1), command]
    : [...queue, command];
}
export type TerminalPaneProps = {
  client: CockpitClient;
  request: Omit<TerminalOpenRequest, "mode" | "takeover" | "cols" | "rows">;
  controlAllowed: boolean;
  pendingControl?: boolean;
  onRelease?: () => void;
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

function commandInput(text: string | null, bytes: string | null): TerminalCommand {
  return { type: "terminal.input", text, bytes };
}
export function TerminalPane({ client, request, controlAllowed, pendingControl = false, onRelease, onRetry, onResync, onClosed, onClosePane, registerStream }: TerminalPaneProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const streamRef = useRef<TerminalStream | null>(null);
  const [ownership, setOwnership] = useState<"pending" | "observing" | "owned" | "conflict" | "released" | "lost">("observing");
  const [error, setError] = useState<PaneError | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [closed, setClosed] = useState(false);
  const lastSequence = useRef<bigint | null>(null);
  const ownershipRef = useRef(ownership);
  const pendingCommands = useRef<TerminalCommand[]>([]);
  const flushPending = () => {
    const stream = streamRef.current;
    if (ownershipRef.current !== "owned" || !stream || pendingCommands.current.length === 0) return;
    pendingCommands.current.forEach((command) => stream.send(command));
    pendingCommands.current = [];
  };

  useEffect(() => {
    if (!hostRef.current) return;
    const terminal = new Terminal({
      convertEol: false,
      cursorBlink: false,
      fontFamily: '"IosevkaTerm Nerd Font Mono", "FiraCode Nerd Font Mono", "IBM Plex Mono", "Noto Sans Mono", monospace',
      fontSize: 13,
      theme: { background: "#0c1016", foreground: "#d8dee8" },
      scrollback: 5000,
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(hostRef.current);
    fit.fit();
    terminalRef.current = terminal;
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => fit.fit());
    observer?.observe(hostRef.current);
    return () => {
      observer?.disconnect();
      terminal.dispose();
      terminalRef.current = null;
    };
  }, []);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const sendInput = (command: TerminalCommand) => {
      if (ownership === "owned" && streamRef.current) streamRef.current.send(command);
      else if (pendingControl) pendingCommands.current = appendPendingControlCommand(pendingCommands.current, command);
    };
    const data = terminal.onData((text) => sendInput(commandInput(text, null)));
    const binary = terminal.onBinary((bytes) => sendInput(commandInput(null, btoa(bytes))));
    const resize = terminal.onResize(({ cols, rows }) => {
      if (ownership === "owned" && streamRef.current) streamRef.current.send({ type: "terminal.resize", cols, rows, cell_width_px: 0, cell_height_px: 0 });
    });
    const wheel = (event: WheelEvent) => {
      if (ownership !== "owned" || !streamRef.current || event.deltaY === 0) return;
      event.preventDefault();
      const fontSize = typeof terminal.options.fontSize === "number" ? terminal.options.fontSize : 13;
      const lines = Math.min(65535, Math.max(1, Math.ceil(Math.abs(event.deltaY) / Math.max(1, fontSize))));
      streamRef.current.send({
        type: "terminal.scroll",
        direction: event.deltaY < 0 ? "up" : "down",
        lines,
        source: "wheel",
        column: null,
        row: null,
        modifiers: 0,
      });
    };
    hostRef.current?.addEventListener("wheel", wheel, { passive: false });
    return () => {
      data.dispose();
      binary.dispose();
      resize.dispose();
      hostRef.current?.removeEventListener("wheel", wheel);
    };
  }, [ownership, pendingControl]);

  useEffect(() => {
    if (!pendingControl && !controlAllowed && ownership !== "owned") pendingCommands.current = [];
  }, [pendingControl, controlAllowed, ownership]);

  useEffect(() => {
    ownershipRef.current = ownership;
  }, [ownership]);

  useEffect(() => {
    if (ownership === "owned") flushPending();
  }, [ownership]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (pendingControl && !controlAllowed) {
      setError(null);
      ownershipRef.current = "pending";
      setOwnership("pending");
      return;
    }
    if (!terminal) return;
    const mode = controlAllowed ? "control" : "observe";
    const openRequest: TerminalOpenRequest = {
      ...request,
      mode,
      takeover: controlAllowed,
      cols: Math.max(1, Math.min(65535, terminal.cols || 80)),
      rows: Math.max(1, Math.min(65535, terminal.rows || 24)),
    };
    let cancelled = false;
    let stream: TerminalStream | null = null;
    lastSequence.current = null;
    setError(null);
    setClosed(false);
    ownershipRef.current = controlAllowed ? "pending" : "observing";
    setOwnership(ownershipRef.current);
    const fail = (code: string, message: string, releaseControl = false) => {
      pendingCommands.current = [];
      cancelled = true;
      setError({ code, message });
      ownershipRef.current = "released";
      setOwnership("released");
      streamRef.current = null;
      if (releaseControl) onRelease?.();
      if (stream) registerStream?.(stream, false);
      stream?.close();
    };
    const onMessage = (message: TerminalStreamMessage) => {
      if (cancelled) return;
      if (message.type === "ownership") {
        if (message.state === "conflict" || message.state === "lost") {
          fail(message.state, message.message ?? "Terminal control is unavailable");
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
          if (controlAllowed) { ownershipRef.current = "owned"; setOwnership("owned"); flushPending(); }
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
        onRelease?.();
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
      const shouldRelease = controlAllowed && stream !== null && !cancelled;
      cancelled = true;
      if (shouldRelease && stream) {
        try {
          stream.send({ type: "terminal.release" });
        } catch {
          // The transport may already have closed during cancellation.
        }
        onRelease?.();
      }
      if (streamRef.current === stream) streamRef.current = null;
      if (stream) registerStream?.(stream, false);
      stream?.close();
    };
  }, [client, request.session_id, request.pane_id, controlAllowed, pendingControl, attempt, registerStream]);

  return (
    <div className="terminal-host" ref={hostRef} aria-label={`Terminal ${request.pane_id}`}>
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
            if (error.code === "conflict" || error.code === "lost") onRetry?.();
            else setAttempt((value) => value + 1);
          }}>{error.code === "conflict" || error.code === "lost" ? "Retry control" : "Retry"}</button>
          <button type="button" className="recovery-button" onClick={onResync}>Resync</button>
        </div>
      ) : null}
    </div>
  );
}
