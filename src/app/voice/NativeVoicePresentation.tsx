import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import "./voice.css";
import "./nativeVoicePresentation.css";

type NativeVoiceSnapshot = {
  view_generation: number;
  revision: number;
  status: string;
  phase: string | null;
  muted: boolean;
  context: { session_id: string; selected_root_id: string | null; project_scope: { repository_id: string | null; workspace_id: string | null } | null } | null;
  transcript: { role: string; turn: number; text: string; final: boolean }[];
  handoffs: unknown[];
  error: string | null;
  end_reason: string | null;
  earlier_text_trimmed: boolean;
};

function receiptDescription(receipt: unknown): { label: string; message: string | null } {
  if (!receipt || typeof receipt !== "object") return { label: "Handoff status unavailable", message: null };
  const value = receipt as Record<string, unknown>;
  const status = value.status ?? value.stage;
  const labels: Record<string, string> = {
    accepted: "Handoff recorded · Supervisor processing not confirmed",
    stored: "Stored · waiting to be picked up",
    read: "Delivered · picked up",
    acked: "Captured · processing not confirmed",
    rejected: "Handoff rejected",
    unknown: "Delivery not confirmed · check Inbox before trying again",
    outcome_unknown: "Delivery not confirmed · check Inbox before trying again",
    update: "Supervisor update",
  };
  const entries = Array.isArray(value.entries) ? value.entries : [];
  const update = entries.flatMap((entry: unknown) => {
    if (!entry || typeof entry !== "object") return [];
    const text = (entry as Record<string, unknown>).text;
    return typeof text === "string" ? [text] : [];
  }).join("\n");
  return {
    label: typeof status === "string" && Object.hasOwn(labels, status) ? labels[status] : "Handoff status unavailable",
    message: typeof value.message === "string" ? value.message : update || null,
  };
}

function statusText(snapshot: NativeVoiceSnapshot): string {
  switch (snapshot.status) {
    case "idle": return "Ready to start";
    case "starting": return "Connecting";
    case "active": return snapshot.muted ? "Microphone muted" : "Microphone on";
    case "stopped": return "Call ended";
    case "stopping": return "Ending call";
    case "error": return "Call ended with an error";
    default: return snapshot.status;
  }
}

/** Native owns the call; this view only observes its presentation state. */
export function NativeVoicePresentation() {
  const [snapshot, setSnapshot] = useState<NativeVoiceSnapshot | null>(null);
  const [observationError, setObservationError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const transcriptRef = useRef<HTMLDivElement>(null);
  const followRef = useRef(true);
  const [hasNewer, setHasNewer] = useState(false);
  const contentSignature = snapshot ? JSON.stringify([snapshot.view_generation, snapshot.transcript, snapshot.handoffs]) : null;

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    let newest: NativeVoiceSnapshot | null = null;
    setSnapshot(null);
    setObservationError(null);
    const accept = (next: NativeVoiceSnapshot) => {
      if (!alive) return;
      if (newest && (next.view_generation < newest.view_generation ||
          (next.view_generation === newest.view_generation && next.revision <= newest.revision))) return;
      newest = next;
      setSnapshot(next);
    };
    void (async () => {
      // Subscribe first: a delayed snapshot must not overwrite a newer event.
      const unsubscribe = await listen<NativeVoiceSnapshot>("cockpit-native-voice-presentation", event => accept(event.payload));
      if (!alive) { unsubscribe(); return; }
      unlisten = unsubscribe;
      accept(await invoke<NativeVoiceSnapshot>("cockpit_native_voice_snapshot"));
    })().catch(error => {
      if (alive) setObservationError(error instanceof Error ? error.message : String(error));
    });
    return () => { alive = false; unlisten?.(); };
  }, [attempt]);

  useEffect(() => {
    const element = transcriptRef.current;
    if (!element || contentSignature === null) return;
    if (followRef.current) element.scrollTop = element.scrollHeight;
    else setHasNewer(true);
  }, [contentSignature]);

  return <section className="voice-root native-voice-presentation" aria-label="Voice">
    <div className="voice-conversation">
      <header className="voice-header">
        <div><strong>Supervisor</strong><span className="voice-header-session">{snapshot?.context?.session_id ? `Session: ${snapshot.context.session_id}` : "General conversation"}</span></div>
        <span className="voice-header-status" role="status">{snapshot ? statusText(snapshot) : "Loading voice status…"}{snapshot?.phase ? ` · ${snapshot.phase}` : ""}</span>
      </header>
      {observationError && <div className="voice-dock" role="alert"><p className="voice-notice voice-error">Voice status unavailable: {observationError}</p><button type="button" onClick={() => setAttempt(value => value + 1)}>Retry</button></div>}
      <div className="voice-transcript-wrap">
        <div className="voice-transcript" role="region" aria-label="Conversation transcript" tabIndex={0} ref={transcriptRef} onScroll={event => {
          const element = event.currentTarget;
          followRef.current = element.scrollHeight - element.scrollTop - element.clientHeight < 48;
          if (followRef.current) setHasNewer(false);
        }}>
          {snapshot?.earlier_text_trimmed && <p className="voice-secondary">Earlier transcript text was removed from this view.</p>}
          {snapshot && snapshot.transcript.length === 0 && snapshot.handoffs.length === 0 && <p className="voice-empty">Your conversation and handoff receipts will appear here after you start.</p>}
          {snapshot?.transcript.map(entry => <div className="voice-transcript-row" key={`${snapshot.view_generation}:${entry.role}:${entry.turn}`}>
            <div className="voice-speaker">{entry.role === "user" ? "You" : "Voice agent"}{entry.role === "assistant" && <span className="voice-ai">AI</span>}</div>
            <div className={entry.final ? "voice-utterance" : "voice-utterance voice-partial"}>{entry.text}{!entry.final && <span className="voice-partial-label"> (partial)</span>}</div>
          </div>)}
          {!!snapshot?.handoffs.length && <section className="voice-handoffs" aria-label="Handoff receipts"><h2>Handoffs</h2>{snapshot.handoffs.map((receipt, index) => {
            if (receipt && typeof receipt === "object" && "status" in receipt && receipt.status === "update") {
              const update = receipt as { thread_id?: unknown; entries?: unknown };
              const entries = Array.isArray(update.entries) ? update.entries : [];
              return <div className="voice-receipt" key={index}>
                <strong>Supervisor thread update</strong>
                {typeof update.thread_id === "string" && <p>Thread: {update.thread_id}</p>}
                {entries.map((entry: unknown, entryIndex: number) => {
                  if (!entry || typeof entry !== "object") return null;
                  const item = entry as { kind?: unknown; text?: unknown };
                  if (typeof item.text !== "string") return null;
                  return <p key={entryIndex}>{typeof item.kind === "string" && <strong>{item.kind}: </strong>}{item.text}</p>;
                })}
              </div>;
            }
            const description = receiptDescription(receipt);
            return <div className="voice-receipt" key={index}><strong>{description.label}</strong>{description.message && <p>{description.message}</p>}</div>;
          })}</section>}
        </div>
        {hasNewer && <button type="button" className="voice-jump" onClick={() => {
          const element = transcriptRef.current;
          if (element) element.scrollTop = element.scrollHeight;
          followRef.current = true;
          setHasNewer(false);
        }}>Jump to latest</button>}
      </div>
      {(snapshot?.error || snapshot?.end_reason) && <div className="voice-dock">
        {snapshot.error && <p className="voice-notice voice-error" role="alert">{snapshot.error}</p>}
        {snapshot.end_reason && <p className="voice-notice" role="status">{snapshot.end_reason}</p>}
      </div>}
    </div>
  </section>;
}
