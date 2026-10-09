import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import type { NativeDeferReason, NativeRefuseReason, NativeStopReceipt, RunRetirement } from "../../src/protocol/generated/v1";
import { errorText, hasFields, nonemptyFields, type CockpitCli } from "./cliCall";
import { mainSessions } from "./identity";

export type RetirementReadiness =
  | { kind: "ready" }
  | { kind: "defer"; reason: NativeDeferReason }
  | { kind: "refuse"; reason: NativeRefuseReason; text: string };

export interface RetirementReadinessInput {
  retirement: RunRetirement;
  agentKind: string;
  nativeSession: string;
  boundSession: string;
  pid: number;
  mode: unknown;
  entries: unknown;
  idle: boolean;
  pendingMessages: boolean;
  admittedSubmission: boolean;
  asyncJobs: { running: readonly unknown[] } | null;
  mainPendingAsyncWork: boolean;
  liveSubagents: boolean;
  editorText: string;
}

// Native journal timestamps are RFC3339 strings, not message payload timestamps.
// Reject invalid dates rather than letting Date.parse normalize ambiguous input.
function journalTime(value: unknown): bigint | null {
  if (typeof value !== "string") return null;
  const parts = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,9}))?(Z|[+-]\d{2}:\d{2})$/.exec(value);
  if (!parts) return null;
  const [, year, month, day, hour, minute, second, fraction, zone] = parts;
  const y = Number(year);
  const m = Number(month);
  const days = m === 2 ? (y % 4 === 0 && (y % 100 !== 0 || y % 400 === 0) ? 29 : 28) : [31, 0, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1];
  if (m < 1 || m > 12 || Number(day) < 1 || Number(day) > days ||
      Number(hour) > 23 || Number(minute) > 59 || Number(second) > 59 ||
      (zone !== "Z" && (Number(zone.slice(1, 3)) > 23 || Number(zone.slice(4)) > 59))) return null;
  const millis = Date.parse(value);
  if (!Number.isFinite(millis)) return null;
  // Rust acceptance timestamps may have finer precision than native JS entries.
  return BigInt(Math.floor(millis / 1_000)) * 1_000_000_000n + BigInt((fraction ?? "").padEnd(9, "0"));
}

export function retirementReadiness(input: RetirementReadinessInput): RetirementReadiness {
  const refuse = (text: string): RetirementReadiness => ({ kind: "refuse", reason: "native_refused", text });
  const identity = input.retirement.identity;
  if (input.agentKind !== "main" || !identity || !input.nativeSession ||
      input.nativeSession !== input.boundSession || input.nativeSession !== identity.omp_session_id ||
      !Number.isSafeInteger(input.pid) || input.pid <= 0 || input.pid !== identity.process.pid) {
    return refuse("The exact bound native main session/process is not available.");
  }
  if (input.mode !== "tui") return refuse("Native retirement requires the supported TUI shutdown mode.");
  const acceptedAt = journalTime(input.retirement.created_at);
  if (acceptedAt === null || !Array.isArray(input.entries)) return refuse("Acceptance timestamp or full native journal is unavailable.");
  for (const entry of input.entries) {
    if (!entry || typeof entry !== "object") return refuse("Native journal contains a malformed entry.");
    const e = entry as { type?: unknown; timestamp?: unknown; message?: { role?: unknown } };
    const timestamp = journalTime(e.timestamp);
    if (typeof e.type !== "string" || timestamp === null) return refuse("Native journal contains a missing or invalid entry timestamp.");
    if (e.type !== "message") continue;
    if (!e.message || typeof e.message !== "object" || typeof e.message.role !== "string") return refuse("Native journal contains a malformed message.");
    // Includes extension-generated wakes, alternate branches and equal-time
    // ambiguity. Content/markers are never evidence of trusted provenance.
    if (e.message.role === "user" && timestamp >= acceptedAt) {
      return { kind: "refuse", reason: "user_activity", text: "Native user input arrived at or after acceptance; the worker is retained." };
    }
  }
  if ([input.idle, input.pendingMessages, input.admittedSubmission, input.mainPendingAsyncWork, input.liveSubagents].some(value => typeof value !== "boolean")) {
    return refuse("Native pending-work observations are unavailable.");
  }
  if (!input.idle || input.admittedSubmission) return { kind: "defer", reason: "busy" };
  if (input.pendingMessages) return { kind: "defer", reason: "pending_messages" };
  if (!input.asyncJobs || !Array.isArray(input.asyncJobs.running) || input.asyncJobs.running.length > 0 || input.mainPendingAsyncWork) return { kind: "defer", reason: "async_jobs" };
  if (input.liveSubagents) return { kind: "defer", reason: "live_subagents" };
  if (typeof input.editorText !== "string") return refuse("Native editor draft observation is unavailable.");
  if (input.editorText.trim()) return { kind: "defer", reason: "editor_draft" };
  return { kind: "ready" };
}

export function readRetirementReadiness(pi: ExtensionAPI, ctx: ExtensionContext, boundSession: string, retirement: RunRetirement): RetirementReadiness {
  try {
    if (ctx.mode !== "tui") return { kind: "refuse", reason: "native_refused", text: "Native retirement requires the supported TUI shutdown mode." };
    const registry = pi.pi.AgentRegistry.global();
    const main = registry.get(ctx.agent.id);
    if (ctx.agent.kind !== "main" || main?.kind !== "main" || !main.session || main.session.isDisposed ||
        main.session.sessionManager.getSessionId() !== ctx.sessionManager.getSessionId()) {
      return { kind: "refuse", reason: "native_refused", text: "The actual native main registry session cannot be verified." };
    }
    const liveSubagents = registry.list().some(ref => ref.kind !== "main" && ref.kind !== "advisor" &&
      (ref.status === "running" || ref.session?.hasPendingAsyncWork()));
    return retirementReadiness({
      retirement, agentKind: ctx.agent.kind, nativeSession: ctx.sessionManager.getSessionId(),
      boundSession, pid: process.pid, mode: ctx.mode,
      // getEntries, unlike getBranch, includes all alternate native branches.
      entries: ctx.sessionManager.getEntries(), idle: ctx.isIdle(),
      pendingMessages: ctx.hasPendingMessages() || main.session.queuedMessageCount > 0,
      admittedSubmission: main.session.hasAdmittedSubmission,
      asyncJobs: ctx.getAsyncJobSnapshot(), mainPendingAsyncWork: main.session.hasPendingAsyncWork(),
      liveSubagents, editorText: ctx.ui.getEditorText(),
    });
  } catch (error) {
    return { kind: "refuse", reason: "native_refused", text: `Native readiness evidence unavailable: ${errorText(error)}`.slice(0, 240) };
  }
}

// An isolated consumer of exact owned offers. The CLI, not the receipt, proves
// authority; the owner later proves OS process exit. No forced lifecycle APIs.
export function createRetirementHandler(deps: {
  readiness: (retirement: RunRetirement) => RetirementReadiness;
  receipt: (retirement: RunRetirement, outcome: NativeStopReceipt) => Promise<void>;
  shutdown: () => void;
  active: () => boolean;
}): RetirementHandler {
  let requested: RunRetirement | undefined;
  let latest: RunRetirement | null = null;
  let deferred: { id: string; reason: NativeDeferReason } | undefined;
  let handling = false;
  return {
    async observe(retirement: RunRetirement | null): Promise<void> {
      if (!deps.active()) return;
      latest = retirement;
      if (requested && (!retirement || retirement.retirement_id !== requested.retirement_id ||
          !["native_stop_offered", "native_stop_deferred", "native_stop_requested"].includes(retirement.state.state))) requested = undefined;
      if (handling) return;
      if (!retirement || requested) return;
      if (retirement.state.state !== "native_stop_offered" && retirement.state.state !== "native_stop_deferred") return;
      handling = true;
      try {
        const readiness = deps.readiness(retirement);
        if (readiness.kind === "defer") {
          if (deferred?.id === retirement.retirement_id && deferred.reason === readiness.reason) return;
          await deps.receipt(retirement, { outcome: "deferred", reason: readiness.reason });
          deferred = { id: retirement.retirement_id, reason: readiness.reason };
        } else if (readiness.kind === "refuse") {
          await deps.receipt(retirement, { outcome: "refused", reason: readiness.reason, text: readiness.text });
        } else {
          await deps.receipt(retirement, { outcome: "shutdown_requested" });
          // No await between the fresh native check and the public shutdown call.
          // A rejected receipt or newly admitted user work must never cause exit.
          if (!deps.active() || latest?.retirement_id !== retirement.retirement_id ||
              !["native_stop_offered", "native_stop_deferred", "native_stop_requested"].includes(latest.state.state) ||
              deps.readiness(retirement).kind !== "ready") return;
          requested = retirement;
          deps.shutdown();
        }
      } finally { handling = false; }
    },
    async agentEnd(): Promise<void> {
      if (requested) {
        if (deps.active() && deps.readiness(requested).kind === "ready") deps.shutdown();
      } else await this.observe(latest);
    },
    observationUnavailable() { requested = undefined; latest = null; },
  };
}

export interface RetirementHandler {
  observe(retirement: RunRetirement | null): Promise<void>;
  agentEnd(): Promise<void>;
  observationUnavailable(): void;
}

function isNativeProcess(value: unknown): boolean {
  return hasFields(value, ["pid", "start_ticks", "kernel_boot_id"]) &&
    typeof value.pid === "number" && Number.isInteger(value.pid) && value.pid > 0 && value.pid <= 0xffff_ffff &&
    typeof value.start_ticks === "number" && Number.isSafeInteger(value.start_ticks) && value.start_ticks >= 0 &&
    (value.kernel_boot_id === null || (typeof value.kernel_boot_id === "string" && value.kernel_boot_id.length > 0));
}

export function isRunRetirement(value: unknown): value is RunRetirement {
  if (!hasFields(value, ["retirement_id", "trigger", "result_message_id", "task_revision", "identity", "state", "created_at", "updated_at"]) ||
      !nonemptyFields(value, ["retirement_id", "result_message_id", "task_revision"]) ||
      typeof value.trigger !== "string" || !["accept", "accept_recovery", "operator_conflict_resolution"].includes(value.trigger) ||
      journalTime(value.created_at) === null || journalTime(value.updated_at) === null ||
      value.state === null || typeof value.state !== "object" || Array.isArray(value.state) ||
      !("state" in value.state) || typeof value.state.state !== "string") return false;
  const state = value.state;
  if (typeof state.state !== "string") return false;
  if (!["waiting", "native_stop_offered"].includes(state.state) && journalTime("at" in state ? state.at : undefined) === null) return false;
  switch (state.state) {
    case "waiting":
      if (!hasFields(state, ["state", "blockers"]) || !Array.isArray(state.blockers) ||
          !state.blockers.every(blocker => typeof blocker === "string" && ["open_descendant_runs", "running_subagents"].includes(blocker))) return false;
      break;
    case "native_stop_offered":
      if (!hasFields(state, ["state", "offered_at"]) || journalTime(state.offered_at) === null) return false;
      break;
    case "native_stop_deferred":
      if (!hasFields(state, ["state", "offered_at", "reason", "at"]) || journalTime(state.offered_at) === null ||
          typeof state.reason !== "string" || !["busy", "pending_messages", "async_jobs", "live_subagents", "editor_draft"].includes(state.reason)) return false;
      break;
    case "native_stop_requested":
    case "close_intent":
      if (!hasFields(state, ["state", "at"])) return false;
      break;
    case "native_stopped":
      if (!hasFields(state, ["state", "at", "evidence"]) || typeof state.evidence !== "string" ||
          !["exited_after_shutdown_request", "already_exited"].includes(state.evidence)) return false;
      break;
    case "retired":
      if (!hasFields(state, ["state", "at", "terminal"]) || typeof state.terminal !== "string" ||
          !["closed_by_cockpit", "already_absent", "absent_after_uncertain_close"].includes(state.terminal)) return false;
      break;
    case "retained":
      if (!hasFields(state, ["state", "at", "reason", "native_stopped"]) || typeof state.native_stopped !== "boolean" ||
          typeof state.reason !== "string" || !["identity_incomplete", "identity_changed", "endpoint_changed", "native_process_unverifiable",
            "process_pane_mismatch", "worker_unresponsive", "worker_busy_timeout", "user_activity", "native_refused", "shared_tab", "tab_renamed",
            "pane_moved", "foreground_process", "observation_unavailable", "herdr_refused"].includes(state.reason)) return false;
      break;
    case "unknown":
      if (!hasFields(state, ["state", "at", "phase", "detail"]) || typeof state.phase !== "string" ||
          !["native_stop", "terminal_close"].includes(state.phase) || typeof state.detail !== "string") return false;
      break;
    default: return false;
  }
  if (value.identity === null) return state.state === "retained" && "reason" in state && state.reason === "identity_incomplete";
  const identity = value.identity;
  if (!hasFields(identity, ["run_attempt", "launch_attempt", "launch_tag", "endpoint_identity", "session_id", "workspace_id",
      "tab_id", "pane_id", "terminal_id", "herdr_boot_id", "omp_session_id", "process", "shell"]) ||
      !nonemptyFields(identity, ["launch_tag", "endpoint_identity", "session_id", "workspace_id", "tab_id", "pane_id", "terminal_id", "omp_session_id"]) ||
      ![identity.run_attempt, identity.launch_attempt].every(attempt => typeof attempt === "number" && Number.isInteger(attempt) && attempt >= 0 && attempt <= 0xffff_ffff) ||
      !(identity.herdr_boot_id === null || (typeof identity.herdr_boot_id === "string" && identity.herdr_boot_id.length > 0)) ||
      !isNativeProcess(identity.process) || !hasFields(identity.shell, ["process", "executable_device", "executable_inode", "argv_digest"]) ||
      !nonemptyFields(identity.shell, ["executable_device", "executable_inode", "argv_digest"]) || !isNativeProcess(identity.shell.process)) return false;
  return true;
}

export function bindRetirementHandler(pi: ExtensionAPI, runId: string, cli: CockpitCli, ctx: ExtensionContext, signal: AbortSignal): RetirementHandler {
    const session = ctx.sessionManager.getSessionId();
    return createRetirementHandler({
      readiness: retirement => readRetirementReadiness(pi, ctx, mainSessions.get(runId) ?? "", retirement),
      active: () => !signal.aborted && ctx.sessionManager.getSessionId() === session && mainSessions.get(runId) === session,
      receipt: async (retirement, outcome) => {
        const args = ["run", "retirement-receipt", "--retirement", retirement.retirement_id];
        if (outcome.outcome === "shutdown_requested") args.push("--shutdown-requested");
        else if (outcome.outcome === "deferred") args.push("--deferred", outcome.reason);
        else args.push("--refused", outcome.text, "--refuse-reason", outcome.reason);
        await cli.call(ctx, args, signal);
      },
      shutdown: () => ctx.shutdown(),
    });
}
