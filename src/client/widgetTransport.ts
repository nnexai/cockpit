import { CockpitClientError } from "./CockpitClient";
import type { WidgetEvent, WidgetWindowReport } from "../protocol/generated/v1";
import { parseWidgetEvent, parseWidgetWindowReport } from "./widgetProtocol";

/** Each subscription owns its cursor; a snapshot is its only initial authority. */
export function widgetEventCursor(): (value: unknown) => WidgetEvent | undefined {
  let sequence: number | undefined;
  return (value) => {
    const event = parseWidgetEvent(value);
    if (sequence === undefined && event.type !== "snapshot") {
      throw new CockpitClientError("malformed_response", "Widget stream did not start with a snapshot");
    }
    if (sequence !== undefined) {
      if (event.sequence <= sequence) return undefined;
      if (event.type !== "snapshot" && event.sequence !== sequence + 1) {
        throw new CockpitClientError("stream_error", "Widget stream lost events; a fresh snapshot is required");
      }
    }
    sequence = event.sequence;
    return event;
  };
}

export interface WidgetReportQueue {
  report(value: WidgetWindowReport): void;
  close(): void;
}

/** Trailing coalescing preserves the newest report and never outlives its stream. */
export function widgetReports(send: (report: WidgetWindowReport) => void): WidgetReportQueue {
  let pending: WidgetWindowReport | undefined;
  let timer: ReturnType<typeof globalThis.setTimeout> | undefined;
  let closed = false;
  return {
    report(value) {
      if (closed) return;
      pending = parseWidgetWindowReport(value);
      if (timer !== undefined) return;
      timer = globalThis.setTimeout(() => {
        timer = undefined;
        const report = pending;
        pending = undefined;
        if (!closed && report) send(report);
      }, 100);
    },
    close() {
      closed = true;
      pending = undefined;
      clearTimeout(timer);
      timer = undefined;
    },
  };
}

/** Native IPC cannot cancel an issued mutation. Aborting stops accepting its result. */
export function widgetAbortable<T>(operation: () => Promise<T>, signal?: AbortSignal): Promise<T> {
  try { signal?.throwIfAborted(); } catch (error) { return Promise.reject(error); }
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((accept, fail) => { resolve = accept; reject = fail; });
  const abort = () => reject(signal?.reason ?? new DOMException("Aborted", "AbortError"));
  signal?.addEventListener("abort", abort, { once: true });
  operation().then((value) => {
    if (!signal?.aborted) resolve(value);
  }, reject).finally(() => signal?.removeEventListener("abort", abort));
  return promise;
}
