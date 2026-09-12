#!/usr/bin/env node

import { get } from 'node:http';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import readline from 'node:readline';

const helperPath = path.join(path.dirname(fileURLToPath(import.meta.url)), 'browser-helper.mjs');
const child = spawn(process.env.NODE_BINARY || process.execPath, [helperPath], { stdio: ['pipe', 'pipe', 'inherit'] });
const lines = readline.createInterface({ input: child.stdout, crlfDelay: Infinity });
const pending = new Map();
let nextId = 1;
let stopped = false;
let mjpegRequest;
let stalledRequest;
let mjpegResponse;
let mjpegBuffer = Buffer.alloc(0);
let boundary;
const mjpegParts = [];

function waitForExit() { return child.exitCode !== null ? Promise.resolve() : new Promise((resolve) => child.once('exit', resolve)); }
lines.on('line', (line) => {
  let response;
  try { response = JSON.parse(line); } catch { return; }
  const request = pending.get(response.id);
  if (!request) return;
  pending.delete(response.id); clearTimeout(request.timer);
  if (response.ok === true) request.resolve(response.result); else request.reject(new Error(response.error || 'helper rejected request'));
});
child.once('exit', () => { for (const request of pending.values()) { clearTimeout(request.timer); request.reject(new Error('browser helper exited during smoke')); } pending.clear(); });
function request(method, fields = {}) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`timed out waiting for ${method}`)); }, 30_000);
    pending.set(id, { resolve, reject, timer }); child.stdin.write(`${JSON.stringify({ id, method, ...fields })}\n`);
  });
}
function assert(condition, message) { if (!condition) throw new Error(message); }

function consumeMjpeg(chunk) {
  mjpegBuffer = Buffer.concat([mjpegBuffer, chunk]);
  const marker = Buffer.from(`--${boundary}\r\n`, 'ascii');
  while (true) {
    const start = mjpegBuffer.indexOf(marker);
    if (start < 0) {
      if (mjpegBuffer.byteLength > 6 * 1024 * 1024 + 4 * 1024) throw new Error('MJPEG parser retained an oversized partial frame');
      return;
    }
    if (start > 0) mjpegBuffer = mjpegBuffer.subarray(start);
    const headerEnd = mjpegBuffer.indexOf('\r\n\r\n');
    if (headerEnd < 0) { if (mjpegBuffer.byteLength > marker.byteLength + 4 * 1024) throw new Error('MJPEG headers exceeded limit'); return; }
    const headers = mjpegBuffer.subarray(marker.byteLength, headerEnd).toString('ascii');
    const length = Number(/^content-length:\s*(\d+)\s*$/im.exec(headers)?.[1]);
    if (!Number.isInteger(length) || length <= 0 || length > 6 * 1024 * 1024) throw new Error('MJPEG part had an invalid Content-Length');
    const bodyStart = headerEnd + 4;
    const bodyEnd = bodyStart + length;
    if (mjpegBuffer.byteLength < bodyEnd + 2) return;
    if (mjpegBuffer[bodyEnd] !== 13 || mjpegBuffer[bodyEnd + 1] !== 10) throw new Error('MJPEG part trailer was invalid');
    const jpeg = mjpegBuffer.subarray(bodyStart, bodyEnd);
    assert(jpeg[0] === 0xff && jpeg[1] === 0xd8 && jpeg[jpeg.length - 2] === 0xff && jpeg[jpeg.length - 1] === 0xd9, 'MJPEG part was not a complete JPEG');
    const value = (name) => Number(new RegExp(`^${name}:\\s*(.+)\\s*$`, 'im').exec(headers)?.[1]);
    const geometry = { sequence: value('x-frame-sequence'), pixelWidth: value('x-pixel-width'), pixelHeight: value('x-pixel-height'), viewport: { width: value('x-viewport-width'), height: value('x-viewport-height'), scale: value('x-viewport-scale'), offsetX: value('x-viewport-offset-x'), offsetY: value('x-viewport-offset-y') } };
    assert(Number.isInteger(geometry.sequence) && geometry.sequence > 0 && Number.isInteger(geometry.pixelWidth) && geometry.pixelWidth > 0 && Number.isInteger(geometry.pixelHeight) && geometry.pixelHeight > 0 && Number.isInteger(geometry.viewport.width) && geometry.viewport.width > 0 && Number.isInteger(geometry.viewport.height) && geometry.viewport.height > 0 && Number.isFinite(geometry.viewport.scale) && Number.isFinite(geometry.viewport.offsetX) && Number.isFinite(geometry.viewport.offsetY), 'MJPEG headers omitted correlated geometry');
    mjpegParts.push({ jpegLength: jpeg.byteLength, geometry, receivedAt: performance.now() });
    mjpegBuffer = mjpegBuffer.subarray(bodyEnd + 2);
  }
}

function openMjpeg(streamUrl) {
  const url = new URL(streamUrl);
  assert(url.protocol === 'http:' && url.hostname === '127.0.0.1' && url.pathname === '/mjpeg', 'stream URL was not loopback HTTP MJPEG');
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out opening MJPEG stream')), 8_000);
    const request = get(url, (response) => {
      const contentType = String(response.headers['content-type'] || '');
      const streamBoundary = String(response.headers['x-mjpeg-boundary'] || '');
      if (response.statusCode !== 200 || contentType !== 'application/octet-stream' || !/^[A-Za-z0-9-]{1,80}$/.test(streamBoundary) || response.headers['access-control-allow-origin'] !== '*') { clearTimeout(timeout); reject(new Error(`MJPEG response was ${response.statusCode} ${contentType}`)); return; }
      boundary = streamBoundary;
      response.on('data', consumeMjpeg);
      response.on('error', reject);
      clearTimeout(timeout);
      resolve({ request, response });
    });
    request.once('error', reject);
  });
}

async function expectRejectedHttp(streamUrl) {
  const url = new URL(streamUrl);
  url.searchParams.set('token', 'malformed');
  const response = await new Promise((resolve, reject) => {
    const request = get(url, (result) => { result.resume(); resolve(result); });
    request.once('error', reject);
  });
  assert(response.statusCode === 404, 'malformed MJPEG request was accepted');
}

async function openStalledMjpeg(streamUrl) {
  const url = new URL(streamUrl);
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out opening stalled MJPEG stream')), 8_000);
    const request = get(url, (response) => {
      if (response.statusCode !== 200) { clearTimeout(timeout); reject(new Error('stalled MJPEG stream was rejected')); return; }
      const client = { request, response, closed: false };
      response.once('close', () => { client.closed = true; });
      response.pause();
      clearTimeout(timeout);
      resolve(client);
    });
    request.once('error', reject);
  });
}

async function waitUntil(predicate, description, timeoutMs = 8_000) {
  const deadline = performance.now() + timeoutMs;
  while (!predicate()) {
    if (performance.now() >= deadline) throw new Error(`timed out waiting for ${description}`);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}
async function waitForPartAfter(count, timeoutMs = 8_000) { await waitUntil(() => mjpegParts.length > count, 'an MJPEG part after control input', timeoutMs); return mjpegParts.at(-1); }
async function click(x, y) { await request('input', { event: { kind: 'mouseDown', x, y, button: 'left' } }); return request('input', { event: { kind: 'mouseUp', x, y, button: 'left' } }); }
async function expectRejected(method, fields, text) { try { await request(method, fields); } catch (error) { assert(String(error.message).includes(text), `${method} error was not explicit`); return; } throw new Error(`${method} unexpectedly succeeded`); }

async function run() {
  const acceptedHttps = await request('validateUrl', { url: 'https://google.de' }); assert(acceptedHttps.url === 'https://google.de/', 'absolute HTTPS URL was not accepted');
  const acceptedBare = await request('validateUrl', { url: 'google.de' }); assert(acceptedBare.url === 'https://google.de/', 'bare domain was not normalized to HTTPS');
  const scaledViewport = await request('testViewport', { metadata: { deviceWidth: 2048, deviceHeight: 1440, pageScaleFactor: 2, scrollOffsetX: 17, scrollOffsetY: 23 } });
  assert(scaledViewport.width === 1024 && scaledViewport.height === 720 && scaledViewport.scale === 2 && scaledViewport.offsetX === 17 && scaledViewport.offsetY === 23, 'screencast metadata scale conversion was incorrect');
  await expectRejected('validateUrl', { url: 'https://user:pass@google.de' }, 'credentials'); await expectRejected('validateUrl', { url: 'ftp://google.de' }, 'http:// and https://');
  const first = await request('start');
  assert(typeof first.streamUrl === 'string' && !('jpegDataUrl' in first), 'start mixed JPEG bytes into the control response');
  assert(first.pixelWidth > 0 && first.pixelHeight > 0 && first.viewport?.width > 0 && first.viewport?.height > 0, 'start omitted dynamic frame geometry');
  ({ request: mjpegRequest, response: mjpegResponse } = await openMjpeg(first.streamUrl));
  await waitUntil(() => mjpegParts.length > 0, 'initial MJPEG image with geometry headers');
  assert(mjpegParts[0].jpegLength > 0, 'first MJPEG part was empty');
  assert(mjpegParts[0].geometry.pixelWidth === first.pixelWidth && mjpegParts[0].geometry.pixelHeight === first.pixelHeight, 'part headers disagreed with control geometry');
  await expectRejected('navigate', { url: 'ftp://google.de' }, 'http:// and https://');
  const rejectionPreserved = await request('inspect');
  const rejectionSnapshot = await request('snapshot');
  assert(rejectionPreserved.url === first.url && rejectionSnapshot.streamUrl === first.streamUrl, 'helper or MJPEG control state did not survive navigation rejection');
  await expectRejectedHttp(first.streamUrl);
  const recoveredCount = mjpegParts.length;
  mjpegRequest.destroy(); mjpegResponse.destroy();
  await new Promise((resolve) => setTimeout(resolve, 20));
  ({ request: mjpegRequest, response: mjpegResponse } = await openMjpeg(first.streamUrl));
  await waitForPartAfter(recoveredCount);
  stalledRequest = await openStalledMjpeg(first.streamUrl);
  const reloaded = await request('reload');
  assert(reloaded.url === first.url, 'reload left the local fixture URL');
  // Reload may be fully deduplicated. A visual click must still reach the
  // active stream while a paused peer is attached. Linux loopback can accept a
  // finite burst without producing socket backpressure, so this smoke cannot
  // deterministically force the helper's 2s stalled-write timer.
  const beforePressure = mjpegParts.length;
  await click(820, 255);
  await waitForPartAfter(beforePressure);
  stalledRequest.request.destroy();
  await waitUntil(() => stalledRequest.closed, 'explicit paused MJPEG client close', 4_000);
  await request('reload'); // reset the fixture before the ordinary interaction path
  let focused = false;
  for (const [x, y] of [[300, 255], [300, 280], [300, 230], [300, 305]]) { const ack = await click(x, y); const inspected = await request('inspect'); if (inspected.activeElement === 'INPUT#name') { assert(ack.cursor === 'text', `input cursor was ${ack.cursor}`); focused = true; break; } }
  assert(focused, 'mouse input did not focus the fixture input');
  const beforeTypingPart = mjpegParts.length;
  for (const [key, code] of [['A', 'KeyA'], ['d', 'KeyD'], ['a', 'KeyA']]) { const down = await request('input', { event: { kind: 'keyDown', key, code, text: key } }); assert(typeof down.cursor === 'string', 'keyDown did not return a cursor acknowledgement'); await request('input', { event: { kind: 'keyUp', key, code } }); }
  const typedPart = await waitForPartAfter(beforeTypingPart);
  assert(typedPart.receivedAt - mjpegParts[beforeTypingPart - 1].receivedAt >= 20, 'screencast acknowledgement pacing was not bounded');
  let greeting;
  for (const [x, y] of [[820, 230], [820, 255], [820, 280], [820, 305], [800, 255], [840, 255]]) { const ack = await click(x, y); const inspected = await request('inspect'); if (inspected.fixtureStatus?.includes('Hello, Ada')) { assert(ack.cursor === 'pointer', `button cursor was ${ack.cursor}`); greeting = inspected; break; } }
  assert(greeting, 'pointer click did not activate the fixture button');
  const beforeScroll = await request('inspect'); await request('input', { event: { kind: 'mouseMove', x: 500, y: 600, buttons: 0, modifiers: 0 } }); const wheelAck = await request('input', { event: { kind: 'wheel', x: 500, y: 600, deltaX: 0, deltaY: 500, modifiers: 0 } }); assert(typeof wheelAck.cursor === 'string', 'wheel acknowledgement was not bounded control state'); const afterScroll = await request('inspect'); assert(afterScroll.scrollY > beforeScroll.scrollY, 'wheel input did not change scrollY');
  await request('input', { event: { kind: 'mouseDown', x: 210, y: 120, button: 'left', buttons: 1 } }); await request('input', { event: { kind: 'mouseMove', x: 620, y: 120, button: 'left', buttons: 1 } }); await request('input', { event: { kind: 'mouseUp', x: 620, y: 120, button: 'left', buttons: 0 } }); const selected = await request('inspect'); assert(selected.selectedText?.length > 0, 'drag input did not select fixture text');
  await waitForPartAfter(beforeTypingPart + 1);
  console.log('interactive-browser-panel MJPEG smoke passed: multipart JPEG parts with atomically correlated geometry headers, reconnect, paused-peer active progress, pointer, typing, button, wheel, selection, reload, inspect. Caveats: CDP may not emit an identical encoded JPEG callback, and finite Linux loopback writes cannot deterministically force the helper stalled-write timer; both implementations remain bounded but those paths are not faked by this smoke.');
}

try { await run(); await request('stop'); stopped = true; } finally {
  if (!stopped) { try { await request('stop'); } catch {} }
  stalledRequest?.request?.destroy(); stalledRequest?.response?.destroy();
  mjpegRequest?.destroy(); mjpegResponse?.destroy();
  child.stdin.end(); await waitForExit(); lines.close();
}
