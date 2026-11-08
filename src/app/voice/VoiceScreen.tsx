import { useEffect, useRef, useState } from "react";
import { useVoiceSession, type VoiceTransport } from "./useVoiceSession";
import "./voice.css";

export type VoiceChoice = { id: string; label: string };

export type VoiceScreenProps = {
  transport: VoiceTransport;
  sessionId: string | null;
  supervisorName?: string | null;
  projectName?: string | null;
  roots: VoiceChoice[];
  projects: VoiceChoice[];
  workspaces: VoiceChoice[];
  selectedRootId: string | null;
  selectedProjectId: string | null;
  selectedWorkspaceId: string | null;
  onRootChange: (id: string | null) => void;
  onProjectChange: (id: string | null) => void;
  onWorkspaceChange: (id: string | null) => void;
  onStopReady?: (stop: (() => Promise<void>) | null) => void;
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

function VoiceConversation({ transport, sessionId, supervisorName, roots, projects, workspaces, selectedRootId, selectedProjectId, selectedWorkspaceId, onRootChange, onProjectChange, onWorkspaceChange, onStopReady }: VoiceScreenProps) {
  const { status, phase, transcript, handoffs, error, endReason, remoteStream, muted, start, stop, setMuted } = useVoiceSession(transport);
  const audioRef = useRef<HTMLAudioElement>(null);
  const transcriptRef = useRef<HTMLDivElement>(null);
  const followRef = useRef(true);
  const [hasNewer, setHasNewer] = useState(false);
  const [playbackBlocked, setPlaybackBlocked] = useState(false);
  const [speakerMuted, setSpeakerMuted] = useState(false);
  const active = status === "starting" || status === "active";
  const invalidSelection = (selectedRootId !== null && !roots.some((choice) => choice.id === selectedRootId)) ||
    (selectedProjectId !== null && !projects.some((choice) => choice.id === selectedProjectId)) ||
    (selectedWorkspaceId !== null && !workspaces.some((choice) => choice.id === selectedWorkspaceId));

  useEffect(() => {
    onStopReady?.(stop);
    return () => onStopReady?.(null);
  }, [onStopReady, stop]);

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;
    let current = true;
    audio.srcObject = remoteStream;
    setPlaybackBlocked(false);
    if (remoteStream) {
      void audio.play().then(() => {
        if (current) setPlaybackBlocked(false);
      }).catch(() => {
        if (current) setPlaybackBlocked(true);
      });
    }
    return () => {
      current = false;
      audio.pause();
      audio.srcObject = null;
    };
  }, [remoteStream]);

  useEffect(() => {
    const element = transcriptRef.current;
    if (!element) return;
    if (followRef.current) element.scrollTop = element.scrollHeight;
    else setHasNewer(true);
  }, [transcript, handoffs]);

  const retryPlayback = () => {
    const audio = audioRef.current;
    if (!audio || !remoteStream) return;
    void audio.play().then(() => setPlaybackBlocked(false)).catch(() => setPlaybackBlocked(true));
  };
  const statusText = status === "starting" ? "Connecting · waiting for microphone permission if requested"
    : status === "active" ? muted ? "Microphone muted" : "Microphone on"
    : status === "error" ? "Call ended with an error"
    : status === "stopped" ? "Call ended" : roots.length === 0 ? "Supervisor unavailable" : "Ready to start";

  return <section className="voice-root" aria-label="Voice">
    <aside className="voice-context" aria-label="Call context">
      <h1>Voice</h1>
      <div className="voice-context-section">
        <h2>Talking to</h2>
        <p>{supervisorName || "Supervisor"}</p>
        <p className="voice-secondary">Session: {sessionId || "No session selected"}</p>
        <label className="voice-select">Supervisor root
          <select value={selectedRootId ?? ""} disabled={active} onChange={(event) => onRootChange(event.target.value || null)}>
            <option value="">No root selected</option>
            {selectedRootId && !roots.some((choice) => choice.id === selectedRootId) && <option value={selectedRootId}>Selected root unavailable</option>}
            {roots.map((choice) => <option key={choice.id} value={choice.id}>{choice.label}</option>)}
          </select>
        </label>
        <label className="voice-select">Project context
          <select value={selectedProjectId ?? ""} disabled={active} onChange={(event) => onProjectChange(event.target.value || null)}>
            <option value="">No project selected</option>
            {selectedProjectId && !projects.some((choice) => choice.id === selectedProjectId) && <option value={selectedProjectId}>Selected project unavailable</option>}
            {projects.map((choice) => <option key={choice.id} value={choice.id}>{choice.label}</option>)}
          </select>
        </label>
        <label className="voice-select">Space context
          <select value={selectedWorkspaceId ?? ""} disabled={active} onChange={(event) => onWorkspaceChange(event.target.value || null)}>
            <option value="">No Space selected</option>
            {selectedWorkspaceId && !workspaces.some((choice) => choice.id === selectedWorkspaceId) && <option value={selectedWorkspaceId}>Selected Space unavailable</option>}
            {workspaces.map((choice) => <option key={choice.id} value={choice.id}>{choice.label}</option>)}
          </select>
        </label>
        {active && <p className="voice-secondary">Selection is fixed for this call.</p>}
      </div>
      <div className="voice-context-section voice-privacy">
        <h2>Privacy</h2>
        <p>Starting a call asks for microphone access. Your browser or app may request permission.</p>
        <p>Audio is sent to OpenAI. Cockpit receives conversation text and handoff requests. OpenAI audio retention is not known to Cockpit.</p>
        <p>The transcript is kept on this screen while it remains open. Ending a call does not cancel work already handed off.</p>
      </div>
    </aside>
    <div className="voice-conversation">
      <header className="voice-header">
        <div><strong>{supervisorName || "Supervisor"}</strong><span className="voice-header-session">{sessionId ? `Session: ${sessionId}` : "Select a session to start"}</span></div>
        <span className="voice-header-status" role="status">{statusText}</span>
      </header>
      <div className="voice-transcript-wrap">
        <div className="voice-transcript" role="region" aria-label="Conversation transcript" tabIndex={0} ref={transcriptRef} onScroll={(event) => {
          const element = event.currentTarget;
          followRef.current = element.scrollHeight - element.scrollTop - element.clientHeight < 48;
          if (followRef.current) setHasNewer(false);
        }}>
          {transcript.length === 0 && handoffs.length === 0 ? <p className="voice-empty">Your conversation and handoff receipts will appear here after you start.</p> : null}
          {transcript.map((entry) => <div className="voice-transcript-row" key={`${entry.generation}:${entry.role}:${entry.turn}`}>
            <div className="voice-speaker">{entry.role === "user" ? "You" : "Voice agent"}{entry.role === "assistant" && <span className="voice-ai">AI</span>}</div>
            <div className={entry.final ? "voice-utterance" : "voice-utterance voice-partial"}>{entry.text}{!entry.final && <span className="voice-partial-label"> (partial)</span>}</div>
          </div>)}
          {handoffs.length > 0 && <section className="voice-handoffs" aria-label="Handoff receipts"><h2>Handoffs</h2>{handoffs.map((receipt, index) => {
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
      <div className="voice-dock">
        {error && <p className="voice-notice voice-error" role="alert">{error}</p>}
        {endReason && <p className="voice-notice" role="status">{endReason}</p>}
        {playbackBlocked && remoteStream && <p className="voice-notice" role="alert">Audio playback was blocked. <button type="button" onClick={retryPlayback}>Play audio</button></p>}
        <div className="voice-dock-status"><span className="voice-state-dot" data-live={status === "active" && !muted} aria-hidden="true" /><span>{statusText}{status === "active" && phase ? ` · ${phase}` : ""}</span></div>
        {status === "active" && muted && <p className="voice-hint">The microphone remains reserved until the call ends.</p>}
        {!sessionId && <p className="voice-hint">Select a session before starting voice.</p>}
        {!active && roots.length === 0 && <p className="voice-hint">Start a supervisor in Workbench first.</p>}
        {invalidSelection && <p className="voice-hint">The selected root, project or Space is unavailable. Choose another selection before starting.</p>}
        <div className="voice-actions">
          {!active ? <button type="button" className="voice-primary" disabled={!sessionId || invalidSelection || roots.length === 0} onClick={() => void start()}>{status === "idle" ? "Start voice call" : "Start new call"}</button> : <>
            {status === "active" && <><button type="button" aria-label="Mute microphone" aria-pressed={muted} onClick={() => setMuted(!muted)}>{muted ? "Unmute" : "Mute"}</button><button type="button" aria-label="Mute speaker" aria-pressed={speakerMuted} onClick={() => setSpeakerMuted(!speakerMuted)}>{speakerMuted ? "Turn speaker on" : "Turn speaker off"}</button></>}
            <button type="button" onClick={() => void stop().catch(() => undefined)}>{status === "starting" ? "Cancel" : "End call"}</button>
          </>}
        </div>
        <audio ref={audioRef} autoPlay playsInline muted={speakerMuted} />
      </div>
    </div>
  </section>;
}

/** A session switch tears down the previous call and never relabels its transcript. */
export function VoiceScreen(props: VoiceScreenProps) {
  return <VoiceConversation key={props.sessionId ?? ""} {...props} />;
}
