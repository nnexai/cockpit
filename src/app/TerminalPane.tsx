import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import type { CockpitClient, TerminalStream } from "../client/CockpitClient";
import type { TerminalCommand, TerminalMouseButton, TerminalMouseKind, TerminalOpenRequest, TerminalOwnershipState, TerminalStreamMessage } from "../protocol/generated/v1";
import { createClipboardAccess, type ClipboardAccess } from "../client/clipboard";


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
  focusEpoch: number;
  focusToken: number;
  terminalMouseInput: boolean;
  deferAttachment?: boolean;
  onRequestControl?: () => void;
  onSelect?: () => void;
  onReady?: () => void;
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
    fontFamily: 'ui-monospace, "FiraCode Nerd Font Mono", "Hack Nerd Font Mono", "IBM Plex Mono", "Noto Sans Mono", monospace',
    fontSize,
    lineHeight: 1,
    // Herdr owns terminal scroll position. A local full-height scrollbar has
    // no authoritative position and reads as a second pane divider.
    scrollbar: { showScrollbar: false, width: 8 },
    theme: { background: "#0c1016", foreground: "#d8dee8" },
    scrollback: 5000,
    vtExtensions: { kittyKeyboard: true },
  });
}

export async function copyTerminalSelection(terminal: Pick<Terminal, "getSelection">, clipboard: ClipboardAccess = createClipboardAccess()): Promise<boolean> {
  const selection = terminal.getSelection();
  if (!selection) return false;
  await clipboard.writeText(selection);
  return true;
}

export async function readTerminalClipboard(clipboard: ClipboardAccess = createClipboardAccess()): Promise<string> {
  return clipboard.readText();
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
type TerminalResize = Extract<TerminalCommand, { type: "terminal.resize" }>;

export function TerminalPane({ client, request, selected, controlAllowed, controlPending, focusEpoch, focusToken, terminalMouseInput, deferAttachment = false, onRequestControl, onSelect, onReady, onResync, onClosed, onClosePane, registerStream }: TerminalPaneProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const contextMenuRef = useRef<HTMLDivElement>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const lastResizeRef = useRef<TerminalResize | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const streamRef = useRef<TerminalStream | null>(null);
  const attachmentGeneration = useRef(0);
  const [ownership, setOwnership] = useState<TerminalOwnershipState>("observing");
  const [error, setError] = useState<PaneError | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [closed, setClosed] = useState(false);
  const [terminalReady, setTerminalReady] = useState(false);
  const [clipboardError, setClipboardError] = useState<{ operation: "copy" | "paste"; message: string } | null>(null);
  const [clipboardBusy, setClipboardBusy] = useState(false);
  const [terminalContextOpen, setTerminalContextOpen] = useState(false);
  const [terminalContextPosition, setTerminalContextPosition] = useState<{ x: number; y: number } | null>(null);
  const contextSelectionRef = useRef<string | null>(null);
  const lastSequence = useRef<bigint | null>(null);
  const ownershipRef = useRef(ownership);
  const pendingCommands = useRef<TerminalCommand[]>([]);
  const pendingIntentRef = useRef<{ epoch: number; paneId: string; token: number } | null>(null);
  const [controlRequested, setControlRequested] = useState(controlAllowed);
  const controlRequestedRef = useRef(controlAllowed);
  const controlRequestPendingRef = useRef(false);
  const takeoverRequestedRef = useRef(false);
  const controlAllowedRef = useRef(controlAllowed);
  const pendingPasteRef = useRef<{ text: string; intent: { epoch: number; paneId: string; token: number } } | null>(null);
  controlAllowedRef.current = controlAllowed;
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const onRequestControlRef = useRef(onRequestControl);
  onRequestControlRef.current = onRequestControl;
  const onSelectRef = useRef(onSelect);
  const onReadyRef = useRef(onReady);
  onReadyRef.current = onReady;
  onSelectRef.current = onSelect;
  const controlPendingRef = useRef(controlPending);
  controlPendingRef.current = controlPending;
  const focusEpochRef = useRef(focusEpoch);
  focusEpochRef.current = focusEpoch;
  const focusTokenRef = useRef(focusToken);
  focusTokenRef.current = focusToken;
  const activeMousePointer = useRef<{ button: TerminalMouseButton; pointerId: number } | null>(null);
  const mouseModeRef = useRef(false);
  const lastMouseMotionAt = useRef(0);
  const currentIntent = () => ({ epoch: focusEpochRef.current, paneId: request.pane_id, token: focusTokenRef.current });
  const clearPendingCommands = () => {
    pendingCommands.current = [];
    pendingIntentRef.current = null;
  };
  const sendInput = (command: TerminalCommand) => {
    const hasSelectedFocusIntent = selectedRef.current && focusTokenRef.current > 0;
    if (!controlAllowedRef.current && !controlRequestPendingRef.current && !controlPendingRef.current && !hasSelectedFocusIntent) return;
    if (controlAllowedRef.current && ownershipRef.current === "owned" && streamRef.current) streamRef.current.send(command);
    else {
      const intent = currentIntent();
      if (pendingIntentRef.current
        && (pendingIntentRef.current.epoch !== intent.epoch
          || pendingIntentRef.current.paneId !== intent.paneId
          || pendingIntentRef.current.token !== intent.token)) {
        pendingCommands.current = [];
      }
      pendingIntentRef.current = intent;
      pendingCommands.current = appendPendingControlCommand(pendingCommands.current, command);
    }
  };
  const releaseCapturedPointer = () => {
    const active = activeMousePointer.current;
    if (!active) return;
    const host = hostRef.current;
    if (host?.hasPointerCapture(active.pointerId)) host.releasePointerCapture(active.pointerId);
    activeMousePointer.current = null;
    lastMouseMotionAt.current = 0;
  };
  const clearMouseMode = () => {
    mouseModeRef.current = false;
    pendingCommands.current = pendingCommands.current.filter((command) => command.type !== "terminal.mouse");
    releaseCapturedPointer();
  };
  const flushPending = () => {
    const stream = streamRef.current;
    const intent = pendingIntentRef.current;
    const current = currentIntent();
    if (!controlAllowedRef.current || !controlRequestedRef.current || ownershipRef.current !== "owned" || !stream || pendingCommands.current.length === 0
      || !intent || intent.epoch !== current.epoch || intent.paneId !== current.paneId || intent.token !== current.token) return;
    pendingCommands.current.forEach((command) => stream.send(command));
    clearPendingCommands();
  };
  const requestControl = () => {
    terminalRef.current?.focus();
    onRequestControlRef.current?.();
    if (!selectedRef.current) onSelectRef.current?.();
    if (controlAllowedRef.current && controlRequestedRef.current && ownershipRef.current === "owned") return;
    takeoverRequestedRef.current = true;
    controlRequestPendingRef.current = true;
    controlRequestedRef.current = controlAllowedRef.current;
    setControlRequested(controlAllowedRef.current);
  };

  const flushPendingPaste = () => {
    const pending = pendingPasteRef.current;
    const terminal = terminalRef.current;
    if (!pending || !terminal) return;
    const current = currentIntent();
    if (pending.intent.epoch !== current.epoch || pending.intent.paneId !== current.paneId) {
      pendingPasteRef.current = null;
      return;
    }
    if (pending.intent.token !== current.token) {
      if (!controlPendingRef.current && !controlAllowedRef.current) {
        pendingPasteRef.current = null;
        return;
      }
      pending.intent = current;
    }
    if (!controlAllowedRef.current || ownershipRef.current !== "owned" || !streamRef.current) return;
    pendingPasteRef.current = null;
    terminal.paste(pending.text);
  };

  const copySelection = async () => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const capturedSelection = contextSelectionRef.current;
    contextSelectionRef.current = null;
    setClipboardBusy(true);
    setClipboardError(null);
    try {
      if (capturedSelection !== null) {
        if (capturedSelection) await createClipboardAccess().writeText(capturedSelection);
      } else {
        await copyTerminalSelection(terminal);
      }
    } catch (cause) {
      setClipboardError({ operation: "copy", message: cause instanceof Error ? cause.message : "Could not copy terminal selection" });
    } finally {
      setClipboardBusy(false);
    }
  };

  const pasteClipboard = async () => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    requestControl();
    const intent = currentIntent();
    setClipboardBusy(true);
    setClipboardError(null);
    try {
      const text = await readTerminalClipboard();
      pendingPasteRef.current = { text, intent };
      flushPendingPaste();
    } catch (cause) {
      setClipboardError({ operation: "paste", message: cause instanceof Error ? cause.message : "Could not read the clipboard" });
    } finally {
      setClipboardBusy(false);
    }
  };

  useEffect(() => {
    if (!terminalContextOpen) return;
    const dismiss = (event: PointerEvent) => {
      if (!contextMenuRef.current?.contains(event.target as Node)) setTerminalContextOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setTerminalContextOpen(false);
    };
    window.addEventListener("pointerdown", dismiss);
    window.addEventListener("keydown", escape);
    contextMenuRef.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    return () => {
      window.removeEventListener("pointerdown", dismiss);
      window.removeEventListener("keydown", escape);
    };
  }, [terminalContextOpen]);

  useLayoutEffect(() => {
    if (!terminalContextOpen || !terminalContextPosition) return;
    const bounds = contextMenuRef.current?.getBoundingClientRect();
    if (!bounds) return;
    const gutter = 8;
    const x = Math.max(gutter, Math.min(terminalContextPosition.x, window.innerWidth - bounds.width - gutter));
    const y = Math.max(gutter, Math.min(terminalContextPosition.y, window.innerHeight - bounds.height - gutter));
    if (x === terminalContextPosition.x && y === terminalContextPosition.y) return;
    setTerminalContextPosition({ x, y });
  }, [terminalContextOpen, terminalContextPosition]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const terminal = createCockpitTerminal();
    const fit = new FitAddon();
    let resizeTimer: number | null = null;
    let disposed = false;
    terminal.open(host);
    terminal.loadAddon(fit);
    fitRef.current = fit;
    fit.fit();
    setTerminalReady(true);
    terminalRef.current = terminal;
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => {
      if (resizeTimer !== null) window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(() => {
        resizeTimer = null;
        if (!disposed && terminalRef.current === terminal) fit.fit();
      }, 100);
    });
    observer?.observe(host);
    return () => {
      disposed = true;
      observer?.disconnect();
      if (resizeTimer !== null) window.clearTimeout(resizeTimer);
      terminal.dispose();
      if (fitRef.current === fit) fitRef.current = null;
      if (terminalRef.current === terminal) terminalRef.current = null;
    };
  }, []);
  useEffect(() => {
    if (deferAttachment && terminalReady) onReadyRef.current?.();
  }, [deferAttachment, terminalReady]);

  useEffect(() => {
    flushPendingPaste();
  }, [controlAllowed, controlPending, focusEpoch, focusToken, ownership]);

  useEffect(() => () => {
    pendingPasteRef.current = null;
  }, []);

  useEffect(() => {
    if (selected && terminalReady && !deferAttachment) terminalRef.current?.focus();
  }, [deferAttachment, selected, terminalReady]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const data = terminal.onData((text) => sendInput(commandInput(text, null)));
    const binary = terminal.onBinary((bytes) => sendInput(commandInput(null, btoa(bytes))));
    terminal.attachCustomKeyEventHandler((event) => {
      const key = event.key.toLowerCase();
      const shortcutModifier = event.ctrlKey || event.metaKey;
      if (event.type === "keydown" && shortcutModifier && event.shiftKey && key === "c" && !event.altKey) {
        if (!terminal.hasSelection()) return true;
        void copySelection();
        event.preventDefault();
        return false;
      }
      if (event.type === "keydown" && shortcutModifier && event.shiftKey && key === "v" && !event.altKey) {
        event.preventDefault();
        void pasteClipboard();
        return false;
      }
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
      const resizeCommand: TerminalResize = {
        type: "terminal.resize",
        cols,
        rows,
        cell_width_px: bounds ? Math.max(1, Math.round(bounds.width / Math.max(1, cols))) : 0,
        cell_height_px: bounds ? Math.max(1, Math.round(bounds.height / Math.max(1, rows))) : 0,
      };
      const previous = lastResizeRef.current;
      if (
        previous
        && previous.cols === resizeCommand.cols
        && previous.rows === resizeCommand.rows
        && previous.cell_width_px === resizeCommand.cell_width_px
        && previous.cell_height_px === resizeCommand.cell_height_px
      ) return;
      lastResizeRef.current = resizeCommand;
      stream.send(resizeCommand);
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
  }, []);
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
      if (!controlAllowed) clearPendingCommands();
      clearMouseMode();
    }
    if (controlRequestedRef.current) {
      controlRequestedRef.current = false;
      setControlRequested(false);
    }
  }, [controlAllowed, controlPending]);

  useEffect(() => {
    const intent = pendingIntentRef.current;
    if (!intent) return;
    if (intent.epoch !== focusEpoch || intent.paneId !== request.pane_id || intent.token !== focusToken) clearPendingCommands();
  }, [focusEpoch, focusToken, request.pane_id]);

  useEffect(() => {
    if (!terminalMouseInput) clearMouseMode();
  }, [terminalMouseInput]);



  useEffect(() => {
    ownershipRef.current = ownership;
  }, [ownership]);
  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal || !terminalReady || deferAttachment) return;
    const restoreFocus = selectedRef.current && controlAllowedRef.current;
    const geometry = terminalCellGeometry(terminal);
    // The authoritative snapshot can clear a focus transition one render
    // before the local control request state catches up. Open the confirmed
    // target directly in control mode so an observe handshake is not started
    // only to be aborted on the next render.
    const wantsControl = (controlRequested || controlAllowed) && ownership !== "conflict" && ownership !== "lost";
    const openRequest: TerminalOpenRequest = {
      ...request,
      mode: wantsControl ? "control" : "observe",
      takeover: wantsControl && takeoverRequestedRef.current,
      cols: Math.max(1, Math.min(65535, terminal.cols || 80)),
      rows: Math.max(1, Math.min(65535, terminal.rows || 24)),
      cell_width_px: geometry.cell_width_px,
      cell_height_px: geometry.cell_height_px,
    };
    lastResizeRef.current = {
      type: "terminal.resize",
      cols: openRequest.cols,
      rows: openRequest.rows,
      cell_width_px: openRequest.cell_width_px,
      cell_height_px: openRequest.cell_height_px,
    };
    let cancelled = false;
    let stream: TerminalStream | null = null;
    let observedStreamId: string | null = null;
    const controller = new AbortController();
    const generation = ++attachmentGeneration.current;
    clearMouseMode();
    lastSequence.current = null;
    setError(null);
    setClosed(false);
    ownershipRef.current = controlRequested ? "pending" : "observing";
    setOwnership(ownershipRef.current);
    const fail = (code: string, message: string) => {
      clearPendingCommands();
      clearMouseMode();
      cancelled = true;
      controller.abort();
      onReadyRef.current?.();
      setError({ code, message });
      ownershipRef.current = "released";
      setOwnership("released");
      streamRef.current = null;
      if (stream) registerStream?.(stream, false);
      stream?.close();
    };
    const onMessage = (message: TerminalStreamMessage) => {
      if (cancelled || generation !== attachmentGeneration.current) return;
      if (message.type !== "mouse_mode" && observedStreamId === null) observedStreamId = message.stream_id;
      if (message.type === "mouse_mode") {
        if (
          ownershipRef.current === "lost" ||
          ownershipRef.current === "released" ||
          ownershipRef.current === "conflict" ||
          message.session_id !== request.session_id ||
          message.pane_id !== request.pane_id ||
          (observedStreamId !== null && message.stream_id !== observedStreamId)
        ) return;
        observedStreamId = message.stream_id;
        if (message.enabled) mouseModeRef.current = true;
        else clearMouseMode();
        return;
      }
      if (message.type === "ownership") {
        if (message.state === "lost" || message.state === "conflict") {
          clearPendingCommands();
          clearMouseMode();
          controlRequestPendingRef.current = false;
          controlRequestedRef.current = false;
          takeoverRequestedRef.current = false;
          setControlRequested(false);
        } else if (message.state === "released") clearMouseMode();
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
          // xterm can briefly detach its hidden textarea while applying a full
          // screen frame. Restore only that lost terminal focus, never override
          // an explicit focus change to another control.
          const retainedFocus = terminal.element?.contains(document.activeElement) ?? false;
          terminal.write(text, () => {
            if (retainedFocus && terminalRef.current === terminal && document.activeElement === document.body) terminal.focus();
          });
        } catch {
          fail("terminal_frame", "Terminal sent an invalid frame");
        }
        return;
      }
      if (message.type === "error" || message.type === "disconnected") {
        fail(message.code, message.message);
      } else if (message.type === "closed") {
        cancelled = true;
        clearMouseMode();
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
    }, controller.signal).then((opened) => {
      if (cancelled || generation !== attachmentGeneration.current) {
        opened.close();
        return;
      }
      stream = opened;
      streamRef.current = opened;
      if (!cancelled && generation === attachmentGeneration.current && terminalRef.current === terminal && fitRef.current) {
        fitRef.current.fit();
      }
      if (restoreFocus) terminal.focus();
      onReadyRef.current?.();
      flushPending();
      registerStream?.(opened, true);
    }, (cause: unknown) => {
      if (cancelled) return;
      const typed = cause instanceof Error ? cause : new Error("Could not attach terminal");
      fail((typed as Error & { code?: string }).code ?? "terminal_attach_failed", typed.message);
    });
    return () => {
      cancelled = true;
      clearMouseMode();
      controller.abort();
      if (streamRef.current === stream) streamRef.current = null;
      if (stream) registerStream?.(stream, false);
      stream?.close();
    };
  }, [client, deferAttachment, request.session_id, request.pane_id, controlAllowed, controlRequested, terminalReady, attempt, registerStream]);

  const sendPointerMouse = (kind: TerminalMouseKind, button: TerminalMouseButton | null, event: React.PointerEvent<HTMLDivElement>) => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const bounds = terminal.element?.querySelector<HTMLElement>(".xterm-screen")?.getBoundingClientRect()
      ?? hostRef.current?.getBoundingClientRect();
    if (!bounds) return;
    forwardTerminalMouse(terminalMouseInput && mouseModeRef.current, sendInput, kind, button, event, bounds, terminal.cols, terminal.rows);
  };
  return (
    <div
      className="terminal-host"
      ref={hostRef}
      aria-label={`Terminal ${request.pane_id}`}
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
        contextSelectionRef.current = terminalRef.current?.getSelection() ?? null;
        setTerminalContextPosition({ x: event.clientX, y: event.clientY });
        setTerminalContextOpen(true);
      }}
      onPointerDownCapture={(event) => {
        const button = terminalMouseButton(event.button);
        if (!button) return;
        requestControl();
        // Shift is xterm's conventional selection override for app mouse mode.
        if (!terminalMouseInput || !mouseModeRef.current || event.shiftKey) return;
        event.preventDefault();
        event.stopPropagation();
        lastMouseMotionAt.current = 0;
        activeMousePointer.current = { button, pointerId: event.pointerId };
        event.currentTarget.setPointerCapture(event.pointerId);
        sendPointerMouse("down", button, event);
      }}
      onPointerMoveCapture={(event) => {
        const active = activeMousePointer.current;
        if (!active || active.pointerId !== event.pointerId) return;
        if (!terminalMouseInput || !mouseModeRef.current) {
          clearMouseMode();
          return;
        }
        if (event.timeStamp - lastMouseMotionAt.current < 16) return;
        lastMouseMotionAt.current = event.timeStamp;
        sendPointerMouse("drag", active.button, event);
        event.preventDefault();
        event.stopPropagation();
      }}
      onPointerUpCapture={(event) => {
        const active = activeMousePointer.current;
        if (!active || active.pointerId !== event.pointerId) return;
        if (terminalMouseInput && mouseModeRef.current) {
          event.preventDefault();
          event.stopPropagation();
          sendPointerMouse("up", active.button, event);
        }
        releaseCapturedPointer();
      }}
      onPointerCancel={(event) => {
        const active = activeMousePointer.current;
        if (!active || active.pointerId !== event.pointerId) return;
        if (terminalMouseInput && mouseModeRef.current) {
          event.preventDefault();
          event.stopPropagation();
          sendPointerMouse("up", active.button, event);
        }
        releaseCapturedPointer();
      }}
    >
      {terminalContextOpen && terminalContextPosition ? <div ref={contextMenuRef} className="terminal-context-menu" role="menu" aria-label="Terminal clipboard actions" style={{ left: terminalContextPosition.x, top: terminalContextPosition.y }} onContextMenu={(event) => event.preventDefault()}>
        <button type="button" role="menuitem" disabled={clipboardBusy || !(contextSelectionRef.current || terminalRef.current?.getSelection())} onClick={() => { setTerminalContextOpen(false); void copySelection(); }}>Copy</button>
        <button type="button" role="menuitem" disabled={clipboardBusy} onClick={() => { setTerminalContextOpen(false); void pasteClipboard(); }}>Paste</button>
      </div> : null}
      {clipboardBusy ? <span className="terminal-status" role="status" aria-label="Clipboard operation in progress" title="Clipboard operation in progress">⟳</span> : clipboardError ? <span className="terminal-status terminal-status-error" role="alert" aria-label={clipboardError.message} title={clipboardError.message}><span aria-hidden="true">!</span><button type="button" className="terminal-status-retry" aria-label={`Retry ${clipboardError.operation}`} onClick={() => { void (clipboardError.operation === "copy" ? copySelection() : pasteClipboard()); }}>↻</button></span> : null}
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
