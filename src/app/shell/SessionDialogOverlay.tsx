import { useEffect, useRef, useState } from "react";
import type { SessionSummary } from "../../protocol/generated/v1";
import { trapModalTab, useModalFocus } from "../input/modal";

export function reconcileSessionChoice(sessions: SessionSummary[], selected: string, currentSessionId: string | null): string {
  if (sessions.some((session) => session.id === selected)) return selected;
  if (currentSessionId && sessions.some((session) => session.id === currentSessionId)) return currentSessionId;
  return sessions[0]?.id ?? "";
}

export function SessionDialogOverlay({ sessions, currentSessionId, onRefresh, onDismiss, onSession }: { sessions: SessionSummary[]; currentSessionId: string | null; onRefresh: () => Promise<void>; onDismiss: () => void; onSession: (sessionId: string) => void }) {
  const [sessionId, setSessionId] = useState(() => reconcileSessionChoice(sessions, "", currentSessionId));
  const [query, setQuery] = useState("");
  const selectedSessionRef = useRef<HTMLButtonElement | null>(null);
  const ref = useModalFocus<HTMLFormElement>(onDismiss);
  useEffect(() => setSessionId((selected) => reconcileSessionChoice(sessions, selected, currentSessionId)), [sessions, currentSessionId]);
  const normalized = query.trim().toLocaleLowerCase();
  const filteredSessions = sessions.filter((session) => !normalized || `${session.label} ${session.id} ${session.running ? "running" : "stopped"}`.toLocaleLowerCase().includes(normalized));
  useEffect(() => { selectedSessionRef.current?.scrollIntoView?.({ block: "nearest" }); }, [sessionId, normalized]);
  const selectRelativeSession = (direction: 1 | -1) => {
    if (filteredSessions.length === 0) return;
    const current = filteredSessions.findIndex((session) => session.id === sessionId);
    const next = current < 0 ? (direction === 1 ? 0 : filteredSessions.length - 1) : (current + direction + filteredSessions.length) % filteredSessions.length;
    setSessionId(filteredSessions[next].id);
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><form ref={ref} className="chooser-overlay session-chooser" role="dialog" aria-modal="true" aria-labelledby="session-chooser-title" onSubmit={(event) => { event.preventDefault(); if (sessionId && sessionId !== currentSessionId) onSession(sessionId); onDismiss(); }} onKeyDown={(event) => {
    if (event.ctrlKey && !event.altKey && !event.metaKey && (event.key.toLowerCase() === "n" || event.key.toLowerCase() === "p")) { event.preventDefault(); selectRelativeSession(event.key.toLowerCase() === "n" ? 1 : -1); return; }
    trapModalTab(event, ref.current);
  }}>
    <h2 id="session-chooser-title">Switch session</h2>
    {sessions.length === 0 ? <div className="empty-choice" role="status">No sessions are available.</div> : <><input className="session-search" aria-label="Find a session" placeholder="Find a session…" autoComplete="off" value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={(event) => { if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); selectRelativeSession(event.key === "ArrowDown" ? 1 : -1); } }} /><div className="session-list" role="listbox" aria-label="Session">{filteredSessions.length === 0 ? <p className="empty-choice" role="status">No sessions match.</p> : filteredSessions.map((session) => <button ref={session.id === sessionId ? selectedSessionRef : null} key={session.id} type="button" role="option" aria-selected={session.id === sessionId} data-session-id={session.id} className={`session-choice${session.id === sessionId ? " is-selected" : ""}`} onClick={() => setSessionId(session.id)}><span>{session.label}</span><small>{session.running ? "running" : "stopped"}</small></button>)}</div></>}
    <footer>{sessions.length === 0 ? <button type="button" onClick={() => { void onRefresh().catch(() => undefined); }}>Refresh</button> : null}<button type="button" onClick={onDismiss}>Cancel</button><button type="submit" disabled={!sessionId || sessionId === currentSessionId}>Switch</button></footer>
  </form></div>;
}
