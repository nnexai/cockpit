import { useEffect, useRef, useState } from "react";
import type { WidgetContent } from "../../protocol/generated/v1";
import type { LeafCtx, TabLayoutState } from "../layout/tabLayoutStore";
import { PaneChrome } from "../layout/PaneChrome";
import { WidgetFrame } from "./WidgetFrame";
import { WidgetChoices } from "./WidgetChoices";
import { WidgetTabs } from "./WidgetTabs";
import { WidgetChip } from "./WidgetChip";
import { useWidgets, widgetKey } from "./widgetStore";
import "./widgets.css";

export function WidgetDock({ ctx, tab, width, selected, inputBlocked, live, location, onSelect, onZoom, onGoToAgent }: {
  ctx: LeafCtx; tab: TabLayoutState; width: number; selected: boolean; inputBlocked: boolean; live: boolean; location?: string;
  onSelect(): void; onZoom(): void; onGoToAgent(paneId: string, tabId: string, spaceId: string): void;
}) {
  const store = useWidgets(ctx.client);
  const widgets = store.widgets(ctx.sessionId, tab.tabId);
  const current = widgets.find(w => w.key.id === tab.viewers.widget?.currentId) ?? widgets[0];
  const wrapper = useRef<HTMLDivElement>(null);
  const retiringFocus = useRef(false);
  const [removeError, setRemoveError] = useState<string | null>(null);
  const [selectionError, setSelectionError] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);
  const preview = useRef<{ key: string; document: string; revision: number; selection: unknown; hasSelection: boolean; content: WidgetContent } | null>(null);
  const key = current ? widgetKey(current.key) : null;
  const revision = current?.revision;
  const selectionOwner = useRef({ key, revision });
  selectionOwner.current = { key, revision };
  useEffect(() => { if (current) store.fetchContent(current); }, [store, key, revision]);
  useEffect(() => { setRemoveError(null); setSelectionError(null); }, [key, revision]);
  if (!current) return null;
  const agent = current.source?.agent_label ?? (current.source ? "pane" : "CLI");
  const state = store.contentState(current);
  if (state.content?.body.type === "html" && preview.current?.content !== state.content) {
    const valueJson = state.content.selection?.value_json;
    let selection: unknown = null;
    let hasSelection = false;
    if (valueJson !== null && valueJson !== undefined) {
      try { selection = JSON.parse(valueJson); hasSelection = true; } catch { /* Invalid stored data is never interpreted. */ }
    }
    preview.current = { key: widgetKey(current.key), document: state.content.body.document, revision: state.content.revision, selection, hasSelection, content: state.content };
  }
  else if (preview.current?.key !== widgetKey(current.key) || current.kind !== "html") preview.current = null;
  const html = preview.current;
  const remove = () => {
    setRemoving(true); setRemoveError(null);
    void store.remove(current.key).catch(error => setRemoveError(error instanceof Error ? error.message : String(error))).finally(() => setRemoving(false));
  };
  return <>
    <PaneChrome leaf={{ t: "leaf", id: `${tab.tabId}:widget`, kind: "widget", w: 1 }} title={current.title} selected={selected} zoomed={tab.zoomLeafId === `${tab.tabId}:widget`}
      titleContent={<WidgetTabs widgets={widgets} currentId={current.key.id} unseen={store.unseen} width={width} onCurrent={id => store.current(tab.tabId, id)} />}
      controls={<WidgetChip widget={current} width={width} live={live} location={location} onGoToAgent={() => { if (current.source?.status === "present") onGoToAgent(current.source.pane_id, current.source.tab_id, current.source.space_id); }} />}
      onZoom={onZoom} onClose={remove} closeDisabled={removing || inputBlocked} />
    <div ref={wrapper} className="widget-dock" data-widget-tab={tab.tabId} data-input-blocked={inputBlocked || undefined} tabIndex={0} role="group" aria-label={`Widget: ${current.title}, from ${agent}`}
      onFocus={event => { if (event.target === event.currentTarget && !inputBlocked && !retiringFocus.current) onSelect(); }}
      onKeyDown={event => { if (event.key === "Enter" && event.target === event.currentTarget && !inputBlocked) { event.preventDefault(); wrapper.current?.querySelector<HTMLElement>('iframe:not([data-pending]), input[type="radio"]:not(:disabled)')?.focus(); } }}>
      {removeError ? <div role="alert" className="widget-inline-error">{removeError}<button type="button" onClick={remove}>Retry removal</button></div> : null}
      {state.error ? <div role="alert" className="widget-inline-error">{state.error}<button type="button" onClick={() => store.fetchContent(current)}>Retry loading</button></div> : null}
      {selectionError ? <div role="alert" className="widget-inline-error">{selectionError}</div> : null}
      {!state.content && !state.error && !html ? <div role="status" className="widget-loading">Loading widget…</div> : null}
      {html ? <WidgetFrame key={key} widgetKey={key!} document={html.document} revision={html.revision} currentRevision={current.revision}
        selection={html.selection} hasSelection={html.hasSelection} agent={agent} inputBlocked={inputBlocked}
        onUserFocus={() => { if (!selected && !inputBlocked && !retiringFocus.current) onSelect(); }} onSelect={valueJson => {
          setSelectionError(null);
          void store.selectPage(current.key, html.revision, html.document, valueJson).catch(error => {
            if (selectionOwner.current.key === key && selectionOwner.current.revision === html.revision) {
              setSelectionError(error instanceof Error ? error.message : "Could not store this selection.");
            }
          });
        }} onFocusRetired={() => {
        retiringFocus.current = true;
        wrapper.current?.focus({ preventScroll: true });
        retiringFocus.current = false;
      }} /> : null}
      {state.content?.body.type === "choices" ? <WidgetChoices key={key} client={ctx.client} widget={current} spec={state.content.body.spec} inputBlocked={inputBlocked} /> : null}
    </div>
  </>;
}
