import { useCallback, useEffect, useRef, useState, type ClipboardEvent, type CompositionEvent, type FormEvent, type KeyboardEvent, type PointerEvent, type WheelEvent } from "react";
import type { BrowserCaptureSubmission, BrowserInlineCaptureProvenance, BrowserPoint, BrowserRect, BrowserTarget, BrowserViewCommand, BrowserViewCommandOutcome, BrowserViewDraftAnnotation, BrowserViewDraftState, BrowserViewEvent, BrowserViewInspectResult, BrowserViewLocation, BrowserViewOpenRequest, BrowserViewPresentation, BrowserViewSnapshot, BrowserViewViewportRequest } from "../../protocol/generated/v1";
import type { BrowserViewFramePacket, BrowserViewStream, CockpitClient } from "../../client/CockpitClient";
import { BrowserFrameError, FramePresenter, validateFrameDescriptor } from "./framePresenter";
import { createBrowserTransform } from "./transform";
import "./browser.css";

export interface BrowserPaneProps {
  client: CockpitClient; target: BrowserTarget; viewport: BrowserViewViewportRequest;
  visible?: boolean; presentation?: BrowserViewPresentation; clientId?: string;
  inputActive?: boolean; onInteractionFocus?: () => void; onFeedback?: () => void;
  onHide?: () => void; onBackToTerminals?: () => void; className?: string;
}

type PaneStatus = "hidden" | "loading" | "ready" | "stale" | "error" | "unsupported" | "empty";
type Tool = "browse" | "select" | "freehand" | "region" | "element";
type Gesture = { pointerId: number; points: BrowserPoint[]; origin: BrowserPoint; frame: number; document: number; viewport: number };
const COLORS = ["#d62828", "#1769aa", "#2a9d55", "#c27803", "#7c3aed"] as const;
const TOLERANCE = 1.5;

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
  if (status === "stale") return "Showing last confirmed frame; waiting for recovery";
  if (status === "empty") return "No browser page is selected";
  if (status === "hidden") return "Browser view hidden";
  return message ?? (status === "unsupported" ? "Browser view is unsupported by this runtime" : "Browser stream error");
};
const button = (value: number): "left" | "middle" | "right" | null => value === 0 ? "left" : value === 1 ? "middle" : value === 2 ? "right" : null;
const modifiers = (event: { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }): number => (event.altKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.metaKey ? 4 : 0) | (event.shiftKey ? 8 : 0);
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

export function BrowserPane({ client, target, viewport, visible = true, presentation = "split", clientId, inputActive = true, onInteractionFocus, onFeedback, onHide, onBackToTerminals, className }: BrowserPaneProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const streamRef = useRef<BrowserViewStream | null>(null);
  const identityRef = useRef<{ id: string; epoch: number } | null>(null);
  const snapshotRef = useRef<BrowserViewSnapshot | null>(null);
  const frameRef = useRef<{ descriptor: BrowserViewFramePacket["descriptor"]; sequence: number } | null>(null);
  const draftRef = useRef<BrowserViewDraftState | null>(null);
  const inputSequence = useRef(1);
  const gestureRef = useRef<Gesture | null>(null);
  const controlPromiseRef = useRef<Promise<boolean> | null>(null);
  const remotePointerRef = useRef<number | null>(null);
  const remotePointRef = useRef<BrowserPoint | null>(null);
  const inspectRequestRef = useRef(0);
  const compositionActiveRef = useRef(false);
  const suppressInputRef = useRef(false);
  const urlEditing = useRef(false);
  const clientRef = useRef(clientId ?? newId("cockpit-browser-view"));
  const viewportRef = useRef(viewport);
  viewportRef.current = viewport;
  const [status, setStatus] = useState<PaneStatus>(visible ? "loading" : "hidden");
  const [message, setMessage] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<BrowserViewSnapshot | null>(null);
  const [frame, setFrame] = useState<{ descriptor: BrowserViewFramePacket["descriptor"]; sequence: number } | null>(null);
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

  const applySnapshot = useCallback((next: BrowserViewSnapshot) => {
    snapshotRef.current = next; inputSequence.current = Math.max(inputSequence.current, next.control.next_input_sequence); setSnapshot(next);
    if (!urlEditing.current) setUrl(next.navigation?.url ?? "");
  }, []);
  const applyDraft = useCallback((next: BrowserViewDraftState) => {
    draftRef.current = next; setDraft(next);
    setSelectedId((selected) => selected && !next.annotations.some((annotation) => annotation.id === selected) ? null : selected);
  }, []);
  const command = useCallback(async (value: BrowserViewCommand): Promise<BrowserViewCommandOutcome | null> => {
    const stream = streamRef.current; const identity = identityRef.current;
    if (!stream || !identity) { setMessage("Browser controls are still connecting."); return null; }
    try {
      const response = await stream.command({ view_id: identity.id, stream_epoch: identity.epoch, request_id: newId("browser-command"), command: value });
      if (response.status !== "accepted") {
        setMessage(response.message);
        if (response.status === "stale") setStatus("stale");
        else if (response.status === "unsupported") setStatus("unsupported");
        else if (response.status === "rejected" || response.status === "outcome_unknown") setStatus("error");
        return null;
      }
      if (response.outcome.type === "snapshot") applySnapshot(response.outcome.snapshot);
      else if (response.outcome.type === "control" && snapshotRef.current) applySnapshot({ ...snapshotRef.current, control: response.outcome.control });
      else if (response.outcome.type === "draft") applyDraft(response.outcome.draft);
      else if (response.outcome.type === "capture_prepared") setMessage(`Composing capture ${response.outcome.capture_id}…`);
      else if (response.outcome.type === "clipboard" && response.outcome.text && navigator.clipboard?.writeText) await navigator.clipboard.writeText(response.outcome.text);
      return response.outcome;
    } catch (error) { setStatus("error"); setMessage(errorMessage(error)); return null; }
  }, [applyDraft, applySnapshot]);
  const ensureControl = useCallback(async (): Promise<boolean> => {
    const current = snapshotRef.current;
    if (!current) return false;
    if (current.control.status === "controlled") return true;
    if (controlPromiseRef.current) return controlPromiseRef.current;
    if (!current.control.can_take_control) { setMessage("Another Cockpit view controls this browser."); return false; }
    const pending = command({
      type: "take_control",
      viewport: {
        css_width: current.viewport?.css_width ?? viewport.css_width,
        css_height: current.viewport?.css_height ?? viewport.css_height,
        device_pixel_ratio: current.viewport?.device_pixel_ratio ?? viewport.device_pixel_ratio,
      },
    }).then(() => snapshotRef.current?.control.status === "controlled").catch(() => false);
    controlPromiseRef.current = pending;
    try {
      return await pending;
    } finally {
      if (controlPromiseRef.current === pending) controlPromiseRef.current = null;
    }
  }, [command, viewport]);
  const openDraft = useCallback((): void => {
    const current = snapshotRef.current; const documentContext = current ? context(current) : null;
    if (!documentContext) return;
    void command({ type: "draft", context: documentContext, draft_id: null, expected_revision: null, command: { type: "open", draft_id: null } });
  }, [command]);

  useEffect(() => {
    const controller = new AbortController(); let closed = false; let presenter: FramePresenter | null = null; let cursor: number | null = null;
    const close = (): void => { closed = true; controller.abort(); presenter?.close(); streamRef.current?.close(); streamRef.current = null; };
    streamRef.current?.close(); streamRef.current = null;
    identityRef.current = null; snapshotRef.current = null; draftRef.current = null; frameRef.current = null; setSnapshot(null); setDraft(null); setFrame(null); setInspection(null); setGesture(null); gestureRef.current = null;
    if (!visible) { setStatus("hidden"); return close; }
    setStatus("loading"); setMessage(null);
    const event = (incoming: BrowserViewEvent): void => {
      if (closed) return;
      if (incoming.type === "attached") {
        if (identityRef.current) return;
        identityRef.current = { id: incoming.metadata.view_id, epoch: incoming.metadata.stream_epoch }; cursor = incoming.metadata.metadata_sequence; applySnapshot(incoming.snapshot); setStatus(statusFor(incoming.snapshot)); queueMicrotask(openDraft); return;
      }
      const identity = identityRef.current;
      if (!identity || incoming.metadata.view_id !== identity.id || incoming.metadata.stream_epoch !== identity.epoch) return;
      if (cursor !== null && incoming.metadata.metadata_sequence !== cursor + 1) { setStatus("stale"); setMessage("Browser metadata needs a fresh snapshot."); close(); setRetry((value) => value + 1); return; }
      cursor = incoming.metadata.metadata_sequence;
      const previous = snapshotRef.current; if (!previous) return;
      let next = previous;
      switch (incoming.type) {
        case "targets_changed": next = { ...previous, targets: incoming.targets, displayed_target_id: incoming.displayed_target_id }; break;
        case "document_changed": next = { ...previous, document: incoming.document }; draftRef.current = null; setDraft(null); setSelectedId(null); queueMicrotask(openDraft); break;
        case "viewport_changed": next = { ...previous, viewport: incoming.viewport }; break;
        case "navigation_changed": next = { ...previous, navigation: incoming.navigation }; break;
        case "cursor_changed": next = { ...previous, cursor: incoming.cursor }; break;
        case "focus_changed": next = { ...previous, focus: incoming.focus }; break;
        case "blocker_changed": next = { ...previous, blocker: incoming.blocker }; break;
        case "capabilities_changed": next = { ...previous, capabilities: incoming.capabilities }; break;
        case "control_changed": next = { ...previous, control: incoming.control }; break;
        case "frame_descriptor": return;
        case "frame_transport_revoked": setStatus("stale"); setMessage(incoming.message); return;
        case "failed": setStatus("error"); setMessage(incoming.message); return;
        case "closed": setStatus("error"); setMessage(incoming.reason); return;
      }
      applySnapshot(next); setStatus(frameRef.current && statusFor(next) === "loading" ? "stale" : statusFor(next));
    };
    presenter = new FramePresenter({
      validate: (descriptor) => {
        const current = snapshotRef.current; const identity = identityRef.current;
        if (!current?.document || !current.viewport || !current.displayed_target_id || !identity) throw new BrowserFrameError("identity_mismatch", "Browser frame arrived before its snapshot");
        validateFrameDescriptor(descriptor, { streamEpoch: identity.epoch, targetId: current.displayed_target_id, displayedTargetId: current.displayed_target_id, documentGeneration: current.document.document_generation, viewportRevision: current.viewport.viewport_revision, viewportCssWidth: current.viewport.css_width, viewportCssHeight: current.viewport.css_height });
      },
      present: (image, descriptor) => {
        const targetCanvas = canvasRef.current; const drawing = targetCanvas?.getContext("2d");
        if (!targetCanvas || !drawing) throw new Error("Browser view canvas is unavailable");
        targetCanvas.width = descriptor.image_width; targetCanvas.height = descriptor.image_height; drawing.clearRect(0, 0, targetCanvas.width, targetCanvas.height); drawing.drawImage(image, 0, 0, descriptor.image_width, descriptor.image_height);
        const accepted = { descriptor, sequence: descriptor.frame_sequence }; frameRef.current = accepted; setFrame(accepted); setStatus("ready"); setMessage(null);
      },
      onError: (error) => { if (!closed) { setStatus(error instanceof BrowserFrameError && error.reason === "identity_mismatch" ? "stale" : "error"); setMessage(error instanceof BrowserFrameError ? "A late browser frame was discarded; waiting for recovery." : errorMessage(error)); } },
    });
    const request: BrowserViewOpenRequest = { target, client_id: clientId ?? clientRef.current, presentation, viewport: viewportRef.current, takeover: false };
    void client.openBrowserView(request, event, (packet) => presenter?.push(packet), (error) => { if (!closed) { setStatus("error"); setMessage(errorMessage(error)); } }, controller.signal).then((stream) => { if (closed) stream.close(); else { streamRef.current = stream; openDraft(); } }).catch((error: unknown) => { if (!closed && !controller.signal.aborted) { setStatus("error"); setMessage(errorMessage(error)); } });
    return close;
  }, [applySnapshot, client, clientId, openDraft, presentation, retry, target.endpoint_path, target.pane_id, target.session_id, target.space_id, visible]);
  useEffect(() => { if (!inputActive && snapshotRef.current?.control.status === "controlled") void command({ type: "release_control", lease_generation: snapshotRef.current.control.lease_generation }); }, [command, inputActive]);
  useEffect(() => { const timer = window.setTimeout(() => { const current = snapshotRef.current; const documentContext = current ? context(current) : null; if (visible && current?.control.status === "controlled" && documentContext && current.viewport && (current.viewport.css_width !== viewport.css_width || current.viewport.css_height !== viewport.css_height || current.viewport.device_pixel_ratio !== viewport.device_pixel_ratio)) void command({ type: "resize", context: documentContext, viewport }); }, 120); return () => window.clearTimeout(timer); }, [command, viewport, visible]);

  const pointFor = useCallback((event: { clientX: number; clientY: number }, allowOutside = false): BrowserPoint | null => {
    const current = frameRef.current; const surface = surfaceRef.current; if (!current || !surface) return null;
    const bounds = surface.getBoundingClientRect(); const aspect = current.descriptor.image_width / current.descriptor.image_height;
    const width = Math.min(bounds.width, bounds.height * aspect); const height = width / aspect;
    const painted = { left: bounds.left + (bounds.width - width) / 2, top: bounds.top + (bounds.height - height) / 2, width, height };
    let clientX = event.clientX; let clientY = event.clientY;
    if (allowOutside) {
      clientX = Math.max(painted.left, Math.min(painted.left + painted.width - Number.EPSILON, clientX));
      clientY = Math.max(painted.top, Math.min(painted.top + painted.height - Number.EPSILON, clientY));
    }
    return createBrowserTransform(current.descriptor, painted)?.clientToDocument(clientX, clientY) ?? null;
  }, []);
  const localLocation = (): BrowserViewLocation | null => snapshotRef.current && frameRef.current ? location(snapshotRef.current, frameRef.current.descriptor) : null;
  const nextInput = (): number => inputSequence.current++;
  const persist = (annotation: BrowserViewDraftAnnotation): void => {
    const current = snapshotRef.current; const documentContext = current ? context(current) : null; const currentDraft = draftRef.current;
    if (!documentContext || !currentDraft) { setMessage("The browser draft is still recovering."); return; }
    void command({ type: "draft", context: documentContext, draft_id: currentDraft.draft_id, expected_revision: currentDraft.revision, command: { type: "upsert_annotation", annotation } });
  };
  const removeAnnotation = (annotationId: string): void => {
    const current = snapshotRef.current; const documentContext = current ? context(current) : null; const currentDraft = draftRef.current;
    if (!documentContext || !currentDraft) { setMessage("The browser draft is still recovering."); return; }
    void command({ type: "draft", context: documentContext, draft_id: currentDraft.draft_id, expected_revision: currentDraft.revision, command: { type: "remove_annotation", annotation_id: annotationId } });
    setSelectedId((selected) => selected === annotationId ? null : selected);
  };
  const inspect = async (point: BrowserPoint): Promise<BrowserViewInspectResult | null> => {
    const current = snapshotRef.current; const inspected = localLocation(); const request = ++inspectRequestRef.current;
    if (!current?.cursor || !inspected) return null;
    const outcome = await command({ type: "inspect", command: { location: inspected, pointer_sample_sequence: current.cursor.pointer_sample_sequence, x: point.x, y: point.y } });
    if (request !== inspectRequestRef.current) return null;
    return outcome?.type === "inspection" ? outcome.inspection : null;
  };
  const remotePointer = (event: PointerEvent<HTMLDivElement>, kind: "move" | "down" | "up" | "cancel"): void => {
    const active = remotePointerRef.current === event.pointerId;
    const point = pointFor(event, active);
    if (!point) return;
    if (kind === "down") remotePointerRef.current = event.pointerId;
    remotePointRef.current = point;
    void ensureControl().then((controlled) => {
      const where = controlled ? localLocation() : null;
      if (where) void command({ type: "pointer", location: where, input: { kind, button: button(event.button), x: point.x, y: point.y, buttons: event.buttons, modifiers: modifiers(event), click_count: event.detail, input_sequence: nextInput() } });
    });
    if (kind === "up" || kind === "cancel") {
      remotePointerRef.current = null;
      remotePointRef.current = null;
    }
  };
  const releaseRemotePointer = (): void => {
    const current = snapshotRef.current; const where = localLocation(); const point = remotePointRef.current;
    if (current && where && point && current.control.status === "controlled") void command({ type: "pointer", location: where, input: { kind: "cancel", button: null, x: point.x, y: point.y, buttons: 0, modifiers: 0, click_count: 0, input_sequence: nextInput() } });
    remotePointerRef.current = null; remotePointRef.current = null;
  };
  useEffect(() => {
    if (!inputActive) releaseRemotePointer();
  }, [inputActive, releaseRemotePointer]);
  const previousToolRef = useRef<Tool>(tool);
  useEffect(() => {
    if (previousToolRef.current === "browse" && tool !== "browse") releaseRemotePointer();
    previousToolRef.current = tool;
  }, [releaseRemotePointer, tool]);
  const onPointerDown = (event: PointerEvent<HTMLDivElement>): void => {
    onInteractionFocus?.();
    if (tool === "browse") {
      const point = pointFor(event);
      if (point) {
        remotePointerRef.current = event.pointerId;
        event.currentTarget.setPointerCapture(event.pointerId);
        inputRef.current?.focus({ preventScroll: true });
        remotePointer(event, "down");
      }
      return;
    }
    event.preventDefault(); const point = pointFor(event); if (!point) return;
    if (tool === "element") { void inspect(point); return; }
    if (tool === "select") return;
    const current = snapshotRef.current; const accepted = frameRef.current;
    if (!current?.document || !current.viewport || !accepted) return;
    const active = { pointerId: event.pointerId, points: [point], origin: point, frame: accepted.sequence, document: current.document.document_generation, viewport: current.viewport.viewport_revision };
    gestureRef.current = active; setGesture(active); event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>): void => {
    if (tool === "browse") { remotePointer(event, "move"); return; }
    const active = gestureRef.current;
    if (active?.pointerId === event.pointerId) { const point = pointFor(event); if (point) { const next = { ...active, points: [...active.points, point] }; gestureRef.current = next; setGesture(next); } }
    else if (tool === "element") { const point = pointFor(event); if (point) void inspect(point); }
  };
  const finishGesture = (event: PointerEvent<HTMLDivElement>, cancelled: boolean): void => {
    const active = gestureRef.current; gestureRef.current = null; setGesture(null); if (!active || cancelled) return;
    const current = snapshotRef.current; const accepted = frameRef.current; const last = pointFor(event) ?? active.points[active.points.length - 1];
    if (!current?.document || !current.viewport || !accepted || current.document.document_generation !== active.document || current.viewport.viewport_revision !== active.viewport) { setMessage("The page geometry changed while drawing; the unfinished mark was cancelled."); return; }
    if (tool === "freehand") { const points = simplify([...active.points, last]); if (points.length > 1) persist({ id: annotationId(), kind: "freehand", color, points, bounds: null, evidence: null, comment: null }); }
    if (tool === "region") { const bounds = rectFrom(active.origin, last); if (bounds.width >= 1 || bounds.height >= 1) persist({ id: annotationId(), kind: "region", color, points: [active.origin, last], bounds, evidence: null, comment: null }); }
  };
  const onPointerUp = (event: PointerEvent<HTMLDivElement>): void => {
    if (tool === "browse") { remotePointer(event, "up"); if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); return; }
    if (tool === "element") {
      const point = pointFor(event); if (!point) return;
      void inspect(point).then((result) => {
        if (result?.inspectable && result.evidence && result.bounds && result.pointer_sample_sequence === snapshotRef.current?.cursor?.pointer_sample_sequence && result.location.document_generation === snapshotRef.current?.document?.document_generation) persist({ id: annotationId(), kind: "element", color, points: [{ x: result.bounds.x, y: result.bounds.y }], bounds: result.bounds, evidence: result.evidence, comment: null });
        else if (result) setMessage(result.limitation ?? "The selected element changed before it could be saved.");
      });
      return;
    }
    finishGesture(event, false);
  };
  const onPointerCancel = (event: PointerEvent<HTMLDivElement>): void => {
    if (tool === "browse") { remotePointer(event, "cancel"); if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); return; }
    finishGesture(event, true);
  };
  const onWheel = (event: WheelEvent<HTMLDivElement>): void => {
    if (tool !== "browse") return;
    const point = pointFor(event); if (!point) return;
    const line = 16; const page = snapshotRef.current?.viewport;
    const scale = event.deltaMode === 1 ? line : event.deltaMode === 2 ? (page?.css_height ?? 800) : 1;
    event.preventDefault(); onInteractionFocus?.(); void ensureControl().then((controlled) => {
      const where = controlled ? localLocation() : null;
      if (where) void command({ type: "wheel", location: where, input: { x: point.x, y: point.y, delta_x_css: event.deltaX * scale, delta_y_css: event.deltaY * scale, modifiers: modifiers(event), input_sequence: nextInput() } });
    });
  };
  const sendKey = (event: KeyboardEvent<HTMLElement>, kind: "down" | "up"): void => {
    if (event.defaultPrevented || tool !== "browse" || event.target !== event.currentTarget) return;
    event.preventDefault(); onInteractionFocus?.(); void ensureControl().then((controlled) => {
      const current = controlled ? snapshotRef.current : null; const documentContext = current ? context(current) : null;
      if (documentContext) void command({ type: "keyboard", context: documentContext, input: { kind, key: event.key, code: event.code, location: event.location, modifiers: modifiers(event), repeat: event.repeat, input_sequence: nextInput() } });
    });
  };
  const sendText = (event: FormEvent<HTMLTextAreaElement>): void => {
    const text = event.currentTarget.value;
    event.currentTarget.value = "";
    onInteractionFocus?.(); void ensureControl().then((controlled) => {
      const current = controlled ? snapshotRef.current : null; const documentContext = current ? context(current) : null;
      if (documentContext) void command({ type: "text", context: documentContext, input: { text, input_sequence: nextInput() } });
    });
  };
  const sendComposition = (event: CompositionEvent<HTMLElement>, kind: "start" | "update" | "commit"): void => {
    compositionActiveRef.current = kind !== "commit";
    if (kind === "commit") suppressInputRef.current = true;
    if (tool !== "browse") return;
    void ensureControl().then((controlled) => {
      const current = controlled ? snapshotRef.current : null; const documentContext = current ? context(current) : null;
      if (documentContext) void command({ type: "composition", context: documentContext, input: { kind, text: event.data, input_sequence: nextInput() } });
    });
  };
  const clipboard = (event: ClipboardEvent<HTMLTextAreaElement>, copy: boolean): void => {
    if (event.defaultPrevented || tool !== "browse") return;
    event.preventDefault(); onInteractionFocus?.();
    const commandValue = copy ? { type: "copy" as const } : { type: "paste" as const, text: event.clipboardData.getData("text/plain") };
    void ensureControl().then((controlled) => {
      const current = controlled ? snapshotRef.current : null; const documentContext = current ? context(current) : null;
      if (documentContext) void command({ type: "clipboard", context: documentContext, command: commandValue });
    });
  };
  const navigation = (action: "back" | "forward" | "reload" | "stop" | "navigate", address?: string): void => {
    const value = action === "navigate" ? { type: "navigate" as const, url: address ?? "" } : { type: action };
    onInteractionFocus?.(); void ensureControl().then((controlled) => {
      const current = controlled ? snapshotRef.current : null; const documentContext = current ? context(current) : null;
      if (documentContext) void command({ type: "navigation", context: documentContext, command: value });
    });
  };
  const saveNote = (): void => {
    const annotation = draftRef.current?.annotations.find((candidate) => candidate.id === noteId);
    if (!annotation) return; persist({ ...annotation, comment: noteValue.trim() || null }); setNoteId(null); setNoteValue("");
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
  const capture = async (captureAsShown: boolean): Promise<void> => {
    const preparedSnapshot = snapshotRef.current; const where = localLocation(); const preparedDraft = draftRef.current; const beforeFrame = frameRef.current;
    if (!preparedSnapshot || !where || !preparedDraft || !beforeFrame || !preparedSnapshot.document || !preparedSnapshot.viewport) { setMessage("Wait for a confirmed frame and recovered draft before capture."); return; }
    try {
      const prepared = await command({ type: "capture", command: { location: where, draft_id: preparedDraft.draft_id, draft_revision: preparedDraft.revision, annotation_ids: preparedDraft.annotations.map((annotation) => annotation.id), capture_as_shown: captureAsShown } });
      if (!prepared || prepared.type !== "capture_prepared") return;
      const accepted = frameRef.current;
      if (!accepted || accepted.sequence !== prepared.descriptor.frame_sequence || accepted.descriptor.target_id !== prepared.descriptor.target_id || accepted.descriptor.stream_epoch !== prepared.descriptor.stream_epoch || accepted.descriptor.document_generation !== prepared.descriptor.document_generation || accepted.descriptor.viewport_revision !== prepared.descriptor.viewport_revision) { setMessage("Capture pixels changed before composition; retry capture."); return; }
      const png = await pngBase64(preparedDraft.annotations, accepted.descriptor);
      if (!png) { setMessage("Could not compose the captured browser image."); return; }
      const submission: BrowserCaptureSubmission = {
        association_key: preparedSnapshot.identity.association_key, browser_instance: preparedSnapshot.identity.browser_incarnation, capture_id: prepared.capture_id,
        page: { url: preparedSnapshot.navigation?.url ?? "", title: preparedSnapshot.navigation?.title ?? "", tab_id: null, document_id: preparedSnapshot.document.frame_id, captured_at: new Date().toISOString(), viewport: { width: preparedSnapshot.viewport.css_width, height: preparedSnapshot.viewport.css_height, scroll_x: preparedSnapshot.viewport.scroll_x, scroll_y: preparedSnapshot.viewport.scroll_y, device_pixel_ratio: preparedSnapshot.viewport.device_pixel_ratio, visual_scale: preparedSnapshot.viewport.visual_scale }, image_width: accepted.descriptor.image_width, image_height: accepted.descriptor.image_height },
        annotations: preparedDraft.annotations.map((annotation) => ({ id: annotation.id, kind: annotation.kind, comment: annotation.comment ?? "", color: annotation.color, points: annotation.points, bounds: annotation.bounds, element: annotation.evidence })), png_base64: png,
      };
      const provenance: BrowserInlineCaptureProvenance = { target_id: prepared.descriptor.target_id, frame_id: preparedSnapshot.document.frame_id, document_generation: prepared.descriptor.document_generation, frame_generation: preparedSnapshot.document.frame_generation, stream_epoch: prepared.descriptor.stream_epoch, frame_sequence: prepared.descriptor.frame_sequence, viewport_revision: prepared.descriptor.viewport_revision, pixel_captured_at_micros: prepared.descriptor.capture_timestamp_micros, capture_as_shown: captureAsShown };
      const saved = await command({ type: "draft", context: context(preparedSnapshot)!, draft_id: preparedDraft.draft_id, expected_revision: preparedDraft.revision, command: { type: "save_capture", submission, annotation_ids: preparedDraft.annotations.map((annotation) => annotation.id), provenance } });
      if (saved?.type === "capture" && saved.capture.state === "saved") { setMessage(`Capture ${saved.capture.saved.capture_id} saved for feedback.`); onFeedback?.(); }
    } catch (error) {
      setStatus("error"); setMessage(`Could not capture browser image: ${errorMessage(error)}`);
    }
  };
  const annotations = draft?.annotations ?? [];
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
  const blocker = snapshot?.blocker;
  const targetTabs = snapshot?.targets.filter((candidate) => candidate.kind === "page" || candidate.kind === "popup").sort((left, right) => left.order - right.order) ?? [];
  return <section className={["browser-pane", `browser-pane-${status}`, `browser-tool-${tool}`, className].filter(Boolean).join(" ")} aria-label="Browser view">
    <header className="browser-toolbar"><strong>Browser</strong><span className="browser-toolbar-status" role="status" aria-live="polite">{statusText(status, message)}</span><div className="browser-toolbar-actions">{presentation === "browser_only" && onBackToTerminals ? <button type="button" onClick={onBackToTerminals}>Back to terminals</button> : null}{(status === "stale" || status === "error") ? <button type="button" onClick={() => setRetry((value) => value + 1)}>Retry</button> : null}{onHide ? <button type="button" onClick={onHide}>Hide</button> : null}</div></header>
    <div className="browser-tabs" role="tablist" aria-label="Browser tabs">{targetTabs.map((browserTarget) => <div key={browserTarget.target_id} className="browser-tab-wrap"><button type="button" role="tab" aria-selected={browserTarget.target_id === snapshot?.displayed_target_id} title={browserTarget.url} onClick={() => { onInteractionFocus?.(); void command({ type: "tab", command: { type: "select", target_id: browserTarget.target_id } }); }}>{browserTarget.title || browserTarget.url || "New tab"}</button>{browserTarget.can_close ? <button type="button" className="browser-tab-close" aria-label="Close browser tab" onClick={() => void command({ type: "tab", command: { type: "close", target_id: browserTarget.target_id } })}>×</button> : null}</div>)}<button type="button" className="browser-new-tab" aria-label="New browser tab" onClick={() => void command({ type: "tab", command: { type: "create", url: null } })}>+</button></div>
    <div className="browser-navigation"><button type="button" disabled={!snapshot?.navigation?.can_go_back} onClick={() => navigation("back")}>←</button><button type="button" disabled={!snapshot?.navigation?.can_go_forward} onClick={() => navigation("forward")}>→</button><button type="button" disabled={!snapshot} onClick={() => navigation(snapshot?.navigation?.loading ? "stop" : "reload")}>{snapshot?.navigation?.loading ? "■" : "↻"}</button><form onSubmit={(event) => { event.preventDefault(); navigation("navigate", url); }}><input value={url} onFocus={() => { urlEditing.current = true; onInteractionFocus?.(); }} onBlur={() => { urlEditing.current = false; setUrl(snapshotRef.current?.navigation?.url ?? ""); }} onChange={(event) => setUrl(event.target.value)} aria-label="Page URL" placeholder="Enter URL" /></form></div>
    <div className="browser-annotation-toolbar" role="toolbar" aria-label="Annotation tools">{(["browse", "select", "freehand", "element", "region"] as const).map((candidate) => <button key={candidate} type="button" className={tool === candidate ? "is-active" : undefined} aria-pressed={tool === candidate} onClick={() => { setTool(candidate); gestureRef.current = null; setGesture(null); if (candidate !== "browse") onInteractionFocus?.(); }}>{candidate}</button>)}<span className="browser-color-picker">{COLORS.map((candidate) => <button key={candidate} type="button" className={color === candidate ? "is-active" : undefined} aria-label={`Use ${candidate} annotation color`} style={{ background: candidate, borderColor: candidate }} onClick={() => setColor(candidate)} />)}</span><button type="button" disabled={!selectedId} onClick={() => { const current = snapshotRef.current; const documentContext = current ? context(current) : null; const currentDraft = draftRef.current; if (selectedId && documentContext && currentDraft) void command({ type: "draft", context: documentContext, draft_id: currentDraft.draft_id, expected_revision: currentDraft.revision, command: { type: "remove_annotation", annotation_id: selectedId } }); }}>Remove</button><button type="button" aria-expanded={notesOpen} onClick={() => setNotesOpen((open) => !open)}>Notes {annotations.length}</button><button type="button" disabled={!draft || !frame} onClick={() => void capture(false)}>Capture</button>{draft?.freshness === "review_required" ? <button type="button" disabled={!frame} onClick={() => void capture(true)}>Capture as shown</button> : null}<button type="button" onClick={onFeedback}>Feedback</button></div>
      <div ref={surfaceRef} className="browser-surface" style={{ cursor: tool === "browse" ? snapshot?.cursor?.cursor ?? "default" : tool === "select" ? "default" : "crosshair" }} onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerCancel} onWheel={onWheel}>
      <canvas ref={canvasRef} className="browser-frame" aria-label="Live browser frame" />
      <textarea ref={inputRef} className="browser-input-sink" data-browser-input aria-label="Browser keyboard input" onInput={sendText} onKeyDown={(event) => sendKey(event, "down")} onKeyUp={(event) => sendKey(event, "up")} onPaste={(event) => clipboard(event, false)} onCopy={(event) => clipboard(event, true)} onCompositionStart={(event) => sendComposition(event, "start")} onCompositionUpdate={(event) => sendComposition(event, "update")} onCompositionEnd={(event) => sendComposition(event, "commit")} />
      {descriptor ? <svg className="browser-annotation-layer" viewBox={`0 0 ${descriptor.image_width} ${descriptor.image_height}`} preserveAspectRatio="xMidYMid meet" aria-label="Browser annotations">{annotations.map((annotation) => draw(annotation))}{annotations.map(drawLabel)}{transient ? draw(transient, true) : null}{inspection?.bounds && tool === "element" ? (() => { const start = imagePoint({ x: inspection.bounds.x, y: inspection.bounds.y }); const end = imagePoint({ x: inspection.bounds.x + inspection.bounds.width, y: inspection.bounds.y + inspection.bounds.height }); return start && end ? <rect className="browser-element-hover" x={start.x} y={start.y} width={end.x - start.x} height={end.y - start.y} /> : null; })() : null}</svg> : null}
      {!frame ? <div className="browser-state" role={status === "error" || status === "unsupported" ? "alert" : undefined}><strong>{statusText(status, message)}</strong>{status === "loading" ? <span>Waiting for the first confirmed browser frame.</span> : null}</div> : null}
      {frame && (status === "stale" || status === "error" || status === "unsupported") ? <div className="browser-recovery" role="status">{statusText(status, message)}</div> : null}
      {blocker ? <div className="browser-blocker" role="alert"><strong>{blocker.message}</strong>{blocker.kind === "dialog" ? <div><button type="button" onClick={() => void command({ type: "dialog", blocker_id: blocker.blocker_id, command: { type: "accept", text: blocker.default_prompt } })}>Accept</button>{blocker.cancellable ? <button type="button" onClick={() => void command({ type: "dialog", blocker_id: blocker.blocker_id, command: { type: "dismiss" } })}>Dismiss</button> : null}</div> : blocker.kind === "download" ? <div><button type="button" onClick={() => void command({ type: "download", blocker_id: blocker.blocker_id, command: { type: "accept" } })}>Save download</button><button type="button" onClick={() => void command({ type: "download", blocker_id: blocker.blocker_id, command: { type: "cancel" } })}>Cancel</button></div> : blocker.kind === "permission" ? <div><button type="button" onClick={() => void command({ type: "permission", blocker_id: blocker.blocker_id, command: { decision: "allow" } })}>Allow</button><button type="button" onClick={() => void command({ type: "permission", blocker_id: blocker.blocker_id, command: { decision: "deny" } })}>Deny</button></div> : blocker.kind === "file_chooser" ? <button type="button" onClick={() => void command({ type: "file", blocker_id: blocker.blocker_id, command: { type: "cancel" } })}>Cancel file chooser</button> : null}</div> : null}
      {noteId ? <div className="browser-note-editor"><textarea autoFocus value={noteValue} onChange={(event) => setNoteValue(event.target.value)} maxLength={4000} aria-label="Annotation note" onKeyDown={(event) => { if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) { event.preventDefault(); saveNote(); } if (event.key === "Escape") { setNoteId(null); setNoteValue(""); } }} /><div><button type="button" onClick={saveNote}>Save</button><button type="button" onClick={() => { setNoteId(null); setNoteValue(""); }}>Cancel</button></div></div> : null}
    </div>
    {notesOpen ? <aside className="browser-notes" aria-label="Annotation notes"><header><strong>Notes</strong><button type="button" onClick={() => setNotesOpen(false)}>Close</button></header>{annotations.length === 0 ? <p>No annotations yet.</p> : <ul>{annotations.map((annotation, index) => <li key={annotation.id}><button type="button" className={selectedId === annotation.id ? "is-selected" : undefined} onClick={() => setSelectedId(annotation.id)}>{index + 1}. {kindFor(annotation)}{annotation.comment ? ` — ${annotation.comment}` : ""}</button><button type="button" onClick={() => { setNoteId(annotation.id); setNoteValue(annotation.comment ?? ""); }}>+ Text</button></li>)}</ul>}<button type="button" disabled={annotations.length === 0} onClick={() => { const current = snapshotRef.current; const documentContext = current ? context(current) : null; const currentDraft = draftRef.current; if (documentContext && currentDraft) void command({ type: "draft", context: documentContext, draft_id: currentDraft.draft_id, expected_revision: currentDraft.revision, command: { type: "clear" } }); }}>Clear annotations</button></aside> : null}
  </section>;
}
