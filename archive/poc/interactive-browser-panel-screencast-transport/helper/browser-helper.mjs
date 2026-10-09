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
const DEFAULT_VIEWPORT = { width: 1024, height: 720 };
const TARGET_FPS = 30;
const FRAME_INTERVAL_MS = Math.round(1_000 / TARGET_FPS);
const RESOURCE_TEST = process.env.POC_RESOURCE_TEST === '1';
const MAX_LINE = 8 * 1024;
const MAX_FRAME_BYTES = 6 * 1024 * 1024;
const MAX_INBOUND_FRAME_BYTES = 1_024;
const CLIENT_WRITE_TIMEOUT_MS = 2_000;
const FRAME_HEADER_BYTES = 48;
const FRAME_MAGIC = 0x49504246; // "IPBF"
const FRAME_VERSION = 1;

let playwright;
let fixtureServer;
let frameServer;
let frameToken;
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
const frameClients = new Map();
const frameSockets = new Set();

function protocolError(message) {
  const error = new Error(message);
  error.protocol = true;
  return error;
}

function assertObject(value, name) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw protocolError(`${name} must be an object`);
  }
  return value;
}

function assertString(value, name, max = 256) {
  if (typeof value !== 'string' || value.length === 0 || value.length > max) {
    throw protocolError(`${name} must be a non-empty string of at most ${max} characters`);
  }
  return value;
}

function assertNumber(value, name, min, max) {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < min || value > max) {
    throw protocolError(`${name} must be a finite number between ${min} and ${max}`);
  }
  return value;
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function websocketControlFrame(opcode, payload = Buffer.alloc(0)) {
  if (payload.byteLength > 125) throw new Error('WebSocket control payload is too large');
  return Buffer.concat([Buffer.from([0x80 | opcode, payload.byteLength]), payload]);
}

function removeClient(client) {
  clearTimeout(client.ackTimer);
  client.pending = undefined;
  client.input = Buffer.alloc(0);
  frameClients.delete(client.socket);
}

function closeClient(client, status = 1008, payload) {
  if (!frameClients.has(client.socket)) return;
  const canReplyCleanly = status === 1000 && !client.awaitingSequence && payload?.byteLength <= 125;
  removeClient(client);
  if (client.socket.destroyed) return;
  if (canReplyCleanly) {
    client.socket.end(websocketControlFrame(0x8, payload), () => client.socket.destroy());
  } else {
    client.socket.destroy();
  }
}

function rejectUpgrade(socket, status = 400) {
  if (socket.destroyed) return;
  socket.end(`HTTP/1.1 ${status} Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n`, () => socket.destroy());
}

function writeClientFrame(client, frame) {
  if (!frameClients.has(client.socket) || client.socket.destroyed) return;
  client.awaitingSequence = frame.sequence;
  client.ackTimer = setTimeout(() => closeClient(client, 1013), CLIENT_WRITE_TIMEOUT_MS);
  try {
    client.socket.write(frame.message);
  } catch {
    closeClient(client, 1011);
  }
}

function queueLatestFrame(client, frame) {
  if (!frameClients.has(client.socket) || client.socket.destroyed) return;
  // The browser ACK is the delivery boundary: exactly one displayed-or-pending
  // frame may exist per client, and each newer arrival replaces the pending one.
  if (client.awaitingSequence !== undefined) {
    client.pending = frame;
    return;
  }
  writeClientFrame(client, frame);
}

function acknowledgeFrame(client, payload) {
  const expected = client.awaitingSequence;
  if (expected === undefined || !/^ack:\d{1,10}$/.test(payload)) return closeClient(client, 1008);
  const sequence = Number(payload.slice(4));
  if (!Number.isSafeInteger(sequence) || sequence !== expected) return closeClient(client, 1008);
  clearTimeout(client.ackTimer);
  client.awaitingSequence = undefined;
  const pending = client.pending;
  client.pending = undefined;
  if (pending) writeClientFrame(client, pending);
}

function consumeClientData(client, chunk) {
  client.input = Buffer.concat([client.input, chunk]);
  if (client.input.byteLength > MAX_INBOUND_FRAME_BYTES + 14) {
    closeClient(client, 1009);
    return;
  }
  while (client.input.byteLength >= 2) {
    const first = client.input[0];
    const second = client.input[1];
    const opcode = first & 0x0f;
    const fin = (first & 0x80) !== 0;
    const masked = (second & 0x80) !== 0;
    let payloadLength = second & 0x7f;
    let headerBytes = 2;
    if (!fin || (first & 0x70) !== 0 || !masked) return closeClient(client, 1002);
    if (payloadLength === 126) {
      if (client.input.byteLength < 4) return;
      payloadLength = client.input.readUInt16BE(2);
      headerBytes = 4;
    } else if (payloadLength === 127) {
      return closeClient(client, 1009);
    }
    if (payloadLength > MAX_INBOUND_FRAME_BYTES) return closeClient(client, 1009);
    if (opcode >= 0x8 && payloadLength > 125) return closeClient(client, 1002);
    if (client.input.byteLength < headerBytes + 4 + payloadLength) return;
    const maskOffset = headerBytes;
    const payloadOffset = maskOffset + 4;
    const payload = Buffer.from(client.input.subarray(payloadOffset, payloadOffset + payloadLength));
    for (let index = 0; index < payload.byteLength; index += 1) payload[index] ^= client.input[maskOffset + (index % 4)];
    client.input = client.input.subarray(payloadOffset + payloadLength);
    if (opcode === 0x8) {
      if (payload.byteLength === 1) return closeClient(client, 1002);
      return closeClient(client, 1000, payload);
    }
    if (opcode === 0x9) {
      client.socket.write(websocketControlFrame(0xA, payload));
      continue;
    }
    if (opcode === 0xA) continue;
    if (opcode === 0x1) {
      acknowledgeFrame(client, payload.toString('utf8'));
      continue;
    }
    return closeClient(client, 1003);
  }
}

async function startFrameStream() {
  if (frameServer) return;
  frameToken = randomBytes(24).toString('base64url');
  frameServer = createServer((_, response) => {
    response.writeHead(426, { 'content-type': 'text/plain; charset=utf-8' });
    response.end('WebSocket upgrade required');
  });
  frameServer.on('connection', (socket) => {
    frameSockets.add(socket);
    socket.on('close', () => frameSockets.delete(socket));
    socket.on('error', () => frameSockets.delete(socket));
  });
  frameServer.on('upgrade', (request, socket, head) => {
    let requestUrl;
    try { requestUrl = new URL(request.url || '/', 'http://127.0.0.1'); } catch { rejectUpgrade(socket); return; }
    const key = request.headers['sec-websocket-key'];
    const connection = request.headers.connection || '';
    const validKey = typeof key === 'string' && /^[A-Za-z0-9+/]{22}==$/.test(key) && Buffer.from(key, 'base64').byteLength === 16;
    if (request.method !== 'GET' || head.byteLength !== 0 || requestUrl.pathname !== '/frames' ||
      requestUrl.searchParams.get('token') !== frameToken || request.headers['sec-websocket-version'] !== '13' ||
      request.headers.upgrade?.toLowerCase() !== 'websocket' || !/(?:^|,)\s*upgrade\s*(?:,|$)/i.test(connection) || !validKey) {
      rejectUpgrade(socket);
      return;
    }
    const accept = createHash('sha1').update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
    socket.setNoDelay(true);
    socket.write([
      'HTTP/1.1 101 Switching Protocols',
      'Upgrade: websocket',
      'Connection: Upgrade',
      `Sec-WebSocket-Accept: ${accept}`,
      '',
      '',
    ].join('\r\n'));
    const client = { socket, input: Buffer.alloc(0), awaitingSequence: undefined, pending: undefined, ackTimer: undefined };
    frameClients.set(socket, client);
    socket.on('data', (chunk) => {
      try { consumeClientData(client, chunk); } catch { closeClient(client, 1002); }
    });
    socket.on('close', () => removeClient(client));
    socket.on('error', () => removeClient(client));
    if (latestFrame) publishFrame(latestFrame);
  });
  await new Promise((resolve, reject) => {
    frameServer.once('error', reject);
    frameServer.listen(0, '127.0.0.1', resolve);
  });
  const address = frameServer.address();
  if (!address || typeof address === 'string') throw new Error('Unable to determine frame stream port');
  streamUrl = `ws://127.0.0.1:${address.port}/frames?token=${frameToken}`;
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

async function currentViewport(metadata = {}) {
  try {
    const metrics = await cdp.send('Page.getLayoutMetrics');
    const viewport = metrics.cssVisualViewport || metrics.visualViewport || {};
    return {
      width: boundedDimension(viewport.clientWidth, boundedDimension(metadata.deviceWidth, DEFAULT_VIEWPORT.width)),
      height: boundedDimension(viewport.clientHeight, boundedDimension(metadata.deviceHeight, DEFAULT_VIEWPORT.height)),
      scale: Number.isFinite(viewport.scale) && viewport.scale > 0 ? viewport.scale : 1,
      offsetX: Number.isFinite(viewport.pageX) ? viewport.pageX : 0,
      offsetY: Number.isFinite(viewport.pageY) ? viewport.pageY : 0,
    };
  } catch {
    return {
      width: boundedDimension(metadata.deviceWidth, DEFAULT_VIEWPORT.width),
      height: boundedDimension(metadata.deviceHeight, DEFAULT_VIEWPORT.height),
      scale: Number.isFinite(metadata.pageScaleFactor) && metadata.pageScaleFactor > 0 ? metadata.pageScaleFactor : 1,
      offsetX: 0,
      offsetY: Number.isFinite(metadata.offsetTop) ? metadata.offsetTop : 0,
    };
  }
}

function publicFrame(frame) {
  return {
    sequence: frame.sequence,
    pixelWidth: frame.pixelWidth,
    pixelHeight: frame.pixelHeight,
    viewport: frame.viewport,
  };
}

function encodeWebsocketFrame(frame) {
  const payloadLength = FRAME_HEADER_BYTES + frame.bytes.byteLength;
  const websocketHeaderBytes = payloadLength <= 125 ? 2 : payloadLength <= 0xffff ? 4 : 10;
  const message = Buffer.allocUnsafe(websocketHeaderBytes + payloadLength);
  message[0] = 0x82;
  if (websocketHeaderBytes === 2) {
    message[1] = payloadLength;
  } else if (websocketHeaderBytes === 4) {
    message[1] = 126;
    message.writeUInt16BE(payloadLength, 2);
  } else {
    message[1] = 127;
    message.writeBigUInt64BE(BigInt(payloadLength), 2);
  }
  const offset = websocketHeaderBytes;
  message.writeUInt32BE(FRAME_MAGIC, offset);
  message.writeUInt8(FRAME_VERSION, offset + 4);
  message.writeUInt8(0, offset + 5);
  message.writeUInt16BE(FRAME_HEADER_BYTES, offset + 6);
  message.writeUInt32BE(frame.sequence >>> 0, offset + 8);
  message.writeUInt32BE(frame.pixelWidth, offset + 12);
  message.writeUInt32BE(frame.pixelHeight, offset + 16);
  message.writeUInt32BE(frame.viewport.width, offset + 20);
  message.writeUInt32BE(frame.viewport.height, offset + 24);
  message.writeFloatBE(frame.viewport.scale, offset + 28);
  message.writeFloatBE(frame.viewport.offsetX, offset + 32);
  message.writeFloatBE(frame.viewport.offsetY, offset + 36);
  message.writeUInt32BE(frame.bytes.byteLength, offset + 40);
  message.writeUInt32BE(0, offset + 44);
  frame.bytes.copy(message, offset + FRAME_HEADER_BYTES);
  return message;
}

function publishFrame(frame) {
  if (frameClients.size === 0) return;
  const delivery = { sequence: frame.sequence, message: encodeWebsocketFrame(frame) };
  for (const client of frameClients.values()) queueLatestFrame(client, delivery);
}

async function startFixture() {
  fixtureServer = createServer(async (request, response) => {
    const requestUrl = new URL(request.url || '/', 'http://127.0.0.1');
    if (request.method !== 'GET' || !['/', '/index.html'].includes(requestUrl.pathname)) {
      response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
      response.end('Not found');
      return;
    }
    try {
      const html = await readFile(FIXTURE_PATH);
      response.writeHead(200, {
        'content-type': 'text/html; charset=utf-8',
        'cache-control': 'no-store',
        'content-length': html.byteLength,
      });
      response.end(html);
    } catch {
      response.writeHead(500, { 'content-type': 'text/plain; charset=utf-8' });
      response.end('Fixture unavailable');
    }
  });
  await new Promise((resolve, reject) => {
    fixtureServer.once('error', reject);
    fixtureServer.listen(0, '127.0.0.1', resolve);
  });
  const address = fixtureServer.address();
  if (!address || typeof address === 'string') throw new Error('Unable to determine fixture port');
  fixtureUrl = `http://127.0.0.1:${address.port}/${RESOURCE_TEST ? '?resource-test=1' : ''}`;
}

async function startBrowser() {
  try { playwright ??= await import('playwright'); } catch (error) {
    throw new Error(`Playwright is unavailable; run "bun install --frozen-lockfile" then "bunx playwright install chromium" in this POC directory, or set BROWSER_BINARY (${error.message})`);
  }
  const executablePath = process.env.BROWSER_BINARY;
  try {
    browser = await playwright.chromium.launch({
      headless: true,
      ...(executablePath ? { executablePath } : {}),
      args: ['--no-first-run', '--no-default-browser-check', '--disable-background-networking'],
    });
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
  if (bytes.byteLength === 0 || bytes.byteLength > MAX_FRAME_BYTES) throw new Error('screencast frame exceeded binary transport limit');
  const dimensions = jpegDimensions(bytes);
  if (!dimensions) throw new Error('screencast frame was not a valid JPEG');
  const viewport = await currentViewport(event.metadata && typeof event.metadata === 'object' ? event.metadata : {});
  const frame = {
    sequence: frameSequence + 1,
    pixelWidth: dimensions.width,
    pixelHeight: dimensions.height,
    viewport,
    bytes,
  };
  // This only suppresses helper-to-webview transport work. Chromium already
  // captured and JPEG-encoded this CDP frame before the helper can compare it.
  if (!latestFrame || !samePublishedFrame(frame, latestFrame)) {
    latestFrame = frame;
    publishFrame(frame);
  }
  // Increment only after the latest published frame has been synchronously
  // replaced or the duplicate comparison has completed, so control snapshots
  // cannot observe a half-processed callback.
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
    // is cadence-bound; duplicates merely skip helper/webview transport work.
    void (async () => {
      try { await processScreencastFrame(event); } catch (error) { frameError ??= error; }
      try { await acknowledgeScreencastFrame(event.sessionId); } catch (error) { frameError ??= error; }
    })();
  });
  await cdp.send('Page.startScreencast', {
    everyNthFrame: 1,
    format: 'jpeg',
    maxHeight: 1_600,
    maxWidth: 2_560,
    quality: 70,
  });
  screencastStarted = true;
  await waitForScreencastAfter(baseline);
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
  const result = await cdp.send('Runtime.evaluate', {
    expression: `(() => {
      const element = document.elementFromPoint(${pointX}, ${pointY});
      const computedCursor = getComputedStyle(element || document.body).cursor;
      const editable = !!element && (element.matches('input, textarea, [contenteditable="true"]') || !!element.closest?.('[contenteditable="true"]'));
      const cursor = computedCursor === 'auto' && editable ? 'text' : computedCursor;
      const safeCursors = new Set(['default', 'auto', 'pointer', 'text', 'crosshair', 'move', 'not-allowed', 'wait', 'grab', 'grabbing', 'cell', 'help', 'progress', 'zoom-in', 'zoom-out', 'col-resize', 'row-resize', 'e-resize', 'w-resize', 'n-resize', 's-resize']);
      return {
        title: document.title.slice(0, 200), url: location.href.slice(0, 2048),
        activeElement: (() => { const active = document.activeElement; if (!active) return 'BODY'; const id = active.id ? '#' + active.id.slice(0, 80) : ''; return (active.tagName || 'UNKNOWN').slice(0, 40) + id; })(),
        fixtureStatus: (document.querySelector('#result')?.textContent || '').slice(0, 240),
        selectedText: (window.getSelection()?.toString() || '').slice(0, 240),
        scrollY: Math.max(0, Math.round(window.scrollY)),
        cursor: safeCursors.has(cursor) ? cursor : 'default',
      };
    })()`, returnByValue: true, awaitPromise: false,
  });
  return result.result?.value || { title: '', url: fixtureUrl, activeElement: 'BODY', fixtureStatus: '', selectedText: '', scrollY: 0, cursor: 'default' };
}

async function snapshot() {
  ensureReady();
  const [frame, state] = await Promise.all([waitForLatestFrame(), evaluatePageState()]);
  return {
    streamUrl, ...publicFrame(frame),
    title: String(state.title || '').slice(0, 200), url: String(state.url || fixtureUrl).slice(0, 2048), cursor: String(state.cursor || 'default'),
  };
}

async function inspect() {
  ensureReady();
  const state = await evaluatePageState();
  return {
    status: 'ready', fixtureStatus: String(state.fixtureStatus || '').slice(0, 240), selectedText: String(state.selectedText || '').slice(0, 240),
    scrollY: Math.max(0, Math.min(100000, Number(state.scrollY) || 0)), title: String(state.title || '').slice(0, 200),
    url: String(state.url || fixtureUrl).slice(0, 2048), activeElement: String(state.activeElement || 'BODY').slice(0, 128), cursor: String(state.cursor || 'default'),
  };
}

function validateEvent(event) {
  assertObject(event, 'event');
  const kind = assertString(event.kind, 'event.kind', 32);
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
  for (const client of [...frameClients.values()]) {
    removeClient(client);
    client.socket.destroy();
  }
  for (const socket of frameSockets) socket.destroy();
  frameSockets.clear();
  await closeServer(frameServer); frameServer = undefined; streamUrl = undefined; frameToken = undefined;
  try { await browser?.close(); } catch {}
  browser = undefined; context = undefined; page = undefined; cdp = undefined;
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
  if (line.length > MAX_LINE) { process.stdout.write(`${JSON.stringify({ ok: false, error: 'response exceeded control protocol limit' })}\n`); return; }
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
