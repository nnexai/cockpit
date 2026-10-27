import { useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { UiIcon } from "../UiIcon";
import { trapModalTab, useModalFocus } from "../input/modal";
import { QUICK_ASK_AGENTS } from "./quickAsk";
import type { QuickAskState } from "./useQuickAsk";
import "./quickAsk.css";

const MAX_INPUT_HEIGHT = 240;

export function QuickAskDialog({ state, onClose }: { state: QuickAskState; onClose(): void }) {
  const { harness, installed, question, cwd, busy, error } = state;
  const dialogRef = useModalFocus<HTMLElement>(onClose);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [editingCwd, setEditingCwd] = useState(false);
  const agents = QUICK_ASK_AGENTS.filter(agent => installed === null ? agent.id === harness : installed.includes(agent.id));
  const label = QUICK_ASK_AGENTS.find(agent => agent.id === harness)?.label ?? "agent";
  const noAgent = installed !== null && installed.length === 0;
  useLayoutEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    input.style.height = "auto";
    input.style.height = `${Math.min(input.scrollHeight, MAX_INPUT_HEIGHT)}px`;
  }, [question]);
  const keyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Enter" && !event.shiftKey && !event.altKey && event.target === inputRef.current) {
      event.preventDefault();
      if (!event.repeat && !noAgent) state.ask();
      return;
    }
    trapModalTab(event, dialogRef.current);
  };
  const agentKey = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = agents[(index + step + agents.length) % agents.length]!;
    state.setHarness(next.id);
    event.currentTarget.parentElement?.querySelector<HTMLButtonElement>(`[data-agent="${next.id}"]`)?.focus();
  };
  const finishCwd = (focusInput: boolean) => { setEditingCwd(false); if (focusInput) inputRef.current?.focus(); };
  return <div className="overlay-scrim" role="presentation" onPointerDown={event => { if (event.target === event.currentTarget) onClose(); }}>
    <section ref={dialogRef} className="command-overlay quick-ask" role="dialog" aria-modal="true" aria-label="Ask an agent" aria-busy={busy || undefined} onKeyDown={keyDown}>
      <div className="command-search-box quick-ask-box">
        <UiIcon name="comment" />
        <textarea ref={inputRef} autoFocus rows={1} value={question} readOnly={busy} aria-label={`Question for ${label}`} placeholder={`Ask ${label}…`} onChange={event => state.setQuestion(event.target.value)} />
      </div>
      <div className="quick-ask-meta">
        {agents.length > 1 ? <div className="quick-ask-agents" role="radiogroup" aria-label="Agent">
          {agents.map((agent, index) => <button key={agent.id} type="button" role="radio" data-agent={agent.id} aria-checked={agent.id === harness} tabIndex={agent.id === harness ? 0 : -1} className="quick-ask-agent" disabled={busy} onClick={() => { state.setHarness(agent.id); inputRef.current?.focus(); }} onKeyDown={event => agentKey(event, index)}>{agent.label}</button>)}
        </div> : <span className="quick-ask-agent-single">{agents[0]?.label ?? "No agent"}</span>}
        {editingCwd
          ? <input className="quick-ask-cwd-input" autoFocus aria-label="Folder" value={cwd} spellCheck={false} onChange={event => state.setCwd(event.target.value)} onBlur={() => finishCwd(false)}
            onKeyDown={event => { if (event.key === "Enter" || event.key === "Escape") { event.preventDefault(); event.stopPropagation(); finishCwd(true); } }} />
          : <button type="button" className="quick-ask-cwd" title={`Runs in ${cwd || "…"} · click to change`} disabled={busy} onClick={() => setEditingCwd(true)}><UiIcon name="folder" /><span><bdi>{cwd || "Choose a folder"}</bdi></span></button>}
      </div>
      <footer className={`command-footer quick-ask-footer${error || noAgent ? " is-error" : ""}`} role={error || noAgent ? "alert" : undefined}>
        {error ?? (noAgent ? "No supported agent CLI found on PATH." : busy ? `Starting ${label}…` : "Enter ask · Shift+Enter new line · Esc close")}
      </footer>
    </section>
  </div>;
}
