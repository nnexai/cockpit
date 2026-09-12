#!/usr/bin/env node

import { spawn } from 'node:child_process';
import net from 'node:net';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import readline from 'node:readline';

const helperPath = path.join(path.dirname(fileURLToPath(import.meta.url)), 'browser-helper.mjs');
const child = spawn(process.env.NODE_BINARY || process.execPath, [helperPath], { stdio: ['pipe', 'pipe', 'inherit'] });
const lines = readline.createInterface({ input: child.stdout, crlfDelay: Infinity });
const pending = new Map();
let nextId = 1;
let stopped = false;
let frameSocket;
let frameBuffer = Buffer.alloc(0);
let latestFrame;
const frameArrivalGaps = [];
let receivedPongs = 0;
let stalledSocket;
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

function parseBinaryFrame(payload) {
  if (payload.byteLength < 48) return null;
  const view = new DataView(payload.buffer, payload.byteOffset, payload.byteLength);
  if (view.getUint32(0) !== 0x49504246 || view.getUint8(4) !== 1 || view.getUint16(6) !== 48) return null;
  const jpegLength = view.getUint32(40);
  if (!jpegLength || payload.byteLength !== 48 + jpegLength) return null;
  return { sequence: view.getUint32(8), pixelWidth: view.getUint32(12), pixelHeight: view.getUint32(16), viewport: { width: view.getUint32(20), height: view.getUint32(24), scale: view.getFloat32(28), offsetX: view.getFloat32(32), offsetY: view.getFloat32(36) }, jpegLength, receivedAt: performance.now() };
}

function consumeFrameSocketData(chunk) {
  frameBuffer = Buffer.concat([frameBuffer, chunk]);
  while (frameBuffer.byteLength >= 2) {
    const first = frameBuffer[0]; const second = frameBuffer[1];
    let headerBytes = 2; let length = second & 0x7f;
    if (length === 126) { if (frameBuffer.byteLength < 4) return; length = frameBuffer.readUInt16BE(2); headerBytes = 4; }
    else if (length === 127) { if (frameBuffer.byteLength < 10) return; const rawLength = frameBuffer.readBigUInt64BE(2); if (rawLength > BigInt(6 * 1024 * 1024 + 48)) throw new Error('binary websocket frame was too large'); length = Number(rawLength); headerBytes = 10; }
    if (second & 0x80) throw new Error('server websocket frame unexpectedly had a mask');
    if (frameBuffer.byteLength < headerBytes + length) return;
    const payload = frameBuffer.subarray(headerBytes, headerBytes + length); frameBuffer = frameBuffer.subarray(headerBytes + length);
    const opcode = first & 0x0f;
    if (opcode === 2) {
      const frame = parseBinaryFrame(payload);
      if (frame) {
        if (latestFrame) frameArrivalGaps.push(frame.receivedAt - latestFrame.receivedAt);
        latestFrame = frame;
        frameSocket?.write(maskedControlFrame(0x1, Buffer.from(`ack:${frame.sequence}`)));
      }
    } else if (opcode === 0xA) receivedPongs += 1;
  }
}

async function connectFrameStream(streamUrl) {
  const url = new URL(streamUrl);
  assert(url.protocol === 'ws:' && url.hostname === '127.0.0.1', 'stream URL was not loopback WebSocket');
  frameSocket = net.createConnection({ host: url.hostname, port: Number(url.port) });
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out opening binary frame stream')), 8_000);
    frameSocket.once('error', reject);
    frameSocket.once('connect', () => {
      frameSocket.write(`GET ${url.pathname}${url.search} HTTP/1.1\r\nHost: ${url.host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n`);
    });
    let handshake = Buffer.alloc(0);
    const receiveHandshake = (chunk) => {
      handshake = Buffer.concat([handshake, chunk]);
      const end = handshake.indexOf('\r\n\r\n');
      if (end < 0) return;
      frameSocket.off('data', receiveHandshake); clearTimeout(timeout);
      const response = handshake.subarray(0, end).toString('ascii');
      if (!response.startsWith('HTTP/1.1 101')) { reject(new Error(`frame stream upgrade failed: ${response.split('\r\n')[0]}`)); return; }
      frameSocket.on('data', consumeFrameSocketData);
      const remainder = handshake.subarray(end + 4); if (remainder.byteLength) consumeFrameSocketData(remainder);
      resolve();
    };
    frameSocket.on('data', receiveHandshake);
  });
}

function maskedControlFrame(opcode, payload = Buffer.alloc(0)) {
  const mask = Buffer.from([0x1d, 0x2c, 0x3b, 0x4a]);
  const masked = Buffer.from(payload);
  for (let index = 0; index < masked.byteLength; index += 1) masked[index] ^= mask[index % 4];
  return Buffer.concat([Buffer.from([0x80 | opcode, 0x80 | masked.byteLength]), mask, masked]);
}

async function waitUntil(predicate, description, timeoutMs = 4_000) {
  const deadline = performance.now() + timeoutMs;
  while (!predicate()) {
    if (performance.now() >= deadline) throw new Error(`timed out waiting for ${description}`);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

async function expectMalformedUpgrade(streamUrl) {
  const url = new URL(streamUrl);
  const socket = net.createConnection({ host: url.hostname, port: Number(url.port) });
  const response = await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('malformed upgrade was not rejected')), 4_000);
    socket.once('error', reject);
    socket.once('connect', () => socket.write(`GET ${url.pathname}${url.search} HTTP/1.1\r\nHost: ${url.host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: malformed\r\nSec-WebSocket-Version: 12\r\n\r\n`));
    socket.once('data', (chunk) => { clearTimeout(timeout); resolve(chunk.toString('ascii')); });
  });
  socket.destroy();
  assert(String(response).startsWith('HTTP/1.1 400'), 'malformed WebSocket upgrade was accepted');
}

async function openStalledFrameStream(streamUrl) {
  const url = new URL(streamUrl);
  const socket = net.createConnection({ host: url.hostname, port: Number(url.port) });
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out opening stalled frame stream')), 4_000);
    socket.once('error', reject);
    socket.once('connect', () => socket.write(`GET ${url.pathname}${url.search} HTTP/1.1\r\nHost: ${url.host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: c3RhbGxlZC1jbGllbnQtMQ==\r\nSec-WebSocket-Version: 13\r\n\r\n`));
    socket.once('data', (chunk) => {
      clearTimeout(timeout);
      if (!chunk.toString('ascii', 0, Math.min(chunk.byteLength, 32)).startsWith('HTTP/1.1 101')) reject(new Error('stalled frame stream was not accepted'));
      else resolve();
    });
  });
  socket.pause();
  return socket;
}

async function waitForFrameAfter(sequence, timeoutMs = 8_000) {
  const deadline = performance.now() + timeoutMs;
  while (!latestFrame || latestFrame.sequence <= sequence) {
    if (performance.now() >= deadline) throw new Error(`timed out waiting for binary frame after ${sequence}`);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  return latestFrame;
}
async function click(x, y) { await request('input', { event: { kind: 'mouseDown', x, y, button: 'left' } }); return request('input', { event: { kind: 'mouseUp', x, y, button: 'left' } }); }
async function expectRejected(method, fields, text) { try { await request(method, fields); } catch (error) { assert(String(error.message).includes(text), `${method} error was not explicit`); return; } throw new Error(`${method} unexpectedly succeeded`); }

async function run() {
  const acceptedHttps = await request('validateUrl', { url: 'https://google.de' }); assert(acceptedHttps.url === 'https://google.de/', 'absolute HTTPS URL was not accepted');
  const acceptedBare = await request('validateUrl', { url: 'google.de' }); assert(acceptedBare.url === 'https://google.de/', 'bare domain was not normalized to HTTPS');
  await expectRejected('validateUrl', { url: 'https://user:pass@google.de' }, 'credentials'); await expectRejected('validateUrl', { url: 'ftp://google.de' }, 'http:// and https://');
  const first = await request('start');
  assert(typeof first.streamUrl === 'string' && !('jpegDataUrl' in first), 'start mixed JPEG bytes into the control response');
  assert(first.pixelWidth > 0 && first.pixelHeight > 0 && first.viewport?.width > 0 && first.viewport?.height > 0, 'start omitted dynamic frame geometry');
  await connectFrameStream(first.streamUrl);
  const initialFrame = await waitForFrameAfter(0);
  assert(initialFrame.pixelWidth === first.pixelWidth && initialFrame.pixelHeight === first.pixelHeight, 'stream frame dimensions disagreed with control metadata');
  assert(initialFrame.viewport.width === first.viewport.width && initialFrame.viewport.height === first.viewport.height, 'stream viewport geometry disagreed with control metadata');
  await expectMalformedUpgrade(first.streamUrl);
  frameSocket.write(maskedControlFrame(0x9, Buffer.from('probe')));
  await waitUntil(() => receivedPongs === 1, 'masked ping response');
  const closingSocket = frameSocket;
  const closed = new Promise((resolve) => closingSocket.once('close', resolve));
  closingSocket.write(maskedControlFrame(0x8, Buffer.alloc(0)));
  await closed;
  frameBuffer = Buffer.alloc(0);
  latestFrame = undefined;
  await connectFrameStream(first.streamUrl);
  const recoveredFrame = await waitForFrameAfter(0);
  assert(recoveredFrame.sequence >= initialFrame.sequence, 'closed frame client did not recover the helper current frame');
  const unmaskedSocket = frameSocket;
  const malformedClosed = new Promise((resolve) => unmaskedSocket.once('close', resolve));
  unmaskedSocket.write(Buffer.from([0x89, 0x00]));
  await malformedClosed;
  frameBuffer = Buffer.alloc(0);
  latestFrame = undefined;
  await connectFrameStream(first.streamUrl);
  const recoveredAfterMalformed = await waitForFrameAfter(0);
  assert(recoveredAfterMalformed.sequence >= recoveredFrame.sequence, 'malformed client frame did not leave the helper recoverable');
  stalledSocket = await openStalledFrameStream(first.streamUrl);
  const reloaded = await request('reload');
  assert(reloaded.url === first.url, 'reload left the local fixture URL');
  // This peer never sends ack:<sequence>; the helper must time it out without
  // blocking the acknowledged primary stream's latest-frame progress.
  await new Promise((resolve) => setTimeout(resolve, 2_100));
  let focused = false;
  for (const [x, y] of [[300, 255], [300, 280], [300, 230], [300, 305]]) { const ack = await click(x, y); const inspected = await request('inspect'); if (inspected.activeElement === 'INPUT#name') { assert(ack.cursor === 'text', `input cursor was ${ack.cursor}`); focused = true; break; } }
  assert(focused, 'mouse input did not focus the fixture input');
  const beforeTypingFrame = latestFrame;
  for (const [key, code] of [['A', 'KeyA'], ['d', 'KeyD'], ['a', 'KeyA']]) { const down = await request('input', { event: { kind: 'keyDown', key, code, text: key } }); assert(typeof down.cursor === 'string', 'keyDown did not return a cursor acknowledgement'); await request('input', { event: { kind: 'keyUp', key, code } }); }
  const typedFrame = await waitForFrameAfter(beforeTypingFrame.sequence);
  assert(frameArrivalGaps.some((gap) => gap >= 20), 'screencast acknowledgement pacing was not bounded');
  let greeting;
  for (const [x, y] of [[820, 230], [820, 255], [820, 280], [820, 305], [800, 255], [840, 255]]) { const ack = await click(x, y); const inspected = await request('inspect'); if (inspected.fixtureStatus?.includes('Hello, Ada')) { assert(ack.cursor === 'pointer', `button cursor was ${ack.cursor}`); greeting = inspected; break; } }
  assert(greeting, 'pointer click did not activate the fixture button');
  const beforeScroll = await request('inspect'); await request('input', { event: { kind: 'mouseMove', x: 500, y: 600, buttons: 0, modifiers: 0 } }); const wheelAck = await request('input', { event: { kind: 'wheel', x: 500, y: 600, deltaX: 0, deltaY: 500, modifiers: 0 } }); assert(typeof wheelAck.cursor === 'string', 'wheel acknowledgement was not bounded control state'); const afterScroll = await request('inspect'); assert(afterScroll.scrollY > beforeScroll.scrollY, 'wheel input did not change scrollY');
  await request('input', { event: { kind: 'mouseDown', x: 210, y: 120, button: 'left', buttons: 1 } }); await request('input', { event: { kind: 'mouseMove', x: 620, y: 120, button: 'left', buttons: 1 } }); await request('input', { event: { kind: 'mouseUp', x: 620, y: 120, button: 'left', buttons: 0 } }); const selected = await request('inspect'); assert(selected.selectedText?.length > 0, 'drag input did not select fixture text');
  const changedFrame = await waitForFrameAfter(typedFrame.sequence);
  assert(changedFrame.jpegLength > 0 && changedFrame.sequence > recoveredAfterMalformed.sequence, 'active client did not receive the current binary frame while a stalled client was connected');
  console.log('interactive-browser-panel transport smoke passed: binary frame stream, malformed/close/ping handling, paced current-frame recovery, pointer, typing, button, wheel, selection, reload, inspect');
}

try { await run(); await request('stop'); stopped = true; } finally { if (!stopped) { try { await request('stop'); } catch {} } stalledSocket?.destroy(); frameSocket?.destroy(); child.stdin.end(); await waitForExit(); lines.close(); }
