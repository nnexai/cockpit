'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');

const policies = new WeakMap();
const COMMAND_TIMEOUT_MS = 5000;
const STARTUP_TIMEOUT_MS = 10000;
const PREFIX = 'Cockpit browser user-agent policy failed: ';

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  // Targets can arrive without a hook caller (popups), but still need a terminal waiter.
  promise.catch(() => {});
  return { promise, resolve, reject };
}

function boundedRecord(filename, maximum) {
  const fd = fs.openSync(filename, 'r');
  try {
    const bytes = Buffer.alloc(maximum + 1);
    const count = fs.readSync(fd, bytes, 0, bytes.length, 0);
    if (count > maximum) throw new Error('owned browser binding exceeds its size limit');
    return bytes.subarray(0, count).toString('utf8');
  } finally {
    fs.closeSync(fd);
  }
}

function connection() {
  const binding = JSON.parse(boundedRecord(path.join(__dirname, 'browser-user-agent.json'), 16384));
  if (typeof binding.profile_path !== 'string' || !path.isAbsolute(binding.profile_path)) {
    throw new Error('owned browser profile binding is invalid');
  }
  const record = boundedRecord(path.join(binding.profile_path, 'DevToolsActivePort'), 512);
  const lines = record.replace(/\r?\n$/, '').split(/\r?\n/);
  const port = Number(lines[0]);
  if (lines.length !== 2 || !/^\d+$/.test(lines[0]) || port < 1 || port > 65535 ||
      !/^\/devtools\/browser\/[^\s?#]+$/.test(lines[1])) {
    throw new Error('owned browser DevToolsActivePort record is invalid');
  }
  if (!process.argv[1]) throw new Error('CLI daemon entry point is unavailable');
  const requireCore = createRequire(path.resolve(process.argv[1]));
  const packagePath = requireCore.resolve('playwright-core/package.json');
  if (requireCore(packagePath).name !== 'playwright-core') {
    throw new Error('CLI paired Playwright-core package identity is invalid');
  }
  const { ws: WebSocket } = requireCore(path.join(path.dirname(packagePath), 'lib', 'utilsBundle'));
  if (typeof WebSocket !== 'function') throw new Error('CLI paired WebSocket facility is unavailable');
  return { WebSocket, endpoint: `ws://127.0.0.1:${port}${lines[1]}` };
}

class TargetGone extends Error {
  constructor() { super('page target closed before user-agent readiness'); }
}

class Policy {
  constructor(context, browser) {
    this.context = context;
    this.browser = browser;
    this.nextId = 0;
    this.pending = new Map();
    this.targets = new Map();
    this.sessions = new Map();
    this.failure = null;
    this.stopped = false;
    this.startupDestroyed = new Set();
    this.onOwnerClose = () => this.stop(new Error('owned browser closed'));
    context.on('close', this.onOwnerClose);
    browser?.on('disconnected', this.onOwnerClose);
    // Defer guarded work until the registry contains this policy, including synchronous failures.
    this.started = Promise.resolve().then(() => this.start()).catch(async error => {
      await this.fail(error);
      throw this.failure || error;
    });
    this.started.catch(() => {});
  }

  target(id) {
    if (this.startupDestroyed?.has(id)) {
      const gone = deferred();
      gone.reject(new TargetGone());
      return gone;
    }
    let target = this.targets.get(id);
    if (!target) {
      target = { ...deferred(), id, sessionId: null, parentId: null, children: new Set(), timer: setTimeout(() => {
        void this.fail(new Error('timed out waiting for page target user-agent readiness'));
      }, COMMAND_TIMEOUT_MS) };
      this.targets.set(id, target);
    }
    return target;
  }

  async start() {
    const deadline = setTimeout(() => {
      void this.fail(new Error('timed out initializing the browser user-agent policy'));
    }, STARTUP_TIMEOUT_MS);
    try {
      const { WebSocket, endpoint } = connection();
      if (this.stopped) throw this.failure || new Error('owned browser closed');
      this.socket = new WebSocket(endpoint);
      const opened = deferred();
      this.opened = opened;
      this.onOpen = () => opened.resolve();
      this.onError = () => { void this.fail(new Error('browser policy WebSocket failed')); };
      this.onClose = () => {
        if (!this.stopped) void this.fail(new Error('browser policy WebSocket closed unexpectedly'));
        this.removeSocketListeners();
      };
      this.onMessage = data => {
        try { this.message(JSON.parse(data.toString())); }
        catch (error) { void this.fail(error); }
      };
      this.socket.on('open', this.onOpen);
      this.socket.on('error', this.onError);
      this.socket.on('close', this.onClose);
      this.socket.on('message', this.onMessage);
      await opened.promise;
      const version = await this.send('Browser.getVersion');
      if (typeof version.userAgent !== 'string' || !version.userAgent.trim()) {
        throw new Error('native browser user-agent is empty or invalid');
      }
      this.userAgent = version.userAgent.replaceAll('HeadlessChrome/', 'Chrome/');
      await this.send('Target.setAutoAttach', {
        autoAttach: true, waitForDebuggerOnStart: true, flatten: true,
        filter: [{ type: 'page', exclude: false }, { exclude: true }],
      });
      const { targetInfos } = await this.send('Target.getTargets');
      if (!Array.isArray(targetInfos)) throw new Error('browser page target snapshot is invalid');
      // Auto-attach's response is not an attached-event barrier. Reserve readiness for each
      // snapshot target, including those whose attached event has not arrived yet.
      await Promise.all(targetInfos.filter(info => info.type === 'page').map(info =>
        this.target(info.targetId).promise.catch(error => {
          if (!(error instanceof TargetGone)) throw error;
        })));
      if (this.stopped) throw this.failure || new Error('owned browser closed');
    } finally {
      this.startupDestroyed = null;
      clearTimeout(deadline);
    }
  }

  send(method, params = {}, sessionId) {
    if (this.stopped) return Promise.reject(this.failure || new Error('owned browser closed'));
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`timed out awaiting ${method}`));
      }, COMMAND_TIMEOUT_MS);
      this.pending.set(id, { resolve, reject, timer, sessionId });
      try {
        this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
      } catch (error) {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(error);
      }
    });
  }

  message(message) {
    if (this.stopped) return;
    if (message.id !== undefined) {
      const request = this.pending.get(message.id);
      if (!request) return;
      this.pending.delete(message.id);
      clearTimeout(request.timer);
      if (message.error) request.reject(new Error(message.error.message || 'browser policy command rejected'));
      else request.resolve(message.result || {});
      return;
    }
    if (message.method === 'Target.attachedToTarget') {
      const { sessionId, targetInfo, waitingForDebugger } = message.params;
      if ((targetInfo.type !== 'page' && targetInfo.type !== 'iframe') || this.startupDestroyed?.has(targetInfo.targetId)) return;
      const parentId = message.sessionId ? this.sessions.get(message.sessionId) : null;
      if (targetInfo.type === 'iframe' && !parentId) return;
      const target = this.target(targetInfo.targetId);
      if (target.sessionId) return;
      target.sessionId = sessionId;
      target.parentId = parentId;
      if (parentId) this.targets.get(parentId).children.add(target.id);
      this.sessions.set(sessionId, targetInfo.targetId);
      void this.configure(target, waitingForDebugger).catch(error => {
        if (!(error instanceof TargetGone)) void this.fail(error);
      });
    } else if (message.method === 'Target.detachedFromTarget') {
      const id = this.sessions.get(message.params.sessionId);
      if (id) this.forget(id);
    } else if (message.method === 'Target.targetDestroyed') {
      this.forget(message.params.targetId);
    }
  }

  async configure(target, waitingForDebugger) {
    // OOPIFs have their own renderer UA. Install their recursive document-only gate
    // while this page/frame is still paused, before it can create another renderer.
    await this.send('Target.setAutoAttach', {
      autoAttach: true, waitForDebuggerOnStart: true, flatten: true,
      filter: [{ type: 'iframe', exclude: false }, { exclude: true }],
    }, target.sessionId);
    if (this.targets.get(target.id) !== target) throw new TargetGone();
    const overriding = this.send('Emulation.setUserAgentOverride', { userAgent: this.userAgent }, target.sessionId);
    if (waitingForDebugger) {
      let overrideFailure;
      overriding.catch(error => { overrideFailure = error; });
      const resuming = (async () => {
        // Chromium stores the HTTP UA in its browser handler, but the final override
        // ACK can wait for a noopener renderer's startup gate. This browser-handled
        // ACK proves that ordered submission reached the target before resuming it.
        const { targetInfo } = await this.send('Target.getTargetInfo', {}, target.sessionId);
        if (overrideFailure) throw overrideFailure;
        if (this.targets.get(target.id) !== target) throw new TargetGone();
        if (targetInfo?.targetId !== target.id) throw new Error('browser user-agent startup target identity changed');
        // The renderer receives its queued override before this runtime command;
        // readiness still requires both final acknowledgements.
        await this.send('Runtime.runIfWaitingForDebugger', {}, target.sessionId);
      })();
      await Promise.all([overriding, resuming]);
    } else {
      await overriding;
    }
    if (this.targets.get(target.id) !== target) throw new TargetGone();
    clearTimeout(target.timer);
    target.resolve();
  }

  forget(id) {
    this.startupDestroyed?.add(id);
    const target = this.targets.get(id);
    if (!target) return;
    this.targets.delete(id);
    if (target.parentId) this.targets.get(target.parentId)?.children.delete(id);
    for (const childId of target.children) this.forget(childId);
    clearTimeout(target.timer);
    this.sessions.delete(target.sessionId);
    const error = new TargetGone();
    target.reject(error);
    for (const [requestId, request] of this.pending) {
      if (request.sessionId && request.sessionId === target.sessionId) {
        clearTimeout(request.timer);
        this.pending.delete(requestId);
        request.reject(error);
      }
    }
  }

  async ready(page) {
    try {
      await this.started;
      if (this.failure) throw this.failure;
      if (page.isClosed()) throw new TargetGone();
      const session = await this.context.newCDPSession(page);
      let targetId;
      try {
        ({ targetInfo: { targetId } } = await session.send('Target.getTargetInfo'));
      } finally {
        await session.detach();
      }
      if (this.stopped) throw this.failure || new Error('owned browser closed');
      if (page.isClosed()) throw new TargetGone();
      await this.target(targetId).promise;
    } catch (error) {
      if (this.failure) {
        await this.closing;
        throw this.failure;
      }
      if (error instanceof TargetGone || page.isClosed()) throw this.failure || error;
      await this.fail(error);
      throw this.failure || error;
    }
  }

  async fail(cause) {
    if (this.failure) return this.closing;
    if (this.stopped) return;
    this.failure = new Error(PREFIX + String(cause?.message || cause).slice(0, 240));
    // Keep paused targets paused; closing the owned context destroys them rather than
    // ever resuming a page that failed its override. Old CLI loaders swallow hook errors.
    this.stop(this.failure, false);
    this.closing = Promise.resolve().then(() => this.context.close()).catch(async error => {
      if (!this.browser) throw error;
      await this.browser.close();
    }).then(() => this.terminateSocket(), () => {
      // A failed shutdown must not disconnect the debugger gate and thereby release
      // an unconfigured page. Retain it and surface the shutdown failure explicitly.
      this.failure = new Error(this.failure.message + '; owned browser shutdown failed');
    });
    await this.closing;
  }

  stop(error, terminate = true) {
    if (this.stopped) return;
    this.stopped = true;
    this.context.removeListener('close', this.onOwnerClose);
    this.browser?.removeListener('disconnected', this.onOwnerClose);
    this.opened?.reject(error);
    for (const request of this.pending.values()) {
      clearTimeout(request.timer);
      request.reject(error);
    }
    this.pending.clear();
    for (const target of this.targets.values()) {
      clearTimeout(target.timer);
      target.reject(error);
    }
    this.targets.clear();
    this.sessions.clear();
    if (terminate) this.terminateSocket();
  }

  terminateSocket() {
    // Retain the error/close handlers until terminate finishes: ws can emit an error
    // while aborting a connection that has not opened yet.
    if (this.socket) {
      this.socket.removeListener('message', this.onMessage);
      this.socket.removeListener('open', this.onOpen);
      this.socket.terminate();
    }
  }

  removeSocketListeners() {
    this.socket.removeListener('open', this.onOpen);
    this.socket.removeListener('message', this.onMessage);
    this.socket.removeListener('error', this.onError);
    this.socket.removeListener('close', this.onClose);
  }
}

async function initialize({ page }) {
  const context = page.context();
  const browser = context.browser();
  const owner = browser || context;
  let policy = policies.get(owner);
  if (!policy) {
    policy = new Policy(context, browser);
    policies.set(owner, policy);
  }
  await policy.ready(page);
}

module.exports = initialize;
module.exports.default = initialize;
