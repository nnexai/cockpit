// Only run while a watch is unavailable and the document is visible. Reads are
// serialized so a slow backend cannot accumulate overlapping fallback work.
export function watchFallback(refresh: () => Promise<unknown>) {
  let enabled = false;
  let closed = false;
  let timer: number | null = null;
  let running = false;
  const schedule = () => {
    if (closed || !enabled || running || timer !== null || document.visibilityState === "hidden") return;
    timer = window.setTimeout(() => {
      timer = null;
      if (closed || !enabled || document.visibilityState === "hidden") return;
      running = true;
      void Promise.resolve().then(refresh).catch(() => {}).finally(() => { running = false; schedule(); });
    }, 30_000);
  };
  const clear = () => { if (timer !== null) { window.clearTimeout(timer); timer = null; } };
  const visibility = () => { if (document.visibilityState === "hidden") clear(); else schedule(); };
  document.addEventListener("visibilitychange", visibility);
  return {
    start: () => { enabled = true; schedule(); },
    stop: () => { enabled = false; clear(); },
    close: () => { closed = true; clear(); document.removeEventListener("visibilitychange", visibility); },
  };
}
