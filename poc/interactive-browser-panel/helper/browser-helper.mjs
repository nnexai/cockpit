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
    throw new Error(`Playwright is unavailable; run bun install in this POC directory (${error.message})`);
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
  } catch (error) {
    throw new Error(`Could not launch Chromium through Playwright${executablePath ? ` (${executablePath})` : ''}: ${error.message}. Install Chromium with 'bunx playwright install chromium' or set BROWSER_BINARY.`);
  }
}

function ensureReady() {
  if (!page || !cdp || !fixtureUrl) throw new Error('Browser is not started');
}

function assertFixtureUrl(value) {
  const candidate = assertString(value, 'url', 2048);
  let parsed;
  try {
    parsed = new URL(candidate);
  } catch {
    throw protocolError('url must be an absolute loopback fixture URL');
  }
  const expected = new URL(fixtureUrl);
  if (parsed.protocol !== 'http:' || parsed.hostname !== '127.0.0.1' || parsed.port !== expected.port || !['/', '/index.html'].includes(parsed.pathname) || parsed.username || parsed.password) {
    throw protocolError('navigation is restricted to the local fixture origin');
  }
  return parsed.href;
}

async function evaluatePageState() {
  const result = await cdp.send('Runtime.evaluate', {
    expression: `(() => ({
      title: document.title.slice(0, 200),
      url: location.href.slice(0, 2048),
      activeElement: (() => {
        const element = document.activeElement;
        if (!element) return 'BODY';
        const id = element.id ? '#' + element.id.slice(0, 80) : '';
        return (element.tagName || 'UNKNOWN').slice(0, 40) + id;
      })(),
      fixtureStatus: (document.querySelector('#result')?.textContent || '').slice(0, 240)
    }))()`,
    returnByValue: true,
    awaitPromise: false,
  });
  return result.result?.value || { title: '', url: fixtureUrl, activeElement: 'BODY', fixtureStatus: '' };
}

async function captureScreenshot() {
  let lastError;
  for (const delay of [0, 50, 100, 200, 400]) {
    if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
    try {
      // Playwright's cross-platform screenshot path waits for a paint; CDP remains
      // the control/inspection boundary for input and Runtime state.
      const image = await page.screenshot({ type: 'png' });
      return image.toString('base64');
    } catch (error) {
      lastError = error;
    }
  }
  throw new Error(`Unable to capture Chromium screenshot after bounded retries: ${lastError?.message || 'unknown error'}`);
}

async function snapshot() {
  ensureReady();
  const [image, state] = await Promise.all([
    captureScreenshot(),
    evaluatePageState(),
  ]);
  return {
    pngDataUrl: `data:image/png;base64,${image}`,
    width: WIDTH,
    height: HEIGHT,
    title: String(state.title || '').slice(0, 200),
    url: String(state.url || fixtureUrl).slice(0, 2048),
  };
}

async function inspect() {
  ensureReady();
  const state = await evaluatePageState();
  return {
    status: 'ready',
    fixtureStatus: String(state.fixtureStatus || '').slice(0, 240),
    title: String(state.title || '').slice(0, 200),
    url: String(state.url || fixtureUrl).slice(0, 2048),
    activeElement: String(state.activeElement || 'BODY').slice(0, 128),
  };
}

function validateEvent(event) {
  assertObject(event, 'event');
  const kind = assertString(event.kind, 'event.kind', 32);
  if (event.modifiers !== undefined && (!Number.isInteger(event.modifiers) || event.modifiers < 0 || event.modifiers > 15)) {
    throw protocolError('event.modifiers must be an integer bitmask from 0 to 15');
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

async function dispatchInput(event) {
  const kind = validateEvent(event);
  if (kind === 'mouseMove' || kind === 'mouseDown' || kind === 'mouseUp') {
    const type = { mouseMove: 'mouseMoved', mouseDown: 'mousePressed', mouseUp: 'mouseReleased' }[kind];
    await cdp.send('Input.dispatchMouseEvent', {
      type,
      x: event.x,
      y: event.y,
      button: kind === 'mouseMove' ? 'none' : (event.button || 'left'),
      clickCount: kind === 'mouseMove' ? 0 : 1,
      modifiers: event.modifiers || 0,
    });
  } else if (kind === 'wheel') {
    await cdp.send('Input.dispatchMouseEvent', {
      type: 'mouseWheel', x: event.x, y: event.y,
      deltaX: event.deltaX || 0, deltaY: event.deltaY || 0,
      modifiers: event.modifiers || 0,
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
  // A fresh bounded capture makes the effect of a click or keystroke observable to the panel.
  return snapshot();
}

async function navigate(url) {
  ensureReady();
  await page.goto(assertFixtureUrl(url), { waitUntil: 'domcontentloaded' });
  return snapshot();
}

async function reload() {
  ensureReady();
  await page.reload({ waitUntil: 'domcontentloaded' });
  return snapshot();
}

async function shutdown() {
  if (shuttingDown) return;
  shuttingDown = true;
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
