import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import type { Run } from "../../src/protocol/generated/v1";

// Loaded explicitly with `omp -e`, never installed into the user's OMP config.
// The run ID is context; the CLI reads private job credentials for authority.
// Native evidence still fences actual child bootstrap and physical control.
// OMP reuses the imported module graph but rebinds factories per clone. This
// process-private evidence is learned only from the actual main session hook,
// never from inherited environment variables or an agent-supplied argument.
export const mainSessions = new Map<string, string>();

// Native Bash has no env input. tool_call replaces its schema-valid command,
// which every Bash backend executes after direnv/snapshot setup. This changes
// the shell's exported capability, never process.env or ordinary-tool admission.
export function childBashCommand(command: string, credential?: string): string {
  const prefix = "unset COCKPIT_JOB_CREDENTIAL_FILE;";
  if (!credential) return `${prefix}\n${command}`;
  const quoted = `'${credential.replaceAll("'", "'\\''")}'`;
  return `${prefix}\nexport COCKPIT_JOB_CREDENTIAL_FILE=${quoted};\n${command}`;
}


export function reportIdentity(agent: { kind: string; id: string }, kind: string): string[] {
  if (agent.kind !== "main" && (kind === "initialized" || kind === "result")) {
    throw new Error("Only the run's main OMP agent may file initialization or work-result receipts. Children may report progress/questions within their scope.");
  }
  return agent.kind === "main" ? ["--agent-kind", "main"] : ["--agent-kind", "subagent", "--subagent-id", agent.id];
}


export function requireSupervisorManagement(run: Pick<Run, "run_id" | "root_id" | "parent_run_id" | "kind"> | undefined, agent: { kind: string }): void {
  if (agent.kind !== "main" || !run || run.parent_run_id !== null || run.run_id !== run.root_id ||
      (run.kind !== "supervisor" && run.kind !== "adopted")) {
    throw new Error("Only a main supervisor job may manage workers in its own subtree. Cached role is context; core checks current role-scoped credentials and ownership.");
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
