// Pure types and helpers for the browser pane: input intents, draft identity,
// status text and geometry. Nothing here touches React state.
import type { BrowserFeedbackDeliveryStatus, BrowserPoint, BrowserRect, BrowserTarget, BrowserViewDraftAnnotation, BrowserViewDraftState, BrowserViewLocation, BrowserViewSnapshot } from "../../protocol/generated/v1";
import type { BrowserViewFramePacket } from "../../client/CockpitClient";

export type PaneStatus = "hidden" | "loading" | "ready" | "stale" | "error" | "unsupported" | "empty";
export type Tool = "browse" | "select" | "freehand" | "region" | "element";
export type Gesture = { pointerId: number; points: BrowserPoint[]; origin: BrowserPoint; frame: number; document: number; viewport: number };
export type WheelIntent = { clientX: number; clientY: number; deltaX: number; deltaY: number; modifiers: number };
export type InputJob = { generation: number; kind: "move" | "wheel" | "boundary" | "release"; run: () => Promise<void>; wheel?: WheelIntent };
export type PointerIntent = {
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
export type ElementInspectionIntent = {
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
export type DraftAssociationOwner = {
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
  retiredDraftIds: Set<string>;
  retiredDocumentKeys: Set<string>;
};
export type PendingAnnotationMutation = {
  key: string;
  kind: "upsert" | "remove";
  draftId: string;
  expectedRevision: number;
  annotation: BrowserViewDraftAnnotation | null;
  annotationId: string;
};
export type CaptureIdentity = {
  owner: DraftAssociationOwner;
  draftId: string;
  draftRevision: number;
  editorGeneration: number;
  noteId: string | null;
  noteText: string;
  notesOpen: boolean;
};
export type SavedDelivery = {
  capture_id: string;
  ids: string[];
  operation_id: string;
  state: DeliveryState;
  message: string;
  blocked: boolean;
  hasReceipt: boolean;
};
export const MAX_INPUT_JOBS = 128;
export const deliveryOperationId = (captureId: string): string => `browser-feedback-${captureId}`;
export type DeliveryState = BrowserFeedbackDeliveryStatus["state"];
export const MAX_ANNOTATION_POINTS = 8_192; // Must not exceed BrowserView's draft point limit.
const TOLERANCE = 1.5;
export const errorMessage = (error: unknown): string => error instanceof Error && error.message ? error.message : "The browser view stream is unavailable.";
export const newId = (prefix: string): string => `${prefix}-${globalThis.crypto?.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`}`;
export const annotationId = (): string => globalThis.crypto?.randomUUID?.() ?? "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (character) => {
  const value = Math.floor(Math.random() * 16);
  return (character === "x" ? value : (value & 0x3) | 0x8).toString(16);
});
export const statusFor = (snapshot: BrowserViewSnapshot): PaneStatus => snapshot.blocker?.kind === "unsupported" ? "unsupported" : snapshot.displayed_target_id ? "loading" : "empty";
export const statusText = (status: PaneStatus, message: string | null): string => {
  if (status === "ready") return "Live browser view";
  if (status === "loading") return "Loading browser view…";
  if (status === "stale") return message ?? "The browser rejected a stale input; it was not replayed.";
  if (status === "empty") return "No browser page is selected";
  if (status === "hidden") return "Browser view hidden";
  return message ?? (status === "unsupported" ? "Browser view is unsupported by this runtime" : "Browser stream error");
};
/** How long the view may lag behind dropped frames or refused input before the pane says so. */
export const LAG_NOTICE_DELAY_MS = 2_000;
export const FRAME_BEHIND_MESSAGE = "Browser view is behind: newer page frames could not be shown.";
/**
 * Holds back a transient problem (a dropped frame, a refused input) until it
 * has gone unresolved for a while. A newer frame or the next accepted input
 * resolves it, so a problem that fixes itself never reaches the pane.
 */
export class DelayedNotice {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private message: string | null = null;
  private shown = false;
  constructor(
    private readonly show: (message: string) => void,
    private readonly clear: (message: string) => void,
    private readonly delayMs = LAG_NOTICE_DELAY_MS,
  ) {}
  report(message: string): void {
    this.message = message;
    if (this.shown) { this.show(message); return; }
    if (this.timer !== null) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      if (this.message === null) return;
      this.shown = true;
      this.show(this.message);
    }, this.delayMs);
  }
  resolve(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    const message = this.message;
    this.message = null;
    if (this.shown && message !== null) { this.shown = false; this.clear(message); }
  }
  dispose(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    this.message = null;
    this.shown = false;
  }
}
export const button = (value: number): "left" | "middle" | "right" | null => value === 0 ? "left" : value === 1 ? "middle" : value === 2 ? "right" : null;
export const modifiers = (event: { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }): number => (event.altKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.metaKey ? 4 : 0) | (event.shiftKey ? 8 : 0);
export const isLocalBrowserChrome = (target: EventTarget | null): boolean =>
  target instanceof Element && target.closest(".browser-note-editor, .browser-blocker, .browser-recovery") !== null;
// The helper's blank start page is an implementation detail; show an empty address bar.
export const addressBarUrl = (value: string | null | undefined): string => {
  if (!value) return "";
  try { return new URL(value).pathname === "/__cockpit_browser_start__" ? "" : value; } catch { return value; }
};
export const navigationUrl = (value: string): string => {
  const trimmed = value.trim();
  return /^[a-z][a-z\d+.-]*:/i.test(trimmed) ? trimmed : `https://${trimmed}`;
};
export const rectFrom = (first: BrowserPoint, last: BrowserPoint): BrowserRect => ({ x: Math.min(first.x, last.x), y: Math.min(first.y, last.y), width: Math.abs(last.x - first.x), height: Math.abs(last.y - first.y) });
export const kindFor = (annotation: BrowserViewDraftAnnotation): "freehand" | "region" | "element" => annotation.kind;
// Native serialization can round a coordinate by one ULP (for example,
// 99.99999999999999 to 100). A durable draft still acknowledges the same mark.
const sameDraftNumber = (a: number, b: number): boolean =>
  Number.isFinite(a) && Number.isFinite(b)
  && Math.abs(a - b) <= 2 * Number.EPSILON * Math.max(1, Math.abs(a), Math.abs(b));
const sameDraftPoint = (a: BrowserPoint, b: BrowserPoint): boolean => sameDraftNumber(a.x, b.x) && sameDraftNumber(a.y, b.y);
export function sameDraftAnnotation(actual: BrowserViewDraftAnnotation, requested: BrowserViewDraftAnnotation): boolean {
  const a = actual.bounds; const b = requested.bounds;
  const boundsMatch = a === null || b === null
    ? a === b
    : sameDraftNumber(a.x, b.x) && sameDraftNumber(a.y, b.y) && sameDraftNumber(a.width, b.width) && sameDraftNumber(a.height, b.height);
  const x = actual.evidence; const y = requested.evidence;
  const evidenceMatches = x === null || y === null
    ? x === y
    : x.tag === y.tag && x.text === y.text && x.role === y.role && x.name === y.name
      && x.excerpt === y.excerpt && x.locators.length === y.locators.length
      && x.locators.every((locator, index) => locator === y.locators[index]);
  return actual.id === requested.id && actual.kind === requested.kind && actual.color === requested.color
    && actual.comment === requested.comment && boundsMatch && evidenceMatches
    && actual.points.length === requested.points.length
    && actual.points.every((point, index) => sameDraftPoint(point, requested.points[index]));
}
export function simplify(points: BrowserPoint[]): BrowserPoint[] {
  if (points.length < 3) return points;
  const kept = new Uint8Array(points.length); kept[0] = 1; kept[points.length - 1] = 1;
  const squared = (point: BrowserPoint, from: BrowserPoint, to: BrowserPoint): number => {
    const dx = to.x - from.x; const dy = to.y - from.y;
    const ratio = dx === 0 && dy === 0 ? 0 : Math.max(0, Math.min(1, ((point.x - from.x) * dx + (point.y - from.y) * dy) / (dx * dx + dy * dy)));
    const x = point.x - from.x - ratio * dx; const y = point.y - from.y - ratio * dy; return x * x + y * y;
  };
  const ranges = [0, points.length - 1];
  while (ranges.length) {
    const to = ranges.pop()!; const from = ranges.pop()!;
    let maximum = TOLERANCE * TOLERANCE; let index = -1;
    for (let candidate = from + 1; candidate < to; candidate += 1) {
      const distance = squared(points[candidate], points[from], points[to]);
      if (distance > maximum) { maximum = distance; index = candidate; }
    }
    if (index >= 0) { kept[index] = 1; ranges.push(from, index, index, to); }
  }
  return points.filter((_, index) => kept[index] === 1);
}
export function context(snapshot: BrowserViewSnapshot) {
  return snapshot.document && snapshot.displayed_target_id ? { target_id: snapshot.displayed_target_id, document_generation: snapshot.document.document_generation, lease_generation: snapshot.control.lease_generation } : null;
}
export function location(snapshot: BrowserViewSnapshot, frame: BrowserViewFramePacket["descriptor"]): BrowserViewLocation | null {
  return snapshot.document && snapshot.viewport && snapshot.displayed_target_id ? { target_id: snapshot.displayed_target_id, document_generation: snapshot.document.document_generation, viewport_revision: frame.viewport_revision, presented_frame_sequence: frame.frame_sequence, lease_generation: snapshot.control.lease_generation } : null;
}
export const retiredDraft = (owner: DraftAssociationOwner, draft: BrowserViewDraftState): boolean =>
  owner.retiredDraftIds.has(draft.draft_id);
export const retiredDocumentKey = (targetId: string, generation: number): string => `${targetId}:${generation}`;
export const ownsBrowserControl = (snapshot: BrowserViewSnapshot | null): boolean =>
  snapshot?.control.status === "controlled" && snapshot.control.controller_view_id === snapshot.identity.view_id;
/** Map a CSS-pixel page point to the presented frame image, or null outside a finite mapping. */
export function imagePointFor(descriptor: BrowserViewFramePacket["descriptor"], point: BrowserPoint): BrowserPoint | null {
  const x = (point.x - descriptor.scroll_x - descriptor.viewport_offset_x) / descriptor.viewport_css_width * descriptor.image_width;
  const y = (point.y - descriptor.scroll_y - descriptor.viewport_offset_y) / descriptor.viewport_css_height * descriptor.image_height;
  return Number.isFinite(x) && Number.isFinite(y) ? { x, y } : null;
}
