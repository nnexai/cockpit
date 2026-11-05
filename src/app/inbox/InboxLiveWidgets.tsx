import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { InboxWidgetEvent, WidgetSummary } from "../../protocol/generated/v1";
import { WidgetFrame } from "../widgets/WidgetFrame";
import { useWidgets, widgetKey, type WidgetStore } from "../widgets/widgetStore";
import "../widgets/widgets.css";
import "./inboxLiveWidgets.css";

/** Resolve only the exact publication, never a later widget reusing the same public id. */
export function InboxTimelineWidget({ client, sessionId, event, visible }: { client: CockpitClient; sessionId: string; event: InboxWidgetEvent; visible: boolean }) {
  const store = useWidgets(client);
  const widget = store.byKey.get(widgetKey(event.key));
  const binding = widget?.live?.binding;
  const expected = event.binding;
  const matches = event.key.session_id === sessionId && widget?.kind === "html"
    && widget.revision === event.revision && widget.content.sha256 === event.content_sha256
    && widget.live?.publication_id === event.publication_id
    && binding?.run_id === expected.run_id && binding.attempt === expected.attempt
    && binding.session_id === expected.session_id && binding.attachment_id === expected.attachment_id
    && binding.attachment_epoch === expected.attachment_epoch;
  if (!matches || !widget) return <article className="inbox-live-widget">
    <header><strong>{event.title}</strong><span>View unavailable</span></header>
    <p>This published view is no longer available.</p>
  </article>;
  return <InlineWidget widget={widget} store={store} visible={visible} />;
}

function InlineWidget({ widget, store, visible }: { widget: WidgetSummary; store: WidgetStore; visible: boolean }) {
  const host = useRef<HTMLElement>(null);
  const [intersecting, setIntersecting] = useState(false);
  const [focused, setFocused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const key = widgetKey(widget.key);
  useLayoutEffect(() => () => {
    const node = host.current;
    if (node?.contains(document.activeElement)) {
      const row = node.closest<HTMLElement>(".inbox-widget-event");
      if (row) { row.tabIndex = -1; row.focus({ preventScroll: true }); }
    }
  }, []);
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
    <header><strong>{widget.title}</strong><span><span role="status">{status}</span>{validUpdated && <> · <time dateTime={validUpdated.toISOString()}>Updated {validUpdated.toLocaleTimeString()}</time></>}</span></header>
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
