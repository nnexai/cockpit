import { useRef, type ClipboardEvent, type CompositionEvent, type KeyboardEvent, type WheelEvent } from "react";
import { context, isLocalBrowserChrome, location, modifiers, ownsBrowserControl, type WheelIntent } from "./browserPaneModel";
import type { BrowserPaneState } from "./useBrowserPaneState";
import type { BrowserFrameGeometry, BrowserViewCommandApi } from "./useBrowserViewCommand";
import type { BrowserInputQueue } from "./useBrowserInputQueue";
import type { BrowserAnnotationMutations } from "./useBrowserAnnotationMutations";

export interface BrowserPageHandlers {
  onWheel(event: WheelEvent<HTMLDivElement>): void;
  onKeyDown(event: KeyboardEvent<HTMLDivElement>): void;
  onKeyUp(event: KeyboardEvent<HTMLDivElement>): void;
  onPaste(event: ClipboardEvent<HTMLDivElement>): void;
  onCopy(event: ClipboardEvent<HTMLDivElement>): void;
  onCompositionStart(event: CompositionEvent<HTMLDivElement>): void;
  onCompositionUpdate(event: CompositionEvent<HTMLDivElement>): void;
  onCompositionEnd(event: CompositionEvent<HTMLDivElement>): void;
}

export function useBrowserPageInput(state: BrowserPaneState, geometry: BrowserFrameGeometry, view: BrowserViewCommandApi, input: BrowserInputQueue, marks: BrowserAnnotationMutations, { inputActive, onInteractionFocus }: { inputActive: boolean; onInteractionFocus: (() => void) | undefined }): BrowserPageHandlers {
  const { liveInputEnabledRef, tool, snapshotRef, frameRef, inputGenerationRef, draftRef, selectedId } = state;
  const { viewportPointFor, frameSupportsViewportInput } = geometry;
  const { command, ensureControl } = view;
  const { enqueueInput, nextInput } = input;
  const { removeAnnotation } = marks;
  const annotationDeleteKeyRef = useRef(false);
  const onWheel = (event: WheelEvent<HTMLDivElement>): void => {
    if (!liveInputEnabledRef.current || tool !== "browse" || isLocalBrowserChrome(event.target)) return;
    const current = snapshotRef.current;
    event.preventDefault(); onInteractionFocus?.();
    const line = 16;
    const page = current?.viewport;
    const scale = event.deltaMode === 1 ? line : event.deltaMode === 2 ? (page?.css_height ?? 800) : 1;
    const intent: WheelIntent = { clientX: event.clientX, clientY: event.clientY, deltaX: event.deltaX * scale, deltaY: event.deltaY * scale, modifiers: modifiers(event) };
    const generation = inputGenerationRef.current;
    void enqueueInput("wheel", async () => {
      const controlled = await ensureControl();
      if (!controlled || generation !== inputGenerationRef.current || !liveInputEnabledRef.current) return;
      const latest = snapshotRef.current;
      const accepted = frameRef.current;
      const point = viewportPointFor(intent, false, true);
      const paintedLocation = latest && accepted && ownsBrowserControl(latest) && point
        ? location(latest, accepted.descriptor)
        : null;
      // A scroll-only viewport event advances the input revision before its
      // pixels arrive; use the painted transform but the current wire cursor.
      const inputLocation = paintedLocation && latest?.viewport
        ? { ...paintedLocation, viewport_revision: latest.viewport.viewport_revision }
        : null;
      if (!point || !inputLocation || generation !== inputGenerationRef.current) {
        return;
      }
      const input_sequence = nextInput();
      await command({ type: "wheel", location: inputLocation, input: { x: point.x, y: point.y, delta_x_css: intent.deltaX, delta_y_css: intent.deltaY, modifiers: intent.modifiers, input_sequence } });
    }, intent);
  };
  const sendKey = (event: KeyboardEvent<HTMLElement>, kind: "down" | "up"): void => {
    if (!inputActive || !liveInputEnabledRef.current || event.defaultPrevented || tool !== "browse" || event.target !== event.currentTarget) return;
    if (!event.nativeEvent.isComposing && event.key !== "Dead") event.preventDefault();
    const key = { key: event.key, code: event.code, location: event.location, modifiers: modifiers(event), repeat: event.repeat };
    onInteractionFocus?.();
    void enqueueInput("boundary", async () => {
      const controlled = await ensureControl(); const current = controlled ? snapshotRef.current : null; const documentContext = current ? context(current) : null;
      if (!documentContext || !frameSupportsViewportInput()) return;
      const input_sequence = nextInput();
      await command({ type: "keyboard", context: documentContext, input: { kind, ...key, input_sequence } });
    });
  };
  const onSurfaceKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (annotationDeleteKeyRef.current && event.target === event.currentTarget && event.key === "Delete") {
      if (event.repeat) { event.preventDefault(); return; }
      // A release may have landed on other chrome. A fresh press owns a new
      // cycle; do not suppress its down while forwarding its up to the page.
      annotationDeleteKeyRef.current = false;
    }
    if (event.target === event.currentTarget && event.key === "Delete" && !event.nativeEvent.isComposing
      && !event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey) {
      const selected = draftRef.current?.annotations.find((annotation) => annotation.id === selectedId);
      if (selected) {
        event.preventDefault();
        annotationDeleteKeyRef.current = true;
        removeAnnotation(selected.id);
        return;
      }
    }
    sendKey(event, "down");
  };
  const onSurfaceKeyUp = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (annotationDeleteKeyRef.current && event.key === "Delete") {
      annotationDeleteKeyRef.current = false;
      event.preventDefault();
      return;
    }
    sendKey(event, "up");
  };
  const sendComposition = (event: CompositionEvent<HTMLElement>, kind: "start" | "update" | "commit"): void => {
    if (!liveInputEnabledRef.current || tool !== "browse" || isLocalBrowserChrome(event.target)) return;
    void enqueueInput("boundary", async () => {
      const controlled = await ensureControl(); const current = controlled ? snapshotRef.current : null; const documentContext = current ? context(current) : null;
      if (!documentContext || !frameSupportsViewportInput()) return;
      const input_sequence = nextInput();
      await command({ type: "composition", context: documentContext, input: { kind, text: event.data, input_sequence } });
    });
  };
  const clipboard = (event: ClipboardEvent<HTMLElement>, copy: boolean): void => {
    if (!liveInputEnabledRef.current || event.defaultPrevented || tool !== "browse" || isLocalBrowserChrome(event.target)) return;
    event.preventDefault(); onInteractionFocus?.();
    const commandValue = copy ? { type: "copy" as const } : { type: "paste" as const, text: event.clipboardData.getData("text/plain") };
    void enqueueInput("boundary", async () => {
      const controlled = await ensureControl(); const current = controlled ? snapshotRef.current : null; const documentContext = current ? context(current) : null;
      if (documentContext && frameSupportsViewportInput()) await command({ type: "clipboard", context: documentContext, command: commandValue });
    });
  };
  return {
    onWheel, onKeyDown: onSurfaceKeyDown, onKeyUp: onSurfaceKeyUp,
    onPaste: (event) => clipboard(event, false), onCopy: (event) => clipboard(event, true),
    onCompositionStart: (event) => sendComposition(event, "start"),
    onCompositionUpdate: (event) => sendComposition(event, "update"),
    onCompositionEnd: (event) => sendComposition(event, "commit"),
  };
}
