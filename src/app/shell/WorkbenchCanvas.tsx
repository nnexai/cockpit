import { TabCanvas } from "../layout/TabCanvas";
import { LeafHost } from "../layout/LeafHost";
import type { LayoutAction, TabLayoutState } from "../layout/tabLayoutStore";
import type { Leaf } from "../layout/splitTree";
import type { Rect } from "../layout/solveLayout";
import type { WorkbenchContentProps } from "./WorkbenchContent";

export function WorkbenchCanvas(props: WorkbenchContentProps) {
  const { client, state, ctx, tabLayout, snapshot, spaces, allTabs, modalOpen, onFocus, browserInputActive, mutationBusy, selectLeaf, zoom, closeLeaf, onRetry, openContext, terminalMouseInput, attachedPaneIds, attachFocusSuppressed, onPanePrepared, switching, setPaintedTab, onReconnect, registerStream, canvasRef, setAttachFocusSuppressed, canvasTabs, area, popup, popupPending, registerTransient, setPrefixHint } = props;
  const dispatchCanvas = (action: LayoutAction) => {
    if (action.type === "select-leaf") { props.onSelectLeaf(action.tabId, action.leafId); return; }
    if (action.type === "zoom-toggle") {
      const id = action.leafId ?? ctx.getState().tabs[action.tabId]?.selectedLeafId;
      if (id) props.onSelectLeaf(action.tabId, id);
    }
    ctx.dispatch(action);
  };
  const renderLeaf = (hostTab: TabLayoutState, leaf: Leaf, rect: Rect) => {
    const active = hostTab.tabId === tabLayout?.tabId;
    if (!active && leaf.kind === "widget") return null;
    const pane = snapshot?.panes.find(candidate => candidate.id === leaf.id && candidate.tab_id === hostTab.tabId);
    const selected = active && hostTab.selectedLeafId === leaf.id;
    const pending = selected && state.focusPending !== null;
    const focusError = selected ? state.focusError : null;
    return <LeafHost ctx={ctx} tab={hostTab} leaf={leaf} rect={rect} pane={pane} selected={selected}
      widgetInputBlocked={modalOpen || !active} widgetLive={state.sync === "live"} widgetLocation={`${spaces.find(space => space.id === hostTab.spaceId)?.label ?? "Space"} · ${allTabs.find(tab => tab.id === hostTab.tabId)?.label ?? "tab"}`}
      onWidgetAgent={(paneId, tabId, spaceId) => { if (state.sync === "live") onFocus({ kind: "pane", target_id: paneId }, { paneId, tabId, spaceId }); }}
      browserInputActive={browserInputActive && active && !modalOpen && state.sync === "live"} browserLiveInputEnabled={active && !modalOpen && state.sync === "live"} focusStatus={pending ? "pending" : focusError ? "error" : null} focusError={focusError?.message}
      closeDisabled={mutationBusy}
      onSelect={() => selectLeaf(leaf.id)}
      onZoom={() => { selectLeaf(leaf.id); zoom(leaf.id); }} onClose={() => { selectLeaf(leaf.id); closeLeaf(leaf.id); }} onRetryFocus={onRetry}
      onMenu={event => { selectLeaf(leaf.id); openContext(event, { kind: "pane", id: leaf.id }); }}
      terminal={pane ? {
        client, request: { session_id: state.sessionId!, pane_id: pane.id }, selected, presented: selected,
        controlAllowed: selected && !browserInputActive && !modalOpen && state.sync === "live" && snapshot?.focused_pane_id === pane.id && !state.focusPending && !state.focusError,
        controlPending: pending, focusEpoch: state.epoch, focusToken: state.focusToken, terminalMouseInput,
        deferAttachment: state.sync !== "live" && !(state.sync === "loading" && attachedPaneIds.current.has(pane.id)), focusOnAttach: active && !attachFocusSuppressed,
        onRequestControl: () => selectLeaf(pane.id), onSelect: () => selectLeaf(pane.id), onPrepared: () => {
          onPanePrepared(pane.id);
          if (active && switching && pane.id === hostTab.focusedPaneId) setPaintedTab(hostTab);
        },
        onResync: onReconnect, onClosed: onReconnect, onClosePane: () => closeLeaf(pane.id), registerStream: (stream, attached) => {
          registerStream(stream, attached);
          if (attached) attachedPaneIds.current.add(pane.id); else attachedPaneIds.current.delete(pane.id);
        },
      } : null} />;
  };
  return <div ref={canvasRef} data-suppress-attach-focus={attachFocusSuppressed || undefined} style={{ position: "relative", flex: "1 1 0", minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column" }} onPointerDownCapture={() => setAttachFocusSuppressed(false)}
    onContextMenu={event => { const pane = (event.target as HTMLElement).closest<HTMLElement>("[data-leaf-id]"); if (pane?.dataset.leafId) { selectLeaf(pane.dataset.leafId); openContext(event, { kind: "pane", id: pane.dataset.leafId }); } }}>
    {canvasTabs.length ? canvasTabs.map(hostTab => <div key={hostTab.tabId} style={{ position: switching ? "absolute" : "relative", inset: switching ? 0 : undefined, flex: "1 1 0", minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column", visibility: switching && hostTab.tabId === tabLayout?.tabId ? "hidden" : "visible", pointerEvents: hostTab.tabId !== tabLayout?.tabId ? "none" : undefined }} inert={hostTab.tabId !== tabLayout?.tabId}><TabCanvas tab={hostTab} area={area} inputBlocked={Boolean(popup || popupPending)} renderLeaf={(leaf, rect) => renderLeaf(hostTab, leaf, rect)} dispatch={dispatchCanvas} registerTransient={registerTransient} announce={setPrefixHint} /></div>) : <div className="empty-main"><strong>No panes</strong><span>Create a tab or select another space.</span></div>}
  </div>;
}
