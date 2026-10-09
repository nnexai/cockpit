import { useRef } from "react";
import type { FitAddon } from "@xterm/addon-fit";
import type { Terminal } from "@xterm/xterm";
import type { TerminalCommand, TerminalOpenRequest } from "../../protocol/generated/v1";
import type { TerminalRefs } from "./paneState";
import { terminalCellGeometry, terminalScreenBounds, validTerminalGrid, type TerminalGrid } from "./cockpitTerminal";
import { snapTerminalFontSize } from "./terminalTheme";

/** How long a resize may go unanswered before the next one is sent anyway. */
const RESIZE_ANSWER_TIMEOUT_MS = 150;
type TerminalResize = Extract<TerminalCommand, { type: "terminal.resize" }>;
function sameResize(previous: TerminalResize | null, command: TerminalResize): boolean {
  return previous !== null && previous.cols === command.cols && previous.rows === command.rows
    && previous.cell_width_px === command.cell_width_px && previous.cell_height_px === command.cell_height_px;
}

export function useTerminalResize(refs: TerminalRefs) {
  const { fitRef, terminalRef, streamRef, ownershipRef, onCellGeometryRef } = refs;
  const lastResizeRef = useRef<TerminalResize | null>(null);
  const desiredViewportGridRef = useRef<TerminalGrid | null>(null);
  const renderedGridRef = useRef<TerminalGrid | null>(null);
  const authoritativeFrameRef = useRef(false);
  const suppressFrameResizeRef = useRef(false);
  const resizeInFlightRef = useRef<{ grid: TerminalGrid; timer: number } | null>(null);
  const resizeFollowUpRef = useRef(false);
  const settleResize = () => {
    const inFlight = resizeInFlightRef.current;
    if (inFlight) window.clearTimeout(inFlight.timer);
    resizeInFlightRef.current = null;
    if (!resizeFollowUpRef.current) return;
    resizeFollowUpRef.current = false;
    requestViewportSizing();
  };
  const requestViewportSizing = () => {
    const fit = fitRef.current;
    const terminal = terminalRef.current;
    if (!fit || !terminal) return;
    const proposed = fit.proposeDimensions();
    if (!proposed || !validTerminalGrid(proposed.cols, proposed.rows)) return;
    desiredViewportGridRef.current = { cols: proposed.cols, rows: proposed.rows };
    if (!authoritativeFrameRef.current) {
      fit.fit();
      desiredViewportGridRef.current = { cols: terminal.cols, rows: terminal.rows };
      renderedGridRef.current = { cols: terminal.cols, rows: terminal.rows };
      return;
    }
    const stream = streamRef.current;
    if (!stream || ownershipRef.current !== "owned") return;
    const geometry = terminalCellGeometry(terminal, renderedGridRef.current ?? { cols: terminal.cols, rows: terminal.rows });
    const resizeCommand: TerminalResize = {
      type: "terminal.resize",
      cols: proposed.cols,
      rows: proposed.rows,
      cell_width_px: geometry.cell_width_px,
      cell_height_px: geometry.cell_height_px,
    };
    const previous = lastResizeRef.current;
    if (sameResize(previous, resizeCommand)) return;
    // One resize at a time: Herdr answers each with a frame at the new grid,
    // and sizes asked for in between are folded into the next request.
    if (resizeInFlightRef.current) {
      resizeFollowUpRef.current = true;
      return;
    }
    lastResizeRef.current = resizeCommand;
    stream.send(resizeCommand);
    resizeInFlightRef.current = {
      grid: { cols: resizeCommand.cols, rows: resizeCommand.rows },
      timer: window.setTimeout(settleResize, RESIZE_ANSWER_TIMEOUT_MS),
    };
  };
  const seedGrid = (terminal: Terminal) => {
    const initialGrid = { cols: terminal.cols, rows: terminal.rows };
    desiredViewportGridRef.current = initialGrid;
    renderedGridRef.current = initialGrid;
  };
  const observeViewport = (host: HTMLElement, terminal: Terminal, fit: FitAddon) => {
    let resizeFrame: number | null = null;
    let disposed = false;
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => {
      if (resizeFrame !== null) return;
      resizeFrame = window.requestAnimationFrame(() => {
        resizeFrame = null;
        if (!disposed && terminalRef.current === terminal) requestViewportSizing();
      });
    });
    observer?.observe(host);
    // Moving between displays changes the pixel ratio; re-snap and refit.
    let dprQuery: MediaQueryList | null = null;
    const onDprChange = () => {
      if (disposed) return;
      terminal.options.fontSize = snapTerminalFontSize();
      fit.fit();
      requestViewportSizing();
      watchDpr();
    };
    const watchDpr = () => {
      dprQuery?.removeEventListener("change", onDprChange);
      dprQuery = typeof matchMedia === "function" ? matchMedia(`(resolution: ${globalThis.devicePixelRatio}dppx)`) : null;
      dprQuery?.addEventListener("change", onDprChange, { once: true });
    };
    watchDpr();
    return () => {
      disposed = true;
      dprQuery?.removeEventListener("change", onDprChange);
      observer?.disconnect();
      if (resizeFrame !== null) window.cancelAnimationFrame(resizeFrame);
      if (resizeInFlightRef.current) window.clearTimeout(resizeInFlightRef.current.timer);
      resizeInFlightRef.current = null;
    };
  };
  const handleTerminalResize = (terminal: Terminal, { cols, rows }: TerminalGrid) => {
    if (suppressFrameResizeRef.current) return;
    desiredViewportGridRef.current = { cols, rows };
    onCellGeometryRef.current?.(terminalCellGeometry(terminal, { cols, rows }));
    renderedGridRef.current = { cols, rows };
    const stream = streamRef.current;
    if (!stream || ownershipRef.current !== "owned") return;
    const bounds = terminalScreenBounds(terminal);
    const resizeCommand: TerminalResize = {
      type: "terminal.resize",
      cols,
      rows,
      cell_width_px: bounds ? Math.max(1, Math.round(bounds.width / Math.max(1, cols))) : 0,
      cell_height_px: bounds ? Math.max(1, Math.round(bounds.height / Math.max(1, rows))) : 0,
    };
    const previous = lastResizeRef.current;
    if (sameResize(previous, resizeCommand)) return;
    lastResizeRef.current = resizeCommand;
    stream.send(resizeCommand);
  };
  const attachmentGeometry = (terminal: Terminal) => {
    const viewportGrid = desiredViewportGridRef.current ?? {
      cols: Math.max(1, Math.min(65535, terminal.cols || 80)),
      rows: Math.max(1, Math.min(65535, terminal.rows || 24)),
    };
    const geometry = terminalCellGeometry(terminal, renderedGridRef.current ?? { cols: terminal.cols, rows: terminal.rows });
    return { ...viewportGrid, ...geometry };
  };
  const seedAttachment = (openRequest: TerminalOpenRequest) => {
    lastResizeRef.current = {
      type: "terminal.resize",
      cols: openRequest.cols,
      rows: openRequest.rows,
      cell_width_px: openRequest.cell_width_px,
      cell_height_px: openRequest.cell_height_px,
    };
  };
  const cancelInFlight = () => {
  if (resizeInFlightRef.current) window.clearTimeout(resizeInFlightRef.current.timer);
  resizeInFlightRef.current = null;
  resizeFollowUpRef.current = false;
  };
  const acceptFrameGrid = (frameGrid: TerminalGrid) => {
    authoritativeFrameRef.current = true;
    const inFlight = resizeInFlightRef.current;
    if (inFlight && inFlight.grid.cols === frameGrid.cols && inFlight.grid.rows === frameGrid.rows) settleResize();
  };
  const resizeToFrame = (terminal: Terminal, frameGrid: TerminalGrid) => {
    suppressFrameResizeRef.current = true;
    try {
      terminal.resize(frameGrid.cols, frameGrid.rows);
    } finally {
      suppressFrameResizeRef.current = false;
    }
    renderedGridRef.current = frameGrid;
  };
  const markRendered = (frameGrid: TerminalGrid) => { renderedGridRef.current = frameGrid; };
  return { requestViewportSizing, seedGrid, observeViewport, handleTerminalResize, attachmentGeometry, seedAttachment, cancelInFlight, acceptFrameGrid, resizeToFrame, markRendered };
}
export type TerminalResizeController = ReturnType<typeof useTerminalResize>;
