import { randomUUID } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import readline from 'node:readline';
import { createServer } from 'node:http';

const MAX_WIDTH = 2560;
const MAX_HEIGHT = 1600;
const MAX_PIXELS = MAX_WIDTH * MAX_HEIGHT;
const MAX_JPEG = 6 * 1024 * 1024;
const HEADER_BYTES = 96;
const MAX_HANDSHAKE = 8192;
const MAX_WS_PAYLOAD = 64 * 1024;
const MAX_WS_BUFFER = MAX_WS_PAYLOAD + 14;
const WS_GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';

const START_PAGE_PATH = '/__cockpit_browser_start__';
const START_PAGE_HTML = '<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Cockpit browser ready</title><style>html,body{height:100%;margin:0}body{display:grid;place-items:center;background:#f4f6f8;color:#1d2733;font:16px system-ui,sans-serif}main{max-width:34rem;padding:2rem;text-align:center}p{color:#526170}</style></head><body><main><strong>Inline browser ready</strong><p>Enter a URL above to navigate.</p></main></body></html>';

let frameBarrier = Promise.resolve();
let context;
let browser;
let page;
let pageCdp;
let browserCdp;
let state;
let server;
let dummyServer;
let dummyUrl;

function startDummyPageServer() {
  if (dummyUrl) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const current = createServer((request, response) => {
      const pathname = new URL(request.url || '/', 'http://127.0.0.1').pathname;
      if (request.method !== 'GET' || pathname !== START_PAGE_PATH) {
        response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
        response.end('Not found');
        return;
      }
      const body = Buffer.from(START_PAGE_HTML);
      response.writeHead(200, {
        'content-type': 'text/html; charset=utf-8',
        'cache-control': 'no-store',
        'content-length': body.byteLength,
      });
      response.end(body);
    });
    dummyServer = current;
    current.once('error', (error) => {
      dummyServer = undefined;
      dummyUrl = undefined;
      reject(error);
    });
    current.listen(0, '127.0.0.1', () => {
      const address = current.address();
      if (!address || typeof address === 'string') {
        dummyServer = undefined;
        dummyUrl = undefined;
        current.close();
        reject(new Error('Unable to determine the local browser start page port'));
        return;
      }
      dummyUrl = `http://127.0.0.1:${address.port}${START_PAGE_PATH}`;
      resolve();
    });
  });
}
async function stopDummyPageServer() {
  const current = dummyServer;
  dummyServer = undefined;
  dummyUrl = undefined;
  if (!current) return;
  await new Promise((resolve) => current.close(() => resolve()));
}
async function ensureInitialPage() {
  if (page.url() !== 'about:blank') return;
  const empty = await page.evaluate(() => {
    const body = document.body;
    return !body || (!body.textContent?.trim() && body.children.length === 0);
  }).catch(() => false);
  if (!empty) return;
  await startDummyPageServer();
  await page.goto(dummyUrl, { waitUntil: 'domcontentloaded' });
}

let port;
let sockets = new Set();
let grants = new Map();
let viewGrants = new Map();
let latestFrame = null;
let pendingBlocker = null;
let observedPage = null;
let observedPageHandlers = null;
let screencastListener = null;
let WebSocketServer;
let WebSocket;

const emit = (value) => process.stdout.write(`${JSON.stringify(value)}\n`);
const metadata = () => ({
  view_id: state.viewId,
  stream_epoch: state.streamEpoch,
  metadata_sequence: ++state.metadataSequence,
});
const grantExpiry = (value) => Number.isFinite(Number(value)) ? Number(value) : Date.parse(value);

function capabilities() {
  return {
    pointer_input: 'supported', keyboard_input: 'supported', text_input: 'supported',
    composition_input: 'supported', clipboard_read: 'supported', clipboard_write: 'supported',
    dialogs: 'supported', file_chooser: 'unsupported', downloads: 'unsupported',
    permissions: 'unsupported', inspection: 'supported', capture: 'supported', drafts: 'supported',
    audio: 'unavailable',
  };
}
function documentState() {
  return {
    target_id: state.targetId,
    frame_id: state.frameId,
    document_generation: state.documentGeneration,
    frame_generation: state.frameGeneration,
  };
}
function viewportState() {
  return {
    viewport_revision: state.viewportRevision,
    css_width: state.cssWidth,
    css_height: state.cssHeight,
    visual_offset_x: 0,
    visual_offset_y: 0,
    scroll_x: state.scrollX,
    scroll_y: state.scrollY,
    visual_scale: 1,
    page_scale: 1,
    device_pixel_ratio: state.devicePixelRatio,
    geometry_fresh: true,
  };
}
function navigationState() {
  return { url: state.url, title: state.title, loading: state.loading, can_go_back: state.canGoBack, can_go_forward: state.canGoForward, requested_url: state.requestedUrl };
}
function snapshot() {
  return {
    identity: {
      association_key: state.associationKey,
      browser_incarnation: state.browserIncarnation,
      view_id: state.viewId,
      stream_epoch: state.streamEpoch,
    },
    metadata_sequence: state.metadataSequence,
    targets: state.targets,
    displayed_target_id: state.targetId,
    document: documentState(),
    viewport: viewportState(),
    navigation: navigationState(),
    cursor: state.cursor,
    focus: state.focus,
    blocker: pendingBlocker,
    capabilities: capabilities(),
    control: {
      status: state.controlled ? 'controlled' : 'observing', controller_view_id: state.controllerViewId, lease_generation: state.leaseGeneration,
      next_input_sequence: state.nextInputSequence, can_take_control: !state.controlled,
    },
    frame_grant: state.frameGrant,
  };
}
async function importPlaywright(corePath) {
  const pkg = JSON.parse(await readFile(join(corePath, 'package.json'), 'utf8'));
  const module = await import(pathToFileURL(join(corePath, pkg.module || pkg.main || 'index.js')).href);
  return module.default || module;
}
async function importWebSocket(corePath) {
  const require = createRequire(pathToFileURL(join(corePath, 'package.json')));
  const bundle = require(join(corePath, 'lib', 'utilsBundle'));
  WebSocketServer = bundle.wsServer;
  WebSocket = bundle.ws;
  if (!WebSocketServer || !WebSocket) throw new Error('playwright-core dependency does not provide ws');
}

function boundedInteger(value, fallback, maximum) {
  if (value === undefined || value === null) return fallback;
  if (!Number.isFinite(Number(value))) throw new Error('viewport dimension is not finite');
  const rounded = Math.round(Number(value));
  if (rounded < 1 || rounded > maximum) throw new Error('viewport dimension exceeds frozen bounds');
  return rounded;
}
function boundedNumber(value, fallback, maximum = Number.MAX_SAFE_INTEGER) {
  if (value === undefined || value === null) return fallback;
  const number = Number(value);
  if (!Number.isFinite(number) || number < 0 || number > maximum) throw new Error('geometry value is not finite');
  return number;
}
function nextGeneration(previous) {
  return Math.max(Date.now(), Number(previous || 0) + 1);
}
function requestedViewport(viewport) {
  if (!viewport || typeof viewport !== 'object') throw new Error('viewport is missing');
  const width = boundedInteger(viewport.css_width, 1, MAX_WIDTH);
  const height = boundedInteger(viewport.css_height, 1, MAX_HEIGHT);
  const dpr = Number(viewport.device_pixel_ratio ?? 1);
  if (!Number.isFinite(dpr) || dpr <= 0 || dpr > 16) throw new Error('device pixel ratio is not finite');
  return { width, height, dpr };
}
async function applyRequestedViewport(viewport) {
  const requested = requestedViewport(viewport);
  await pageCdp.send('Emulation.setDeviceMetricsOverride', {
    width: requested.width,
    height: requested.height,
    deviceScaleFactor: requested.dpr,
    mobile: false,
    screenWidth: requested.width,
    screenHeight: requested.height,
  });
  return requested;
}
async function updatePageState() {
  state.url = page.url();
  try { state.title = await page.title(); } catch { state.title = ''; }
  try {
    const metrics = await pageCdp.send('Page.getLayoutMetrics');
    const viewport = metrics.cssVisualViewport || metrics.visualViewport || {};
    const width = boundedInteger(viewport.clientWidth, state.cssWidth, MAX_WIDTH);
    const height = boundedInteger(viewport.clientHeight, state.cssHeight, MAX_HEIGHT);
    const changed = width !== state.cssWidth || height !== state.cssHeight;
    state.cssWidth = width;
    state.cssHeight = height;
    state.scrollX = boundedNumber(viewport.pageX, 0);
    state.scrollY = boundedNumber(viewport.pageY, 0);
    if (changed) state.viewportRevision++;
    return changed;
  } catch {
    // Keep the last known finite geometry when Chromium is between documents.
    return false;
  }
}
function emitEvent(type, fields) {
  emit({ type: 'event', event: { type, metadata: metadata(), ...fields } });
}
function emitNavigation() { emitEvent('navigation_changed', { navigation: navigationState() }); }
function emitFocus() { emitEvent('focus_changed', { focus: state.focus }); }
function emitCursor() { emitEvent('cursor_changed', { cursor: state.cursor }); }
function emitBlocker() { emitEvent('blocker_changed', { blocker: pendingBlocker }); }
function emitControl() {
  emitEvent('control_changed', { control: snapshot().control });
}
async function updateHistory() {
  try {
    const history = await pageCdp.send('Page.getNavigationHistory');
    state.canGoBack = history.currentIndex > 0;
    state.canGoForward = history.currentIndex >= 0 && history.currentIndex + 1 < history.entries.length;
  } catch { state.canGoBack = false; state.canGoForward = false; }
}
async function updateFrameId() {
  try { state.frameId = (await pageCdp.send('Page.getFrameTree')).frameTree.frame.id; } catch { state.frameId = 'main'; }
}
async function updateFocus() {
  try {
    state.focus = await page.evaluate(() => {
      const active = document.activeElement;
      const editable = !!active && (active.matches('input, textarea, select') || active.isContentEditable);
      return { page_focused: document.hasFocus(), editable, selection_available: !!window.getSelection()?.toString(), composition_active: false };
    });
  } catch { state.focus = { page_focused: false, editable: false, selection_available: false, composition_active: false }; }
}
function normalizedCursor(value) {
  const cursors = new Set(['default', 'pointer', 'text', 'crosshair', 'move', 'not_allowed', 'wait', 'grab', 'grabbing', 'cell', 'help', 'progress', 'zoom_in', 'zoom_out', 'col_resize', 'row_resize', 'e_resize', 'w_resize', 'n_resize', 's_resize', 'ne_resize', 'nw_resize', 'se_resize', 'sw_resize']);
  if (!cursors.has(value)) return 'default';
  return ({ col_resize: 'column_resize', row_resize: 'row_resize', e_resize: 'east_resize', w_resize: 'west_resize', n_resize: 'north_resize', s_resize: 'south_resize', ne_resize: 'northeast_resize', nw_resize: 'northwest_resize', se_resize: 'southeast_resize', sw_resize: 'southwest_resize' })[value] || value;
}
async function updateCursor() {
  if (!state.pointer) return;
  try {
    const cursor = normalizedCursor(await page.evaluate(({ x, y }) => {
      const element = document.elementFromPoint(x, y);
      return element ? getComputedStyle(element).cursor : 'default';
    }, state.pointer));
    const next = { cursor, pointer_sample_sequence: state.pointerSampleSequence, target_id: state.targetId, document_generation: state.documentGeneration, viewport_revision: state.viewportRevision };
    if (JSON.stringify(next) !== JSON.stringify(state.cursor)) { state.cursor = next; emitCursor(); }
  } catch {}
}
function setBlocker(kind, message, defaultPrompt, resolve) {
  if (pendingBlocker?.resolve) pendingBlocker.resolve('dismiss');
  pendingBlocker = { blocker_id: randomUUID(), kind, message: String(message || '').slice(0, 4096), default_prompt: defaultPrompt ? String(defaultPrompt).slice(0, 4096) : null, target_id: state.targetId, document_generation: state.documentGeneration, cancellable: true, resolve };
  emitBlocker();
  return pendingBlocker;
}
function clearBlocker(id) {
  if (!pendingBlocker || pendingBlocker.blocker_id !== id) return false;
  pendingBlocker = null;
  emitBlocker();
  return true;
}
function commandInputSequence(command) {
  const input = command.input;
  return input && Number.isSafeInteger(input.input_sequence) ? input.input_sequence : null;
}
function requireControl(command) {
  const sequence = commandInputSequence(command);
  if (!state.controlled) throw Object.assign(new Error('Browser input control is not held by this view'), { code: 'browser_control_required' });
  if (sequence !== null && sequence !== state.nextInputSequence) throw Object.assign(new Error('Browser input sequence is stale'), { code: 'stale_input_sequence' });
  return sequence;
}
function advanceInput(sequence) { if (sequence !== null) state.nextInputSequence = sequence + 1; }
async function releaseHeldInput() {
  const buttons = state?.pressedButtons || 0;
  const buttonNames = [[1, 'left'], [2, 'right'], [4, 'middle']];
  for (const [mask, button] of buttonNames) {
    if (!(buttons & mask)) continue;
    try {
      await pageCdp?.send('Input.dispatchMouseEvent', {
        type: 'mouseReleased',
        x: state.pointer?.x || 0,
        y: state.pointer?.y || 0,
        button,
        buttons: 0,
        clickCount: 1,
        modifiers: 0,
      });
    } catch {}
  }
  for (const input of state?.heldKeys?.values() || []) {
    try {
      await pageCdp?.send('Input.dispatchKeyEvent', {
        type: 'keyUp',
        key: input.key,
        code: input.code,
        location: input.location,
        modifiers: input.modifiers,
        autoRepeat: false,
      });
    } catch {}
  }
  if (state) {
    state.pressedButtons = 0;
    state.heldKeys.clear();
  }
}
async function enumerateTargets() {
  const all = (await browserCdp.send('Target.getTargets')).targetInfos;
  state.targets = all
    .filter((target) => ['page', 'background_page', 'service_worker'].includes(target.type))
    .map((target, order) => ({
      target_id: target.targetId,
      kind: target.type === 'page' ? 'page' : 'background',
      title: target.title || '', url: target.url || '', order,
      opener_target_id: target.openerId || null, can_close: target.type === 'page',
    }));
  if (!state.targets.some((target) => target.target_id === state.targetId)) throw new Error('attached browser target no longer exists');
  const active = state.targets.find((target) => target.target_id === state.targetId);
  state.url = active?.url || state.url;
  state.title = active?.title || state.title;
}

async function pageForTarget(targetId) {
  for (const candidate of context?.pages() || []) {
    const candidateCdp = await context.newCDPSession(candidate);
    try {
      if ((await candidateCdp.send('Target.getTargetInfo')).targetInfo?.targetId === targetId) return { page: candidate, cdp: candidateCdp };
    } catch {}
    try { await candidateCdp.detach(); } catch {}
  }
  throw new Error('stable CDP target is not an attachable page');
}

async function bindPage(targetId, restartScreencast = true) {
  const previousTarget = state?.targetId;
  if (restartScreencast) try { await pageCdp?.send('Page.stopScreencast'); } catch {}
  if (observedPage && observedPageHandlers) {
    for (const [event, handler] of observedPageHandlers) observedPage.off?.(event, handler);
  }
  if (screencastListener && pageCdp) pageCdp.off?.('Page.screencastFrame', screencastListener);
  observedPage = null;
  observedPageHandlers = null;
  screencastListener = null;
  try { await pageCdp?.detach(); } catch {}
  const selected = await pageForTarget(targetId);
  page = selected.page;
  pageCdp = selected.cdp;
  state.targetId = targetId;
  if (previousTarget && previousTarget !== targetId) {
    state.documentGeneration = nextGeneration(state.documentGeneration);
    state.frameGeneration = nextGeneration(state.frameGeneration);
    state.viewportRevision++;
    state.cursor = null;
    latestFrame = null;
  }
  if (!restartScreencast) await ensureInitialPage();

  await installPageObservers();
  await applyRequestedViewport({ css_width: state.cssWidth, css_height: state.cssHeight, device_pixel_ratio: state.devicePixelRatio });
  await updatePageState();
  await updateFrameId();
  await updateHistory();
  if (previousTarget && previousTarget !== targetId) {
    emitEvent('document_changed', { document: documentState() });
    emitNavigation();
  }
  if (restartScreencast) await startScreencast();
}

async function refreshTargets() {
  await enumerateTargets();
  emitEvent('targets_changed', { targets: state.targets, displayed_target_id: state.targetId });
}

function envelope(descriptor, jpeg) {
  const h = Buffer.alloc(HEADER_BYTES);
  h.writeUInt32BE(0x49424656, 0);
  h.writeUInt16BE(2, 4);
  h.writeUInt16BE(HEADER_BYTES, 6);
  h.writeBigUInt64BE(BigInt(descriptor.stream_epoch), 8);
  h.writeBigUInt64BE(BigInt(descriptor.frame_sequence), 16);
  h.writeBigUInt64BE(BigInt(descriptor.document_generation), 24);
  h.writeBigUInt64BE(BigInt(descriptor.viewport_revision), 32);
  h.writeUInt32BE(descriptor.image_width, 40);
  h.writeUInt32BE(descriptor.image_height, 44);
  h.writeFloatBE(descriptor.viewport_css_width, 48);
  h.writeFloatBE(descriptor.viewport_css_height, 52);
  h.writeFloatBE(descriptor.viewport_offset_x, 56);
  h.writeFloatBE(descriptor.viewport_offset_y, 60);
  h.writeFloatBE(descriptor.scroll_x, 64);
  h.writeFloatBE(descriptor.scroll_y, 68);
  h.writeBigUInt64BE(BigInt(descriptor.capture_timestamp_micros), 72);
  h.writeUInt32BE(descriptor.jpeg_length, 80);
  h.writeUInt32BE(0, 84);
  return Buffer.concat([h, jpeg]);
}
function ackSession(sessionId, cdp = pageCdp) {
  if (sessionId !== undefined) cdp?.send('Page.screencastFrameAck', { sessionId }).catch(() => {});
}
function acknowledgeFrame(frame) {
  if (!frame || frame.acknowledged || frame.references > 0) return;
  frame.acknowledged = true;
  ackSession(frame.sessionId, frame.cdp);
}
function retainFrame(frame) {
  if (frame && !frame.acknowledged) frame.references++;
}
function releaseFrame(frame) {
  if (!frame || frame.references <= 0) return;
  frame.references--;
  acknowledgeFrame(frame);
}
function forgetSocketFrame(socket, slot) {
  const frame = socket[slot];
  if (!frame) return;
  socket[slot] = undefined;
  if (slot === 'awaitingFrame') socket.frameSequence = undefined;
  releaseFrame(frame);
}
function writeFrame(socket, frame, retained = false) {
  if (!socket.authorized || socket.readyState !== WebSocket.OPEN) return false;
  socket.awaitingFrame = frame;
  socket.frameSequence = frame.descriptor.frame_sequence;
  if (!retained) retainFrame(frame);
  try {
    socket.send(frame.payload, { binary: true });
    return true;
  } catch {
    forgetSocketFrame(socket, 'awaitingFrame');
    socket.close(1011, 'frame transport failed');
    return false;
  }
}
function queueFrame(socket, frame) {
  if (!socket.authorized || socket.readyState !== WebSocket.OPEN) return false;
  if (socket.awaitingFrame) {
    if (socket.pendingFrame) forgetSocketFrame(socket, 'pendingFrame');
    socket.pendingFrame = frame;
    retainFrame(frame);
    return true;
  }
  return writeFrame(socket, frame);
}
function releaseSocketFrame(socket, sequence) {
  const frame = socket.awaitingFrame;
  if (!frame || frame.descriptor.frame_sequence !== sequence) return false;
  forgetSocketFrame(socket, 'awaitingFrame');
  const pending = socket.pendingFrame;
  socket.pendingFrame = undefined;
  if (pending) {
    if (socket.authorized && socket.readyState === WebSocket.OPEN) {
      releaseFrame(pending);
      writeFrame(socket, pending);
    } else {
      releaseFrame(pending);
    }
  }
  return true;
}
function cleanupSocket(socket) {
  socket.authorized = false;
  forgetSocketFrame(socket, 'awaitingFrame');
  forgetSocketFrame(socket, 'pendingFrame');
  socket.frameSequence = undefined;
  sockets.delete(socket);
}
function resetFrameTransport() {
  latestFrame = null;
  for (const socket of sockets) {
    forgetSocketFrame(socket, 'awaitingFrame');
    forgetSocketFrame(socket, 'pendingFrame');
  }
}
function enqueueFrame(frame) {
  let next;
  try {
    if (!frame || typeof frame.data !== 'string') throw new Error('screencast payload is not base64 text');
    const jpeg = Buffer.from(frame.data, 'base64');
    const rawMetadata = frame.metadata && typeof frame.metadata === 'object' ? frame.metadata : {};
    const width = boundedInteger(rawMetadata.deviceWidth, state.cssWidth, MAX_WIDTH);
    const height = boundedInteger(rawMetadata.deviceHeight, state.cssHeight, MAX_HEIGHT);
    if (!jpeg.length || jpeg.length > MAX_JPEG || jpeg[0] !== 0xff || jpeg[1] !== 0xd8
      || jpeg[jpeg.length - 2] !== 0xff || jpeg[jpeg.length - 1] !== 0xd9
      || width * height > MAX_PIXELS) {
      throw new Error('screencast frame exceeds frozen bounds');
    }
    const scrollX = boundedNumber(rawMetadata.scrollOffsetX, state.scrollX);
    const scrollY = boundedNumber(rawMetadata.scrollOffsetY, state.scrollY);
    const timestamp = boundedNumber(rawMetadata.timestamp, Date.now() / 1000, Number.MAX_SAFE_INTEGER / 1_000_000);
    const descriptor = {
      target_id: state.targetId,
      stream_epoch: state.streamEpoch,
      frame_sequence: ++state.frameSequence,
      document_generation: state.documentGeneration,
      viewport_revision: state.viewportRevision,
      image_width: width,
      image_height: height,
      viewport_css_width: state.cssWidth,
      viewport_css_height: state.cssHeight,
      viewport_offset_x: 0,
      viewport_offset_y: 0,
      scroll_x: scrollX,
      scroll_y: scrollY,
      capture_timestamp_micros: Math.floor(timestamp * 1_000_000),
      jpeg_length: jpeg.length,
    };
    next = {
      descriptor,
      sessionId: frame.sessionId,
      cdp: frame._cdp || pageCdp,
      payload: envelope(descriptor, jpeg),
      references: 0,
      acknowledged: false,
    };
    emit({ type: 'frame', descriptor });
  } catch (error) {
    emit({ type: 'failed', code: 'browser_frame_invalid', message: String(error.message || error) });
    ackSession(frame?.sessionId, frame?.cdp);
    return;
  }
  latestFrame = next;
  let delivered = false;
  for (const socket of sockets) {
    if (!socket.authorized || socket.readyState !== WebSocket.OPEN) continue;
    if (queueFrame(socket, next)) delivered = true;
  }
  if (!delivered) acknowledgeFrame(next);
}

function startFrameServer() {
  return new Promise((resolve, reject) => {
    server = new WebSocketServer({ host: '127.0.0.1', port: 0, maxPayload: MAX_WS_PAYLOAD, perMessageDeflate: false });
    server.once('listening', () => { port = server.address().port; resolve(); });
    server.on('connection', (socket, request) => {
      if (request.headers.origin && request.headers.origin !== `http://127.0.0.1:${port}`) { socket.close(1008, 'unexpected origin'); return; }
      socket.authorized = false;
      sockets.add(socket);
      socket.on('close', () => cleanupSocket(socket));
      socket.on('error', () => cleanupSocket(socket));
      socket.on('message', (payload, binary) => {
        if (binary || payload.length > MAX_WS_PAYLOAD) { socket.close(1003, 'text control required'); return; }
        let value; try { value = JSON.parse(payload.toString('utf8')); } catch { socket.close(1003, 'invalid control'); return; }
        if (!socket.authorized) {
          const grant = typeof value?.grant === 'string' ? value.grant : '';
          const expiry = grants.get(grant);
          if (!grant || !Number.isFinite(expiry) || expiry < Date.now()) { socket.close(1008, 'invalid grant'); return; }
          grants.delete(grant); socket.authorized = true; if (latestFrame) queueFrame(socket, latestFrame); return;
        }
        if (!value || !['ack', 'discard'].includes(value.type) || !Number.isSafeInteger(value.frame_sequence) || !releaseSocketFrame(socket, value.frame_sequence)) socket.close(1008, 'invalid credit');
      });
    });
  });
}

function proofMatches(command) {
  if (!command || typeof command !== 'object') return false;
  const proof = command.location || command.context;
  if (proof !== undefined) {
    if (!proof || typeof proof !== 'object' || proof.target_id !== state.targetId
      || proof.document_generation !== state.documentGeneration) return false;
    if (proof.viewport_revision !== undefined && proof.viewport_revision !== state.viewportRevision) return false;
    if (proof.presented_frame_sequence !== undefined && proof.presented_frame_sequence !== state.frameSequence) return false;
    if (proof.lease_generation !== undefined && proof.lease_generation !== state.leaseGeneration) return false;
  }
  if (command.type === 'tab' && command.command && command.command.target_id !== undefined
    && command.command.target_id !== state.targetId) return false;
  return true;
}

function validateNavigationUrl(value) {
  if (typeof value !== 'string' || value.length > 8 * 1024 || !value.trim()) {
    throw new Error('browser URL must be an absolute http, https, or about URL');
  }
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error('browser URL must be an absolute http, https, or about URL');
  }
  if (!['http:', 'https:', 'about:'].includes(parsed.protocol)
    || (['http:', 'https:'].includes(parsed.protocol) && !parsed.hostname)) {
    throw new Error('browser URL has an unsupported or unsafe scheme');
  }
  if (parsed.username || parsed.password) throw new Error('browser URL must not contain userinfo');
  return parsed.href;
}
async function startScreencast() {
  await pageCdp.send('Page.startScreencast', {
    format: 'jpeg', quality: 80, maxWidth: state.cssWidth, maxHeight: state.cssHeight, everyNthFrame: 1,
  });
}
async function installPageObservers() {
  if (observedPage && observedPageHandlers) {
    for (const [event, handler] of observedPageHandlers) observedPage.off?.(event, handler);
  }
  if (screencastListener && pageCdp) pageCdp.off?.('Page.screencastFrame', screencastListener);
  const observed = page;
  const cdp = pageCdp;
  const onFrameNavigated = (frame) => {
    if (frame !== observed.mainFrame()) return;
    state.documentGeneration = nextGeneration(state.documentGeneration);
    state.frameGeneration = nextGeneration(state.frameGeneration);
    resetFrameTransport();
    void cdp.send('Page.stopScreencast').catch(() => {});
    frameBarrier = frameBarrier.then(async () => {
      const viewportChanged = await updatePageState();
      await updateFrameId();
      await updateHistory();
      if (viewportChanged) emitEvent('viewport_changed', { viewport: viewportState() });
      emitEvent('document_changed', { document: documentState() });
      emitNavigation();
      await startScreencast();
    });
  };
  const onLoad = () => { void updatePageState().then(updateHistory).then(emitNavigation).catch(() => {}); };
  const onDialog = (dialog) => {
    const blocker = setBlocker('dialog', dialog.message(), dialog.defaultValue?.(), async (decision, text) => {
      if (decision === 'accept') await dialog.accept(text);
      else await dialog.dismiss();
    });
    void dialog.type().then((type) => { blocker.message = `${type}: ${blocker.message}`; }).catch(() => {});
  };
  const onFileChooser = (chooser) => {
    const blocker = setBlocker('file_chooser', 'File upload requires a local file-selection adapter', null, async () => chooser.setFiles([]));
    blocker.cancellable = true;
  };
  const onDownload = (download) => {
    setBlocker('download', `Download requested: ${download.suggestedFilename()}`, null, async () => {});
  };
  observedPageHandlers = [
    ['framenavigated', onFrameNavigated],
    ['load', onLoad],
    ['dialog', onDialog],
    ['filechooser', onFileChooser],
    ['download', onDownload],
  ];
  for (const [event, handler] of observedPageHandlers) observed.on(event, handler);
  observedPage = observed;
  screencastListener = (frame) => {
    frame._cdp = cdp;
    const barrier = frameBarrier;
    void barrier.then(() => enqueueFrame(frame)).catch(() => cdp.send('Page.screencastFrameAck', { sessionId: frame.sessionId }).catch(() => {}));
  };
  cdp.on('Page.screencastFrame', screencastListener);
}
async function attach(message) {
  const { chromium } = await importPlaywright(message.playwright_core);
  await importWebSocket(message.playwright_core);
  browser = await chromium.connectOverCDP(message.cdp_endpoint);
  context = browser.contexts()[0];
  browserCdp = await browser.newBrowserCDPSession();
  const viewport = requestedViewport(message.viewport);
  state = {
    associationKey: message.association_key, browserIncarnation: message.browser_incarnation,
    viewId: message.view_id, streamEpoch: message.stream_epoch, metadataSequence: 0,
    documentGeneration: nextGeneration(), frameGeneration: nextGeneration(), viewportRevision: 1, frameSequence: 0,
    leaseGeneration: 1, nextInputSequence: 1, cssWidth: viewport.width, cssHeight: viewport.height,
    devicePixelRatio: viewport.dpr, scrollX: 0, scrollY: 0, targets: [], targetId: message.target_id,
    frameId: 'main', url: '', title: '', frameGrant: message.frame_grant, loading: false,
    canGoBack: false, canGoForward: false, requestedUrl: null, controlled: false,
    focus: { page_focused: false, editable: false, selection_available: false, composition_active: false },
    cursor: null, pointer: null, pointerSampleSequence: 0, viewIds: new Set([message.view_id]), controllerViewId: null,
    pressedButtons: 0, heldKeys: new Map(),
  };
  if (typeof message.target_id !== 'string' || !message.target_id) throw new Error('stable CDP target identity is required');
  viewGrants.set(message.view_id, message.frame_grant.grant);
  grants.set(message.frame_grant.grant, grantExpiry(message.frame_grant.expires_at));
  await startFrameServer();
  await bindPage(message.target_id, false);
  await enumerateTargets();
  browserCdp.on('Target.targetCreated', () => { void refreshTargets().catch(() => {}); });
  browserCdp.on('Target.targetDestroyed', () => { void refreshTargets().catch(() => {}); });
  browserCdp.on('Target.targetInfoChanged', () => { void refreshTargets().catch(() => {}); });
  await startScreencast();
  emit({
    type: 'ready',
    frame_endpoint: `ws://127.0.0.1:${port}`,
    snapshot: snapshot(),
  });
}
async function command(request) {
  const base = { view_id: request.view_id, stream_epoch: state.streamEpoch, request_id: request.request_id };
  if (!state.viewIds.has(request.view_id) || request.stream_epoch !== state.streamEpoch || !proofMatches(request.command)) {
    return {
      status: 'stale', ...base, current_stream_epoch: state.streamEpoch,
      current_metadata_sequence: state.metadataSequence, code: 'stale_location',
      message: 'target, document, viewport, frame, or lease proof changed',
    };
  }
  try {
    if (['resize', 'navigation', 'pointer', 'wheel', 'keyboard', 'text', 'composition', 'clipboard'].includes(request.command.type) && state.controllerViewId !== request.view_id) {
      return { status: 'rejected', ...base, code: 'browser_control_required', message: 'Another browser view holds the input lease' };
    }
    if (request.command.type === 'take_control') {
      if (state.controlled && state.controllerViewId !== request.view_id) {
        return { status: 'rejected', ...base, code: 'browser_control_required', message: 'Another browser view holds the input lease' };
      }
      await applyRequestedViewport(request.command.viewport);
      state.controlled = true;
      state.controllerViewId = request.view_id;
      state.leaseGeneration++;
      state.nextInputSequence = 1;
      emitControl();
      return { status: 'accepted', ...base, outcome: { type: 'control', control: snapshot().control } };
    }
    if (request.command.type === 'release_control') {
      if (request.command.lease_generation !== state.leaseGeneration || state.controllerViewId !== request.view_id) {
        throw Object.assign(new Error('stale control lease'), { code: 'stale_control' });
      }
      await releaseHeldInput();
      state.controlled = false;
      state.controllerViewId = null;
      state.leaseGeneration++;
      emitControl();
      return { status: 'accepted', ...base, outcome: { type: 'control', control: snapshot().control } };
    }
    if (request.command.type === 'resize') { requireControl(request.command); await applyRequestedViewport(request.command.viewport); state.viewportRevision++; await updatePageState(); emitEvent('viewport_changed', { viewport: viewportState() }); return { status: 'accepted', ...base, outcome: { type: 'none' } }; }
    if (request.command.type === 'navigation') {
      requireControl(request.command);
      const action = request.command.command;
      let url;
      if (action.type === 'navigate') {
        try {
          url = validateNavigationUrl(action.url);
        } catch (error) {
          return { status: 'rejected', ...base, code: 'invalid_browser_url', message: error.message };
        }
      }
      state.loading = true;
      emitNavigation();
      if (action.type === 'navigate') await page.goto(url);
      else if (action.type === 'back') await page.goBack();
      else if (action.type === 'forward') await page.goForward();
      else if (action.type === 'reload') await page.reload();
      else if (action.type === 'stop') await pageCdp.send('Page.stopLoading');
      else return { status: 'unsupported', ...base, capability: 'navigation', message: 'Navigation command is not implemented' };
      await frameBarrier;
      const viewportChanged = await updatePageState();
      await updateHistory(); state.loading = false;
      if (viewportChanged) emitEvent('viewport_changed', { viewport: viewportState() });
      emitNavigation();
      return { status: 'accepted', ...base, outcome: { type: 'none' } };
    }
    if (request.command.type === 'pointer') {
      const sequence = requireControl(request.command);
      const i = request.command.input;
      await pageCdp.send('Input.dispatchMouseEvent', {
        type: i.kind === 'down' ? 'mousePressed' : i.kind === 'up' || i.kind === 'cancel' ? 'mouseReleased' : 'mouseMoved',
        x: i.x, y: i.y, button: i.button || 'none', buttons: i.buttons,
        clickCount: i.click_count, modifiers: i.modifiers,
      });
      advanceInput(sequence);
      state.pointer = { x: i.x, y: i.y };
      state.pressedButtons = i.buttons;
      state.pointerSampleSequence++;
      await updateCursor();
      return { status: 'accepted', ...base, outcome: { type: 'none' } };
    }
    if (request.command.type === 'wheel') { const sequence = requireControl(request.command); const i = request.command.input; await pageCdp.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: i.x, y: i.y, deltaX: i.delta_x_css, deltaY: i.delta_y_css, modifiers: i.modifiers }); advanceInput(sequence); await updatePageState(); emitEvent('viewport_changed', { viewport: viewportState() }); return { status: 'accepted', ...base, outcome: { type: 'none' } }; }
    if (request.command.type === 'keyboard') {
      const sequence = requireControl(request.command);
      const i = request.command.input;
      await pageCdp.send('Input.dispatchKeyEvent', {
        type: i.kind === 'down' ? 'keyDown' : 'keyUp',
        key: i.key,
        code: i.code,
        location: i.location,
        modifiers: i.modifiers,
        autoRepeat: i.repeat,
      });
      if (i.kind === 'down') state.heldKeys.set(i.code, i);
      else state.heldKeys.delete(i.code);
      advanceInput(sequence);
      await updateFocus();
      emitFocus();
      return { status: 'accepted', ...base, outcome: { type: 'none' } };
    }
    if (request.command.type === 'composition') { const sequence = requireControl(request.command); const i = request.command.input; if (i.kind === 'commit') await pageCdp.send('Input.insertText', { text: i.text }); else await pageCdp.send('Input.imeSetComposition', { text: i.kind === 'cancel' ? '' : i.text, selectionStart: i.text.length, selectionEnd: i.text.length, replacementStart: 0, replacementEnd: 0 }); state.focus.composition_active = i.kind === 'start' || i.kind === 'update'; advanceInput(sequence); emitFocus(); return { status: 'accepted', ...base, outcome: { type: 'none' } }; }
    if (request.command.type === 'clipboard') { const sequence = requireControl(request.command); if (request.command.command.type === 'paste') { await pageCdp.send('Input.insertText', { text: request.command.command.text }); advanceInput(sequence); return { status: 'accepted', ...base, outcome: { type: 'clipboard', text: null } }; } const text = await page.evaluate(() => window.getSelection()?.toString().slice(0, 16384) || ''); advanceInput(sequence); return { status: 'accepted', ...base, outcome: { type: 'clipboard', text } }; }
    if (request.command.type === 'tab') { const action = request.command.command; if (action.type === 'select') await bindPage(action.target_id); else if (action.type === 'create') { const created = await context.newPage(); if (action.url) await created.goto(validateNavigationUrl(action.url)); const cdp = await context.newCDPSession(created); const targetId = (await cdp.send('Target.getTargetInfo')).targetInfo?.targetId; await cdp.detach(); if (!targetId) throw new Error('created tab has no target'); await bindPage(targetId); } else if (action.type === 'close') { if (action.target_id === state.targetId) return { status: 'rejected', ...base, code: 'browser_selected_tab', message: 'Select another tab before closing this tab' }; const closing = await pageForTarget(action.target_id); await closing.page.close(); } await refreshTargets(); return { status: 'accepted', ...base, outcome: { type: 'snapshot', snapshot: snapshot() } }; }
    if (request.command.type === 'dialog') {
      if (!pendingBlocker || pendingBlocker.blocker_id !== request.command.blocker_id || pendingBlocker.kind !== 'dialog') {
        throw Object.assign(new Error('stale dialog'), { code: 'stale_dialog' });
      }
      const blocker = pendingBlocker;
      await blocker.resolve(request.command.command.type, request.command.command.text);
      clearBlocker(blocker.blocker_id);
      return { status: 'accepted', ...base, outcome: { type: 'none' } };
    }
    if (['file', 'download', 'permission'].includes(request.command.type)) {
      const blocker = pendingBlocker;
      if (blocker && blocker.blocker_id === request.command.blocker_id) {
        if (request.command.type === 'file' || request.command.type === 'download') await blocker.resolve('dismiss');
        clearBlocker(blocker.blocker_id);
      }
      return { status: 'unsupported', ...base, capability: request.command.type, message: `Browser ${request.command.type} handling is unavailable` };
    }
    if (request.command.type === 'inspect') {
      const i = request.command.command;
      if (i.pointer_sample_sequence !== state.pointerSampleSequence) {
        throw Object.assign(new Error('stale pointer'), { code: 'stale_pointer' });
      }
      const result = await page.evaluate(({ x, y }) => {
        const e = document.elementFromPoint(x, y);
        if (!e) return null;
        const r = e.getBoundingClientRect();
        return {
          bounds: { x: r.x, y: r.y, width: r.width, height: r.height },
          evidence: {
            tag: e.localName,
            text: (e.innerText || e.textContent || '').trim().slice(0, 1024),
            role: e.getAttribute('role'),
            name: e.getAttribute('aria-label') || e.getAttribute('name'),
            locators: [e.id ? `#${e.id}` : e.localName],
            excerpt: e.outerHTML.slice(0, 1024),
          },
        };
      }, i);
      return {
        status: 'accepted',
        ...base,
        outcome: {
          type: 'inspection',
          inspection: {
            location: i.location,
            frame_id: state.frameId,
            frame_generation: state.frameGeneration,
            pointer_sample_sequence: state.pointerSampleSequence,
            bounds: result?.bounds || null,
            evidence: result?.evidence || null,
            inspectable: !!result,
            freshness: result ? 'fresh' : 'unavailable',
            limitation: result ? null : 'No page element is present at this point',
          },
        },
      };
    }
    if (request.command.type === 'capture') {
      if (!latestFrame
        || latestFrame.descriptor.target_id !== state.targetId
        || latestFrame.descriptor.document_generation !== state.documentGeneration
        || latestFrame.descriptor.frame_sequence !== request.command.command.location.presented_frame_sequence) {
        throw Object.assign(new Error('stale capture frame'), { code: 'stale_capture' });
      }
      return { status: 'accepted', ...base, outcome: { type: 'capture_prepared', capture_id: randomUUID(), descriptor: latestFrame.descriptor } };
    }
    return { status: 'unsupported', ...base, capability: request.command.type, message: 'This browser-view command is not implemented by the inline helper' };
  } catch (error) {
    if (request.command.type === 'navigation' && state.loading) {
      state.loading = false;
      emitNavigation();
    }
    const code = error?.code;
    if (['browser_control_required', 'stale_input_sequence', 'stale_control', 'stale_dialog', 'stale_pointer', 'stale_capture'].includes(code)) {
      return { status: 'rejected', ...base, code, message: String(error.message || error) };
    }
    return { status: 'outcome_unknown', ...base, code: 'browser_command_uncertain', message: String(error.message || error) };
  }
}
async function detach() {
  await releaseHeldInput();
  try { await pageCdp?.send('Page.stopScreencast'); } catch {}
  grants.clear();
  viewGrants.clear();
  for (const socket of sockets) socket.terminate();
  sockets.clear();
  server?.close();
  await stopDummyPageServer();
  try { await browser?.disconnect(); } catch {}
  process.exit(0);
}
const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
let inputQueue = Promise.resolve();
async function processInputLine(line) {
  try {
    const message = JSON.parse(line);
    if (message.type === 'attach') await attach(message);
    else if (message.type === 'command') emit({ type: 'command_response', response: await command(message.request) });
    else if (message.type === 'grant') {
      grants.set(message.grant.grant, grantExpiry(message.grant.expires_at));
      viewGrants.set(message.grant.view_id, message.grant.grant);
    } else if (message.type === 'attach_view') {
      const previous = viewGrants.get(message.view_id);
      if (previous) grants.delete(previous);
      state.viewIds.add(message.view_id);
      viewGrants.set(message.view_id, message.frame_grant.grant);
      grants.set(message.frame_grant.grant, grantExpiry(message.frame_grant.expires_at));
    } else if (message.type === 'detach_view') {
      state.viewIds.delete(message.view_id);
      const grant = viewGrants.get(message.view_id);
      if (grant) grants.delete(grant);
      viewGrants.delete(message.view_id);
      if (state.controllerViewId === message.view_id) {
        await releaseHeldInput();
        state.controlled = false;
        state.controllerViewId = null;
        state.leaseGeneration++;
        emitControl();
      }
    } else if (message.type === 'pause') {
      await releaseHeldInput();
      if (state.controlled) {
        state.controlled = false;
        state.controllerViewId = null;
        state.leaseGeneration++;
        state.nextInputSequence = 1;
        emitControl();
      }
      for (const grant of viewGrants.values()) grants.delete(grant);
      viewGrants.clear();
      state.viewIds.clear();
      await pageCdp?.send('Page.stopScreencast');
    }
    else if (message.type === 'resume') await startScreencast();
    else if (message.type === 'detach') await detach();
    else if (message.type === 'stop') await detach();
  } catch (error) {
    emit({ type: 'failed', code: 'browser_helper_failed', message: String(error.message || error) });
  }
}
input.on('line', (line) => {
  inputQueue = inputQueue.then(() => processInputLine(line), () => processInputLine(line));
});
process.on('uncaughtException', (error) => {
  emit({ type: 'failed', code: 'browser_helper_crashed', message: String(error.message || error) });
  process.exit(1);
});
