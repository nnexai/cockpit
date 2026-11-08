import type { VoiceTransport, VoiceTransportEvent } from "../app/voice/useVoiceSession";
import type {
  VoiceConnectRequest, VoiceConnectResponse, VoiceControlRequest,
  VoiceEventsRequest, VoiceEventsResponse,
} from "./voiceProtocol";

/** Connect is deliberately not abortable: its response identifies a call that must be stopped. */
export interface VoiceTransportApi {
  connect(request: VoiceConnectRequest): Promise<VoiceConnectResponse>;
  events(request: VoiceEventsRequest, signal: AbortSignal): Promise<VoiceEventsResponse>;
  control(request: VoiceControlRequest): Promise<void>;
}
export interface VoiceTransportSelection {
  selectedRootId?: VoiceConnectRequest["selected_root_id"];
  projectScope?: VoiceConnectRequest["project_scope"];
}


type Call = {
  controller: AbortController;
  stopped: boolean;
  identity?: { call_id: string; generation: number };
  connect?: Promise<string>;
  remoteStop?: Promise<void>;
  muteQueue?: Promise<void>;
  stopping?: Promise<void>;
  detachAbort: () => void;
};

function aborted(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("Voice connection aborted", "AbortError");
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function createVoiceTransport(
  sessionId: string,
  api: VoiceTransportApi,
  selection: VoiceTransportSelection = {},
): VoiceTransport {
  const selectedRootId = selection.selectedRootId;
  const projectScope = selection.projectScope == null ? selection.projectScope : { ...selection.projectScope };
  const subscribers = new Set<(event: VoiceTransportEvent) => void>();
  let current: Call | undefined;

  const emit = (event: VoiceTransportEvent) => {
    for (const callback of subscribers) callback(event);
  };

  const remoteStop = (call: Call): Promise<void> => {
    if (!call.identity) return Promise.resolve();
    // Cache even a failed request: an unknown mutation outcome must never be retried.
    return call.remoteStop ??= Promise.resolve().then(() => api.control({
      ...call.identity!, control: { type: "stop" },
    }));
  };

  const stopCall = (call: Call): Promise<void> => {
    if (call.stopping) return call.stopping;
    call.stopped = true;
    call.controller.abort();
    call.detachAbort();
    call.stopping = (async () => {
      // Connect may still acquire a remote call after cancellation.
      await call.connect?.catch(() => undefined);
      await remoteStop(call);
    })().finally(() => {
      if (current === call) current = undefined;
    });
    return call.stopping;
  };

  const poll = async (call: Call) => {
    const identity = call.identity!;
    let cursor = 0;
    try {
      while (!call.stopped) {
        const response = await api.events({ ...identity, after_seq: cursor, wait_ms: 25000 }, call.controller.signal);
        if (call.stopped) return;
        if (!Number.isSafeInteger(response.next_seq) || response.next_seq < cursor) {
          throw new Error("Invalid voice event cursor");
        }
        let last = cursor;
        const fresh: typeof response.events = [];
        for (const event of response.events) {
          if (event.call_id !== identity.call_id || event.generation !== identity.generation) {
            throw new Error("Stale voice event identity");
          }
          if (!Number.isSafeInteger(event.seq) || event.seq < 0) throw new Error("Invalid voice event sequence");
          if (event.seq <= cursor || event.seq === last) continue;
          if (event.seq !== last + 1) throw new Error("Voice event sequence lost or out of order");
          last = event.seq;
          fresh.push(event);
        }
        if (response.next_seq !== last) throw new Error("Invalid voice event cursor");
        for (const event of fresh) {
          switch (event.kind) {
            case "phase": emit({ type: "phase", phase: event.phase }); break;
            case "transcript": emit({ type: "transcript", role: event.role, turn: event.turn, text: event.text, final: event.final }); break;
            case "handoff": emit({ type: "handoff", receipt: event.receipt }); break;
            case "ended":
              emit({ type: "ended", reason: event.reason ?? undefined, error: event.error ?? undefined });
              return;
          }
          if (call.stopped) return;
        }
        cursor = response.next_seq;
        if (response.ended) {
          emit({ type: "ended" });
          return;
        }
      }
    } catch (error) {
      if (!call.stopped) emit({ type: "ended", error: `Voice event stream failed: ${message(error)}` });
    }
  };

  return {
    connect(offer, signal) {
      if (current) return Promise.reject(new Error("Voice transport already has a call"));
      if (signal.aborted) return Promise.reject(aborted(signal));
      const call: Call = { controller: new AbortController(), stopped: false, detachAbort: () => undefined };
      current = call;
      const onAbort = () => {
        void stopCall(call).catch((error: unknown) => {
          emit({ type: "ended", error: `Could not stop voice: ${message(error)}` });
        });
      };
      signal.addEventListener("abort", onAbort, { once: true });
      call.detachAbort = () => signal.removeEventListener("abort", onAbort);
      call.connect = Promise.resolve().then(async () => {
        try {
          if (call.stopped || signal.aborted) throw aborted(signal);
          const response = await api.connect({
            session_id: sessionId,
            sdp_offer: offer,
            ...(selectedRootId === undefined ? {} : { selected_root_id: selectedRootId }),
            ...(projectScope === undefined ? {} : { project_scope: projectScope }),
          });
          call.identity = { call_id: response.call_id, generation: response.generation };
          if (call.stopped || signal.aborted) {
            await remoteStop(call);
            throw aborted(signal);
          }
          void poll(call);
          return response.sdp_answer;
        } catch (error) {
          if (!call.stopped) {
            call.detachAbort();
            if (current === call) current = undefined;
          }
          throw error;
        }
      });
      return call.connect;
    },
    setMuted(muted) {
      const call = current;
      if (!call || call.stopped) return Promise.reject(new Error("Voice call is not active"));
      const operation = (call.muteQueue ?? Promise.resolve()).catch(() => undefined).then(async () => {
        await call.connect;
        if (call.stopped || !call.identity) throw new Error("Voice call is not active");
        await api.control({ ...call.identity, control: { type: "mute", muted } });
      });
      call.muteQueue = operation;
      return operation;
    },
    stop() {
      return current ? stopCall(current) : Promise.resolve();
    },
    subscribe(callback) {
      subscribers.add(callback);
      return () => { subscribers.delete(callback); };
    },
  };
}
