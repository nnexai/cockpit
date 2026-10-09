import type { BrowserFeedbackSendResponse, BrowserTarget, BrowserViewDraftAnnotation, BrowserViewViewportRequest } from "../../protocol/generated/v1";
import type { CockpitClient } from "../../client/CockpitClient";
import { addressBarUrl, rectFrom, statusText } from "./browserPaneModel";
import { AnnotationLayer, BrowserAnnotationToolbar, BrowserBlockerPanel, BrowserFrameSurface, BrowserNavigationBar, BrowserNoteEditor, BrowserTabStrip } from "./BrowserChrome";
import { BrowserFeedbackPanel } from "./BrowserFeedbackPanel";
import { useBrowserPaneState } from "./useBrowserPaneState";
import { useBrowserFrameGeometry, useBrowserViewCommand } from "./useBrowserViewCommand";
import { useBrowserInputQueue } from "./useBrowserInputQueue";
import { useBrowserDrafts } from "./useBrowserDrafts";
import { useBrowserAnnotationMutations } from "./useBrowserAnnotationMutations";
import { useBrowserViewStream, useBrowserViewportResize } from "./useBrowserViewStream";
import { useBrowserElementInspection, useBrowserPointerInput } from "./useBrowserPointerInput";
import { useBrowserPageInput } from "./useBrowserPageInput";
import { useBrowserNavigation } from "./useBrowserNavigation";
import { useBrowserFeedbackCapture } from "./useBrowserFeedbackCapture";
import { useBrowserNoteEditor } from "./useBrowserNoteEditor";
import "./browser.css";

export interface BrowserPaneProps {
  client: CockpitClient; target: BrowserTarget; viewport: BrowserViewViewportRequest;
  visible?: boolean; clientId?: string;
  inputActive?: boolean; liveInputEnabled?: boolean; onInteractionFocus?: () => void; onFeedback?: (captureIds: string[], operationId: string, acknowledgeDuplicateRisk: boolean) => Promise<BrowserFeedbackSendResponse>;
  onReconnect?: () => void | Promise<void>;
  onCloseBrowser?: () => void | Promise<void>;
  className?: string;
}

export function BrowserPane({ client, target, viewport, visible = true, clientId, inputActive = true, liveInputEnabled = true, onInteractionFocus, onFeedback, onReconnect, onCloseBrowser, className }: BrowserPaneProps) {
  const state = useBrowserPaneState({ target, visible, liveInputEnabled });
  const geometry = useBrowserFrameGeometry(state, viewport);
  const view = useBrowserViewCommand(state, geometry);
  const input = useBrowserInputQueue(state, view);
  const drafts = useBrowserDrafts(state, view, client);
  const marks = useBrowserAnnotationMutations(state, view, drafts, client);
  useBrowserViewStream(state, geometry, input, drafts, { client, target, clientId, visible, liveInputEnabled });
  useBrowserViewportResize(state, geometry, view, input, { viewport, visible });
  const elementInspection = useBrowserElementInspection(state, geometry, view);
  const pointerHandlers = useBrowserPointerInput(state, geometry, view, input, marks, elementInspection, onInteractionFocus);
  const pageHandlers = useBrowserPageInput(state, geometry, view, input, marks, { inputActive, onInteractionFocus });
  const { navigation, tabCommand, reconnectView } = useBrowserNavigation(state, view, input, drafts, onReconnect);
  const feedback = useBrowserFeedbackCapture(state, geometry, view, drafts);
  const editor = useBrowserNoteEditor(state, drafts, marks);
  const { snapshot, draft, frame, gesture, tool, color, inspection, selectedId, status, message, annotationNotice, pendingCapture, noteId, noteValue, noteEditorDismissed, url, setUrl, urlEditing, snapshotRef, gestureRef, setGesture, setTool, setColor, associationOwner, ownerKey, workScope, feedbackPanelRef, surfaceRef, canvasRef, setPendingCaptureState, setMessage } = state;
  const { discardDraft } = drafts;
  const { removeAnnotation, retryAnnotationMutations, discardAnnotationMutations } = marks;
  const { command } = view;

  const draftMatchesSnapshot = Boolean(draft && snapshot?.displayed_target_id === draft.target_id && snapshot.document?.document_generation === draft.document_generation);
  const annotations = draftMatchesSnapshot ? (draft?.annotations ?? []) : [];
  const descriptor = frame?.descriptor;
  const transient = gesture && (tool === "freehand" || tool === "region") ? { id: "transient", kind: tool === "freehand" ? "freehand" : "region", color, points: gesture.points, bounds: tool === "region" ? rectFrom(gesture.origin, gesture.points[gesture.points.length - 1]) : null, evidence: null, comment: null } satisfies BrowserViewDraftAnnotation : null;
  const inspectionBounds = inspection?.bounds && descriptor ? { ...inspection.bounds, x: inspection.bounds.x + descriptor.scroll_x, y: inspection.bounds.y + descriptor.scroll_y } : null;
  const blocker = snapshot?.blocker;
  const targetTabs = snapshot?.targets.filter((candidate) => candidate.kind === "page" || candidate.kind === "popup").sort((left, right) => left.order - right.order) ?? [];
  const closableTabs = targetTabs.filter((candidate) => candidate.can_close);
  const selectedAnnotation = annotations.find((annotation) => annotation.id === selectedId) ?? null;
  return <section className={["browser-pane", `browser-pane-${status}`, `browser-tool-${tool}`, className].filter(Boolean).join(" ")} aria-label="Browser view">
    <BrowserTabStrip
      tabs={targetTabs}
      displayedTargetId={snapshot?.displayed_target_id}
      status={status}
      message={message}
      onSelect={(targetId) => { onInteractionFocus?.(); tabCommand({ type: "tab", command: { type: "select", target_id: targetId } }); }}
      onClose={(targetId) => {
        if (closableTabs.length === 1 && onCloseBrowser) {
          onInteractionFocus?.();
          void onCloseBrowser();
          return;
        }
        tabCommand({ type: "tab", command: { type: "close", target_id: targetId } });
      }}
      onCreate={() => tabCommand({ type: "tab", command: { type: "create", url: null } })}
      onRetry={() => void reconnectView()}
    />
    <div className="browser-controls">
      <BrowserNavigationBar
        navigationState={snapshot?.navigation}
        available={Boolean(snapshot)}
        url={url}
        onNavigate={navigation}
        onUrlChange={setUrl}
        onUrlFocus={() => { urlEditing.current = true; onInteractionFocus?.(); }}
        onUrlBlur={() => { urlEditing.current = false; setUrl(addressBarUrl(snapshotRef.current?.navigation?.url)); }}
      />
      <BrowserAnnotationToolbar
        tool={tool} color={color} hasDraft={Boolean(draft)} hasSelection={Boolean(selectedAnnotation)}
        annotationCount={annotations.length} retainedMutationCount={associationOwner.pendingAnnotationMutations.length}
        pendingCapture={Boolean(pendingCapture)} canCapture={Boolean(draft && frame && annotations.length > 0)}
        onToolChange={(candidate) => { setTool(candidate); gestureRef.current = null; setGesture(null); if (candidate !== "browse") onInteractionFocus?.(); }}
        onColorChange={setColor}
        onRemove={(ctrlKey) => { if (ctrlKey) void discardDraft(); else if (selectedId) removeAnnotation(selectedId); }}
        onEditNote={() => { if (selectedAnnotation) editor.openNoteEditor(selectedAnnotation); }}
        onRetryMutations={() => void retryAnnotationMutations()}
        onDiscardMutations={() => void discardAnnotationMutations()}
        onCapture={() => void feedback.capture(false)}
      />
    </div>
    <BrowserFeedbackPanel key={ownerKey} ref={feedbackPanelRef} client={client} scope={workScope} onSend={onFeedback} pendingCapture={pendingCapture}
      onPendingFeedbackChange={feedback.onPendingFeedbackChange}
      onPendingCaptureChange={setPendingCaptureState}
      onMessage={setMessage}
      beforeRecovery={feedback.beforeRecovery}
      onRecovered={feedback.onRecovered}
    />
    <BrowserFrameSurface surfaceRef={surfaceRef} canvasRef={canvasRef}
      cursor={tool === "browse" ? snapshot?.cursor?.cursor ?? "default" : tool === "select" ? "default" : "crosshair"}
      handlers={{ ...pointerHandlers, ...pageHandlers }}>
    {descriptor ? <AnnotationLayer descriptor={descriptor} annotations={annotations} transient={transient} selectedId={selectedId} tool={tool} inspectionBounds={inspectionBounds} onSelect={editor.selectAnnotation} /> : null}
    {frame && (status === "error" || status === "unsupported") ? <div className="browser-recovery" role="status">{statusText(status, message)}</div> : null}
    {annotationNotice && status !== "error" && status !== "unsupported" ? <div className="browser-recovery" role="alert">{annotationNotice}</div> : null}
    {blocker ? <BrowserBlockerPanel blocker={blocker} onCommand={(value) => void command(value)} /> : null}
    {noteId && selectedId === noteId && !noteEditorDismissed ? <BrowserNoteEditor editorRef={editor.noteEditorRef} style={editor.noteEditorStyle} value={noteValue} onChange={editor.changeNote} onSave={editor.saveNote} onDismiss={editor.dismissNoteEditor} /> : null}
    </BrowserFrameSurface>
  </section>;
}
