import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { CockpitCliError, delay, errorText, type CockpitCli, type ControlReceipt, type Notifier, type TerminalControlReceipt } from "./cliCall";
import { mainSessions } from "./identity";
import type { InboxMessage } from "./wake";
import { deliveryStore, subjectFor } from "./delivery";


export function decodeControl(text: string, agentId: string): { op: "cancel" } | { op: "send"; text: string } {
  const value = JSON.parse(text) as { subagent_id?: unknown; op?: { op?: unknown; text?: unknown } };
  if (value.subagent_id !== agentId) throw new Error("Control targets another subagent; refusing.");
  if (value.op?.op === "cancel") return { op: "cancel" };
  if (value.op?.op === "send" && typeof value.op.text === "string" && value.op.text.trim()) return { op: "send", text: value.op.text };
  throw new Error("Invalid subagent control operation.");
}

export function lifecycleStatus(messages: readonly unknown[], cancelled: boolean): "done" | "failed" | "cancelled" {
  if (cancelled) return "cancelled";
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index] as { role?: string; stopReason?: string };
    if (message.role === "assistant") return message.stopReason === "error" ? "failed" : message.stopReason === "aborted" ? "cancelled" : "done";
  }
  return "failed";
}


async function applyControl(pi: ExtensionAPI, runId: string, cli: CockpitCli, ctx: ExtensionContext, message: InboxMessage, signal: AbortSignal, markCancelled: () => void, markStopping: (stopping: boolean) => void): Promise<{ receipt: ControlReceipt; journalClosing: boolean; terminalReceipt?: TerminalControlReceipt }> {
  let journalClosing = false;
  let terminalReceipt: TerminalControlReceipt | undefined;
  let receipt: ControlReceipt;
  try {
              const control = decodeControl(message.text, ctx.agent.id);
              await cli.refresh(ctx, signal); // includes bound main + actual child native evidence
              if (typeof pi.pi.AgentRegistry?.global !== "function" || typeof pi.pi.finalizeSubagentLifecycle !== "function") {
                throw new Error("OMP18.6.1 native subagent lifecycle controller is unavailable.");
              }
              const registry = pi.pi.AgentRegistry.global();
              const ref = registry.get(ctx.agent.id);
              const target = ref?.session;
              const main = registry.get(pi.pi.MAIN_AGENT_ID)?.session;
              if (!ref || ref.kind !== "sub" || !target || target.isDisposed || target.sessionManager.getSessionId() !== ctx.sessionManager.getSessionId() ||
                  !main || main.sessionManager.getSessionId() !== mainSessions.get(runId)) {
                throw new Error("Native subagent/main session ownership changed; refusing control.");
              }
              if (control.op === "cancel") {
                terminalReceipt = cli.prepareTerminalReceipt(ctx, message.seq);
                markStopping(true);
                // ctx.abort() only stops a turn: owned async jobs can wake it
                // again. The native lifecycle owner tombstones this exact ref,
                // aborts the task executor and disposes its owned background work.
                journalClosing = true;
                await pi.pi.finalizeSubagentLifecycle({
                  id: ref.id, session: target, aborted: true, abortKind: "signal",
                  keepAlive: false, isolated: false, agentIdleTtlMs: 0, reviveSession: null,
                });
                await target.dispose();
                const terminal = registry.get(ref.id);
                if (terminal !== ref || terminal.status !== "aborted" || terminal.session !== null || !target.isDisposed || target.hasPendingAsyncWork()) {
                  throw new Error("Native subagent cancellation did not reach terminal disposal.");
                }
                markCancelled();
              } else {
                // This is an external Cockpit controller, not an impersonated
                // OMP parent. Native IRC interrupts waits at a safe boundary;
                // the parent-IRC special-case would steer and is never used.
                const delivery = await target.deliverIrcMessage({
                  id: message.message_id, from: "Cockpit", to: ref.id,
                  body: control.text, ts: Date.now(),
                });
                if (delivery !== "injected" && delivery !== "woken") throw new Error("Native subagent rejected control delivery.");
              }
              receipt = { applied: true };
            } catch (error) { receipt = { applied: false, error: errorText(error) }; }
  markStopping(false);
  return { receipt, journalClosing, terminalReceipt };
}

interface ControlLoopDeps {
  pi: ExtensionAPI;
  runId: string;
  cli: CockpitCli;
  notifier: Notifier;
  receipts: Map<number, ControlReceipt>;
  markCancelled(): void;
  markStopping(stopping: boolean): void;
  isCancelled(): boolean;
}

async function runControlLoop(deps: ControlLoopDeps, ctx: ExtensionContext, signal: AbortSignal): Promise<void> {
  const { pi, runId, cli, notifier, receipts, markCancelled, isCancelled } = deps;
  const store = deliveryStore(runId, subjectFor(ctx));
    const session = ctx.sessionManager.getSessionId();
    while (!signal.aborted && ctx.sessionManager.getSessionId() === session) {
      try {
        const durable = store.load();
        const response = await cli.call<{ messages: InboxMessage[] }>(ctx, ["subagent", "controls", "--id", ctx.agent.id, "--wait", "--timeout", "30"], signal);
        if (signal.aborted || ctx.sessionManager.getSessionId() !== session) return;
        for (const message of response.messages) {
          if (signal.aborted) return;
          if (!Number.isSafeInteger(message.seq) || message.seq <= 0) throw new Error("Malformed Cockpit subagent control sequence.");
          let receipt = receipts.get(message.seq);
          let terminalReceipt: TerminalControlReceipt | undefined;
          if (!receipt) {
            // A crash between OMP queue/abort and Cockpit receipt is uncertain;
            // persist intent and never replay an uncertain Send automatically.
            receipt = { applied: false, error: "Control application interrupted; prior delivery is unknown and will not be repeated automatically." };
            receipts.set(message.seq, receipt);
            durable.controls.push({ seq: message.seq, body: structuredClone(message), receipt });
            store.save(); // Capture body and uncertain intent before native effects.
            const outcome = await applyControl(pi, runId, cli, ctx, message, signal, markCancelled, deps.markStopping);
            const { receipt: nextReceipt } = outcome;
            terminalReceipt = outcome.terminalReceipt;
            receipt = nextReceipt;
            receipts.set(message.seq, receipt);
            const retained = durable.controls.find(control => control.seq === message.seq)!;
            retained.receipt = receipt;
            store.save(); // Private job journal survives native child disposal.
          }
          const args = ["subagent", "control-done", "--seq", String(message.seq), ...(receipt.applied ? ["--applied"] : ["--failed", receipt.error || "Control failed"])];
          // Cancellation's receipt must survive agent_end aborting the loop.
          try {
            if (terminalReceipt) await terminalReceipt(receipt);
            else await cli.call(ctx, args);
          }
          catch (error) { notifier.notify(ctx, error); throw error; }
          if (isCancelled()) return; // The pinned terminal adapter recorded status.
        }
        notifier.reset();
      } catch (error) {
        if (signal.aborted || ctx.sessionManager.getSessionId() !== session) return;
        if (error instanceof CockpitCliError && ["job_unauthorized", "job_credential_invalid", "caller_mismatch", "session_mismatch"].includes(error.code)) return;
        if (error instanceof CockpitCliError && error.code === "transport_incompatible") notifier.notify(ctx, error);
        await delay(ctx, 2_000, signal);
      }
    }
}

export interface SubagentLifecycle {
  captureTask(prompt: string, ctx: ExtensionContext): void;
  reportAssignment(ctx: ExtensionContext): Promise<void> | undefined;
  agentStart(ctx: ExtensionContext): Promise<void>;
  agentEnd(event: { willContinue: boolean; messages: readonly unknown[] }, ctx: ExtensionContext): Promise<void>;
  stop(): void;
}

export function createSubagentLifecycle(pi: ExtensionAPI, runId: string, cli: CockpitCli, notifier: Notifier): SubagentLifecycle {
  let controlLoopAbort: AbortController | undefined;
  let cancelled = false;
  let stopping = false;
  let taskSummary: string | undefined;
  let assignmentReported = false;
  const controlReceipts = new Map<number, ControlReceipt>();
  const telemetry = async (ctx: ExtensionContext, status: string, summary?: string, signal?: AbortSignal) => {
    const args = ["subagent", "update", "--id", ctx.agent.id, "--role", ctx.agent.name, "--label", ctx.agent.id, "--status", status];
    if (ctx.agent.parentId && ctx.agent.parentId !== "Main") args.push("--parent", ctx.agent.parentId);
    if (summary) args.push("--summary", summary.slice(0, 4_000));
    await cli.call(ctx, args, signal);
    if (summary !== undefined) assignmentReported = true;
  };

  const loop: ControlLoopDeps = {
    pi, runId, cli, notifier, receipts: controlReceipts,
    markCancelled() { cancelled = true; },
    markStopping(value) { stopping = value; },
    isCancelled() { return cancelled; },
  };
  return {
    captureTask(prompt, ctx) {
      if (ctx.agent.kind !== "main" && taskSummary === undefined) taskSummary = prompt.slice(0, 4_000);
    },
    reportAssignment(ctx) {
      if (ctx.agent.kind !== "main" && !assignmentReported && controlLoopAbort && !controlLoopAbort.signal.aborted) {
        return telemetry(ctx, "running", taskSummary, controlLoopAbort.signal).catch(error => {
          if (error instanceof CockpitCliError && error.code === "transport_incompatible") notifier.notify(ctx, error);
        });
      }
    },
    async agentStart(ctx) {
    if (ctx.agent.kind === "main") return;
    if (controlLoopAbort && !controlLoopAbort.signal.aborted) return;
    cancelled = false;
    stopping = false;
    controlReceipts.clear();
    controlLoopAbort = new AbortController();
    const lifecycle = controlLoopAbort;
    const session = ctx.sessionManager.getSessionId();
    while (!lifecycle.signal.aborted && ctx.sessionManager.getSessionId() === session) {
      try {
        const retained = deliveryStore(runId, subjectFor(ctx)).load();
        for (const control of retained.controls) controlReceipts.set(control.seq, control.receipt);
        await telemetry(ctx, "running", assignmentReported ? undefined : taskSummary, lifecycle.signal);
        await cli.replay(ctx, lifecycle.signal);
        if (lifecycle.signal.aborted || ctx.sessionManager.getSessionId() !== session) return;
        void runControlLoop(loop, ctx, lifecycle.signal);
        return;
      } catch (error) {
        if (lifecycle.signal.aborted || ctx.sessionManager.getSessionId() !== session) return;
        if (error instanceof CockpitCliError && ["job_unauthorized", "job_credential_invalid", "caller_mismatch", "session_mismatch"].includes(error.code)) return;
        if (error instanceof CockpitCliError && error.code === "transport_incompatible") notifier.notify(ctx, error);
        await delay(ctx, 2_000, lifecycle.signal);
      }
    }
    },
    async agentEnd(event, ctx) {
      if (event.willContinue || ctx.hasPendingMessages()) return;
      controlLoopAbort?.abort();
      if (stopping || cancelled) return; // Exact ControlDone owns Cancel status.
      try { await telemetry(ctx, lifecycleStatus(event.messages, cancelled)); }
      catch (error) { notifier.notify(ctx, error); }
      return;
    },
    stop() { controlLoopAbort?.abort(); },
  };
}
