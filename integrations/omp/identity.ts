import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import type { Run, TaskView } from "../../src/protocol/generated/v1";

// Loaded explicitly with `omp -e`, never installed into the user's OMP config.
// The run environment is only a binding hint. Native ctx.agent/session evidence
// is supplied on every CLI call; an inherited pane/run does not make a sub main.
const READ_ONLY_TOOLS: Record<string, true> = { read: true, find: true, grep: true, glob: true, web_search: true, web_fetch: true, search_tool: true, cockpit_context: true, cockpit_inbox: true, cockpit_report: true };
// OMP reuses the imported module graph but rebinds factories per clone. This
// process-private evidence is learned only from the actual main session hook,
// never from inherited environment variables or an agent-supplied argument.
export const mainSessions = new Map<string, string>();

export function prepareToolAllowed(toolName: string, operation?: unknown): boolean {
  // The same native tools may be surfaced through the functions namespace.
  // Unknown namespaces and wrapper/eval tools remain fail-closed.
  const name = toolName.startsWith("functions.") ? toolName.slice("functions.".length) : toolName;
  if (name === "cockpit_message") return operation === "show";
  if (name === "cockpit_task") return operation === "list" || operation === "show";
  if (READ_ONLY_TOOLS[name] !== true) return false;
  return name !== "cockpit_inbox" || operation === "list" || operation === "ack";
}

export function reportIdentity(agent: { kind: string; id: string }, kind: string): string[] {
  if (agent.kind !== "main" && (kind === "ready" || kind === "result")) {
    throw new Error("Only the bound main OMP session may file initialization or work-result receipts. Subagents may report progress/questions to ancestors.");
  }
  return agent.kind === "main" ? ["--agent-kind", "main"] : ["--agent-kind", "subagent", "--subagent-id", agent.id];
}


export function requireSupervisorManagement(run: Omit<Pick<Run, "run_id" | "root_id" | "parent_run_id" | "kind" | "stage" | "bound_omp_session">, "kind" | "stage"> & { kind: string; stage: string }, agent: { kind: string }, nativeSession: string): void {
  if (agent.kind !== "main" || run.parent_run_id !== null || run.run_id !== run.root_id ||
      (run.kind !== "supervisor" && run.kind !== "adopted") || run.stage !== "active" ||
      !nativeSession || run.bound_omp_session !== nativeSession) {
    throw new Error("Only the active bound native main supervisor may manage workers in its own subtree. Worker and internal subagent sessions cannot grant themselves authority.");
  }
}

export function requireWorkerExecution(run: Run, view: TaskView): void {
  if (run.kind !== "worker" || run.stage !== "working" || run.close_reason !== null ||
      !run.task_id || !run.init_receipt || run.init_receipt.kind !== "ready" ||
      !run.work_plan?.plan_revision || !Array.isArray(run.grants) ||
      !run.grants.some(grant => grant.scope === "execute" && grant.plan_revision === run.work_plan!.plan_revision) ||
      view.current_run_id !== run.run_id || view.task.task_id !== run.task_id ||
      view.task.checked !== false || view.task.diagnostic !== null ||
      !view.dependencies || !["none", "satisfied"].includes(view.dependencies.state) ||
      !Array.isArray(view.dependencies.unmet) || view.dependencies.unmet.length !== 0 ||
      !Array.isArray(view.dependencies.problems)) {
    throw new Error("Worker execution is not authorized by the current canonical attempt, exact-plan Execute grant and satisfied prerequisites. Reads, questions and reports remain available.");
  }
}

export function requireNativeChild(pi: ExtensionAPI, ctx: ExtensionContext, boundMain: string): void {
  if (ctx.agent.kind === "main") return;
  if (ctx.agent.kind !== "sub" || !boundMain || typeof pi.pi.AgentRegistry?.global !== "function") {
    throw new Error("Native child registry evidence is unavailable.");
  }
  const registry = pi.pi.AgentRegistry.global();
  const child = registry.get(ctx.agent.id);
  const main = registry.get(pi.pi.MAIN_AGENT_ID);
  if (!child || child.id !== ctx.agent.id || child.kind !== "sub" || !child.session ||
      child.session.isDisposed || child.status === "aborted" ||
      child.session.sessionManager.getSessionId() !== ctx.sessionManager.getSessionId() ||
      !main || main.kind !== "main" || !main.session || main.session.isDisposed ||
      main.session.sessionManager.getSessionId() !== boundMain) {
    throw new Error("Native child/main session ownership changed; refusing inherited identity.");
  }
}

export function callerIdentity(pi: ExtensionAPI, runId: string, ctx: ExtensionContext): string[] {
  const nativeSession = ctx.sessionManager.getSessionId();
    const mainSession = ctx.agent.kind === "main" ? nativeSession : mainSessions.get(runId);
    if (!mainSession) throw new Error("The native main OMP session is not bound; refusing inherited subagent identity.");
    requireNativeChild(pi, ctx, mainSession);
    return ["--omp-session", nativeSession, "--omp-main-session", mainSession,
      "--omp-pid", String(process.pid), ...reportIdentity(ctx.agent, "progress")];
}
