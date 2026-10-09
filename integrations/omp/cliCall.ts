import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import type { Run, TaskView } from "../../src/protocol/generated/v1";
import { callerIdentity, requireWorkerExecution } from "./identity";

export const errorText = (error: unknown): string => error instanceof Error ? error.message : String(error);
export const resultText = (value: unknown) => ({ content: [{ type: "text" as const, text: JSON.stringify(value) }], details: value });

export class CockpitCliError extends Error {
  constructor(readonly code: string, message: string) { super(message); }
}

function cliError(text: string, structured: boolean): Error {
  if (structured) {
    let value: unknown;
    try { value = JSON.parse(text); } catch { return new Error(text); }
    if (value !== null && typeof value === "object" && !Array.isArray(value)
      && Object.keys(value).length === 2 && "code" in value && "message" in value
      && typeof value.code === "string" && value.code.length > 0 && typeof value.message === "string") {
      return new CockpitCliError(value.code, text);
    }
  }
  return new Error(text);
}

export function hasFields<const K extends string>(value: unknown, fields: readonly K[]): value is { [P in K]: unknown } {
  return value !== null && typeof value === "object" && !Array.isArray(value) &&
    Object.keys(value).length === fields.length && fields.every(field => Object.hasOwn(value, field));
}
export const nonemptyFields = (value: Record<string, unknown>, fields: string[]) =>
  fields.every(field => typeof value[field] === "string" && value[field].length > 0);

export interface CliResponse {
  code: number;
  killed: boolean;
  stderr: string;
  stdout: string;
}

export function parseCliResponse<T>(response: CliResponse): T {
  if (response.code !== 0 || response.killed) {
    throw cliError(response.stderr.trim() || response.stdout.trim() || `cockpit-cli failed (${response.code})`, !response.killed && response.code !== 0);
  }
  return JSON.parse(response.stdout) as T;
}

export interface Notifier {
  notify(ctx: ExtensionContext, error: unknown): void;
  reset(): void;
}

export function createNotifier(): Notifier {
  let errorNotified = false;
  return {
    notify(ctx, error) {
      if (!errorNotified) ctx.ui.notify(`Cockpit orchestration: ${errorText(error)}`, "error");
      errorNotified = true;
    },
    reset() { errorNotified = false; },
  };
}

export interface CockpitCli {
  readonly path: string;
  call<T>(ctx: ExtensionContext, args: string[], signal?: AbortSignal, timeout?: number): Promise<T>;
  refresh(ctx: ExtensionContext, signal?: AbortSignal): Promise<Run>;
  workerAllowed(ctx: ExtensionContext, fresh: Run, signal?: AbortSignal): Promise<void>;
  workAllowed(ctx: ExtensionContext, signal?: AbortSignal): Promise<Run>;
}

export function createCli(pi: ExtensionAPI, runId: string): CockpitCli {
  const path = process.env.COCKPIT_CLI_PATH || "cockpit-cli";
  const call = async <T>(ctx: ExtensionContext, args: string[], signal?: AbortSignal, timeout = 35_000): Promise<T> => {
    const routing: string[] = [];
    if (process.env.COCKPIT_CONFIG_PATH) routing.push("--config", process.env.COCKPIT_CONFIG_PATH);
    if (process.env.COCKPIT_SESSION_ID) routing.push("--herdr-session", process.env.COCKPIT_SESSION_ID);
    const socket = process.env.COCKPIT_HERDR_SOCKET || process.env.HERDR_SOCKET_PATH;
    if (socket) routing.push("--herdr-socket", socket);
    const response = await pi.exec(path, [...args, ...routing, ...callerIdentity(pi, runId, ctx), "--json"], { signal, timeout, cwd: ctx.cwd });
    return parseCliResponse<T>(response);
  };
  const refresh = async (ctx: ExtensionContext, signal?: AbortSignal): Promise<Run> => {
    const fresh = await call<Run>(ctx, ["run", "show", "--self"], signal);
    if (fresh.run_id !== runId) throw new Error("Cockpit run binding changed; refusing tools from this OMP process.");
    return fresh;
  };
  const workerAllowed = async (ctx: ExtensionContext, fresh: Run, signal?: AbortSignal) => {
    if (!fresh.task_id) throw new Error("Worker has no canonical task.");
    const view = await call<TaskView>(ctx, ["task", "show", fresh.task_id], signal);
    requireWorkerExecution(fresh, view);
  };
  const workAllowed = async (ctx: ExtensionContext, signal?: AbortSignal): Promise<Run> => {
    const fresh = await refresh(ctx, signal);
    if (fresh.kind === "worker") await workerAllowed(ctx, fresh, signal);
    else if (fresh.stage !== "active") throw new Error("The supervisor is not active; no task mutations are authorized.");
    return fresh;
  };
  return { path, call, refresh, workerAllowed, workAllowed };
}

export async function delay(ctx: ExtensionContext, ms: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) return;
  await new Promise<void>(resolve => {
    const finish = () => { signal.removeEventListener("abort", aborted); resolve(); };
    const timer = ctx.setTimeout(finish, ms);
    const aborted = () => { ctx.clearTimer(timer); finish(); };
    signal.addEventListener("abort", aborted, { once: true });
  });
}
