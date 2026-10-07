import type { NativeDeferReason, RetainReason, RetirementBlocker, RetirementPhase, Run, TerminalOutcome } from "../../protocol/generated/v1";

export type RetirementView = { label: string; detail: string; tier: "notice" | "recover" | null; nativeStopped: boolean };

const BLOCKERS: Record<RetirementBlocker, string> = {
  open_descendant_runs: "open descendant runs",
  running_subagents: "running subagents",
};
const DEFER_DETAILS: Record<NativeDeferReason, string> = {
  busy: "The worker is completing its current turn.",
  pending_messages: "The worker has queued input.",
  async_jobs: "The worker has active or unverifiable asynchronous work.",
  live_subagents: "The worker has active subagents.",
  editor_draft: "The worker terminal has unsent text; it will not exit until the draft is cleared.",
};
const RETAIN_DETAILS: Record<RetainReason, string> = {
  identity_incomplete: "The launch identity is incomplete.",
  identity_changed: "The recorded worker identity changed.",
  endpoint_changed: "The Herdr endpoint incarnation changed.",
  native_process_unverifiable: "The worker process incarnation could not be verified.",
  process_pane_mismatch: "The worker process no longer matches its recorded terminal.",
  worker_unresponsive: "The worker did not respond before the retirement deadline.",
  worker_busy_timeout: "The worker stayed busy past the retirement deadline.",
  user_activity: "Later user content was received; Cockpit conservatively retains the worker, including after an inbox wake.",
  native_refused: "The worker refused self-retirement because local readiness or identity could not be verified.",
  shared_tab: "Its tab contains other terminals.",
  tab_renamed: "Its tab was renamed.",
  pane_moved: "Its terminal was moved.",
  foreground_process: "Something other than the idle shell is running in its terminal.",
  observation_unavailable: "Fresh terminal observation was unavailable before the deadline.",
  herdr_refused: "Herdr refused terminal closure.",
};

const TERMINAL_DETAILS: Record<TerminalOutcome, string> = {
  closed_by_cockpit: "The worker exited; its terminal was closed. Its now-empty Space may also have closed. Task, result, history, worktree and files are kept.",
  already_absent: "The worker exited; its terminal is no longer present.",
  absent_after_uncertain_close: "The worker exited; its terminal is no longer present.",
};
const UNKNOWN_VIEWS: Record<RetirementPhase, RetirementView> = {
  native_stop: { label: "Accepted · stop not confirmed", detail: "Cockpit asked the worker to exit but could not confirm it stopped. Check its terminal.", tier: "recover", nativeStopped: false },
  terminal_close: { label: "Accepted · terminal close not confirmed", detail: "Check the worker terminal; Cockpit will not retry.", tier: "recover", nativeStopped: true },
};
/** Retirement records describe saved acceptance effects, never current terminal authority. */
export function retirementView(run: Run): RetirementView | null {
  if (run.stage !== "closed" || run.close_reason !== "accepted" || !run.retirement) return null;
  const state = run.retirement.state;
  switch (state.state) {
    case "waiting":
      return state.blockers.length ? {
        label: "Accepted · retiring after sub-runs close",
        detail: `The worker keeps running until its open runs/subagents finish. Waiting for ${state.blockers.map(blocker => BLOCKERS[blocker]).join(" and ")}.`,
        tier: "notice", nativeStopped: false,
      } : {
        label: "Accepted · stopping worker", detail: "Cockpit asked the worker to exit once idle.", tier: null, nativeStopped: false,
      };
    case "native_stop_offered":
      return { label: "Accepted · stopping worker", detail: "Cockpit asked the worker to exit once idle.", tier: null, nativeStopped: false };
    case "native_stop_deferred":
      return { label: state.reason === "editor_draft" ? "Accepted · waiting for your draft" : "Accepted · waiting for worker to be idle", detail: DEFER_DETAILS[state.reason], tier: state.reason === "editor_draft" ? "notice" : null, nativeStopped: false };
    case "native_stop_requested":
      return { label: "Accepted · stopping worker…", detail: "Waiting for the OMP process to exit.", tier: null, nativeStopped: false };
    case "native_stopped":
    case "close_intent":
      return { label: "Accepted · worker stopped", detail: "Closing only its owned terminal; an empty Space may close naturally.", tier: null, nativeStopped: true };
    case "retired":
      return { label: "Accepted · worker retired", detail: TERMINAL_DETAILS[state.terminal], tier: null, nativeStopped: true };
    case "retained":
      return { label: state.native_stopped ? "Accepted · worker stopped · terminal kept" : "Accepted · worker retained", detail: `${state.native_stopped ? "" : "Native stop is not confirmed; Cockpit will not close this terminal. "}${RETAIN_DETAILS[state.reason]}`, tier: "notice", nativeStopped: state.native_stopped };
    case "unknown":
      return { ...UNKNOWN_VIEWS[state.phase] };
    default: {
      const exhaustive: never = state;
      throw new Error(`Unsupported retirement state: ${JSON.stringify(exhaustive)}`);
    }
  }
}
