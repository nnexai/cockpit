import { useEffect, useMemo, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { InboxThread, InboxView, WidgetSummary } from "../../protocol/generated/v1";
import { WidgetFrame } from "../widgets/WidgetFrame";
import { useWidgets, widgetKey, type WidgetStore } from "../widgets/widgetStore";
import "../widgets/widgets.css";
import "./inboxLiveWidgets.css";

/** Only runs actually attached to this thread can supply a live card. */
export function InboxLiveWidgets({ client, sessionId, thread, view, visible }: { client: CockpitClient; sessionId: string; thread: InboxThread; view: InboxView | null; visible: boolean }) {
  const store = useWidgets(client);
  const runs = [...thread.tasks.flatMap(task => task.run ? [task.run] : []),
    ...(view?.owners.filter(owner => thread.owner_root_ids.includes(owner.root_id)) ?? [])];
  const widgets = store.widgets(sessionId).filter(widget => {
    const binding = widget.live?.binding, source = widget.source;
    return widget.kind === "html" && binding && source && binding.session_id === sessionId
      && runs.some(run => run.run_id === binding.run_id && run.attempt === binding.attempt
        && run.session_id === binding.session_id && run.attachment_id === binding.attachment_id
        && run.attachment_epoch === binding.attachment_epoch && (!run.location || (run.location.pane_id === source.pane_id
        && run.location.tab_id === source.tab_id && run.location.workspace_id === source.space_id)));
  });
  if (!widgets.length) return null;
  return <section className="inbox-live-widgets" aria-label="Live views from agents">
    {widgets.map(widget => <InlineWidget key={widgetKey(widget.key)} widget={widget} store={store} visible={visible} />)}
  </section>;
}

function InlineWidget({ widget, store, visible }: { widget: WidgetSummary; store: WidgetStore; visible: boolean }) {
  const host = useRef<HTMLElement>(null);
  const [intersecting, setIntersecting] = useState(false);
  const [focused, setFocused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const key = widgetKey(widget.key);
  useEffect(() => {
    const node = host.current;
    if (!node) return;
    if (typeof IntersectionObserver === "undefined") { setIntersecting(true); return; }
    const observer = new IntersectionObserver(entries => setIntersecting(entries.some(entry => entry.isIntersecting)), { rootMargin: "150px" });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  const mounted = visible && (intersecting || focused);
  useEffect(() => {
    if (!mounted) return;
    const release = store.retainContent(widget.key);
    store.fetchContent(widget);
    return release;
  }, [store, key, widget.revision, mounted, store.snapshotGeneration]);
  const content = store.contentState(widget);
  const live = widget.live!;
  const generation = store.snapshotGeneration;
  const state = useMemo(() => {
    if (!live.state) return null;
    try { return { version: live.state.version, value: JSON.parse(live.state.value_json) as unknown, updated_at_ms: live.state.updated_at_ms }; }
    catch { return null; }
  }, [live.state]);
  const status = !store.connected ? "Reconnecting · last known" : widget.source?.status !== "present" ? "Source closed · last known" : live.status === "ended" ? "Feed ended · last known" : "Live";
  const selection = content.content?.selection?.value_json;
  let selected: unknown = null;
  try { if (selection != null) selected = JSON.parse(selection); } catch { /* Invalid selection is not interpreted. */ }
  const html = content.content?.body.type === "html" ? content.content.body.document : null;
  const updated = state ? new Date(state.updated_at_ms) : null;
  const validUpdated = updated && Number.isFinite(updated.getTime()) ? updated : null;
  return <article ref={host} tabIndex={-1} className="inbox-live-widget" onFocusCapture={() => setFocused(true)} onBlurCapture={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocused(false); }}>
    <header><strong>{widget.title}</strong><span role="status">{status}{validUpdated && <> · <time dateTime={validUpdated.toISOString()}>Updated {validUpdated.toLocaleTimeString()}</time></>}</span></header>
    {error && <p role="alert">{error}</p>}
    {mounted && content.error && <p role="alert">{content.error} <button type="button" onClick={() => store.fetchContent(widget)}>Retry</button></p>}
    {mounted && !html && !content.error && <p role="status">Loading live view…</p>}
    {mounted && html && <div className="inbox-live-widget-frame"><WidgetFrame key={`${key}:${generation}:${JSON.stringify(live.binding)}`} widgetKey={key} document={html} revision={widget.revision} currentRevision={widget.revision}
      selection={selected} hasSelection={selection != null} agent={widget.source?.agent_label ?? "agent"} inputBlocked={!visible || !store.connected}
      liveState={{ ...state, status: store.connected ? live.status : "reconnecting" }}
      onSelect={value => { setError(null); void store.selectPage(widget.key, widget.revision, html, value, true, generation).catch(reason => setError(reason instanceof Error ? reason.message : String(reason))); }}
      onEmit={async value_json => { await store.emitPage(widget.key, widget.revision, html, generation, live.binding, content.content!.sha256, value_json); }}
      onFocusRetired={() => host.current?.focus({ preventScroll: true })} /></div>}
  </article>;
}
