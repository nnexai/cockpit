import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import type { Run } from "../../src/protocol/generated/v1";
import { callerIdentity } from "./identity";
import { deliveryStore, operationDigest, subjectFor, type OutboxEntry } from "./delivery";

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

export interface PromptBinding {
  run_id: string;
  root_id: string;
  parent_run_id: string | null;
  kind: Run["kind"];
  space_id: string | null;
  session_id?: string;
  project_space_id?: string | null;
  task_id?: string | null;
  checkout_claim_id?: string | null;
  brief: string;
  cached: boolean;
}

const PROMPT_ENTRY = "cockpit-orchestration-prompt-v2";

async function executableIdentity(path: string): Promise<string | undefined> {
  try {
    const stat = await Bun.file(path).stat();
    if (!stat.isFile() || (stat.mode & 0o111) === 0) return;
    return [stat.dev, stat.ino, stat.size, stat.mtimeMs, stat.ctimeMs, stat.mode].join(":");
  } catch { return; }
}

function executablePath(path: string, cwd: string): string {
  if (path.startsWith("/")) return Bun.fileURLToPath(Bun.pathToFileURL(path));
  const relative = path.split("/").map(encodeURIComponent).join("/");
  return Bun.fileURLToPath(new URL(relative, Bun.pathToFileURL(`${cwd}/`)));
}

export interface ControlReceipt { applied: boolean; error?: string }
export type TerminalControlReceipt = (outcome: ControlReceipt) => Promise<void>;

export interface CockpitCli {
  readonly path: string;
  call<T>(ctx: ExtensionContext, args: string[], signal?: AbortSignal, timeout?: number): Promise<T>;
  semantic<T>(ctx: ExtensionContext, args: string[], signal?: AbortSignal, payload?: { flag: string; value: unknown }, operationId?: string): Promise<T>;
  replay(ctx: ExtensionContext, signal?: AbortSignal): Promise<void>;
  childCredential(ctx: ExtensionContext): string | undefined;
  refresh(ctx: ExtensionContext, signal?: AbortSignal): Promise<Run>;
  prompt(ctx: ExtensionContext): PromptBinding | undefined;
  attach(ctx: ExtensionContext, signal: AbortSignal): Promise<unknown>;
  context<T>(ctx: ExtensionContext, args: string[], signal?: AbortSignal): Promise<T>;
  prepareTerminalReceipt(ctx: ExtensionContext, seq: number): TerminalControlReceipt;
}

function createCliSelector(pi: ExtensionAPI, launchHint: string | undefined) {
  let path = launchHint || "cockpit-cli";
  let hintIdentity: string | undefined;
  let hintRetired = false;
  let installationDirectory = launchHint?.startsWith("/") ? new URL(".", Bun.pathToFileURL(launchHint)) : undefined;
  const compatibility = new Map<string, { identity: string; error?: Error }>();
  const select = async (ctx: ExtensionContext, signal?: AbortSignal): Promise<string> => {
    const candidates: string[] = [];
    const searchPaths = (process.env.PATH ?? "").split(":").filter(Boolean);
    if (launchHint && !hintRetired) {
      let hint: string | undefined;
      if (launchHint.includes("/")) hint = executablePath(launchHint, ctx.cwd);
      else {
        for (const directory of searchPaths) {
          const candidate = executablePath(`${directory}/${launchHint}`, ctx.cwd);
          if (await executableIdentity(candidate)) { hint = candidate; break; }
        }
      }
      const identity = hint === undefined ? undefined : await executableIdentity(hint);
      if (!identity || (hintIdentity !== undefined && hintIdentity !== identity)) hintRetired = true;
      else if (hint) {
        hintIdentity = identity;
        installationDirectory ??= new URL(".", Bun.pathToFileURL(hint));
        candidates.push(hint);
      }
    }
    // The installed CLI sibling survives removal of a versioned launch hint.
    if (installationDirectory) candidates.push(Bun.fileURLToPath(new URL("cockpit-cli", installationDirectory)));
    for (const directory of searchPaths) candidates.push(executablePath(`${directory}/cockpit-cli`, ctx.cwd));
    let mismatch: Error | undefined;
    for (const candidate of new Set(candidates)) {
      if (signal?.aborted) throw new Error("Cockpit transport selection cancelled.");
      const identity = await executableIdentity(candidate);
      if (!identity) { compatibility.delete(candidate); continue; }
      const saved = compatibility.get(candidate);
      if (saved?.identity === identity) {
        if (saved.error) { mismatch ??= saved.error; continue; }
        path = candidate;
        return candidate;
      }
      try {
        const response = await pi.exec(candidate, ["orchestration", "protocol", "--json"], { signal, timeout: 5_000, cwd: ctx.cwd });
        if (response.killed) throw new Error(`Cockpit protocol probe timed out for ${candidate}.`);
        let protocol: unknown;
        try { protocol = parseCliResponse<unknown>(response); }
        catch {
          throw new CockpitCliError("transport_incompatible",
            `CLI ${candidate} cannot answer orchestration protocol --json; select a current host CLI with orchestration protocol 2 and store schema 2.`);
        }
        if (!hasFields(protocol, ["orchestration_protocol", "store_schema", "client_role"]) ||
            protocol.orchestration_protocol !== 2 || protocol.store_schema !== 2 || protocol.client_role !== "host") {
          const error = new CockpitCliError("transport_incompatible",
            `CLI ${candidate} is not a compatible Cockpit host client; install/select a host CLI with orchestration protocol 2 and store schema 2.`);
          compatibility.set(candidate, { identity, error });
          mismatch ??= error;
          continue;
        }
        if (signal?.aborted) throw new Error("Cockpit transport selection cancelled.");
        if (await executableIdentity(candidate) !== identity) continue;
        compatibility.set(candidate, { identity });
        path = candidate;
        return candidate;
      } catch (error) {
        if (signal?.aborted) throw error;
        mismatch ??= error instanceof CockpitCliError && error.code === "transport_incompatible"
          ? error : new Error(`Cannot verify Cockpit CLI ${candidate}: ${errorText(error)}`);
      }
    }
    throw mismatch ?? new Error("Cockpit host CLI is unavailable; local work can continue while the current installation reconnects.");
  };
  return {
    get path() { return path; },
    select,
    async matchesCurrentIdentity(selected: string): Promise<boolean> {
      if (await executableIdentity(selected) !== compatibility.get(selected)?.identity) {
        compatibility.delete(selected);
        return false;
      }
      return true;
    },
    async failedCall(selected: string, error: unknown): Promise<void> {
      if (!(error instanceof CockpitCliError)) compatibility.delete(selected);
      if (!await executableIdentity(selected)) hintRetired ||= selected === launchHint;
    },
  };
}

function createPromptCache(pi: ExtensionAPI, runId: string, cacheScope: string) {
  let lastPrompt: PromptBinding | undefined;
  let connected = false;
  let persistedSession = "";
  const prompt = (ctx: ExtensionContext): PromptBinding | undefined => {
    if (!lastPrompt) {
      for (const entry of ctx.sessionManager.getEntries()) {
        const e = entry as { type?: string; customType?: string; data?: { scope?: string; binding?: PromptBinding } };
        const saved = e.data?.binding;
        if (e.type === "custom" && e.customType === PROMPT_ENTRY && e.data?.scope === cacheScope &&
            saved?.run_id === runId && typeof saved.root_id === "string" &&
            (saved.parent_run_id === null || typeof saved.parent_run_id === "string") &&
            ["supervisor", "adopted", "worker"].includes(saved.kind) && typeof saved.brief === "string" &&
            (saved.space_id === null || typeof saved.space_id === "string") &&
            (saved.session_id === undefined || typeof saved.session_id === "string") &&
            (saved.project_space_id === undefined || saved.project_space_id === null || typeof saved.project_space_id === "string")) {
          lastPrompt = saved;
          persistedSession = ctx.sessionManager.getSessionId();
        }
      }
    }
    if (lastPrompt && persistedSession !== ctx.sessionManager.getSessionId()) {
      pi.appendEntry(PROMPT_ENTRY, { scope: cacheScope, binding: lastPrompt });
      persistedSession = ctx.sessionManager.getSessionId();
      connected = false;
    }
    return lastPrompt ? { ...lastPrompt, cached: !connected } : undefined;
  };
  const remember = (fresh: Run, nativeSession: string) => {
    const binding: PromptBinding = { run_id: fresh.run_id, root_id: fresh.root_id,
      parent_run_id: fresh.parent_run_id, kind: fresh.kind,
      space_id: fresh.attachment?.location?.workspace_id ?? null,
      session_id: fresh.session_id,
      project_space_id: fresh.setup?.project_workspace_id ?? fresh.attachment?.location?.workspace_id ?? null,
      brief: fresh.brief, cached: true,
      ...(fresh.task_id !== undefined ? { task_id: fresh.task_id } : {}),
      ...(fresh.checkout_claim_id !== undefined ? { checkout_claim_id: fresh.checkout_claim_id } : {}) };
    if (typeof binding.root_id === "string" && typeof binding.brief === "string" &&
        JSON.stringify(lastPrompt) !== JSON.stringify(binding)) {
      lastPrompt = binding;
      pi.appendEntry(PROMPT_ENTRY, { scope: cacheScope, binding });
      persistedSession = nativeSession;
    }
  };
  return {
    prompt,
    remember,
    setConnected(value: boolean) { connected = value; },
  };
}

export function createCli(pi: ExtensionAPI, runId: string): CockpitCli {
  const launchHint = process.env.COCKPIT_CLI_PATH;
  const config = process.env.COCKPIT_CONFIG_PATH;
  const session = process.env.COCKPIT_SESSION_ID;
  const socket = process.env.COCKPIT_HERDR_SOCKET || process.env.HERDR_SOCKET_PATH;
  const cacheScope = JSON.stringify([runId, config ?? null]);
  const selector = createCliSelector(pi, launchHint);
  const attachmentId = process.env.COCKPIT_ATTACHMENT_ID;
  const epoch = Number(process.env.COCKPIT_ATTACHMENT_EPOCH);
  const attachments = new Map<string, string>();
  const children = new Map<string, { operationId: string; file?: string; pending?: Promise<string> }>();
  const promptCache = createPromptCache(pi, runId, cacheScope);
  const prompt = promptCache.prompt;
  const routing: string[] = [];
  if (config) routing.push("--config", config);
  if (session) routing.push("--herdr-session", session);
  if (socket) routing.push("--herdr-socket", socket);
  const execute = async <T>(ctx: ExtensionContext, args: string[], signal?: AbortSignal, timeout = 35_000, provenance = true): Promise<T> => {
    const nativeSession = ctx.sessionManager.getSessionId();
    const identity = callerIdentity(pi, runId, ctx);
    let selected: string;
    try { selected = await selector.select(ctx, signal); }
    catch (error) { promptCache.setConnected(false); throw error; }
    const active = async () => {
      if (signal?.aborted || ctx.sessionManager.getSessionId() !== nativeSession) throw new Error("Cockpit call superseded.");
      callerIdentity(pi, runId, ctx);
      if (!await selector.matchesCurrentIdentity(selected)) {
        throw new Error("Cockpit CLI changed during the call; reconnect with the current compatible installation.");
      }
      if (signal?.aborted || ctx.sessionManager.getSessionId() !== nativeSession) throw new Error("Cockpit call superseded.");
      callerIdentity(pi, runId, ctx);
    };
    await active();
    let credentialFile: string | undefined;
    if (ctx.agent.kind !== "main") {
      const key = JSON.stringify([ctx.agent.id, nativeSession]);
      let child = children.get(key);
      if (!child) { child = { operationId: crypto.randomUUID() }; children.set(key, child); }
      if (!child.file) {
        if (!child.pending) {
          const registration = ["run", "child-register", "--operation-id", child.operationId,
            "--child-id", ctx.agent.id, "--child-session-id", nativeSession];
          if (ctx.agent.parentId && ctx.agent.parentId !== "Main") registration.push("--parent-child-id", ctx.agent.parentId);
          const current = child;
          current.pending = (async () => {
            // Only bootstrap uses the main job file with real native evidence.
            // Keep this private transport response out of model-visible results.
            const result = parseCliResponse<{ child_credential_file?: unknown }>(
              await pi.exec(selected, [...registration, ...routing, ...identity, "--json"], { signal, timeout, cwd: ctx.cwd }));
            await active();
            if (typeof result.child_credential_file !== "string" || !result.child_credential_file.startsWith("/")) {
              throw new Error("Cockpit did not provision a private native-child credential.");
            }
            current.file = result.child_credential_file;
            return current.file;
          })().finally(() => { current.pending = undefined; });
        }
        credentialFile = await child.pending;
      } else credentialFile = child.file;
    }
    await active();
    const command = [...args, ...(provenance ? routing : config ? ["--config", config] : []),
      ...(provenance ? identity : [])];
    if (provenance) command.push("--json");
    try {
      return parseCliResponse<T>(credentialFile
        ? await pi.exec("/usr/bin/env", [`COCKPIT_JOB_CREDENTIAL_FILE=${credentialFile}`, selected, ...command], { signal, timeout, cwd: ctx.cwd })
        : await pi.exec(selected, command, { signal, timeout, cwd: ctx.cwd }));
    } catch (error) {
      promptCache.setConnected(false);
      // Submitted mutations can have unknown delivery; never replay them here.
      await selector.failedCall(selected, error);
      throw error;
    }
  };
  const call = <T>(ctx: ExtensionContext, args: string[], signal?: AbortSignal, timeout?: number) => execute<T>(ctx, args, signal, timeout);
  const deliver = async <T>(ctx: ExtensionContext, entry: OutboxEntry, signal?: AbortSignal): Promise<T> => {
    const store = deliveryStore(runId, subjectFor(ctx));
    store.load();
    if (entry.state === "delivered") return entry.result as T;
    if (entry.state === "conflict") throw new CockpitCliError(entry.error!.code, entry.error!.message);
    const args = [...entry.args];
    if (entry.payload) args.push(entry.payload.flag, store.payload(entry.operation_id, entry.payload.value));
    try {
      const result = await call<T>(ctx, args, signal);
      entry.result = result;
      entry.state = "delivered";
      store.save();
      return result;
    } catch (error) {
      // Definitive rejection is terminal; transport failures retain this exact
      // UUID/payload for replay. Never generate a replacement or rebase scope.
      if (entry.state === "delivered") { entry.state = "pending"; delete entry.result; }
      if (error instanceof CockpitCliError && (
        error.code.endsWith("_conflict") || error.code.endsWith("_stale") || error.code.endsWith("_mismatch") ||
        error.code.startsWith("invalid_") ||
        ["actor_forbidden", "job_unauthorized", "job_credential_invalid", "job_auth_invalid", "report_requires_main"].includes(error.code))) {
        entry.state = "conflict";
        entry.error = { code: error.code, message: error.message };
        store.save();
      }
      if (entry.state === "pending") {
        throw new CockpitCliError(error instanceof CockpitCliError ? error.code : "delivery_pending",
          `${errorText(error)} Durable operation_id=${entry.operation_id} remains pending with its original payload/provenance; retry this same ID only.`);
      }
      throw error;
    }
  };
  const matchesOriginal = (entry: OutboxEntry, args: string[], payload?: { flag: string; value: unknown }) => {
    const originalArgs = [...args];
    if (entry.provenance.task_revision) originalArgs.push("--original-task-revision", entry.provenance.task_revision);
    if (entry.provenance.claim_id) originalArgs.push("--original-claim-id", entry.provenance.claim_id);
    return entry.request_digest === operationDigest(originalArgs, payload);
  };
  const semantic = async <T>(ctx: ExtensionContext, args: string[], signal?: AbortSignal, payload?: { flag: string; value: unknown }, operationId?: string): Promise<T> => {
    callerIdentity(pi, runId, ctx); // Preserve actual child narrowing before capture.
    const index = args.findIndex(arg => arg === "--operation-id" || arg === "--message-id");
    const value = payload?.value;
    const payloadId = value && typeof value === "object" && "operation_id" in value && typeof value.operation_id === "string" ? value.operation_id : undefined;
    const id = operationId ?? (index < 0 ? payloadId : args[index + 1]);
    if (!id) throw new Error("A semantic delivery requires its original operation UUID.");
    const store = deliveryStore(runId, subjectFor(ctx));
    const state = store.load();
    let entry = state.outbox.find(item => item.operation_id === id);
    if (entry && !matchesOriginal(entry, args, payload)) throw new CockpitCliError("operation_conflict", "The retained operation UUID belongs to a different original payload.");
    if (!entry) {
      let taskRevision: string | undefined;
      let claimId: string | undefined;
      const originalArgs = [...args];
      if (args[0] === "run" && (args[1] === "report" || args[1] === "message")) {
        const binding = prompt(ctx);
        if (!binding || binding.kind === "worker" || binding.task_id) {
          const fresh = await call<Run>(ctx, ["run", "show", "--self"], signal);
          if (fresh.run_id !== runId) throw new Error("Cockpit source provenance belongs to a different run.");
          claimId = fresh.checkout_claim_id ?? undefined;
          if (fresh.task_id) {
            const view = await call<{ task: { task_id: string; task_revision: string } }>(ctx, ["task", "show", fresh.task_id], signal);
            if (view?.task?.task_id !== fresh.task_id || typeof view.task.task_revision !== "string") throw new Error("Cockpit canonical task provenance is unavailable.");
            taskRevision = view.task.task_revision;
          }
        }
        if (taskRevision) originalArgs.push("--original-task-revision", taskRevision);
        if (claimId) originalArgs.push("--original-claim-id", claimId);
      }
      const concurrent = state.outbox.find(item => item.operation_id === id);
      if (concurrent) {
        if (!matchesOriginal(concurrent, args, payload)) throw new CockpitCliError("operation_conflict", "The retained operation UUID belongs to a different original payload.");
        return deliver<T>(ctx, concurrent, signal);
      }
      entry = { operation_id: id, request_digest: operationDigest(originalArgs, payload), args: originalArgs,
        ...(payload ? { payload: structuredClone(payload) } : {}),
        provenance: { run_id: runId, attachment_id: process.env.COCKPIT_ATTACHMENT_ID!,
          epoch: Number(process.env.COCKPIT_ATTACHMENT_EPOCH), subject: subjectFor(ctx),
          ...(taskRevision ? { task_revision: taskRevision } : {}), ...(claimId ? { claim_id: claimId } : {}) },
        state: "pending" };
      state.outbox.push(entry);
      store.save(); // Original operation is durable before selecting a transport.
    }
    return deliver<T>(ctx, entry, signal);
  };
  const replay = async (ctx: ExtensionContext, signal?: AbortSignal): Promise<void> => {
    const state = deliveryStore(runId, subjectFor(ctx)).load();
    for (const entry of state.outbox) {
      if (signal?.aborted) return;
      if (entry.state !== "pending") continue;
      try { await deliver(ctx, entry, signal); }
      catch (error) { if (entry.state === "pending") throw error; }
    }
  };
  const attach = (ctx: ExtensionContext, signal: AbortSignal) => {
    if (ctx.agent.kind !== "main" || !attachmentId || !Number.isSafeInteger(epoch) || epoch < 1) {
      return Promise.reject(new Error("Cockpit attachment credential environment is unavailable; local work can continue."));
    }
    const session = ctx.sessionManager.getSessionId();
    let operationId = attachments.get(session);
    if (!operationId) { operationId = crypto.randomUUID(); attachments.set(session, operationId); }
    return call(ctx, ["run", "attach", "--operation-id", operationId, "--attachment-id", attachmentId,
      "--expected-attachment-epoch", String(epoch), "--main-session-id", session], signal);
  };
  const prepareTerminalReceipt = (ctx: ExtensionContext, seq: number): TerminalControlReceipt => {
    if (ctx.agent.kind !== "sub" || !Number.isSafeInteger(seq) || seq <= 0) {
      throw new Error("A terminal control receipt requires its actual native child and exact control sequence.");
    }
    const nativeSession = ctx.sessionManager.getSessionId();
    const identity = callerIdentity(pi, runId, ctx);
    const file = children.get(JSON.stringify([ctx.agent.id, nativeSession]))?.file;
    if (!file) throw new Error("The actual child has no narrowed credential for its terminal receipt.");
    const childId = ctx.agent.id;
    return async (outcome: ControlReceipt) => {
      if (ctx.sessionManager.getSessionId() !== nativeSession || ctx.agent.id !== childId) {
        throw new Error("The captured terminal child identity changed.");
      }
      const selected = await selector.select(ctx);
      const command = ["subagent", "control-done", "--terminal-receipt", "--seq", String(seq),
        ...(outcome.applied ? ["--applied"] : ["--failed", outcome.error || "Control failed"]),
        ...routing, ...identity, "--json"];
      // This closure asserts no live registry state. Core accepts only this
      // credential's registered subject and exact pending Cancel outcome.
      parseCliResponse(await pi.exec("/usr/bin/env", [`COCKPIT_JOB_CREDENTIAL_FILE=${file}`, selected, ...command],
        { timeout: 35_000, cwd: ctx.cwd }));
    };
  };
  const refresh = async (ctx: ExtensionContext, signal?: AbortSignal): Promise<Run> => {
    const nativeSession = ctx.sessionManager.getSessionId();
    try {
      const fresh = await call<Run>(ctx, ["run", "show", "--self"], signal);
      if (fresh.run_id !== runId) throw new Error("Cockpit run binding changed; refusing tools from this OMP process.");
      if (signal?.aborted || ctx.sessionManager.getSessionId() !== nativeSession) throw new Error("Cockpit refresh superseded.");
      if (ctx.agent.kind === "main") {
        promptCache.remember(fresh, nativeSession);
      }
      promptCache.setConnected(true);
      return fresh;
    } catch (error) { promptCache.setConnected(false); throw error; }
  };
  return { get path() { return selector.path; }, call, semantic, replay, refresh, prompt, attach, prepareTerminalReceipt,
    childCredential(ctx) {
      if (ctx.agent.kind !== "sub") return undefined;
      callerIdentity(pi, runId, ctx);
      return children.get(JSON.stringify([ctx.agent.id, ctx.sessionManager.getSessionId()]))?.file;
    },
    context: <T>(ctx: ExtensionContext, args: string[], signal?: AbortSignal) => execute<T>(ctx, args, signal, 35_000, false) };
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
