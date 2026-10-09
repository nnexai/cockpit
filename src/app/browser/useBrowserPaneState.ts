import { useCallback, useEffect, useRef, useState, type Dispatch, type RefObject, type SetStateAction } from "react";
import type { BrowserPoint, BrowserTarget, BrowserViewDraftState, BrowserViewInspectResult, BrowserViewPendingCapture, BrowserViewSnapshot, BrowserWorkScope } from "../../protocol/generated/v1";
import type { BrowserViewFramePacket, BrowserViewStream } from "../../client/CockpitClient";
import type { FramePresenter } from "./framePresenter";
import type { BrowserFeedbackPanelHandle } from "./BrowserFeedbackPanel";
import { COLORS } from "./AnnotationControls";
import { addressBarUrl, DelayedNotice, retiredDraft, type DraftAssociationOwner, type ElementInspectionIntent, type Gesture, type InputJob, type PaneStatus, type PointerIntent, type Tool } from "./browserPaneModel";
import { frameContext } from "./browserCanvas";

export function useBrowserPaneState({ target, visible, liveInputEnabled }: { target: BrowserTarget; visible: boolean; liveInputEnabled: boolean }): BrowserPaneState {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const streamRef = useRef<BrowserViewStream | null>(null);
  const identityRef = useRef<{ id: string; epoch: number } | null>(null);
  const frameRef = useRef<{ descriptor: BrowserViewFramePacket["descriptor"]; sequence: number } | null>(null);
  const presenterRef = useRef<FramePresenter | null>(null);
  const snapshotRef = useRef<BrowserViewSnapshot | null>(null);
  const pendingDeliveryIdsRef = useRef<string[] | null>(null);
  const feedbackPanelRef = useRef<BrowserFeedbackPanelHandle | null>(null);
  const inputOverloadedRef = useRef(false);
  const releaseOverloadedInputRef = useRef<(() => void) | null>(null);
  const draftRef = useRef<BrowserViewDraftState | null>(null);
  const pendingCaptureRef = useRef<BrowserViewPendingCapture | null>(null);
  const captureInFlightRef = useRef(false);
  const draftRequestRef = useRef(0);
  const editorDirtyRef = useRef(false);
  const inputSequence = useRef(1);
  const inputJobsRef = useRef<InputJob[]>([]);
  const inputGenerationRef = useRef(0);
  const gestureRef = useRef<Gesture | null>(null);
  const remotePointerRef = useRef<number | null>(null);
  const remotePointRef = useRef<BrowserPoint | null>(null);
  const remotePointerIntentRef = useRef<PointerIntent | null>(null);
  const elementIntentRef = useRef<ElementInspectionIntent | null>(null);
  const pendingElementClientPointRef = useRef<{ clientX: number; clientY: number; targetId: string; documentGeneration: number; viewportRevision: number } | null>(null);
  const inspectRequestRef = useRef(0);
  const inspectionNoticeRef = useRef<string | null>(null);
  // A refused input is only worth a notice when the next input does not go through.
  const staleInputNoticeRef = useRef<DelayedNotice | null>(null);
  staleInputNoticeRef.current ??= new DelayedNotice(
    (notice) => { setStatus("stale"); setMessage(notice); },
    (notice) => {
      setMessage((current) => current === notice ? null : current);
      setStatus((current) => current === "stale" ? (streamRef.current ? "ready" : "loading") : current);
    },
  );
  useEffect(() => () => staleInputNoticeRef.current?.dispose(), []);
  const hoverInspectTimerRef = useRef<number | null>(null);
  const hoverInspectPointRef = useRef<{ clientX: number; clientY: number } | null>(null);
  const selectingElementRef = useRef(false);
  const urlEditing = useRef(false);
  const errorRef = useRef(false);
  const liveInputEnabledRef = useRef(liveInputEnabled);
  liveInputEnabledRef.current = liveInputEnabled;
  const associationOwnerRef = useRef<DraftAssociationOwner | null>(null);
  const ownerKey = `${target.session_id}:${target.tab_id ?? ""}:${target.pane_id ?? ""}:${target.endpoint_path ?? ""}`;
  if (!associationOwnerRef.current || associationOwnerRef.current.key !== ownerKey) {
    associationOwnerRef.current = { key: ownerKey, target: { ...target }, draft: null, localDraftRevision: null, noteId: null, noteText: "", notesOpen: false, editorGeneration: 0, sealed: false, mutationTail: Promise.resolve(), pendingAnnotationMutations: [], retiredDraftIds: new Set(), retiredDocumentKeys: new Set() };
  }
  const associationOwner = associationOwnerRef.current!;
  const workScope: BrowserWorkScope = { kind: "tab", target: associationOwner.target };
  const [status, setStatus] = useState<PaneStatus>(visible ? "loading" : "hidden");
  const [message, setMessage] = useState<string | null>(null);
  const [annotationNotice, setAnnotationNotice] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<BrowserViewSnapshot | null>(null);
  const [frame, setFrame] = useState<{ descriptor: BrowserViewFramePacket["descriptor"]; sequence: number } | null>(null);
  const [pendingCapture, setPendingCapture] = useState<BrowserViewPendingCapture | null>(null);
  const [draft, setDraft] = useState<BrowserViewDraftState | null>(null);
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
  const [noteEditorDismissed, setNoteEditorDismissed] = useState(false);
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
    if (!urlEditing.current) setUrl(addressBarUrl(next.navigation?.url));
  }, [associationOwner]);
  const applyDraft = useCallback((next: BrowserViewDraftState) => {
    if (retiredDraft(associationOwner, next)) return;
    const current = draftRef.current;
    if (current && current.draft_id === next.draft_id && next.revision < current.revision) return;
    draftRef.current = next; associationOwner.draft = next; setDraft(next);
    setSelectedId((selected) => selected && !next.annotations.some((annotation) => annotation.id === selected) ? null : selected);
    const editingId = noteIdRef.current;
    if (editingId && !next.annotations.some((annotation) => annotation.id === editingId)) {
      noteIdRef.current = null;
      noteValueRef.current = "";
      editorDirtyRef.current = false;
      setNoteId(null);
      setNoteValue("");
      setNoteEditorDismissed(false);
    }
    const editor = next.editor;
    if (!editorDirtyRef.current) {
      const selectedAnnotation = editor.note_annotation_id ?? editor.selected_annotation_id;
      const liveSelected = selectedAnnotation && next.annotations.some((annotation) => annotation.id === selectedAnnotation) ? selectedAnnotation : null;
      setSelectedId((value) => value ?? liveSelected);
      setNoteId(liveSelected);
      setNoteValue(liveSelected ? (editor.note_text ?? next.annotations.find((annotation) => annotation.id === liveSelected)?.comment ?? "").slice(0, 4000) : "");
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
    const drawing = frameContext(canvas);
    if (canvas && drawing) drawing.clearRect(0, 0, canvas.width, canvas.height);
  }, []);
  const invalidateInteractionFrame = useCallback(() => {
    gestureRef.current = null;
    setGesture(null);
  }, []);
  return {
    canvasRef, surfaceRef, streamRef, identityRef, frameRef, presenterRef, snapshotRef,
    pendingDeliveryIdsRef, feedbackPanelRef, inputOverloadedRef, releaseOverloadedInputRef,
    draftRef, pendingCaptureRef, captureInFlightRef, draftRequestRef, editorDirtyRef,
    inputSequence, inputJobsRef, inputGenerationRef, gestureRef, remotePointerRef, remotePointRef,
    remotePointerIntentRef, elementIntentRef, pendingElementClientPointRef, inspectRequestRef,
    inspectionNoticeRef, staleInputNoticeRef, hoverInspectTimerRef, hoverInspectPointRef,
    selectingElementRef, urlEditing, errorRef, liveInputEnabledRef, associationOwnerRef,
    ownerKey, associationOwner, workScope, status, setStatus, message, setMessage,
    annotationNotice, setAnnotationNotice, snapshot, setSnapshot, frame, setFrame,
    pendingCapture, setPendingCapture, draft, setDraft, tool, setTool, color, setColor,
    selectedId, setSelectedId, gesture, setGesture, inspection, setInspection, notesOpen,
    setNotesOpen, noteId, setNoteId, noteValue, setNoteValue, url, setUrl, retry, setRetry,
    noteIdRef, noteValueRef, noteEditorDismissed, setNoteEditorDismissed, editorRevisionRef,
    markEditorDirty, applySnapshot, applyDraft, setPendingCaptureState, clearPresentedFrame,
    invalidateInteractionFrame,
  };
}

type Ref<T> = RefObject<T>;
type Setter<T> = Dispatch<SetStateAction<T>>;

export interface BrowserPaneState {
  associationOwnerRef: Ref<DraftAssociationOwner | null>; associationOwner: DraftAssociationOwner; ownerKey: string; workScope: BrowserWorkScope;
  canvasRef: Ref<HTMLCanvasElement | null>; surfaceRef: Ref<HTMLDivElement | null>; feedbackPanelRef: Ref<BrowserFeedbackPanelHandle | null>;
  streamRef: Ref<BrowserViewStream | null>; identityRef: Ref<{ id: string; epoch: number } | null>;
  frameRef: Ref<{ descriptor: BrowserViewFramePacket["descriptor"]; sequence: number } | null>; presenterRef: Ref<FramePresenter | null>;
  snapshotRef: Ref<BrowserViewSnapshot | null>; errorRef: Ref<boolean>; staleInputNoticeRef: Ref<DelayedNotice | null>; liveInputEnabledRef: Ref<boolean>;
  inputSequence: Ref<number>; inputJobsRef: Ref<InputJob[]>; inputGenerationRef: Ref<number>; inputOverloadedRef: Ref<boolean>; releaseOverloadedInputRef: Ref<(() => void) | null>;
  gestureRef: Ref<Gesture | null>; remotePointerRef: Ref<number | null>; remotePointRef: Ref<BrowserPoint | null>; remotePointerIntentRef: Ref<PointerIntent | null>;
  elementIntentRef: Ref<ElementInspectionIntent | null>;
  pendingElementClientPointRef: Ref<{ clientX: number; clientY: number; targetId: string; documentGeneration: number; viewportRevision: number } | null>;
  inspectRequestRef: Ref<number>; inspectionNoticeRef: Ref<string | null>;
  hoverInspectTimerRef: Ref<number | null>; hoverInspectPointRef: Ref<{ clientX: number; clientY: number } | null>; selectingElementRef: Ref<boolean>; urlEditing: Ref<boolean>;
  draftRef: Ref<BrowserViewDraftState | null>; draftRequestRef: Ref<number>; editorDirtyRef: Ref<boolean>; editorRevisionRef: Ref<number>;
  noteIdRef: Ref<string | null>; noteValueRef: Ref<string>;
  pendingCaptureRef: Ref<BrowserViewPendingCapture | null>; pendingDeliveryIdsRef: Ref<string[] | null>; captureInFlightRef: Ref<boolean>;
  status: PaneStatus; setStatus: Setter<PaneStatus>; message: string | null; setMessage: Setter<string | null>;
  annotationNotice: string | null; setAnnotationNotice: Setter<string | null>;
  snapshot: BrowserViewSnapshot | null; setSnapshot: Setter<BrowserViewSnapshot | null>;
  frame: { descriptor: BrowserViewFramePacket["descriptor"]; sequence: number } | null; setFrame: Setter<{ descriptor: BrowserViewFramePacket["descriptor"]; sequence: number } | null>;
  pendingCapture: BrowserViewPendingCapture | null; setPendingCapture: Setter<BrowserViewPendingCapture | null>;
  draft: BrowserViewDraftState | null; setDraft: Setter<BrowserViewDraftState | null>;
  tool: Tool; setTool: Setter<Tool>; color: string; setColor: Setter<string>; selectedId: string | null; setSelectedId: Setter<string | null>;
  gesture: Gesture | null; setGesture: Setter<Gesture | null>; inspection: BrowserViewInspectResult | null; setInspection: Setter<BrowserViewInspectResult | null>;
  notesOpen: boolean; setNotesOpen: Setter<boolean>; noteId: string | null; setNoteId: Setter<string | null>; noteValue: string; setNoteValue: Setter<string>;
  noteEditorDismissed: boolean; setNoteEditorDismissed: Setter<boolean>; url: string; setUrl: Setter<string>; retry: number; setRetry: Setter<number>;
  markEditorDirty(): void; applySnapshot(next: BrowserViewSnapshot): void; applyDraft(next: BrowserViewDraftState): void;
  setPendingCaptureState(next: BrowserViewPendingCapture | null): void; clearPresentedFrame(clearCanvas?: boolean): void; invalidateInteractionFrame(): void;
}
