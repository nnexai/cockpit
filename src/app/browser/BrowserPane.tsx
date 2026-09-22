import { useCallback, useEffect, useRef, useState, type ClipboardEvent, type CompositionEvent, type KeyboardEvent, type PointerEvent, type WheelEvent } from "react";
import type { BrowserCaptureSubmission, BrowserDraftRecoveryAction, BrowserFeedbackSendResponse, BrowserInlineCaptureProvenance, BrowserPoint, BrowserRect, BrowserTarget, BrowserViewCommand, BrowserViewCommandOutcome, BrowserViewDraftAnnotation, BrowserViewDraftState, BrowserViewEvent, BrowserViewInspectResult, BrowserViewLocation, BrowserViewOpenRequest, BrowserViewPendingCapture, BrowserViewPresentation, BrowserViewSnapshot, BrowserViewViewportRequest } from "../../protocol/generated/v1";
import type { BrowserViewFramePacket, BrowserViewStream, CockpitClient } from "../../client/CockpitClient";
import { BrowserFrameError, FramePresenter, IBFV_V2_DEFAULT_LIMITS, validateFrameDescriptor } from "./framePresenter";
import { createBrowserTransform } from "./transform";
import "./browser.css";

export type BrowserPaneRecoveryRegistration = {
  guard: () => Promise<void>;
  retry: () => Promise<void>;
  discard: () => Promise<void>;
  describe: () => string;
};
export interface BrowserPaneProps {
  client: CockpitClient; target: BrowserTarget; viewport: BrowserViewViewportRequest;
  visible?: boolean; presentation?: BrowserViewPresentation; clientId?: string;
  inputActive?: boolean; liveInputEnabled?: boolean; onInteractionFocus?: () => void; onFeedback?: (captureIds: string[], operationId: string, acknowledgeDuplicateRisk: boolean) => Promise<BrowserFeedbackSendResponse>;
  onReconnect?: () => void | Promise<void>; onBackToTerminals?: () => void; onExpand?: () => void;
  registerCloseGuard?: (registration: BrowserPaneRecoveryRegistration | null) => void;
  className?: string;
}

type PaneStatus = "hidden" | "loading" | "ready" | "stale" | "error" | "unsupported" | "empty";
type Tool = "browse" | "select" | "freehand" | "region" | "element";
type Gesture = { pointerId: number; points: BrowserPoint[]; origin: BrowserPoint; frame: number; document: number; viewport: number };
type WheelIntent = { clientX: number; clientY: number; deltaX: number; deltaY: number; modifiers: number };
type InputJob = { generation: number; kind: "move" | "wheel" | "boundary"; run: () => Promise<void>; wheel?: WheelIntent };
type PointerIntent = {
  pointerId: number;
  kind: "move" | "down" | "up" | "cancel";
  point: BrowserPoint;
  button: "left" | "middle" | "right" | null;
  buttons: number;
  modifiers: number;
  clickCount: number;
  location: BrowserViewLocation;
  frame: BrowserViewFramePacket["descriptor"];
};
type ElementInspectionIntent = {
  point: BrowserPoint;
  location: BrowserViewLocation;
  frame: BrowserViewFramePacket["descriptor"];
  targetId: string;
  documentGeneration: number;
  viewportRevision: number;
  frameId: string;
  frameGeneration: number;
  pointerSampleSequence: number | null;
};
type DraftAssociationOwner = {
  key: string;
  target: BrowserTarget;
  draft: BrowserViewDraftState | null;
  localDraftRevision: number | null;
  noteId: string | null;
  noteText: string;
  notesOpen: boolean;
  editorGeneration: number;
  sealed: boolean;
  mutationTail: Promise<void>;
  pendingAnnotationMutations: PendingAnnotationMutation[];
};
type PendingAnnotationMutation = {
  key: string;
  kind: "upsert" | "remove";
  draftId: string;
  expectedRevision: number;
  annotation: BrowserViewDraftAnnotation | null;
  annotationId: string;
};
type CaptureIdentity = {
  owner: DraftAssociationOwner;
  draftId: string;
  draftRevision: number;
  editorGeneration: number;
  noteId: string | null;
  noteText: string;
  notesOpen: boolean;
};
const MAX_INPUT_JOBS = 128;
const deliveryOperationId = (captureId: string): string => `browser-feedback-${captureId}`;
type DeliveryState = "pending" | "outcome_unknown" | "rejected";
const COLORS = ["#d62828", "#1769aa", "#2a9d55", "#c27803", "#7c3aed"] as const;
const TOLERANCE = 1.5;
const annotationIconPaths = {
  browse: "M2 1.5 14 9.5 9.2 10.6 12 14.8 10.1 16 7.3 11.8 4.5 14.8Z",
  select: "M14.5 8a6.5 6.5 0 1 1-13 0 6.5 6.5 0 0 1 13 0Zm-9 0 2.1 2.1L11.8 6.5",
  freehand: "M3 12c1.5-4 3-7 5-7 1.4 0 1.4 2 2.5 2 1 0 1.5-1.5 2.5-3",
  element: "M8 1v3m0 8v3M1 8h3m8 0h3M12 8a4 4 0 1 1-8 0 4 4 0 0 1 8 0Z",
  region: "M2.5 2.5h11v11h-11Z",
  remove: "M3 4.5h10M6 2.5h4M5 4.5l.6 9h4.8l.6-9M7 7v4M9 7v4",
  notes: "M14 11a3 3 0 0 1-3 3H6l-3 2v-8a3 3 0 0 1 3-3h5a3 3 0 0 1 3 3Z",
  feedback: "M14 11a3 3 0 0 1-3 3H6l-3 2v-8a3 3 0 0 1 3-3h5a3 3 0 0 1 3 3Z",
  expand: "M6 1H1v5m8-5h5v5M1 9v5h5m8-5v5H9",
} as const;
type AnnotationIconName = keyof typeof annotationIconPaths;
const AnnotationIcon = ({ name }: { name: AnnotationIconName }) => <svg className={`browser-annotation-icon browser-annotation-icon-${name}`} viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.35" strokeLinecap="round" strokeLinejoin="round"><path d={annotationIconPaths[name]} /></svg>;

const errorMessage = (error: unknown): string => error instanceof Error && error.message ? error.message : "The browser view stream is unavailable.";
const newId = (prefix: string): string => `${prefix}-${globalThis.crypto?.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`}`;
const annotationId = (): string => globalThis.crypto?.randomUUID?.() ?? "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (character) => {
  const value = Math.floor(Math.random() * 16);
  return (character === "x" ? value : (value & 0x3) | 0x8).toString(16);
});
const statusFor = (snapshot: BrowserViewSnapshot): PaneStatus => snapshot.blocker?.kind === "unsupported" ? "unsupported" : snapshot.displayed_target_id ? "loading" : "empty";
const statusText = (status: PaneStatus, message: string | null): string => {
  if (status === "ready") return "Live browser view";
  if (status === "loading") return "Loading browser view…";
  if (status === "stale") return "Showing last confirmed frame";
  if (status === "empty") return "No browser page is selected";
  if (status === "hidden") return "Browser view hidden";
  return message ?? (status === "unsupported" ? "Browser view is unsupported by this runtime" : "Browser stream error");
};
const hasRecoverableDraft = (candidate: BrowserViewDraftState): boolean =>
  candidate.annotations.length > 0 || candidate.editor.note_text.trim().length > 0;
const button = (value: number): "left" | "middle" | "right" | null => value === 0 ? "left" : value === 1 ? "middle" : value === 2 ? "right" : null;
const modifiers = (event: { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }): number => (event.altKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.metaKey ? 4 : 0) | (event.shiftKey ? 8 : 0);
const isLocalBrowserChrome = (target: EventTarget | null): boolean =>
  target instanceof Element && target.closest(".browser-note-editor, .browser-blocker, .browser-recovery") !== null;
const navigationUrl = (value: string): string => {
  const trimmed = value.trim();
  return /^[a-z][a-z\d+.-]*:/i.test(trimmed) ? trimmed : `https://${trimmed}`;
};
const rectFrom = (first: BrowserPoint, last: BrowserPoint): BrowserRect => ({ x: Math.min(first.x, last.x), y: Math.min(first.y, last.y), width: Math.abs(last.x - first.x), height: Math.abs(last.y - first.y) });
const kindFor = (annotation: BrowserViewDraftAnnotation): "freehand" | "region" | "element" => annotation.kind;
function simplify(points: BrowserPoint[]): BrowserPoint[] {
  if (points.length < 3) return points;
  const kept = new Uint8Array(points.length); kept[0] = 1; kept[points.length - 1] = 1;
  const squared = (point: BrowserPoint, from: BrowserPoint, to: BrowserPoint): number => {
    const dx = to.x - from.x; const dy = to.y - from.y;
    const ratio = dx === 0 && dy === 0 ? 0 : Math.max(0, Math.min(1, ((point.x - from.x) * dx + (point.y - from.y) * dy) / (dx * dx + dy * dy)));
    const x = point.x - from.x - ratio * dx; const y = point.y - from.y - ratio * dy; return x * x + y * y;
  };
  const visit = (from: number, to: number): void => {
    let maximum = TOLERANCE * TOLERANCE; let index = -1;
    for (let candidate = from + 1; candidate < to; candidate += 1) { const distance = squared(points[candidate], points[from], points[to]); if (distance > maximum) { maximum = distance; index = candidate; } }
    if (index >= 0) { kept[index] = 1; visit(from, index); visit(index, to); }
  };
  visit(0, points.length - 1); return points.filter((_, index) => kept[index] === 1);
}
function context(snapshot: BrowserViewSnapshot) {
  return snapshot.document && snapshot.displayed_target_id ? { target_id: snapshot.displayed_target_id, document_generation: snapshot.document.document_generation, lease_generation: snapshot.control.lease_generation } : null;
}
function location(snapshot: BrowserViewSnapshot, frame: BrowserViewFramePacket["descriptor"]): BrowserViewLocation | null {
  return snapshot.document && snapshot.viewport && snapshot.displayed_target_id ? { target_id: snapshot.displayed_target_id, document_generation: snapshot.document.document_generation, viewport_revision: snapshot.viewport.viewport_revision, presented_frame_sequence: frame.frame_sequence, lease_generation: snapshot.control.lease_generation } : null;
}
const ownsBrowserControl = (snapshot: BrowserViewSnapshot | null): boolean =>
  snapshot?.control.status === "controlled" && snapshot.control.controller_view_id === snapshot.identity.view_id;
export function BrowserPane({ client, target, viewport, visible = true, presentation = "split", clientId, inputActive = true, liveInputEnabled = true, onInteractionFocus, onFeedback, onReconnect, onBackToTerminals, onExpand, registerCloseGuard, className }: BrowserPaneProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const streamRef = useRef<BrowserViewStream | null>(null);
  const identityRef = useRef<{ id: string; epoch: number } | null>(null);
  const frameRef = useRef<{ descriptor: BrowserViewFramePacket["descriptor"]; sequence: number } | null>(null);
  const presenterRef = useRef<FramePresenter | null>(null);
  const snapshotRef = useRef<BrowserViewSnapshot | null>(null);
  const pendingDeliveryIdsRef = useRef<string[] | null>(null);
  const pendingDeliveryIdentityRef = useRef<CaptureIdentity | null>(null);
  const pendingDeliveryOperationRef = useRef<string | null>(null);
  const inputOverloadedRef = useRef(false);
  const releaseOverloadedInputRef = useRef<(() => void) | null>(null);
  const draftRef = useRef<BrowserViewDraftState | null>(null);
  const pendingCaptureRef = useRef<BrowserViewPendingCapture | null>(null);
  const draftRequestRef = useRef(0);
  const editorDirtyRef = useRef(false);
  const editorPersistRef = useRef<Promise<void> | null>(null);
  const wasLiveInputRef = useRef(liveInputEnabled);
  const inputSequence = useRef(1);
  const captureRef = useRef<((captureAsShown: boolean) => Promise<void>) | null>(null);
  const inputJobsRef = useRef<InputJob[]>([]);
  const inputGenerationRef = useRef(0);
  const inputDrainRef = useRef<Promise<void> | null>(null);
  const gestureRef = useRef<Gesture | null>(null);
  const controlPromiseRef = useRef<Promise<boolean> | null>(null);
  const remotePointerRef = useRef<number | null>(null);
  const remotePointRef = useRef<BrowserPoint | null>(null);
  const navigationRequestRef = useRef(0);
  const remotePointerIntentRef = useRef<PointerIntent | null>(null);
  const elementIntentRef = useRef<ElementInspectionIntent | null>(null);
  const pendingElementClientPointRef = useRef<{ clientX: number; clientY: number; targetId: string; documentGeneration: number; viewportRevision: number } | null>(null);
  const inspectRequestRef = useRef(0);
  const urlEditing = useRef(false);
  const errorRef = useRef(false);
  const clientRef = useRef(clientId ?? newId("cockpit-browser-view"));
  const viewportRef = useRef(viewport);
  viewportRef.current = viewport;
  const liveInputEnabledRef = useRef(liveInputEnabled);
  liveInputEnabledRef.current = liveInputEnabled;
  const associationOwnerRef = useRef<DraftAssociationOwner | null>(null);
  const ownerKey = `${target.session_id}:${target.space_id ?? ""}:${target.pane_id ?? ""}:${target.endpoint_path ?? ""}`;
  if (!associationOwnerRef.current || associationOwnerRef.current.key !== ownerKey) {
    if (associationOwnerRef.current) associationOwnerRef.current.sealed = true;
    associationOwnerRef.current = { key: ownerKey, target: { ...target }, draft: null, localDraftRevision: null, noteId: null, noteText: "", notesOpen: false, editorGeneration: 0, sealed: false, mutationTail: Promise.resolve(), pendingAnnotationMutations: [] };
  }
  const associationOwner = associationOwnerRef.current!;
  const previousAssociationOwnerKeyRef = useRef<string | null>(null);
  const associationChanged = previousAssociationOwnerKeyRef.current !== ownerKey;
  previousAssociationOwnerKeyRef.current = ownerKey;
  const [status, setStatus] = useState<PaneStatus>(visible ? "loading" : "hidden");
  const [message, setMessage] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<BrowserViewSnapshot | null>(null);
  const [frame, setFrame] = useState<{ descriptor: BrowserViewFramePacket["descriptor"]; sequence: number } | null>(null);
  const [pendingCapture, setPendingCapture] = useState<BrowserViewPendingCapture | null>(null);
  const [deliveryState, setDeliveryState] = useState<DeliveryState | null>(null);
  const [deliveryDuplicateRisk, setDeliveryDuplicateRisk] = useState(false);
  const [draft, setDraft] = useState<BrowserViewDraftState | null>(null);
  const [staleDrafts, setStaleDrafts] = useState<BrowserViewDraftState[]>([]);
  const [tool, setTool] = useState<Tool>("browse");
  const [color, setColor] = useState<string>(COLORS[0]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [gesture, setGesture] = useState<Gesture | null>(null);
  const [inspection, setInspection] = useState<BrowserViewInspectResult | null>(null);
  const [notesOpen, setNotesOpen] = useState(false);
  const [noteId, setNoteId] = useState<string | null>(null);
  const [noteValue, setNoteValue] = useState("");
  const [url, setUrl] = useState("");
  const [retry, setRetry] = useState(0);
  const noteIdRef = useRef<string | null>(noteId);
  noteIdRef.current = noteId;
  const noteValueRef = useRef(noteValue);
  noteValueRef.current = noteValue;
  const notesOpenRef = useRef(notesOpen);
  notesOpenRef.current = notesOpen;
  const editorRevisionRef = useRef(0);
  const markEditorDirty = useCallback(() => {
    editorRevisionRef.current += 1;
    associationOwner.editorGeneration = editorRevisionRef.current;
    editorDirtyRef.current = true;
  }, [associationOwner]);
  associationOwner.noteId = noteId;
  associationOwner.noteText = noteValue.slice(0, 4000);
  associationOwner.notesOpen = notesOpen;
  associationOwner.editorGeneration = editorRevisionRef.current;

  const applySnapshot = useCallback((next: BrowserViewSnapshot) => {
    snapshotRef.current = next;
    setSnapshot(next);
    if (!urlEditing.current) setUrl(next.navigation?.url ?? "");
  }, []);
  const applyDraft = useCallback((next: BrowserViewDraftState) => {
    const current = draftRef.current;
    if (current && current.draft_id === next.draft_id && next.revision < current.revision) return;
    draftRef.current = next; associationOwner.draft = next; setDraft(next);
    setSelectedId((selected) => selected && !next.annotations.some((annotation) => annotation.id === selected) ? null : selected);
    const editor = next.editor;
    if (!editorDirtyRef.current) {
      const selectedAnnotation = editor.note_annotation_id ?? editor.selected_annotation_id;
      setSelectedId((selected) => selected ?? selectedAnnotation);
      setNoteId(selectedAnnotation);
      setNoteValue((editor.note_text ?? next.annotations.find((annotation) => annotation.id === selectedAnnotation)?.comment ?? "").slice(0, 4000));
      setNotesOpen(Boolean(editor.notes_open));
    } else {
      const localEditor = { note_annotation_id: noteIdRef.current, note_text: noteValueRef.current.slice(0, 4000) };
      const durableEditor = { note_annotation_id: editor.note_annotation_id ?? editor.selected_annotation_id, note_text: editor.note_text ?? "" };
      if (localEditor.note_annotation_id === durableEditor.note_annotation_id && localEditor.note_text === durableEditor.note_text) editorDirtyRef.current = false;
    }
  }, [associationOwner]);
  const setPendingCaptureState = useCallback((next: BrowserViewPendingCapture | null) => {
    pendingCaptureRef.current = next;
    setPendingCapture(next);
  }, []);
  const clearPresentedFrame = useCallback((clearCanvas = true) => {
    frameRef.current = null;
    setFrame(null);
    if (!clearCanvas) return;
    const canvas = canvasRef.current;
    const drawing = canvas?.getContext("2d");
    if (canvas && drawing) drawing.clearRect(0, 0, canvas.width, canvas.height);
  }, []);
  const invalidateInteractionFrame = useCallback((nextMessage: string) => {
    gestureRef.current = null;
    setGesture(null);
    // Keep the displayed identity and queued key releases. Location-sensitive
    // jobs revalidate against current metadata before dispatch.
    setStatus("stale");
    setMessage(nextMessage);
  }, []);
  const runInputJobs = useCallback((): Promise<void> => {
    if (inputDrainRef.current) return inputDrainRef.current;
    const drain = Promise.resolve().then(async () => {
      try {
        while (inputJobsRef.current.length > 0) {
          const job = inputJobsRef.current.shift()!;
          if (job.generation !== inputGenerationRef.current) continue;
          try { await job.run(); } catch { /* command reports transport errors */ }
        }
      } finally {
        inputDrainRef.current = null;
      }
    });
    inputDrainRef.current = drain;
    return drain;
  }, []);
  const enqueueInput = useCallback((kind: InputJob["kind"], run: () => Promise<void>, wheel?: WheelIntent): Promise<void> => {
    const jobs = inputJobsRef.current;
    const generation = inputGenerationRef.current;
    const next = { generation, kind, run };
    if (kind === "move") {
      const pending = jobs.findIndex((job) => job.kind === "move");
      if (pending >= 0) jobs[pending] = next;
      else if (jobs.length < MAX_INPUT_JOBS) jobs.push(next);
      else {
        setStatus("error");
        setMessage("Browser input queue is full; motion was refused. Release and retry the gesture.");
        return Promise.resolve();
      }
    } else if (kind === "wheel") {
      const last = jobs.at(-1);
      if (wheel && last?.kind === "wheel" && last.generation === generation && last.wheel
        && last.wheel.clientX === wheel.clientX && last.wheel.clientY === wheel.clientY
        && last.wheel.modifiers === wheel.modifiers) {
        last.wheel.deltaX += wheel.deltaX;
        last.wheel.deltaY += wheel.deltaY;
        return runInputJobs();
      }
      if (jobs.length >= MAX_INPUT_JOBS) {
        setStatus("error");
        setMessage("Browser input queue is full; wheel movement was refused. Retry the scroll.");
        return Promise.resolve();
      }
      jobs.push({ ...next, wheel });
    } else if (jobs.length >= MAX_INPUT_JOBS) {
      inputOverloadedRef.current = true;
      inputGenerationRef.current += 1;
      jobs.length = 0;
      remotePointerRef.current = null;
      remotePointerIntentRef.current = null;
      remotePointRef.current = null;
      gestureRef.current = null;
      setGesture(null);
      releaseOverloadedInputRef.current?.();
      setStatus("error");
      setMessage("Browser input queue is full; held input was refused. Control was released; retry the gesture.");
      return Promise.resolve();
    } else {
      jobs.push(next);
    }
    return runInputJobs();
  }, [runInputJobs]);
  const flushInput = runInputJobs;
  const command = useCallback(async (value: BrowserViewCommand): Promise<BrowserViewCommandOutcome | null> => {
    if (!liveInputEnabledRef.current) return null;
    const stream = streamRef.current; const identity = identityRef.current;
    if (!stream || !identity) { setMessage("Browser controls are still connecting."); return null; }
    try {
      if (!liveInputEnabledRef.current) return null;
      const response = await stream.command({ view_id: identity.id, stream_epoch: identity.epoch, request_id: newId("browser-command"), command: value });
      if (!liveInputEnabledRef.current) return null;
      if (identityRef.current !== identity) return null;
      if (response.status !== "accepted") {
        if ("code" in response && response.code === "stale_input_sequence") {
          inputSequence.current = snapshotRef.current?.control.next_input_sequence ?? 1;
          setStatus("stale");
          setMessage(`Input stale: ${response.message || "the input sequence is no longer current"} Retry the gesture; it was not replayed.`);
          return null;
        }
        if (response.status === "stale") {
          setStatus("stale");
          setMessage(`Input stale: ${response.message} Retry the gesture; it was not replayed.`);
        } else if (response.status === "unsupported") {
          setStatus("unsupported");
          setMessage(`Input unsupported: ${response.message}`);
        } else if (response.status === "rejected") {
          errorRef.current = true;
          setStatus("error");
          setMessage(`Input rejected: ${response.message}`);
        } else if (response.status === "outcome_unknown") {
          errorRef.current = true;
          setStatus("error");
          setMessage(`Input outcome unknown: ${response.message} Do not retry this gesture automatically; start a new gesture.`);
        }
        return null;
      }
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
            invalidateInteractionFrame("Showing last confirmed frame while the browser catches up.");
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
      if (identityRef.current !== identity || streamRef.current !== stream) return null;
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
  const openDraft = useCallback(async (): Promise<void> => {
    const current = snapshotRef.current; const documentContext = current ? context(current) : null;
    if (!documentContext || !current?.document || current.document.target_id !== current.displayed_target_id) return;
    const retained = draftRef.current;
    if (retained && (retained.target_id !== documentContext.target_id || retained.document_generation !== documentContext.document_generation)) {
      if (hasRecoverableDraft(retained) || editorDirtyRef.current) {
        setStatus("stale");
        setMessage("Draft marks belong to an earlier page; choose an explicit recovery action.");
        return;
      }
      draftRef.current = null;
      associationOwner.draft = null;
      associationOwner.localDraftRevision = null;
      setDraft(null);
      setSelectedId(null);
      setNoteId(null);
      setNoteValue("");
    }
    const request = ++draftRequestRef.current;
    const listed = await command({ type: "draft", context: documentContext, draft_id: null, expected_revision: null, command: { type: "list" } });
    if (request !== draftRequestRef.current || !listed || listed.type !== "draft_inventory") return;
    const latest = snapshotRef.current;
    if (!latest || JSON.stringify(context(latest)) !== JSON.stringify(documentContext)) { setPendingCaptureState(null); return; }
    const currentDraft = listed.inventory.drafts.find((candidate) => candidate.target_id === documentContext.target_id && candidate.document_generation === documentContext.document_generation);
    setStaleDrafts(listed.inventory.drafts.filter((candidate) => candidate.draft_id !== currentDraft?.draft_id && hasRecoverableDraft(candidate)));
    // Pending captures belong to the association, including earlier documents.
    setPendingCaptureState(listed.inventory.pending_capture);
    const draftId = currentDraft?.draft_id ?? null;
    await command({ type: "draft", context: documentContext, draft_id: draftId, expected_revision: null, command: { type: "open", draft_id: draftId } });
  }, [associationOwner, command, setPendingCaptureState]);
  const startCurrentDraft = useCallback((): void => {
    if (editorDirtyRef.current) {
      setMessage("The current draft editor still has unsaved text; close or save it before starting a new draft.");
      return;
    }
    void associationOwner.mutationTail.then(() => {
      draftRequestRef.current += 1;
      draftRef.current = null; associationOwner.draft = null; associationOwner.localDraftRevision = null;
      setDraft(null);
      setSelectedId(null);
      setNoteId(null);
      setNoteValue("");
      void openDraft();
    }).catch((error) => setMessage(`Could not preserve the earlier draft: ${errorMessage(error)}`));
  }, [associationOwner, openDraft]);
  const discardStaleDraft = useCallback(async (): Promise<void> => {
    const stale = draftRef.current;
    if (!stale) return;
    try {
      const response = await client.browserDraftRecovery({ target, action: { type: "discard_draft", draft_id: stale.draft_id, expected_revision: stale.revision } });
      if (response.type !== "draft_inventory" && response.type !== "none") throw new Error("The stale draft discard was not acknowledged.");
      startCurrentDraft();
    } catch (error) {
      setStatus("error");
      setMessage(`Could not discard stale draft: ${errorMessage(error)}`);
    }
  }, [client, startCurrentDraft, target]);
  const discardInventoryDraft = useCallback(async (stale: BrowserViewDraftState): Promise<void> => {
    try {
      const response = await client.browserDraftRecovery({ target, action: { type: "discard_draft", draft_id: stale.draft_id, expected_revision: stale.revision } });
      if (response.type !== "draft_inventory" && response.type !== "none") throw new Error("The stale draft discard was not acknowledged.");
      setStaleDrafts((current) => current.filter((candidate) => candidate.draft_id !== stale.draft_id));
    } catch (error) {
      setStatus("error");
      setMessage(`Could not discard stale draft: ${errorMessage(error)}`);
    }
  }, [client, target]);
  const reviewInventoryDraft = useCallback((stale: BrowserViewDraftState): void => {
    draftRef.current = stale; associationOwner.draft = stale; associationOwner.localDraftRevision = null;
    setDraft(stale);
    setSelectedId(null);
    setNoteId((stale.editor.note_annotation_id ?? stale.editor.selected_annotation_id));
    setNoteValue(stale.editor.note_text.slice(0, 4000));
    setStatus("stale");
    setMessage("Reviewing a retained draft from an earlier page.");
  }, [associationOwner]);
  const queueDraftMutation = useCallback(<T,>(run: () => Promise<T>): Promise<T> => {
    const next = associationOwner.mutationTail.catch(() => undefined).then(run);
    associationOwner.mutationTail = next.then(() => undefined, () => undefined);
    return next;
  }, [associationOwner]);
  const persistEditor = useCallback(async (requestedGeneration = associationOwner.editorGeneration): Promise<void> => {
    const owner = associationOwner;
    let savedNoteId: string | null = null;
    let savedNoteText = "";
    let savedNotesOpen = false;
    const result = await queueDraftMutation(async () => {
      const currentDraft = owner.draft;
      if (!currentDraft) throw new Error("The browser draft association changed before editor save completed.");
      savedNoteId = owner.noteId;
      savedNoteText = owner.noteText.slice(0, 4000);
      savedNotesOpen = owner.notesOpen;
      const editor: BrowserViewDraftState["editor"] = {
        selected_annotation_id: savedNoteId,
        notes_open: savedNotesOpen,
        note_annotation_id: savedNoteId,
        note_text: savedNoteText,
      };
      const action: BrowserDraftRecoveryAction = {
        type: "set_editor",
        draft_id: currentDraft.draft_id,
        expected_revision: currentDraft.revision,
        editor,
      };
      const response = await client.browserDraftRecovery({ target: owner.target, action });
      if (response.type === "draft") {
        owner.draft = response.draft;
        owner.localDraftRevision = response.draft.revision;
        if (associationOwnerRef.current === owner && !owner.sealed) applyDraft(response.draft);
      }
      return response;
    });
    if (result.type !== "draft" || !owner.draft || result.draft.draft_id !== owner.draft.draft_id) throw new Error("The browser draft editor save was not acknowledged; retry Close.");
    owner.draft = result.draft;
    owner.localDraftRevision = result.draft.revision;
    const acknowledged = result.draft.editor;
    if ((acknowledged.note_annotation_id ?? acknowledged.selected_annotation_id) !== savedNoteId || (acknowledged.note_text ?? "") !== savedNoteText || Boolean(acknowledged.notes_open) !== savedNotesOpen) throw new Error("The browser draft editor save is still pending; retry Close.");
    if (owner.editorGeneration === requestedGeneration && owner.noteId === savedNoteId && owner.noteText.slice(0, 4000) === savedNoteText && owner.notesOpen === savedNotesOpen) {
      editorDirtyRef.current = false;
    } else {
      editorDirtyRef.current = true;
    }
  }, [applyDraft, associationOwner, client, queueDraftMutation]);
  const closeGuard = useCallback(async (): Promise<void> => {
    await associationOwner.mutationTail;
    if (editorDirtyRef.current) {
      editorPersistRef.current = persistEditor();
      try { await editorPersistRef.current; } finally { editorPersistRef.current = null; }
      if (editorDirtyRef.current) throw new Error("A newer browser editor change is still saving; retry Close.");
    }
    if (associationOwner.pendingAnnotationMutations.length > 0) throw new Error("Annotation changes are not acknowledged; retry the annotation action before closing.");
    if (pendingDeliveryIdsRef.current) throw new Error("Annotation delivery is pending; retry Send annotations before closing.");
  }, [persistEditor, associationOwner]);
  const [editorTick, setEditorTick] = useState(0);
  useEffect(() => {
    if (!editorDirtyRef.current || !draftRef.current) return;
    const generation = editorRevisionRef.current;
    const timer = window.setTimeout(() => {
      if (editorPersistRef.current) return;
      editorPersistRef.current = persistEditor(generation).catch(() => undefined).finally(() => {
        editorPersistRef.current = null;
        if (editorDirtyRef.current && editorRevisionRef.current !== generation) setEditorTick((tick) => tick + 1);
      });
    }, 120);
    return () => window.clearTimeout(timer);
  }, [editorTick, noteId, noteValue, notesOpen, persistEditor]);
  useEffect(() => {
    let closed = false;
    let presenter: FramePresenter | null = null;
    let stream: BrowserViewStream | null = null;
    let cursor: number | null = null;
    const controller = new AbortController();
    const close = () => {
      if (closed) return;
      closed = true;
      if (editorDirtyRef.current && draftRef.current) void persistEditor().catch(() => undefined);
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
    pendingDeliveryIdentityRef.current = associationChanged ? null : pendingDeliveryIdentityRef.current;
    pendingDeliveryOperationRef.current = associationChanged ? null : pendingDeliveryOperationRef.current;
    pendingCaptureRef.current = associationChanged ? null : pendingCaptureRef.current;
    draftRequestRef.current += 1;
    inputJobsRef.current = [];
    inputOverloadedRef.current = false;
    setFrame(null);
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
      setDeliveryState(null);
      setDeliveryDuplicateRisk(false);
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
    const event = (incoming: BrowserViewEvent): void => {
      if (closed) return;
      if (incoming.type === "attached") {
        if (identityRef.current) return;
        identityRef.current = { id: incoming.metadata.view_id, epoch: incoming.metadata.stream_epoch }; cursor = incoming.metadata.metadata_sequence; inputSequence.current = incoming.snapshot.control.next_input_sequence; applySnapshot(incoming.snapshot); setStatus(statusFor(incoming.snapshot)); queueMicrotask(openDraft); return;
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
            ++inputGenerationRef.current; clearPresentedFrame(); inputJobsRef.current = []; remotePointerRef.current = null; remotePointerIntentRef.current = null; remotePointRef.current = null; gestureRef.current = null; setGesture(null); draftRequestRef.current += 1; setSelectedId(null); setInspection(null);
          }
          break;
        }
        case "document_changed": {
          next = { ...previous, document: incoming.document };
          ++inputGenerationRef.current; clearPresentedFrame(); inputJobsRef.current = []; remotePointerRef.current = null; remotePointerIntentRef.current = null; remotePointRef.current = null; gestureRef.current = null; setGesture(null); draftRequestRef.current += 1; setSelectedId(null); setInspection(null);
          setStatus("stale"); setMessage("The page changed; earlier draft marks remain available for explicit recovery.");
          break;
        }
        case "navigation_changed": {
          next = { ...previous, navigation: incoming.navigation };
          if (incoming.navigation?.loading) invalidateInteractionFrame("Showing last confirmed frame while navigation completes.");
          break;
        }
        case "viewport_changed": {
          next = { ...previous, viewport: incoming.viewport };
          if (previous.viewport?.viewport_revision !== incoming.viewport?.viewport_revision) {
            // Scrolling is a normal repaint, not a stream failure. Coordinate
            // input waits for geometry matching the recovered frame.
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
        case "frame_transport_revoked": setStatus("stale"); setMessage(incoming.message); return;
        case "failed": setStatus("error"); setMessage(incoming.message); return;
        case "closed": setStatus("error"); setMessage(incoming.reason); return;
      }
      applySnapshot(next);
      presenter?.revalidate();
      if (incoming.type === "document_changed") queueMicrotask(() => void openDraft());
      if (incoming.type !== "viewport_changed" && incoming.type !== "document_changed" && incoming.type !== "navigation_changed") {
        const nextStatus = statusFor(next);
        if (nextStatus !== "loading" || !frameRef.current) setStatus(nextStatus);
      }
    };
    presenter = new FramePresenter({
      isExpectedStale: (descriptor) => {
        const current = snapshotRef.current; const identity = identityRef.current;
        if (!current?.document || !current.viewport || !identity) return false;
        return descriptor.stream_epoch !== identity.epoch
          || descriptor.target_id !== current.displayed_target_id
          || descriptor.document_generation !== current.document.document_generation
          || descriptor.viewport_revision !== current.viewport.viewport_revision
          || Math.abs(descriptor.viewport_css_width - current.viewport.css_width) > 0.01
          || Math.abs(descriptor.viewport_css_height - current.viewport.css_height) > 0.01
          || Math.abs(descriptor.scroll_x - current.viewport.scroll_x) > 0.01
          || Math.abs(descriptor.scroll_y - current.viewport.scroll_y) > 0.01;
      },
      shouldDefer: (descriptor) => {
        const current = snapshotRef.current; const identity = identityRef.current; const presented = frameRef.current;
        if (!current?.document || !current.viewport || !identity || descriptor.stream_epoch !== identity.epoch) return false;
        if (presented && descriptor.frame_sequence <= presented.sequence) return false;
        const knownTarget = descriptor.target_id === current.displayed_target_id || current.targets.some((candidate) => candidate.target_id === descriptor.target_id);
        return knownTarget && (descriptor.document_generation > current.document.document_generation
          || (descriptor.document_generation === current.document.document_generation
            && descriptor.viewport_revision > current.viewport.viewport_revision));
      },
      validate: (descriptor) => {
        const current = snapshotRef.current; const identity = identityRef.current;
        if (!current?.document || !current.viewport || !current.displayed_target_id || !identity) throw new BrowserFrameError("identity_mismatch", "Browser frame arrived before its snapshot");
        validateFrameDescriptor(descriptor, { streamEpoch: identity.epoch, targetId: current.displayed_target_id, displayedTargetId: current.displayed_target_id, documentGeneration: current.document.document_generation, viewportRevision: current.viewport.viewport_revision, viewportCssWidth: current.viewport.css_width, viewportCssHeight: current.viewport.css_height, scrollX: current.viewport.scroll_x, scrollY: current.viewport.scroll_y });
      },
      present: (image, descriptor) => {
        if (!frameMatchesCurrent({ descriptor, sequence: descriptor.frame_sequence })) throw new BrowserFrameError("identity_mismatch", "Browser frame became stale while decoding");
        const targetCanvas = canvasRef.current; const drawing = targetCanvas?.getContext("2d");
        if (!targetCanvas || !drawing) throw new Error("Browser view canvas is unavailable");
        targetCanvas.width = descriptor.image_width; targetCanvas.height = descriptor.image_height; drawing.clearRect(0, 0, targetCanvas.width, targetCanvas.height); drawing.drawImage(image, 0, 0, descriptor.image_width, descriptor.image_height);
        const accepted = { descriptor, sequence: descriptor.frame_sequence }; frameRef.current = accepted; setFrame(accepted); if (!errorRef.current) setStatus("ready");
      },
      onError: (error) => { if (!closed) { errorRef.current = true; setStatus("error"); setMessage(errorMessage(error)); } },
    });
    presenterRef.current = presenter;
    const request: BrowserViewOpenRequest = { target, client_id: clientId ?? clientRef.current, presentation, viewport: paneViewport(), takeover: false };
    void client.openBrowserView(request, event, (packet) => presenter?.push(packet), (error) => { if (!closed) { setStatus("error"); setMessage(errorMessage(error)); } }, controller.signal).then((opened) => {
      stream = opened;
      if (closed) opened.close();
      else { streamRef.current = opened; void openDraft(); }
    }).catch((error: unknown) => { if (!closed && !controller.signal.aborted) { setStatus("error"); setMessage(errorMessage(error)); } });
    return close;
  // Browser-only is local layout state; it must not revoke the live frame stream.
  }, [applySnapshot, associationChanged, associationOwner, clearPresentedFrame, client, clientId, frameMatchesCurrent, invalidateInteractionFrame, openDraft, paneViewport, persistEditor, retry, target.endpoint_path, target.pane_id, target.session_id, target.space_id, visible]);
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
  useEffect(() => {
    let timer = 0;
    const scheduleResize = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const current = snapshotRef.current;
        const requested = paneViewport();
        if (!visible || !current?.viewport || !ownsBrowserControl(current)
          || (current.viewport.css_width === requested.css_width
            && current.viewport.css_height === requested.css_height
            && current.viewport.device_pixel_ratio === requested.device_pixel_ratio)) return;
        void enqueueInput("boundary", async () => {
          const latest = snapshotRef.current;
          const latestRequested = paneViewport();
          const documentContext = latest ? context(latest) : null;
          if (!latest || !latest.viewport || !ownsBrowserControl(latest) || !documentContext
            || (latest.viewport.css_width === latestRequested.css_width
              && latest.viewport.css_height === latestRequested.css_height
              && latest.viewport.device_pixel_ratio === latestRequested.device_pixel_ratio)) return;
          await command({ type: "resize", context: documentContext, viewport: latestRequested });
          // Metadata invalidates old geometry; a response must not erase a newer frame.
        });
      }, 120);
    };
    scheduleResize();
    window.addEventListener("resize", scheduleResize);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("resize", scheduleResize);
    };
  }, [command, enqueueInput, paneViewport, snapshot?.control.status, snapshot?.control.controller_view_id, snapshot?.viewport?.viewport_revision, viewport, visible]);

  const paintedRectFor = useCallback((allowScrollTransition = false) => {
    const current = frameRef.current; const surface = surfaceRef.current;
    if (!current || !surface || !(allowScrollTransition ? frameSupportsViewportInput(current) : frameMatchesCurrent(current))) return null;
    const bounds = surface.getBoundingClientRect(); const aspect = current.descriptor.image_width / current.descriptor.image_height;
    const width = Math.min(bounds.width, bounds.height * aspect); const height = width / aspect;
    return { left: bounds.left + (bounds.width - width) / 2, top: bounds.top + (bounds.height - height) / 2, width, height };
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
  const nextInput = (): number => inputSequence.current++;
  const persist = async (annotation: BrowserViewDraftAnnotation): Promise<boolean> => {
    const owner = associationOwner;
    const editorGenerationAtIntent = owner.editorGeneration;
    const noteIdAtIntent = owner.noteId;
    const noteTextAtIntent = owner.noteText.slice(0, 4000);
    const notesOpenAtIntent = owner.notesOpen;
    const current = snapshotRef.current; const documentContext = current ? context(current) : null;
    const draftAtIntent = owner.draft;
    if (!documentContext || !draftAtIntent) { setMessage("The browser draft is still recovering."); return false; }
    const mutationKey = newId("annotation-intent");
    owner.pendingAnnotationMutations.push({ key: mutationKey, kind: "upsert", draftId: draftAtIntent.draft_id, expectedRevision: draftAtIntent.revision, annotation: { ...annotation, points: annotation.points.map((point) => ({ ...point })), bounds: annotation.bounds ? { ...annotation.bounds } : null }, annotationId: annotation.id });
    try {
      let expectedRevision = draftAtIntent.revision;
      const expectedDraft = () => {
        const candidate = owner.draft;
        if (!candidate || candidate.draft_id !== draftAtIntent.draft_id) throw new Error("The browser draft association changed before annotation save completed.");
        if (candidate.revision !== draftAtIntent.revision && owner.localDraftRevision !== candidate.revision) {
          throw new Error("The browser draft changed externally before annotation save completed.");
        }
        expectedRevision = candidate.revision;
        return candidate;
      };
      let acknowledgedResult = await queueDraftMutation(async () => {
        expectedDraft();
        return command({ type: "draft", context: documentContext, draft_id: draftAtIntent.draft_id, expected_revision: expectedRevision, command: { type: "upsert_annotation", annotation } });
      });
      if (acknowledgedResult?.type === "draft") {
        owner.draft = acknowledgedResult.draft;
        owner.localDraftRevision = acknowledgedResult.draft.revision;
      }
      let latest = snapshotRef.current; let sameContext = Boolean(latest && JSON.stringify(context(latest)) === JSON.stringify(documentContext));
      const mutation = owner.pendingAnnotationMutations.find((candidate) => candidate.key === mutationKey);
      if (mutation) mutation.expectedRevision = expectedRevision;
      let accepted = sameContext && acknowledgedResult?.type === "draft" && acknowledgedResult.draft.draft_id === draftAtIntent.draft_id && acknowledgedResult.draft.annotations.some((candidate: BrowserViewDraftAnnotation) => candidate.id === annotation.id && JSON.stringify(candidate) === JSON.stringify(annotation));
      if (!accepted) {
        const recovery = await queueDraftMutation(() => client.browserDraftRecovery({ target: owner.target, action: { type: "upsert_annotation", draft_id: draftAtIntent.draft_id, expected_revision: expectedRevision, annotation } }));
        acknowledgedResult = recovery.type === "draft" ? recovery : null;
        if (acknowledgedResult?.type === "draft") {
          owner.draft = acknowledgedResult.draft;
          owner.localDraftRevision = acknowledgedResult.draft.revision;
          if (associationOwnerRef.current === owner && !owner.sealed) applyDraft(acknowledgedResult.draft);
        }
        accepted = Boolean(acknowledgedResult?.type === "draft" && acknowledgedResult.draft.draft_id === draftAtIntent.draft_id && acknowledgedResult.draft.annotations.some((candidate: BrowserViewDraftAnnotation) => candidate.id === annotation.id && JSON.stringify(candidate) === JSON.stringify(annotation)));
      }
      latest = snapshotRef.current; sameContext = Boolean(latest && JSON.stringify(context(latest)) === JSON.stringify(documentContext));
      if (accepted) {
        owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((mutation) => mutation.key !== mutationKey);
        if (associationOwnerRef.current === owner && !owner.sealed) {
          const editorUnchanged = owner.editorGeneration === editorGenerationAtIntent
            && owner.noteId === noteIdAtIntent
            && owner.noteText.slice(0, 4000) === noteTextAtIntent
            && owner.notesOpen === notesOpenAtIntent;
          if (editorUnchanged) {
            setSelectedId(annotation.id);
            setNoteId(annotation.id);
            setNoteValue((annotation.comment ?? "").slice(0, 4000));
            editorDirtyRef.current = false;
          } else {
            setMessage("Annotation saved; newer editor work was retained.");
          }
        }
      } else setMessage("Annotation save was not acknowledged; retry it before closing.");
      return accepted;
    } catch (error) {
      setMessage(`Annotation save failed; retry it before closing: ${errorMessage(error)}`);
      return false;
    }
  };
  const removeAnnotation = (annotationId: string): void => {
    const owner = associationOwner;
    const current = snapshotRef.current; const documentContext = current ? context(current) : null;
    const draftAtIntent = owner.draft;
    if (!documentContext || !draftAtIntent) { setMessage("The browser draft is still recovering."); return; }
    const mutationKey = newId("annotation-intent");
    const previousAnnotation = draftAtIntent.annotations.find((annotation) => annotation.id === annotationId) ?? null;
    owner.pendingAnnotationMutations.push({ key: mutationKey, kind: "remove", draftId: draftAtIntent.draft_id, expectedRevision: draftAtIntent.revision, annotation: previousAnnotation ? { ...previousAnnotation, points: previousAnnotation.points.map((point) => ({ ...point })), bounds: previousAnnotation.bounds ? { ...previousAnnotation.bounds } : null } : null, annotationId });
    void (async () => {
      let expectedRevision = draftAtIntent.revision;
      const expectedDraft = () => {
        const candidate = owner.draft;
        if (!candidate || candidate.draft_id !== draftAtIntent.draft_id) throw new Error("The browser draft association changed before annotation removal completed.");
        if (candidate.revision !== draftAtIntent.revision && owner.localDraftRevision !== candidate.revision) {
          throw new Error("The browser draft changed externally before annotation removal completed.");
        }
        expectedRevision = candidate.revision;
      };
      let acknowledgedResult = await queueDraftMutation(async () => {
        expectedDraft();
        return command({ type: "draft", context: documentContext, draft_id: draftAtIntent.draft_id, expected_revision: expectedRevision, command: { type: "remove_annotation", annotation_id: annotationId } });
      });
      if (acknowledgedResult?.type === "draft") {
        owner.draft = acknowledgedResult.draft;
        owner.localDraftRevision = acknowledgedResult.draft.revision;
      }
      const mutation = owner.pendingAnnotationMutations.find((candidate) => candidate.key === mutationKey);
      if (mutation) mutation.expectedRevision = expectedRevision;
      const latest = snapshotRef.current;
      let acknowledged = Boolean(latest && JSON.stringify(context(latest)) === JSON.stringify(documentContext) && acknowledgedResult?.type === "draft" && acknowledgedResult.draft.draft_id === draftAtIntent.draft_id && !acknowledgedResult.draft.annotations.some((candidate: BrowserViewDraftAnnotation) => candidate.id === annotationId));
      if (!acknowledged) {
        const recovery = await queueDraftMutation(() => client.browserDraftRecovery({ target: owner.target, action: { type: "remove_annotation", draft_id: draftAtIntent.draft_id, expected_revision: expectedRevision, annotation_id: annotationId } }));
        acknowledgedResult = recovery.type === "draft" ? recovery : null;
        if (acknowledgedResult?.type === "draft") {
          owner.draft = acknowledgedResult.draft;
          owner.localDraftRevision = acknowledgedResult.draft.revision;
          if (associationOwnerRef.current === owner && !owner.sealed) applyDraft(acknowledgedResult.draft);
        }
        acknowledged = Boolean(acknowledgedResult?.type === "draft" && acknowledgedResult.draft.draft_id === draftAtIntent.draft_id && !acknowledgedResult.draft.annotations.some((candidate: BrowserViewDraftAnnotation) => candidate.id === annotationId));
      }
      if (acknowledged) {
        owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((mutation) => mutation.key !== mutationKey);
      }
      else setMessage("Annotation removal was not acknowledged; retry it before closing.");
    })().catch((error) => setMessage(`Annotation removal failed; retry it before closing: ${errorMessage(error)}`));
    setSelectedId((selected) => selected === annotationId ? null : selected);
  };
  const retryAnnotationMutations = useCallback(async (): Promise<void> => {
    const owner = associationOwner;
    for (const mutation of [...owner.pendingAnnotationMutations]) {
      try {
        const resolution = await queueDraftMutation(async () => {
          const inventory = await client.browserDraftRecovery({ target: owner.target, action: { type: "list" } });
          if (inventory.type !== "draft_inventory") throw new Error("The retained annotation inventory could not be read.");
          const current = inventory.inventory.drafts.find((candidate) => candidate.draft_id === mutation.draftId);
          if (!current) throw new Error("The retained annotation draft no longer exists.");
          const alreadyApplied = mutation.kind === "upsert"
            ? current.annotations.some((candidate) => candidate.id === mutation.annotationId && JSON.stringify(candidate) === JSON.stringify(mutation.annotation))
            : !current.annotations.some((candidate) => candidate.id === mutation.annotationId);
          let expectedRevision = mutation.expectedRevision;
          if (current.revision !== expectedRevision) {
            if (alreadyApplied) return { draft: current, acknowledged: true, local: false };
            if (owner.localDraftRevision === current.revision && current.revision > expectedRevision) {
              expectedRevision = current.revision;
              mutation.expectedRevision = expectedRevision;
            } else {
              throw new Error("The retained annotation action conflicts with a newer draft revision.");
            }
          }
          const action: BrowserDraftRecoveryAction = mutation.kind === "upsert"
            ? { type: "upsert_annotation", draft_id: mutation.draftId, expected_revision: expectedRevision, annotation: mutation.annotation! }
            : { type: "remove_annotation", draft_id: mutation.draftId, expected_revision: expectedRevision, annotation_id: mutation.annotationId };
          const response = await client.browserDraftRecovery({ target: owner.target, action });
          const nextDraft = response.type === "draft" ? response.draft : null;
          const acknowledged = Boolean(nextDraft && (mutation.kind === "upsert"
            ? nextDraft.annotations.some((candidate) => candidate.id === mutation.annotationId && JSON.stringify(candidate) === JSON.stringify(mutation.annotation))
            : !nextDraft.annotations.some((candidate) => candidate.id === mutation.annotationId)));
          return { draft: nextDraft, acknowledged, local: true };
        });
        if (!resolution.draft || !resolution.acknowledged) throw new Error("The retained annotation action was not acknowledged.");
        owner.draft = resolution.draft;
        if (resolution.local) owner.localDraftRevision = resolution.draft.revision;
        owner.pendingAnnotationMutations = owner.pendingAnnotationMutations.filter((candidate) => candidate.key !== mutation.key);
        if (associationOwnerRef.current === owner && !owner.sealed) applyDraft(resolution.draft);
      } catch (error) {
        setMessage(`Could not reconcile retained annotation work: ${errorMessage(error)}`);
        return;
      }
    }
    setMessage("Retained annotation work was acknowledged.");
  }, [applyDraft, associationOwner, client, queueDraftMutation]);
  const discardAnnotationMutations = useCallback(async (): Promise<void> => {
    const owner = associationOwner;
    const discarded = owner.pendingAnnotationMutations.length;
    const hadUnknownDelivery = Boolean(pendingDeliveryIdsRef.current);
    owner.pendingAnnotationMutations = [];
    if (hadUnknownDelivery) {
      pendingDeliveryIdsRef.current = null;
      pendingDeliveryIdentityRef.current = null;
      pendingDeliveryOperationRef.current = null;
      setDeliveryState(null);
      setDeliveryDuplicateRisk(false);
    }
    if (discarded === 0 && !hadUnknownDelivery) return;
    setMessage(hadUnknownDelivery
      ? "Discarded local retry intents. Feedback may already exist remotely; no undo or duplicate send was attempted."
      : "Discarded local annotation retry intents. A prior unknown write may still exist remotely; no undo was attempted.");
  }, [associationOwner]);
  const inspect = async (point: BrowserPoint): Promise<BrowserViewInspectResult | null> => {
    const current = snapshotRef.current;
    const accepted = frameRef.current;
    const inspected = current && accepted && frameMatchesCurrent(accepted) ? location(current, accepted.descriptor) : null;
    const request = ++inspectRequestRef.current;
    if (!current?.document || !current.viewport || !accepted || !inspected) {
      setMessage("Element inspection is waiting for a confirmed browser frame.");
      return null;
    }
    const pointerSampleSequence = current.cursor
      && current.cursor.target_id === inspected.target_id
      && current.cursor.document_generation === inspected.document_generation
      && current.cursor.viewport_revision === inspected.viewport_revision
      ? current.cursor.pointer_sample_sequence
      : null;
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
    const outcome = await command({ type: "inspect", command: { location: intent.location, pointer_sample_sequence: intent.pointerSampleSequence, x: intent.point.x, y: intent.point.y } });
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
  const remotePointer = (event: PointerEvent<HTMLDivElement>, kind: "move" | "down" | "up" | "cancel"): void => {
    if (!liveInputEnabledRef.current) return;
    const active = remotePointerRef.current === event.pointerId;
    const point = viewportPointFor(event, active);
    const current = snapshotRef.current;
    const accepted = frameRef.current;
    const where = current && accepted && frameMatchesCurrent(accepted) ? location(current, accepted.descriptor) : null;
    if (!point || !accepted || !where) {
      setStatus("stale");
      setMessage("Input cancelled before dispatch; the browser frame is not ready for this gesture.");
      return;
    }
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
        setStatus("stale");
        setMessage("Input stale: the original browser geometry changed; retry the gesture.");
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
  const releaseRemotePointer = useCallback((): Promise<void> => {
    const intent = remotePointerIntentRef.current;
    const current = snapshotRef.current;
    remotePointerRef.current = null;
    remotePointerIntentRef.current = null;
    remotePointRef.current = null;
    if (!current || !intent || !ownsBrowserControl(current)) return flushInput();
    const where = locationForInputIntent(intent);
    if (!where) return flushInput();
    return enqueueInput("boundary", async () => {
      const input_sequence = inputSequence.current++;
      await command({ type: "pointer", location: where, input: { kind: "cancel", button: null, x: intent.point.x, y: intent.point.y, buttons: 0, modifiers: 0, click_count: 0, input_sequence } });
    });
  }, [command, enqueueInput, flushInput, locationForInputIntent]);
  const previousToolRef = useRef<Tool>(tool);
  useEffect(() => {
    if (previousToolRef.current !== tool) {
      previousToolRef.current = tool;
      gestureRef.current = null;
      setGesture(null);
      void releaseRemotePointer();
    }
  }, [releaseRemotePointer, tool]);
  useEffect(() => {
    const release = () => {
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
      const current = snapshotRef.current;
      if (current?.document && current.viewport && current.displayed_target_id) {
        const accepted = frameRef.current;
        if (!accepted || !frameMatchesCurrent(accepted)) {
          pendingElementClientPointRef.current = { clientX: event.clientX, clientY: event.clientY, targetId: current.displayed_target_id, documentGeneration: current.document.document_generation, viewportRevision: current.viewport.viewport_revision };
          setMessage("Element inspection is waiting for a confirmed browser frame.");
        } else {
          const point = viewportPointFor(event);
          if (point) void inspect(point);
        }
      } else setMessage("Element inspection is unavailable until a page and frame are ready.");
      return;
    }
    if (!documentPoint) return;
    if (tool === "select") return;
    const current = snapshotRef.current; const accepted = frameRef.current;
    if (!current?.document || !current.viewport || !accepted) return;
    const active = { pointerId: event.pointerId, points: [documentPoint], origin: documentPoint, frame: accepted.sequence, document: current.document.document_generation, viewport: current.viewport.viewport_revision };
    gestureRef.current = active; setGesture(active); event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>): void => {
    if (isLocalBrowserChrome(event.target)) return;
    if (tool === "browse") { remotePointer(event, "move"); return; }
    const active = gestureRef.current;
    if (active?.pointerId === event.pointerId) { const point = pointFor(event); if (point) { const next = { ...active, points: [...active.points, point] }; gestureRef.current = next; setGesture(next); } }
    else if (tool === "element") { const point = viewportPointFor(event); if (point) void inspect(point); }
  };
  const finishGesture = (event: PointerEvent<HTMLDivElement>, cancelled: boolean): void => {
    const active = gestureRef.current; gestureRef.current = null; setGesture(null); if (!active || cancelled) return;
    const current = snapshotRef.current; const accepted = frameRef.current; const last = pointFor(event) ?? active.points[active.points.length - 1];
    if (!current?.document || !current.viewport || !accepted || current.document.document_generation !== active.document || current.viewport.viewport_revision !== active.viewport) { setMessage("The page geometry changed while drawing; the unfinished mark was cancelled."); return; }
    if (tool === "freehand") { const points = simplify([...active.points, last]); if (points.length > 1) persist({ id: annotationId(), kind: "freehand", color, points, bounds: null, evidence: null, comment: null }); }
    if (tool === "region") { const bounds = rectFrom(active.origin, last); if (bounds.width >= 1 || bounds.height >= 1) persist({ id: annotationId(), kind: "region", color, points: [active.origin, last], bounds, evidence: null, comment: null }); }
  };
  const onPointerUp = (event: PointerEvent<HTMLDivElement>): void => {
    if (isLocalBrowserChrome(event.target)) return;
    if (tool === "browse") { remotePointer(event, "up"); if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); return; }
    if (tool === "element") {
      const current = snapshotRef.current;
      const accepted = frameRef.current;
      if (!current?.document || !current.viewport || !current.displayed_target_id) {
        setMessage("Element inspection is unavailable until a page and frame are ready.");
        return;
      }
      if (!accepted || !frameMatchesCurrent(accepted)) {
        pendingElementClientPointRef.current = { clientX: event.clientX, clientY: event.clientY, targetId: current.displayed_target_id, documentGeneration: current.document.document_generation, viewportRevision: current.viewport.viewport_revision };
        setMessage("Element inspection is waiting for a confirmed browser frame.");
        return;
      }
      const point = viewportPointFor(event);
      if (!point) return;
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
        } else if (result) setMessage(result.limitation ?? "The selected element is not fresh or accessible; select it again.");
      });
      return;
    }
    finishGesture(event, false);
  };
  const onPointerCancel = (event: PointerEvent<HTMLDivElement>): void => {
    if (isLocalBrowserChrome(event.target)) return;
    if (tool === "browse") { remotePointer(event, "cancel"); if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); return; }
    finishGesture(event, true);
  };
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
      const inputLocation = latest && accepted && ownsBrowserControl(latest) && point
        ? location(latest, accepted.descriptor)
        : null;
      if (!point || !inputLocation || generation !== inputGenerationRef.current) {
        setStatus("stale");
        setMessage("Scroll is waiting for the resized browser frame. Retry once it is ready.");
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
  const navigation = (action: "back" | "forward" | "reload" | "stop" | "navigate", address?: string): void => {
    if (!liveInputEnabledRef.current) return;
    const value = action === "navigate" ? { type: "navigate" as const, url: navigationUrl(address ?? "") } : { type: action };
    const attempt = ++navigationRequestRef.current;
    if (value.type === "navigate") setUrl(snapshotRef.current?.navigation?.url ?? "");
    urlEditing.current = false;
    draftRequestRef.current += 1;
    setSelectedId(null);
    setInspection(null);
    gestureRef.current = null;
    setGesture(null);
    setMessage("Navigation started; existing draft marks remain available for review.");
    onInteractionFocus?.(); void releaseRemotePointer();
    void enqueueInput("boundary", async () => {
      const controlled = await ensureControl(); const current = controlled ? snapshotRef.current : null; const documentContext = current ? context(current) : null;
      if (!documentContext) return;
      const outcome = await command({ type: "navigation", context: documentContext, command: value });
      if (!outcome && navigationRequestRef.current === attempt) setUrl(snapshotRef.current?.navigation?.url ?? "");
    });
  };
  const saveNote = (): void => {
    const editingNoteId = noteId;
    const annotation = draftRef.current?.annotations.find((candidate) => candidate.id === editingNoteId);
    if (!annotation) return;
    void persist({ ...annotation, comment: noteValue.trim() || null }).then((accepted) => {
      if (accepted && noteId === editingNoteId) { setNoteId(null); setNoteValue(""); markEditorDirty(); }
    });
  };
  const pngBase64 = async (annotationsToPaint: BrowserViewDraftAnnotation[], pinnedDescriptor: BrowserViewFramePacket["descriptor"]): Promise<string | null> => {
    const source = canvasRef.current;
    if (!source || !snapshotRef.current?.document || !snapshotRef.current.viewport) return null;
    const output = document.createElement("canvas"); output.width = source.width; output.height = source.height;
    const drawing = output.getContext("2d"); if (!drawing) return null;
    drawing.drawImage(source, 0, 0);
    const map = (point: BrowserPoint): BrowserPoint => ({
      x: (point.x - pinnedDescriptor.scroll_x - pinnedDescriptor.viewport_offset_x) / pinnedDescriptor.viewport_css_width * output.width,
      y: (point.y - pinnedDescriptor.scroll_y - pinnedDescriptor.viewport_offset_y) / pinnedDescriptor.viewport_css_height * output.height,
    });
    for (const annotation of annotationsToPaint) {
      drawing.strokeStyle = annotation.color; drawing.fillStyle = annotation.color; drawing.lineWidth = kindFor(annotation) === "freehand" ? 3 : 2;
      drawing.lineCap = "round"; drawing.lineJoin = "round";
      if (kindFor(annotation) === "freehand") {
        const points = annotation.points.map(map); if (points.length < 2) continue;
        drawing.beginPath(); drawing.moveTo(points[0].x, points[0].y); for (const point of points.slice(1)) drawing.lineTo(point.x, point.y); drawing.stroke();
      } else if (annotation.bounds) {
        const start = map({ x: annotation.bounds.x, y: annotation.bounds.y }); const end = map({ x: annotation.bounds.x + annotation.bounds.width, y: annotation.bounds.y + annotation.bounds.height });
        drawing.globalAlpha = 0.09; drawing.fillRect(start.x, start.y, end.x - start.x, end.y - start.y); drawing.globalAlpha = 1; drawing.strokeRect(start.x, start.y, end.x - start.x, end.y - start.y);
      }
      const anchor = annotation.bounds ? { x: annotation.bounds.x, y: annotation.bounds.y } : annotation.points[0];
      const label = annotation.comment?.trim().replace(/\s+/g, " ").slice(0, 240);
      if (anchor && label) {
        const point = map(anchor);
        const x = Math.max(2, Math.min(output.width - 2, point.x + 8));
        const y = Math.max(14, Math.min(output.height - 2, point.y + 16));
        drawing.font = "600 14px sans-serif"; drawing.textBaseline = "alphabetic"; drawing.lineWidth = 4;
        drawing.strokeStyle = "#0c1016"; drawing.strokeText(label, x, y); drawing.fillStyle = annotation.color; drawing.fillText(label, x, y);
      }
    }
    const blob = await new Promise<Blob | null>((resolve) => output.toBlob(resolve, "image/png"));
    if (!blob) return null;
    const data = await new Promise<string>((resolve, reject) => { const reader = new FileReader(); reader.onload = () => typeof reader.result === "string" ? resolve(reader.result) : reject(new Error("PNG data is unavailable")); reader.onerror = () => reject(reader.error ?? new Error("Could not read PNG")); reader.readAsDataURL(blob); });
    const comma = data.indexOf(","); return comma >= 0 ? data.slice(comma + 1) : null;
  };
  const deliverAnnotations = async (ids: string[], operationId: string, acknowledgeDuplicateRisk: boolean): Promise<boolean> => {
    if (!onFeedback) { errorRef.current = true; setStatus("error"); setMessage("Annotation delivery is unavailable."); return false; }
    setDeliveryState("pending");
    try {
      const response = await onFeedback(ids, operationId, acknowledgeDuplicateRisk);
      if (response.operation_id !== operationId) {
        errorRef.current = true;
        setDeliveryState("outcome_unknown");
        setStatus("error");
        setMessage("Feedback delivery returned a different operation identity; inspect the saved receipt before retrying.");
        return false;
      }
      if (response.state === "accepted") {
        errorRef.current = false;
        setDeliveryState(null);
        setDeliveryDuplicateRisk(false);
        setStatus("ready");
        setMessage(response.message);
        return true;
      }
      errorRef.current = true;
      setDeliveryState(response.state === "outcome_unknown" ? "outcome_unknown" : "rejected");
      setStatus("error");
      setMessage(response.message);
    } catch (error) {
      errorRef.current = true;
      setDeliveryState("outcome_unknown");
      setStatus("error");
      setMessage(`Could not confirm annotation delivery; the original operation was retained: ${errorMessage(error)}`);
    }
    return false;
  };
  const finishCaptureDelivery = async (identity: CaptureIdentity | null): Promise<void> => {
    pendingDeliveryIdsRef.current = null;
    pendingDeliveryIdentityRef.current = null;
    pendingDeliveryOperationRef.current = null;
    setDeliveryState(null);
    setDeliveryDuplicateRisk(false);
    if (!identity) {
      setMessage("Annotations sent; the original draft identity was unavailable, so newer browser work was retained.");
      return;
    }
    const owner = associationOwnerRef.current;
    const currentDraft = draftRef.current;
    const editorUnchanged = identity.owner.editorGeneration === identity.editorGeneration
      && identity.owner.noteId === identity.noteId
      && identity.owner.noteText.slice(0, 4000) === identity.noteText
      && identity.owner.notesOpen === identity.notesOpen;
    const exactCapture = owner === identity.owner && !identity.owner.sealed
      && currentDraft?.draft_id === identity.draftId
      && currentDraft.revision === identity.draftRevision
      && editorUnchanged;
    if (!exactCapture) {
      setMessage("Annotations sent; newer browser draft work was retained.");
      return;
    }
    identity.owner.draft = null;
    draftRef.current = null;
    draftRequestRef.current += 1;
    setDraft(null);
    setSelectedId(null);
    setNoteId(null);
    setNoteValue("");
    await openDraft();
  };
  const recoverPendingCapture = async (action: "retry_pending" | "discard_pending"): Promise<BrowserViewCommandOutcome | null> => {
    try {
      return await client.browserDraftRecovery({ target, action: { type: action } });
    } catch (error) {
      errorRef.current = true; setStatus("error"); setMessage(`Could not recover pending capture: ${errorMessage(error)}`);
      return null;
    }
  };
  const discardPendingCapture = async (): Promise<void> => {
    if (!pendingCaptureRef.current) { setMessage("The pending capture is still recovering."); return; }
    const discarded = await recoverPendingCapture("discard_pending");
    if (discarded?.type !== "capture" || discarded.capture.state !== "absent") return;
    pendingDeliveryIdsRef.current = null;
    pendingDeliveryIdentityRef.current = null;
    pendingDeliveryOperationRef.current = null;
    setDeliveryState(null);
    setDeliveryDuplicateRisk(false);
    await openDraft();
  };
  const resolveDuplicateRisk = async (): Promise<void> => {
    const ids = pendingDeliveryIdsRef.current;
    if (!ids || !pendingDeliveryOperationRef.current) return;
    const operationId = newId("browser-feedback");
    pendingDeliveryOperationRef.current = operationId;
    setDeliveryDuplicateRisk(false);
    const identity = pendingDeliveryIdentityRef.current;
    if (await deliverAnnotations(ids, operationId, true)) await finishCaptureDelivery(identity);
  };
  const capture = async (captureAsShown: boolean): Promise<void> => {
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
    const deliveryIds = pendingDeliveryIdsRef.current;
    if (deliveryIds) {
      const deliveryIdentity = pendingDeliveryIdentityRef.current ?? captureIdentity;
      let operationId = pendingDeliveryOperationRef.current;
      if (!operationId) {
        setDeliveryState("outcome_unknown");
        setStatus("error");
        setMessage("The retained feedback operation identity is unavailable; resync the saved receipt before retrying.");
        return;
      }
      if (deliveryState === "rejected") {
        operationId = newId("browser-feedback");
        pendingDeliveryOperationRef.current = operationId;
        setDeliveryDuplicateRisk(false);
      }
      if (await deliverAnnotations(deliveryIds, operationId, false)) await finishCaptureDelivery(deliveryIdentity);
      return;
    }
    if (pending) {
      const retried = await recoverPendingCapture("retry_pending");
      if (retried?.type === "capture" && retried.capture.state === "saved") {
        setPendingCaptureState(null);
        const ids = [...retried.capture.saved.annotation_ids];
        pendingDeliveryIdsRef.current = ids;
        const operationId = pendingDeliveryOperationRef.current ?? deliveryOperationId(retried.capture.saved.capture_id);
        pendingDeliveryOperationRef.current = operationId;
        const deliveryIdentity = pendingDeliveryIdentityRef.current ?? {
          owner: captureOwner,
          draftId: pending.draft_id,
          draftRevision: pending.draft_revision,
          editorGeneration: captureOwner.editorGeneration,
          noteId: captureOwner.noteId,
          noteText: captureOwner.noteText.slice(0, 4000),
          notesOpen: captureOwner.notesOpen,
        };
        pendingDeliveryIdentityRef.current = deliveryIdentity;
        if (await deliverAnnotations(ids, operationId, false)) await finishCaptureDelivery(deliveryIdentity);
      }
      if (retried?.type === "capture" && retried.capture.state === "pending") {
        setPendingCaptureState(retried.capture.pending);
        errorRef.current = true;
        setStatus("error");
        setMessage(retried.capture.pending.last_error ?? "Could not save the pending capture; retry or discard it.");
      }
      if (retried?.type === "capture" && retried.capture.state === "absent") {
        setPendingCaptureState(null);
        await openDraft();
      }
      return;
    }
    const preparedSnapshot = current; const where = localLocation(); const preparedDraft = draftRef.current; const beforeFrame = frameRef.current;
    if (!preparedSnapshot || !where || !preparedDraft || !beforeFrame || !preparedSnapshot.document || !preparedSnapshot.viewport) { setMessage("Wait for a confirmed frame and annotations before sending."); return; }
    try {
      const prepared = await command({ type: "capture", command: { location: where, draft_id: preparedDraft.draft_id, draft_revision: preparedDraft.revision, annotation_ids: preparedDraft.annotations.map((annotation) => annotation.id), capture_as_shown: captureAsShown } });
      if (!prepared || prepared.type !== "capture_prepared") return;
      const accepted = frameRef.current;
      if (!accepted || accepted.sequence !== prepared.descriptor.frame_sequence || accepted.descriptor.target_id !== prepared.descriptor.target_id || accepted.descriptor.stream_epoch !== prepared.descriptor.stream_epoch || accepted.descriptor.document_generation !== prepared.descriptor.document_generation || accepted.descriptor.viewport_revision !== prepared.descriptor.viewport_revision) { setMessage("Capture pixels changed before composition; retry capture."); return; }
      const png = await pngBase64(preparedDraft.annotations, accepted.descriptor);
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
        pendingDeliveryIdsRef.current = ids;
        pendingDeliveryIdentityRef.current = captureIdentity;
        const operationId = deliveryOperationId(saved.capture.saved.capture_id);
        pendingDeliveryOperationRef.current = operationId;
        if (await deliverAnnotations(ids, operationId, false)) await finishCaptureDelivery(captureIdentity);
      }
    } catch (error) {
      setStatus("error"); setMessage(`Could not capture browser image: ${errorMessage(error)}`);
    }
  };
  captureRef.current = capture;
  useEffect(() => {
    if (!registerCloseGuard) return;
    const recovery: BrowserPaneRecoveryRegistration = {
      guard: closeGuard,
      retry: async () => {
        await retryAnnotationMutations();
        if (pendingCaptureRef.current) await captureRef.current?.(false);
        if (pendingDeliveryIdsRef.current) setMessage("Feedback delivery outcome is unresolved; review or explicitly discard it before retrying.");
      },
      discard: discardAnnotationMutations,
      describe: () => {
        const draftLabel = associationOwner.draft ? `draft ${associationOwner.draft.draft_id} revision ${associationOwner.draft.revision}` : "browser draft";
        const pending = associationOwner.pendingAnnotationMutations;
        const ids = pending.map((mutation) => mutation.annotationId).join(", ");
        return `${draftLabel}; ${pending.length} retained annotation ${pending.length === 1 ? "intent" : "intents"}${ids ? ` (${ids})` : ""}; ${pendingDeliveryIdsRef.current ? "feedback delivery pending" : "no feedback delivery pending"}`;
      },
    };
    registerCloseGuard(recovery);
    return () => registerCloseGuard(null);
  }, [associationOwner, closeGuard, discardAnnotationMutations, registerCloseGuard, retryAnnotationMutations]);
  const draftMatchesSnapshot = Boolean(draft && snapshot?.displayed_target_id === draft.target_id && snapshot.document?.document_generation === draft.document_generation);
  const staleDraft = Boolean(draft && !draftMatchesSnapshot && (hasRecoverableDraft(draft) || editorDirtyRef.current));
  const annotations = draftMatchesSnapshot ? (draft?.annotations ?? []) : [];
  const descriptor = frame?.descriptor;
  const imagePoint = (point: BrowserPoint): BrowserPoint | null => {
    if (!descriptor) return null;
    const x = (point.x - descriptor.scroll_x - descriptor.viewport_offset_x) / descriptor.viewport_css_width * descriptor.image_width;
    const y = (point.y - descriptor.scroll_y - descriptor.viewport_offset_y) / descriptor.viewport_css_height * descriptor.image_height;
    return Number.isFinite(x) && Number.isFinite(y) ? { x, y } : null;
  };
  const draw = (annotation: BrowserViewDraftAnnotation, transient = false) => {
    if (!descriptor) return null; const selected = annotation.id === selectedId; const annotationKind = kindFor(annotation);
    if (annotationKind === "region" || annotationKind === "element") {
      if (!annotation.bounds) return null; const start = imagePoint({ x: annotation.bounds.x, y: annotation.bounds.y }); const end = imagePoint({ x: annotation.bounds.x + annotation.bounds.width, y: annotation.bounds.y + annotation.bounds.height });
      return start && end ? <rect key={annotation.id} className={`browser-annotation browser-annotation-${annotationKind}${selected ? " is-selected" : ""}${transient ? " is-transient" : ""}`} x={start.x} y={start.y} width={end.x - start.x} height={end.y - start.y} stroke={annotation.color} onPointerDown={(event) => { if (tool === "select") { event.stopPropagation(); setSelectedId(annotation.id); } }} /> : null;
    }
    const points = annotation.points.map(imagePoint).filter((point): point is BrowserPoint => point !== null);
    return points.length > 1 ? <polyline key={annotation.id} className={`browser-annotation browser-annotation-freehand${selected ? " is-selected" : ""}${transient ? " is-transient" : ""}`} points={points.map((point) => `${point.x},${point.y}`).join(" ")} stroke={annotation.color} onPointerDown={(event) => { if (tool === "select") { event.stopPropagation(); setSelectedId(annotation.id); } }} /> : null;
  };
  const drawLabel = (annotation: BrowserViewDraftAnnotation) => {
    if (!descriptor || !annotation.comment?.trim()) return null;
    const anchor = annotation.bounds ? { x: annotation.bounds.x, y: annotation.bounds.y } : annotation.points[0];
    const point = anchor ? imagePoint(anchor) : null;
    return point ? <text key={`${annotation.id}-label`} className="browser-annotation-label" x={point.x + 8} y={point.y + 16} fill={annotation.color}>{annotation.comment.trim().replace(/\s+/g, " ").slice(0, 240)}</text> : null;
  };
  const transient = gesture && (tool === "freehand" || tool === "region") ? { id: "transient", kind: tool === "freehand" ? "freehand" : "region", color, points: gesture.points, bounds: tool === "region" ? rectFrom(gesture.origin, gesture.points[gesture.points.length - 1]) : null, evidence: null, comment: null } satisfies BrowserViewDraftAnnotation : null;
  const inspectionBounds = inspection?.bounds && descriptor ? { ...inspection.bounds, x: inspection.bounds.x + descriptor.scroll_x, y: inspection.bounds.y + descriptor.scroll_y } : null;
  const blocker = snapshot?.blocker;
  const targetTabs = snapshot?.targets.filter((candidate) => candidate.kind === "page" || candidate.kind === "popup").sort((left, right) => left.order - right.order) ?? [];
  const tabCommand = (commandValue: Extract<BrowserViewCommand, { type: "tab" }>) => {
    if (!liveInputEnabledRef.current) return;
    void releaseRemotePointer();
    void enqueueInput("boundary", async () => { await command(commandValue); });
  };
  const reconnectView = async (): Promise<void> => {
    try {
      await onReconnect?.();
      setRetry((value) => value + 1);
    } catch (error) {
      setStatus("error");
      setMessage(`Browser reconnect failed: ${errorMessage(error)}`);
    }
  };
  return <section className={["browser-pane", `browser-pane-${status}`, `browser-tool-${tool}`, className].filter(Boolean).join(" ")} aria-label="Browser view">
    <header className="browser-toolbar"><strong>Browser</strong><span className="browser-toolbar-status" role="status" aria-live="polite">{statusText(status, message)}</span><div className="browser-toolbar-actions">{(status === "stale" || status === "error") ? <button type="button" onClick={() => void reconnectView()}>Retry</button> : null}{presentation === "browser_only" && onBackToTerminals ? <button type="button" className="browser-toolbar-icon" aria-label="Restore split" title="Restore split" onClick={onBackToTerminals}><AnnotationIcon name="expand" /></button> : null}{presentation !== "browser_only" && onExpand ? <button type="button" className="browser-toolbar-icon" aria-label="Expand browser" title="Expand browser" onClick={onExpand}><AnnotationIcon name="expand" /></button> : null}</div></header>
    <div className="browser-tabs" role="tablist" aria-label="Browser tabs">{targetTabs.map((browserTarget) => <div key={browserTarget.target_id} className="browser-tab-wrap"><button type="button" role="tab" aria-selected={browserTarget.target_id === snapshot?.displayed_target_id} title={browserTarget.url} onClick={() => { onInteractionFocus?.(); tabCommand({ type: "tab", command: { type: "select", target_id: browserTarget.target_id } }); }}>{browserTarget.title || browserTarget.url || "New tab"}</button>{browserTarget.can_close ? <button type="button" className="browser-tab-close" aria-label="Close browser tab" onClick={() => tabCommand({ type: "tab", command: { type: "close", target_id: browserTarget.target_id } })}>×</button> : null}</div>)}<button type="button" className="browser-new-tab" aria-label="New browser tab" onClick={() => tabCommand({ type: "tab", command: { type: "create", url: null } })}>+</button></div>
    <div className="browser-navigation"><button type="button" disabled={!snapshot?.navigation?.can_go_back} onClick={() => navigation("back")}>←</button><button type="button" disabled={!snapshot?.navigation?.can_go_forward} onClick={() => navigation("forward")}>→</button><button type="button" disabled={!snapshot} onClick={() => navigation(snapshot?.navigation?.loading ? "stop" : "reload")}>{snapshot?.navigation?.loading ? "■" : "↻"}</button><form onSubmit={(event) => { event.preventDefault(); navigation("navigate", url); }}><input value={url} onFocus={() => { urlEditing.current = true; onInteractionFocus?.(); }} onBlur={() => { urlEditing.current = false; setUrl(snapshotRef.current?.navigation?.url ?? ""); }} onChange={(event) => setUrl(event.target.value)} aria-label="Page URL" placeholder="Enter URL" /></form></div>
    <div className="browser-annotation-toolbar" role="toolbar" aria-label="Annotation tools">
      {(["browse", "select", "freehand", "element", "region"] as const).map((candidate) => {
        const label = candidate[0].toUpperCase() + candidate.slice(1);
        return <button key={candidate} type="button" className={tool === candidate ? "is-active" : undefined} aria-pressed={tool === candidate} aria-label={label} title={`${label} tool`} onClick={() => { setTool(candidate); gestureRef.current = null; setGesture(null); if (candidate !== "browse") onInteractionFocus?.(); }}><AnnotationIcon name={candidate} /></button>;
      })}
      <span className="browser-color-picker">{COLORS.map((candidate) => <button key={candidate} type="button" className={color === candidate ? "is-active" : undefined} aria-label={`Use ${candidate} annotation color`} title={`Use ${candidate} annotation color`} style={{ background: candidate, borderColor: candidate }} onClick={() => setColor(candidate)} />)}</span>
      <button type="button" disabled={!selectedId} aria-label="Remove" title="Remove selected annotation" onClick={() => { if (selectedId) removeAnnotation(selectedId); }}><AnnotationIcon name="remove" /></button>
      <button type="button" className="browser-annotation-notes" aria-expanded={notesOpen} aria-label={`Notes ${annotations.length}`} title={`Notes (${annotations.length})`} onClick={() => { markEditorDirty(); setNotesOpen((open) => !open); }}><AnnotationIcon name="notes" /><span className="browser-annotation-count" aria-hidden="true">{annotations.length}</span></button>
      {pendingCapture ? <><span className="browser-capture-pending" role="status">Pending capture · retry sending or discard it</span><button type="button" aria-label="Discard pending capture" title="Discard pending capture" onClick={() => void discardPendingCapture()}><AnnotationIcon name="remove" /></button></> : null}
      {pendingDeliveryIdsRef.current && deliveryState ? <div className="browser-capture-pending" role="status">
        <span>{deliveryState === "outcome_unknown" ? "Feedback delivery outcome is unknown; inspect before retrying." : deliveryState === "rejected" ? "Feedback delivery was rejected; retry the same saved capture." : "Feedback delivery is pending."}</span>
        {deliveryState === "outcome_unknown" ? <label><input type="checkbox" checked={deliveryDuplicateRisk} onChange={(event) => setDeliveryDuplicateRisk(event.target.checked)} /> I checked the destination; retry may duplicate feedback.</label> : null}
        {deliveryState === "outcome_unknown" ? <button type="button" disabled={!deliveryDuplicateRisk} onClick={() => void resolveDuplicateRisk()}>Resolve and retry</button> : null}
      </div> : null}
      {associationOwner.pendingAnnotationMutations.length > 0 ? <><span className="browser-capture-pending" role="status">Retained annotation changes need review; unknown delivery is not replayed automatically.</span><button type="button" aria-label="Retry retained annotation changes" title="Retry retained annotation changes" onClick={() => void retryAnnotationMutations()}>Retry saves</button><button type="button" aria-label="Discard retained annotation changes" title="Discard retained annotation changes" onClick={() => void discardAnnotationMutations()}>Discard retry intent</button></> : null}
      <button type="button" disabled={pendingCapture ? false : !draft || !frame || annotations.length === 0} aria-label="Send annotations" title={pendingCapture ? "Retry pending capture" : "Capture and send annotations"} onClick={() => void capture(false)}><AnnotationIcon name="feedback" /><span>Send annotations</span></button>
    </div>
      <div ref={surfaceRef} className="browser-surface" tabIndex={0} style={{ cursor: tool === "browse" ? snapshot?.cursor?.cursor ?? "default" : tool === "select" ? "default" : "crosshair" }} onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerCancel} onWheel={onWheel} onKeyDown={(event) => sendKey(event, "down")} onKeyUp={(event) => sendKey(event, "up")} onPaste={(event) => clipboard(event, false)} onCopy={(event) => clipboard(event, true)} onCompositionStart={(event) => sendComposition(event, "start")} onCompositionUpdate={(event) => sendComposition(event, "update")} onCompositionEnd={(event) => sendComposition(event, "commit")}>
      <canvas ref={canvasRef} className="browser-frame" aria-label="Live browser frame" />
      {descriptor ? <svg className="browser-annotation-layer" viewBox={`0 0 ${descriptor.image_width} ${descriptor.image_height}`} preserveAspectRatio="xMidYMid meet" aria-label="Browser annotations">{annotations.map((annotation) => draw(annotation))}{annotations.map(drawLabel)}{transient ? draw(transient, true) : null}{inspectionBounds && tool === "element" ? (() => { const start = imagePoint({ x: inspectionBounds.x, y: inspectionBounds.y }); const end = imagePoint({ x: inspectionBounds.x + inspectionBounds.width, y: inspectionBounds.y + inspectionBounds.height }); return start && end ? <rect className="browser-element-hover" x={start.x} y={start.y} width={end.x - start.x} height={end.y - start.y} /> : null; })() : null}</svg> : null}
      {frame && (status === "stale" || status === "error" || status === "unsupported") ? <div className="browser-recovery" role="status">{statusText(status, message)}</div> : null}
      {(staleDraft || staleDrafts.length > 0) ? <div className="browser-recovery browser-recovery-actions" role="status"><span>{staleDraft ? "Draft marks belong to an earlier page; review before continuing." : "Retained drafts are available for review."}</span>{staleDraft ? <><button type="button" onClick={startCurrentDraft}>Start current draft</button><button type="button" onClick={() => void discardStaleDraft()}>Discard stale draft</button></> : null}{staleDrafts.length > 0 ? <ul>{staleDrafts.map((candidate) => <li key={candidate.draft_id}><button type="button" onClick={() => reviewInventoryDraft(candidate)}>Review draft revision {candidate.revision}</button><button type="button" onClick={() => void discardInventoryDraft(candidate)}>Discard</button></li>)}</ul> : null}</div> : null}
      {blocker ? <div className="browser-blocker" role="alert"><strong>{blocker.message}</strong>{blocker.kind === "dialog" ? <div><button type="button" onClick={() => void command({ type: "dialog", blocker_id: blocker.blocker_id, command: { type: "accept", text: blocker.default_prompt } })}>Accept</button>{blocker.cancellable ? <button type="button" onClick={() => void command({ type: "dialog", blocker_id: blocker.blocker_id, command: { type: "dismiss" } })}>Dismiss</button> : null}</div> : blocker.kind === "download" ? <div><button type="button" onClick={() => void command({ type: "download", blocker_id: blocker.blocker_id, command: { type: "accept" } })}>Save download</button><button type="button" onClick={() => void command({ type: "download", blocker_id: blocker.blocker_id, command: { type: "cancel" } })}>Cancel</button></div> : blocker.kind === "permission" ? <div><button type="button" onClick={() => void command({ type: "permission", blocker_id: blocker.blocker_id, command: { decision: "allow" } })}>Allow</button><button type="button" onClick={() => void command({ type: "permission", blocker_id: blocker.blocker_id, command: { decision: "deny" } })}>Deny</button></div> : blocker.kind === "file_chooser" ? <button type="button" onClick={() => void command({ type: "file", blocker_id: blocker.blocker_id, command: { type: "cancel" } })}>Cancel file chooser</button> : null}</div> : null}
      {noteId ? <div className="browser-note-editor"><textarea autoFocus value={noteValue} onChange={(event) => { markEditorDirty(); setNoteValue(event.target.value.slice(0, 4000)); }} maxLength={4000} aria-label="Annotation note" onKeyDown={(event) => { if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) { event.preventDefault(); saveNote(); } if (event.key === "Escape") { setNoteId(null); setNoteValue(""); markEditorDirty(); } }} /><div><button type="button" onClick={saveNote}>Save</button><button type="button" onClick={() => { setNoteId(null); setNoteValue(""); markEditorDirty(); }}>Cancel</button></div></div> : null}
    </div>
    {notesOpen ? <aside className="browser-notes" aria-label="Annotation notes"><header><strong>Notes</strong><button type="button" onClick={() => { markEditorDirty(); setNotesOpen(false); }}>Close</button></header>{annotations.length === 0 ? <p>No annotations yet.</p> : <ul>{annotations.map((annotation, index) => <li key={annotation.id}><button type="button" className={selectedId === annotation.id ? "is-selected" : undefined} onClick={() => { setSelectedId(annotation.id); setNoteId(annotation.id); setNoteValue(annotation.comment ?? ""); }}>{index + 1}. {kindFor(annotation)}{annotation.comment ? ` — ${annotation.comment}` : ""}</button></li>)}</ul>}<button type="button" disabled={annotations.length === 0} onClick={() => { const current = snapshotRef.current; const documentContext = current ? context(current) : null; const currentDraft = draftRef.current; if (documentContext && currentDraft) void command({ type: "draft", context: documentContext, draft_id: currentDraft.draft_id, expected_revision: currentDraft.revision, command: { type: "clear" } }); }}>Clear annotations</button></aside> : null}
  </section>;
}
