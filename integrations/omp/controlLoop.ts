import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { delay, errorText, type CockpitCli, type Notifier } from "./cliCall";
import { mainSessions } from "./identity";
import type { InboxMessage } from "./wake";

const CONTROL_ENTRY = "cockpit-orchestration-control-v1";

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

type ControlReceipt = { applied: boolean; error?: string };

async function applyControl(pi: ExtensionAPI, runId: string, cli: CockpitCli, ctx: ExtensionContext, message: InboxMessage, signal: AbortSignal, markCancelled: () => void): Promise<{ receipt: ControlReceipt; journalClosing: boolean }> {
  let journalClosing = false;
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
  return { receipt, journalClosing };
}

interface ControlLoopDeps {
  pi: ExtensionAPI;
  runId: string;
  cli: CockpitCli;
  notifier: Notifier;
  receipts: Map<number, ControlReceipt>;
  markCancelled(): void;
  isCancelled(): boolean;
  telemetry(ctx: ExtensionContext, status: string, summary?: string): Promise<void>;
}

async function runControlLoop(deps: ControlLoopDeps, ctx: ExtensionContext, signal: AbortSignal): Promise<void> {
  const { pi, runId, cli, notifier, receipts, markCancelled, isCancelled, telemetry } = deps;
    while (!signal.aborted) {
      try {
        const response = await cli.call<{ messages: InboxMessage[] }>(ctx, ["subagent", "controls", "--id", ctx.agent.id, "--wait", "--timeout", "30"], signal);
        if (signal.aborted) return;
        for (const message of response.messages) {
          if (signal.aborted) return;
          if (!Number.isSafeInteger(message.seq) || message.seq <= 0) throw new Error("Malformed Cockpit subagent control sequence.");
          let receipt = receipts.get(message.seq);
          if (!receipt) {
            // A crash between OMP queue/abort and Cockpit receipt is uncertain;
            // persist intent and never replay an uncertain Send automatically.
            receipt = { applied: false, error: "Control application interrupted; prior delivery is unknown and will not be repeated automatically." };
            receipts.set(message.seq, receipt);
            pi.appendEntry(CONTROL_ENTRY, { seq: message.seq, ...receipt });
            const { receipt: outcome, journalClosing } = await applyControl(pi, runId, cli, ctx, message, signal, markCancelled);
            receipt = outcome;
            receipts.set(message.seq, receipt);
            // Native hard cancellation seals its session journal; do not
            // reopen it. The following core receipt is the durable outcome.
            if (!journalClosing) pi.appendEntry(CONTROL_ENTRY, { seq: message.seq, ...receipt });
          }
          const args = ["subagent", "control-done", "--seq", String(message.seq), ...(receipt.applied ? ["--applied"] : ["--failed", receipt.error || "Control failed"])];
          // Cancellation's receipt must survive agent_end aborting the loop.
          try { await cli.call(ctx, args); }
          catch (error) { notifier.notify(ctx, error); throw error; }
          if (isCancelled()) {
            try { await telemetry(ctx, "cancelled"); }
            catch (error) { notifier.notify(ctx, error); }
            return;
          }
        }
        notifier.reset();
      } catch (error) {
        if (signal.aborted) return;
        notifier.notify(ctx, error);
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
  let taskSummary: string | undefined;
  let assignmentReported = false;
  const controlReceipts = new Map<number, ControlReceipt>();
  const telemetry = async (ctx: ExtensionContext, status: string, summary?: string) => {
    const args = ["subagent", "update", "--id", ctx.agent.id, "--role", ctx.agent.name, "--label", ctx.agent.id, "--status", status];
    if (ctx.agent.parentId && ctx.agent.parentId !== "Main") args.push("--parent", ctx.agent.parentId);
    if (summary) args.push("--summary", summary.slice(0, 4_000));
    await cli.call(ctx, args);
    if (summary !== undefined) assignmentReported = true;
  };

  const loop: ControlLoopDeps = {
    pi, runId, cli, notifier, receipts: controlReceipts, telemetry,
    markCancelled() { cancelled = true; },
    isCancelled() { return cancelled; },
  };
  return {
    captureTask(prompt, ctx) {
      if (ctx.agent.kind !== "main" && taskSummary === undefined) taskSummary = prompt.slice(0, 4_000);
    },
    reportAssignment(ctx) {
      if (ctx.agent.kind !== "main" && !assignmentReported && controlLoopAbort && !controlLoopAbort.signal.aborted) {
        return telemetry(ctx, "running", taskSummary).catch(error => { notifier.notify(ctx, error); });
      }
    },
    async agentStart(ctx) {
    if (ctx.agent.kind === "main") return;
    if (controlLoopAbort && !controlLoopAbort.signal.aborted) return;
    cancelled = false;
    controlReceipts.clear();
    for (const entry of ctx.sessionManager.getEntries()) {
      if (entry.type !== "custom" || entry.customType !== CONTROL_ENTRY) continue;
      const data = entry.data as { seq?: number; applied?: boolean; error?: string };
      if (Number.isSafeInteger(data?.seq) && typeof data.applied === "boolean") controlReceipts.set(data.seq!, { applied: data.applied, error: data.error });
    }
    controlLoopAbort = new AbortController();
    try {
      await telemetry(ctx, "running", assignmentReported ? undefined : taskSummary);
      void runControlLoop(loop, ctx, controlLoopAbort.signal);
    } catch (error) { notifier.notify(ctx, error); }
    },
    async agentEnd(event, ctx) {
      if (event.willContinue || ctx.hasPendingMessages()) return;
      controlLoopAbort?.abort();
      try { await telemetry(ctx, lifecycleStatus(event.messages, cancelled)); }
      catch (error) { notifier.notify(ctx, error); }
      return;
    },
    stop() { controlLoopAbort?.abort(); },
  };
}
