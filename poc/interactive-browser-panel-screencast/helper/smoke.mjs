#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import readline from 'node:readline';

const helperPath = path.join(path.dirname(fileURLToPath(import.meta.url)), 'browser-helper.mjs');
const child = spawn(process.env.NODE_BINARY || process.execPath, [helperPath], {
  stdio: ['pipe', 'pipe', 'inherit'],
});
const lines = readline.createInterface({ input: child.stdout, crlfDelay: Infinity });
const pending = new Map();
let nextId = 1;
let stopped = false;

function waitForExit() {
  if (child.exitCode !== null) return Promise.resolve();
  return new Promise((resolve) => child.once('exit', resolve));
}

lines.on('line', (line) => {
  let response;
  try { response = JSON.parse(line); } catch { return; }
  const request = pending.get(response.id);
  if (!request) return;
  pending.delete(response.id);
  clearTimeout(request.timer);
  if (response.ok === true) request.resolve(response.result);
  else request.reject(new Error(response.error || 'helper rejected request'));
});

child.once('exit', () => {
  for (const request of pending.values()) {
    clearTimeout(request.timer);
    request.reject(new Error('browser helper exited during smoke'));
  }
  pending.clear();
});

function request(method, fields = {}) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`timed out waiting for ${method}`));
    }, 30_000);
    pending.set(id, { resolve, reject, timer });
    child.stdin.write(`${JSON.stringify({ id, method, ...fields })}\n`);
  });
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function click(x, y) {
  await request('input', { event: { kind: 'mouseDown', x, y, button: 'left' } });
  return request('input', { event: { kind: 'mouseUp', x, y, button: 'left' } });
}

async function expectRejected(method, fields, text) {
  try {
    await request(method, fields);
  } catch (error) {
    assert(String(error.message).includes(text), `${method} error was not explicit`);
    return;
  }
  throw new Error(`${method} unexpectedly succeeded`);
}

async function run() {
  const acceptedHttps = await request('validateUrl', { url: 'https://google.de' });
  assert(acceptedHttps.url === 'https://google.de/', 'absolute HTTPS URL was not accepted');
  const acceptedBare = await request('validateUrl', { url: 'google.de' });
  assert(acceptedBare.url === 'https://google.de/', 'bare domain was not normalized to HTTPS');
  await expectRejected('validateUrl', { url: 'https://user:pass@google.de' }, 'credentials');
  await expectRejected('validateUrl', { url: 'ftp://google.de' }, 'http:// and https://');
  const first = await request('start');
  assert(first.width === 1024 && first.height === 720, 'start did not return the fixed viewport');
  assert(first.pngDataUrl.startsWith('data:image/png;base64,'), 'start did not return a PNG data URL');

  const reloaded = await request('reload');
  assert(reloaded.url === first.url, 'reload left the local fixture URL');

  let focused = false;
  for (const [x, y] of [[300, 255], [300, 280], [300, 230], [300, 305]]) {
    const ack = await click(x, y);
    const inspected = await request('inspect');
    if (inspected.activeElement === 'INPUT#name') {
      assert(ack.cursor === 'text', `input cursor was ${ack.cursor}`);
      focused = true;
      break;
    }
  }
  assert(focused, 'mouse input did not focus the fixture input');
  const beforeTyping = await request('snapshot');
  for (const [key, code] of [['A', 'KeyA'], ['d', 'KeyD'], ['a', 'KeyA']]) {
    const down = await request('input', { event: { kind: 'keyDown', key, code, text: key } });
    assert(typeof down.cursor === 'string', 'keyDown did not return a cursor acknowledgement');
    await request('input', { event: { kind: 'keyUp', key, code } });
  }

  let greeting;
  for (const [x, y] of [[820, 230], [820, 255], [820, 280], [820, 305], [800, 255], [840, 255]]) {
    const ack = await click(x, y);
    const inspected = await request('inspect');
    if (inspected.fixtureStatus?.includes('Hello, Ada')) {
      assert(ack.cursor === 'pointer', `button cursor was ${ack.cursor}`);
      greeting = inspected;
      break;
    }
  }
  assert(greeting, 'pointer click did not activate the fixture button');

  const beforeScroll = await request('inspect');
  await request('input', { event: { kind: 'mouseMove', x: 500, y: 600, buttons: 0, modifiers: 0 } });
  const wheelAck = await request('input', { event: { kind: 'wheel', x: 500, y: 600, deltaX: 0, deltaY: 500, modifiers: 0 } });
  assert(!wheelAck.pngDataUrl && typeof wheelAck.cursor === 'string', 'wheel acknowledgement was not small');
  const afterScroll = await request('inspect');
  assert(afterScroll.scrollY > beforeScroll.scrollY, 'wheel input did not change scrollY');

  await request('input', { event: { kind: 'mouseDown', x: 210, y: 120, button: 'left', buttons: 1 } });
  await request('input', { event: { kind: 'mouseMove', x: 620, y: 120, button: 'left', buttons: 1 } });
  await request('input', { event: { kind: 'mouseUp', x: 620, y: 120, button: 'left', buttons: 0 } });
  const selected = await request('inspect');
  assert(selected.selectedText?.length > 0, 'drag input did not select fixture text');

  const finalSnapshot = await request('snapshot');
  assert(finalSnapshot.pngDataUrl !== beforeTyping.pngDataUrl, 'frame did not change after typing, scrolling, and selection');
  console.log('interactive-browser-panel smoke passed: pointer, typing, button, wheel, selection, reload, inspect, snapshot');
}

try {
  await run();
  await request('stop');
  stopped = true;
} finally {
  if (!stopped) {
    try { await request('stop'); } catch { /* helper may have failed before startup */ }
  }
  child.stdin.end();
  await waitForExit();
  lines.close();
}
