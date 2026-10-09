import { useRef } from "react";
import type { TerminalStream } from "../../client/CockpitClient";
import type { TerminalCommand } from "../../protocol/generated/v1";
import type { TerminalRefs, PaneError, PaneIntent, TerminalPaneRequest } from "./paneState";
import { appendPendingControlCommand } from "./terminalInput";

export function useTerminalControl(refs: TerminalRefs, request: TerminalPaneRequest, setError: (error: PaneError) => void) {
  const { terminalRef, streamRef, ownershipRef, controlAllowedRef, selectedRef, presentedRef, onRequestControlRef, onSelectRef, controlPendingRef, focusEpochRef, focusTokenRef } = refs;
  const pendingCommands = useRef<TerminalCommand[]>([]);
  const pendingIntentRef = useRef<{ epoch: number; paneId: string; token: number } | null>(null);
  const controlRequestPendingRef = useRef(false);
  const currentIntent = () => ({ epoch: focusEpochRef.current, paneId: request.pane_id, token: focusTokenRef.current });
  const sendStreamCommand = (stream: TerminalStream, command: TerminalCommand): boolean => {
    try {
      stream.send(command);
      return true;
    } catch (cause) {
      if (streamRef.current !== stream) return false;
      const typed = cause instanceof Error ? cause as Error & { code?: string; operationCode?: string } : null;
      setError({ code: typed?.operationCode ?? typed?.code ?? "terminal_input_failed", message: typed?.message ?? "Terminal input failed" });
      return false;
    }
  };
  const clearPendingCommands = () => {
    pendingCommands.current = [];
    pendingIntentRef.current = null;
  };
  const sendInput = (command: TerminalCommand) => {
    if (!selectedRef.current || ownershipRef.current === "lost" || ownershipRef.current === "conflict") return;
    // Popup input is never buffered while attaching or reconnecting.
    if (request.target_kind === "popup" && (!controlAllowedRef.current || ownershipRef.current !== "owned" || !streamRef.current)) return;
    const hasSelectedFocusIntent = selectedRef.current && focusTokenRef.current > 0;
    if (!controlAllowedRef.current && !controlRequestPendingRef.current && !controlPendingRef.current && !hasSelectedFocusIntent) return;
    if (controlAllowedRef.current && ownershipRef.current === "owned" && streamRef.current) sendStreamCommand(streamRef.current, command);
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
  const flushPending = () => {
    const stream = streamRef.current;
    const intent = pendingIntentRef.current;
    const current = currentIntent();
    if (!selectedRef.current || !controlAllowedRef.current || ownershipRef.current !== "owned" || !stream || pendingCommands.current.length === 0
      || !intent || intent.epoch !== current.epoch || intent.paneId !== current.paneId || intent.token !== current.token) return;
    for (const command of pendingCommands.current) sendStreamCommand(stream, command);
    clearPendingCommands();
  };
  const requestControl = () => {
    if (selectedRef.current && presentedRef.current && controlAllowedRef.current && ownershipRef.current === "owned") terminalRef.current?.focus();
    onRequestControlRef.current?.();
    if (!selectedRef.current) onSelectRef.current?.();
    if (controlAllowedRef.current && ownershipRef.current === "owned") return;
    if (ownershipRef.current === "lost" || ownershipRef.current === "conflict") return;
    controlRequestPendingRef.current = true;
  };
  const dropPendingMouseCommands = () => {
    pendingCommands.current = pendingCommands.current.filter((command) => command.type !== "terminal.mouse");
  };
  const dropStaleIntent = (current: PaneIntent) => {
    const intent = pendingIntentRef.current;
    if (!intent) return;
    if (intent.epoch !== current.epoch || intent.paneId !== current.paneId || intent.token !== current.token) clearPendingCommands();
  };
  const cancelControlRequest = () => { controlRequestPendingRef.current = false; };
  return { currentIntent, sendInput, flushPending, clearPendingCommands, requestControl, dropPendingMouseCommands, dropStaleIntent, cancelControlRequest };
}
export type TerminalControl = ReturnType<typeof useTerminalControl>;
