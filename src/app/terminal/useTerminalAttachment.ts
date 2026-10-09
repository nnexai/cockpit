import { useEffect, useRef, useState } from "react";
import type { CockpitClient, TerminalStream } from "../../client/CockpitClient";
import type { TerminalOpenRequest, TerminalOwnershipState, TerminalStreamMessage } from "../../protocol/generated/v1";
import type { TerminalRefs, TerminalPaneRequest, PaneError } from "./paneState";
import type { TerminalControl } from "./useTerminalControl";
import type { TerminalPointer } from "./useTerminalPointer";
import type { TerminalClipboard } from "./terminalClipboard";
import type { TerminalResizeController } from "./useTerminalResize";
import type { TerminalFrameQueues } from "./useTerminalFrameQueue";

type AttachmentOptions = {
  client: CockpitClient; request: TerminalPaneRequest; deferAttachment: boolean; terminalReady: boolean;
  refs: TerminalRefs; control: TerminalControl; pointer: TerminalPointer; clipboard: TerminalClipboard;
  sizing: TerminalResizeController; frames: TerminalFrameQueues;
  setOwnership(ownership: TerminalOwnershipState): void; setError(error: PaneError | null): void;
  setClosed(closed: boolean): void; setFramePainted(painted: boolean): void;
  onResync?: () => void; onClosed?: () => void;
};
export function useTerminalAttachment({ client, request, deferAttachment, terminalReady, refs, control, pointer, clipboard, sizing, frames, setOwnership, setError, setClosed, setFramePainted, onResync, onClosed }: AttachmentOptions) {
  const { terminalRef, streamRef, ownershipRef, onReadyRef, registerStreamRef } = refs;
  const { clearPendingCommands, flushPending } = control;
  const { clearMouseMode } = pointer;
  const attachmentGeneration = useRef(0);
  const [attempt, setAttempt] = useState(0);
  const attachRetryKeyRef = useRef<string | null>(null);
  const attachRetryCountRef = useRef(0);
  const takeoverRequestedRef = useRef(false);
  const attachRetryTimerRef = useRef<number | null>(null);
  const retryAttachment = (takeover = false) => {
    clearPendingCommands();
    clipboard.dropPendingPaste();
    takeoverRequestedRef.current = takeover;
    attachRetryCountRef.current = 0;
    setAttempt((value) => value + 1);
  };
  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal || !terminalReady || deferAttachment) return;
    setFramePainted(false);
    const { cols, rows, cell_width_px, cell_height_px } = sizing.attachmentGeometry(terminal);
    const openRequest: TerminalOpenRequest = {
      ...request,
      mode: "control",
      takeover: takeoverRequestedRef.current,
      cols,
      rows,
      cell_width_px,
      cell_height_px,
    };
    takeoverRequestedRef.current = false;
    const registerAttachment = registerStreamRef.current;
    sizing.seedAttachment(openRequest);
    let cancelled = false;
    const retryKey = `${request.session_id}:${request.pane_id}`;
    if (attachRetryKeyRef.current !== retryKey) {
      attachRetryKeyRef.current = retryKey;
      attachRetryCountRef.current = 0;
    }
    let retryScheduled = false;
    const schedulePaneVisibilityRetry = (cause: unknown): boolean => {
      const typed = cause instanceof Error
        ? cause as Error & { code?: string; operationCode?: string }
        : cause !== null && typeof cause === "object"
          ? cause as { code?: string; operationCode?: string; message?: string }
          : null;
      const code = typed?.operationCode ?? typed?.code;
      const message = typed?.message ?? "";
      const visibilityRace = code === "pane_not_in_layout"
        || /not in its tab's layout|terminal websocket closed/i.test(message);
      if (!visibilityRace || retryScheduled || attachRetryCountRef.current >= 4) return false;
      const delay = 40 * 2 ** attachRetryCountRef.current;
      attachRetryCountRef.current += 1;
      attachRetryTimerRef.current = window.setTimeout(() => {
        attachRetryTimerRef.current = null;
        setAttempt((value) => value + 1);
      }, delay);
      return true;
    };
    let stream: TerminalStream | null = null;
    let observedStreamId: string | null = null;

    const controller = new AbortController();
    const generation = ++attachmentGeneration.current;
    // The open request carries the current viewport grid.
    sizing.cancelInFlight();
    setError(null);
    setClosed(false);
    ownershipRef.current = "pending";
    setOwnership("pending");
    const fail = (code: string, message: string) => {
      clearPendingCommands();
      clearMouseMode();
      cancelled = true;
      frameQueue.clear();
      controller.abort();
      frameQueue.disposeReadiness();
      onReadyRef.current?.();
      setError({ code, message });
      ownershipRef.current = "released";
      setOwnership("released");
      streamRef.current = null;
      if (stream) registerAttachment?.(stream, false);
      stream?.close();
      stream = null;
    };
    const frameQueue = frames.start({ terminal, cancelled: () => cancelled, isLatest: () => generation === attachmentGeneration.current, fail, onBacklog: onResync });
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
        pointer.applyMouseMode(message.enabled);
        return;
      }
      if (message.type === "ownership") {
        if (message.state === "lost" || message.state === "conflict") {
          clearPendingCommands();
          clearMouseMode();
          control.cancelControlRequest();
          clipboard.dropPendingPaste();
          cancelled = true;
          frameQueue.clear();
          controller.abort();
          frameQueue.disposeReadiness();
          if (attachRetryTimerRef.current !== null) {
            window.clearTimeout(attachRetryTimerRef.current);
            attachRetryTimerRef.current = null;
          }
          sizing.cancelInFlight();
          streamRef.current = null;
          if (stream) registerAttachment?.(stream, false);
          stream?.close();
          stream = null;
          onReadyRef.current?.();
        } else if (message.state === "released") clearMouseMode();
        ownershipRef.current = message.state;
        setOwnership(message.state);
        if (message.state === "owned") sizing.requestViewportSizing();
        if (message.state === "owned") flushPending();
        return;
      }
      if (message.type === "frame") {
        frameQueue.receive(message);
        return;
      }
      if (message.type === "error" || message.type === "disconnected") {
        if (schedulePaneVisibilityRetry(message)) return;
        fail(message.code, message.message);
      } else if (message.type === "closed") {
        cancelled = true;
        frameQueue.clear();
        clearMouseMode();
        frameQueue.disposeReadiness();
        setClosed(true);
        setError(null);
        onReadyRef.current?.();
        setOwnership("released");
        streamRef.current = null;
        onClosed?.();
        if (stream) registerAttachment?.(stream, false);
        stream?.close();
        stream = null;
      }
    };
    const failFromCause = (cause: unknown) => {
      if (cancelled || schedulePaneVisibilityRetry(cause)) return;
      const typed = cause instanceof Error ? cause : new Error("Could not attach terminal");
      const error = typed as Error & { code?: string; operationCode?: string };
      fail(error.operationCode ?? error.code ?? "terminal_attach_failed", typed.message);
    };
    void client.openTerminal(openRequest, onMessage, failFromCause, controller.signal).then((opened) => {
      if (cancelled || generation !== attachmentGeneration.current) {
        opened.close();
        return;
      }
      stream = opened;
      streamRef.current = opened;
      if (!cancelled && generation === attachmentGeneration.current && terminalRef.current === terminal
        && ownershipRef.current === "owned") sizing.requestViewportSizing();
      flushPending();
      registerAttachment?.(opened, true);
    }, failFromCause);
    return () => {
      cancelled = true;
      frameQueue.clear();
      clearMouseMode();
      controller.abort();
      frameQueue.disposeReadiness();
      if (streamRef.current === stream) streamRef.current = null;
      if (attachRetryTimerRef.current !== null) {
        window.clearTimeout(attachRetryTimerRef.current);
        attachRetryTimerRef.current = null;
      }
      if (stream) registerAttachment?.(stream, false);
      stream?.close();
    };
  }, [client, deferAttachment, request.session_id, request.pane_id, request.target_kind, terminalReady, attempt]);
  return { retryAttachment };
}
