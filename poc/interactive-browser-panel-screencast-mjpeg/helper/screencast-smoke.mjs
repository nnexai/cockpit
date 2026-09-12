#!/usr/bin/env node

import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const FIXTURE_PATH = path.join(ROOT, 'fixture', 'index.html');
const WIDTH = 1024;
const HEIGHT = 720;
const JPEG_QUALITY = 70;
const FRAME_TIMEOUT_MS = 8_000;

let browser;
let context;
let page;
let cdp;
let server;
let screencastStarted = false;
let latestFrame;
let frameCount = 0;
let frameError;

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function frameMetadata(value) {
  return value && typeof value === 'object' ? value : {};
}

function jpegDimensions(base64) {
  const bytes = Buffer.from(base64, 'base64');
  if (bytes.length < 4 || bytes[0] !== 0xff || bytes[1] !== 0xd8) return null;

  let offset = 2;
  while (offset + 3 < bytes.length) {
    if (bytes[offset] !== 0xff) {
      offset += 1;
      continue;
    }
    while (offset < bytes.length && bytes[offset] === 0xff) offset += 1;
    if (offset >= bytes.length) break;
    const marker = bytes[offset++];
    if (marker === 0xd9 || marker === 0xda) break;
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) continue;
    if (offset + 1 >= bytes.length) break;
    const segmentLength = bytes.readUInt16BE(offset);
    if (segmentLength < 2 || offset + segmentLength > bytes.length) break;
    const isStartOfFrame = [
      0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7,
      0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf,
    ].includes(marker);
    if (isStartOfFrame && segmentLength >= 7) {
      return {
        height: bytes.readUInt16BE(offset + 3),
        width: bytes.readUInt16BE(offset + 5),
      };
    }
    offset += segmentLength;
  }
  return null;
}

async function startFixture() {
  const fixture = await readFile(FIXTURE_PATH);
  server = createServer((request, response) => {
    if (request.method !== 'GET' || !['/', '/index.html'].includes(request.url)) {
      response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
      response.end('Not found');
      return;
    }
    response.writeHead(200, {
      'cache-control': 'no-store',
      'content-type': 'text/html; charset=utf-8',
      'content-length': fixture.byteLength,
    });
    response.end(fixture);
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  assert(address && typeof address === 'object', 'fixture server did not expose an address');
  return `http://127.0.0.1:${address.port}/index.html`;
}

function listenForFrames() {
  cdp.on('Page.screencastFrame', (event) => {
    frameCount += 1;
    // Keep one frame only. Chromium's acknowledgement is the protocol-level
    // backpressure boundary, so this callback never creates a frame queue.
    latestFrame = {
      data: event.data,
      metadata: frameMetadata(event.metadata),
      receivedAt: performance.now(),
      sequence: frameCount,
    };
    // A screencast frame must be acknowledged before Chromium sends the next
    // one. The promise is intentionally not retained; the protocol bounds it.
    void cdp.send('Page.screencastFrameAck', { sessionId: event.sessionId }).catch((error) => {
      frameError ??= error;
    });
  });
}

async function waitForFrameAfter(sequence, timeoutMs = FRAME_TIMEOUT_MS) {
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

async function cleanup() {
  if (cdp && screencastStarted) {
    await cdp.send('Page.stopScreencast').catch(() => {});
    screencastStarted = false;
  }
  if (cdp) await cdp.detach().catch(() => {});
  if (browser) await browser.close().catch(() => {});
  if (server) {
    server.closeAllConnections?.();
    await new Promise((resolve) => server.close(() => resolve())).catch(() => {});
    server = undefined;
  }
  cdp = undefined;
  page = undefined;
  context = undefined;
  browser = undefined;
}

async function run() {
  const fixtureUrl = await startFixture();
  const { chromium } = await import('playwright');
  browser = await chromium.launch({ headless: true });
  context = await browser.newContext({
    deviceScaleFactor: 1,
    viewport: { width: WIDTH, height: HEIGHT },
  });
  page = await context.newPage();
  await page.goto(fixtureUrl, { waitUntil: 'load' });
  cdp = await context.newCDPSession(page);
  listenForFrames();

  const screencastStartedAt = performance.now();
  await cdp.send('Page.startScreencast', {
    everyNthFrame: 1,
    format: 'jpeg',
    maxHeight: HEIGHT,
    maxWidth: WIDTH,
    quality: JPEG_QUALITY,
  });
  screencastStarted = true;

  const first = await waitForFrameAfter(0);
  const firstFrameLatencyMs = first.receivedAt - screencastStartedAt;
  const firstDimensions = jpegDimensions(first.data);
  const firstMetadata = frameMetadata(first.metadata);
  assert(first.data.length > 0, 'first screencast frame has an empty payload');
  assert(firstDimensions?.width === WIDTH && firstDimensions?.height === HEIGHT,
    `first JPEG dimensions were ${JSON.stringify(firstDimensions)}, expected ${WIDTH}x${HEIGHT}`);
  assert(firstMetadata.deviceWidth === WIDTH && firstMetadata.deviceHeight === HEIGHT,
    `first frame metadata dimensions were ${firstMetadata.deviceWidth}x${firstMetadata.deviceHeight}`);
  assert(Number.isFinite(firstMetadata.timestamp), 'first frame metadata has no timestamp');

  const inputStartedAt = performance.now();
  await page.locator('#name').fill('Ada');
  await page.getByRole('button', { name: 'Greet' }).click();
  await page.waitForFunction(() => document.querySelector('#result')?.dataset.changed === 'true');
  const greeting = await page.locator('#result').textContent();
  assert(greeting?.includes('Hello, Ada!'), `greeting state did not update: ${greeting}`);

  // Move into the long reading area before sending a real wheel event.
  await page.mouse.move(WIDTH / 2, HEIGHT - 100);
  await page.mouse.wheel(0, 480);
  await page.waitForFunction(() => window.scrollY > 0);
  const scrollY = await page.evaluate(() => window.scrollY);
  assert(scrollY > 0, `wheel input did not change scroll state: ${scrollY}`);

  const postInput = await waitForFrameAfter(first.sequence);
  const postInputFrameLatencyMs = postInput.receivedAt - inputStartedAt;
  const postDimensions = jpegDimensions(postInput.data);
  const postMetadata = frameMetadata(postInput.metadata);
  assert(postDimensions?.width === WIDTH && postDimensions?.height === HEIGHT,
    `post-input JPEG dimensions were ${JSON.stringify(postDimensions)}, expected ${WIDTH}x${HEIGHT}`);
  assert(postInput.data !== first.data, 'post-input screencast frame bytes did not change');
  assert(postMetadata.deviceWidth === WIDTH && postMetadata.deviceHeight === HEIGHT,
    `post-input frame metadata dimensions were ${postMetadata.deviceWidth}x${postMetadata.deviceHeight}`);
  assert(Number.isFinite(postMetadata.timestamp), 'post-input frame metadata has no timestamp');

  console.log(JSON.stringify({
    scope: 'CDP screencast-only smoke experiment',
    firstFrameLatencyMs: Number(firstFrameLatencyMs.toFixed(1)),
    frameCount,
    firstPayloadBytes: Buffer.from(first.data, 'base64').byteLength,
    postInputFrameLatencyMs: Number(postInputFrameLatencyMs.toFixed(1)),
    postInputPayloadBytes: Buffer.from(postInput.data, 'base64').byteLength,
    postInputFrameDiffers: true,
    greeting,
    scrollY,
  }));
}

let exitCode = 0;
try {
  await run();
} catch (error) {
  exitCode = 1;
  console.error(`screencast-smoke failed: ${error instanceof Error ? error.message : String(error)}`);
} finally {
  await cleanup();
}
process.exitCode = exitCode;
