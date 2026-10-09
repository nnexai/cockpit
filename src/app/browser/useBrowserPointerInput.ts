import { useEffect, useRef, type PointerEvent } from "react";
import type { BrowserPoint, BrowserViewInspectResult } from "../../protocol/generated/v1";
import { MAX_ANNOTATION_POINTS, annotationId, button, isLocalBrowserChrome, location, modifiers, rectFrom, simplify, type ElementInspectionIntent, type PointerIntent, type Tool } from "./browserPaneModel";
import type { BrowserPaneState } from "./useBrowserPaneState";
import type { BrowserFrameGeometry, BrowserViewCommandApi } from "./useBrowserViewCommand";
import type { BrowserInputQueue } from "./useBrowserInputQueue";
import type { BrowserAnnotationMutations } from "./useBrowserAnnotationMutations";

export interface BrowserElementInspection { inspect(point: BrowserPoint): Promise<BrowserViewInspectResult | null> }

export function useBrowserElementInspection(state: BrowserPaneState, geometry: BrowserFrameGeometry, view: BrowserViewCommandApi): BrowserElementInspection {
  const { snapshotRef, frameRef, inspectRequestRef, elementIntentRef, inspectionNoticeRef, pendingElementClientPointRef, frame, snapshot, setMessage, setInspection, setAnnotationNotice, setStatus } = state;
  const { frameMatchesCurrent, viewportPointFor } = geometry;
  const { command } = view;
  const inspect = async (point: BrowserPoint): Promise<BrowserViewInspectResult | null> => {
    const current = snapshotRef.current;
    const accepted = frameRef.current;
    const inspected = current && accepted && frameMatchesCurrent(accepted) ? location(current, accepted.descriptor) : null;
    const request = ++inspectRequestRef.current;
    if (!current?.document || !current.viewport || !accepted || !inspected) {
      setMessage("Element inspection is waiting for a confirmed browser frame.");
      return null;
    }
    // The local pointer is measured against the painted frame. A remote
    // cursor sample may advance independently while inspection is in flight.
    const pointerSampleSequence = null;
    const intent: ElementInspectionIntent = {
      point: { x: point.x, y: point.y },
      location: inspected,
      frame: { ...accepted.descriptor },
      targetId: inspected.target_id,
      documentGeneration: inspected.document_generation,
      viewportRevision: inspected.viewport_revision,
      frameId: current.document.frame_id,
      frameGeneration: current.document.frame_generation,
      pointerSampleSequence,
    };
    elementIntentRef.current = intent;
    const outcome = await command({ type: "inspect", command: { location: intent.location, pointer_sample_sequence: intent.pointerSampleSequence, x: intent.point.x, y: intent.point.y } },
      () => request === inspectRequestRef.current && elementIntentRef.current === intent);
    if (request !== inspectRequestRef.current || elementIntentRef.current !== intent) return null;
    elementIntentRef.current = null;
    const result = outcome?.type === "inspection" ? outcome.inspection : null;
    if (!result) {
      setInspection(null);
      return null;
    }
    const latest = snapshotRef.current;
    const stillCurrent = Boolean(latest?.document && latest.displayed_target_id === intent.targetId
      && latest.document.document_generation === intent.documentGeneration
      && latest.document.frame_id === intent.frameId
      && latest.document.frame_generation === intent.frameGeneration
      && latest.viewport?.viewport_revision === intent.viewportRevision
      && result.location.target_id === intent.targetId
      && result.location.document_generation === intent.documentGeneration
      && result.location.viewport_revision === intent.viewportRevision
      && result.location.presented_frame_sequence === intent.location.presented_frame_sequence
      && result.frame_id === intent.frameId
      && result.frame_generation === intent.frameGeneration
      && result.pointer_sample_sequence === intent.pointerSampleSequence);
    if (!stillCurrent) {
      setInspection(null);
      setMessage("Element inspection became stale; select the element again.");
      return null;
    }
    if (inspectionNoticeRef.current && result.inspectable && result.freshness === "fresh") {
      const notice = inspectionNoticeRef.current;
      inspectionNoticeRef.current = null;
      setMessage((current) => current === notice ? null : current);
      setAnnotationNotice((current) => current === notice ? null : current);
      setStatus((current) => current === "unsupported" ? "ready" : current);
    }
    setInspection(result);
    return result;
  };
  useEffect(() => {
    const pending = pendingElementClientPointRef.current;
    if (!pending || !frameRef.current) return;
    const current = snapshotRef.current;
    if (!current?.document || !current.viewport || current.displayed_target_id !== pending.targetId
      || current.document.document_generation !== pending.documentGeneration
      || current.viewport.viewport_revision !== pending.viewportRevision) {
      pendingElementClientPointRef.current = null;
      setInspection(null);
      setMessage("Element inspection became stale before the browser frame was ready.");
      return;
    }
    if (!frameMatchesCurrent()) return;
    const point = viewportPointFor(pending);
    pendingElementClientPointRef.current = null;
    if (point) void inspect(point);
  }, [frame, frameMatchesCurrent, snapshot, viewportPointFor]);
  return { inspect };
}

export interface BrowserPointerHandlers {
  onPointerDown(event: PointerEvent<HTMLDivElement>): void;
  onPointerMove(event: PointerEvent<HTMLDivElement>): void;
  onPointerUp(event: PointerEvent<HTMLDivElement>): void;
  onPointerCancel(event: PointerEvent<HTMLDivElement>): void;
}

export function useBrowserPointerInput(state: BrowserPaneState, geometry: BrowserFrameGeometry, view: BrowserViewCommandApi, input: BrowserInputQueue, marks: BrowserAnnotationMutations, inspection: BrowserElementInspection, onInteractionFocus: (() => void) | undefined): BrowserPointerHandlers {
  const { liveInputEnabledRef, remotePointerRef, remotePointerIntentRef, remotePointRef, snapshotRef, frameRef, tool, color, gestureRef, surfaceRef, selectingElementRef, hoverInspectTimerRef, hoverInspectPointRef, inspectRequestRef, elementIntentRef, pendingElementClientPointRef, inspectionNoticeRef, setGesture, setMessage, setAnnotationNotice } = state;
  const { viewportPointFor, pointFor, frameMatchesCurrent, locationForInputIntent } = geometry;
  const { command, ensureControl } = view;
  const { enqueueInput, releaseRemotePointer, nextInput } = input;
  const { persist } = marks;
  const { inspect } = inspection;
  const remotePointer = (event: PointerEvent<HTMLDivElement>, kind: "move" | "down" | "up" | "cancel"): void => {
    if (!liveInputEnabledRef.current) return;
    const active = remotePointerRef.current === event.pointerId;
    const point = viewportPointFor(event, active);
    const current = snapshotRef.current;
    const accepted = frameRef.current;
    const where = current && accepted && frameMatchesCurrent(accepted) ? location(current, accepted.descriptor) : null;
    if (!point || !accepted || !where) return;
    const intent: PointerIntent = {
      pointerId: event.pointerId,
      kind,
      point: { x: point.x, y: point.y },
      button: button(event.button),
      buttons: event.buttons,
      modifiers: modifiers(event),
      clickCount: kind === "down" || kind === "up" ? Math.max(1, event.detail) : 0,
      location: { ...where },
      frame: { ...accepted.descriptor },
    };
    if (kind === "down") remotePointerRef.current = event.pointerId;
    remotePointerIntentRef.current = intent;
    remotePointRef.current = intent.point;
    void enqueueInput(kind === "move" ? "move" : "boundary", async () => {
      const controlled = await ensureControl();
      if (!controlled) {
        if (kind === "down") {
          remotePointerRef.current = null;
          remotePointerIntentRef.current = null;
          remotePointRef.current = null;
        }
        return;
      }
      const inputLocation = locationForInputIntent(intent);
      if (!inputLocation) {
        remotePointerRef.current = null;
        remotePointerIntentRef.current = null;
        remotePointRef.current = null;
        return;
      }
      const input_sequence = nextInput();
      const outcome = await command({ type: "pointer", location: inputLocation, input: { kind, button: intent.button, x: intent.point.x, y: intent.point.y, buttons: intent.buttons, modifiers: intent.modifiers, click_count: intent.clickCount, input_sequence } });
      if (kind === "up" || kind === "cancel" || !outcome) {
        remotePointerRef.current = null;
        remotePointerIntentRef.current = null;
        remotePointRef.current = null;
      }
    });
  };
  const previousToolRef = useRef<Tool>(tool);
  useEffect(() => {
    if (previousToolRef.current !== tool) {
      previousToolRef.current = tool;
      const active = gestureRef.current;
      if (active && surfaceRef.current?.hasPointerCapture(active.pointerId)) surfaceRef.current.releasePointerCapture(active.pointerId);
      gestureRef.current = null;
      setGesture(null);
      void releaseRemotePointer();
    }
  }, [releaseRemotePointer, tool]);
  useEffect(() => {
    const release = () => {
      const active = gestureRef.current;
      if (active && surfaceRef.current?.hasPointerCapture(active.pointerId)) surfaceRef.current.releasePointerCapture(active.pointerId);
      gestureRef.current = null;
      setGesture(null);
      void releaseRemotePointer();
    };
    window.addEventListener("blur", release);
    return () => window.removeEventListener("blur", release);
  }, [releaseRemotePointer]);
  const onPointerDown = (event: PointerEvent<HTMLDivElement>): void => {
    if (isLocalBrowserChrome(event.target)) return;
    if ((tool === "browse" || tool === "element") && !liveInputEnabledRef.current) return;
    onInteractionFocus?.();
    if (tool === "browse") {
      const point = pointFor(event);
      if (point) {
        event.currentTarget.setPointerCapture(event.pointerId);
        event.currentTarget.focus({ preventScroll: true });
        remotePointer(event, "down");
      }
      return;
    }
    event.preventDefault();
    const documentPoint = pointFor(event);
    if (tool === "element") {
      setAnnotationNotice(null);
      selectingElementRef.current = true;
      if (hoverInspectTimerRef.current !== null) window.clearTimeout(hoverInspectTimerRef.current);
      hoverInspectTimerRef.current = null;
      ++inspectRequestRef.current;
      hoverInspectPointRef.current = null;
      elementIntentRef.current = null;
      event.currentTarget.setPointerCapture(event.pointerId);
      event.currentTarget.focus({ preventScroll: true });
      return;
    }
    if (!documentPoint) return;
    if (tool === "select") return;
    const current = snapshotRef.current; const accepted = frameRef.current;
    if (!current?.document || !current.viewport || !accepted) return;
    setAnnotationNotice(null);
    const active = { pointerId: event.pointerId, points: [documentPoint], origin: documentPoint, frame: accepted.sequence, document: current.document.document_generation, viewport: current.viewport.viewport_revision };
    gestureRef.current = active; setGesture(active); event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>): void => {
    if (isLocalBrowserChrome(event.target)) return;
    if (tool === "browse") { remotePointer(event, "move"); return; }
    const active = gestureRef.current;
    if (active?.pointerId === event.pointerId) {
      const point = pointFor(event);
      if (point) {
        if (tool === "region") {
          const next = { ...active, points: [active.origin, point] };
          gestureRef.current = next; setGesture(next);
        } else if (active.points.length >= MAX_ANNOTATION_POINTS - 1) {
          gestureRef.current = null; setGesture(null);
          if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
          setAnnotationNotice(`Freehand annotation exceeds ${MAX_ANNOTATION_POINTS} points; the unfinished mark was cancelled. Draw a shorter stroke.`);
        } else {
          active.points.push(point);
          setGesture({ ...active });
        }
      }
    } else if (tool === "element" && !selectingElementRef.current) {
      hoverInspectPointRef.current = { clientX: event.clientX, clientY: event.clientY };
      if (hoverInspectTimerRef.current === null) {
        hoverInspectTimerRef.current = window.setTimeout(() => {
          hoverInspectTimerRef.current = null;
          const sample = hoverInspectPointRef.current;
          if (!selectingElementRef.current && sample) {
            const point = viewportPointFor(sample);
            if (point) void inspect(point);
          }
        }, 40);
      }
    }
  };
  const finishGesture = (event: PointerEvent<HTMLDivElement>, cancelled: boolean): void => {
    const active = gestureRef.current; gestureRef.current = null; setGesture(null); if (!active || cancelled) return;
    const current = snapshotRef.current; const accepted = frameRef.current; const last = pointFor(event) ?? active.points[active.points.length - 1];
    if (!current?.document || !current.viewport || !accepted || current.document.document_generation !== active.document || current.viewport.viewport_revision !== active.viewport) { setMessage("The page geometry changed while drawing; the unfinished mark was cancelled."); return; }
    if (tool === "freehand") {
      if (active.points.length >= MAX_ANNOTATION_POINTS) {
        setAnnotationNotice(`Freehand annotation exceeds ${MAX_ANNOTATION_POINTS} points; the unfinished mark was cancelled. Draw a shorter stroke.`);
        return;
      }
      active.points.push(last);
      const points = simplify(active.points);
      if (points.length > 1) persist({ id: annotationId(), kind: "freehand", color, points, bounds: null, evidence: null, comment: null });
    }
    if (tool === "region") { const bounds = rectFrom(active.origin, last); if (bounds.width >= 1 || bounds.height >= 1) persist({ id: annotationId(), kind: "region", color, points: [active.origin, last], bounds, evidence: null, comment: null }); }
  };
  const onPointerUp = (event: PointerEvent<HTMLDivElement>): void => {
    if (isLocalBrowserChrome(event.target)) return;
    if (tool === "browse") { remotePointer(event, "up"); if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); return; }
    if (tool === "element") {
      if (hoverInspectTimerRef.current !== null) window.clearTimeout(hoverInspectTimerRef.current);
      hoverInspectTimerRef.current = null;
      if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
      const current = snapshotRef.current;
      hoverInspectPointRef.current = null;
      const accepted = frameRef.current;
      if (!current?.document || !current.viewport || !current.displayed_target_id) {
        selectingElementRef.current = false;
        setMessage("Element inspection is unavailable until a page and frame are ready.");
        return;
      }
      if (!accepted || !frameMatchesCurrent(accepted)) {
        selectingElementRef.current = false;
        pendingElementClientPointRef.current = { clientX: event.clientX, clientY: event.clientY, targetId: current.displayed_target_id, documentGeneration: current.document.document_generation, viewportRevision: current.viewport.viewport_revision };
        setMessage("Element inspection is waiting for a confirmed browser frame.");
        return;
      }
      const point = viewportPointFor(event);
      if (!point) { selectingElementRef.current = false; return; }
      void inspect(point).then((result) => {
        const latest = snapshotRef.current;
        if (result?.freshness === "fresh" && result.inspectable && result.evidence && result.bounds && latest?.document && latest.viewport
          && result.location.target_id === latest.displayed_target_id
          && result.location.document_generation === latest.document.document_generation
          && result.location.viewport_revision === latest.viewport.viewport_revision
          && result.frame_id === latest.document.frame_id
          && result.frame_generation === latest.document.frame_generation) {
          const bounds = {
            ...result.bounds,
            x: result.bounds.x + latest.viewport.scroll_x,
            y: result.bounds.y + latest.viewport.scroll_y,
          };
          void persist({ id: annotationId(), kind: "element", color, points: [{ x: bounds.x, y: bounds.y }], bounds, evidence: result.evidence, comment: null });
        } else if (result) {
          const notice = result.limitation ?? "The selected element is not fresh or accessible; select it again.";
          inspectionNoticeRef.current = notice;
          setMessage(notice);
          setAnnotationNotice(notice);
        }
      }).finally(() => { selectingElementRef.current = false; });
      return;
    }
    finishGesture(event, false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  };
  const onPointerCancel = (event: PointerEvent<HTMLDivElement>): void => {
    if (isLocalBrowserChrome(event.target)) return;
    if (tool === "browse") { remotePointer(event, "cancel"); if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); return; }
    if (tool === "element") {
      selectingElementRef.current = false;
      if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
      return;
    }
    finishGesture(event, true);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  };
  return { onPointerDown, onPointerMove, onPointerUp, onPointerCancel };
}
