import { useCallback, useEffect, useRef, useState } from "react";
import { createVoicePeer, type VoicePeer } from "./voicePeer";

export type VoiceTranscriptEntry = {
  generation: number;
  role: "user" | "assistant";
  turn: string | number;
  text: string;
  final: boolean;
};

/** Transcript text is the current complete text for the turn, not a delta. */
export type VoiceTransportEvent =
  | { type: "phase"; phase: string }
  | { type: "transcript"; role: VoiceTranscriptEntry["role"]; turn: string | number; text: string; final: boolean }
  | { type: "handoff"; receipt: unknown }
  | { type: "ended"; reason?: string; error?: string };

export type VoiceTransport = {
  connect(offer: string, signal: AbortSignal): Promise<string>;
  setMuted(muted: boolean): void | Promise<void>;
  /** Stops the voice bridge only; it must not cancel work handed to the supervisor. */
  stop(): void | Promise<void>;
  subscribe(callback: (event: VoiceTransportEvent) => void): () => void;
};

export type VoiceSessionStatus = "idle" | "starting" | "active" | "stopped" | "error";

type Session = {
  generation: number;
  transport: VoiceTransport;
  peer: VoicePeer | null;
  unsubscribe: (() => void) | null;
  connect: Promise<string> | null;
  connectPending: boolean;
  connected: boolean;
  muteVersion: number;
};

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function useVoiceSession(transport: VoiceTransport) {
  const [status, setStatus] = useState<VoiceSessionStatus>("idle");
  const [phase, setPhase] = useState<string | null>(null);
  const [transcript, setTranscript] = useState<VoiceTranscriptEntry[]>([]);
  const [handoffs, setHandoffs] = useState<unknown[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [endReason, setEndReason] = useState<string | null>(null);
  const [remoteStream, setRemoteStream] = useState<MediaStream | null>(null);
  const [muted, setMutedState] = useState(false);
  const mutedRef = useRef(false);
  const mounted = useRef(true);
  const generation = useRef(0);
  const current = useRef<Session | null>(null);
  const cleanup = useRef<Promise<void>>(Promise.resolve());

  const finish = useCallback((session: Session, outcome: "stopped" | "error", message?: string, reason?: string) => {
    if (current.current !== session) return cleanup.current;
    current.current = null;
    const endedGeneration = ++generation.current;
    session.peer?.stop();
    session.unsubscribe?.();
    session.unsubscribe = null;
    if (mounted.current) {
      setStatus(outcome);
      setError(message ?? null);
      setEndReason(reason ?? null);
      setPhase(null);
      setRemoteStream(null);
    }

    // Stop promptly, then fence a connect that ignored abort before another start.
    // Its late completion could otherwise recreate a bridge after stop returned.
    const pendingConnect = session.connectPending ? session.connect : null;
    const previous = cleanup.current;
    const stopping = (async () => {
      await previous.catch(() => undefined);
      let failure: unknown;
      try { await session.transport.stop(); } catch (cause) { failure = cause; }
      if (pendingConnect) {
        await pendingConnect.catch(() => undefined);
        try { await session.transport.stop(); } catch (cause) { failure ??= cause; }
      }
      if (failure !== undefined) throw failure;
    })();
    cleanup.current = stopping;
    void stopping.catch((cause: unknown) => {
      if (mounted.current && generation.current === endedGeneration) {
        setStatus("error");
        setError(`Could not stop voice: ${errorMessage(cause)}`);
      }
    });
    return stopping;
  }, []);

  const stop = useCallback(async () => {
    const session = current.current;
    if (session) await finish(session, "stopped");
    else await cleanup.current;
  }, [finish]);

  const syncMute = useCallback((session: Session, value: boolean) => {
    const version = ++session.muteVersion;
    void Promise.resolve().then(() => {
      if (current.current !== session) return;
      return session.transport.setMuted(value);
    }).then(() => {
      if (current.current !== session || session.muteVersion !== version) return;
      session.peer?.setMuted(value);
      setMutedState(value);
    }, (cause: unknown) => {
      if (current.current !== session) return;
      setError(`Could not update voice mute: ${errorMessage(cause)}`);
      if (session.muteVersion === version) {
        mutedRef.current = true;
        session.peer?.setMuted(true);
        setMutedState(true);
      }
    });
  }, []);

  const start = useCallback(async () => {
    if (current.current) return;
    const session: Session = {
      generation: ++generation.current, transport, peer: null, unsubscribe: null,
      connect: null, connectPending: false, connected: false, muteVersion: 0,
    };
    current.current = session;
    const isCurrent = () => mounted.current && current.current === session;
    setStatus("starting");
    setError(null);
    setEndReason(null);
    setPhase(null);
    setRemoteStream(null);
    try {
      await cleanup.current;
      if (!isCurrent()) return;
      if (typeof document !== "undefined" && document.visibilityState === "hidden") {
        finish(session, "stopped", undefined, "Voice stopped because the app is hidden.");
        return;
      }
      const unsubscribe = transport.subscribe((event) => {
        if (!isCurrent()) return;
        switch (event.type) {
          case "phase": setPhase(event.phase); break;
          case "transcript":
            setTranscript((entries) => {
              const index = entries.findIndex((entry) => entry.generation === session.generation && entry.role === event.role && entry.turn === event.turn);
              if (index < 0) return [...entries, { generation: session.generation, role: event.role, turn: event.turn, text: event.text, final: event.final }];
              if (entries[index].final) return entries;
              const next = entries.slice();
              next[index] = { ...entries[index], text: event.text, final: event.final };
              return next;
            });
            break;
          case "handoff": setHandoffs((receipts) => [...receipts, event.receipt]); break;
          case "ended":
            void finish(session, event.error ? "error" : "stopped", event.error, event.reason);
            break;
        }
      });
      if (!isCurrent()) { unsubscribe(); return; }
      session.unsubscribe = unsubscribe;
      session.peer = createVoicePeer({
        signalOffer: (offer, signal) => {
          session.connectPending = true;
          const request = Promise.resolve().then(() => transport.connect(offer, signal));
          session.connect = request;
          void request.then(() => {
            session.connectPending = false;
            if (isCurrent()) {
              session.connected = true;
              syncMute(session, mutedRef.current);
            }
          }, () => { session.connectPending = false; });
          return request;
        },
        onConnectionStateChange: (state) => {
          if (!isCurrent()) return;
          if (state === "connected") setStatus("active");
          else if (state === "failed" || state === "closed" || state === "disconnected") {
            void finish(session, "error", `Voice peer connection ${state}.`);
          }
        },
        onRemoteStream: (stream) => { if (isCurrent()) setRemoteStream(stream); },
        onError: (cause) => { if (isCurrent()) void finish(session, "error", errorMessage(cause)); },
      });
      session.peer.setMuted(true);
      await session.peer.start();
    } catch (cause) {
      if (isCurrent()) void finish(session, "error", errorMessage(cause));
    }
  }, [finish, syncMute, transport]);

  const setMuted = useCallback((value: boolean) => {
    mutedRef.current = value;
    const session = current.current;
    if (!session) {
      setMutedState(value);
      return;
    }
    // Keep the microphone closed until the remote control acknowledges unmute.
    session.peer?.setMuted(true);
    setMutedState(true);
    if (session.connected) syncMute(session, value);
  }, [syncMute]);

  useEffect(() => {
    mounted.current = true;
    const onVisibility = () => {
      if (document.visibilityState === "hidden" && current.current) {
        void finish(current.current, "stopped", undefined, "Voice stopped because the app is hidden.");
      }
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      if (current.current) void finish(current.current, "stopped");
      mounted.current = false;
    };
  }, [finish, transport]);

  return { status, phase, transcript, handoffs, error, endReason, remoteStream, muted, start, stop, setMuted };
}
