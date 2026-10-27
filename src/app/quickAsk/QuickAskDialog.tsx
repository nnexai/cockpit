import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import type { QuickAskAvailability, QuickAskHarness, QuickAskStatus } from "../../protocol/generated/v1";
import { ErrorSlot } from "../ErrorSlot";
import { UiIcon } from "../UiIcon";
import { detectPlatform } from "../input/shortcuts";
import "../projects/taskSetup.css";
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
  const deliveryId = useId();
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
      case "creating_tab": message = launchPrompt ? "Starting tab with your question. Can't be recalled." : "Creating tab… question not sent."; break;
      case "checking_environment": message = "Checking tab… question not sent."; break;
      case "starting_agent": message = launchPrompt ? `Starting ${agent?.label ?? "the agent"} with your question.` : `Starting ${agent?.label ?? "the agent"}… question not sent.`; break;
      case "waiting_for_agent": message = `${agent?.label ?? "The agent"} needs input${status.location ? ` in “${status.location.tab_label}”` : ""} before it takes the question.`; break;
      case "ready_to_send": message = manualPaste ? "Copy the question and paste it in the tab yourself." : `${agent?.label ?? "The agent"} is ready. Check its prompt, then confirm.`; break;
      case "sending": message = "Sending… can't be cancelled."; break;
      case "submitted": message = `Herdr acknowledged the send${status.location ? ` to “${status.location.tab_label}”` : ""}. Check the tab for a reply.`; break;
      case "initial_question_supplied": message = `Question supplied at launch${status.location ? ` in “${status.location.tab_label}”` : ""}. Receipt not confirmed: check the tab.`; break;
      case "cancelled": message = "Not sent."; break;
      case "failed": message = `${agent?.label ?? "The agent"} didn't get your question${status.error?.message ? `: ${status.error.message}` : "."}${status.location ? ` Tab “${status.location.tab_label}” is still open.` : ""}`; isError = true; break;
      case "outcome_unknown": message = "Couldn't confirm the send. Check the tab before asking again."; isError = true; break;
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
    <section ref={dialogRef} className="setup-dialog task-setup quick-ask" role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={keyDown}>
      <header className="task-setup-header"><h2 id={titleId}>Ask an agent</h2><button type="button" className="task-setup-close" aria-label="Close" onClick={dismiss}><UiIcon name="close" /></button></header>
      <div className="task-setup-body">
        <div className="quick-ask-row"><span id={agentId} className="quick-ask-label">Agent</span><div className="quick-ask-agents" role="radiogroup" aria-labelledby={agentId}>
          {AGENTS.map(item => {
            const found = availability ? availability.agents.find(entry => entry.harness === item.id)?.found ?? false : undefined;
            return <label key={item.id} title={item.executable} className={`quick-ask-agent${harness === item.id ? " is-selected" : ""}${found === false ? " is-unavailable" : ""}`}>
              <input type="radio" name={agentId} value={item.id} checked={harness === item.id} disabled={!editable || found !== true} aria-label={found === false ? `${item.label}, Not found on PATH` : item.label} onChange={() => props.setHarness(item.id)} />
              <span className="quick-ask-agent-name">{item.label}{found === false ? <small> · not on PATH</small> : null}</span>{harness === item.id ? <span aria-hidden="true">✓</span> : null}
            </label>;
          })}
        </div></div>
        <div className="quick-ask-row"><label className="quick-ask-label" htmlFor={questionId}>Question</label><div>
          <textarea ref={questionRef} id={questionId} tabIndex={0} rows={4} value={question} readOnly={!editable} onChange={event => props.setQuestion(event.target.value)} placeholder="Ask anything…" aria-describedby={`${hintId} ${deliveryId}`} aria-invalid={Boolean(validation)} />
          <div id={hintId} className="quick-ask-hint"><span><kbd>{detectPlatform() === "mac" ? "Cmd" : "Ctrl"}</kbd> <kbd>Enter</kbd> asks · <kbd>Enter</kbd> adds a line</span>{bytes >= 13107 ? <span className={bytes > 16384 ? "is-error" : ""}>{bytes} / 16,384 bytes</span> : null}</div>
          <p id={deliveryId} className={`quick-ask-delivery${launchPrompt ? " is-direct" : ""}`}>{launchPrompt ? "Sent at launch in the command line." : manualPaste ? "You paste it into the agent yourself." : "You confirm the agent’s prompt before it is sent."}</p>
          {phase === "submitted" || phase === "initial_question_supplied" ? <p className="quick-ask-retained">Kept here. New question clears it.</p> : null}
          {launchPrompt && editable ? <p className="quick-ask-validation">Your question will be visible in launch process arguments and layout metadata, and may persist there. Do not include secrets.</p> : null}
          {validation ? <p className="quick-ask-validation" role="alert">{validation}</p> : null}
        </div></div>
        <div className="quick-ask-row"><span className="quick-ask-label">Runs in</span><div className="quick-ask-folder">
          {editingCwd || !cwd ? <><input id={`${questionId}-cwd`} aria-label="Runs in" value={cwd} onChange={event => props.setCwd(event.target.value)} disabled={!editable} placeholder="Absolute folder path" />{!cwd ? <p className="quick-ask-hint">This Space has no checkout and the focused terminal reported no folder. Enter an absolute path.</p> : null}</> : <div className="quick-ask-path"><code title={cwd}>{cwd}</code><span title={props.cwdSource}>{props.cwdSource.startsWith("Checkout of Space") ? "Space checkout" : props.cwdSource === "Folder of the selected terminal" ? "Terminal folder" : props.cwdSource}</span>{editable ? <button type="button" className="task-setup-link" onClick={() => setEditingCwd(true)}>Change</button> : null}</div>}
        </div></div>
        <details className="quick-ask-more"><summary>More</summary><label><input type="checkbox" checked={props.switchToTab} disabled={!editable} onChange={event => props.setSwitchToTab(event.target.checked)} /> Switch to the new tab</label><p>{launchPrompt ? <>Starts <code>{selected?.executable}</code> with your question in its launch arguments. Launch acknowledgement does not confirm the agent received it.</> : manualPaste ? <>Starts <code>{selected?.executable}</code> in a new tab. Copy and paste your question manually after checking the agent's prompt.</> : <>Starts <code>{selected?.executable}</code> in a new tab. Check the agent's question input, then confirm Send question to submit it once.</>}</p></details>
        {phase === "ready_to_send" && !manualPaste ? <div className="quick-ask-confirm"><label><input type="checkbox" checked={props.promptConfirmed} disabled={props.busy || !props.promptConfirmationAvailable} onChange={event => props.setPromptConfirmed(event.target.checked)} /> I confirmed the agent's normal question prompt is visible and empty</label><p>Check the agent’s input, then confirm above to enable Send question.</p></div> : null}
      </div>
      <footer className="task-setup-footer quick-ask-footer">
        <div id={statusId} className="quick-ask-status"><ErrorSlot placement="dialog" message={message} error={isError} /></div>
        <div className="quick-ask-actions">
          {unknown ? <>
            <button type="button" onClick={props.copyQuestion}>Copy question</button>
            <button type="button" onClick={props.dismissUnknown} disabled={!props.unknownAcknowledged}>Dismiss</button>
            <button ref={primaryRef} type="button" className="setup-primary" onClick={props.openTab} disabled={!status?.location}>Open tab</button>
          </> : phase === "submitted" || phase === "initial_question_supplied" ? <>
            <button type="button" onClick={props.newQuestion}>New question</button>
            <button type="button" onClick={dismiss}>Close</button>
            {status?.location ? <button type="button" className="setup-primary" onClick={props.openTab}>Open tab</button> : null}
          </> : phase === "ready_to_send" && manualPaste ? <>
            <button type="button" onClick={props.copyQuestion}>Copy question</button>
            {status?.location ? <button type="button" onClick={props.openTab}>Open tab</button> : null}
            <button type="button" onClick={dismiss}>Close</button>
          </> : <>
            {status?.location && (phase === "waiting_for_agent" || phase === "ready_to_send" || phase === "failed") ? <button type="button" onClick={props.openTab}>Open tab</button> : null}
            <button type="button" onClick={dismiss}>{status?.cancellable ? "Don't send" : phase === "sending" || (launchPrompt && !editable) ? "Stop waiting" : editable ? "Cancel" : "Close"}</button>
            {editable ? <button ref={primaryRef} type="button" className="setup-primary" onClick={props.start} disabled={!canStart} aria-describedby={`${hintId} ${statusId}`}>Ask {selected?.label ?? "agent"}</button> : phase === "ready_to_send" ? <button ref={primaryRef} type="button" className="setup-primary" onClick={props.send} disabled={props.busy || !props.promptConfirmationAvailable || !props.promptConfirmed} aria-describedby={statusId}>Send question</button> : phase === "waiting_for_agent" ? null : <button type="button" className="setup-primary" disabled>{phase === "sending" ? "Sending…" : "Starting…"}</button>}
          </>}
        </div>
      </footer>
    </section>
  </div>;
}
