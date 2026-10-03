import { useCallback, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type SyntheticEvent } from "react";
import type { ViewerContext } from "../../protocol/generated/v1";
import { ContextViewer, createContextViewState, type ContextViewState } from "../context/ContextViewer";
import type { LeafCtx, ViewerSlot } from "./tabLayoutStore";
import { runtimeSource } from "./reconcile";
import { closeViewerLeaf, openViewerLeaf, viewerErrorCode, viewerErrorMessage, VIEWER_MISSING_MESSAGE } from "./viewerLifecycle";

export type ViewerLeafProps = { ctx: LeafCtx; tabId: string; slot: ViewerSlot; selected: boolean; onSelect(): void };

/** Shared Files/Review body; pane chrome and within-tab movement belong to the canvas. */
export function ViewerLeafBody({ ctx, tabId, slot, selected, onSelect, kind, children }: ViewerLeafProps & {
  kind: "files" | "review";
  children(context: ViewerContext, view: ContextViewState, onChange: (next: ContextViewState) => void, onViewerError: (error: unknown) => void): ReactNode;
}) {
  const body = useRef<HTMLDivElement>(null);
  const [closing, setClosing] = useState(false);
  const context = slot.context;
  const sourceId = context?.source_id ?? null;
  const savedView = sourceId ? slot.viewsBySource[sourceId] : undefined;
  const view = useMemo(() => (savedView as ContextViewState | undefined) ?? createContextViewState(), [savedView, sourceId]);
  useLayoutEffect(() => {
    if (selected && body.current && !body.current.closest("[data-suppress-attach-focus]") && !document.activeElement?.closest('dialog[open], [role="dialog"][aria-modal="true"]') && !body.current.contains(document.activeElement)) body.current.focus({ preventScroll: true });
  }, [selected]);
  const selectFromPane = (event: SyntheticEvent<HTMLDivElement>) => {
    if (event.target instanceof Node && event.currentTarget.contains(event.target)) onSelect();
  };
  const onChange = useCallback((next: ContextViewState) => {
    if (sourceId) ctx.dispatch({ type: "viewer/source-view", tabId, kind, sourceId, view: next });
  }, [ctx, tabId, kind, sourceId]);
  const onViewerError = useCallback((error: unknown) => {
    if (viewerErrorCode(error) !== "viewer_not_found") return;
    const current = ctx.getState().tabs[tabId]?.viewers[kind];
    if (!current?.context || !context || current.context.binding_id !== context.binding_id || current.context.viewer_id !== context.viewer_id) return;
    ctx.dispatch({ type: "viewer/failed", tabId, kind, error: viewerErrorMessage(error), requestId: current.requestId });
  }, [ctx, tabId, kind, context?.viewer_id, context?.binding_id]);
  const close = async () => {
    setClosing(true);
    try { await closeViewerLeaf(ctx, tabId, kind); }
    catch { /* The lifecycle retains the leaf and exposes the release error inline. */ }
    finally { setClosing(false); }
  };
  const retry = () => {
    const tab = ctx.getState().tabs[tabId];
    const sourcePaneId = tab && Object.hasOwn(tab.terminals, slot.sourcePaneId) ? slot.sourcePaneId : tab ? runtimeSource(tab, tab.selectedLeafId) : null;
    if (!sourcePaneId) {
      ctx.dispatch({ type: "viewer/failed", tabId, kind, error: "No terminal in this tab" });
      return;
    }
    void openViewerLeaf(ctx, tabId, kind, slot.selector, "row", sourcePaneId);
  };
  return <div ref={body} tabIndex={0} className="viewer-leaf-body" aria-label={`${kind === "files" ? "Files" : "Review"} viewer`} onPointerDownCapture={selectFromPane} onFocusCapture={selectFromPane} style={{ display: "flex", flexDirection: "column", minWidth: 0, minHeight: 0, height: "100%", flex: 1 }}>
    {slot.status === "opening" ? <div className="context-empty"><div className="context-empty-message" role="status"><span>Opening {kind === "files" ? "Files" : "Review"}…</span><button type="button" onClick={() => void close()} disabled={closing}>Close</button></div></div>
      : slot.status === "error" || !context ? <div className="context-empty"><div className="context-empty-message" role="alert"><span>{slot.error ?? "The viewer could not be opened."}</span><button type="button" onClick={retry} disabled={closing}>{slot.error === VIEWER_MISSING_MESSAGE ? "Reopen" : "Retry"}</button><button type="button" onClick={() => void close()} disabled={closing}>{closing ? "Closing…" : "Close"}</button></div></div>
      : children(context, view, onChange, onViewerError)}
  </div>;
}

export function FilesLeaf(props: ViewerLeafProps) {
  const { ctx, tabId } = props;
  const onOpenRepository = useCallback(async (path: string) => {
    const state = ctx.getState();
    const tab = state.tabs[tabId];
    if (state.sessionId !== ctx.sessionId || state.serverInstance !== ctx.serverInstance || !tab) throw new Error("This tab is no longer available.");
    const previousSource = tab.viewers.files?.sourcePaneId;
    const sourcePaneId = previousSource && Object.hasOwn(tab.terminals, previousSource) ? previousSource : runtimeSource(tab, tab.selectedLeafId);
    if (!sourcePaneId) throw new Error("No terminal in this tab");
    const options = await ctx.client.viewerSources(ctx.sessionId, sourcePaneId);
    const fresh = ctx.getState();
    const freshTab = fresh.tabs[tabId];
    if (fresh.sessionId !== ctx.sessionId || fresh.serverInstance !== ctx.serverInstance || !freshTab || freshTab.spaceId !== tab.spaceId || !Object.hasOwn(freshTab.terminals, sourcePaneId)
      || options.session_id !== ctx.sessionId || options.pane_id !== sourcePaneId || options.tab_id !== tabId || options.space_id !== freshTab.spaceId) {
      throw new Error("The repository source is no longer available in this tab.");
    }
    const root = options.roots.find(root => root.kind === "repository" && root.path === path);
    if (!root) throw new Error("This repository is no longer available in this Space.");
    await openViewerLeaf(ctx, tabId, "files", { kind: "files_repository", rootId: root.root_id }, "row", sourcePaneId);
  }, [ctx, tabId]);
  return <ViewerLeafBody {...props} kind="files">{(context, value, onChange, onViewerError) => <ContextViewer key={`${context.viewer_id}\0${context.binding_id}`} client={ctx.client} context={context} value={value} onChange={onChange} onViewerError={onViewerError} onOpenRepository={onOpenRepository} space={{ target: { session_id: context.session_id, space_id: context.space_id }, label: "Space", live: true }} />}</ViewerLeafBody>;
}
