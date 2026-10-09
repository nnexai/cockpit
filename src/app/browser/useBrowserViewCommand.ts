import { useCallback, useRef } from "react";
import type { BrowserPoint, BrowserViewCommand, BrowserViewCommandOutcome, BrowserViewLocation, BrowserViewViewportRequest } from "../../protocol/generated/v1";
import type { BrowserViewFramePacket } from "../../client/CockpitClient";
import { IBFV_V2_DEFAULT_LIMITS } from "./framePresenter";
import { createBrowserTransform } from "./transform";
import { errorMessage, location, newId, ownsBrowserControl, type PointerIntent } from "./browserPaneModel";
import type { BrowserPaneState } from "./useBrowserPaneState";

export function useBrowserFrameGeometry(state: BrowserPaneState, viewport: BrowserViewViewportRequest): BrowserFrameGeometry {
  const { frameRef, snapshotRef, identityRef, surfaceRef } = state;
  const viewportRef = useRef(viewport);
  viewportRef.current = viewport;
  const frameSupportsViewportInput = useCallback((candidate = frameRef.current): boolean => {
    const current = snapshotRef.current; const identity = identityRef.current;
    const descriptor = candidate?.descriptor;
    return Boolean(current?.document && current.viewport && current.displayed_target_id && identity && descriptor
      && descriptor.stream_epoch === identity.epoch
      && descriptor.target_id === current.displayed_target_id
      && descriptor.document_generation === current.document.document_generation
      && Math.abs(descriptor.viewport_css_width - current.viewport.css_width) <= 0.01
      && Math.abs(descriptor.viewport_css_height - current.viewport.css_height) <= 0.01);
  }, []);
  const inputIntentMatchesCurrent = useCallback((intent: { location: BrowserViewLocation; frame: BrowserViewFramePacket["descriptor"] }): boolean => {
    const current = snapshotRef.current;
    const descriptor = frameRef.current?.descriptor;
    return Boolean(current?.document && current.viewport && current.displayed_target_id
      && descriptor
      && intent.location.target_id === current.displayed_target_id
      && intent.location.document_generation === current.document.document_generation
      && intent.location.viewport_revision === current.viewport.viewport_revision
      && descriptor.target_id === intent.location.target_id
      && descriptor.document_generation === intent.location.document_generation
      && descriptor.viewport_revision === intent.location.viewport_revision
      && Math.abs(descriptor.viewport_css_width - intent.frame.viewport_css_width) <= 0.01
      && Math.abs(descriptor.viewport_css_height - intent.frame.viewport_css_height) <= 0.01
      && Math.abs(descriptor.viewport_offset_x - intent.frame.viewport_offset_x) <= 0.01
      && Math.abs(descriptor.viewport_offset_y - intent.frame.viewport_offset_y) <= 0.01
      && Math.abs(descriptor.scroll_x - intent.frame.scroll_x) <= 0.01
      && Math.abs(descriptor.scroll_y - intent.frame.scroll_y) <= 0.01);
  }, []);
  const locationForInputIntent = useCallback((intent: Pick<PointerIntent, "location" | "frame">): BrowserViewLocation | null => {
    const current = snapshotRef.current;
    return current && ownsBrowserControl(current) && inputIntentMatchesCurrent(intent)
      ? { ...intent.location, lease_generation: current.control.lease_generation }
      : null;
  }, [inputIntentMatchesCurrent]);
  const frameMatchesCurrent = useCallback((candidate = frameRef.current): boolean => {
    const current = snapshotRef.current; const descriptor = candidate?.descriptor;
    return frameSupportsViewportInput(candidate) && Boolean(descriptor && current?.viewport
      && descriptor.viewport_revision === current.viewport.viewport_revision
      && Math.abs(descriptor.scroll_x - current.viewport.scroll_x) <= 0.01
      && Math.abs(descriptor.scroll_y - current.viewport.scroll_y) <= 0.01);
  }, [frameSupportsViewportInput]);
  const paneViewport = useCallback((): BrowserViewViewportRequest => {
    const bounds = surfaceRef.current?.getBoundingClientRect();
    const cssWidth = Math.max(1, Math.round(bounds?.width || viewportRef.current.css_width));
    const cssHeight = Math.max(1, Math.round(bounds?.height || viewportRef.current.css_height));
    const requestedDpr = typeof window === "undefined" ? viewportRef.current.device_pixel_ratio : window.devicePixelRatio;
    const maxDpr = Math.min(
      IBFV_V2_DEFAULT_LIMITS.max_width / cssWidth,
      IBFV_V2_DEFAULT_LIMITS.max_height / cssHeight,
      Math.sqrt(IBFV_V2_DEFAULT_LIMITS.max_pixels / (cssWidth * cssHeight)),
    );
    return {
      css_width: cssWidth,
      css_height: cssHeight,
      device_pixel_ratio: Math.min(Math.max(0.1, requestedDpr || 1), maxDpr),
    };
  }, []);
  const paintedRectFor = useCallback((allowScrollTransition = false) => {
    const current = frameRef.current; const surface = surfaceRef.current;
    if (!current || !surface || !(allowScrollTransition ? frameSupportsViewportInput(current) : frameMatchesCurrent(current))) return null;
    const bounds = surface.getBoundingClientRect();
    return { left: bounds.left, top: bounds.top, width: bounds.width, height: bounds.height };
  }, [frameMatchesCurrent, frameSupportsViewportInput]);
  const pointFor = useCallback((event: { clientX: number; clientY: number }, allowOutside = false): BrowserPoint | null => {
    const painted = paintedRectFor(); if (!painted || !frameRef.current) return null;
    let clientX = event.clientX; let clientY = event.clientY;
    if (allowOutside) {
      clientX = Math.max(painted.left, Math.min(painted.left + painted.width - Number.EPSILON, clientX));
      clientY = Math.max(painted.top, Math.min(painted.top + painted.height - Number.EPSILON, clientY));
    }
    return createBrowserTransform(frameRef.current.descriptor, painted)?.clientToDocument(clientX, clientY) ?? null;
  }, [paintedRectFor]);
  const viewportPointFor = useCallback((event: { clientX: number; clientY: number }, allowOutside = false, allowScrollTransition = false): BrowserPoint | null => {
    const painted = paintedRectFor(allowScrollTransition); if (!painted || !frameRef.current) return null;
    let clientX = event.clientX; let clientY = event.clientY;
    if (allowOutside) {
      clientX = Math.max(painted.left, Math.min(painted.left + painted.width - Number.EPSILON, clientX));
      clientY = Math.max(painted.top, Math.min(painted.top + painted.height - Number.EPSILON, clientY));
    }
    return createBrowserTransform(frameRef.current.descriptor, painted)?.clientToViewport(clientX, clientY) ?? null;
  }, [paintedRectFor]);
  const localLocation = (): BrowserViewLocation | null => snapshotRef.current && frameMatchesCurrent() && frameRef.current ? location(snapshotRef.current, frameRef.current.descriptor) : null;
  return { frameSupportsViewportInput, inputIntentMatchesCurrent, locationForInputIntent, frameMatchesCurrent, paneViewport, paintedRectFor, pointFor, viewportPointFor, localLocation };
}

export interface BrowserFrameGeometry {
  frameSupportsViewportInput(candidate?: BrowserPaneState["frame"]): boolean;
  inputIntentMatchesCurrent(intent: { location: BrowserViewLocation; frame: BrowserViewFramePacket["descriptor"] }): boolean;
  locationForInputIntent(intent: Pick<PointerIntent, "location" | "frame">): BrowserViewLocation | null;
  frameMatchesCurrent(candidate?: BrowserPaneState["frame"]): boolean;
  paneViewport(): BrowserViewViewportRequest;
  paintedRectFor(allowScrollTransition?: boolean): { left: number; top: number; width: number; height: number } | null;
  pointFor(event: { clientX: number; clientY: number }, allowOutside?: boolean): BrowserPoint | null;
  viewportPointFor(event: { clientX: number; clientY: number }, allowOutside?: boolean, allowScrollTransition?: boolean): BrowserPoint | null;
  localLocation(): BrowserViewLocation | null;
}

export function useBrowserViewCommand(state: BrowserPaneState, geometry: BrowserFrameGeometry): BrowserViewCommandApi {
  const { liveInputEnabledRef, streamRef, identityRef, snapshotRef, inputSequence, staleInputNoticeRef, inspectionNoticeRef, errorRef, frameRef, presenterRef, releaseOverloadedInputRef, setMessage, setStatus, applyDraft, applySnapshot, invalidateInteractionFrame, setPendingCaptureState } = state;
  const { paneViewport } = geometry;
  const controlPromiseRef = useRef<Promise<boolean> | null>(null);
  const command = useCallback(async (value: BrowserViewCommand, stillCurrent?: () => boolean): Promise<BrowserViewCommandOutcome | null> => {
    if (!liveInputEnabledRef.current) return null;
    const stream = streamRef.current; const identity = identityRef.current;
    if (!stream || !identity) { setMessage("Browser controls are still connecting."); return null; }
    try {
      if (!liveInputEnabledRef.current) return null;
      const response = await stream.command({ view_id: identity.id, stream_epoch: identity.epoch, request_id: newId("browser-command"), command: value });
      if (!liveInputEnabledRef.current) return null;
      if (identityRef.current !== identity || (stillCurrent && !stillCurrent())) return null;
      if (response.status !== "accepted") {
        if ("code" in response && response.code === "stale_input_sequence") {
          inputSequence.current = snapshotRef.current?.control.next_input_sequence ?? 1;
          staleInputNoticeRef.current?.report(`Input stale: ${response.message || "the input sequence is no longer current"} Retry the gesture; it was not replayed.`);
          return null;
        }
        if (response.status === "stale") return null;
        if (response.status === "unsupported") {
          const notice = `Input unsupported: ${response.message}`;
          inspectionNoticeRef.current = value.type === "inspect" ? notice : null;
          setStatus("unsupported");
          setMessage(notice);
        } else if (response.status === "rejected") {
          inspectionNoticeRef.current = null;
          errorRef.current = true;
          setStatus("error");
          setMessage(`Input rejected: ${response.message}`);
        } else if (response.status === "outcome_unknown") {
          inspectionNoticeRef.current = null;
          errorRef.current = true;
          setStatus("error");
          setMessage(`Input outcome unknown: ${response.message} Do not retry this gesture automatically; start a new gesture.`);
        }
        return null;
      }
      staleInputNoticeRef.current?.resolve();
      if (response.outcome.type === "snapshot") {
        const current = snapshotRef.current;
        const next = response.outcome.snapshot;
        if (!current || (next.identity.association_key === current.identity.association_key
          && next.identity.browser_incarnation === current.identity.browser_incarnation
          && next.identity.stream_epoch === current.identity.stream_epoch
          && next.metadata_sequence >= current.metadata_sequence)) {
          const descriptor = frameRef.current?.descriptor;
          if (descriptor && (descriptor.target_id !== next.displayed_target_id
            || descriptor.document_generation !== next.document?.document_generation
            || descriptor.viewport_revision !== next.viewport?.viewport_revision
            || Math.abs(descriptor.viewport_css_width - (next.viewport?.css_width ?? descriptor.viewport_css_width)) > 0.01
            || Math.abs(descriptor.viewport_css_height - (next.viewport?.css_height ?? descriptor.viewport_css_height)) > 0.01
            || Math.abs(descriptor.scroll_x - (next.viewport?.scroll_x ?? descriptor.scroll_x)) > 0.01
            || Math.abs(descriptor.scroll_y - (next.viewport?.scroll_y ?? descriptor.scroll_y)) > 0.01)) {
            invalidateInteractionFrame();
          }
          applySnapshot(next);
          presenterRef.current?.revalidate();
        }
      } else if (response.outcome.type === "control" && snapshotRef.current
        && response.outcome.control.lease_generation >= snapshotRef.current.control.lease_generation) { inputSequence.current = response.outcome.control.next_input_sequence; applySnapshot({ ...snapshotRef.current, control: response.outcome.control }); }
      else if (response.outcome.type === "draft") {
        const current = snapshotRef.current;
        if (current?.displayed_target_id === response.outcome.draft.target_id
          && current?.document?.document_generation === response.outcome.draft.document_generation) applyDraft(response.outcome.draft);
      } else if (response.outcome.type === "capture_prepared") setMessage(`Composing capture ${response.outcome.capture_id}…`);
      else if (response.outcome.type === "capture") {
        if (response.outcome.capture.state === "pending") {
          setPendingCaptureState(response.outcome.capture.pending);
          setMessage("Capture is pending; Send annotations will retry it.");
        } else if (response.outcome.capture.state === "saved") {
          setPendingCaptureState(null);
        }
      } else if (response.outcome.type === "clipboard" && response.outcome.text && navigator.clipboard?.writeText) await navigator.clipboard.writeText(response.outcome.text);
      errorRef.current = false;
      return response.outcome;
    } catch (error) {
      if (identityRef.current !== identity || streamRef.current !== stream || (stillCurrent && !stillCurrent())) return null;
      errorRef.current = true;
      setStatus("error");
      setMessage(errorMessage(error));
      return null;
    }
  }, [applyDraft, applySnapshot, invalidateInteractionFrame, setPendingCaptureState]);
  releaseOverloadedInputRef.current = () => {
    const current = snapshotRef.current;
    if (current && ownsBrowserControl(current)) {
      void command({ type: "release_control", lease_generation: current.control.lease_generation });
    }
  };
  const ensureControl = useCallback(async (): Promise<boolean> => {
    if (!liveInputEnabledRef.current) return false;
    const current = snapshotRef.current;
    const identity = identityRef.current;
    if (!current || !identity) return false;
    if (ownsBrowserControl(current)) return true;
    if (controlPromiseRef.current) return controlPromiseRef.current;
    if (!current.control.can_take_control) { setMessage("Another Cockpit view controls this browser."); return false; }
    const pending = command({
      type: "take_control", viewport: paneViewport(),
    }).then(() => identityRef.current === identity && ownsBrowserControl(snapshotRef.current)).catch(() => false);
    controlPromiseRef.current = pending;
    void pending.finally(() => { if (controlPromiseRef.current === pending) controlPromiseRef.current = null; });
    return pending;
  }, [command, paneViewport]);
  return { command, ensureControl };
}

export interface BrowserViewCommandApi {
  command(value: BrowserViewCommand, stillCurrent?: () => boolean): Promise<BrowserViewCommandOutcome | null>;
  ensureControl(): Promise<boolean>;
}
