import { useSyncExternalStore } from "react";
import type { CockpitClient, WidgetStream } from "../../client/CockpitClient";
import type { WidgetContent, WidgetEvent, WidgetKey, WidgetSummary, WidgetWindowReport } from "../../protocol/generated/v1";
import type { LeafCtx } from "../layout/tabLayoutStore";
import { decideArrival } from "./arrival";

export const widgetKey = (key: WidgetKey): string => JSON.stringify([key.session_id, key.tab_id, key.id]);
export type WidgetContentState = { content: WidgetContent | null; error: string | null; loading: boolean };
const runtimes = new WeakMap<CockpitClient, WidgetStore>();
const MAX_CACHED_WIDGETS = 16;
export class WidgetStore {
  readonly byKey = new Map<string, WidgetSummary>();
  readonly undisplayedOwn = new Set<string>();
  readonly awaitingClick = new Set<string>();
  readonly unseen = new Set<string>();
  readonly everDisplayed = new Set<string>();
  private pending = new Set<string>();
  private userRemovals = new Set<string>();
  private userRemovalFocus = new Map<string, Element>();
  private content = new Map<string, WidgetContentState>();
  private contentRequests = new Map<string, AbortController>();
  private listeners = new Set<() => void>();
  private revision = 0;
  private sequence = -1;
  private ctx: LeafCtx | null = null;
  private report: WidgetWindowReport = { session_id: null, displayed_tab_id: null, blocker: null };
  private stream: WidgetStream | null = null;
  private abort: AbortController | null = null;
  private reconnect: number | undefined;
  private users = 0;
  private generation = 0;
  private announce: (widget: WidgetSummary, docked: boolean) => void = () => undefined;
  private blockerFor: ((widget: WidgetSummary) => WidgetWindowReport["blocker"]) | null = null;
  constructor(readonly client: CockpitClient) {}
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  getSnapshot = (): number => this.revision;
  private notify(): void { this.revision++; for (const listener of this.listeners) listener(); }
  widgets(sessionId: string, tabId?: string): WidgetSummary[] {
    return [...this.byKey.values()].filter(w => w.key.session_id === sessionId && (tabId === undefined || w.key.tab_id === tabId)).sort((a, b) => a.created_seq - b.created_seq);
  }
  tabHasDot(sessionId: string, tabId: string): boolean { return this.widgets(sessionId, tabId).some(w => this.undisplayedOwn.has(widgetKey(w.key)) || this.awaitingClick.has(widgetKey(w.key))); }
  needsClick(sessionId: string, tabId: string): boolean { return this.widgets(sessionId, tabId).some(w => this.awaitingClick.has(widgetKey(w.key))); }
  bind(ctx: LeafCtx, announce: (widget: WidgetSummary, docked: boolean) => void, blockerFor?: (widget: WidgetSummary) => WidgetWindowReport["blocker"]): void {
    this.ctx = ctx; this.announce = announce; this.blockerFor = blockerFor ?? null;
  }
  retain(): () => void {
    this.users++;
    if (this.users === 1) this.connect();
    return () => { if (--this.users === 0) { this.generation++; this.abort?.abort(); this.stream?.close(); this.stream = null; window.clearTimeout(this.reconnect); this.reconnect = undefined; } };
  }
  private connect(): void {
    const generation = ++this.generation;
    this.abort = new AbortController();
    let failed = false;
    const fail = () => {
      if (failed || generation !== this.generation || !this.users) return;
      failed = true; this.stream?.close(); this.stream = null; this.abort?.abort();
      this.reconnect = window.setTimeout(() => { this.reconnect = undefined; if (this.users) this.connect(); }, 1000);
    };
    void this.client.subscribeWidgets(event => { if (!failed && generation === this.generation) this.accept(event); }, fail, this.abort.signal).then(stream => {
      if (failed || generation !== this.generation || !this.users) { stream.close(); return; }
      this.stream = stream; stream.report(this.report);
    }, fail);
  }
  accept(event: WidgetEvent): void {
    if (event.type !== "snapshot" && event.sequence <= this.sequence) return;
    if (event.type !== "snapshot" && this.sequence >= 0 && event.sequence !== this.sequence + 1) {
      this.stream?.close(); this.abort?.abort(); this.stream = null;
      this.generation++;
      if (this.users && !this.reconnect) this.reconnect = window.setTimeout(() => { this.reconnect = undefined; this.connect(); }, 1000);
      return;
    }
    this.sequence = event.sequence;
    if (event.type === "snapshot") {
      this.userRemovals.clear(); this.userRemovalFocus.clear();
      const live = new Set(event.widgets.map(w => widgetKey(w.key)));
      for (const old of [...this.byKey.values()]) if (!live.has(widgetKey(old.key))) this.removeLocal(old.key, false);
      for (const widget of [...event.widgets].sort((a, b) => a.created_seq - b.created_seq)) this.upsert(widget, !this.everDisplayed.has(widgetKey(widget.key)), true);
    } else if (event.type === "removed") { this.userRemovals.delete(widgetKey(event.key)); this.removeLocal(event.key, event.reason === "user"); }
    else this.upsert(event.widget, false);
    this.trimContent(); this.notify();
  }
  private upsert(widget: WidgetSummary, snapshotArrival: boolean, snapshot = false): void {
    if (widget.change === "reopened") { this.userRemovals.delete(widgetKey(widget.key)); this.userRemovalFocus.delete(widgetKey(widget.key)); }
    const key = widgetKey(widget.key), previous = this.byKey.get(key);
    if (previous && previous.revision !== widget.revision) this.dropContent(key);
    this.byKey.set(key, widget);
    if (snapshotArrival || (!snapshot && (widget.change === "opened" || widget.change === "reopened"))) {
      this.pending.add(key); this.applyArrival(widget, snapshot);
    } else {
      const decision = this.arrival(widget, snapshot && previous && previous.revision !== widget.revision ? "replaced" : widget.change);
      if (decision.kind === "replace_in_place" && decision.markUnseen && this.everDisplayed.has(key)) this.unseen.add(key);
    }
  }
  private arrival(widget: WidgetSummary, change = widget.change) {
    const tab = this.ctx?.getState().tabs[widget.key.tab_id];
    const displayed = this.report.session_id === widget.key.session_id && this.report.displayed_tab_id === widget.key.tab_id;
    const frame = typeof document !== "undefined" ? document.activeElement : null;
    return decideArrival({ change, arrival: widget.arrival, tabDisplayed: displayed, blocker: displayed ? this.blockerFor ? this.blockerFor(widget) : this.report.blocker : null,
      dockPresent: Boolean(tab?.viewers.widget), isCurrent: tab?.viewers.widget?.currentId === widget.key.id,
      focusInsideFrame: frame?.tagName === "IFRAME" && frame.closest("[data-widget-tab]")?.getAttribute("data-widget-tab") === widget.key.tab_id });
  }
  private applyArrival(widget: WidgetSummary, quiet = false): void {
    const key = widgetKey(widget.key), decision = this.arrival(widget, "opened");
    let docked = false;
    if (decision.kind === "dock_now") {
      docked = this.dock(widget, decision.makeCurrent);
      if (docked) { this.pending.delete(key); if (!decision.makeCurrent) this.unseen.add(key); }
    } else if (decision.kind === "wait_click") { this.awaitingClick.add(key); this.pending.delete(key); }
    else if (decision.kind === "wait_tab") this.undisplayedOwn.add(key);
    else if (decision.kind === "defer_until_clear" && decision.indicator === "widgets_button") this.awaitingClick.add(key);
    if (!quiet && "announce" in decision && decision.announce) this.announce(widget, docked);
  }
  updateWindow(report: WidgetWindowReport): void {
    const changed = JSON.stringify(this.report) !== JSON.stringify(report);
    this.report = report;
    if (changed) this.stream?.report(report);
    let applied = false;
    for (const key of [...this.pending]) {
      const widget = this.byKey.get(key); if (!widget) continue;
      const before = this.pending.has(key); this.applyArrival(widget, true);
      if (before && !this.pending.has(key)) applied = true;
    }
    this.trimContent(); if (changed || applied) this.notify();
  }
  private dock(widget: WidgetSummary, makeCurrent = true): boolean {
    const ctx = this.ctx;
    if (!ctx || ctx.sessionId !== widget.key.session_id) return false;
    const tab = ctx.getState().tabs[widget.key.tab_id]; if (!tab) return false;
    const beside = widget.source && tab.terminals[widget.source.pane_id] ? widget.source.pane_id : tab.lastRealLeafId ?? Object.keys(tab.terminals).at(-1);
    if (!beside) return false;
    if (!tab.viewers.widget) ctx.dispatch({ type: "widget/dock", tabId: tab.tabId, besideLeafId: beside, currentId: widget.key.id });
    else if (makeCurrent) ctx.dispatch({ type: "widget/current", tabId: tab.tabId, currentId: widget.key.id });
    const key = widgetKey(widget.key); this.undisplayedOwn.delete(key); this.awaitingClick.delete(key);
    this.everDisplayed.add(key); if (makeCurrent) this.unseen.delete(key);
    return true;
  }
  show(tabId: string, id?: string): void {
    if (!this.ctx) return;
    const widgets = this.widgets(this.ctx.sessionId, tabId);
    const target = id ? widgets.find(w => w.key.id === id) : [...widgets].reverse().find(w => this.awaitingClick.has(widgetKey(w.key))) ?? widgets.at(-1);
    if (!target) return;
    if (this.dock(target)) {
      for (const widget of widgets) { const key = widgetKey(widget.key); this.awaitingClick.delete(key); this.undisplayedOwn.delete(key); this.pending.delete(key); this.everDisplayed.add(key); }
      this.trimContent(); this.notify();
    }
  }
  current(tabId: string, id: string): void { this.show(tabId, id); }
  cycle(tabId: string, step: number): void {
    if (!this.ctx) return;
    const widgets = this.widgets(this.ctx.sessionId, tabId), current = this.ctx.getState().tabs[tabId]?.viewers.widget?.currentId;
    if (!widgets.length) return;
    this.current(tabId, widgets[(widgets.findIndex(w => w.key.id === current) + step + widgets.length) % widgets.length].key.id);
  }
  async remove(key: WidgetKey): Promise<void> {
    const encoded = widgetKey(key);
    this.userRemovals.add(encoded);
    if (typeof document !== "undefined" && document.activeElement) this.userRemovalFocus.set(encoded, document.activeElement);
    try {
      await this.client.widgetRemove({ key });
      // The matching owner event can precede the response. Never delete a later reopen.
      if (this.userRemovals.has(encoded)) { this.userRemovals.delete(encoded); this.removeLocal(key, true); this.notify(); }
    } catch (error) { this.userRemovals.delete(encoded); this.userRemovalFocus.delete(encoded); throw error; }
  }
  private removeLocal(key: WidgetKey, userRemoval: boolean): void {
    const encoded = widgetKey(key), siblings = this.widgets(key.session_id, key.tab_id), index = siblings.findIndex(w => widgetKey(w.key) === encoded);
    const requestedFocus = this.userRemovalFocus.get(encoded);
    this.userRemovalFocus.delete(encoded);
    this.byKey.delete(encoded); this.pending.delete(encoded); this.awaitingClick.delete(encoded); this.undisplayedOwn.delete(encoded); this.unseen.delete(encoded); this.everDisplayed.delete(encoded); this.dropContent(encoded);
    const ctx = this.ctx, tab = ctx?.sessionId === key.session_id ? ctx.getState().tabs[key.tab_id] : null;
    if (tab?.viewers.widget) {
      const remaining = siblings.filter(w => widgetKey(w.key) !== encoded);
      if (!remaining.length) {
        const focused = typeof document !== "undefined" ? document.activeElement : null;
        // Disabling a clicked Remove button can leave BODY focused before the
        // owner responds. Keep that local user intent, but never override a
        // dialog or other destination focused during the request.
        const owner = typeof document !== "undefined" && focused === document.body ? requestedFocus ?? focused : focused;
        const ownsFocus = owner?.closest("[data-widget-tab]")?.getAttribute("data-widget-tab") === key.tab_id
          || owner?.closest("[data-leaf-id]")?.getAttribute("data-leaf-id") === `${key.tab_id}:widget`
          || owner?.closest("[data-pane-header]")?.getAttribute("data-pane-header") === `${key.tab_id}:widget`;
        const restoreFocus = userRemoval && ownsFocus;
        ctx!.dispatch({ type: "widget/undock", tabId: key.tab_id });
        if (restoreFocus) requestAnimationFrame(() => {
          if (document.activeElement !== focused && document.activeElement !== document.body) return;
          if (this.report.session_id !== key.session_id || this.report.displayed_tab_id !== key.tab_id || this.ctx?.sessionId !== key.session_id) return;
          const leafId = ctx!.getState().tabs[key.tab_id]?.selectedLeafId;
          const host = [...document.querySelectorAll<HTMLElement>("[data-leaf-id]")].find(node => node.dataset.leafId === leafId);
          const target = host?.querySelector<HTMLElement>('textarea:not([disabled]), input:not([disabled]), [contenteditable="true"], [tabindex="0"]') ?? host;
          target?.focus({ preventScroll: true });
        });
      }
      else if (tab.viewers.widget.currentId === key.id) {
        const next = remaining[Math.min(Math.max(index, 0), remaining.length - 1)];
        ctx!.dispatch({ type: "widget/current", tabId: key.tab_id, currentId: next.key.id });
        this.unseen.delete(widgetKey(next.key));
      }
    }
  }
  contentState(widget: WidgetSummary): WidgetContentState { return this.content.get(widgetKey(widget.key)) ?? { content: null, error: null, loading: false }; }
  fetchContent(widget: WidgetSummary): void {
    const key = widgetKey(widget.key);
    if (this.contentRequests.has(key) || this.content.get(key)?.content?.revision === widget.revision) return;
    const abort = new AbortController(); this.contentRequests.set(key, abort);
    this.content.set(key, { content: null, error: null, loading: true }); this.notify();
    void this.client.widgetContent({ key: widget.key, revision: widget.revision }, abort.signal).then(content => {
      if (this.contentRequests.get(key) !== abort || this.byKey.get(key)?.revision !== widget.revision) return;
      if (widgetKey(content.key) !== key || content.revision !== widget.revision) throw new Error("Widget content belongs to another revision.");
      this.content.set(key, { content, error: null, loading: false });
    }).catch(error => { if (this.contentRequests.get(key) === abort && !abort.signal.aborted) this.content.set(key, { content: null, error: error instanceof Error ? error.message : String(error), loading: false }); }).finally(() => {
      if (this.contentRequests.get(key) === abort) { this.contentRequests.delete(key); this.trimContent(); this.notify(); }
    });
  }
  async selectPage(key: WidgetKey, revision: number, document: string, valueJson: string): Promise<void> {
    const encoded = widgetKey(key), widget = this.byKey.get(encoded), state = this.content.get(encoded);
    const content = state?.content;
    const tab = this.ctx?.sessionId === key.session_id ? this.ctx.getState().tabs[key.tab_id] : null;
    if (!widget || widget.revision !== revision || tab?.viewers.widget?.currentId !== key.id
      || !content || content.revision !== revision || widgetKey(content.key) !== encoded
      || content.body.type !== "html" || content.body.document !== document) return;
    const response = await this.client.widgetSelect({ key, revision, value: { type: "page", value_json: valueJson } });
    // A late response never installs selection state into superseded content.
    const latest = this.content.get(encoded)?.content;
    if (this.byKey.get(encoded)?.revision !== revision || !latest || latest.revision !== revision
      || latest.body.type !== "html" || latest.body.document !== document
      || (latest.selection?.at_ms !== null && latest.selection?.at_ms !== undefined && latest.selection.at_ms > response.at_ms)) return;
    this.content.set(encoded, { error: null, loading: false, content: { ...latest, selection: {
      id: key.id, revision, status: "selected", value_json: valueJson, at_ms: response.at_ms, removed_at_ms: null,
    } } });
    this.notify();
  }
  private dropContent(key: string): void { this.contentRequests.get(key)?.abort(); this.contentRequests.delete(key); this.content.delete(key); }
  private trimContent(): void {
    // Cache only the current content of each existing dock, never hidden revisions.
    for (const key of this.content.keys()) {
      const widget = this.byKey.get(key), ctx = this.ctx;
      const tab = widget && ctx?.sessionId === widget.key.session_id ? ctx.getState().tabs[widget.key.tab_id] : null;
      if (!widget || tab?.viewers.widget?.currentId !== widget.key.id) this.dropContent(key);
    }
    while (this.content.size > MAX_CACHED_WIDGETS) this.dropContent(this.content.keys().next().value!);
  }
}
export function getWidgetStore(client: CockpitClient): WidgetStore { let store = runtimes.get(client); if (!store) { store = new WidgetStore(client); runtimes.set(client, store); } return store; }
export function useWidgets(client: CockpitClient): WidgetStore { const store = getWidgetStore(client); useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot); return store; }
