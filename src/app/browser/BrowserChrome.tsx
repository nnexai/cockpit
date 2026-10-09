// Stateless browser chrome and saved-feedback receipt controls.
import type { CSSProperties, ReactNode, RefObject } from "react";
import type { BrowserPoint, BrowserRect, BrowserViewCommand, BrowserViewDraftAnnotation, BrowserViewSnapshot } from "../../protocol/generated/v1";
import type { BrowserViewFramePacket } from "../../client/CockpitClient";
import { UiIcon } from "../UiIcon";
import { imagePointFor, kindFor, statusText, type PaneStatus, type SavedDelivery, type Tool } from "./browserPaneModel";
import { AnnotationIcon, AnnotationToolButtons, BrowserColorPicker } from "./AnnotationControls";
import type { BrowserPointerHandlers } from "./useBrowserPointerInput";
import type { BrowserPageHandlers } from "./useBrowserPageInput";

type BrowserTargetTab = BrowserViewSnapshot["targets"][number];

export function BrowserTabStrip({ tabs, displayedTargetId, status, message, onSelect, onClose, onCreate, onRetry }: {
  tabs: BrowserTargetTab[];
  displayedTargetId: string | null | undefined;
  status: PaneStatus;
  message: string | null;
  onSelect: (targetId: string) => void;
  onClose: (targetId: string) => void;
  onCreate: () => void;
  onRetry: () => void;
}) {
  const statusLabel = statusText(status, message);
  return <header className="browser-toolbar">
    <div className="browser-tabs" role="tablist" aria-label="Browser tabs">
      {tabs.map((tab) => {
        const tabName = tab.title || tab.url || "New tab";
        return <div key={tab.target_id} className="browser-tab-wrap">
          <button type="button" role="tab" aria-selected={tab.target_id === displayedTargetId} title={tab.url} onClick={() => onSelect(tab.target_id)}>{tabName}</button>
          {tab.can_close
            ? <button type="button" className="browser-tab-close browser-toolbar-icon" aria-label={`Close ${tabName}`} title={`Close ${tabName}`} onClick={(event) => { event.stopPropagation(); onClose(tab.target_id); }}>
              <UiIcon name="close" />
            </button>
            : null}
        </div>;
      })}
      <button type="button" className="browser-new-tab browser-toolbar-icon" aria-label="New browser tab" title="New browser tab" onClick={onCreate}><UiIcon name="plus" /></button>
    </div>
    <div className="browser-toolbar-actions">
      <span className={`browser-toolbar-status is-${status}`} role="status" aria-live="polite" title={statusLabel}>{statusLabel}</span>
      {status === "error" ? <button type="button" onClick={onRetry}>Retry</button> : null}
    </div>
  </header>;
}

const deliveryStateLabel = (state: SavedDelivery["state"]): string =>
  state === "outcome_unknown" ? "Outcome unknown" : state === "rejected" ? "Rejected" : state === "accepted" ? "Accepted" : "Pending";

export function SavedDeliveryList({ deliveries, selectedCaptureId, duplicateRisk, sendDisabled = false, busy = false, onSelect, onDuplicateRiskChange, onResolveDuplicateRisk, onAcknowledge, onRetry }: {
  deliveries: SavedDelivery[];
  selectedCaptureId: string | null;
  duplicateRisk: boolean;
  sendDisabled?: boolean;
  busy?: boolean;
  onSelect: (captureId: string) => void;
  onDuplicateRiskChange: (checked: boolean) => void;
  onResolveDuplicateRisk: () => void;
  onAcknowledge: (captureId: string) => void;
  onRetry: (captureId: string) => void;
}) {
  return <div className="browser-capture-pending" aria-label="Saved feedback recovery">
    {deliveries.map((item, index) => {
      const selected = selectedCaptureId === item.capture_id;
      const unknownAndSelected = item.state === "outcome_unknown" && selected && !item.blocked;
      return <div key={item.capture_id}>
        <button type="button" aria-pressed={selected} aria-label={`Select saved capture ${index + 1}`} onClick={() => onSelect(item.capture_id)}>Saved capture {index + 1} · {deliveryStateLabel(item.state)}</button>
        <span role="status">{item.message}</span>
        {unknownAndSelected
          ? <label><input type="checkbox" checked={duplicateRisk} onChange={(event) => onDuplicateRiskChange(event.target.checked)} /> I checked the destination; a new operation may duplicate feedback.</label>
          : null}
        {unknownAndSelected ? <button type="button" disabled={!duplicateRisk || sendDisabled || busy} onClick={onResolveDuplicateRisk}>Resolve and retry</button> : null}
        {item.state === "accepted" ? <button type="button" disabled={item.blocked || busy} onClick={() => onAcknowledge(item.capture_id)}>Acknowledge saved receipt</button> : null}
        {item.state !== "accepted" && item.state !== "outcome_unknown"
          ? <button type="button" disabled={item.blocked || sendDisabled || busy} aria-label={`Retry saved capture ${index + 1}`} onClick={() => onRetry(item.capture_id)}>Retry saved feedback</button>
          : null}
      </div>;
    })}
  </div>;
}

export function AnnotationLayer({ descriptor, annotations, transient, selectedId, tool, inspectionBounds, onSelect }: {
  descriptor: BrowserViewFramePacket["descriptor"];
  annotations: BrowserViewDraftAnnotation[];
  transient: BrowserViewDraftAnnotation | null;
  selectedId: string | null;
  tool: Tool;
  inspectionBounds: BrowserRect | null;
  onSelect: (annotation: BrowserViewDraftAnnotation) => void;
}) {
  const imagePoint = (point: BrowserPoint) => imagePointFor(descriptor, point);
  const imageRect = (bounds: BrowserRect) => {
    const start = imagePoint({ x: bounds.x, y: bounds.y });
    const end = imagePoint({ x: bounds.x + bounds.width, y: bounds.y + bounds.height });
    return start && end ? { x: start.x, y: start.y, width: end.x - start.x, height: end.y - start.y } : null;
  };
  const draw = (annotation: BrowserViewDraftAnnotation, isTransient = false) => {
    const annotationKind = kindFor(annotation);
    const className = `browser-annotation browser-annotation-${annotationKind}${annotation.id === selectedId ? " is-selected" : ""}${isTransient ? " is-transient" : ""}`;
    const onPointerDown = (event: { stopPropagation: () => void }) => { if (tool === "select") { event.stopPropagation(); onSelect(annotation); } };
    if (annotationKind === "region" || annotationKind === "element") {
      const rect = annotation.bounds ? imageRect(annotation.bounds) : null;
      return rect ? <rect key={annotation.id} className={className} {...rect} stroke={annotation.color} onPointerDown={onPointerDown} /> : null;
    }
    const points = annotation.points.map(imagePoint).filter((point): point is BrowserPoint => point !== null);
    return points.length > 1
      ? <polyline key={annotation.id} className={className} points={points.map((point) => `${point.x},${point.y}`).join(" ")} stroke={annotation.color} onPointerDown={onPointerDown} />
      : null;
  };
  const drawLabel = (annotation: BrowserViewDraftAnnotation) => {
    if (!annotation.comment?.trim()) return null;
    const anchor = annotation.bounds ? { x: annotation.bounds.x, y: annotation.bounds.y } : annotation.points[0];
    const point = anchor ? imagePoint(anchor) : null;
    return point
      ? <text key={`${annotation.id}-label`} className="browser-annotation-label" x={point.x + 8} y={point.y + 16} fill={annotation.color}>{annotation.comment.trim().replace(/\s+/g, " ").slice(0, 240)}</text>
      : null;
  };
  const hover = inspectionBounds && tool === "element" ? imageRect(inspectionBounds) : null;
  return <svg className="browser-annotation-layer" viewBox={`0 0 ${descriptor.image_width} ${descriptor.image_height}`} preserveAspectRatio="none" aria-label="Browser annotations">
    {annotations.map((annotation) => draw(annotation))}
    {annotations.map(drawLabel)}
    {transient ? draw(transient, true) : null}
    {hover ? <rect className="browser-element-hover" {...hover} /> : null}
  </svg>;
}

export type NavigationAction = "back" | "forward" | "reload" | "stop" | "navigate";

export function BrowserNavigationBar({ navigationState, available, url, onNavigate, onUrlChange, onUrlFocus, onUrlBlur }: {
  navigationState: BrowserViewSnapshot["navigation"] | undefined;
  available: boolean;
  url: string;
  onNavigate: (action: NavigationAction, address?: string) => void;
  onUrlChange: (value: string) => void;
  onUrlFocus: () => void;
  onUrlBlur: () => void;
}) {
  const loading = Boolean(navigationState?.loading);
  return <div className="browser-navigation">
    <button type="button" className="browser-toolbar-icon" aria-label="Back" title="Back" disabled={!navigationState?.can_go_back} onClick={() => onNavigate("back")}><UiIcon name="back" /></button>
    <button type="button" className="browser-toolbar-icon" aria-label="Forward" title="Forward" disabled={!navigationState?.can_go_forward} onClick={() => onNavigate("forward")}><UiIcon name="forward" /></button>
    <button type="button" className="browser-toolbar-icon" aria-label={loading ? "Stop loading" : "Reload"} title={loading ? "Stop loading" : "Reload"} disabled={!available} onClick={() => onNavigate(loading ? "stop" : "reload")}>
      <UiIcon name={loading ? "stop" : "refresh"} />
    </button>
    <form onSubmit={(event) => { event.preventDefault(); onNavigate("navigate", url); }}>
      <input value={url} onFocus={onUrlFocus} onBlur={onUrlBlur} onChange={(event) => onUrlChange(event.target.value)} aria-label="Page URL" placeholder="Enter URL" />
    </form>
  </div>;
}

export function BrowserAnnotationToolbar({ tool, color, hasDraft, hasSelection, annotationCount, retainedMutationCount, pendingCapture, canCapture, onToolChange, onColorChange, onRemove, onEditNote, onRetryMutations, onDiscardMutations, onCapture }: {
  tool: Tool; color: string; hasDraft: boolean; hasSelection: boolean; annotationCount: number;
  retainedMutationCount: number; pendingCapture: boolean; canCapture: boolean;
  onToolChange(tool: Tool): void; onColorChange(color: string): void; onRemove(ctrlKey: boolean): void;
  onEditNote(): void; onRetryMutations(): void; onDiscardMutations(): void; onCapture(): void;
}) {
  return <div className="browser-annotation-toolbar" role="toolbar" aria-label="Annotation tools">
    <AnnotationToolButtons tool={tool} onChange={onToolChange} />
    <BrowserColorPicker color={color} onChange={onColorChange} />
    <button type="button" disabled={!hasDraft} aria-label="Remove selected annotation or Control-click to discard draft" title="Remove selected annotation · Control-click to discard draft" onClick={(event) => onRemove(event.ctrlKey)}><AnnotationIcon name="remove" /></button>
    <button type="button" disabled={!hasSelection} aria-label="Edit selected annotation note" title="Edit selected annotation note" onClick={onEditNote}><AnnotationIcon name="notes" /></button>
    <span className="browser-annotation-notes" aria-label={`Notes ${annotationCount}`} title={`Notes ${annotationCount}`}>{annotationCount}</span>
    {retainedMutationCount > 0
      ? <>
        <span className="browser-capture-pending" role="status">Retained annotation changes need review; unknown delivery is not replayed automatically.</span>
        <button type="button" aria-label="Retry retained annotation changes" title="Retry retained annotation changes" onClick={onRetryMutations}>Retry saves</button>
        <button type="button" aria-label="Discard retained annotation changes" title="Discard retained annotation changes" onClick={onDiscardMutations}>Discard retry intent</button>
      </>
      : null}
    {!pendingCapture ? <button type="button" className="browser-send-annotations" disabled={!canCapture} aria-label="Send annotations" title="Send annotations" onClick={onCapture}><AnnotationIcon name="feedback" /></button> : null}
  </div>;
}

export function BrowserBlockerPanel({ blocker, onCommand }: { blocker: NonNullable<BrowserViewSnapshot["blocker"]>; onCommand(value: BrowserViewCommand): void }) {
  return <div className="browser-blocker" role="alert"><strong>{blocker.message}</strong>{blocker.kind === "dialog" ? <div><button type="button" onClick={() => onCommand({ type: "dialog", blocker_id: blocker.blocker_id, command: { type: "accept", text: blocker.default_prompt } })}>Accept</button>{blocker.cancellable ? <button type="button" onClick={() => onCommand({ type: "dialog", blocker_id: blocker.blocker_id, command: { type: "dismiss" } })}>Dismiss</button> : null}</div> : blocker.kind === "download" ? <div><button type="button" onClick={() => onCommand({ type: "download", blocker_id: blocker.blocker_id, command: { type: "accept" } })}>Save download</button><button type="button" onClick={() => onCommand({ type: "download", blocker_id: blocker.blocker_id, command: { type: "cancel" } })}>Cancel</button></div> : blocker.kind === "permission" ? <div><button type="button" onClick={() => onCommand({ type: "permission", blocker_id: blocker.blocker_id, command: { decision: "allow" } })}>Allow</button><button type="button" onClick={() => onCommand({ type: "permission", blocker_id: blocker.blocker_id, command: { decision: "deny" } })}>Deny</button></div> : blocker.kind === "file_chooser" ? <button type="button" onClick={() => onCommand({ type: "file", blocker_id: blocker.blocker_id, command: { type: "cancel" } })}>Cancel file chooser</button> : null}</div>;
}

export function BrowserNoteEditor({ editorRef, style, value, onChange, onSave, onDismiss }: {
  editorRef: RefObject<HTMLDivElement | null>; style: CSSProperties | undefined; value: string;
  onChange(value: string): void; onSave(): void; onDismiss(): void;
}) {
  return <div ref={editorRef} className="browser-note-editor" style={style}><textarea autoFocus value={value} onChange={(event) => onChange(event.target.value)} maxLength={4000} aria-label="Annotation note" onKeyDown={(event) => { if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) { event.preventDefault(); onSave(); } if (event.key === "Escape") { event.preventDefault(); onDismiss(); } }} /><div><button type="button" onClick={onSave}>Save</button><button type="button" onClick={onDismiss}>Close</button></div></div>;
}

export function BrowserFrameSurface({ surfaceRef, canvasRef, cursor, handlers, children }: {
  surfaceRef: RefObject<HTMLDivElement | null>; canvasRef: RefObject<HTMLCanvasElement | null>;
  cursor: CSSProperties["cursor"]; handlers: BrowserPointerHandlers & BrowserPageHandlers; children: ReactNode;
}) {
  return <div ref={surfaceRef} className="browser-surface" tabIndex={0} style={{ cursor }} {...handlers}>
    <canvas ref={canvasRef} className="browser-frame" aria-label="Live browser frame" />
    {children}
  </div>;
}
