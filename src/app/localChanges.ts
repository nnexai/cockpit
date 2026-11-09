import { CockpitClientError } from "../client/CockpitClient";
import type { ClosableStream, CockpitClient, LocalChangeEvent, LocalChangeScope } from "../client/CockpitClient";

const delays = [1500, 3000, 6000, 12000];
type Consumer = { onEvent: (event: LocalChangeEvent) => Promise<void>; onError: (error: CockpitClientError) => void; pending: Promise<void>; closed: boolean };
type Entry = { consumers: Set<Consumer>; close: () => void; retry: () => void; baseline?: LocalChangeEvent };
const clients = new WeakMap<CockpitClient, Map<string, Entry>>();

/** Share only identical, live scopes within one client. Each listener finishes before credit. */
export function subscribeLocalChanges(
  client: CockpitClient, scope: LocalChangeScope,
  onEvent: (event: LocalChangeEvent) => Promise<void>,
  onError: (error: CockpitClientError) => void,
): ClosableStream & { retry(): void } {
  // Reconnects must retain the original authority even if a caller mutates its input.
  scope = JSON.parse(JSON.stringify(scope)) as LocalChangeScope;
  let entries = clients.get(client);
  if (!entries) { entries = new Map(); clients.set(client, entries); }
  const key = JSON.stringify(Object.fromEntries(Object.entries(scope).sort(([a], [b]) => a.localeCompare(b)).map(([name, value]) => [name, name === "paths" && Array.isArray(value) ? [...value].sort() : value])));
  let entry = entries.get(key);
  if (!entry) {
    if (entries.size >= 32) {
      let closed = false;
      let admitted: (ClosableStream & { retry(): void }) | undefined;
      const retry = () => {
        if (closed) return;
        if (admitted) { admitted.retry(); return; }
        if (entries!.size >= 32) {
          onError(new CockpitClientError("stream_error", "Too many local changes subscriptions"));
        } else admitted = subscribeLocalChanges(client, scope, onEvent, onError);
      };
      queueMicrotask(retry);
      return { retry, close: () => { closed = true; admitted?.close(); } };
    }
    const consumers = new Set<Consumer>();
    let disposed = false;
    let generation = 0;
    let controller: AbortController | undefined;
    let active: ClosableStream | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let attempts = 0;
    const visible = () => typeof document === "undefined" || document.visibilityState !== "hidden";
    const report = (error: CockpitClientError) => { for (const consumer of consumers) if (!consumer.closed) consumer.onError(error); };
    const dispatch = (consumer: Consumer, event: LocalChangeEvent): Promise<boolean> => {
      const result = consumer.pending.then(async () => {
        if (!consumer.closed) {
          try { await consumer.onEvent(event); }
          catch (error) {
            if (!consumer.closed) consumer.onError(error as CockpitClientError);
            return false;
          }
        }
        return true;
      });
      consumer.pending = result.then(() => {});
      return result;
    };
    const stop = () => {
      generation++;
      if (timer !== undefined) { clearTimeout(timer); timer = undefined; }
      controller?.abort(); controller = undefined;
      active?.close(); active = undefined;
      entry!.baseline = undefined;
    };
    const start = () => {
      if (disposed || !visible() || controller) return;
      const current = ++generation;
      const request = new AbortController();
      controller = request;
      const failed = (error: CockpitClientError) => {
        if (disposed || current !== generation || request.signal.aborted) return;
        controller = undefined;
        request.abort();
        active?.close(); active = undefined;
        entry!.baseline = undefined;
        report(error);
        if (attempts < delays.length) timer = setTimeout(() => { timer = undefined; start(); }, delays[attempts++]);
      };
      void client.subscribeLocalChanges(scope, async event => {
        if (disposed || current !== generation || request.signal.aborted) return;
        if (event.kind === "ready") entry!.baseline = event;
        const delivered = new Set<Consumer>();
        let reconciled = true;
        for (;;) {
          const batch = [...consumers].filter(consumer => !delivered.has(consumer));
          if (batch.length === 0) break;
          const results = await Promise.all(batch.map(consumer => {
            delivered.add(consumer);
            return dispatch(consumer, event);
          }));
          if (results.some(result => !result)) reconciled = false;
        }
        if (event.kind === "ready" && reconciled && !disposed && current === generation && !request.signal.aborted) attempts = 0;
      }, failed, request.signal).then(stream => {
        if (disposed || current !== generation || request.signal.aborted) stream.close();
        else active = stream;
      }, error => failed(error as CockpitClientError));
    };
    const retry = () => { if (disposed) return; stop(); attempts = 0; start(); };
    const visibility = () => { if (visible()) retry(); else stop(); };
    const focus = () => { if (visible() && !controller && timer === undefined) retry(); };
    entry = { consumers, retry, close: () => {
      disposed = true; stop();
      if (typeof document !== "undefined") document.removeEventListener("visibilitychange", visibility);
      if (typeof window !== "undefined") window.removeEventListener("focus", focus);
      entries!.delete(key);
    } };
    entries.set(key, entry);
    if (typeof document !== "undefined") document.addEventListener("visibilitychange", visibility);
    if (typeof window !== "undefined") window.addEventListener("focus", focus);
    // Start in a microtask so the first consumer is registered before any synchronous transport event.
    queueMicrotask(start);
  }
  const selected = entry;
  const consumer: Consumer = { onEvent, onError, pending: Promise.resolve(), closed: false };
  selected.consumers.add(consumer);
  if (selected.baseline) {
    const baseline = selected.baseline;
    consumer.pending = Promise.resolve().then(async () => {
      if (!consumer.closed) {
        try { await consumer.onEvent(baseline); }
        catch (error) { if (!consumer.closed) consumer.onError(error as CockpitClientError); }
      }
    });
  }
  return { close: () => {
    if (consumer.closed) return;
    consumer.closed = true;
    selected.consumers.delete(consumer);
    if (selected.consumers.size === 0) selected.close();
  }, retry: selected.retry };
}
