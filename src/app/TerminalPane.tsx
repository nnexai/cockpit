import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import type { CockpitClient, TerminalStream } from "../client/CockpitClient";
import type { TerminalOwnershipState } from "../protocol/generated/v1";
import { shortcutForms } from "./input/shortcuts";
import { createCockpitTerminal, loadGpuRenderer, terminalCellGeometry } from "./terminal/cockpitTerminal";
import { useTerminalRefs, type PaneError, type TerminalPaneRequest } from "./terminal/paneState";
import { useTerminalControl } from "./terminal/useTerminalControl";
import { useTerminalPointer } from "./terminal/useTerminalPointer";
import { useTerminalResize } from "./terminal/useTerminalResize";
import { useTerminalFrameQueue } from "./terminal/useTerminalFrameQueue";
import { useTerminalClipboard, useTerminalClipboardEffects } from "./terminal/terminalClipboard";
import { useXtermInputBindings } from "./terminal/useXtermInputBindings";
import { useTerminalAttachment } from "./terminal/useTerminalAttachment";


export type TerminalPaneProps = {
  client: CockpitClient;
  request: TerminalPaneRequest;
  selected: boolean;
  /** Whether the pane is painted; a hidden element cannot take DOM focus, so a tab switch focuses only once its pane is shown. */
  presented?: boolean;
  /** Confirmed input eligibility, independent of the control-only attachment lifecycle. */
  controlAllowed: boolean;
  controlPending: boolean;
  focusEpoch: number;
  focusToken: number;
  terminalMouseInput: boolean;
  deferAttachment?: boolean;
  /** Whether attachment/control confirmation may move DOM focus into the terminal; false when returning to a connected sidebar invoker. */
  focusOnAttach?: boolean;
  onRequestControl?: () => void;
  onSelect?: () => void;
  onReady?: () => void;
  onResync?: () => void;
  onClosed?: () => void;
  onClosePane?: () => void;
  registerStream?: (stream: TerminalStream, active: boolean) => void;
  accessibleLabel?: string;
  onStateChange?: (state: { ready: boolean; error: string | null }) => void;
  onCellGeometry?: (geometry: { cell_width_px: number; cell_height_px: number }) => void;
};


export function TerminalPane({ client, request, selected, presented = true, controlAllowed, controlPending, focusEpoch, focusToken, terminalMouseInput, deferAttachment = false, focusOnAttach = true, onRequestControl, onSelect, onReady, onResync, onClosed, onClosePane, registerStream, accessibleLabel, onStateChange, onCellGeometry }: TerminalPaneProps) {
  const contextMenuRef = useRef<HTMLDivElement>(null);
  const [ownership, setOwnership] = useState<TerminalOwnershipState>("pending");
  const [error, setError] = useState<PaneError | null>(null);
  const [closed, setClosed] = useState(false);
  const [terminalReady, setTerminalReady] = useState(false);
  const [framePainted, setFramePainted] = useState(false);
  const refs = useTerminalRefs({ selected, presented, focusOnAttach, controlAllowed, controlPending, focusEpoch, focusToken, onCellGeometry, onReady, onRequestControl, onSelect, registerStream }, ownership);
  const { hostRef, fitRef, terminalRef, ownershipRef, onCellGeometryRef, focusOnAttachRef } = refs;
  useEffect(() => {
    onStateChange?.({ ready: framePainted && ownership === "owned" && !closed && !error, error: error?.message ?? (closed ? "The popup terminal process has closed; waiting for Herdr state." : ownership === "lost" || ownership === "conflict" ? "Control taken by another client" : null) });
  }, [framePainted, ownership, closed, error, onStateChange]);
  const control = useTerminalControl(refs, request, setError);
  const pointer = useTerminalPointer(refs, control, terminalMouseInput);
  const sizing = useTerminalResize(refs);
  const frames = useTerminalFrameQueue(refs, sizing, setFramePainted);
  const clipboard = useTerminalClipboard(refs, control);
  const { clipboardError, clipboardBusy, terminalContextOpen, setTerminalContextOpen, terminalContextPosition, setTerminalContextPosition, contextSelectionRef, copySelection, pasteClipboard } = clipboard;

  useEffect(() => {
    if (!terminalContextOpen) return;
    const dismiss = (event: PointerEvent) => {
      if (!contextMenuRef.current?.contains(event.target as Node)) setTerminalContextOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.target instanceof HTMLElement && event.target.closest("[data-server-modal]") && request.target_kind !== "popup") return;
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
    terminal.open(host);
    if (accessibleLabel && terminal.textarea) terminal.textarea.setAttribute("aria-label", accessibleLabel);
    terminal.loadAddon(fit);
    loadGpuRenderer(terminal);
    fitRef.current = fit;
    terminalRef.current = terminal;
    fit.fit();
    onCellGeometryRef.current?.(terminalCellGeometry(terminal));
    sizing.seedGrid(terminal);
    setTerminalReady(true);
    const disposeViewport = sizing.observeViewport(host, terminal, fit);
    return () => {
      disposeViewport();
      terminal.dispose();
      if (fitRef.current === fit) fitRef.current = null;
      if (terminalRef.current === terminal) terminalRef.current = null;
    };
  }, []);

  useTerminalClipboardEffects(clipboard, { controlAllowed, controlPending, focusEpoch, focusToken, ownership });

  useEffect(() => {
    if (selected && presented && controlAllowed && ownership === "owned" && terminalReady && !deferAttachment && focusOnAttachRef.current) terminalRef.current?.focus();
  }, [controlAllowed, deferAttachment, focusEpoch, ownership, presented, selected, terminalReady]);
  useXtermInputBindings(refs, request, control, clipboard, sizing, pointer);
  useEffect(() => {
    if (controlAllowed) {
      control.cancelControlRequest();
      control.flushPending();
    } else if (!controlPending) {
      control.cancelControlRequest();
      control.clearPendingCommands();
      pointer.releaseCapturedPointer();
    }
  }, [controlAllowed, controlPending, selected]);

  useEffect(() => {
    control.dropStaleIntent({ epoch: focusEpoch, paneId: request.pane_id, token: focusToken });
  }, [focusEpoch, focusToken, request.pane_id]);

  useEffect(() => {
    if (!terminalMouseInput) pointer.clearMouseMode();
  }, [terminalMouseInput]);



  useEffect(() => {
    ownershipRef.current = ownership;
  }, [ownership]);
  const { retryAttachment } = useTerminalAttachment({ client, request, deferAttachment, terminalReady, refs, control, pointer, clipboard, sizing, frames, setOwnership, setError, setClosed, setFramePainted, onResync, onClosed });

  return (
    <div
      className="terminal-host"
      ref={hostRef}
      aria-label={accessibleLabel ?? `Terminal ${request.pane_id}`}
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
        contextSelectionRef.current = terminalRef.current?.getSelection() ?? null;
        setTerminalContextPosition({ x: event.clientX, y: event.clientY });
        setTerminalContextOpen(true);
      }}
      {...pointer.handlers}
    >
      {terminalContextOpen && terminalContextPosition ? <div ref={contextMenuRef} className="terminal-context-menu" role="menu" aria-label="Terminal clipboard actions" style={{ left: terminalContextPosition.x, top: terminalContextPosition.y }} onContextMenu={(event) => event.preventDefault()}>
        <button type="button" role="menuitem" disabled={clipboardBusy || !(contextSelectionRef.current || terminalRef.current?.getSelection())} onClick={() => { setTerminalContextOpen(false); void copySelection(); }}>Copy<kbd>{shortcutForms("terminal-copy").at(-1)}</kbd></button>
        <button type="button" role="menuitem" disabled={clipboardBusy} onClick={() => { setTerminalContextOpen(false); void pasteClipboard(); }}>Paste<kbd>{shortcutForms("terminal-paste").at(-1)}</kbd></button>
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
          <button type="button" className="recovery-button" onClick={() => retryAttachment()}>Retry</button>
          <button type="button" className="recovery-button" onClick={onResync}>Resync</button>
        </div>
      ) : ownership === "lost" || ownership === "conflict" ? (
        <div className="terminal-overlay" role="alert">
          <span>Control taken by another client</span>
          <button type="button" className="recovery-button" onClick={() => retryAttachment(true)}>Take control</button>
          <button type="button" className="recovery-button" onClick={() => retryAttachment()}>Retry</button>
        </div>
      ) : null}
    </div>
  );
}
