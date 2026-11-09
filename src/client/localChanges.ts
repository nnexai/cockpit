import type { LocalChangeAck, LocalChangeEvent, LocalChangeScope, LocalChangeTopic } from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";
import type { CockpitClient } from "./CockpitClient";

const encoder = new TextEncoder();
const topics: Record<LocalChangeTopic, true> = { binding: true, scratchpad: true, todos: true, decisions: true, comments: true, listing: true, follow_status: true, files: true, index: true, space_git: true, space_tags: true, orchestration: true };
const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const exact = (value: Record<string, unknown>, keys: string[]) => Object.keys(value).every(key => keys.includes(key));
const bounded = (value: unknown, max: number): value is string => typeof value === "string" && value.length > 0 && encoder.encode(value).length <= max && !/[\u0000-\u001f\u007f]/.test(value);

/** Validate the outbound frame size before allocating a transport. The host reauthorizes the scope. */
export function localChangeScopeFrame(scope: LocalChangeScope): string {
  const frame = JSON.stringify(scope);
  if (encoder.encode(frame).length > 256 * 1024) throw new CockpitClientError("malformed_response", "Local changes scope exceeds transport bounds");
  return frame;
}

export function parseLocalChangeEvent(value: unknown): LocalChangeEvent {
  const invalid = () => new CockpitClientError("malformed_response", "Local changes event is malformed");
  if (!record(value) || !exact(value, ["stream_id", "sequence", "kind", "topics", "reason"]) || !bounded(value.stream_id, 256)
    || !Number.isSafeInteger(value.sequence) || (value.sequence as number) <= 0
    || !["ready", "changed", "unavailable"].includes(value.kind as string)
    || !Array.isArray(value.topics) || value.topics.length > 12
    || value.topics.some(topic => typeof topic !== "string" || !Object.hasOwn(topics, topic)) || new Set(value.topics).size !== value.topics.length) throw invalid();
  if (value.kind === "unavailable") {
    if (!record(value.reason) || !exact(value.reason, ["code", "message"]) || !bounded(value.reason.code, 64) || !bounded(value.reason.message, 512)) throw invalid();
  } else if ("reason" in value) throw invalid();
  if (encoder.encode(JSON.stringify(value)).length > 4096) throw invalid();
  return value as unknown as LocalChangeEvent;
}

/** One event credit, validated independently of the transport. */
export function localChangeReceiver(
  onEvent: (event: LocalChangeEvent) => Promise<void>,
  acknowledge: (ack: LocalChangeAck) => Promise<void>,
  fail: (error: CockpitClientError) => void,
  ready: () => void,
): (value: unknown) => void {
  let streamId: string | undefined;
  let sequence = 0;
  let outstanding = false;
  return (raw) => {
    try {
      const event = parseLocalChangeEvent(raw);
      if (outstanding || (streamId === undefined ? event.sequence !== 1 || !["ready", "unavailable"].includes(event.kind)
        : event.stream_id !== streamId || event.sequence !== sequence + 1 || event.kind === "ready")) {
        throw new CockpitClientError("malformed_response", "Local changes stream ordering is invalid");
      }
      streamId = event.stream_id;
      sequence = event.sequence;
      outstanding = true;
      ready();
      void Promise.resolve().then(() => onEvent(event)).then(() => {
        // Native delivery can race the ACK command's response; credit is issued now.
        outstanding = false;
        return acknowledge({ stream_id: event.stream_id, sequence: event.sequence });
      }).then(() => {
        if (event.kind === "unavailable") fail(new CockpitClientError("stream_error", event.reason!.message));
      }).catch(cause => fail(cause instanceof CockpitClientError ? cause : new CockpitClientError("stream_error", "Local changes consumer or acknowledgement failed", { cause })));
    } catch (cause) {
      fail(cause instanceof CockpitClientError ? cause : new CockpitClientError("malformed_response", "Local changes event is malformed", { cause }));
    }
  };
}

/** Admission includes subscriptions still opening; failure, cancellation and close release it. */
export function limitLocalChangeSubscriptions(open: CockpitClient["subscribeLocalChanges"]): CockpitClient["subscribeLocalChanges"] {
  let count = 0;
  return async (scope, onEvent, onError, signal) => {
    if (count >= 32) throw new CockpitClientError("stream_error", "Too many local changes subscriptions");
    count++;
    let released = false;
    const release = () => {
      if (released) return;
      released = true;
      count--;
      signal?.removeEventListener("abort", release);
    };
    signal?.addEventListener("abort", release, { once: true });
    if (signal?.aborted) release();
    try {
      const stream = await open(scope, onEvent, error => { release(); onError(error); }, signal);
      return { close: () => { release(); stream.close(); } };
    } catch (error) {
      release();
      throw error;
    }
  };
}
