import { useEffect, useRef, type DOMAttributes, type PointerEvent as ReactPointerEvent } from "react";
import type { Terminal } from "@xterm/xterm";
import type { TerminalMouseButton, TerminalMouseKind } from "../../protocol/generated/v1";
import type { TerminalRefs } from "./paneState";
import type { TerminalControl } from "./useTerminalControl";
import { terminalScreenBounds } from "./cockpitTerminal";
import { applicationFontSize } from "./terminalTheme";
import { forwardTerminalMouse, terminalMouseButton, terminalScrollCommand, terminalTouchScrollCommand } from "./terminalMouse";

export function useTerminalPointer(refs: TerminalRefs, control: TerminalControl, terminalMouseInput: boolean, scratch = false) {
  const { hostRef, terminalRef } = refs;
  const { sendInput, requestControl } = control;
  const activeMousePointer = useRef<{ button: TerminalMouseButton; pointerId: number } | null>(null);
  const mouseModeRef = useRef(false);
  const lastMouseMotionAt = useRef(0);
  const touchPointers = useRef(new Set<number>());
  const touchGesture = useRef<{ id: number; x: number; y: number; originY: number; time: number; lines: number; direction: number; dragging: boolean; dead: boolean; selectedAtStart: boolean } | null>(null);
  useEffect(() => {
    const host = hostRef.current;
    if (!host || scratch) return;
    const touch = (event: TouchEvent) => {
      if (!terminalRef.current?.element?.contains(event.target as Node)) return;
      event.stopPropagation();
      if (event.type === "touchend" && event.cancelable) event.preventDefault();
    };
    const contextMenu = () => { if (touchGesture.current) touchGesture.current.dead = true; };
    const lostCapture = (event: PointerEvent) => {
      if (touchGesture.current?.id === event.pointerId) touchGesture.current.dead = true;
    };
    const compatibilityMouse = (event: MouseEvent) => {
      const capabilities = (event as MouseEvent & { sourceCapabilities?: { firesTouchEvents: boolean } | null }).sourceCapabilities;
      if (capabilities?.firesTouchEvents && terminalRef.current?.element?.contains(event.target as Node)) {
        event.preventDefault();
        event.stopPropagation();
      }
    };
    const options = { capture: true, passive: false };
    for (const type of ["touchstart", "touchmove", "touchend", "touchcancel"] as const) host.addEventListener(type, touch, options);
    host.addEventListener("contextmenu", contextMenu, true);
    host.addEventListener("lostpointercapture", lostCapture, true);
    host.addEventListener("mousedown", compatibilityMouse, true);
    return () => {
      for (const type of ["touchstart", "touchmove", "touchend", "touchcancel"] as const) host.removeEventListener(type, touch, options);
      host.removeEventListener("contextmenu", contextMenu, true);
      host.removeEventListener("lostpointercapture", lostCapture, true);
      host.removeEventListener("mousedown", compatibilityMouse, true);
      touchPointers.current.clear();
      touchGesture.current = null;
    };
  }, [hostRef, terminalRef, scratch]);
  useEffect(() => {
    if (!refs.selectedRef.current && touchGesture.current?.selectedAtStart) touchGesture.current.dead = true;
  }, [refs.selectedRef.current]);
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
    if (scratch) return true;
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
      if (scratch) return;
      if (event.pointerType === "touch") {
        if (!touchGesture.current && !terminalRef.current?.element?.contains(event.target as Node)) return;
        touchPointers.current.add(event.pointerId);
        if (touchGesture.current || touchPointers.current.size > 1) {
          if (touchGesture.current) touchGesture.current.dead = true;
        } else {
          touchGesture.current = { id: event.pointerId, x: event.clientX, y: event.clientY, originY: event.clientY, time: event.timeStamp, lines: 0, direction: 0, dragging: false, dead: false, selectedAtStart: refs.selectedRef.current };
        }
        return;
      }
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
      if (event.pointerType === "touch" && !scratch) {
        const gesture = touchGesture.current;
        if (!gesture || gesture.dead || gesture.id !== event.pointerId) return;
        if (!gesture.dragging) {
          if (Math.hypot(event.clientX - gesture.x, event.clientY - gesture.y) <= 10) return;
          gesture.dragging = true;
          requestControl({ focus: false });
        }
        const terminal = terminalRef.current;
        const bounds = terminal && (terminalScreenBounds(terminal) ?? hostRef.current?.getBoundingClientRect());
        if (!terminal || !bounds || bounds.height <= 0 || terminal.rows <= 0) return;
        const delta = event.clientY - gesture.originY;
        const direction = Math.sign(delta);
        if (direction && gesture.direction && direction !== gesture.direction) {
          gesture.originY = event.clientY;
          gesture.lines = 0;
          gesture.direction = 0;
          return;
        }
        if (direction) gesture.direction = direction;
        const total = Math.floor(Math.abs(delta) / (bounds.height / terminal.rows));
        const lines = Math.min(65535, total - gesture.lines);
        if (lines > 0) {
          sendInput(terminalTouchScrollCommand(direction > 0 ? "up" : "down", lines, event.clientX, event.clientY, bounds, terminal.cols, terminal.rows));
          gesture.lines += lines;
        }
        return;
      }
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
      if (event.pointerType === "touch" && !scratch) {
        const gesture = touchGesture.current;
        touchPointers.current.delete(event.pointerId);
        if (gesture && !gesture.dead && gesture.id === event.pointerId && !gesture.dragging && Math.hypot(event.clientX - gesture.x, event.clientY - gesture.y) <= 10 && event.timeStamp - gesture.time < 500) {
          requestControl();
          if (terminalMouseInput && mouseModeRef.current) {
            sendPointerMouse("down", "left", event);
            sendPointerMouse("up", "left", event);
          }
        }
        if (touchPointers.current.size === 0) touchGesture.current = null;
        return;
      }
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
      if (event.pointerType === "touch" && !scratch) {
        touchPointers.current.delete(event.pointerId);
        if (touchGesture.current) touchGesture.current.dead = true;
        if (touchPointers.current.size === 0) touchGesture.current = null;
        return;
      }
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
