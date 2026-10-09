import { useRef, type DOMAttributes, type PointerEvent as ReactPointerEvent } from "react";
import type { Terminal } from "@xterm/xterm";
import type { TerminalMouseButton, TerminalMouseKind } from "../../protocol/generated/v1";
import type { TerminalRefs } from "./paneState";
import type { TerminalControl } from "./useTerminalControl";
import { terminalScreenBounds } from "./cockpitTerminal";
import { applicationFontSize } from "./terminalTheme";
import { forwardTerminalMouse, terminalMouseButton, terminalScrollCommand } from "./terminalMouse";

export function useTerminalPointer(refs: TerminalRefs, control: TerminalControl, terminalMouseInput: boolean) {
  const { hostRef, terminalRef } = refs;
  const { sendInput, requestControl } = control;
  const activeMousePointer = useRef<{ button: TerminalMouseButton; pointerId: number } | null>(null);
  const mouseModeRef = useRef(false);
  const lastMouseMotionAt = useRef(0);
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
    control.dropPendingMouseCommands();
    releaseCapturedPointer();
  };
  const sendPointerMouse = (kind: TerminalMouseKind, button: TerminalMouseButton | null, event: ReactPointerEvent<HTMLDivElement>) => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const bounds = terminalScreenBounds(terminal)
      ?? hostRef.current?.getBoundingClientRect();
    if (!bounds) return;
    forwardTerminalMouse(terminalMouseInput && mouseModeRef.current, sendInput, kind, button, event, bounds, terminal.cols, terminal.rows);
  };
  const applyMouseMode = (enabled: boolean) => {
    if (enabled) mouseModeRef.current = true;
    else clearMouseMode();
  };
  const handleWheel = (terminal: Terminal, event: WheelEvent): boolean => {
      if (event.deltaY === 0) return true;
      event.preventDefault();
      requestControl();
    const bounds = terminalScreenBounds(terminal) ?? hostRef.current?.getBoundingClientRect();
    sendInput(terminalScrollCommand(event, bounds, terminal.cols, terminal.rows, () => typeof terminal.options.fontSize === "number" ? terminal.options.fontSize : applicationFontSize()));
    return false;
  };
  const handlers: Pick<DOMAttributes<HTMLDivElement>, "onPointerDownCapture" | "onPointerMoveCapture" | "onPointerUpCapture" | "onPointerCancel"> = {
    onPointerDownCapture: (event) => {
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
    },
    onPointerMoveCapture: (event) => {
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
    },
    onPointerUpCapture: (event) => {
      const active = activeMousePointer.current;
      if (!active || active.pointerId !== event.pointerId) return;
      if (terminalMouseInput && mouseModeRef.current) {
        event.preventDefault();
        event.stopPropagation();
        sendPointerMouse("up", active.button, event);
      }
      releaseCapturedPointer();
    },
    onPointerCancel: (event) => {
      const active = activeMousePointer.current;
      if (!active || active.pointerId !== event.pointerId) return;
      if (terminalMouseInput && mouseModeRef.current) {
        event.preventDefault();
        event.stopPropagation();
        sendPointerMouse("up", active.button, event);
      }
      releaseCapturedPointer();
    },
  };
  return { releaseCapturedPointer, clearMouseMode, applyMouseMode, handleWheel, handlers };
}
export type TerminalPointer = ReturnType<typeof useTerminalPointer>;
