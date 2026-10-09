import { useEffect, useRef } from "react";
import type { BrowserTarget, BrowserViewEvent, BrowserViewOpenRequest, BrowserViewViewportRequest } from "../../protocol/generated/v1";
import type { BrowserViewFramePacket, BrowserViewStream, CockpitClient } from "../../client/CockpitClient";
import { BrowserFrameError, FramePresenter, canvasBackingSize, validateFrameDescriptor } from "./framePresenter";
import { context, DelayedNotice, errorMessage, FRAME_BEHIND_MESSAGE, newId, ownsBrowserControl, statusFor, viewportFits } from "./browserPaneModel";
import { frameContext } from "./browserCanvas";
import type { BrowserPaneState } from "./useBrowserPaneState";
import type { BrowserFrameGeometry, BrowserViewCommandApi } from "./useBrowserViewCommand";
import type { BrowserInputQueue } from "./useBrowserInputQueue";
import type { BrowserDrafts } from "./useBrowserDrafts";


export function useBrowserViewStream(state: BrowserPaneState, geometry: BrowserFrameGeometry, input: BrowserInputQueue, drafts: BrowserDrafts, { client, target, clientId, visible, liveInputEnabled }: { client: CockpitClient; target: BrowserTarget; clientId: string | undefined; visible: boolean; liveInputEnabled: boolean }) {
  const { associationOwner, snapshotRef, frameRef, streamRef, identityRef, presenterRef, canvasRef, draftRef, draftRequestRef, pendingDeliveryIdsRef, pendingCaptureRef, editorDirtyRef, hoverInspectTimerRef, hoverInspectPointRef, selectingElementRef, inspectRequestRef, elementIntentRef, inputGenerationRef, inputJobsRef, remotePointerRef, remotePointerIntentRef, remotePointRef, gestureRef, inputSequence, inputOverloadedRef, inspectionNoticeRef, errorRef, retry, applySnapshot, clearPresentedFrame, invalidateInteractionFrame, setStatus, setMessage, setGesture, setFrame, setInspection, setSnapshot, setDraft, setPendingCapture, setSelectedId, setNoteId, setNoteValue, setNotesOpen } = state;
  const { frameMatchesCurrent, paneViewport } = geometry;
  const { releaseRemotePointer } = input;
  const { openDraft, persistEditor, refreshBrowserFeedback, retireDraftsFor } = drafts;
  const wasLiveInputRef = useRef(liveInputEnabled);
  const clientRef = useRef(clientId ?? newId("cockpit-browser-view"));
  const previousAssociationOwnerKeyRef = useRef<string | null>(null);
  useEffect(() => {
    const associationChanged = previousAssociationOwnerKeyRef.current !== associationOwner.key;
    previousAssociationOwnerKeyRef.current = associationOwner.key;
    let closed = false;
    let presenter: FramePresenter | null = null;
    // Dropped or stale frames only matter when no newer frame replaces them.
    const frameNotice = new DelayedNotice(
      (notice) => { if (!closed && !errorRef.current) { setStatus("error"); setMessage(notice); } },
      (notice) => { if (!errorRef.current) setMessage((current) => current === notice ? null : current); },
    );
    let stream: BrowserViewStream | null = null;
    let cursor: number | null = null;
    const controller = new AbortController();
    const close = () => {
      if (closed) return;
      closed = true;
      frameNotice.dispose();
      if (editorDirtyRef.current && draftRef.current) void persistEditor().catch(() => undefined);
      if (hoverInspectTimerRef.current !== null) window.clearTimeout(hoverInspectTimerRef.current);
      hoverInspectTimerRef.current = null;
      hoverInspectPointRef.current = null;
      selectingElementRef.current = false;
      ++inspectRequestRef.current;
      elementIntentRef.current = null;
      controller.abort();
      ++inputGenerationRef.current;
      inputJobsRef.current = [];
      remotePointerRef.current = null;
      remotePointerIntentRef.current = null;
      remotePointRef.current = null;
      gestureRef.current = null;
      setGesture(null);
      presenter?.close();
      if (presenterRef.current === presenter) presenterRef.current = null;
      presenter = null;
      if (streamRef.current === stream) streamRef.current = null;
      stream?.close();
      stream = null;
      identityRef.current = null;
      clearPresentedFrame();
      draftRequestRef.current += 1;
    };
    streamRef.current = null;
    identityRef.current = null;
    inputSequence.current = 1;
    pendingDeliveryIdsRef.current = associationChanged ? null : pendingDeliveryIdsRef.current;
    pendingCaptureRef.current = associationChanged ? null : pendingCaptureRef.current;
    draftRequestRef.current += 1;
    inputJobsRef.current = [];
    inputOverloadedRef.current = false;
    setFrame(null);
    inspectionNoticeRef.current = null;
    setInspection(null);
    setGesture(null);
    setStatus(visible ? "loading" : "hidden");
    if (associationChanged) {
      snapshotRef.current = null;
      draftRef.current = null;
      associationOwner.draft = null;
      associationOwner.noteId = null;
      associationOwner.noteText = "";
      associationOwner.notesOpen = false;
      associationOwner.editorGeneration = 0;
      setSnapshot(null);
      setDraft(null);
      setPendingCapture(null);
      setSelectedId(null);
      setNoteId(null);
      setNoteValue("");
      setNotesOpen(false);
      editorDirtyRef.current = false;
    } else {
      setPendingCapture(pendingCaptureRef.current);
    }
    gestureRef.current = null;
    if (!visible) return close;
    const resetDisplayedDocument = () => {
      ++inputGenerationRef.current; clearPresentedFrame(); gestureRef.current = null; setGesture(null); draftRequestRef.current += 1; setSelectedId(null); setInspection(null);
      draftRef.current = null; associationOwner.draft = null; associationOwner.localDraftRevision = null;
      associationOwner.noteId = null; associationOwner.noteText = ""; associationOwner.notesOpen = false; associationOwner.editorGeneration += 1;
      editorDirtyRef.current = false; setDraft(null); setNoteId(null); setNoteValue(""); setNotesOpen(false); setMessage(null); setStatus("loading");
    };
    const event = (incoming: BrowserViewEvent): void => {
      if (closed) return;
      if (incoming.type === "attached") {
        if (identityRef.current) return;
        identityRef.current = { id: incoming.metadata.view_id, epoch: incoming.metadata.stream_epoch }; cursor = incoming.metadata.metadata_sequence; inputSequence.current = incoming.snapshot.control.next_input_sequence; applySnapshot(incoming.snapshot); setStatus(statusFor(incoming.snapshot)); queueMicrotask(() => { void openDraft(); void refreshBrowserFeedback(); }); return;
      }
      const identity = identityRef.current;
      if (!identity || incoming.metadata.view_id !== identity.id || incoming.metadata.stream_epoch !== identity.epoch) return;
      if (cursor !== null && incoming.metadata.metadata_sequence <= cursor) return;
      cursor = incoming.metadata.metadata_sequence;
      const previous = snapshotRef.current; if (!previous) return;
      let next = previous;
      switch (incoming.type) {
        case "targets_changed": {
          next = { ...previous, targets: incoming.targets, displayed_target_id: incoming.displayed_target_id };
          if (previous.displayed_target_id !== incoming.displayed_target_id) {
            void releaseRemotePointer();
            resetDisplayedDocument();
          }
          break;
        }
        case "document_changed": {
          if (incoming.document && previous.document
            && previous.document.target_id === incoming.document.target_id
            && previous.document.document_generation !== incoming.document.document_generation
            && previous.displayed_target_id === incoming.document.target_id) {
            retireDraftsFor(previous.document.target_id, previous.document.document_generation);
          }
          void releaseRemotePointer();
          next = { ...previous, document: incoming.document };
          resetDisplayedDocument();
          break;
        }
        case "navigation_changed": {
          next = { ...previous, navigation: incoming.navigation };
          if (incoming.navigation?.loading) invalidateInteractionFrame();
          else if (previous.navigation?.url !== incoming.navigation?.url) {
            setMessage((current) => current?.includes("browser URL has an unsupported or unsafe scheme") ? null : current);
          }
          break;
        }
        case "viewport_changed": {
          next = { ...previous, viewport: incoming.viewport };
          if (previous.viewport?.viewport_revision !== incoming.viewport?.viewport_revision) {
            gestureRef.current = null;
            setGesture(null);
          }
          setInspection(null);
          break;
        }
        case "cursor_changed": next = { ...previous, cursor: incoming.cursor }; break;
        case "blocker_changed": next = { ...previous, blocker: incoming.blocker }; break;
        case "capabilities_changed": next = { ...previous, capabilities: incoming.capabilities }; break;
        case "control_changed": inputSequence.current = incoming.control.next_input_sequence; next = { ...previous, control: incoming.control }; break;
        case "frame_descriptor": return;
        case "frame_transport_revoked": setStatus("error"); setMessage(incoming.message); return;
        case "failed":
          if (incoming.code === "browser_frame_invalid") { frameNotice.report(FRAME_BEHIND_MESSAGE); return; }
          setStatus("error"); setMessage(incoming.message); return;
        case "closed": setStatus("error"); setMessage(incoming.reason); return;
      }
      applySnapshot(next);
      presenter?.revalidate();
      if (incoming.type === "document_changed"
        || (incoming.type === "targets_changed"
          && previous.displayed_target_id !== next.displayed_target_id
          && next.document?.target_id === next.displayed_target_id)) {
        queueMicrotask(() => void openDraft());
      }
      if (incoming.type !== "viewport_changed" && incoming.type !== "document_changed" && incoming.type !== "navigation_changed") {
        const nextStatus = statusFor(next);
        if (nextStatus !== "loading" || !frameRef.current) setStatus(nextStatus);
      }
    };
    const canPresent = (descriptor: BrowserViewFramePacket["descriptor"]): boolean => {
      const current = snapshotRef.current; const identity = identityRef.current;
      return Boolean(current?.document && current.displayed_target_id && identity
        && descriptor.stream_epoch === identity.epoch
        && descriptor.target_id === current.displayed_target_id
        && descriptor.document_generation === current.document.document_generation);
    };
    presenter = new FramePresenter({
      isExpectedStale: (descriptor) => !canPresent(descriptor),
      shouldDefer: (descriptor) => {
        const current = snapshotRef.current; const identity = identityRef.current; const presented = frameRef.current;
        if (!current?.document || !identity || descriptor.stream_epoch !== identity.epoch) return false;
        if (presented && descriptor.frame_sequence <= presented.sequence) return false;
        const knownTarget = descriptor.target_id === current.displayed_target_id || current.targets.some((candidate) => candidate.target_id === descriptor.target_id);
        return knownTarget && descriptor.document_generation > current.document.document_generation;
      },
      validate: (descriptor) => {
        const current = snapshotRef.current;
        if (!canPresent(descriptor)) throw new BrowserFrameError("identity_mismatch", "Browser frame is for a stale document");
        validateFrameDescriptor(descriptor, { streamEpoch: identityRef.current!.epoch, targetId: current!.displayed_target_id!, displayedTargetId: current!.displayed_target_id!, documentGeneration: current!.document!.document_generation, viewportRevision: descriptor.viewport_revision, viewportCssWidth: descriptor.viewport_css_width, viewportCssHeight: descriptor.viewport_css_height });
      },
      present: (image, descriptor) => {
        if (!canPresent(descriptor)) throw new BrowserFrameError("identity_mismatch", "Browser frame became stale while decoding");
        const targetCanvas = canvasRef.current; const drawing = frameContext(targetCanvas);
        if (!targetCanvas || !drawing) throw new Error("Browser view canvas is unavailable");
        const backing = canvasBackingSize({ width: targetCanvas.width, height: targetCanvas.height }, descriptor, snapshotRef.current?.viewport?.device_pixel_ratio);
        if (targetCanvas.width !== backing.width) targetCanvas.width = backing.width;
        if (targetCanvas.height !== backing.height) targetCanvas.height = backing.height;
        drawing.drawImage(image, 0, 0, targetCanvas.width, targetCanvas.height);
        const accepted = { descriptor, sequence: descriptor.frame_sequence }; frameRef.current = accepted; setFrame(accepted);
        frameNotice.resolve();
        if (!errorRef.current) setStatus(streamRef.current ? "ready" : "loading");
      },
      onError: () => frameNotice.report(FRAME_BEHIND_MESSAGE),
    });
    presenterRef.current = presenter;
    const request: BrowserViewOpenRequest = { target, client_id: clientId ?? clientRef.current, viewport: paneViewport(), takeover: false };
    void client.openBrowserView(request, event, (packet) => presenter?.push(packet), (error) => { if (!closed) { setStatus("error"); setMessage(errorMessage(error)); } }, controller.signal).then((opened) => {
      stream = opened;
      if (closed) opened.close();
      else { streamRef.current = opened; if (frameRef.current && !errorRef.current) setStatus("ready"); void openDraft(); void refreshBrowserFeedback(); }
    }).catch((error: unknown) => { if (!closed && !controller.signal.aborted) { setStatus("error"); setMessage(errorMessage(error)); } });
    return close;
  }, [applySnapshot, associationOwner, clearPresentedFrame, client, clientId, frameMatchesCurrent, invalidateInteractionFrame, openDraft, paneViewport, persistEditor, refreshBrowserFeedback, releaseRemotePointer, retireDraftsFor, retry, target.endpoint_path, target.pane_id, target.session_id, target.tab_id, visible]);
  useEffect(() => {
    if (!liveInputEnabled) {
      ++inputGenerationRef.current;
      inputJobsRef.current = [];
      remotePointerRef.current = null;
      remotePointRef.current = null;
      gestureRef.current = null;
      setGesture(null);
      wasLiveInputRef.current = false;
      return;
    }
    if (!wasLiveInputRef.current) queueMicrotask(() => { void openDraft(); });
    wasLiveInputRef.current = true;
  }, [liveInputEnabled, openDraft]);
}

export function useBrowserViewportResize(state: BrowserPaneState, geometry: BrowserFrameGeometry, view: BrowserViewCommandApi, input: BrowserInputQueue, { viewport, visible }: { viewport: BrowserViewViewportRequest; visible: boolean }) {
  const { snapshotRef, liveInputEnabledRef, snapshot } = state;
  const { paneViewport } = geometry;
  const { command, ensureControl } = view;
  const { enqueueInput } = input;
  useEffect(() => {
    let timer = 0;
    let resolutionQuery: MediaQueryList | null = null;
    const onResolutionChange = () => {
      watchResolution();
      scheduleResize();
    };
    const watchResolution = () => {
      resolutionQuery?.removeEventListener("change", onResolutionChange);
      resolutionQuery = typeof window.matchMedia === "function"
        ? window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`)
        : null;
      resolutionQuery?.addEventListener("change", onResolutionChange, { once: true });
    };
    const scheduleResize = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const current = snapshotRef.current;
        const requested = paneViewport();
        if (!visible || !liveInputEnabledRef.current || !current?.viewport || viewportFits(current.viewport, requested)) return;
        void enqueueInput("boundary", async () => {
          const controlled = await ensureControl();
          const latest = snapshotRef.current;
          const latestRequested = paneViewport();
          const documentContext = latest ? context(latest) : null;
          if (!controlled || !latest || !latest.viewport || !ownsBrowserControl(latest) || !documentContext
            || viewportFits(latest.viewport, latestRequested)) return;
          await command({ type: "resize", context: documentContext, viewport: latestRequested });
          // Metadata invalidates old geometry; a response must not erase a newer frame.
        });
      }, 120);
    };
    scheduleResize();
    watchResolution();
    window.addEventListener("resize", scheduleResize);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("resize", scheduleResize);
      resolutionQuery?.removeEventListener("change", onResolutionChange);
    };
  }, [command, enqueueInput, ensureControl, paneViewport, snapshot?.control.status, snapshot?.control.controller_view_id, snapshot?.viewport?.viewport_revision, viewport, visible]);
}
