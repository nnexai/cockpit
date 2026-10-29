import { useRef, useState, type KeyboardEvent } from "react";
import { trapModalTab, useModalFocus } from "../input/modal";
import "./quickAsk.css";

export type SendAgentTarget = { paneId: string; label: string; space: string };
type Result = { status: "sent" } | { status: "refused"; message: string };

/** Targets come from the current live session; the host rechecks each target. */
export function SendAgentDialog({ targets: currentTargets, onSend, onClose }: {
  targets: readonly SendAgentTarget[];
  onSend(paneId: string, text: string): Promise<void>;
  onClose(): void;
}) {
  const [targets] = useState(() => currentTargets);
  const [selected, setSelected] = useState(() => new Set(targets.map(target => target.paneId)));
  const [text, setText] = useState("");
  const [results, setResults] = useState<Record<string, Result>>({});
  const [busy, setBusy] = useState(false);
  const lock = useRef(false);
  const dialog = useModalFocus<HTMLElement>(() => { if (!lock.current) onClose(); });
  const pending = targets.filter(target => selected.has(target.paneId) && !results[target.paneId]);
  const send = async () => {
    if (lock.current || !text.trim() || !pending.length) return;
    lock.current = true; setBusy(true);
    for (const target of pending) {
      try {
        await onSend(target.paneId, text);
        setResults(current => ({ ...current, [target.paneId]: { status: "sent" } }));
      } catch (failure) {
        const message = failure instanceof Error ? failure.message : String(failure);
        setResults(current => ({ ...current, [target.paneId]: { status: "refused", message } }));
      }
    }
    lock.current = false; setBusy(false);
  };
  const keyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Enter" && !event.shiftKey && !event.altKey && event.target instanceof HTMLTextAreaElement) {
      event.preventDefault(); if (!event.repeat) void send(); return;
    }
    trapModalTab(event, dialog.current);
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={event => { if (event.target === event.currentTarget && !busy) onClose(); }}>
    <section ref={dialog} className="command-overlay quick-ask" role="dialog" aria-modal="true" aria-label="Send to idle agents" aria-busy={busy || undefined} onKeyDown={keyDown}>
      <div className="quick-ask-meta" style={{ display: "block", maxHeight: 240, overflowY: "auto" }}>
        {targets.length === 0 ? "No idle agents available." : targets.map(target => {
          const result = results[target.paneId];
          return <div key={target.paneId}>
            <label><input type="checkbox" checked={selected.has(target.paneId)} disabled={busy || Boolean(result)} onChange={event => setSelected(current => { const next = new Set(current); if (event.target.checked) next.add(target.paneId); else next.delete(target.paneId); return next; })} /> {target.label} · {target.space}</label>
            {result && <span role={result.status === "refused" ? "alert" : undefined}> — {result.status === "sent" ? "Sent" : `Not sent or delivery uncertain: ${result.message}`}</span>}
          </div>;
        })}
      </div>
      <div className="command-search-box quick-ask-box"><textarea aria-label="Message" placeholder="Message idle agents…" value={text} readOnly={busy} onChange={event => setText(event.target.value)} /></div>
      <div className="quick-ask-meta">
        <button type="button" onClick={() => void send()} disabled={busy || !text.trim() || !pending.length}>Send to {pending.length} {pending.length === 1 ? "agent" : "agents"}</button>
        <button type="button" onClick={onClose} disabled={busy}>Close</button>
      </div>
      <footer className="command-footer quick-ask-footer">{busy ? "Sending…" : "Enter send · Shift+Enter new line · Esc close"}</footer>
    </section>
  </div>;
}
