#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import readline from 'node:readline';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const helperPath = path.join(ROOT, 'helper', 'cef-helper.mjs');
const BINARY = process.env.CEF_OSR_PANEL_BINARY || path.join(ROOT, 'build', 'Release', 'cef-osr-panel');
const TIMEOUT_MS = 15_000;
if (!existsSync(helperPath)) throw new Error(`CEF helper not found: ${helperPath}`);
if (!existsSync(BINARY)) throw new Error(`CEF executable not found: ${BINARY}; configure/build with CEF_ROOT first`);

const child = spawn(process.env.NODE_BINARY || process.execPath, [helperPath], {
  cwd: ROOT,
  stdio: ['pipe', 'pipe', 'inherit'],
  env: { ...process.env, CEF_OSR_PANEL_BINARY: BINARY },
});
const lines = readline.createInterface({ input: child.stdout, crlfDelay: Infinity });
const pending = new Map();
let nextId = 1;
let latestFrame;
lines.on('line', (line) => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if (message.event === 'cefFrame') { latestFrame = message.frame; return; }
  const waiter = pending.get(message.id);
  if (!waiter) return;
  pending.delete(message.id);
  clearTimeout(waiter.timer);
  if (message.ok) waiter.resolve(message.result);
  else waiter.reject(new Error(message.error || 'CEF helper rejected request'));
});
function request(method, fields = {}) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`timed out waiting for ${method}`)); }, TIMEOUT_MS);
    pending.set(id, { resolve, reject, timer });
    child.stdin.write(`${JSON.stringify({ id, method, ...fields })}\n`);
  });
}
async function waitForFrameAfter(sequence, oldData) {
  const deadline = Date.now() + TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (latestFrame && latestFrame.sequence > sequence && latestFrame.jpegDataUrl !== oldData) return latestFrame;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(`timed out waiting for changed CEF frame after ${sequence}`);
}
let exitCode = 0;
try {
  const initial = await request('start');
  if (initial.width !== 1024 || initial.height !== 720) throw new Error(`unexpected frame dimensions: ${initial.width}x${initial.height}`);
  const navigated = await request('navigate', { url: initial.url });
  if (navigated.url !== initial.url || navigated.sequence <= initial.sequence) throw new Error(`navigation did not produce a fresh fixture frame: ${navigated.url} sequence ${navigated.sequence}`);
  const greeting = await request('evaluate', { expression: `(() => { const input = document.querySelector('#name'); input.value = 'Ada'; document.querySelector('#greet').click(); return document.querySelector('#result').textContent; })()` });
  if (greeting.result?.value !== 'Hello, Ada! CEF received your input.') throw new Error(`greeting did not update: ${greeting.result?.value}`);
  const firstChanged = await waitForFrameAfter(navigated.sequence, navigated.jpegDataUrl);
  const inspected = await request('inspect');
  if (!inspected.fixtureStatus.includes('Hello, Ada!')) throw new Error(`CEF inspect omitted greeting: ${inspected.fixtureStatus}`);
  await request('input', { event: { kind: 'wheel', x: 500, y: 650, deltaX: 0, deltaY: 600, modifiers: 0 } });
  const scrolled = await waitForFrameAfter(firstChanged.sequence, firstChanged.jpegDataUrl);
  const scrollState = await request('inspect');
  if (!(scrollState.scrollY > 0)) throw new Error(`fixture did not scroll: ${scrollState.scrollY}`);
  console.log(JSON.stringify({ scope: 'CEF OSR OnPaint/CDP helper smoke', firstFrame: initial.sequence, greetingFrame: firstChanged.sequence, scrollFrame: scrolled.sequence, scrollY: scrollState.scrollY }));
} catch (error) {
  exitCode = 1;
  console.error(`CEF smoke failed: ${error instanceof Error ? error.message : String(error)}`);
} finally {
  try { await request('stop'); } catch {}
  child.kill('SIGTERM');
}
process.exitCode = exitCode;
