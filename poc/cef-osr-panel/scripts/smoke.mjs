#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

let WebSocket;
try {
  ({ default: WebSocket } = await import('ws'));
} catch {
  throw new Error('smoke.mjs requires the ws package; run npm install in poc/cef-osr-panel');
}

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const BINARY = process.env.CEF_OSR_PANEL_BINARY || path.join(ROOT, 'build', 'Release', 'cef-osr-panel');
const PORT = Number(process.env.CEF_OSR_PANEL_CDP_PORT || 9223);
const FIXTURE = path.join(ROOT, 'fixture', 'index.html');
const FIXTURE_URL = `file://${FIXTURE}`;
const STARTUP_TIMEOUT_MS = 15_000;
const FRAME_TIMEOUT_MS = 8_000;

if (!existsSync(BINARY)) {
  throw new Error(`CEF executable not found: ${BINARY}; configure and build with -DCEF_ROOT=... first`);
}
if (!existsSync(FIXTURE)) {
  throw new Error(`fixture not found: ${FIXTURE}`);
}

const child = spawn(BINARY, [
  `--fixture=${FIXTURE_URL}`,
  `--remote-debugging-port=${PORT}`,
], {
  cwd: ROOT,
  stdio: ['ignore', 'pipe', 'inherit'],
  env: { ...process.env },
});
const lines = createInterface({ input: child.stdout, crlfDelay: Infinity });
const frames = [];
let output = '';
lines.on('line', (line) => {
  output += `${line}\n`;
  const match = /^FRAME sequence=(\d+) width=(\d+) height=(\d+) checksum=(\d+)$/.exec(line);
  if (match) {
    frames.push({ sequence: Number(match[1]), width: Number(match[2]), height: Number(match[3]), checksum: match[4] });
  }
});

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function waitFor(predicate, timeoutMs, description) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await predicate();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`timed out waiting for ${description}`);
}

async function cdpTarget() {
  try {
    const response = await fetch(`http://127.0.0.1:${PORT}/json/list`);
    if (!response.ok) return null;
    const targets = await response.json();
    return targets.find((target) => target.type === 'page' && target.webSocketDebuggerUrl) || null;
  } catch {
    return null;
  }
}

function connect(url) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url);
    const onError = (error) => reject(error);
    socket.once('error', onError);
    socket.once('open', () => {
      socket.removeListener('error', onError);
      resolve(socket);
    });
  });
}

function cdpCall(socket, method, params = {}) {
  const id = cdpCall.nextId++;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      socket.removeListener('message', onMessage);
      reject(new Error(`CDP call timed out: ${method}`));
    }, FRAME_TIMEOUT_MS);
    const onMessage = (raw) => {
      let message;
      try { message = JSON.parse(raw.toString()); } catch { return; }
      if (message.id !== id) return;
      clearTimeout(timer);
      socket.removeListener('message', onMessage);
      if (message.error) reject(new Error(`${method}: ${message.error.message}`));
      else resolve(message.result);
    };
    socket.on('message', onMessage);
    socket.send(JSON.stringify({ id, method, params }));
  });
}
cdpCall.nextId = 1;

async function waitForChangedFrame(previous, description) {
  return waitFor(() => frames.find((frame) => frame.sequence > previous.sequence && frame.checksum !== previous.checksum), FRAME_TIMEOUT_MS, description);
}

async function cleanup() {
  if (child.exitCode === null) {
    child.kill('SIGTERM');
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      new Promise((resolve) => setTimeout(resolve, 2_000)),
    ]);
    if (child.exitCode === null) child.kill('SIGKILL');
  }
}

let socket;
try {
  const target = await waitFor(cdpTarget, STARTUP_TIMEOUT_MS, 'CEF CDP page target');
  assert(target.url.startsWith('file://'), `expected local fixture target, got ${target.url}`);
  socket = await connect(target.webSocketDebuggerUrl);
  await cdpCall(socket, 'Runtime.enable');

  const initialFrame = await waitFor(() => frames[frames.length - 1], FRAME_TIMEOUT_MS, 'initial CEF OSR frame');
  assert(initialFrame.width > 0 && initialFrame.height > 0,
    `unexpected initial frame size: ${initialFrame.width}x${initialFrame.height}`);

  const greeting = await cdpCall(socket, 'Runtime.evaluate', {
    expression: `(() => {
      const input = document.querySelector('#name');
      input.value = 'Ada Lovelace';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      document.querySelector('#greet').click();
      return document.querySelector('#result').textContent;
    })()`,
    returnByValue: true,
  });
  assert(greeting.result?.value === 'Hello, Ada Lovelace! CEF received your input.',
    `unexpected greeting result: ${JSON.stringify(greeting.result?.value)}`);
  const greetingFrame = await waitForChangedFrame(initialFrame, 'OSR frame after CDP greeting interaction');

  const scroll = await cdpCall(socket, 'Runtime.evaluate', {
    expression: 'window.scrollTo(0, 900); window.scrollY',
    returnByValue: true,
  });
  assert(Number(scroll.result?.value) > 0, `scroll did not move the fixture: ${JSON.stringify(scroll.result?.value)}`);
  const scrolledFrame = await waitForChangedFrame(greetingFrame, 'OSR frame after CDP scrolling');

  console.log(`CEF OSR/CDP smoke passed: greeting frame ${greetingFrame.sequence}, scroll frame ${scrolledFrame.sequence}`);
} catch (error) {
  console.error(`CEF OSR/CDP smoke failed: ${error.message}`);
  if (output) console.error(output.trim());
  process.exitCode = 1;
} finally {
  if (socket) socket.close();
  await cleanup();
}
