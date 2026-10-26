import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import type { QuickAskAvailability, QuickAskHarness, QuickAskStatus } from "../../protocol/generated/v1";
import { useModalFocus } from "../input/modal";
import "./quickAsk.css";

export interface QuickAskDialogProps {
  harness: QuickAskHarness | null;
  setHarness(value: QuickAskHarness): void;
  question: string;
  setQuestion(value: string): void;
  cwd: string;
  setCwd(value: string): void;
  cwdSource: string;
  availability: QuickAskAvailability | null;
  status: QuickAskStatus | null;
  error: string | null;
  validation: string | null;
  switchToTab: boolean;
  setSwitchToTab(value: boolean): void;
  unknownAcknowledged: boolean;
  promptConfirmed: boolean;
  promptConfirmationAvailable: boolean;
  setPromptConfirmed(value: boolean): void;
  directLaunch: boolean;
  manualPaste: boolean;
  onClose(): void;
  busy: boolean;
  start(): void;
  cancel(): void;
  send(): void;
  openTab(): void;
  copyQuestion(): void;
  dismissUnknown(): void;
  newQuestion(): void;
}

const AGENTS: { id: QuickAskHarness; label: string; executable: string }[] = [
  { id: "omp", label: "OMP", executable: "omp" },
  { id: "pi", label: "Pi", executable: "pi" },
  { id: "claude", label: "Claude Code", executable: "claude" },
  { id: "copilot", label: "Copilot CLI", executable: "copilot" },
  { id: "codex", label: "Codex CLI", executable: "codex" },
  { id: "opencode", label: "OpenCode", executable: "opencode" },
];

export function QuickAskDialog(props: QuickAskDialogProps) {
  const { harness, question, cwd, status, availability, error, validation } = props;
  const phase = status?.phase;
  const editable = !phase || phase === "failed" || phase === "cancelled";
  const unknown = phase === "outcome_unknown";
  const agent = AGENTS.find(item => item.id === (status?.harness ?? harness));
  const selected = AGENTS.find(item => item.id === harness);
  const selectedAvailable = availability?.agents.find(item => item.harness === harness)?.found === true;
  const launchPrompt = props.directLaunch;
  const manualPaste = props.manualPaste;
  const canStart = editable && selectedAvailable && !validation && !props.busy && Boolean(cwd);
  const dismiss = () => {
    if (status?.cancellable) props.cancel();
    else props.onClose();
  };
  const dialogRef = useModalFocus<HTMLElement>(dismiss);
  const questionRef = useRef<HTMLTextAreaElement>(null);
  const primaryRef = useRef<HTMLButtonElement>(null);
  const [editingCwd, setEditingCwd] = useState(!cwd);
  const titleId = useId();
  const agentId = useId();
  const questionId = useId();
  const hintId = useId();
  const statusId = useId();
  const bytes = new TextEncoder().encode(question).length;
  useEffect(() => { if (editable) questionRef.current?.focus(); else if (unknown) primaryRef.current?.focus(); }, []);
  useEffect(() => {
    if (phase === "failed") questionRef.current?.focus();
    if (phase === "outcome_unknown") primaryRef.current?.focus();
  }, [phase]);

  let message: string | null = error;
  let isError = Boolean(error);
  if (!message && status) {
    switch (phase) {
      case "creating_tab": message = launchPrompt ? "Starting a new tab with your question. Launch cannot be recalled once sent." : "Creating a new tab. Your question has not been sent."; break;
      case "checking_environment": message = "Checking the new tab. Your question has not been sent."; break;
      case "starting_agent": message = launchPrompt ? `Starting ${agent?.label ?? "the agent"} with your question.` : `Starting ${agent?.label ?? "the agent"}. Your question has not been sent.`; break;
      case "waiting_for_agent": message = `${agent?.label ?? "The agent"} needs your input${status.location ? ` in tab “${status.location.tab_label}”` : ""} before it can take the question.`; break;
      case "ready_to_send": message = manualPaste ? "Open the tab and verify the question prompt. Copy your question and paste it there yourself; Cockpit cannot safely send this multiline question." : `${agent?.label ?? "The agent"} detected. Open the tab and check that the agent's question prompt is visible and empty before confirming.`; break;
      case "sending": message = "Sending your question. This cannot be cancelled once sent."; break;
      case "submitted": message = `Herdr acknowledged sending your question${status.location ? ` in tab “${status.location.tab_label}”` : ""}. Check the tab for the agent's response.`; break;
      case "initial_question_supplied": message = `The initial question was supplied at launch${status.location ? ` in tab “${status.location.tab_label}”` : ""}. This does not confirm the agent received it; check the tab before asking again.`; break;
      case "cancelled": message = "Your question was not sent."; break;
      case "failed": message = `${agent?.label ?? "The agent"} didn't get your question${status.error?.message ? `: ${status.error.message}` : "."} Your question is still here.${status.location ? ` The tab “${status.location.tab_label}” remains open.` : ""}`; isError = true; break;
      case "outcome_unknown": message = "Cockpit couldn't confirm whether your question was sent. Open the tab to check before asking again."; isError = true; break;
    }
  }
  if (!message && availability && !availability.agents.some(item => item.found)) message = "No supported agent was found. Install one or make it available on PATH.";
  const keyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing) {
      if (event.key === "Escape") event.stopPropagation();
      return;
    }
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey) && !event.altKey && !event.repeat) {
      event.preventDefault();
      if (canStart) props.start();
      return;
    }
    if (event.key !== "Tab") return;
    const controls = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled), textarea:not(:disabled), summary, [tabindex]:not([tabindex='-1'])") ?? [])
      .filter(element => element.getClientRects().length > 0 && (!(element instanceof HTMLInputElement) || element.type !== "radio" || element.checked || !harness));
    if (!controls.length) return;
    const current = controls.indexOf(document.activeElement as HTMLElement);
    event.preventDefault();
    controls[(current + (event.shiftKey ? -1 : 1) + controls.length) % controls.length]?.focus();
  };

  return <div className="setup-overlay" role="presentation">
    <section ref={dialogRef} className="setup-dialog quick-ask" role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={keyDown}>
      <header className="quick-ask-header"><h2 id={titleId}>Ask an agent</h2><button type="button" className="quick-ask-close" aria-label="Close" onClick={dismiss}>×</button></header>
      <div className="quick-ask-body">
        <div className="quick-ask-row"><span id={agentId} className="quick-ask-label">Agent</span><div className="quick-ask-agents" role="radiogroup" aria-labelledby={agentId}>
          {AGENTS.map(item => {
            const found = availability?.agents.find(entry => entry.harness === item.id)?.found;
            return <label key={item.id} className={`quick-ask-agent${harness === item.id ? " is-selected" : ""}${found !== true ? " is-unavailable" : ""}`}>
              <input type="radio" name={agentId} value={item.id} checked={harness === item.id} disabled={!editable || found !== true} onChange={() => props.setHarness(item.id)} />
              <span><strong>{item.label}</strong><small>{found === undefined ? "Checking…" : found ? item.id : "Not found on PATH"}</small></span>{harness === item.id ? <span aria-hidden="true">✓</span> : null}
            </label>;
          })}
        </div></div>
        <div className="quick-ask-row"><label className="quick-ask-label" htmlFor={questionId}>Question</label><div>
          <textarea ref={questionRef} id={questionId} tabIndex={0} rows={4} value={question} readOnly={!editable} onChange={event => props.setQuestion(event.target.value)} placeholder="Ask anything. It is sent to the agent exactly as typed, once." aria-describedby={hintId} aria-invalid={Boolean(validation)} />
          <div id={hintId} className="quick-ask-hint"><span>Ctrl/Cmd+Enter starts · Enter adds a line</span>{bytes >= 13107 ? <span className={bytes > 16384 ? "is-error" : ""}>{bytes} / 16,384 bytes</span> : null}</div>
          {launchPrompt && editable ? <p className="quick-ask-validation">Your question will be visible in launch process arguments and layout metadata, and may persist there. Do not include secrets.</p> : null}
          {validation ? <p className="quick-ask-validation" role="alert">{validation}</p> : null}
        </div></div>
        <div className="quick-ask-row"><label className="quick-ask-label" htmlFor={`${questionId}-cwd`}>Runs in</label><div className="quick-ask-folder">
          {editingCwd || !cwd ? <><input id={`${questionId}-cwd`} value={cwd} onChange={event => props.setCwd(event.target.value)} disabled={!editable} placeholder="Absolute folder path" />{!cwd ? <p className="quick-ask-hint">This Space has no checkout and the focused terminal reported no folder. Enter an absolute path.</p> : null}</> : <><span className="quick-ask-path">{cwd}</span><span className="quick-ask-hint">{props.cwdSource}</span>{editable ? <button type="button" className="quick-ask-link" onClick={() => setEditingCwd(true)}>Change…</button> : null}</>}
        </div></div>
        <details className="quick-ask-more"><summary>More</summary><label><input type="checkbox" checked={props.switchToTab} disabled={!editable} onChange={event => props.setSwitchToTab(event.target.checked)} /> Switch to the new tab</label><p>{launchPrompt ? <>Starts <code>{selected?.executable}</code> with your question in its launch arguments. Launch acknowledgement does not confirm the agent received it.</> : manualPaste ? <>Starts <code>{selected?.executable}</code> in a new tab. Copy and paste your question manually after checking the agent's prompt.</> : <>Starts <code>{selected?.executable}</code> in a new tab. Check the agent's question input, then confirm Send question to submit it once.</>}</p></details>
        {phase === "ready_to_send" && !manualPaste ? <label><input type="checkbox" checked={props.promptConfirmed} disabled={props.busy || !props.promptConfirmationAvailable} onChange={event => props.setPromptConfirmed(event.target.checked)} /> I confirmed the agent's normal question prompt is visible and empty</label> : null}
      </div>
      <footer className="quick-ask-footer">
        <div id={statusId} className={`quick-ask-status${isError ? " is-error" : ""}`} role={message ? isError ? "alert" : "status" : undefined}>{message ? <><span aria-hidden="true">{isError ? "!" : "•"}</span> {message}</> : null}</div>
        <div className="quick-ask-actions">
          {status?.location && (phase === "waiting_for_agent" || phase === "ready_to_send" || phase === "submitted" || phase === "initial_question_supplied" || phase === "failed") ? <button type="button" onClick={props.openTab}>Open tab</button> : null}
          {phase === "ready_to_send" && manualPaste ? <button type="button" onClick={props.copyQuestion}>Copy question</button> : null}
          {phase === "submitted" || phase === "initial_question_supplied" ? <button type="button" onClick={props.newQuestion}>New question</button> : null}
          {unknown ? <><button type="button" onClick={props.copyQuestion}>Copy question</button><button type="button" onClick={props.dismissUnknown} disabled={!props.unknownAcknowledged}>Dismiss</button></> : null}
          <button type="button" onClick={dismiss}>{status?.cancellable ? "Don't send" : phase === "sending" ? "Stop waiting" : editable ? "Cancel" : "Close"}</button>
          {editable ? <button ref={primaryRef} type="button" className="quick-ask-primary" onClick={props.start} disabled={!canStart} aria-describedby={`${hintId} ${statusId}`}>Ask {selected?.label ?? "agent"}</button> : phase === "ready_to_send" ? <button ref={primaryRef} type="button" className="quick-ask-primary" onClick={props.send} disabled={props.busy || !props.promptConfirmationAvailable || !props.promptConfirmed || manualPaste} aria-describedby={statusId}>Send question</button> : unknown ? <button ref={primaryRef} type="button" className="quick-ask-primary" onClick={props.openTab} disabled={!status?.location}>Open tab</button> : <button type="button" className="quick-ask-primary" disabled>{phase === "initial_question_supplied" ? "Started" : phase === "submitted" ? "Sent" : phase === "sending" ? "Sending…" : "Starting…"}</button>}
        </div>
      </footer>
    </section>
  </div>;
}
