import type { BrowserCaptureSubmission, BrowserInlineCaptureProvenance, BrowserPoint, BrowserRect, BrowserViewCommandOutcome, BrowserViewPendingCapture } from "../../protocol/generated/v1";
import { context, errorMessage, type CaptureIdentity } from "./browserPaneModel";
import { composeAnnotatedPng } from "./browserCanvas";
import type { BrowserPaneState } from "./useBrowserPaneState";
import type { BrowserFrameGeometry, BrowserViewCommandApi } from "./useBrowserViewCommand";
import type { BrowserDrafts } from "./useBrowserDrafts";

export interface BrowserFeedbackCapture {
  capture(captureAsShown: boolean): Promise<void>;
  onPendingFeedbackChange(ids: string[] | null): void;
  beforeRecovery(): Promise<void>;
  onRecovered(outcome: BrowserViewCommandOutcome, pending: BrowserViewPendingCapture | null): Promise<void>;
}

export function useBrowserFeedbackCapture(state: BrowserPaneState, geometry: BrowserFrameGeometry, view: BrowserViewCommandApi, drafts: BrowserDrafts): BrowserFeedbackCapture {
  const { canvasRef, snapshotRef, frameRef, associationOwner, associationOwnerRef, draftRef, editorDirtyRef, noteIdRef, noteValueRef, pendingCaptureRef, pendingDeliveryIdsRef, feedbackPanelRef, captureInFlightRef, draftRequestRef, setDraft, setSelectedId, setNoteId, setNoteValue, setMessage, setStatus, setPendingCaptureState } = state;
  const { localLocation } = geometry;
  const { command } = view;
  const { openDraft, refreshBrowserFeedback, persistEditor } = drafts;
  const quarantineConsumedDraft = (identity: CaptureIdentity | null): void => {
    if (!identity || associationOwnerRef.current !== identity.owner || identity.owner.sealed) return;
    const current = draftRef.current;
    const editorUnchanged = identity.owner.editorGeneration === identity.editorGeneration
      && identity.owner.noteId === identity.noteId
      && identity.owner.noteText.slice(0, 4000) === identity.noteText
      && identity.owner.notesOpen === identity.notesOpen;
    if (current?.draft_id !== identity.draftId || current.revision !== identity.draftRevision || !editorUnchanged) return;
    draftRef.current = null;
    identity.owner.draft = null;
    identity.owner.localDraftRevision = null;
    identity.owner.noteId = null;
    identity.owner.noteText = "";
    identity.owner.notesOpen = false;
    editorDirtyRef.current = false;
    noteIdRef.current = null;
    noteValueRef.current = "";
    setDraft(null);
    setSelectedId(null);
    setNoteId(null);
    setNoteValue("");
  };
  const captureImpl = async (captureAsShown: boolean): Promise<void> => {
    const captureOwner = associationOwner;
    const captureDraft = draftRef.current;
    const captureIdentity: CaptureIdentity | null = captureDraft ? {
      owner: captureOwner,
      draftId: captureDraft.draft_id,
      draftRevision: captureDraft.revision,
      editorGeneration: captureOwner.editorGeneration,
      noteId: captureOwner.noteId,
      noteText: captureOwner.noteText.slice(0, 4000),
      notesOpen: captureOwner.notesOpen,
    } : null;
    const pending = pendingCaptureRef.current;
    const current = snapshotRef.current;
    if (pending) {
      const recovered = await feedbackPanelRef.current?.recoverPending("retry_pending");
      if (recovered?.type === "capture" && recovered.capture.state === "saved"
        && associationOwnerRef.current === captureOwner && !captureOwner.sealed) {
        await feedbackPanelRef.current?.sendCapture(recovered.capture.saved.capture_id, recovered.capture.saved.annotation_ids);
      }
      return;
    }
    const preparedSnapshot = current; const where = localLocation(); const preparedDraft = draftRef.current; const beforeFrame = frameRef.current;
    if (!preparedSnapshot || !where || !preparedDraft || !beforeFrame || !preparedSnapshot.document || !preparedSnapshot.viewport) { setMessage("Wait for a confirmed frame and annotations before sending."); return; }
    try {
      const prepared = await command({ type: "capture", command: { location: where, draft_id: preparedDraft.draft_id, draft_revision: preparedDraft.revision, annotation_ids: preparedDraft.annotations.map((annotation) => annotation.id), capture_as_shown: captureAsShown } });
      if (!prepared || prepared.type !== "capture_prepared") {
        // Another client may have changed the draft; show what is current.
        if (associationOwnerRef.current === captureOwner && !captureOwner.sealed) await openDraft();
        return;
      }
      const accepted = frameRef.current;
      if (!accepted || accepted.sequence !== prepared.descriptor.frame_sequence || accepted.descriptor.target_id !== prepared.descriptor.target_id || accepted.descriptor.stream_epoch !== prepared.descriptor.stream_epoch || accepted.descriptor.document_generation !== prepared.descriptor.document_generation || accepted.descriptor.viewport_revision !== prepared.descriptor.viewport_revision) { setMessage("Capture pixels changed before composition; retry capture."); return; }
      const png = await composeAnnotatedPng(canvasRef.current, snapshotRef.current, preparedDraft.annotations, accepted.descriptor);
      if (!png) { setMessage("Could not compose the captured browser image."); return; }
      const clampImage = (value: number, maximum: number): number => Number.isFinite(value) ? Math.max(0, Math.min(maximum, value)) : 0;
      const mapCapturePoint = (point: BrowserPoint): BrowserPoint => ({
        x: clampImage((point.x - accepted.descriptor.scroll_x - accepted.descriptor.viewport_offset_x) / accepted.descriptor.viewport_css_width * accepted.descriptor.image_width, accepted.descriptor.image_width),
        y: clampImage((point.y - accepted.descriptor.scroll_y - accepted.descriptor.viewport_offset_y) / accepted.descriptor.viewport_css_height * accepted.descriptor.image_height, accepted.descriptor.image_height),
      });
      const mapCaptureRect = (rect: BrowserRect): BrowserRect => {
        const start = mapCapturePoint({ x: rect.x, y: rect.y });
        const end = mapCapturePoint({ x: rect.x + rect.width, y: rect.y + rect.height });
        return { x: Math.min(start.x, end.x), y: Math.min(start.y, end.y), width: Math.abs(end.x - start.x), height: Math.abs(end.y - start.y) };
      };
      const captureAnnotations = preparedDraft.annotations.map((annotation) => ({
        id: annotation.id,
        kind: annotation.kind,
        comment: annotation.comment ?? "",
        color: annotation.color,
        points: annotation.points.map(mapCapturePoint),
        bounds: annotation.bounds ? mapCaptureRect(annotation.bounds) : null,
        element: annotation.evidence,
      }));
      const submission: BrowserCaptureSubmission = {
        association_key: preparedSnapshot.identity.association_key, browser_instance: preparedSnapshot.identity.browser_incarnation, capture_id: prepared.capture_id,
        page: { url: preparedSnapshot.navigation?.url ?? "", title: preparedSnapshot.navigation?.title ?? "", tab_id: null, document_id: preparedSnapshot.document.frame_id, captured_at: new Date().toISOString(), viewport: { width: preparedSnapshot.viewport.css_width, height: preparedSnapshot.viewport.css_height, scroll_x: preparedSnapshot.viewport.scroll_x, scroll_y: preparedSnapshot.viewport.scroll_y, device_pixel_ratio: preparedSnapshot.viewport.device_pixel_ratio, visual_scale: preparedSnapshot.viewport.visual_scale }, image_width: accepted.descriptor.image_width, image_height: accepted.descriptor.image_height },
        annotations: captureAnnotations, png_base64: png,
      };
      const provenance: BrowserInlineCaptureProvenance = {
        target_id: prepared.descriptor.target_id,
        frame_id: preparedSnapshot.document.frame_id,
        document_generation: prepared.descriptor.document_generation,
        frame_generation: preparedSnapshot.document.frame_generation,
        stream_epoch: prepared.descriptor.stream_epoch,
        frame_sequence: prepared.descriptor.frame_sequence,
        viewport_revision: prepared.descriptor.viewport_revision,
        pixel_captured_at_micros: prepared.descriptor.capture_timestamp_micros,
        capture_as_shown: captureAsShown,
      };
      const saved = await command({
        type: "draft",
        context: context(preparedSnapshot)!,
        draft_id: preparedDraft.draft_id,
        expected_revision: preparedDraft.revision,
        command: { type: "save_capture", submission, annotation_ids: preparedDraft.annotations.map((annotation) => annotation.id), provenance },
      });
      if (saved?.type === "capture" && saved.capture.state === "saved") {
        const ids = [...saved.capture.saved.annotation_ids];
        setPendingCaptureState(null);
        quarantineConsumedDraft(captureIdentity);
        await openDraft();
        if (associationOwnerRef.current !== captureOwner || captureOwner.sealed) return;
        await feedbackPanelRef.current?.sendCapture(saved.capture.saved.capture_id, ids);
        await openDraft();
        if (associationOwnerRef.current === captureOwner && !captureOwner.sealed) await refreshBrowserFeedback(captureOwner);
      }
    } catch (error) {
      setStatus("error"); setMessage(`Could not capture browser image: ${errorMessage(error)}`);
    }
  };
  const capture = async (captureAsShown: boolean): Promise<void> => {
    if (captureInFlightRef.current) return;
    captureInFlightRef.current = true;
    try {
      await captureImpl(captureAsShown);
    } finally {
      captureInFlightRef.current = false;
    }
  };
  const onPendingFeedbackChange = (ids: string[] | null): void => { pendingDeliveryIdsRef.current = ids; };
  const beforeRecovery = async (): Promise<void> => {
    if (captureInFlightRef.current && !pendingCaptureRef.current) throw new Error("Wait for the live capture to finish before recovering saved work.");
    if (editorDirtyRef.current && associationOwner.draft) await persistEditor();
    await associationOwner.mutationTail;
    if (associationOwner.pendingAnnotationMutations.length) throw new Error("Resolve retained annotation changes before recovering saved work.");
  };
  const onRecovered = async (outcome: BrowserViewCommandOutcome, pending: BrowserViewPendingCapture | null): Promise<void> => {
    if (associationOwnerRef.current !== associationOwner || associationOwner.sealed) return;
    if (outcome.type === "capture" && outcome.capture.state === "saved" && pending) {
      quarantineConsumedDraft({
        owner: associationOwner, draftId: pending.draft_id, draftRevision: pending.draft_revision,
        editorGeneration: associationOwner.editorGeneration, noteId: associationOwner.noteId,
        noteText: associationOwner.noteText.slice(0, 4000), notesOpen: associationOwner.notesOpen,
      });
    }
    if (outcome.type === "draft_inventory" && associationOwner.draft
      && !outcome.inventory.drafts.some((item) => item.draft_id === associationOwner.draft!.draft_id)) {
      associationOwner.retiredDraftIds.add(associationOwner.draft.draft_id);
      draftRequestRef.current += 1;
      draftRef.current = null; associationOwner.draft = null; associationOwner.localDraftRevision = null;
      editorDirtyRef.current = false;
      setDraft(null); setSelectedId(null); setNoteId(null); setNoteValue("");
    }
    await openDraft();
  };
  return { capture, onPendingFeedbackChange, beforeRecovery, onRecovered };
}
