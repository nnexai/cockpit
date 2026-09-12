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

async function run() {
  const first = await request('start');
  assert(first.width === 1024 && first.height === 720, 'start did not return the fixed viewport');
  assert(first.pngDataUrl.startsWith('data:image/png;base64,'), 'start did not return a PNG data URL');

  const reloaded = await request('reload');
  assert(reloaded.url === first.url, 'reload left the local fixture URL');

  let focused = false;
  for (const [x, y] of [[300, 230], [300, 255], [300, 280], [300, 305]]) {
    await click(x, y);
    const inspected = await request('inspect');
    if (inspected.activeElement === 'INPUT#name') { focused = true; break; }
  }
  assert(focused, 'mouse input did not focus the fixture input');
  for (const [key, code] of [['A', 'KeyA'], ['d', 'KeyD'], ['a', 'KeyA']]) {
    await request('input', { event: { kind: 'keyDown', key, code, text: key } });
    await request('input', { event: { kind: 'keyUp', key, code } });
  }

  let greeting;
  for (const [x, y] of [[820, 230], [820, 255], [820, 280], [820, 305], [800, 255], [840, 255]]) {
    await click(x, y);
    const inspected = await request('inspect');
    if (inspected.fixtureStatus?.includes('Hello, Ada')) { greeting = inspected; break; }
  }
  assert(greeting, 'pointer click did not activate the fixture button');

  const finalSnapshot = await request('snapshot');
  assert(finalSnapshot.pngDataUrl.length > first.pngDataUrl.length / 2, 'final screenshot was unexpectedly empty');
  console.log('interactive-browser-panel smoke passed: pointer focus, typing, button activation, reload, inspect, and snapshot');
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
