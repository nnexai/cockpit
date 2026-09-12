#!/usr/bin/env node

import { existsSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import process from 'node:process';
import readline from 'node:readline';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const FIXTURE_PATH = path.join(ROOT, 'fixture', 'index.html');
const WIDTH = 1024;
const HEIGHT = 720;
const PORT = Number(process.env.CEF_OSR_PANEL_CDP_PORT || 9223);
const MAX_LINE = 8 * 1024 * 1024;
const FRAME_TIMEOUT_MS = 8_000;

let child;
let childLines;
let socket;
let fixtureUrl;
let latestFrame;
let frameSequence = 0;
let pendingFrame;
let frameWriting = false;
let nextCdpId = 1;
let shuttingDown = false;
let mouseButtons = 0;
let lastPointer = { x: 0, y: 0 };
const cdpPending = new Map();

function protocolError(message) {
  const error = new Error(message);
  error.protocol = true;
  return error;
}
function assertObject(value, name) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw protocolError(`${name} must be an object`);
  return value;
}
function assertString(value, name, max = 256) {
  if (typeof value !== 'string' || value.length === 0 || value.length > max) throw protocolError(`${name} must be a non-empty string of at most ${max} characters`);
  return value;
}
function assertNumber(value, name, min, max) {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < min || value > max) throw protocolError(`${name} must be a finite number between ${min} and ${max}`);
  return value;
}
function binaryPath() {
  return process.env.CEF_OSR_PANEL_BINARY || path.join(ROOT, 'build', 'Release', 'cef-osr-panel');
}
function writeFrameEvent() {
  if (frameWriting) return;
  frameWriting = true;
  const flush = () => {
    const frame = pendingFrame;
    pendingFrame = undefined;
    if (frame) {
      const line = JSON.stringify({ event: 'cefFrame', frame });
      if (line.length > MAX_LINE) {
        frameWriting = false;
        throw new Error('CEF frame exceeded protocol limit');
      }
      if (!process.stdout.write(`${line}\n`)) {
        process.stdout.once('drain', flush);
        return;
      }
    }
    if (pendingFrame) queueMicrotask(flush);
    else frameWriting = false;
  };
  flush();
}
function publishFrame(frame) {
  latestFrame = frame;
  pendingFrame = frame;
  writeFrameEvent();
}
function waitForFrameAfter(sequence) {
  const deadline = performance.now() + FRAME_TIMEOUT_MS;
  return new Promise((resolve, reject) => {
    const check = () => {
      if (latestFrame && latestFrame.sequence > sequence) return resolve(latestFrame);
      if (performance.now() >= deadline) return reject(new Error(`timed out waiting for CEF frame after ${sequence}`));
      setTimeout(check, 25);
    };
    check();
  });
}
function handleChildLine(line) {
  if (line.length > MAX_LINE) return;
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if (message.event !== 'frame' || !message.frame) return;
  const frame = message.frame;
  if (typeof frame.jpegDataUrl !== 'string' || frame.jpegDataUrl.length > MAX_LINE) return;
  frameSequence = Math.max(frameSequence, Number(frame.sequence) || 0);
  publishFrame({
    jpegDataUrl: frame.jpegDataUrl,
    sequence: frameSequence,
    width: Number(frame.width) || WIDTH,
    height: Number(frame.height) || HEIGHT,
  });
}
async function waitForTarget() {
  const deadline = performance.now() + 15_000;
  while (performance.now() < deadline) {
    try {
      const response = await fetch(`http://127.0.0.1:${PORT}/json/list`);
      if (response.ok) {
        const targets = await response.json();
        const target = targets.find((item) => item.type === 'page' && item.webSocketDebuggerUrl);
        if (target) return target;
      }
    } catch { /* sidecar is still starting */ }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`CEF CDP page target did not appear on localhost:${PORT}`);
}
function connect(url) {
  return new Promise((resolve, reject) => {
    import('ws').then(({ default: WebSocket }) => {
      const connection = new WebSocket(url);
      const fail = (error) => reject(error);
      connection.once('error', fail);
      connection.once('open', () => {
        connection.removeListener('error', fail);
        connection.on('message', (raw) => {
          let message;
          try { message = JSON.parse(raw.toString()); } catch { return; }
          if (message.id === undefined) return;
          const pending = cdpPending.get(message.id);
          if (!pending) return;
          cdpPending.delete(message.id);
          clearTimeout(pending.timer);
          if (message.error) pending.reject(new Error(message.error.message));
          else pending.resolve(message.result || {});
        });
        connection.on('close', () => {
          for (const pending of cdpPending.values()) pending.reject(new Error('CEF CDP connection closed'));
          cdpPending.clear();
        });
        resolve(connection);
      });
    }).catch(reject);
  });
}
function cdpCall(method, params = {}) {
  if (!socket) throw new Error('CEF CDP is not connected');
  const id = nextCdpId++;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      cdpPending.delete(id);
      reject(new Error(`CEF CDP call timed out: ${method}`));
    }, FRAME_TIMEOUT_MS);
    cdpPending.set(id, { resolve, reject, timer });
    socket.send(JSON.stringify({ id, method, params }));
  });
}
function navigationUrl(value) {
  let candidate = assertString(value, 'url', 2048).trim();
  if (!/^[a-z][a-z\d+.-]*:\/\//i.test(candidate)) candidate = `https://${candidate}`;
  let parsed;
  try { parsed = new URL(candidate); } catch { throw protocolError('url must be an absolute URL'); }
  if (!['http:', 'https:', 'file:'].includes(parsed.protocol)) throw protocolError('navigation only supports http, https, or file URLs');
  if (parsed.username || parsed.password) throw protocolError('navigation URL must not contain credentials');
  return parsed.href;
}
async function evaluateState() {
  const result = await cdpCall('Runtime.evaluate', {
    expression: `(() => {
      const element = document.elementFromPoint(${lastPointer.x}, ${lastPointer.y});
      const active = document.activeElement;
      return {
        title: document.title.slice(0, 200), url: location.href.slice(0, 2048),
        activeElement: active ? ((active.tagName || 'UNKNOWN') + (active.id ? '#' + active.id.slice(0, 80) : '')) : 'BODY',
        fixtureStatus: (document.querySelector('#result')?.textContent || '').slice(0, 240),
        selectedText: (window.getSelection()?.toString() || '').slice(0, 240),
        scrollY: Math.max(0, Math.round(window.scrollY)),
        cursor: getComputedStyle(element || document.body).cursor
      };
    })()`, returnByValue: true,
  });
  return result.result?.value || {};
}
async function snapshot() {
  if (!socket || !fixtureUrl) throw new Error('CEF browser is not started');
  const state = await evaluateState();
  const frame = latestFrame || await waitForFrameAfter(0);
  return { ...frame, title: state.title || '', url: state.url || fixtureUrl, cursor: state.cursor || 'default' };
}
function validateEvent(event) {
  assertObject(event, 'event');
  const kind = assertString(event.kind, 'event.kind', 32);
  if (event.modifiers !== undefined && (!Number.isInteger(event.modifiers) || event.modifiers < 0 || event.modifiers > 15)) throw protocolError('event.modifiers is invalid');
  if (['mouseMove', 'mouseDown', 'mouseUp'].includes(kind)) {
    assertNumber(event.x, 'event.x', 0, WIDTH); assertNumber(event.y, 'event.y', 0, HEIGHT);
    if (kind !== 'mouseMove' && !['left', 'middle', 'right'].includes(event.button || 'left')) throw protocolError('event.button is invalid');
    return kind;
  }
  if (kind === 'wheel') {
    assertNumber(event.x, 'event.x', 0, WIDTH); assertNumber(event.y, 'event.y', 0, HEIGHT);
    assertNumber(event.deltaX || 0, 'event.deltaX', -10000, 10000); assertNumber(event.deltaY || 0, 'event.deltaY', -10000, 10000);
    return kind;
  }
  if (kind === 'keyDown' || kind === 'keyUp') {
    assertString(event.key, 'event.key', 128); assertString(event.code, 'event.code', 128);
    if (event.text !== undefined && (typeof event.text !== 'string' || event.text.length > 16)) throw protocolError('event.text is invalid');
    return kind;
  }
  throw protocolError(`unsupported input event kind: ${kind}`);
}
function buttonMask(button) { return button === 'left' ? 1 : button === 'right' ? 2 : button === 'middle' ? 4 : 0; }
async function dispatchInput(event) {
  const kind = validateEvent(event);
  if (kind === 'mouseMove' || kind === 'mouseDown' || kind === 'mouseUp') {
    const button = kind === 'mouseMove' ? 0 : buttonMask(event.button || 'left');
    if (kind === 'mouseDown') mouseButtons |= button;
    const buttons = kind === 'mouseUp' ? mouseButtons & ~button : (event.buttons ?? mouseButtons);
    const cdpButton = kind === 'mouseMove' ? (buttons & 1 ? 'left' : buttons & 2 ? 'right' : buttons & 4 ? 'middle' : 'none') : (event.button || 'left');
    lastPointer = { x: event.x, y: event.y };
    await cdpCall('Input.dispatchMouseEvent', { type: { mouseMove: 'mouseMoved', mouseDown: 'mousePressed', mouseUp: 'mouseReleased' }[kind], x: event.x, y: event.y, button: cdpButton, clickCount: kind === 'mouseMove' ? 0 : 1, buttons, modifiers: event.modifiers || 0 });
    if (kind === 'mouseUp') mouseButtons &= ~button;
  } else if (kind === 'wheel') {
    lastPointer = { x: event.x, y: event.y };
    await cdpCall('Input.dispatchMouseEvent', { type: 'mouseWheel', x: event.x, y: event.y, deltaX: event.deltaX || 0, deltaY: event.deltaY || 0, modifiers: event.modifiers || 0 });
  } else {
    await cdpCall('Input.dispatchKeyEvent', { type: kind === 'keyDown' ? 'keyDown' : 'keyUp', key: event.key, code: event.code, modifiers: event.modifiers || 0, ...(kind === 'keyDown' && event.text ? { text: event.text, unmodifiedText: event.text } : {}) });
  }
  const state = await evaluateState();
  return { cursor: state.cursor || 'default' };
}
async function start() {
  if (socket) return snapshot();
  const executable = binaryPath();
  if (!existsSync(executable)) throw new Error(`CEF executable not found: ${executable}; build with CEF_ROOT first`);
  if (!existsSync(FIXTURE_PATH)) throw new Error(`CEF fixture not found: ${FIXTURE_PATH}`);
  fixtureUrl = `file://${FIXTURE_PATH}`;
  child = spawn(executable, [`--fixture=${fixtureUrl}`, `--remote-debugging-port=${PORT}`], { cwd: ROOT, stdio: ['ignore', 'pipe', 'inherit'], env: process.env });
  childLines = readline.createInterface({ input: child.stdout, crlfDelay: Infinity });
  childLines.on('line', handleChildLine);
  const target = await waitForTarget();
  socket = await connect(target.webSocketDebuggerUrl);
  await cdpCall('Runtime.enable');
  await cdpCall('Page.enable');
  await waitForFrameAfter(0);
  return snapshot();
}
async function navigate(url) {
  const baseline = latestFrame?.sequence || 0;
  await cdpCall('Page.navigate', { url: navigationUrl(url) });
  await waitForFrameAfter(baseline);
  return snapshot();
}
async function reload() {
  const baseline = latestFrame?.sequence || 0;
  await cdpCall('Page.reload', { ignoreCache: true });
  await waitForFrameAfter(baseline);
  return snapshot();
}
async function stop() {
  if (shuttingDown) return;
  shuttingDown = true;
  try { socket?.close(); } catch {}
  socket = undefined;
  if (child && child.exitCode === null) {
    child.kill('SIGTERM');
    await new Promise((resolve) => child.once('exit', resolve));
  }
  childLines?.close();
  child = undefined;
  childLines = undefined;
  fixtureUrl = undefined;
  latestFrame = undefined;
  pendingFrame = undefined;
  shuttingDown = false;
}
async function handle(request) {
  assertObject(request, 'request');
  const method = assertString(request.method, 'method', 64);
  if (method === 'start') return start();
  if (method === 'snapshot') return snapshot();
  if (method === 'inspect') { const state = await evaluateState(); return { status: 'ready', ...state, scrollY: Number(state.scrollY) || 0 }; }
  if (method === 'evaluate') {
    const expression = assertString(request.expression, 'expression', 4096);
    return cdpCall('Runtime.evaluate', { expression, returnByValue: true });
  }
  if (method === 'input') return dispatchInput(request.event);
  if (method === 'navigate') return navigate(request.url);
  if (method === 'reload') return reload();
  if (method === 'stop') { await stop(); return { status: 'stopped' }; }
  throw protocolError(`unknown method: ${method}`);
}
function writeResponse(response) {
  const line = JSON.stringify(response);
  if (line.length > MAX_LINE) process.stdout.write(`${JSON.stringify({ ok: false, error: 'response exceeded protocol limit' })}\n`);
  else process.stdout.write(`${line}\n`);
}
const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
process.on('SIGTERM', () => { void stop().finally(() => process.exit(0)); });
process.on('SIGINT', () => { void stop().finally(() => process.exit(0)); });
try {
  for await (const line of input) {
    if (line.length > MAX_LINE || !line.trim()) continue;
    let request;
    try { request = JSON.parse(line); } catch { writeResponse({ ok: false, error: 'request is not valid JSON' }); continue; }
    try { writeResponse({ ok: true, id: request.id ?? null, result: await handle(request) }); if (request.method === 'stop') break; }
    catch (error) { writeResponse({ ok: false, id: request.id ?? null, error: String(error?.message || error) }); }
  }
} finally { await stop(); }
