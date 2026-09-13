#!/usr/bin/env node

import { createHash, randomBytes } from 'node:crypto';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import process from 'node:process';
import readline from 'node:readline';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const FIXTURE_PATH = path.join(ROOT, 'fixture', 'index.html');
const CAPTURE_BOOTSTRAP_PATH = path.join(ROOT, 'helper', 'capture-bootstrap.js');
const DEFAULT_VIEWPORT = { width: 1024, height: 720 };
const MAX_LINE = 8 * 1024;
const MAX_PACKET_BYTES = 2 * 1024 * 1024;
const HEADER_BYTES = 48;
const PACKET_MAGIC = 0x49505743; // IPWC
const CLIENT_ACK_TIMEOUT_MS = 2_000;
const MAX_CONTROL_BYTES = 1_024;
const RESOURCE_TEST = process.env.POC_RESOURCE_TEST === '1';

let playwright; let fixtureServer; let streamServer; let streamUrl; let ingressUrl; let frameToken; let ingressToken;
let browser; let context; let page; let cdp; let fixtureUrl; let shuttingDown = false;
let producer; let latestPacket; let latestKeyframe; let packetGeneration = 0; let keyframeRequested = false; let latestViewport = { ...DEFAULT_VIEWPORT, scale: 1, offsetX: 0, offsetY: 0 };
let lastPointer = { x: 0, y: 0 }; let mouseButtons = 0;
const egressClients = new Map(); const sockets = new Set();

function protocolError(message) { const error = new Error(message); error.protocol = true; return error; }
function assertObject(value, name) { if (!value || typeof value !== 'object' || Array.isArray(value)) throw protocolError(`${name} must be an object`); return value; }
function assertString(value, name, max = 256) { if (typeof value !== 'string' || !value || value.length > max) throw protocolError(`${name} must be a non-empty string of at most ${max} characters`); return value; }
function assertNumber(value, name, min, max) { if (!Number.isFinite(value) || value < min || value > max) throw protocolError(`${name} must be a finite number between ${min} and ${max}`); return value; }
function sleep(ms) { return new Promise((resolve) => setTimeout(resolve, ms)); }
function boundedDimension(value, fallback) { return Number.isFinite(value) && value > 0 && value <= 16_384 ? Math.round(value) : fallback; }

function serverFrame(opcode, payload = Buffer.alloc(0)) {
  const length = payload.byteLength; const head = length <= 125 ? 2 : length <= 0xffff ? 4 : 10; const out = Buffer.allocUnsafe(head + length);
  out[0] = 0x80 | opcode;
  if (head === 2) out[1] = length; else if (head === 4) { out[1] = 126; out.writeUInt16BE(length, 2); } else { out[1] = 127; out.writeBigUInt64BE(BigInt(length), 2); }
  payload.copy(out, head); return out;
}
function rejectUpgrade(socket, status = 400) { if (!socket.destroyed) socket.end(`HTTP/1.1 ${status} Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n`, () => socket.destroy()); }
function removeEgress(client) { clearTimeout(client.ackTimer); client.pending = undefined; client.input = Buffer.alloc(0); egressClients.delete(client.socket); }
function closeEgress(client, status = 1008) { if (!egressClients.has(client.socket)) return; removeEgress(client); if (!client.socket.destroyed) { if (status === 1000) client.socket.end(serverFrame(8), () => client.socket.destroy()); else client.socket.destroy(); } }
function sendPacket(client, packet) {
  client.awaiting = packet.sequence; client.ackTimer = setTimeout(() => closeEgress(client, 1013), CLIENT_ACK_TIMEOUT_MS);
  try { client.socket.write(serverFrame(2, packet.bytes)); } catch { closeEgress(client, 1011); }
}
function queueLatest(client, packet) {
  if (!egressClients.has(client.socket)) return;
  if (client.awaiting === undefined) { sendPacket(client, packet); return; }
  // A VP8 delta may depend on a packet replaced by latest-only delivery.
  // Drop it and request a real source frame marked key instead of decoding an
  // arbitrary broken chain or accumulating an ordered queue.
  if (packet.keyframe) client.pending = packet; else { client.pending = undefined; requestKeyframe(); }
}
function ackEgress(client, text) {
  if (!/^ack:\d{1,12}$/.test(text) || client.awaiting === undefined || Number(text.slice(4)) !== client.awaiting) return closeEgress(client);
  clearTimeout(client.ackTimer); client.awaiting = undefined; const pending = client.pending; client.pending = undefined; if (pending) sendPacket(client, pending);
}
function consumeMaskedFrames(state, chunk, onFrame, onClose) {
  state.input = Buffer.concat([state.input, chunk]);
  if (state.input.byteLength > MAX_PACKET_BYTES + 14) throw protocolError('WebSocket input exceeded bound');
  while (state.input.byteLength >= 2) {
    const first = state.input[0]; const second = state.input[1]; const opcode = first & 0x0f; const masked = Boolean(second & 0x80); let length = second & 0x7f; let head = 2;
    if ((first & 0x70) || !(first & 0x80) || !masked) throw protocolError('WebSocket frame must be a final masked unfragmented frame');
    if (length === 126) { if (state.input.byteLength < 4) return; length = state.input.readUInt16BE(2); head = 4; } else if (length === 127) { if (state.input.byteLength < 10) return; const raw = state.input.readBigUInt64BE(2); if (raw > BigInt(MAX_PACKET_BYTES)) throw protocolError('WebSocket payload exceeded bound'); length = Number(raw); head = 10; }
    if (length > MAX_PACKET_BYTES || (opcode >= 8 && length > 125)) throw protocolError('WebSocket payload exceeded bound');
    if (state.input.byteLength < head + 4 + length) return;
    const mask = head; const start = head + 4; const payload = Buffer.from(state.input.subarray(start, start + length)); for (let i = 0; i < payload.byteLength; i += 1) payload[i] ^= state.input[mask + (i % 4)]; state.input = state.input.subarray(start + length);
    if (opcode === 8) { onClose?.(); return; } if (opcode === 9) { state.socket.write(serverFrame(10, payload)); continue; } if (opcode === 10) continue; onFrame(opcode, payload);
  }
}
function parsePacket(bytes, previousSequence) {
  if (bytes.byteLength < HEADER_BYTES || bytes.byteLength > MAX_PACKET_BYTES) throw protocolError('MediaStream packet exceeded bounds');
  if (bytes.readUInt32BE(0) !== PACKET_MAGIC || bytes.readUInt8(4) !== 1 || bytes.readUInt16BE(6) !== HEADER_BYTES) throw protocolError('invalid MediaStream packet header');
  const sequence = bytes.readUInt32BE(8); const pixelWidth = bytes.readUInt32BE(12); const pixelHeight = bytes.readUInt32BE(16); const codec = bytes.readUInt32BE(20); const payloadLength = bytes.readUInt32BE(36);
  if (!sequence || sequence <= previousSequence || !pixelWidth || !pixelHeight || pixelWidth > 16_384 || pixelHeight > 16_384 || codec !== 1 || !payloadLength || bytes.byteLength !== HEADER_BYTES + payloadLength) throw protocolError('invalid VP8 MediaStream packet fields');
  return { sequence, pixelWidth, pixelHeight, keyframe: Boolean(bytes.readUInt8(5) & 1), bytes };
}
function requestKeyframe() {
  if (keyframeRequested || !producer || producer.socket.destroyed) return;
  keyframeRequested = true;
  try { producer.socket.write(serverFrame(1, Buffer.from('keyframe'))); } catch { keyframeRequested = false; }
}
function publishPacket(packet) { for (const client of egressClients.values()) queueLatest(client, packet); }
function acceptProducerPacket(state, bytes) {
  const parsed = parsePacket(bytes, state.sequence || 0); state.sequence = parsed.sequence;
  const packet = { ...parsed, bytes: Buffer.from(bytes), generation: ++packetGeneration };
  latestPacket = packet; if (packet.keyframe) { latestKeyframe = packet; keyframeRequested = false; } publishPacket(packet);
}

async function startServers() {
  if (streamServer) return;
  frameToken = randomBytes(24).toString('base64url'); ingressToken = randomBytes(24).toString('base64url');
  streamServer = createServer((_, response) => { response.writeHead(426, { 'content-type': 'text/plain; charset=utf-8' }); response.end('WebSocket upgrade required'); });
  streamServer.on('connection', (socket) => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); socket.on('error', () => sockets.delete(socket)); });
  streamServer.on('upgrade', (request, socket, head) => {
    let url; try { url = new URL(request.url || '/', 'http://127.0.0.1'); } catch { rejectUpgrade(socket); return; }
    const key = request.headers['sec-websocket-key']; const connection = String(request.headers.connection || ''); const validKey = typeof key === 'string' && /^[A-Za-z0-9+/]{22}==$/.test(key) && Buffer.from(key, 'base64').byteLength === 16;
    const egress = url.pathname === '/frames' && url.searchParams.get('token') === frameToken; const ingress = url.pathname === '/ingress' && url.searchParams.get('token') === ingressToken;
    if (request.method !== 'GET' || head.byteLength || !validKey || request.headers['sec-websocket-version'] !== '13' || request.headers.upgrade?.toLowerCase() !== 'websocket' || !/(?:^|,)\s*upgrade\s*(?:,|$)/i.test(connection) || (!egress && !ingress)) { rejectUpgrade(socket); return; }
    const accept = createHash('sha1').update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64'); socket.setNoDelay(true); socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    if (egress) {
      const client = { socket, input: Buffer.alloc(0), awaiting: undefined, pending: undefined, ackTimer: undefined }; egressClients.set(socket, client);
      socket.on('data', (chunk) => { try { consumeMaskedFrames(client, chunk, (opcode, payload) => { if (opcode !== 1 || payload.byteLength > MAX_CONTROL_BYTES) throw protocolError('egress accepts only bounded text acknowledgements'); ackEgress(client, payload.toString('utf8')); }, () => closeEgress(client, 1000)); } catch { closeEgress(client); } }); socket.on('close', () => removeEgress(client)); socket.on('error', () => removeEgress(client));
      if (latestKeyframe) queueLatest(client, latestKeyframe); requestKeyframe(); return;
    }
    if (producer) { try { producer.socket.destroy(); } catch {} }
    const state = { socket, input: Buffer.alloc(0), sequence: 0 };
    producer = state; keyframeRequested = false;
    socket.on('data', (chunk) => { try { consumeMaskedFrames(state, chunk, (opcode, payload) => { if (opcode === 1) { if (payload.toString('utf8') !== 'ready') throw protocolError('unknown producer control message'); return; } if (opcode !== 2) throw protocolError('producer accepts only binary VP8 packets'); acceptProducerPacket(state, payload); }, () => socket.destroy()); } catch { socket.destroy(); } });
    socket.on('close', () => { if (producer === state) producer = undefined; });
    socket.on('error', () => { if (producer === state) producer = undefined; });
    requestKeyframe();
  });
  await new Promise((resolve, reject) => { streamServer.once('error', reject); streamServer.listen(0, '127.0.0.1', resolve); }); const address = streamServer.address(); if (!address || typeof address === 'string') throw new Error('unable to determine WebSocket port');
  streamUrl = `ws://127.0.0.1:${address.port}/frames?token=${frameToken}`; ingressUrl = `ws://127.0.0.1:${address.port}/ingress?token=${ingressToken}`;
}
async function startFixture() {
  fixtureServer = createServer(async (request, response) => {
    const url = new URL(request.url || '/', 'http://127.0.0.1');
    if (request.method !== 'GET' || !['/', '/index.html'].includes(url.pathname)) { response.writeHead(404); response.end('Not found'); return; }
    try { const html = await readFile(FIXTURE_PATH); response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store', 'content-length': html.byteLength }); response.end(html); } catch { response.writeHead(500); response.end('Fixture unavailable'); }
  });
  await new Promise((resolve, reject) => { fixtureServer.once('error', reject); fixtureServer.listen(0, '127.0.0.1', resolve); });
  const address = fixtureServer.address();
  if (!address || typeof address === 'string') throw new Error('unable to determine fixture port');
  fixtureUrl = `http://127.0.0.1:${address.port}/${RESOURCE_TEST ? '?resource-test=1' : ''}`;
}
async function waitForPacketAfter(generation, timeout = 10_000) { const deadline = performance.now() + timeout; while (packetGeneration <= generation) { if (performance.now() >= deadline) throw new Error('timed out waiting for MediaStream/WebCodecs VP8 packet'); await sleep(25); } return latestPacket; }
async function waitForKeyframeAfter(generation, timeout = 10_000) { const deadline = performance.now() + timeout; while (!latestPacket?.keyframe || latestPacket.generation <= generation) { requestKeyframe(); if (performance.now() >= deadline) throw new Error('timed out waiting for requested VP8 keyframe'); await sleep(25); } return latestPacket; }
async function capturePage() {
  const before = packetGeneration;
  const control = page.locator('#poc-media-stream-capture-host').locator('#poc-media-stream-capture');
  try {
    await control.waitFor({ state: 'visible', timeout: 10_000 });
  } catch (error) {
    const installError = await page.evaluate(() => window.__pocMediaStreamCaptureInstallError || '').catch(() => '');
    throw new Error(`capture control was not installed after navigation${installError ? `: ${installError}` : ` (${error.message})`}`);
  }
  await control.click({ timeout: 10_000 });
  await waitForKeyframeAfter(before);
}
async function startBrowser() {
  try { playwright ??= await import('playwright'); } catch (error) { throw new Error(`Playwright is unavailable; run "bun install --frozen-lockfile" then "bunx playwright install chromium" (${error.message})`); }
  const executablePath = process.env.BROWSER_BINARY || undefined;
  try {
    // Playwright's headless:true passes legacy --headless. This explicit new
    // mode remains windowless while permitting the verified display-capture path.
    browser = await playwright.chromium.launch({ headless: false, ...(executablePath ? { executablePath } : {}), args: ['--headless=new', '--use-fake-ui-for-media-stream', '--allow-http-screen-capture', '--disable-features=LocalNetworkAccessChecks,LocalNetworkAccessChecksWebSockets', '--no-first-run', '--no-default-browser-check', '--disable-background-networking'] });
    context = await browser.newContext({ viewport: DEFAULT_VIEWPORT, deviceScaleFactor: 1 });
    await context.addInitScript((captureConfig) => { Object.defineProperty(window, '__pocMediaStreamCapture', { value: captureConfig, configurable: false }); }, { ingressUrl });
    await context.addInitScript({ path: CAPTURE_BOOTSTRAP_PATH });
    page = await context.newPage(); cdp = await context.newCDPSession(page); await cdp.send('Page.enable'); await cdp.send('Runtime.enable');
    await page.goto(fixtureUrl, { waitUntil: 'domcontentloaded' }); await capturePage();
  } catch (error) { throw new Error(`could not launch bundled Playwright Chromium with --headless=new MediaStream capture${executablePath ? ` (${executablePath})` : ''}: ${error.message}`); }
}
async function currentViewport() { try { const metrics = await cdp.send('Page.getLayoutMetrics'); const viewport = metrics.cssVisualViewport || metrics.visualViewport || {}; latestViewport = { width: boundedDimension(viewport.clientWidth, DEFAULT_VIEWPORT.width), height: boundedDimension(viewport.clientHeight, DEFAULT_VIEWPORT.height), scale: Number.isFinite(viewport.scale) && viewport.scale > 0 ? viewport.scale : 1, offsetX: Number.isFinite(viewport.pageX) ? viewport.pageX : 0, offsetY: Number.isFinite(viewport.pageY) ? viewport.pageY : 0 }; } catch {} return latestViewport; }
function ensureReady() { if (!page || !cdp || !streamUrl || !latestPacket) throw new Error('browser capture is not started'); }
async function evaluatePageState(x = lastPointer.x, y = lastPointer.y) { const viewport = latestViewport; const result = await cdp.send('Runtime.evaluate', { expression: `(() => { const element = document.elementFromPoint(${Math.max(0, Math.min(viewport.width, x))}, ${Math.max(0, Math.min(viewport.height, y))}); const raw = getComputedStyle(element || document.body).cursor; const editable = !!element && element.matches('input, textarea, [contenteditable="true"]'); return { title: document.title.slice(0,200), url: location.href.slice(0,2048), activeElement: (() => { const a = document.activeElement; return a ? a.tagName + (a.id ? '#' + a.id : '') : 'BODY'; })().slice(0,128), fixtureStatus: (document.querySelector('#result')?.textContent || '').slice(0,240), selectedText: (getSelection()?.toString() || '').slice(0,240), scrollY: Math.max(0, Math.round(scrollY)), cursor: raw === 'auto' && editable ? 'text' : raw }; })()`, returnByValue: true }); return result.result?.value || {}; }
async function snapshot() { ensureReady(); const [viewport, state] = await Promise.all([currentViewport(), evaluatePageState()]); return { streamUrl, sequence: latestPacket.generation, pixelWidth: latestPacket.pixelWidth, pixelHeight: latestPacket.pixelHeight, viewport, title: String(state.title || ''), url: String(state.url || fixtureUrl), cursor: String(state.cursor || 'default') }; }
async function inspect() { ensureReady(); const state = await evaluatePageState(); return { status: 'ready', fixtureStatus: String(state.fixtureStatus || ''), title: String(state.title || ''), url: String(state.url || fixtureUrl), activeElement: String(state.activeElement || 'BODY'), selectedText: String(state.selectedText || ''), scrollY: Math.max(0, Number(state.scrollY) || 0), cursor: String(state.cursor || 'default') }; }
function validateEvent(event) { assertObject(event, 'event'); const kind = assertString(event.kind, 'event.kind', 32); if (event.modifiers !== undefined && (!Number.isInteger(event.modifiers) || event.modifiers < 0 || event.modifiers > 15)) throw protocolError('event.modifiers must be an integer bitmask from 0 to 15'); if (['mouseMove','mouseDown','mouseUp'].includes(kind)) { assertNumber(event.x, 'event.x', 0, latestViewport.width); assertNumber(event.y, 'event.y', 0, latestViewport.height); if (kind !== 'mouseMove' && !['left','middle','right'].includes(event.button ?? 'left')) throw protocolError('event.button must be left, middle, or right'); return kind; } if (kind === 'wheel') { assertNumber(event.x, 'event.x', 0, latestViewport.width); assertNumber(event.y, 'event.y', 0, latestViewport.height); assertNumber(event.deltaX ?? 0, 'event.deltaX', -10000, 10000); assertNumber(event.deltaY ?? 0, 'event.deltaY', -10000, 10000); return kind; } if (kind === 'keyDown' || kind === 'keyUp') { assertString(event.key, 'event.key', 128); assertString(event.code, 'event.code', 128); if (event.text !== undefined && (typeof event.text !== 'string' || event.text.length > 16)) throw protocolError('event.text must be a string of at most 16 characters'); return kind; } throw protocolError(`unsupported input event kind: ${kind}`); }
function buttonMask(button) { return button === 'left' ? 1 : button === 'right' ? 2 : 4; }
async function dispatchInput(event) {
  ensureReady(); await page.bringToFront(); const kind = validateEvent(event);
  if (['mouseMove','mouseDown','mouseUp'].includes(kind)) {
    const button = kind === 'mouseMove' ? 0 : buttonMask(event.button || 'left');
    if (kind === 'mouseDown') mouseButtons |= button;
    const buttons = event.buttons ?? (kind === 'mouseUp' ? mouseButtons & ~button : mouseButtons);
    const cdpButton = kind === 'mouseMove' ? (buttons & 1 ? 'left' : buttons & 2 ? 'right' : buttons & 4 ? 'middle' : 'none') : event.button || 'left';
    lastPointer = { x: event.x, y: event.y };
    await cdp.send('Input.dispatchMouseEvent', { type: { mouseMove:'mouseMoved', mouseDown:'mousePressed', mouseUp:'mouseReleased' }[kind], x:event.x, y:event.y, button: cdpButton, buttons, clickCount: kind === 'mouseMove' ? 0 : 1, modifiers:event.modifiers || 0 });
    if (kind === 'mouseUp') mouseButtons &= ~button;
  } else if (kind === 'wheel') {
    lastPointer = { x:event.x, y:event.y };
    await cdp.send('Input.dispatchMouseEvent', { type:'mouseWheel', x:event.x, y:event.y, deltaX:event.deltaX || 0, deltaY:event.deltaY || 0, modifiers:event.modifiers || 0 });
    await cdp.send('Runtime.evaluate', { expression: 'new Promise((resolve) => requestAnimationFrame(() => resolve()))', awaitPromise: true, returnByValue: true });
  } else {
    await cdp.send('Input.dispatchKeyEvent', { type: kind === 'keyDown' ? 'keyDown' : 'keyUp', key:event.key, code:event.code, modifiers:event.modifiers || 0, ...(kind === 'keyDown' && event.text ? { text:event.text, unmodifiedText:event.text } : {}) });
  }
  const state = await evaluatePageState(); return { cursor: String(state.cursor || 'default') };
}
function normalizeUrl(value) { let candidate = assertString(value, 'url', 2048).trim(); if (!/^[a-z][a-z\d+.-]*:\/\//i.test(candidate)) candidate = `https://${candidate}`; let parsed; try { parsed = new URL(candidate); } catch { throw protocolError('url must be an absolute http:// or https:// URL'); } if (!['http:','https:'].includes(parsed.protocol) || !parsed.hostname || parsed.username || parsed.password) throw protocolError('navigation only supports credential-free http:// and https:// URLs'); return parsed.href; }
async function reload() { ensureReady(); await page.goto(fixtureUrl, { waitUntil:'domcontentloaded' }); await capturePage(); return snapshot(); }
async function navigate(url) { ensureReady(); await page.goto(normalizeUrl(url), { waitUntil:'domcontentloaded' }); await capturePage(); return snapshot(); }
async function closeServer(server) { if (!server) return; server.closeAllConnections?.(); await new Promise((resolve) => server.close(resolve)).catch(() => {}); }
async function shutdown() { if (shuttingDown) return; shuttingDown = true; for (const client of [...egressClients.values()]) closeEgress(client); for (const socket of sockets) socket.destroy(); sockets.clear(); producer?.socket.destroy(); producer = undefined; latestPacket = undefined; latestKeyframe = undefined; packetGeneration = 0; keyframeRequested = false; await closeServer(streamServer); streamServer = undefined; streamUrl = undefined; ingressUrl = undefined; try { await browser?.close(); } catch {} browser = undefined; context = undefined; page = undefined; cdp = undefined; await closeServer(fixtureServer); fixtureServer = undefined; }
async function handle(request) { assertObject(request,'request'); const method = assertString(request.method,'method',64); if (method === 'start') { if (!browser) { shuttingDown = false; await startServers(); await startFixture(); try { await startBrowser(); } catch (error) { await shutdown(); throw error; } } return snapshot(); } if (method === 'snapshot') return snapshot(); if (method === 'inspect') return inspect(); if (method === 'input') return dispatchInput(request.event); if (method === 'reload') return reload(); if (method === 'navigate') return navigate(request.url); if (method === 'validateUrl') return { url: normalizeUrl(request.url) }; if (method === 'stop') { await shutdown(); return { status:'stopped' }; } throw protocolError(`unknown method: ${method}`); }
function writeResponse(response) { const line = JSON.stringify(response); process.stdout.write(`${line.length <= MAX_LINE ? line : JSON.stringify({ok:false,error:'response exceeded control protocol limit'})}\n`); }
const input = readline.createInterface({ input:process.stdin, crlfDelay:Infinity }); process.on('SIGTERM', () => { void shutdown().finally(() => process.exit(0)); }); process.on('SIGINT', () => { void shutdown().finally(() => process.exit(0)); });
try { for await (const line of input) { if (line.length > MAX_LINE) { writeResponse({ok:false,error:'request exceeded control protocol limit'}); continue; } if (!line.trim()) continue; let request; try { request = JSON.parse(line); } catch { writeResponse({ok:false,error:'request is not valid JSON'}); continue; } try { const result = await handle(request); writeResponse({ok:true,id:request.id ?? null,result}); if (request.method === 'stop') break; } catch (error) { writeResponse({ok:false,id:request.id ?? null,error:String(error?.message || error)}); } } } finally { await shutdown(); }
