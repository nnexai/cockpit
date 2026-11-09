import type { CockpitClient, ClosableStream, LocalChangeScope } from "../../client/CockpitClient";

/** Owns one pinned durable stream. Reads serialize with invalidations and explicit refreshes. */
export function orchestrationChanges(
  client: Pick<CockpitClient, "subscribeLocalChanges">,
  scope: () => Promise<LocalChangeScope>,
  readAndApply: (current: () => boolean) => Promise<void>,
  onUnavailable: (error: unknown) => void,
  signal?: AbortSignal,
): { refresh(): Promise<void>; close(): void } {
  let closed = false, epoch = 0, retryMs = 1500, ready = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let abort: AbortController | undefined, stream: ClosableStream | undefined;
  let pinned: LocalChangeScope | undefined;
  let dirty = false, draining: Promise<void> | undefined, drainEpoch = 0;
  const visible = () => typeof document === "undefined" || document.visibilityState !== "hidden";
  const stop = () => { epoch++; ready = false; abort?.abort(); abort = undefined; stream?.close(); stream = undefined; };
  const schedule = () => {
    if (closed || timer !== undefined || !visible()) return;
    const delay = retryMs;
    retryMs = Math.min(retryMs * 2, 30_000);
    timer = setTimeout(() => { timer = undefined; start(); }, delay);
  };
  let failed = false;
  const fail = (error: unknown, attempt: number) => {
    if (closed || attempt !== epoch || failed) return;
    failed = true; stop(); onUnavailable(error); schedule();
  };
  const refresh = (): Promise<void> => {
    if (closed) return Promise.resolve();
    dirty = true;
    if (draining) return drainEpoch === epoch
      ? draining.then(() => dirty ? refresh() : undefined)
      : draining.catch(() => {}).then(refresh);
    const attempt = epoch;
    const active = () => !closed && epoch === attempt;
    if (!ready) return Promise.resolve();
    drainEpoch = attempt;
    draining = (async () => {
      while (dirty && active()) {
        dirty = false;
        await readAndApply(() => active() && !dirty);
        if (!active()) return;
        if (!dirty) retryMs = 1500;
      }
    })().catch(error => { fail(error, attempt); throw error; }).finally(() => { draining = undefined; });
    return draining;
  };
  const start = () => {
    if (closed) return;
    failed = false;
    const attempt = ++epoch;
    const controller = new AbortController(); abort = controller;
    void (async () => {
      try {
        pinned ??= await scope();
        if (closed || attempt !== epoch) return;
        const opened = await client.subscribeLocalChanges(pinned, async event => {
          if (closed || attempt !== epoch) return;
          if (event.kind === "unavailable") throw new Error(event.reason?.message ?? "Orchestration observation unavailable");
          ready = true;
          await refresh();
        }, error => fail(error, attempt), controller.signal);
        if (closed || attempt !== epoch) opened.close(); else stream = opened;
      } catch (error) { fail(error, attempt); }
    })();
  };
  const visibility = () => {
    if (!visible()) { if (timer !== undefined) { clearTimeout(timer); timer = undefined; } }
    else if (failed && timer === undefined) schedule();
  };
  const close = () => {
    if (closed) return;
    closed = true; stop();
    if (typeof document !== "undefined") document.removeEventListener("visibilitychange", visibility);
    signal?.removeEventListener("abort", close);
    clearTimeout(timer);
  };
  if (typeof document !== "undefined") document.addEventListener("visibilitychange", visibility);
  signal?.addEventListener("abort", close, { once: true });
  if (signal?.aborted) close(); else start();
  return { refresh, close };
}
