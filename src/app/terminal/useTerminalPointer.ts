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
  const touchGesture = useRef<{ id: number; x: number; y: number; lastY: number; time: number; lastTime: number; remainder: number; direction: number; velocity: number; dragging: boolean; dead: boolean; suppressTap: boolean; selectedAtStart: boolean } | null>(null);
  const inertiaFrame = useRef<number | null>(null);
  const cancelInertia = () => {
    if (inertiaFrame.current !== null) cancelAnimationFrame(inertiaFrame.current);
    inertiaFrame.current = null;
  };
  const scrollTouch = (gesture: NonNullable<typeof touchGesture.current>, delta: number, x: number, y: number) => {
    const terminal = terminalRef.current;
    const bounds = terminal && (terminalScreenBounds(terminal) ?? hostRef.current?.getBoundingClientRect());
    if (!terminal || !bounds || bounds.height <= 0 || terminal.rows <= 0 || !delta) return;
    const direction = Math.sign(delta);
    if (gesture.direction && direction !== gesture.direction) gesture.remainder = 0;
    gesture.direction = direction;
    gesture.remainder += delta;
    const cellHeight = bounds.height / terminal.rows;
    const lines = Math.min(65535, Math.floor(Math.abs(gesture.remainder) / cellHeight));
    if (lines > 0) {
      sendInput(terminalTouchScrollCommand(direction > 0 ? "up" : "down", lines, x, y, bounds, terminal.cols, terminal.rows));
      gesture.remainder -= direction * lines * cellHeight;
    }
  };
  const startInertia = (gesture: NonNullable<typeof touchGesture.current>, x: number, y: number) => {
    let velocity = Math.max(-2.5, Math.min(2.5, gesture.velocity));
    if (Math.abs(velocity) < 0.15 || window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) return;
    let last = performance.now();
    const started = last;
    const step = (now: number) => {
      inertiaFrame.current = null;
      if ((gesture.selectedAtStart && !refs.selectedRef.current) || now - started > 700) return;
      const elapsed = Math.max(0, Math.min(48, now - last));
      last = now;
      const decay = Math.exp(-elapsed / 220);
      scrollTouch(gesture, velocity * 220 * (1 - decay), x, y);
      velocity *= decay;
      if (Math.abs(velocity) >= 0.05) inertiaFrame.current = requestAnimationFrame(step);
    };
    inertiaFrame.current = requestAnimationFrame(step);
  };
  useEffect(() => {
    const host = hostRef.current;
    if (!host || scratch) return;
    const touch = (event: TouchEvent) => {
      if (!terminalRef.current?.element?.contains(event.target as Node)) return;
      if (event.type === "touchcancel" || event.touches?.length > 1) {
        cancelInertia();
        if (touchGesture.current) touchGesture.current.dead = true;
      }
      event.stopPropagation();
      if (event.type === "touchend" && event.cancelable) event.preventDefault();
    };
    const contextMenu = () => { cancelInertia(); if (touchGesture.current) touchGesture.current.dead = true; };
    const selectionChange = () => { cancelInertia(); if (touchGesture.current) touchGesture.current.dead = true; };
    const lostCapture = (event: PointerEvent) => {
      if (touchGesture.current?.id === event.pointerId) { cancelInertia(); touchGesture.current.dead = true; }
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
    document.addEventListener("selectionchange", selectionChange);
    host.addEventListener("keydown", cancelInertia, true);
    document.addEventListener("visibilitychange", cancelInertia);
    window.addEventListener("resize", cancelInertia);
    host.addEventListener("mousedown", compatibilityMouse, true);
    return () => {
      for (const type of ["touchstart", "touchmove", "touchend", "touchcancel"] as const) host.removeEventListener(type, touch, options);
      host.removeEventListener("contextmenu", contextMenu, true);
      host.removeEventListener("lostpointercapture", lostCapture, true);
      document.removeEventListener("selectionchange", selectionChange);
      host.removeEventListener("keydown", cancelInertia, true);
      document.removeEventListener("visibilitychange", cancelInertia);
      window.removeEventListener("resize", cancelInertia);
      cancelInertia();
      host.removeEventListener("mousedown", compatibilityMouse, true);
      touchPointers.current.clear();
      touchGesture.current = null;
    };
  }, [hostRef, terminalRef, scratch]);
  useEffect(() => {
    if (!refs.selectedRef.current) {
      cancelInertia();
      if (touchGesture.current?.selectedAtStart) touchGesture.current.dead = true;
    }
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
      cancelInertia();
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
      const interruptedInertia = inertiaFrame.current !== null;
      cancelInertia();
      if (scratch) return;
      if (event.pointerType === "touch") {
        if (!touchGesture.current && !terminalRef.current?.element?.contains(event.target as Node)) return;
        touchPointers.current.add(event.pointerId);
        if (touchGesture.current || touchPointers.current.size > 1) {
          if (touchGesture.current) touchGesture.current.dead = true;
        } else {
          touchGesture.current = { id: event.pointerId, x: event.clientX, y: event.clientY, lastY: event.clientY, time: event.timeStamp, lastTime: event.timeStamp, remainder: 0, direction: 0, velocity: 0, dragging: false, dead: false, suppressTap: interruptedInertia, selectedAtStart: refs.selectedRef.current };
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
          if (Math.abs(event.clientX - gesture.x) > Math.abs(event.clientY - gesture.y)) {
            gesture.dead = true;
            return;
          }
          gesture.dragging = true;
          requestControl({ focus: false });
        }
        const delta = event.clientY - gesture.lastY;
        const elapsed = event.timeStamp - gesture.lastTime;
        if (elapsed > 0) {
          const sample = delta / elapsed;
          gesture.velocity = Math.sign(sample) !== Math.sign(gesture.velocity) ? sample : gesture.velocity * 0.35 + sample * 0.65;
        }
        gesture.lastY = event.clientY;
        gesture.lastTime = event.timeStamp;
        scrollTouch(gesture, delta, event.clientX, event.clientY);
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
        if (gesture && !gesture.dead && !gesture.suppressTap && gesture.id === event.pointerId && !gesture.dragging && Math.hypot(event.clientX - gesture.x, event.clientY - gesture.y) <= 10 && event.timeStamp - gesture.time < 500) {
          requestControl();
          if (terminalMouseInput && mouseModeRef.current) {
            sendPointerMouse("down", "left", event);
            sendPointerMouse("up", "left", event);
          }
        }
        else if (gesture && !gesture.dead && gesture.id === event.pointerId && gesture.dragging && event.timeStamp - gesture.lastTime < 80) {
          startInertia(gesture, event.clientX, event.clientY);
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
        cancelInertia();
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
