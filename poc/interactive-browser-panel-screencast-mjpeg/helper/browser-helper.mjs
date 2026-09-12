#!/usr/bin/env node

import { randomBytes } from 'node:crypto';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import process from 'node:process';
import readline from 'node:readline';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const FIXTURE_PATH = path.join(ROOT, 'fixture', 'index.html');
const DEFAULT_VIEWPORT = { width: 1024, height: 720 };
const TARGET_FPS = 30;
const FRAME_INTERVAL_MS = Math.round(1_000 / TARGET_FPS);
const MAX_LINE = 8 * 1024;
const MAX_FRAME_BYTES = 6 * 1024 * 1024;
const CLIENT_WRITE_TIMEOUT_MS = 2_000;
const MJPEG_BOUNDARY = 'cockpit-mjpeg-frame';

let playwright;
let fixtureServer;
let streamServer;
let streamToken;
let streamUrl;
let browser;
let context;
let page;
let cdp;
let fixtureUrl;
let shuttingDown = false;
let lastPointer = { x: 0, y: 0 };
let mouseButtons = 0;
let latestFrame;
let frameSequence = 0;
let screencastStarted = false;
let frameError;
let lastFrameAcknowledgedAt = 0;
const mjpegClients = new Map();
const streamSockets = new Set();

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

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function removeClient(clients, client) {
  clearTimeout(client.stallTimer);
  client.pending = undefined;
  clients.delete(client.response);
}

function closeClient(clients, client) {
  if (!clients.has(client.response)) return;
  removeClient(clients, client);
  if (!client.response.destroyed) client.response.destroy();
}

function writeLatest(clients, client, payload) {
  if (!clients.has(client.response) || client.response.destroyed) return;
  if (client.blocked) {
    // Keep exactly one replacement payload while Node drains its bounded socket
    // buffer. There is never a frame FIFO per connected HTTP client.
    client.pending = payload;
    return;
  }
  let accepted;
  try {
    accepted = client.response.write(payload);
  } catch {
    closeClient(clients, client);
    return;
  }
  if (!accepted) {
    client.blocked = true;
    client.stallTimer = setTimeout(() => closeClient(clients, client), CLIENT_WRITE_TIMEOUT_MS);
  }
}

function flushClient(clients, client) {
  if (!clients.has(client.response)) return;
  clearTimeout(client.stallTimer);
  client.blocked = false;
  const pending = client.pending;
  client.pending = undefined;
  if (pending) writeLatest(clients, client, pending);
}

function addClient(clients, response) {
  const client = { response, blocked: false, pending: undefined, stallTimer: undefined };
  clients.set(response, client);
  response.on('drain', () => flushClient(clients, client));
  response.on('close', () => removeClient(clients, client));
  response.on('error', () => removeClient(clients, client));
  return client;
}

function validLoopbackRequest(request, requestUrl, pathname) {
  return request.method === 'GET' && requestUrl.pathname === pathname && requestUrl.searchParams.get('token') === streamToken;
}

function mjpegPart(frame) {
  return Buffer.concat([
    Buffer.from(
      `--${MJPEG_BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: ${frame.bytes.byteLength}\r\nX-Frame-Sequence: ${frame.sequence}\r\nX-Pixel-Width: ${frame.pixelWidth}\r\nX-Pixel-Height: ${frame.pixelHeight}\r\nX-Viewport-Width: ${frame.viewport.width}\r\nX-Viewport-Height: ${frame.viewport.height}\r\nX-Viewport-Scale: ${frame.viewport.scale}\r\nX-Viewport-Offset-X: ${frame.viewport.offsetX}\r\nX-Viewport-Offset-Y: ${frame.viewport.offsetY}\r\n\r\n`,
      'ascii'
    ),
    frame.bytes,
    Buffer.from('\r\n', 'ascii'),
  ]);
}
function publicFrame(frame) {
  return {
    sequence: frame.sequence,
    pixelWidth: frame.pixelWidth,
    pixelHeight: frame.pixelHeight,
    viewport: frame.viewport,
  };
}


function publishImage(frame) {
  if (mjpegClients.size === 0) return;
  const part = mjpegPart(frame);
  for (const client of mjpegClients.values()) writeLatest(mjpegClients, client, part);
}

async function startFrameStream() {
  if (streamServer) return;
  streamToken = randomBytes(24).toString('base64url');
  streamServer = createServer((request, response) => {
    let requestUrl;
    try { requestUrl = new URL(request.url || '/', 'http://127.0.0.1'); } catch {
      response.writeHead(400, { connection: 'close', 'content-length': '0' }); response.end(); return;
    }
    if (request.method !== 'GET') {
      response.writeHead(405, { allow: 'GET', 'content-length': '0' }); response.end(); return;
    }
    if (requestUrl.pathname === '/mjpeg' && validLoopbackRequest(request, requestUrl, '/mjpeg')) {
      response.writeHead(200, {
        'access-control-allow-origin': '*',
        'access-control-expose-headers': 'x-mjpeg-boundary',
        'cache-control': 'no-store, no-transform',
        connection: 'keep-alive',
        // WebKitGTK rejects fetch() responses typed as multipart despite the
        // byte framing being valid. Keep MJPEG framing, expose its boundary,
        // and use a fetch-streamable type for the native WebView workaround.
        'content-type': 'application/octet-stream',
        'x-mjpeg-boundary': MJPEG_BOUNDARY,
        'x-content-type-options': 'nosniff',
      });
      const client = addClient(mjpegClients, response);
      if (latestFrame) {
        const initialPart = mjpegPart(latestFrame);
        // WebKitGTK may withhold an indefinite octet-stream response from fetch
        // readers. One bounded concatenated burst primes that threshold without
        // defeating blocked-client coalescing. It adds at most 24 parts and
        // targets 1.2 MB; frontend sequence dedupe renders one part.
        const primeCount = Math.max(1, Math.min(24, Math.floor((1_200 * 1024) / initialPart.length)));
        const primePayload = primeCount === 1 ? initialPart : Buffer.concat(Array(primeCount).fill(initialPart));
        writeLatest(mjpegClients, client, primePayload);
      }
      return;
    }
    response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8', 'content-length': '9' });
    response.end('Not found');
  });
  streamServer.on('connection', (socket) => {
    streamSockets.add(socket);
    socket.on('close', () => streamSockets.delete(socket));
    socket.on('error', () => streamSockets.delete(socket));
  });
  await new Promise((resolve, reject) => {
    streamServer.once('error', reject);
    streamServer.listen(0, '127.0.0.1', resolve);
  });
  const address = streamServer.address();
  if (!address || typeof address === 'string') throw new Error('Unable to determine MJPEG stream port');
  const baseUrl = `http://127.0.0.1:${address.port}`;
  streamUrl = `${baseUrl}/mjpeg?token=${streamToken}`;
}

function jpegDimensions(bytes) {
  if (bytes.length < 4 || bytes[0] !== 0xff || bytes[1] !== 0xd8) return null;
  let offset = 2;
  while (offset + 3 < bytes.length) {
    if (bytes[offset] !== 0xff) { offset += 1; continue; }
    while (offset < bytes.length && bytes[offset] === 0xff) offset += 1;
    if (offset >= bytes.length) break;
    const marker = bytes[offset++];
    if (marker === 0xd9 || marker === 0xda) break;
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) continue;
    if (offset + 1 >= bytes.length) break;
    const segmentLength = bytes.readUInt16BE(offset);
    if (segmentLength < 2 || offset + segmentLength > bytes.length) break;
    if ([0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf].includes(marker) && segmentLength >= 7) {
      return { height: bytes.readUInt16BE(offset + 3), width: bytes.readUInt16BE(offset + 5) };
    }
    offset += segmentLength;
  }
  return null;
}

function boundedDimension(value, fallback) {
  return Number.isFinite(value) && value > 0 && value <= 16_384 ? Math.round(value) : fallback;
}

function boundedOffset(value, fallback = 0) {
  return Number.isFinite(value) && value >= -100_000 && value <= 100_000 ? value : fallback;
}

function currentViewport(metadata = {}) {
  // Screencast metadata and JPEG bytes belong to the same CDP callback. Do not
  // query live layout here: it may have advanced beyond the captured image.
  const scale = Number.isFinite(metadata.pageScaleFactor) && metadata.pageScaleFactor > 0 && metadata.pageScaleFactor <= 100 ? metadata.pageScaleFactor : 1;
  return {
    width: boundedDimension(metadata.deviceWidth / scale, DEFAULT_VIEWPORT.width),
    height: boundedDimension(metadata.deviceHeight / scale, DEFAULT_VIEWPORT.height),
    scale,
    offsetX: boundedOffset(metadata.scrollOffsetX),
    offsetY: boundedOffset(metadata.scrollOffsetY, boundedOffset(metadata.offsetTop)),
  };
}

function samePublishedFrame(left, right) {
  return left.pixelWidth === right.pixelWidth &&
    left.pixelHeight === right.pixelHeight &&
    left.viewport.width === right.viewport.width &&
    left.viewport.height === right.viewport.height &&
    left.viewport.scale === right.viewport.scale &&
    left.viewport.offsetX === right.viewport.offsetX &&
    left.viewport.offsetY === right.viewport.offsetY &&
    left.bytes.equals(right.bytes);
}


async function processScreencastFrame(event) {
  const bytes = Buffer.from(event.data, 'base64');
  if (bytes.byteLength === 0 || bytes.byteLength > MAX_FRAME_BYTES) throw new Error('screencast frame exceeded JPEG transport limit');
  const dimensions = jpegDimensions(bytes);
  if (!dimensions) throw new Error('screencast frame was not a valid JPEG');
  const viewport = currentViewport(event.metadata && typeof event.metadata === 'object' ? event.metadata : {});
  const frame = { sequence: frameSequence + 1, pixelWidth: dimensions.width, pixelHeight: dimensions.height, viewport, bytes };
  // This only suppresses helper-to-webview transport work. Chromium already
  // captured and JPEG-encoded this CDP frame before the helper can compare it.
  // Geometry belongs in the equality check: a changed viewport mapping is a
  // distinct MJPEG part with matching headers, atomically consumed by fetch.
  if (!latestFrame || !samePublishedFrame(frame, latestFrame)) {
    latestFrame = frame;
    publishImage(frame);
  }
  frameSequence = frame.sequence;
}

async function acknowledgeScreencastFrame(sessionId) {
  // CDP ACK pacing bounds Chromium capture/encoding work even when this frame
  // is transport-deduplicated below the helper/webview boundary.
  const wait = Math.max(0, FRAME_INTERVAL_MS - (performance.now() - lastFrameAcknowledgedAt));
  if (wait) await sleep(wait);
  await cdp.send('Page.screencastFrameAck', { sessionId });
  lastFrameAcknowledgedAt = performance.now();
}

async function startScreencast() {
  const baseline = frameSequence;
  cdp.on('Page.screencastFrame', (event) => {
    // Chromium sends another frame only after its sessionId is ACKed. Every ACK
    // is cadence-bound; duplicate JPEGs only skip loopback delivery work.
    void (async () => {
      try { await processScreencastFrame(event); } catch (error) { frameError ??= error; }
      try { await acknowledgeScreencastFrame(event.sessionId); } catch (error) { frameError ??= error; }
    })();
  });
  await cdp.send('Page.startScreencast', { everyNthFrame: 1, format: 'jpeg', maxHeight: 1_600, maxWidth: 2_560, quality: 70 });
  screencastStarted = true;
  await waitForScreencastAfter(baseline);
}

async function startFixture() {
  fixtureServer = createServer(async (request, response) => {
    const requestUrl = new URL(request.url || '/', 'http://127.0.0.1');
    if (request.method !== 'GET' || !['/', '/index.html'].includes(requestUrl.pathname)) {
      response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' }); response.end('Not found'); return;
    }
    try {
      const html = await readFile(FIXTURE_PATH);
      response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store', 'content-length': html.byteLength });
      response.end(html);
    } catch {
      response.writeHead(500, { 'content-type': 'text/plain; charset=utf-8' }); response.end('Fixture unavailable');
    }
  });
  await new Promise((resolve, reject) => { fixtureServer.once('error', reject); fixtureServer.listen(0, '127.0.0.1', resolve); });
  const address = fixtureServer.address();
  if (!address || typeof address === 'string') throw new Error('Unable to determine fixture port');
  fixtureUrl = `http://127.0.0.1:${address.port}/`;
}

async function startBrowser() {
  try { playwright ??= await import('playwright'); } catch (error) {
    throw new Error(`Playwright is unavailable; run "bun install --frozen-lockfile" then "bunx playwright install chromium" in this POC directory, or set BROWSER_BINARY (${error.message})`);
  }
  const executablePath = process.env.BROWSER_BINARY;
  try {
    browser = await playwright.chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}), args: ['--no-first-run', '--no-default-browser-check', '--disable-background-networking'] });
    context = await browser.newContext({ viewport: DEFAULT_VIEWPORT, deviceScaleFactor: 1 });
    page = await context.newPage();
    cdp = await context.newCDPSession(page);
    await cdp.send('Page.enable');
    await cdp.send('Runtime.enable');
    await page.goto(fixtureUrl, { waitUntil: 'domcontentloaded' });
    await startScreencast();
  } catch (error) {
    throw new Error(`Could not launch Chromium through Playwright${executablePath ? ` (${executablePath})` : ''}: ${error.message}. Run "bun install --frozen-lockfile" then "bunx playwright install chromium", or set BROWSER_BINARY.`);
  }
}

async function waitForLatestFrame(timeoutMs = 8_000) {
  const deadline = performance.now() + timeoutMs;
  while (!latestFrame) {
    if (frameError) throw frameError;
    const remaining = deadline - performance.now();
    if (remaining <= 0) throw new Error('timed out waiting for the first published screencast frame');
    await sleep(Math.min(remaining, 25));
  }
  if (frameError) throw frameError;
  return latestFrame;
}

async function waitForScreencastAfter(sequence, timeoutMs = 8_000) {
  const deadline = performance.now() + timeoutMs;
  while (frameSequence <= sequence) {
    if (frameError) throw frameError;
    const remaining = deadline - performance.now();
    if (remaining <= 0) throw new Error(`timed out waiting for screencast callback after ${sequence}`);
    await sleep(Math.min(remaining, 25));
  }
  if (frameError) throw frameError;
}

function ensureReady() {
  if (!page || !cdp || !fixtureUrl || !streamUrl) throw new Error('Browser is not started');
}

function assertNavigationUrl(value) {
  let candidate = assertString(value, 'url', 2048).trim();
  if (!/^[a-z][a-z\d+.-]*:\/\//i.test(candidate)) candidate = `https://${candidate}`;
  let parsed;
  try { parsed = new URL(candidate); } catch { throw protocolError('url must be an absolute http:// or https:// URL (bare domains are normalized to https://)'); }
  if (!['http:', 'https:'].includes(parsed.protocol)) throw protocolError('navigation only supports http:// and https:// URLs');
  if (!parsed.hostname || parsed.username || parsed.password) throw protocolError('navigation URL must not contain credentials');
  return parsed.href;
}

async function evaluatePageState(x = lastPointer.x, y = lastPointer.y) {
  const viewport = latestFrame?.viewport || DEFAULT_VIEWPORT;
  const pointX = Math.max(0, Math.min(viewport.width, Number.isFinite(x) ? x : 0));
  const pointY = Math.max(0, Math.min(viewport.height, Number.isFinite(y) ? y : 0));
  const result = await cdp.send('Runtime.evaluate', { expression: `(() => {
    const element = document.elementFromPoint(${pointX}, ${pointY});
    const computedCursor = getComputedStyle(element || document.body).cursor;
    const editable = !!element && (element.matches('input, textarea, [contenteditable="true"]') || !!element.closest?.('[contenteditable="true"]'));
    const cursor = computedCursor === 'auto' && editable ? 'text' : computedCursor;
    const safeCursors = new Set(['default', 'auto', 'pointer', 'text', 'crosshair', 'move', 'not-allowed', 'wait', 'grab', 'grabbing', 'cell', 'help', 'progress', 'zoom-in', 'zoom-out', 'col-resize', 'row-resize', 'e-resize', 'w-resize', 'n-resize', 's-resize']);
    return { title: document.title.slice(0, 200), url: location.href.slice(0, 2048), activeElement: (() => { const active = document.activeElement; if (!active) return 'BODY'; const id = active.id ? '#' + active.id.slice(0, 80) : ''; return (active.tagName || 'UNKNOWN').slice(0, 40) + id; })(), fixtureStatus: (document.querySelector('#result')?.textContent || '').slice(0, 240), selectedText: (window.getSelection()?.toString() || '').slice(0, 240), scrollY: Math.max(0, Math.round(window.scrollY)), cursor: safeCursors.has(cursor) ? cursor : 'default' };
  })()`, returnByValue: true, awaitPromise: false });
  return result.result?.value || { title: '', url: fixtureUrl, activeElement: 'BODY', fixtureStatus: '', selectedText: '', scrollY: 0, cursor: 'default' };
}

async function snapshot() {
  ensureReady();
  const [frame, state] = await Promise.all([waitForLatestFrame(), evaluatePageState()]);
  return { streamUrl, ...publicFrame(frame), title: String(state.title || '').slice(0, 200), url: String(state.url || fixtureUrl).slice(0, 2048), cursor: String(state.cursor || 'default') };
}

async function inspect() {
  ensureReady();
  const state = await evaluatePageState();
  return { status: 'ready', fixtureStatus: String(state.fixtureStatus || '').slice(0, 240), selectedText: String(state.selectedText || '').slice(0, 240), scrollY: Math.max(0, Math.min(100000, Number(state.scrollY) || 0)), title: String(state.title || '').slice(0, 200), url: String(state.url || fixtureUrl).slice(0, 2048), activeElement: String(state.activeElement || 'BODY').slice(0, 128), cursor: String(state.cursor || 'default') };
}

function validateEvent(event) {
  assertObject(event, 'event');
  const kind = assertString(event.kind, 'event', 32);
  const viewport = latestFrame?.viewport || DEFAULT_VIEWPORT;
  if (event.modifiers !== undefined && (!Number.isInteger(event.modifiers) || event.modifiers < 0 || event.modifiers > 15)) throw protocolError('event.modifiers must be an integer bitmask from 0 to 15');
  if (event.buttons !== undefined && (!Number.isInteger(event.buttons) || event.buttons < 0 || event.buttons > 31)) throw protocolError('event.buttons must be an integer bitmask from 0 to 31');
  if (['mouseMove', 'mouseDown', 'mouseUp'].includes(kind)) {
    assertNumber(event.x, 'event.x', 0, viewport.width); assertNumber(event.y, 'event.y', 0, viewport.height);
    if (kind !== 'mouseMove' && !['left', 'middle', 'right'].includes(event.button ?? 'left')) throw protocolError('event.button must be left, middle, or right');
    return kind;
  }
  if (kind === 'wheel') {
    assertNumber(event.x, 'event.x', 0, viewport.width); assertNumber(event.y, 'event.y', 0, viewport.height);
    assertNumber(event.deltaX ?? 0, 'event.deltaX', -10000, 10000); assertNumber(event.deltaY ?? 0, 'event.deltaY', -10000, 10000);
    return kind;
  }
  if (kind === 'keyDown' || kind === 'keyUp') {
    assertString(event.key, 'event.key', 128); assertString(event.code, 'event.code', 128);
    if (event.text !== undefined && typeof event.text !== 'string') throw protocolError('event.text must be a string');
    if (event.text && event.text.length > 16) throw protocolError('event.text is too long');
    return kind;
  }
  throw protocolError(`unsupported input event kind: ${kind}`);
}

function buttonMask(button) { return button === 'left' ? 1 : button === 'right' ? 2 : button === 'middle' ? 4 : 0; }

async function dispatchInput(event) {
  const kind = validateEvent(event);
  if (kind === 'mouseMove' || kind === 'mouseDown' || kind === 'mouseUp') {
    const type = { mouseMove: 'mouseMoved', mouseDown: 'mousePressed', mouseUp: 'mouseReleased' }[kind];
    const button = kind === 'mouseMove' ? 0 : buttonMask(event.button || 'left');
    if (kind === 'mouseDown') mouseButtons |= button;
    const derivedButtons = kind === 'mouseUp' ? mouseButtons & ~button : mouseButtons;
    const buttons = event.buttons ?? derivedButtons;
    const cdpButton = kind === 'mouseMove' ? (buttons & 1 ? 'left' : buttons & 2 ? 'right' : buttons & 4 ? 'middle' : 'none') : (event.button || 'left');
    lastPointer = { x: event.x, y: event.y };
    await cdp.send('Input.dispatchMouseEvent', { type, x: event.x, y: event.y, button: cdpButton, clickCount: kind === 'mouseMove' ? 0 : 1, buttons, modifiers: event.modifiers || 0 });
    if (kind === 'mouseUp') mouseButtons &= ~button;
  } else if (kind === 'wheel') {
    lastPointer = { x: event.x, y: event.y };
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: event.x, y: event.y, deltaX: event.deltaX || 0, deltaY: event.deltaY || 0, modifiers: event.modifiers || 0 });
    await cdp.send('Runtime.evaluate', { expression: 'new Promise((resolve) => requestAnimationFrame(() => resolve()))', awaitPromise: true, returnByValue: true });
  } else {
    await cdp.send('Input.dispatchKeyEvent', { type: kind === 'keyDown' ? 'keyDown' : 'keyUp', key: event.key, code: event.code, modifiers: event.modifiers || 0, ...(kind === 'keyDown' && event.text ? { text: event.text, unmodifiedText: event.text } : {}) });
  }
  const state = await evaluatePageState();
  return { cursor: String(state.cursor || 'default') };
}

async function navigate(url) { ensureReady(); const baseline = frameSequence; await page.goto(assertNavigationUrl(url), { waitUntil: 'domcontentloaded' }); await waitForScreencastAfter(baseline); return snapshot(); }
async function reload() { ensureReady(); const baseline = frameSequence; await page.reload({ waitUntil: 'domcontentloaded' }); await waitForScreencastAfter(baseline); return snapshot(); }

async function closeServer(server) {
  if (!server) return;
  server.closeAllConnections?.();
  await new Promise((resolve) => server.close(() => resolve())).catch(() => {});
}

async function shutdown() {
  if (shuttingDown) return;
  shuttingDown = true;
  try { if (cdp && screencastStarted) await cdp.send('Page.stopScreencast'); } catch {}
  screencastStarted = false;
  for (const client of [...mjpegClients.values()]) closeClient(mjpegClients, client);
  for (const socket of streamSockets) socket.destroy();
  streamSockets.clear();
  await closeServer(streamServer); streamServer = undefined; streamUrl = undefined; streamToken = undefined;
  try { await browser?.close(); } catch {}
  browser = undefined; context = undefined; page = undefined; cdp = undefined;
  latestFrame = undefined;
  await closeServer(fixtureServer); fixtureServer = undefined;
}

async function handle(request) {
  assertObject(request, 'request');
  const method = assertString(request.method, 'method', 64);
  if (method === 'start') {
    if (!browser) {
      await startFrameStream();
      await startFixture();
      try { await startBrowser(); } catch (error) { await shutdown(); throw error; }
    }
    return snapshot();
  }
  if (method === 'validateUrl') return { url: assertNavigationUrl(request.url) };
  if (method === 'testViewport') return currentViewport(assertObject(request.metadata, 'metadata'));
  if (method === 'snapshot') return snapshot();
  if (method === 'inspect') return inspect();
  if (method === 'input') return dispatchInput(request.event);
  if (method === 'navigate') return navigate(request.url);
  if (method === 'reload') return reload();
  if (method === 'stop') { await shutdown(); return { status: 'stopped' }; }
  throw protocolError(`unknown method: ${method}`);
}

function writeResponse(response) {
  const line = JSON.stringify(response);
  if (line.length > MAX_LINE) {
    process.stdout.write(`${JSON.stringify({ ok: false, id: response.id ?? null, error: 'response exceeded control protocol limit' })}\n`);
    return;
  }
  process.stdout.write(`${line}\n`);
}

const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
process.on('SIGTERM', () => { void shutdown().finally(() => process.exit(0)); });
process.on('SIGINT', () => { void shutdown().finally(() => process.exit(0)); });

try {
  for await (const line of input) {
    if (line.length > MAX_LINE) { writeResponse({ ok: false, error: 'request exceeded control protocol limit' }); continue; }
    if (!line.trim()) continue;
    let request;
    try { request = JSON.parse(line); } catch { writeResponse({ ok: false, error: 'request is not valid JSON' }); continue; }
    try { const result = await handle(request); writeResponse({ ok: true, id: request.id ?? null, result }); if (request.method === 'stop') break; }
    catch (error) { writeResponse({ ok: false, id: request.id ?? null, error: String(error?.message || error) }); }
  }
} finally { await shutdown(); }
