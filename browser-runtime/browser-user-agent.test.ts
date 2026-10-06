import { EventEmitter } from "node:events";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";

type Command = { id: number; method: string; params: Record<string, unknown>; sessionId?: string };
type TargetInfo = { targetId: string; type: string };
type Page = { context: () => Context; isClosed: () => boolean; targetId: string };
type Initialize = (args: { page: Page }) => Promise<void>;
const nodeRequire = createRequire(import.meta.url);
const socketKey = Symbol.for("cockpit.user-agent.test.socket");
const socketGlobals = globalThis as typeof globalThis & { [socketKey]?: typeof FakeSocket };
const originalArgv = process.argv[1];
const fixtures: Fixture[] = [];
let active: Fixture;

class FakeSocket extends EventEmitter {
  commands: Command[] = [];
  terminated = false;
  constructor(readonly endpoint: string) {
    super();
    active.socket = this;
    active.socketCount++;
    queueMicrotask(() => this.emit("open"));
  }
  send(payload: string) {
    const command = JSON.parse(payload) as Command;
    this.commands.push(command);
    this.emit("command", command);
    if (active.intercept?.(command, this)) return;
    if (command.method === "Browser.getVersion") this.reply(command, { userAgent: active.nativeUa });
    else if (command.method === "Target.getTargets") this.reply(command, { targetInfos: [...active.existing] });
    else if (command.method === "Target.getTargetInfo") {
      this.reply(command, { targetInfo: { targetId: command.sessionId?.slice("session-".length) } });
    } else if (command.method === "Target.setAutoAttach") {
      this.reply(command);
      if (!command.sessionId && active.attachExisting) for (const target of active.existing) this.attach(target.targetId, false);
    } else this.reply(command);
  }
  reply(command: Command, result: unknown = {}) {
    this.emit("message", Buffer.from(JSON.stringify({ id: command.id, result })));
  }
  reject(command: Command, message: string) {
    this.emit("message", Buffer.from(JSON.stringify({ id: command.id, error: { message } })));
  }
  event(method: string, params: unknown, sessionId?: string) {
    this.emit("message", Buffer.from(JSON.stringify({ method, params, ...(sessionId ? { sessionId } : {}) })));
  }
  attach(targetId: string, waitingForDebugger = true, parentTargetId?: string, type = "page") {
    this.event("Target.attachedToTarget", {
      sessionId: `session-${targetId}`, targetInfo: { targetId, type }, waitingForDebugger,
    }, parentTargetId ? `session-${parentTargetId}` : undefined);
  }
  terminate() {
    this.terminated = true;
    queueMicrotask(() => this.emit("close"));
  }
}

class Context extends EventEmitter {
  closed = false;
  readonly publicBrowser = new EventEmitter() as EventEmitter & { close: () => Promise<void> };
  constructor() {
    super();
    this.publicBrowser.close = vi.fn(async () => {
      this.closed = true;
      this.publicBrowser.emit("disconnected");
    });
  }
  browser() { return this.publicBrowser; }
  close = vi.fn(async () => {
    this.closed = true;
    this.emit("close");
  });
  newCDPSession = vi.fn(async (page: Page) => ({
    send: vi.fn(async () => ({ targetInfo: { targetId: page.targetId } })),
    detach: vi.fn(async () => {}),
  }));
  page(targetId = "initial"): Page {
    return { context: () => this, isClosed: () => this.closed, targetId };
  }
}

class Fixture {
  root = mkdtempSync(join(tmpdir(), "cockpit-ua-unit-"));
  context = new Context();
  nativeUa: unknown = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 HeadlessChrome/154.0.8040.1 Safari/537.36";
  existing: TargetInfo[] = [{ targetId: "initial", type: "page" }];
  attachExisting = true;
  socket!: FakeSocket;
  socketCount = 0;
  intercept?: (command: Command, socket: FakeSocket) => boolean;
  hookPath = join(this.root, "browser-user-agent.cjs");
  profile = join(this.root, "profile");
  initialize: Initialize;
  constructor() {
    active = this;
    fixtures.push(this);
    const packageRoot = join(this.root, "node_modules", "playwright-core");
    mkdirSync(join(packageRoot, "lib"), { recursive: true });
    mkdirSync(this.profile);
    writeFileSync(join(packageRoot, "package.json"), JSON.stringify({ name: "playwright-core" }));
    writeFileSync(join(packageRoot, "lib", "utilsBundle.js"),
      'module.exports = {ws: globalThis[Symbol.for("cockpit.user-agent.test.socket")]};');
    copyFileSync(join(dirname(fileURLToPath(import.meta.url)), "browser-user-agent.cjs"), this.hookPath);
    writeFileSync(join(this.root, "browser-user-agent.json"), JSON.stringify({ profile_path: this.profile }));
    writeFileSync(join(this.profile, "DevToolsActivePort"), "9222\n/devtools/browser/owned-test\n");
    process.argv[1] = join(this.root, "daemon.cjs");
    socketGlobals[socketKey] = FakeSocket;
    this.initialize = nodeRequire(this.hookPath) as Initialize;
  }
  async command(method: string, count = 1): Promise<Command> {
    // Startup is deliberately deferred by the product hook, not by a timer in this fixture.
    await Promise.resolve();
    await Promise.resolve();
    const commands = () => this.socket?.commands.filter(command => command.method === method) || [];
    if (commands().length >= count) return commands()[count - 1];
    const { promise, resolve } = Promise.withResolvers<Command>();
    const listener = () => {
      if (commands().length >= count) {
        this.socket.removeListener("command", listener);
        resolve(commands()[count - 1]);
      }
    };
    this.socket.on("command", listener);
    return promise;
  }
}

async function flush() {
  for (let index = 0; index < 12; index++) await Promise.resolve();
}

afterEach(async () => {
  for (const fixture of fixtures.splice(0)) {
    fixture.context.emit("close");
    await flush();
    for (const filename of Object.keys(nodeRequire.cache)) {
      if (filename.startsWith(fixture.root + "/")) delete nodeRequire.cache[filename];
    }
    rmSync(fixture.root, { recursive: true, force: true });
  }
  process.argv[1] = originalArgv;
  delete socketGlobals[socketKey];
  vi.useRealTimers();
});

describe("CLI-owned native browser user-agent policy", () => {
  it.each([
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 HeadlessChrome/154.0.8040.1 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) HeadlessChrome/121.2.3.4 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) Chrome/154.0.8040.1 Safari/537.36",
  ])("preserves the native platform and version in %s", async nativeUa => {
    const fixture = new Fixture();
    fixture.nativeUa = nativeUa;
    await fixture.initialize({ page: fixture.context.page() });
    const override = await fixture.command("Emulation.setUserAgentOverride");
    expect(override.params).toEqual({ userAgent: nativeUa.replaceAll("HeadlessChrome/", "Chrome/") });
    expect(override.sessionId).toBe("session-initial");
    expect(fixture.socket.commands.find(command => command.method === "Browser.getVersion")?.sessionId).toBeUndefined();
  });

  it("waits for late attached events and every existing target's override acknowledgement", async () => {
    const fixture = new Fixture();
    fixture.attachExisting = false;
    fixture.existing.push({ targetId: "second", type: "page" });
    fixture.intercept = command => command.method === "Emulation.setUserAgentOverride";
    let settled = false;
    const initialized = fixture.initialize({ page: fixture.context.page() }).then(() => { settled = true; });
    await fixture.command("Target.getTargets");
    await flush();
    expect(settled).toBe(false);
    fixture.socket.attach("initial", false);
    const initial = await fixture.command("Emulation.setUserAgentOverride");
    fixture.socket.reply(initial);
    await flush();
    expect(settled).toBe(false);
    fixture.socket.attach("second", false);
    const second = await fixture.command("Emulation.setUserAgentOverride", 2);
    fixture.socket.reply(second);
    await initialized;
    expect(settled).toBe(true);
    expect(fixture.context.closed).toBe(false);
  });

  it("gates concurrent noopener startup on the browser ACK and waits for both renderer ACKs", async () => {
    const fixture = new Fixture();
    await fixture.initialize({ page: fixture.context.page() });
    fixture.intercept = command => ["Emulation.setUserAgentOverride", "Target.getTargetInfo", "Runtime.runIfWaitingForDebugger"].includes(command.method);
    fixture.socket.attach("popup-a");
    fixture.socket.attach("popup-b");
    const first = await fixture.command("Emulation.setUserAgentOverride", 2);
    const second = await fixture.command("Emulation.setUserAgentOverride", 3);
    const firstBarrier = await fixture.command("Target.getTargetInfo");
    const secondBarrier = await fixture.command("Target.getTargetInfo", 2);
    let secondReady = false;
    const ready = fixture.initialize({ page: fixture.context.page("popup-b") }).then(() => { secondReady = true; });
    await flush();
    expect(fixture.socket.commands.filter(command => command.method === "Runtime.runIfWaitingForDebugger")).toHaveLength(0);
    // A new isolated renderer withholds Emulation's final ACK until startup resumes.
    fixture.socket.reply(secondBarrier, { targetInfo: { targetId: "popup-b" } });
    const resumeSecond = await fixture.command("Runtime.runIfWaitingForDebugger");
    expect(resumeSecond.sessionId).toBe(second.sessionId);
    expect(secondBarrier.sessionId).toBe(second.sessionId);
    fixture.socket.reply(resumeSecond);
    await flush();
    expect(secondReady).toBe(false);
    fixture.socket.reply(second);
    await ready;
    expect(fixture.socket.commands.filter(command => command.method === "Runtime.runIfWaitingForDebugger")).toHaveLength(1);
    // The other target's early override ACK cannot bypass its independent browser barrier.
    fixture.socket.reply(first);
    await flush();
    expect(fixture.socket.commands.filter(command => command.method === "Runtime.runIfWaitingForDebugger")).toHaveLength(1);
    fixture.socket.reply(firstBarrier, { targetInfo: { targetId: "popup-a" } });
    const resumeFirst = await fixture.command("Runtime.runIfWaitingForDebugger", 2);
    expect(resumeFirst.sessionId).toBe(first.sessionId);
    fixture.socket.reply(resumeFirst);
  });

  it("does not reinstall when hooks repeat or a capture-only consumer detaches", async () => {
    const fixture = new Fixture();
    await Promise.all([
      fixture.initialize({ page: fixture.context.page() }),
      fixture.initialize({ page: fixture.context.page() }),
    ]);
    const captureSession = await fixture.context.newCDPSession(fixture.context.page());
    await captureSession.detach();
    fixture.socket.attach("new-tab");
    await fixture.initialize({ page: fixture.context.page("new-tab") });
    expect(fixture.socketCount).toBe(1);
    expect(fixture.socket.commands.filter(command => command.method === "Target.setAutoAttach" && !command.sessionId)).toHaveLength(1);
    expect(fixture.socket.terminated).toBe(false);
  });

  it("gates nested OOPIF renderer startup independently and clears both readiness deadlines", async () => {
    vi.useFakeTimers();
    const fixture = new Fixture();
    await fixture.initialize({ page: fixture.context.page() });
    fixture.intercept = command => ["Emulation.setUserAgentOverride", "Target.getTargetInfo", "Runtime.runIfWaitingForDebugger"].includes(command.method);
    fixture.socket.attach("frame", true, "initial", "iframe");
    const frameOverride = await fixture.command("Emulation.setUserAgentOverride", 2);
    const frameBarrier = await fixture.command("Target.getTargetInfo");
    fixture.socket.attach("nested-frame", true, "frame", "iframe");
    const nestedOverride = await fixture.command("Emulation.setUserAgentOverride", 3);
    const nestedBarrier = await fixture.command("Target.getTargetInfo", 2);
    expect(fixture.socket.commands.some(command => command.method === "Runtime.runIfWaitingForDebugger")).toBe(false);
    fixture.socket.reply(nestedBarrier, { targetInfo: { targetId: "nested-frame" } });
    const nestedResume = await fixture.command("Runtime.runIfWaitingForDebugger");
    expect(nestedResume.sessionId).toBe(nestedOverride.sessionId);
    expect(fixture.socket.commands.some(command => command.method === "Runtime.runIfWaitingForDebugger" && command.sessionId === frameOverride.sessionId)).toBe(false);
    fixture.socket.reply(nestedOverride);
    fixture.socket.reply(nestedResume);
    fixture.socket.reply(frameBarrier, { targetInfo: { targetId: "frame" } });
    const frameResume = await fixture.command("Runtime.runIfWaitingForDebugger", 2);
    expect(frameResume.sessionId).toBe(frameOverride.sessionId);
    fixture.socket.reply(frameOverride);
    fixture.socket.reply(frameResume);
    await flush();
    await vi.advanceTimersByTimeAsync(5001);
    expect(fixture.context.closed).toBe(false);
    expect(fixture.socket.terminated).toBe(false);
  });

  it("settles pending OOPIF descendants when their page detaches without closing other pages", async () => {
    vi.useFakeTimers();
    const fixture = new Fixture();
    await fixture.initialize({ page: fixture.context.page() });
    fixture.intercept = command => Boolean(command.sessionId?.includes("frame")) &&
      ["Emulation.setUserAgentOverride", "Target.getTargetInfo"].includes(command.method);
    fixture.socket.attach("frame", true, "initial", "iframe");
    await fixture.command("Emulation.setUserAgentOverride", 2);
    fixture.socket.attach("nested-frame", true, "frame", "iframe");
    await fixture.command("Emulation.setUserAgentOverride", 3);
    fixture.socket.event("Target.detachedFromTarget", { sessionId: "session-initial" });
    await flush();
    fixture.socket.attach("survivor");
    await fixture.initialize({ page: fixture.context.page("survivor") });
    await vi.advanceTimersByTimeAsync(5001);
    expect(fixture.context.closed).toBe(false);
    expect(fixture.socket.commands.some(command => command.method === "Runtime.runIfWaitingForDebugger" && command.sessionId?.includes("frame"))).toBe(false);
  });

  it("fails closed on a child-frame override rejection without resuming that renderer", async () => {
    const fixture = new Fixture();
    await fixture.initialize({ page: fixture.context.page() });
    fixture.intercept = command => command.method === "Emulation.setUserAgentOverride" || command.method === "Target.getTargetInfo";
    fixture.socket.attach("frame", true, "initial", "iframe");
    const override = await fixture.command("Emulation.setUserAgentOverride", 2);
    fixture.socket.reject(override, "iframe override rejected");
    await expect(fixture.initialize({ page: fixture.context.page() })).rejects.toThrow("iframe override rejected");
    expect(fixture.context.closed).toBe(true);
    expect(fixture.socket.commands.some(command => command.method === "Runtime.runIfWaitingForDebugger")).toBe(false);
  });

  it.each(["Target.targetDestroyed", "Target.detachedFromTarget"])("settles %s during apply without stranding another target", async event => {
    const fixture = new Fixture();
    fixture.existing.push({ targetId: "transient", type: "page" });
    fixture.intercept = command => command.method === "Emulation.setUserAgentOverride" && command.sessionId === "session-transient";
    const ready = fixture.initialize({ page: fixture.context.page() });
    const transient = await fixture.command("Emulation.setUserAgentOverride", 2);
    fixture.socket.event(event, { targetId: "transient", sessionId: "session-transient" });
    // An override response already in flight must never revive this destroyed session.
    fixture.socket.reply(transient);
    await ready;
    expect(fixture.context.closed).toBe(false);
    expect(fixture.socket.commands.some(command => command.method === "Runtime.runIfWaitingForDebugger" && command.sessionId === transient.sessionId)).toBe(false);
    fixture.socket.attach("survivor");
    await fixture.initialize({ page: fixture.context.page("survivor") });
  });

  it("handles a destroyed target when the snapshot response is already stale", async () => {
    const fixture = new Fixture();
    fixture.existing.push({ targetId: "gone", type: "page" });
    fixture.attachExisting = false;
    fixture.intercept = (command, socket) => {
      if (command.method !== "Target.getTargets") return false;
      socket.event("Target.targetDestroyed", { targetId: "gone" });
      socket.reply(command, { targetInfos: [...fixture.existing] });
      socket.attach("initial", false);
      return true;
    };
    await fixture.initialize({ page: fixture.context.page() });
    expect(fixture.context.closed).toBe(false);
  });

  it("does not resume a target destroyed in the browser barrier ACK microtask gap", async () => {
    const fixture = new Fixture();
    await fixture.initialize({ page: fixture.context.page() });
    fixture.intercept = command => command.method === "Target.getTargetInfo";
    fixture.socket.attach("transient");
    const barrier = await fixture.command("Target.getTargetInfo");
    const result = fixture.initialize({ page: fixture.context.page("transient") }).catch(error => error as Error);
    await flush();
    fixture.socket.reply(barrier, { targetInfo: { targetId: "transient" } });
    fixture.socket.event("Target.targetDestroyed", { targetId: "transient" });
    expect((await result).message).toContain("page target closed");
    expect(fixture.context.closed).toBe(false);
    expect(fixture.socket.commands.some(command => command.method === "Runtime.runIfWaitingForDebugger")).toBe(false);
  });

  it("closes the owned context before releasing a failed debugger gate or rejecting the hook", async () => {
    const fixture = new Fixture();
    await fixture.initialize({ page: fixture.context.page() });
    const { promise: closing, resolve: finishClose } = Promise.withResolvers<void>();
    fixture.context.close = vi.fn(() => closing);
    fixture.intercept = command => command.method === "Emulation.setUserAgentOverride" || command.method === "Target.getTargetInfo";
    fixture.socket.attach("rejected");
    const override = await fixture.command("Emulation.setUserAgentOverride", 2);
    let hookRejected = false;
    const failed = fixture.initialize({ page: fixture.context.page("rejected") }).catch(error => {
      hookRejected = true;
      return error as Error;
    });
    fixture.socket.reject(override, "override rejected");
    await flush();
    expect(fixture.context.close).toHaveBeenCalledTimes(1);
    expect(fixture.socket.terminated).toBe(false);
    expect(hookRejected).toBe(false);
    expect(fixture.socket.commands.some(command => command.method === "Runtime.runIfWaitingForDebugger")).toBe(false);
    finishClose();
    expect((await failed).message).toBe("Cockpit browser user-agent policy failed: override rejected");
    expect(fixture.socket.terminated).toBe(true);
    await expect(fixture.initialize({ page: fixture.context.page() })).rejects.toThrow("override rejected");
    expect(fixture.socketCount).toBe(1);
  });

  it("retains the debugger gate and reports failure if neither public owner can close", async () => {
    const fixture = new Fixture();
    await fixture.initialize({ page: fixture.context.page() });
    fixture.context.close = vi.fn(async () => { throw new Error("context shutdown rejected"); });
    fixture.context.publicBrowser.close = vi.fn(async () => { throw new Error("browser shutdown rejected"); });
    fixture.intercept = command => command.method === "Emulation.setUserAgentOverride" || command.method === "Target.getTargetInfo";
    fixture.socket.attach("rejected");
    const override = await fixture.command("Emulation.setUserAgentOverride", 2);
    const failure = fixture.initialize({ page: fixture.context.page("rejected") }).catch(error => error as Error);
    fixture.socket.reject(override, "override rejected");
    expect((await failure).message).toContain("override rejected; owned browser shutdown failed");
    expect(fixture.context.close).toHaveBeenCalledTimes(1);
    expect(fixture.context.publicBrowser.close).toHaveBeenCalledTimes(1);
    expect(fixture.socket.terminated).toBe(false);
    expect(fixture.socket.commands.some(command => command.method === "Runtime.runIfWaitingForDebugger")).toBe(false);
    fixture.socket.terminate();
  });

  it.each(["mismatched identity", "rejected command"])("never resumes a target with %s at its browser-side barrier", async fault => {
    const fixture = new Fixture();
    await fixture.initialize({ page: fixture.context.page() });
    fixture.intercept = (command, socket) => {
      if (command.method !== "Target.getTargetInfo") return false;
      if (fault === "mismatched identity") socket.reply(command, { targetInfo: { targetId: "different-target" } });
      else socket.reject(command, "startup barrier rejected");
      return true;
    };
    fixture.socket.attach("isolated");
    await expect(fixture.initialize({ page: fixture.context.page("isolated") })).rejects.toThrow(
      fault === "mismatched identity" ? "startup target identity changed" : "startup barrier rejected",
    );
    expect(fixture.context.closed).toBe(true);
    expect(fixture.socket.commands.some(command => command.method === "Runtime.runIfWaitingForDebugger")).toBe(false);
  });

  it.each(["Browser.getVersion", "Emulation.setUserAgentOverride", "Target.getTargetInfo", "Runtime.runIfWaitingForDebugger"])("fails closed on bounded %s timeout", async method => {
    vi.useFakeTimers();
    const fixture = new Fixture();
    if (method === "Target.getTargetInfo" || method === "Runtime.runIfWaitingForDebugger") fixture.attachExisting = false;
    fixture.intercept = command => command.method === method;
    const failure = fixture.initialize({ page: fixture.context.page() }).catch(error => error as Error);
    if (method === "Target.getTargetInfo" || method === "Runtime.runIfWaitingForDebugger") {
      await fixture.command("Target.getTargets");
      fixture.socket.attach("initial");
    }
    await fixture.command(method);
    await vi.advanceTimersByTimeAsync(5001);
    expect((await failure).message).toMatch(/Cockpit browser user-agent policy failed: timed out/);
    expect(fixture.context.close).toHaveBeenCalledTimes(1);
    expect(fixture.socket.terminated).toBe(true);
  });

  it("fails closed if the auto-attach response has no matching attached event", async () => {
    vi.useFakeTimers();
    const fixture = new Fixture();
    fixture.attachExisting = false;
    const failure = fixture.initialize({ page: fixture.context.page() }).catch(error => error as Error);
    await fixture.command("Target.getTargets");
    await vi.advanceTimersByTimeAsync(5001);
    expect((await failure).message).toContain("timed out waiting for page target");
    expect(fixture.context.closed).toBe(true);
  });

  it.each(["error", "close"])("rejects all waiting hooks and closes ownership on unexpected socket %s", async event => {
    const fixture = new Fixture();
    await fixture.initialize({ page: fixture.context.page() });
    fixture.intercept = command => command.method === "Emulation.setUserAgentOverride";
    fixture.socket.attach("pending");
    const failure = fixture.initialize({ page: fixture.context.page("pending") }).catch(error => error as Error);
    await fixture.command("Emulation.setUserAgentOverride", 2);
    fixture.socket.emit(event, new Error("transport fault"));
    expect((await failure).message).toContain("Cockpit browser user-agent policy failed: browser policy WebSocket");
    expect(fixture.context.close).toHaveBeenCalledTimes(1);
    expect(fixture.socket.terminated).toBe(true);
    expect(fixture.socket.listenerCount("message")).toBe(0);
  });

  it("normal ownership closure rejects pending requests and removes socket/owner listeners", async () => {
    const fixture = new Fixture();
    await fixture.initialize({ page: fixture.context.page() });
    fixture.intercept = command => command.method === "Emulation.setUserAgentOverride";
    fixture.socket.attach("pending");
    const failure = fixture.initialize({ page: fixture.context.page("pending") }).catch(error => error as Error);
    await fixture.command("Emulation.setUserAgentOverride", 2);
    await fixture.context.close();
    expect((await failure).message).toBe("owned browser closed");
    await flush();
    expect(fixture.context.close).toHaveBeenCalledTimes(1);
    expect(fixture.socket.terminated).toBe(true);
    expect(fixture.socket.eventNames()).toEqual([]);
    expect(fixture.context.listenerCount("close")).toBe(0);
    expect(fixture.context.publicBrowser.listenerCount("disconnected")).toBe(0);
  });

  it.each([undefined, "", 154])("rejects invalid native UA %s without any target override", async nativeUa => {
    const fixture = new Fixture();
    fixture.nativeUa = nativeUa;
    await expect(fixture.initialize({ page: fixture.context.page() })).rejects.toThrow("native browser user-agent is empty or invalid");
    expect(fixture.context.closed).toBe(true);
    expect(fixture.socket.commands.some(command => command.method === "Emulation.setUserAgentOverride")).toBe(false);
  });

  it.each(["0\n/devtools/browser/id\n", "70000\n/devtools/browser/id\n", "9222\nws://remote.invalid/devtools/browser/id\n", "9222\n/devtools/browser/id\nextra\n"])("fails closed before connecting with invalid owned endpoint %s", async record => {
    const fixture = new Fixture();
    writeFileSync(join(fixture.profile, "DevToolsActivePort"), record);
    await expect(fixture.initialize({ page: fixture.context.page() })).rejects.toThrow("DevToolsActivePort record is invalid");
    expect(fixture.context.closed).toBe(true);
    expect(fixture.socketCount).toBe(0);
  });

  it("fails closed on malformed binding and missing daemon-paired WebSocket facility", async () => {
    const malformed = new Fixture();
    writeFileSync(join(malformed.root, "browser-user-agent.json"), "{");
    await expect(malformed.initialize({ page: malformed.context.page() })).rejects.toThrow("Cockpit browser user-agent policy failed:");
    expect(malformed.context.closed).toBe(true);
    const missing = new Fixture();
    writeFileSync(join(missing.root, "node_modules", "playwright-core", "lib", "utilsBundle.js"), "module.exports = {};");
    await expect(missing.initialize({ page: missing.context.page() })).rejects.toThrow("CLI paired WebSocket facility is unavailable");
    expect(missing.context.closed).toBe(true);
    expect(missing.socketCount).toBe(0);
  });
});
