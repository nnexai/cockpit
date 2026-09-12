#!/usr/bin/env node

import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import process from 'node:process';
import readline from 'node:readline';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const FIXTURE_PATH = path.join(ROOT, 'fixture', 'index.html');
const WIDTH = 1024;
const HEIGHT = 720;
// Screenshots are bounded to a practical IPC frame size; requests remain tiny.
const MAX_LINE = 8 * 1024 * 1024;

let playwright;
let server;
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

async function startFixture() {
  server = createServer(async (request, response) => {
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
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Unable to determine fixture port');
  fixtureUrl = `http://127.0.0.1:${address.port}/`;
}

async function startBrowser() {
  try {
    playwright ??= await import('playwright');
  } catch (error) {
    throw new Error(`Playwright is unavailable; run "bun install --frozen-lockfile" then "bunx playwright install chromium" in this POC directory, or set BROWSER_BINARY (${error.message})`);
  }
  const executablePath = process.env.BROWSER_BINARY;
  // chromium.launch creates a unique temporary profile and Playwright-owned local
  // transport; the CDP session below keeps the browser boundary explicit.
  const launchOptions = {
    headless: true,
    ...(executablePath ? { executablePath } : {}),
    args: ['--no-first-run', '--no-default-browser-check', '--disable-background-networking'],
  };
  try {
    browser = await playwright.chromium.launch(launchOptions);
    context = await browser.newContext({ viewport: { width: WIDTH, height: HEIGHT }, deviceScaleFactor: 1 });
    page = await context.newPage();
    cdp = await context.newCDPSession(page);
    await cdp.send('Page.enable');
    await cdp.send('Runtime.enable');
    await cdp.send('Page.setLifecycleEventsEnabled', { enabled: true });
    await page.goto(fixtureUrl, { waitUntil: 'domcontentloaded' });
    await startScreencast();
  } catch (error) {
    throw new Error(`Could not launch Chromium through Playwright${executablePath ? ` (${executablePath})` : ''}: ${error.message}. Run "bun install --frozen-lockfile" then "bunx playwright install chromium", or set BROWSER_BINARY.`);
  }
}
async function waitForFrameAfter(sequence, timeoutMs = 8_000) {
  const deadline = performance.now() + timeoutMs;
  while (!latestFrame || latestFrame.sequence <= sequence) {
    if (frameError) throw frameError;
    const remaining = deadline - performance.now();
    if (remaining <= 0) throw new Error(`timed out waiting for screencast frame after ${sequence}`);
    await new Promise((resolve) => setTimeout(resolve, Math.min(remaining, 25)));
  }
  if (frameError) throw frameError;
  return latestFrame;
}

async function startScreencast() {
  const baseline = frameSequence;
  cdp.on('Page.screencastFrame', (event) => {
    frameSequence += 1;
    latestFrame = {
      data: event.data,
      metadata: event.metadata && typeof event.metadata === 'object' ? event.metadata : {},
      sequence: frameSequence,
    };
    writeFrameEvent(latestFrame);
    // Page.startScreencast applies protocol backpressure: Chromium sends the
    // next frame only after every sessionId has been acknowledged.
    void cdp.send('Page.screencastFrameAck', { sessionId: event.sessionId }).catch((error) => {
      frameError ??= error;
    });
  });
  await cdp.send('Page.startScreencast', {
    everyNthFrame: 1,
    format: 'jpeg',
    maxHeight: HEIGHT,
    maxWidth: WIDTH,
    quality: 70,
  });
  screencastStarted = true;
  await waitForFrameAfter(baseline);
}


function ensureReady() {
  if (!page || !cdp || !fixtureUrl) throw new Error('Browser is not started');
}

function assertNavigationUrl(value) {
  let candidate = assertString(value, 'url', 2048).trim();
  if (!/^[a-z][a-z\d+.-]*:\/\//i.test(candidate)) candidate = `https://${candidate}`;
  let parsed;
  try {
    parsed = new URL(candidate);
  } catch {
    throw protocolError('url must be an absolute http:// or https:// URL (bare domains are normalized to https://)');
  }
  if (!['http:', 'https:'].includes(parsed.protocol)) {
    throw protocolError('navigation only supports http:// and https:// URLs');
  }
  if (!parsed.hostname || parsed.username || parsed.password) {
    throw protocolError('navigation URL must not contain credentials');
  }
  return parsed.href;
}

async function evaluatePageState(x = lastPointer.x, y = lastPointer.y) {
  const pointX = Math.max(0, Math.min(WIDTH, Number.isFinite(x) ? x : 0));
  const pointY = Math.max(0, Math.min(HEIGHT, Number.isFinite(y) ? y : 0));
  const result = await cdp.send('Runtime.evaluate', {
    expression: `(() => {
      const element = document.elementFromPoint(${pointX}, ${pointY});
      const computedCursor = getComputedStyle(element || document.body).cursor;
      const editable = !!element && (element.matches('input, textarea, [contenteditable="true"]') || !!element.closest?.('[contenteditable="true"]'));
      const cursor = computedCursor === 'auto' && editable ? 'text' : computedCursor;
      const safeCursors = new Set(['default', 'auto', 'pointer', 'text', 'crosshair', 'move', 'not-allowed', 'wait', 'grab', 'grabbing', 'cell', 'help', 'progress', 'zoom-in', 'zoom-out', 'col-resize', 'row-resize', 'e-resize', 'w-resize', 'n-resize', 's-resize']);
      return {
        title: document.title.slice(0, 200),
        url: location.href.slice(0, 2048),
        activeElement: (() => {
          const active = document.activeElement;
          if (!active) return 'BODY';
          const id = active.id ? '#' + active.id.slice(0, 80) : '';
          return (active.tagName || 'UNKNOWN').slice(0, 40) + id;
        })(),
        fixtureStatus: (document.querySelector('#result')?.textContent || '').slice(0, 240),
        selectedText: (window.getSelection()?.toString() || '').slice(0, 240),
        scrollY: Math.max(0, Math.round(window.scrollY)),
        cursor: safeCursors.has(cursor) ? cursor : 'default'
      };
    })()`,
    returnByValue: true,
    awaitPromise: false,
  });
  return result.result?.value || { title: '', url: fixtureUrl, activeElement: 'BODY', fixtureStatus: '', selectedText: '', scrollY: 0, cursor: 'default' };
}

async function snapshot() {
  ensureReady();
  const [frame, state] = await Promise.all([
    waitForFrameAfter(0),
    evaluatePageState(),
  ]);
  return {
    jpegDataUrl: `data:image/jpeg;base64,${frame.data}`,
    width: WIDTH,
    height: HEIGHT,
    title: String(state.title || '').slice(0, 200),
    url: String(state.url || fixtureUrl).slice(0, 2048),
    cursor: String(state.cursor || 'default'),
  };
}

let pendingFrameEvent;
let frameWritePending = false;
let lastPublishedFrame = 0;

function writeFrameEvent(frame) {
  // The helper keeps at most one frame waiting for the Rust reader. CDP ACKs
  // still happen for every frame, while stdout transport coalesces to latest.
  pendingFrameEvent = frame;
  if (frameWritePending) return;
  frameWritePending = true;
  const flush = () => {
    const next = pendingFrameEvent;
    pendingFrameEvent = undefined;
    if (next && next.sequence > lastPublishedFrame) {
      lastPublishedFrame = next.sequence;
      const line = JSON.stringify({
        event: 'screencastFrame',
        frame: {
          jpegDataUrl: `data:image/jpeg;base64,${next.data}`,
          metadata: next.metadata,
          sequence: next.sequence,
          width: WIDTH,
          height: HEIGHT,
        },
      });
      if (!process.stdout.write(`${line}\n`)) {
        process.stdout.once('drain', flush);
        return;
      }
    }
    if (pendingFrameEvent) queueMicrotask(flush);
    else frameWritePending = false;
  };
  flush();
}

async function inspect() {
  ensureReady();
  const state = await evaluatePageState();
  return {
    status: 'ready',
    fixtureStatus: String(state.fixtureStatus || '').slice(0, 240),
    selectedText: String(state.selectedText || '').slice(0, 240),
    scrollY: Math.max(0, Math.min(100000, Number(state.scrollY) || 0)),
    title: String(state.title || '').slice(0, 200),
    url: String(state.url || fixtureUrl).slice(0, 2048),
    activeElement: String(state.activeElement || 'BODY').slice(0, 128),
    cursor: String(state.cursor || 'default'),
  };
}


function validateEvent(event) {
  assertObject(event, 'event');
  const kind = assertString(event.kind, 'event.kind', 32);
  if (event.modifiers !== undefined && (!Number.isInteger(event.modifiers) || event.modifiers < 0 || event.modifiers > 15)) {
    throw protocolError('event.modifiers must be an integer bitmask from 0 to 15');
  }
  if (event.buttons !== undefined && (!Number.isInteger(event.buttons) || event.buttons < 0 || event.buttons > 31)) {
    throw protocolError('event.buttons must be an integer bitmask from 0 to 31');
  }
  if (['mouseMove', 'mouseDown', 'mouseUp'].includes(kind)) {
    assertNumber(event.x, 'event.x', 0, WIDTH);
    assertNumber(event.y, 'event.y', 0, HEIGHT);
    if (kind !== 'mouseMove') {
      const button = event.button ?? 'left';
      if (!['left', 'middle', 'right'].includes(button)) throw protocolError('event.button must be left, middle, or right');
    }
    return kind;
  }
  if (kind === 'wheel') {
    assertNumber(event.x, 'event.x', 0, WIDTH);
    assertNumber(event.y, 'event.y', 0, HEIGHT);
    assertNumber(event.deltaX ?? 0, 'event.deltaX', -10000, 10000);
    assertNumber(event.deltaY ?? 0, 'event.deltaY', -10000, 10000);
    return kind;
  }
  if (kind === 'keyDown' || kind === 'keyUp') {
    assertString(event.key, 'event.key', 128);
    assertString(event.code, 'event.code', 128);
    if (event.text !== undefined && typeof event.text !== 'string') throw protocolError('event.text must be a string');
    if (event.text && event.text.length > 16) throw protocolError('event.text is too long');
    return kind;
  }
  throw protocolError(`unsupported input event kind: ${kind}`);
}

function buttonMask(button) {
  return button === 'left' ? 1 : button === 'right' ? 2 : button === 'middle' ? 4 : 0;
}
async function dispatchInput(event) {
  const kind = validateEvent(event);
  if (kind === 'mouseMove' || kind === 'mouseDown' || kind === 'mouseUp') {
    const type = { mouseMove: 'mouseMoved', mouseDown: 'mousePressed', mouseUp: 'mouseReleased' }[kind];
    const button = kind === 'mouseMove' ? 0 : buttonMask(event.button || 'left');
    if (kind === 'mouseDown') mouseButtons |= button;
    const derivedButtons = kind === 'mouseUp' ? mouseButtons & ~button : mouseButtons;
    const buttons = event.buttons ?? derivedButtons;
    // This Chromium build ignores dragged mouseMoved events with button:"none";
    // send the active button alongside buttons so text selection progresses.
    const cdpButton = kind === 'mouseMove'
      ? (buttons & 1 ? 'left' : buttons & 2 ? 'right' : buttons & 4 ? 'middle' : 'none')
      : (event.button || 'left');
    lastPointer = { x: event.x, y: event.y };
    await cdp.send('Input.dispatchMouseEvent', {
      type,
      x: event.x,
      y: event.y,
      button: cdpButton,
      clickCount: kind === 'mouseMove' ? 0 : 1,
      buttons,
      modifiers: event.modifiers || 0,
    });
    if (kind === 'mouseUp') mouseButtons &= ~button;
  } else if (kind === 'wheel') {
    lastPointer = { x: event.x, y: event.y };
    await cdp.send('Input.dispatchMouseEvent', {
      type: 'mouseWheel', x: event.x, y: event.y,
      deltaX: event.deltaX || 0, deltaY: event.deltaY || 0,
      modifiers: event.modifiers || 0,
    });
    // Let Chromium commit the wheel scroll before reading the acknowledgement state.
    await cdp.send('Runtime.evaluate', {
      expression: 'new Promise((resolve) => requestAnimationFrame(() => resolve()))',
      awaitPromise: true,
      returnByValue: true,
    });
  } else {
    await cdp.send('Input.dispatchKeyEvent', {
      type: kind === 'keyDown' ? 'keyDown' : 'keyUp',
      key: event.key,
      code: event.code,
      modifiers: event.modifiers || 0,
      ...(kind === 'keyDown' && event.text ? { text: event.text, unmodifiedText: event.text } : {}),
    });
  }
  const state = await evaluatePageState();
  return { cursor: String(state.cursor || 'default') };
}

async function navigate(url) {
  ensureReady();
  const baseline = frameSequence;
  await page.goto(assertNavigationUrl(url), { waitUntil: 'domcontentloaded' });
  await waitForFrameAfter(baseline);
  return snapshot();
}

async function reload() {
  ensureReady();
  const baseline = frameSequence;
  await page.reload({ waitUntil: 'domcontentloaded' });
  await waitForFrameAfter(baseline);
  return snapshot();
}

async function shutdown() {
  if (shuttingDown) return;
  shuttingDown = true;
  try {
    if (cdp && screencastStarted) await cdp.send('Page.stopScreencast');
  } catch { /* process is already exiting */ }
  screencastStarted = false;
  try { await browser?.close(); } catch { /* process is already exiting */ }
  browser = undefined;
  context = undefined;
  page = undefined;
  cdp = undefined;
  try {
    if (server) await new Promise((resolve) => server.close(() => resolve()));
  } catch { /* server may already be closed */ }
  server = undefined;
}

async function handle(request) {
  assertObject(request, 'request');
  const id = request.id === undefined ? null : String(request.id).slice(0, 80);
  const method = assertString(request.method, 'method', 64);
  if (method === 'start') {
    if (!browser) {
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
  if (line.length > MAX_LINE) {
    process.stdout.write(`${JSON.stringify({ ok: false, error: 'response exceeded protocol limit' })}\n`);
    return;
  }
  process.stdout.write(`${line}\n`);
}

const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
process.on('SIGTERM', () => { void shutdown().finally(() => process.exit(0)); });
process.on('SIGINT', () => { void shutdown().finally(() => process.exit(0)); });

try {
  for await (const line of input) {
    if (line.length > MAX_LINE) {
      writeResponse({ ok: false, error: 'request exceeded protocol limit' });
      continue;
    }
    if (!line.trim()) continue;
    let request;
    try { request = JSON.parse(line); } catch { writeResponse({ ok: false, error: 'request is not valid JSON' }); continue; }
    try {
      const result = await handle(request);
      writeResponse({ ok: true, id: request.id ?? null, result });
      if (request.method === 'stop') break;
    } catch (error) {
      writeResponse({ ok: false, id: request.id ?? null, error: String(error?.message || error) });
    }
  }
} finally {
  await shutdown();
}
