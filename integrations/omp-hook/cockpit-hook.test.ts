import { afterEach, describe, expect, it, vi } from "vitest";
import { existsSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn, type ChildProcess } from "node:child_process";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import { bindHook, bootstrapFrames, capturedMessages, frames } from "./cockpit-hook";

vi.mock("node:child_process", () => ({ spawn: vi.fn() }));

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  vi.mocked(spawn).mockReset();
});
const message = (seq: number, kind = "instruction", report?: { kind: string }) => ({ message_id: `message-${seq}`, to_run_id: "run-one", seq, kind, text: `body-${seq}`, report });

type HookAPI = Parameters<typeof bindHook>[0];
type HookContext = Parameters<Parameters<HookAPI["on"]>[1]>[1];

function fixture(batches: unknown[][] = [[message(1)]], idle = true, role = "worker", boards: ({ key: string; text: string } | undefined)[] = []) {
  const root = mkdtempSync(join(tmpdir(), "cockpit-hook-test-"));
  roots.push(root);
  const credentials = join(root, "credentials");
  mkdirSync(credentials, { mode: 0o700 });
  const env: NodeJS.ProcessEnv = { COCKPIT_CLI_PATH: "/fixture/cockpit-cli", COCKPIT_JOB_CREDENTIAL_FILE: join(credentials, "job.json") };
  let sessionId = "actual-session";
  const handlers: Record<string, (event: unknown, context: HookContext) => void> = {};
  const timers = new Map<unknown, () => void>();
  const delays: number[] = [];
  const signals: AbortSignal[] = [];
  const calls: string[][] = [];
  const environments: NodeJS.ProcessEnv[] = [];
  const sends: { text: string; options: unknown }[] = [];
  const waiting = Promise.withResolvers<void>();
  const stopped = Promise.withResolvers<void>();
  const restarted = Promise.withResolvers<void>();
  let blockedWaits = 0;
  const ctx = {
    agent: { kind: "main" as "main" | "sub" }, cwd: root,
    sessionManager: { getSessionId: () => sessionId, getSessionFile: () => join(root, "not-yet-created-session.jsonl") },
    isIdle: () => idle, hasPendingMessages: () => false,
    setTimeout(callback: () => void, ms: number) { const id = {}; delays.push(ms); timers.set(id, callback); return id; },
    clearTimer(id: unknown) { timers.delete(id); },
  };
  const api = {
    on(event: string, handler: (event: unknown, context: HookContext) => void) { handlers[event] = handler; },
    sendUserMessage(text: string, options?: unknown) { sends.push({ text, options }); },
  };
  const run = vi.fn(async (args: string[], commandEnv: NodeJS.ProcessEnv, _cwd: string, signal: AbortSignal) => {
    calls.push(args);
    environments.push({ ...commandEnv });
    signals.push(signal);
    signal.addEventListener("abort", () => stopped.resolve(), { once: true });
    if (args[1] === "context") return {};
    if (args[1] === "brief") return { run_id: "run-one", role, bootstrap: "Bootstrap" };
    if (args[1] === "board") return { key: "compact-key", text: "Compacted board" };
    if (args[1] === "report") return {};
    if (batches.length) {
      const path = args[args.indexOf("--capture-file") + 1];
      const response = { run_id: "run-one", messages: batches.shift(), through_sequence: 1, board: boards.shift() };
      writeFileSync(path, JSON.stringify(response), { mode: 0o600, flag: "wx" });
      return response;
    }
    waiting.resolve();
    if (++blockedWaits === 2) restarted.resolve();
    const { promise, reject } = Promise.withResolvers<never>();
    signal.addEventListener("abort", () => reject(new Error("Aborted")), { once: true });
    return promise;
  });
  bindHook(api, run, env);
  return {
    ctx, env, calls, environments, sends, signals, credentials, api, handlers, delays,
    waiting: waiting.promise, stopped: stopped.promise, restarted: restarted.promise,
    emit(event: string) { handlers[event]?.({}, ctx); },
    changeSession(id: string) { sessionId = id; },
    flush() { const callbacks = [...timers.values()]; timers.clear(); for (const callback of callbacks) callback(); },
  };
}

describe("optional public OMP Cockpit hook", () => {
  it("trusts the CLI deliverable capture without a second classification", () => {
    const delivered = [message(1, "observation"), message(2, "report", { kind: "needs_input" })];
    expect(capturedMessages({ messages: delivered }).messages).toEqual(delivered);
  });

  it("omits unknown task context from dispatcher banners", () => {
    const notice = { ...message(1, "observation"), from: { type: "dispatcher" } };
    expect(frames([notice])[0]).toContain("[observation] from dispatcher: body-1");
    expect(frames([notice])[0]).not.toContain("unknown project");
    expect(frames([{ ...notice, context: { task_title: "Ship it", project: { label: "App" } } }])[0])
      .toContain('[observation] task "Ship it" (App) from dispatcher: body-1');
  });

  it("rejects malformed capture and report envelopes", () => {
    expect(() => capturedMessages({ messages: [{ seq: 1 }] })).toThrow();
    expect(() => capturedMessages({ messages: [{ message_id: "id", seq: 1, kind: "report", report: "bad" }] })).toThrow();
    expect(() => capturedMessages({})).toThrow();
  });

  it("rejects copied wrong-run captures and mismatched envelope recipients", () => {
    expect(capturedMessages({ run_id: "run-one", messages: [message(1)] }, "run-one").messages).toEqual([message(1)]);
    expect(() => capturedMessages({ run_id: "run-other", messages: [message(1)] }, "run-one")).toThrow();
    expect(() => capturedMessages({ run_id: "run-one", messages: [{ ...message(1), to_run_id: "run-other" }] }, "run-one")).toThrow();
    expect(() => capturedMessages({ messages: [message(1)] }, "run-one")).toThrow();
  });

  it("captures and renders board and message annotations as untrusted data", () => {
    const board = { key: "key", text: "Board rows" };
    expect(capturedMessages({ messages: [message(1)], board }).board).toEqual(board);
    expect(() => capturedMessages({ messages: [], board: { key: 1, text: "bad" } })).toThrow();
    const annotated = { ...message(1, "user_directive"), context: {
      thread_title: "Request", worker_state: "idle_no_report", last_activity_at: "yesterday", note: "no action needed",
    } };
    const rendered = frames([annotated], board).join("");
    expect(rendered).toContain('thread "Request" · worker idle_no_report · last activity yesterday · no action needed');
    expect(rendered).toContain("[user_directive]");
    expect(rendered).toContain("--- BEGIN UNTRUSTED DATA 1/1 ---\nBoard:\nBoard rows");
  });

  it("keeps the latest board once for a coalesced flush", async () => {
    const f = fixture([[message(1)], [message(2)]], true, "root", [
      { key: "old", text: "Old board" }, { key: "new", text: "New board" },
    ]);
    f.emit("session_start"); await f.waiting; f.flush();
    expect(f.sends[1].text.match(/Board:\n/g)).toHaveLength(1);
    expect(f.sends[1].text).toContain("New board");
    expect(f.sends[1].text).not.toContain("Old board");
    f.emit("session_shutdown");
  });

  it("refreshes the board after compaction only for the current main root", async () => {
    const root = fixture([], true, "root");
    root.emit("session_start"); await root.waiting;
    root.emit("session_compact"); await Promise.resolve(); await Promise.resolve();
    expect(root.calls).toContainEqual(["inbox", "board", "--json"]);
    expect(root.sends[1].text).toContain("Reconcile:");
    expect(root.sends[1].text).toContain("Board:\nCompacted board");
    root.changeSession("other");
    root.emit("session_compact");
    expect(root.calls.filter(args => args[1] === "board")).toHaveLength(1);
    root.emit("session_shutdown");
    const worker = fixture([], true, "worker");
    worker.emit("session_start"); await worker.waiting;
    worker.emit("session_compact");
    worker.ctx.agent.kind = "sub"; worker.emit("session_compact");
    expect(worker.calls.some(args => args[1] === "board")).toBe(false);
    worker.ctx.agent.kind = "main"; worker.emit("session_shutdown");
  });

  it("splits large compact digests into bounded recoverable untrusted fragments", () => {
    const messages = [{ ...message(1), text: "👩‍💻".repeat(8000) }];
    const parts = frames(messages);
    expect(parts.length).toBeGreaterThan(1);
    for (const part of parts) expect(Buffer.byteLength(part)).toBeLessThanOrEqual(16 * 1024);
    const digest = parts.map(part => part.split(/--- BEGIN UNTRUSTED DATA \d+\/\d+ ---\n/)[1].split(/\n--- END UNTRUSTED DATA/)[0]).join("");
    expect(digest).toContain(JSON.stringify(messages[0].text));
    expect(digest).toContain('"message_id":"message-1"');
  });

  it("renders compact result details without duplicating summary or null fields", () => {
    const result = { ...message(1, "report"), from: { type: "run", run_id: "worker" },
      context: { task_id: "task", task_title: "Build", project: { label: "App" }, run_label: "Worker" },
      text: "Ready for review", in_reply_to: null,
      report: { kind: "result", summary: "Ready for review", outcome: "succeeded",
        source_output: { type: "non_code", artifacts: ["artifact"] } } };
    const rendered = frames([result]).join("");
    expect(rendered).toContain('[result] task "Build" (App) from Worker: Ready for review');
    expect(rendered.match(/Ready for review/g)).toHaveLength(1);
    expect(rendered).toContain('"outcome":"succeeded"');
    expect(rendered).toContain('"artifacts":["artifact"]');
    expect(rendered).not.toContain('"in_reply_to"');
  });

  it("keeps thread correlation and only whitelisted handoff facts inside untrusted frames", () => {
    const handoff = { ...message(1), thread_id: "thread-one", handoff: {
      task_id: "task-one", task_revision: "rev-one", attempt_run_id: "attempt-one", origin: "browser",
      token: "secret-token", binding: {
        requested: { repository_id: "repo", workspace_id: "space", branch: "feature", base_ref: "main", capability: "secret-capability" },
        workspace_checkout: { path: "/private", common_dir: "/private/.git", branch: "checkout-branch", head: "commit" },
      },
    } };
    const answer = { ...message(2, "answer"), thread_id: "thread-one", in_reply_to: "question-one" };
    const report = { ...message(3, "report"), report: { kind: "needs_input", card: { title: "Decision", secret: "secret-card" } } };
    const rendered = frames([handoff, answer, report]).join("");
    expect(rendered).toContain("--- BEGIN UNTRUSTED DATA");
    expect(rendered).toContain('"thread_id":"thread-one"');
    expect(rendered).toContain('"task_revision":"rev-one"');
    expect(rendered).toContain('"attempt_run_id":"attempt-one"');
    expect(rendered).toContain('"requested":{"repository_id":"repo","workspace_id":"space","branch":"feature","base_ref":"main"}');
    expect(rendered).toContain('"workspace_checkout":{"branch":"checkout-branch"}');
    expect(rendered).toContain('"in_reply_to":"question-one"');
    expect(rendered).toContain('"card_title":"Decision"');
    expect(rendered).toContain("Answer messages are already recorded");
    expect(rendered).not.toMatch(/secret-token|secret-capability|secret-card|\/private/);
  });
  it("retains only image metadata and points to scoped local paths", () => {
    const rendered = frames([{ ...message(1), images: [{
      image_id: "sha256-id", mime_type: "image/png", width: 10, height: 20, bytes: 30,
      private_path: "/secret",
    }] }]).join("");
    expect(rendered).toContain('"images":[{"image_id":"sha256-id","mime_type":"image/png","width":10,"height":20,"bytes":30}]');
    expect(rendered).toContain("Run `cockpit-cli inbox images` for local paths.");
    expect(rendered).not.toContain("/secret");
    expect(frames([message(2)]).join("")).not.toContain("inbox images");
  });


  it("preserves oversized canonical bootstrap without truncation", () => {
    const text = `Stages: stage-one [x]\nDescription: ${"🧪".repeat(10_000)}`;
    const parts = bootstrapFrames(text);
    expect(parts.length).toBeGreaterThan(1);
    for (const part of parts) expect(Buffer.byteLength(part)).toBeLessThanOrEqual(16 * 1024);
    const rebuilt = parts.map(part => part.split(/--- BEGIN LAUNCH BRIEF FRAGMENT \d+\/\d+ ---\n/)[1].split(/\n--- END LAUNCH BRIEF FRAGMENT/)[0]).join("");
    expect(rebuilt).toBe(text);
    expect(bootstrapFrames("Small bootstrap")).toEqual(["Small bootstrap"]);
  });

  it("registers actual identity, coalesces bursts, and starts prompts only when idle", async () => {
    // The real CLI filters progress before writing the capture.
    const scoped = { ...message(2), thread_id: "thread-two", handoff: {
      task_id: "task-two", task_revision: "revision-two", binding: { requested: { branch: "feature-two" } },
    } };
    const f = fixture([[message(1)], [scoped], [scoped], []]);
    f.emit("session_start");
    await f.waiting;
    expect(f.env.COCKPIT_SESSION_ID).toBe("actual-session");
    expect(f.calls[0]).toEqual(["run", "context", "--json"]);
    expect(f.calls.some(args => args[0] === "inbox" && args[1] === "next" && args.includes("--deliverable"))).toBe(true);
    expect(f.sends).toHaveLength(1);
    expect(f.delays).toContain(1500);
    f.flush();
    expect(f.sends).toHaveLength(2);
    expect(f.sends[1].options).toEqual({ attribution: "agent" });
    expect(f.sends[1].text.match(/message-2/g)).toHaveLength(1);
    expect(f.sends[1].text).toContain('"thread_id":"thread-two"');
    expect(f.sends[1].text).toContain('"task_revision":"revision-two"');
    expect(f.sends[1].text).toContain('"branch":"feature-two"');
    expect(f.sends[1].text).not.toContain("message-3");
    const spool = readdirSync(f.credentials)[0];
    expect(readdirSync(join(f.credentials, spool))).toEqual([]);
    f.emit("session_shutdown");
    expect(f.signals.every(signal => signal.aborted)).toBe(true);
    expect(f.env.COCKPIT_SESSION_ID).toBeUndefined();
  });

  it("registers from the actual session ID before the advertised journal exists", async () => {
    const f = fixture([]);
    expect(existsSync(f.ctx.sessionManager.getSessionFile())).toBe(false);
    f.emit("session_start"); await f.waiting;
    expect(f.calls[0]).toEqual(["run", "context", "--json"]);
    expect(f.environments[0].COCKPIT_HARNESS).toBe("omp");
    expect(f.environments[0].COCKPIT_SESSION_ID).toBe("actual-session");
    expect(f.environments[0].PI_SESSION_FILE).toBeUndefined();
    expect(existsSync(f.ctx.sessionManager.getSessionFile())).toBe(false);
    f.emit("session_shutdown");
  });

  it("does not clear an independently inherited invalid session-file source", async () => {
    const f = fixture([]);
    f.env.PI_SESSION_FILE = "/independent/unreadable-session.jsonl";
    f.emit("session_start"); await f.waiting;
    expect(f.environments[0].PI_SESSION_FILE).toBe("/independent/unreadable-session.jsonl");
    f.emit("session_shutdown");
    expect(f.env.PI_SESSION_FILE).toBe("/independent/unreadable-session.jsonl");
  });

  it("uses supported steer while busy without aborting tools or submitting a draft", async () => {
    const f = fixture([[
      message(1, "instruction"), message(2, "answer"), message(3, "cancel_request"),
      { ...message(4, "report", { kind: "needs_input" }), from: { type: "run", run_id: "worker" } },
      { ...message(5, "report", { kind: "result" }), from: { type: "run", run_id: "worker" } },
      { ...message(6, "observation"), from: { type: "dispatcher" } },
    ]], false);
    f.env.COCKPIT_BOOTSTRAP_ARGV = "1";
    f.emit("session_start"); f.emit("agent_start"); await f.waiting; f.flush();
    expect(f.sends).toHaveLength(1);
    for (const kind of ["instruction", "answer", "cancel_request", "needs_input", "result", "observation"]) {
      expect(f.sends[0].text).toContain(kind);
    }
    expect(f.sends.every(send => JSON.stringify(send.options) === JSON.stringify({ attribution: "agent", deliverAs: "steer" }))).toBe(true);
    expect(f.signals.every(signal => !signal.aborted)).toBe(true);
    f.emit("session_shutdown");
  });

  it("keeps captured bodies and aborts the old incarnation before a switch", async () => {
    const f = fixture();
    f.emit("session_start"); await f.waiting;
    f.changeSession("different-session");
    f.emit("session_switch"); f.flush();
    expect(f.sends).toHaveLength(1);
    expect(f.signals.every(signal => signal.aborted)).toBe(true);
    const spool = join(f.credentials, readdirSync(f.credentials)[0]);
    const capture = join(spool, readdirSync(spool)[0]);
    expect(readFileSync(capture, "utf8")).toContain("message-1");
  });

  it("recovers never-enqueued capture only when the exact same session resumes", async () => {
    const f = fixture();
    f.emit("session_start"); await f.waiting;
    f.emit("session_switch");
    f.emit("session_start"); await f.restarted;
    expect(f.sends.filter(send => send.text.includes("message-1"))).toHaveLength(1);
    const spool = join(f.credentials, readdirSync(f.credentials)[0]);
    expect(readdirSync(spool)).toEqual([]);
    f.emit("session_shutdown");
  });

  it("retains uncertain queue outcomes and never automatically replays them", async () => {
    const f = fixture();
    f.api.sendUserMessage = text => { if (text.includes("BEGIN UNTRUSTED")) throw new Error("Unknown queue result"); };
    f.emit("session_start"); await f.waiting; f.flush();
    await f.stopped;
    const spool = join(f.credentials, readdirSync(f.credentials)[0]);
    expect(readdirSync(spool)[0]).toMatch(/\.uncertain$/);
    expect(f.signals.every(signal => signal.aborted)).toBe(true);
  });

  it("does not inject a second bootstrap when launch argv already supplied it", async () => {
    const f = fixture();
    f.env.COCKPIT_BOOTSTRAP_ARGV = "1";
    f.emit("session_start"); f.emit("agent_start"); await f.waiting;
    expect(f.sends).toEqual([]);
    f.flush();
    expect(f.sends).toHaveLength(1);
    expect(f.sends[0].text).not.toContain("Bootstrap");
    f.emit("session_shutdown");
  });

  it("queues nothing until the launch-argv bootstrap turn has started", async () => {
    const f = fixture([[message(1, "instruction")]]);
    f.env.COCKPIT_BOOTSTRAP_ARGV = "1";
    f.emit("session_start");
    for (let tick = 0; tick < 20; tick++) await Promise.resolve();
    expect(f.calls.some(args => args[0] === "inbox")).toBe(false);
    expect(f.sends).toEqual([]);
    f.emit("agent_start"); await f.waiting; f.flush();
    expect(f.sends).toHaveLength(1);
    expect(f.sends[0].text).toContain("instruction");
    f.emit("session_shutdown");
  });

  it("releases delivery after the fallback when the launch prompt never starts a turn", async () => {
    const f = fixture([[message(1, "instruction")]]);
    f.env.COCKPIT_BOOTSTRAP_ARGV = "1";
    f.emit("session_start");
    expect(f.delays).toContain(20_000);
    f.flush(); await f.waiting; f.flush();
    expect(f.sends).toHaveLength(1);
    f.emit("session_shutdown");
  });

  it("isolates owned CLI children from the harness foreground group without releasing their lifetime", () => {
    const f = fixture([]);
    const child = Object.assign(new EventEmitter(), {
      stdout: new PassThrough(), stderr: new PassThrough(), kill: vi.fn(),
    });
    // In-process stream double implements only the production command's ChildProcess surface.
    const subprocess = child as unknown as ChildProcess;
    vi.mocked(spawn).mockReturnValue(subprocess);
    bindHook(f.api, undefined, f.env);
    f.emit("session_start");
    expect(spawn).toHaveBeenCalledTimes(1);
    const options = vi.mocked(spawn).mock.calls[0][2];
    expect(options).toMatchObject({
      detached: true, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
      cwd: f.ctx.cwd, env: { COCKPIT_SESSION_ID: "actual-session" },
    });
    expect(options?.signal?.aborted).toBe(false);
    f.emit("session_shutdown");
    expect(options?.signal?.aborted).toBe(true);
    child.emit("error", new Error("Owned child aborted"));
    child.stdout.destroy();
    child.stderr.destroy();
  });

  it("ignores subagents and conflicting inherited identities", () => {
    const f = fixture();
    f.ctx.agent.kind = "sub";
    f.emit("session_start");
    expect(f.calls).toEqual([]);
    f.ctx.agent.kind = "main";
    f.env.COCKPIT_SESSION_ID = "inherited-other";
    f.emit("session_start");
    expect(f.calls).toEqual([]);
    expect(f.env.COCKPIT_SESSION_ID).toBe("inherited-other");
  });
});
