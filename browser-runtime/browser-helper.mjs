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
const FRAME_INTERVAL_MS = 1000 / 30;
const MAX_FRAME_PEERS = 32;
const PREAUTH_TIMEOUT_MS = 5000;

const START_PAGE_PATH = '/__cockpit_browser_start__';
const START_PAGE_HTML = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Cockpit browser ready</title>
<style>
:root{color-scheme:dark}*{box-sizing:border-box}html,body{height:100%;margin:0}body{display:grid;place-items:center;overflow:hidden;background:radial-gradient(circle at 20% 15%,#24324b 0,transparent 34%),radial-gradient(circle at 80% 88%,#142e35 0,transparent 35%),#0b1018;color:#e7edf7;font:15px/1.5 ui-sans-serif,system-ui,sans-serif}main{width:min(31rem,calc(100% - 3rem));padding:2rem 2.25rem;border:1px solid #34445e;border-radius:16px;background:linear-gradient(145deg,#182233eF,#111925eF);box-shadow:0 24px 72px #02050acc;text-align:center}main::before{display:grid;width:42px;height:42px;margin:0 auto 1.15rem;place-items:center;border:1px solid #6387b9;border-radius:12px;background:#1c314b;color:#a9cdfb;content:'↗';font-size:22px}strong{display:block;font-size:18px;font-weight:650;letter-spacing:-.01em}p{margin:8px 0 0;color:#aab9cc}kbd{display:inline-block;margin-top:1.35rem;padding:4px 8px;border:1px solid #3a4d68;border-radius:6px;background:#0b111b;color:#c6ddfb;font:12px ui-monospace,SFMono-Regular,monospace}</style></head>
<body><main><strong>Browser ready</strong><p>Enter a URL in the address bar to begin browsing.</p><kbd>https://example.com</kbd></main></body></html>`;

let frameBarrier = Promise.resolve();
let frameGeometryRepair = Promise.resolve();
let screencastTransition = Promise.resolve();
let screencastActive = false;
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
let grantViews = new Map();
let grantTimers = new Map();
let attachedViewIds = new Set();
let retiredViewKeys = new Set();
let latestFrame = null;
let pendingBlocker = null;
const MAX_FRAME_HISTORY = 8;
let frameHistory = new Map();
let observedPage = null;
let observedPageHandlers = null;
let screencastListener = null;
let lastScreencastAcknowledgement = 0;
let targetTransition = Promise.resolve();
let pageBindingGeneration = 0;
let WebSocketServer;
let WebSocket;

const emit = (value) => process.stdout.write(`${JSON.stringify(value)}\n`);
const metadata = () => ({
  view_id: state.viewId,
  stream_epoch: state.streamEpoch,
  metadata_sequence: ++state.metadataSequence,
});
const grantExpiry = (value) => Number.isFinite(Number(value)) ? Number(value) : Date.parse(value);
function clearGrantTimer(viewId) {
  clearTimeout(grantTimers.get(viewId));
  grantTimers.delete(viewId);
}
function emitViewRetired(viewId) {
  const key = `${state?.streamEpoch}:${viewId}`;
  if (retiredViewKeys.has(key)) return;
  retiredViewKeys.add(key);
  emit({ type: 'view_retired', view_id: viewId, stream_epoch: state.streamEpoch });
}
function retireView(viewId, notify = true) {
  if (!state || !state.viewIds.has(viewId)) return;
  for (const socket of sockets) {
    if (socket.viewId !== viewId) continue;
    socket.authorized = false;
    socket.terminate();
  }
  const grant = viewGrants.get(viewId);
  if (grant) {
    grants.delete(grant);
    grantViews.delete(grant);
  }
  clearGrantTimer(viewId);
  viewGrants.delete(viewId);
  attachedViewIds.delete(viewId);
  state.viewIds.delete(viewId);
  if (state.controllerViewId === viewId) {
    void releaseHeldInput();
    state.controlled = false;
    state.controllerViewId = null;
    state.leaseGeneration++;
    state.nextInputSequence = 1;
    emitControl();
  }
  if (notify) emitViewRetired(viewId);
  if (state.viewIds.size === 0) {
    resetFrameTransport();
    void stopScreencast();
  }
}
function retireUnattachedView(viewId, grant) {
  if (!state || viewGrants.get(viewId) !== grant || attachedViewIds.has(viewId)) return;
  retireView(viewId, true);
}
function registerViewGrant(viewId, grant, expiresAt) {
  const expiry = grantExpiry(expiresAt);
  if (!viewId || !grant || !Number.isFinite(expiry)) throw new Error('browser frame grant is malformed');
  retiredViewKeys.delete(`${state?.streamEpoch}:${viewId}`);
  const previous = viewGrants.get(viewId);
  if (previous) {
    grants.delete(previous);
    grantViews.delete(previous);
  }
  clearGrantTimer(viewId);
  viewGrants.set(viewId, grant);
  grantViews.set(grant, viewId);
  grants.set(grant, expiry);
  const delay = expiry - Date.now();
  if (delay <= 0) {
    retireUnattachedView(viewId, grant);
    return;
  }
  grantTimers.set(viewId, setTimeout(() => retireUnattachedView(viewId, grant), Math.min(delay, 2 ** 31 - 1)));
}

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
    // Requested CSS dimensions and measured visual-viewport geometry are
    // intentionally tracked separately. Until metrics are confirmed, expose
    // the request but mark the geometry stale so consumers cannot act on it.
    css_width: state.geometryFresh ? state.viewportCssWidth : state.requestedCssWidth,
    css_height: state.geometryFresh ? state.viewportCssHeight : state.requestedCssHeight,
    visual_offset_x: state.visualOffsetX,
    visual_offset_y: state.visualOffsetY,
    scroll_x: state.scrollX,
    scroll_y: state.scrollY,
    visual_scale: state.visualScale,
    page_scale: state.pageScale,
    device_pixel_ratio: state.devicePixelRatio,
    geometry_fresh: state.geometryFresh,
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
function startupError(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}
async function importPlaywright(corePath) {
  if (typeof corePath !== 'string' || !corePath) {
    throw startupError(
      'browser_core_missing',
      'Playwright-core package path was not supplied; configure the paired package',
    );
  }
  let pkg;
  try {
    pkg = JSON.parse(await readFile(join(corePath, 'package.json'), 'utf8'));
  } catch {
    throw startupError(
      'browser_core_missing',
      'Playwright-core package metadata is unavailable; repair the paired package',
    );
  }
  if (pkg.name !== 'playwright-core') {
    throw startupError(
      'browser_core_invalid',
      'Configured package is not playwright-core; select the package paired with the CLI',
    );
  }
  const entry = pkg.module || pkg.main || 'index.js';
  if (typeof entry !== 'string' || entry.startsWith('/') || entry.split(/[\\/]/).includes('..')) {
    throw startupError(
      'browser_core_invalid',
      'Playwright-core package entry point is unsafe or invalid',
    );
  }
  let module;
  try {
    module = await import(pathToFileURL(join(corePath, entry)).href);
  } catch {
    throw startupError(
      'browser_core_import_failed',
      'Playwright-core exists but could not be imported; repair its package files and Node compatibility',
    );
  }
  const value = module.default || module;
  if (!value?.chromium || typeof value.chromium.connectOverCDP !== 'function') {
    throw startupError(
      'browser_capability_unsupported',
      'Playwright-core does not provide the chromium CDP attachment facility',
    );
  }
  return value;
}
async function importWebSocket(corePath) {
  let bundle;
  try {
    const require = createRequire(pathToFileURL(join(corePath, 'package.json')));
    bundle = require(join(corePath, 'lib', 'utilsBundle'));
  } catch {
    throw startupError(
      'browser_core_import_failed',
      'Playwright-core websocket support could not be imported; repair the paired package',
    );
  }
  WebSocketServer = bundle.wsServer;
  WebSocket = bundle.ws;
  if (!WebSocketServer || !WebSocket) {
    throw startupError(
      'browser_capability_unsupported',
      'Playwright-core does not provide the websocket frame transport facility',
    );
  }
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
function encodedJpegDimensions(bytes) {
  let offset = 2;
  while (offset + 1 < bytes.length) {
    if (bytes[offset] !== 0xff) throw new Error('screencast JPEG marker is malformed');
    while (offset < bytes.length && bytes[offset] === 0xff) offset++;
    if (offset >= bytes.length) break;
    const marker = bytes[offset++];
    if (marker === 0xd9 || marker === 0xda) break;
    if (marker === 0xd8 || marker === 0x01 || marker >= 0xd0 && marker <= 0xd7) continue;
    if (offset + 2 > bytes.length) throw new Error('screencast JPEG segment is truncated');
    const segmentLength = bytes.readUInt16BE(offset);
    if (segmentLength < 2 || offset + segmentLength > bytes.length) throw new Error('screencast JPEG segment is malformed');
    if ((marker >= 0xc0 && marker <= 0xc3) || (marker >= 0xc5 && marker <= 0xc7)
      || (marker >= 0xc9 && marker <= 0xcb) || (marker >= 0xcd && marker <= 0xcf)) {
      if (segmentLength < 7) throw new Error('screencast JPEG dimensions are malformed');
      const height = bytes.readUInt16BE(offset + 3);
      const width = bytes.readUInt16BE(offset + 5);
      if (!width || !height) throw new Error('screencast JPEG dimensions are empty');
      return { width, height };
    }
    offset += segmentLength;
  }
  throw new Error('screencast JPEG dimensions are unavailable');
}
function nextGeneration(previous) {
  return Math.max(Date.now(), Number(previous || 0) + 1);
}
function requestedViewport(viewport) {
  if (!viewport || typeof viewport !== 'object') throw new Error('viewport is missing');
  const width = boundedInteger(viewport.css_width, 1, MAX_WIDTH);
  const height = boundedInteger(viewport.css_height, 1, MAX_HEIGHT);
  const requestedDpr = Number(viewport.device_pixel_ratio ?? 1);
  if (!Number.isFinite(requestedDpr) || requestedDpr <= 0 || requestedDpr > 16) throw new Error('device pixel ratio is not finite');
  const maxDpr = Math.min(MAX_WIDTH / width, MAX_HEIGHT / height, Math.sqrt(MAX_PIXELS / (width * height)));
  const dpr = Math.min(requestedDpr, maxDpr);
  return { width, height, dpr };
}
function geometrySnapshot() {
  return {
    targetId: state.targetId,
    documentGeneration: state.documentGeneration,
    viewportRevision: state.viewportRevision,
    cssWidth: state.viewportCssWidth,
    cssHeight: state.viewportCssHeight,
    offsetX: state.visualOffsetX,
    offsetY: state.visualOffsetY,
    scrollX: state.scrollX,
    scrollY: state.scrollY,
    visualScale: state.visualScale,
    pageScale: state.pageScale,
    captureToken: state.captureToken,
  };
}
function geometryEqual(left, right) {
  return Boolean(left && right
    && left.targetId === right.targetId
    && left.documentGeneration === right.documentGeneration
    && left.viewportRevision === right.viewportRevision
    && left.cssWidth === right.cssWidth
    && left.cssHeight === right.cssHeight
    && left.offsetX === right.offsetX
    && left.offsetY === right.offsetY
    && left.scrollX === right.scrollX
    && left.scrollY === right.scrollY
    && left.visualScale === right.visualScale
    && left.pageScale === right.pageScale
    && left.captureToken === right.captureToken);
}
function invalidateViewport() {
  state.viewportRevision++;
  state.inputViewportRevision = state.viewportRevision;
  state.geometryFresh = false;
  state.viewportTransition = true;
  resetFrameTransport();
}
async function applyRequestedViewport(viewport) {
  const requested = requestedViewport(viewport);
  await pageCdp.send('Emulation.setDeviceMetricsOverride', {
    width: requested.width,
    height: requested.height,
    deviceScaleFactor: requested.dpr,
    mobile: false,
    screenWidth: Math.max(1, Math.round(requested.width * requested.dpr)),
    screenHeight: Math.max(1, Math.round(requested.height * requested.dpr)),
  });
  state.requestedCssWidth = requested.width;
  state.requestedCssHeight = requested.height;
  state.devicePixelRatio = requested.dpr;
  return requested;
}
function pageBindingIsCurrent(expectedPage, expectedCdp, expectedBinding) {
  return Boolean(state && page === expectedPage && pageCdp === expectedCdp && pageBindingGeneration === expectedBinding);
}
function measuredGeometry(metrics, dpr, requestedCssWidth, requestedCssHeight) {
  const visual = metrics?.cssVisualViewport || metrics?.visualViewport;
  if (!visual || !Number.isFinite(Number(requestedCssWidth))
    || !Number.isFinite(Number(requestedCssHeight))
    || requestedCssWidth <= 0 || requestedCssHeight <= 0
    || !Number.isFinite(Number(visual.offsetX)) || !Number.isFinite(Number(visual.offsetY))
    || !Number.isFinite(Number(visual.pageX)) || !Number.isFinite(Number(visual.pageY))
    || !Number.isFinite(Number(visual.scale)) || Number(visual.scale) <= 0
    || !Number.isFinite(Number(dpr)) || Number(dpr) <= 0) return null;
  return {
    // CDP's cssVisualViewport.clientWidth can shrink when a page gains a
    // scrollbar, while screencast pixels and Input.dispatchMouseEvent still
    // cover the full emulated layout viewport. Keep that coordinate space
    // anchored to the accepted emulation size; otherwise navigation can
    // oscillate frame geometry and shift pointer coordinates by scrollbar width.
    width: requestedCssWidth,
    height: requestedCssHeight,
    offsetX: Number(visual.offsetX),
    offsetY: Number(visual.offsetY),
    scrollX: Number(visual.pageX),
    scrollY: Number(visual.pageY),
    scale: Number(visual.scale),
    pageScale: Number(visual.scale),
    dpr: Number(dpr),
  };
}
async function updatePageState(expectedPage = page, expectedCdp = pageCdp, expectedBinding = pageBindingGeneration) {
  if (!pageBindingIsCurrent(expectedPage, expectedCdp, expectedBinding)) return false;
  state.url = expectedPage.url();
  let title;
  try { title = await expectedPage.title(); } catch { title = ''; }
  if (!pageBindingIsCurrent(expectedPage, expectedCdp, expectedBinding)) return false;
  state.title = title;
  try {
    const metrics = await expectedCdp.send('Page.getLayoutMetrics');
    const measuredDpr = await expectedPage.evaluate(() => Number(window.devicePixelRatio)).catch(() => state.devicePixelRatio);
    if (!pageBindingIsCurrent(expectedPage, expectedCdp, expectedBinding)) return false;
    const geometry = measuredGeometry(metrics, measuredDpr, state.requestedCssWidth, state.requestedCssHeight);
    if (!geometry) {
      state.geometryFresh = false;
      return false;
    }
    const changed = state.viewportCssWidth !== geometry.width
      || state.viewportCssHeight !== geometry.height
      || state.visualOffsetX !== geometry.offsetX
      || state.visualOffsetY !== geometry.offsetY
      || state.scrollX !== geometry.scrollX
      || state.scrollY !== geometry.scrollY
      || state.visualScale !== geometry.scale
      || state.pageScale !== geometry.pageScale
      || state.devicePixelRatio !== geometry.dpr;
    state.viewportCssWidth = geometry.width;
    state.viewportCssHeight = geometry.height;
    state.visualOffsetX = geometry.offsetX;
    state.visualOffsetY = geometry.offsetY;
    state.scrollX = geometry.scrollX;
    state.scrollY = geometry.scrollY;
    state.visualScale = geometry.scale;
    state.pageScale = geometry.pageScale;
    state.devicePixelRatio = geometry.dpr;
    state.geometryFresh = true;
    if (changed && !state.viewportTransition) {
      state.viewportRevision++;
      state.inputViewportRevision = state.viewportRevision;
      resetFrameTransport();
    }
    state.viewportTransition = false;
    return changed;
  } catch {
    state.geometryFresh = false;
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
async function updateHistory(expectedCdp = pageCdp, expectedBinding = pageBindingGeneration) {
  try {
    const history = await expectedCdp.send('Page.getNavigationHistory');
    if (!pageBindingIsCurrent(page, expectedCdp, expectedBinding)) return;
    state.canGoBack = history.currentIndex > 0;
    state.canGoForward = history.currentIndex >= 0 && history.currentIndex + 1 < history.entries.length;
  } catch {
    if (pageBindingIsCurrent(page, expectedCdp, expectedBinding)) {
      state.canGoBack = false; state.canGoForward = false;
    }
  }
}
async function updateFrameId(expectedCdp = pageCdp, expectedBinding = pageBindingGeneration) {
  try {
    const frameTree = await expectedCdp.send('Page.getFrameTree');
    if (pageBindingIsCurrent(page, expectedCdp, expectedBinding)) {
      state.frameId = frameTree.frameTree.frame.id;
      state.loaderId = frameTree.frameTree.frame.loaderId || null;
    }
  } catch {
    if (pageBindingIsCurrent(page, expectedCdp, expectedBinding)) state.frameId = 'main';
  }
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
    const previous = state.cursor;
    state.cursor = next;
    if (!previous || previous.cursor !== next.cursor || previous.target_id !== next.target_id || previous.document_generation !== next.document_generation || previous.viewport_revision !== next.viewport_revision) emitCursor();
  } catch {}
}
function setBlocker(kind, message, defaultPrompt, resolve) {
  if (pendingBlocker?.resolve) pendingBlocker.resolve('dismiss');
  pendingBlocker = { blocker_id: randomUUID(), kind, message: String(message || '').slice(0, 4096), default_prompt: defaultPrompt == null ? null : String(defaultPrompt).slice(0, 4096), target_id: state.targetId, document_generation: state.documentGeneration, cancellable: true, resolve };
  emitBlocker();
  return pendingBlocker;
}
function clearBlocker(id) {
  if (!pendingBlocker || pendingBlocker.blocker_id !== id) return false;
  pendingBlocker = null;
  emitBlocker();
  return true;
}
async function dismissPendingBlocker() {
  const blocker = pendingBlocker;
  if (!blocker) return;
  pendingBlocker = null;
  emitBlocker();
  try { await blocker.resolve?.('dismiss'); } catch {}
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
function requireInputControl(command) {
  const sequence = requireControl(command);
  if (sequence === null) throw Object.assign(new Error('Browser input sequence is required'), { code: 'stale_input_sequence' });
  return sequence;
}
function virtualKeyCode(key, code) {
  const named = { Backspace: 8, Tab: 9, Enter: 13, Shift: 16, Control: 17, Alt: 18, Pause: 19, CapsLock: 20, Escape: 27, Space: 32, PageUp: 33, PageDown: 34, End: 35, Home: 36, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, Insert: 45, Delete: 46 };
  const punctuation = { Backquote: 192, Minus: 189, Equal: 187, BracketLeft: 219, Backslash: 220, BracketRight: 221, Semicolon: 186, Quote: 222, Comma: 188, Period: 190, Slash: 191, NumpadDecimal: 110 };
  return named[key] ?? punctuation[code] ?? (key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0);
}
function advanceInput(sequence) { if (sequence !== null) state.nextInputSequence = sequence + 1; }
function cdpInputPoint(x, y) {
  const scale = state.cdpInputScale;
  return { x: x * scale, y: y * scale };
}
async function releaseHeldInput() {
  const buttons = state?.pressedButtons || 0;
  const buttonNames = [[1, 'left'], [2, 'right'], [4, 'middle']];
  for (const [mask, button] of buttonNames) {
    if (!(buttons & mask)) continue;
    const point = cdpInputPoint(state.pointer?.x || 0, state.pointer?.y || 0);
    try {
      await pageCdp?.send('Input.dispatchMouseEvent', {
        type: 'mouseReleased',
        x: point.x,
        y: point.y,
        button,
        buttons: 0,
        clickCount: 0,
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
async function enumerateTargets(requireSelected = true) {
  const all = (await browserCdp.send('Target.getTargets')).targetInfos;
  state.targets = all
    .filter((target) => ['page', 'background_page', 'service_worker'].includes(target.type))
    .map((target, order) => ({
      target_id: target.targetId,
      kind: target.type === 'page' ? 'page' : 'background',
      title: target.title || '', url: target.url || '', order,
      opener_target_id: target.openerId || null, can_close: target.type === 'page',
    }));
  const active = state.targets.find((target) => target.target_id === state.targetId);
  if (!active && requireSelected) throw new Error('attached browser target no longer exists');
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
  pageBindingGeneration++;
  const previousTarget = state?.targetId;
  const targetChanged = previousTarget && previousTarget !== targetId;
  if (targetChanged) await dismissPendingBlocker();
  if (restartScreencast) await stopScreencast();
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
  if (targetChanged) {
    state.documentGeneration = nextGeneration(state.documentGeneration);
    state.frameGeneration = nextGeneration(state.frameGeneration);
    state.viewportRevision++;
    state.cursor = null;
    state.pointer = null;
    state.pointerSampleSequence++;
    state.cdpInputScale = state.devicePixelRatio;
    resetFrameTransport();
  }
  if (!restartScreencast) await ensureInitialPage();
  await installPageObservers();
  await applyRequestedViewport({ css_width: state.requestedCssWidth, css_height: state.requestedCssHeight, device_pixel_ratio: state.devicePixelRatio });
  await updatePageState();
  await updateFrameId();
  await updateHistory();
  if (targetChanged) {
    emitEvent('document_changed', { document: documentState() });
    emitEvent('viewport_changed', { viewport: viewportState() });
    emitNavigation();
  }
  if (restartScreencast) await startScreencast();
}

function queueTargetTransition(action) {
  const next = targetTransition.then(action, action);
  targetTransition = next.catch(() => {});
  return next;
}

async function resetInputTransition() {
  await releaseHeldInput();
  state.pointer = null;
  state.cursor = null;
  state.pointerSampleSequence++;
  state.focus = { page_focused: false, editable: false, selection_available: false, composition_active: false };
  if (state.controlled) {
    state.leaseGeneration++;
    state.nextInputSequence = 1;
    emitControl();
  }
}

async function closeSelectedPage() {
  return queueTargetTransition(async () => {
    await enumerateTargets();
    const pages = state.targets.filter((target) => target.kind === 'page');
    const selectedIndex = pages.findIndex((target) => target.target_id === state.targetId);
    if (selectedIndex < 0) throw new Error('selected browser target no longer exists');
    if (pages.length < 2) {
      throw Object.assign(new Error('The final browser tab must remain open'), { code: 'browser_last_page' });
    }
    const successor = pages[selectedIndex + 1] || pages[selectedIndex - 1];
    await resetInputTransition();
    await dismissPendingBlocker();
    await page.close();
    await bindPage(successor.target_id);
  });
}

async function reconcileDestroyedTarget() {
  return queueTargetTransition(async () => {
    const previousPages = state.targets.filter((target) => target.kind === 'page');
    const previousIndex = previousPages.findIndex((target) => target.target_id === state.targetId);
    await enumerateTargets(false);
    if (state.targets.some((target) => target.target_id === state.targetId)) {
      emitEvent('targets_changed', { targets: state.targets, displayed_target_id: state.targetId });
      return;
    }
    const pages = state.targets.filter((target) => target.kind === 'page');
    const successor = previousPages.slice(previousIndex + 1).find((candidate) => pages.some((target) => target.target_id === candidate.target_id))
      || previousPages.slice(0, Math.max(previousIndex, 0)).reverse().find((candidate) => pages.some((target) => target.target_id === candidate.target_id))
      || pages[0];
    await resetInputTransition();
    if (successor) {
      await bindPage(successor.target_id);
    } else {
      const created = await context.newPage();
      const cdp = await context.newCDPSession(created);
      const targetId = (await cdp.send('Target.getTargetInfo')).targetInfo?.targetId;
      await cdp.detach();
      if (!targetId) throw new Error('replacement tab has no target');
      await bindPage(targetId);
      await ensureInitialPage();
    }
    await refreshTargetsNow();
  });
}

async function refreshTargetsNow() {
  await enumerateTargets();
  emitEvent('targets_changed', { targets: state.targets, displayed_target_id: state.targetId });
}
function refreshTargets() {
  return queueTargetTransition(refreshTargetsNow);
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
function scheduleSessionAck(frame) {
  if (!frame || frame.sessionId === undefined || frame._ackScheduled) return;
  frame._ackScheduled = true;
  const delay = Math.max(0, FRAME_INTERVAL_MS - (performance.now() - lastScreencastAcknowledgement));
  setTimeout(() => {
    ackSession(frame.sessionId, frame._cdp || pageCdp);
    lastScreencastAcknowledgement = performance.now();
  }, delay);
}
function acknowledgeFrame(frame) {
  if (!frame || frame.acknowledged) return;
  frame.acknowledged = true;
  if (!frame._ackScheduled) ackSession(frame.sessionId, frame.cdp);
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
  if (slot === 'awaitingFrame') {
    socket.frameSequence = undefined;
    clearTimeout(socket.frameTimer);
    socket.frameTimer = undefined;
  }
  releaseFrame(frame);
}
function writeFrame(socket, frame, retained = false) {
  if (!socket.authorized || socket.readyState !== WebSocket.OPEN) return false;
  socket.awaitingFrame = frame;
  socket.frameSequence = frame.descriptor.frame_sequence;
  socket.frameTimer = setTimeout(() => {
    if (socket.awaitingFrame === frame) {
      forgetSocketFrame(socket, 'awaitingFrame');
      socket.close(1011, 'frame credit timeout');
    }
  }, 3000);
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
  if (!socket.authorized || socket.readyState !== WebSocket.OPEN || !state?.viewIds?.has(socket.viewId)) return false;
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
  clearTimeout(socket.preauthTimer);
  socket.preauthTimer = undefined;
  forgetSocketFrame(socket, 'awaitingFrame');
  forgetSocketFrame(socket, 'pendingFrame');
  socket.frameSequence = undefined;
  sockets.delete(socket);
  if (socket.viewId) retireView(socket.viewId, true);
}
function resetFrameTransport() {
  const retired = latestFrame;
  latestFrame = null;
  frameHistory.clear();
  if (state) {
    state.captureToken++;
    state.captureBaseline = null;
  }
  if (retired) acknowledgeFrame(retired);
  for (const socket of sockets) {
    // Pending replacements belong to the retired capture baseline. Release
    // them through their original CDP session; an awaiting frame may still be
    // decoded by the client and remains valid until its own credit arrives.
    forgetSocketFrame(socket, 'pendingFrame');
  }
}
function frameGeometry(frame) {
  const metadata = frame?.metadata && typeof frame.metadata === 'object' ? frame.metadata : {};
  const baseline = frame?._geometry || state.captureBaseline;
  if (!baseline || !geometryEqual(baseline, state.captureBaseline) || !state.geometryFresh) return null;
  for (const [key, expected] of [['scrollOffsetX', baseline.scrollX], ['scrollOffsetY', baseline.scrollY]]) {
    if (metadata[key] !== undefined && (!Number.isFinite(Number(metadata[key])) || Math.abs(Number(metadata[key]) - expected) > 0.01)) return null;
  }
  if (metadata.pageScaleFactor !== undefined
    && (!Number.isFinite(Number(metadata.pageScaleFactor)) || Math.abs(Number(metadata.pageScaleFactor) - baseline.pageScale) > 0.01)) return null;
  return baseline;
}
function frameMetadataDiffers(frame, baseline) {
  const metadata = frame?.metadata && typeof frame.metadata === 'object' ? frame.metadata : {};
  for (const [key, expected] of [['scrollOffsetX', baseline?.scrollX], ['scrollOffsetY', baseline?.scrollY], ['pageScaleFactor', baseline?.pageScale]]) {
    if (metadata[key] !== undefined && Number.isFinite(Number(metadata[key]))
      && Math.abs(Number(metadata[key]) - expected) > 0.01) return true;
  }
  return false;
}
function rebaseScrollFrame(frame, baseline) {
  const metadata = frame?.metadata && typeof frame.metadata === 'object' ? frame.metadata : {};
  const pageScale = metadata.pageScaleFactor === undefined ? baseline?.pageScale : Number(metadata.pageScaleFactor);
  const scrollX = metadata.scrollOffsetX === undefined ? baseline?.scrollX : Number(metadata.scrollOffsetX);
  const scrollY = metadata.scrollOffsetY === undefined ? baseline?.scrollY : Number(metadata.scrollOffsetY);
  if (!baseline || !geometryEqual(baseline, state.captureBaseline)
    || !Number.isFinite(pageScale) || Math.abs(pageScale - baseline.pageScale) > 0.01
    || !Number.isFinite(scrollX) || !Number.isFinite(scrollY)) return false;
  if (Math.abs(scrollX - baseline.scrollX) <= 0.01 && Math.abs(scrollY - baseline.scrollY) <= 0.01) return true;

  // Screencast metadata describes the scroll position of these exact pixels.
  // Advance geometry in place instead of stopping and restarting capture.
  state.scrollX = scrollX;
  state.scrollY = scrollY;
  state.viewportRevision++;
  const nextBaseline = geometrySnapshot();
  state.captureBaseline = nextBaseline;
  frame._geometry = nextBaseline;
  frame._captureToken = nextBaseline.captureToken;
  emitEvent('viewport_changed', { viewport: viewportState() });
  return true;
}
function queueFrameGeometryRepair(expectedPage, expectedCdp, expectedBinding, baseline) {
  frameGeometryRepair = frameGeometryRepair.then(async () => {
    if (!baseline || !pageBindingIsCurrent(expectedPage, expectedCdp, expectedBinding)
      || state.captureToken !== baseline.captureToken) return;
    const changed = await updatePageState(expectedPage, expectedCdp, expectedBinding);
    if (!changed || !pageBindingIsCurrent(expectedPage, expectedCdp, expectedBinding)) return;
    emitEvent('viewport_changed', { viewport: viewportState() });
    await stopScreencast(expectedCdp);
    await startScreencast(expectedCdp, expectedBinding);
    // Only screencast metadata can bind pixels to compositor scroll geometry.
    // A separate screenshot could already show a later scroll position.
  }).catch(() => {});
}
function enqueueFrame(frame) {
  if (!state || (frame?._cdp && frame._cdp !== pageCdp)
    || (frame?._captureToken !== undefined && frame._captureToken !== state.captureToken)) {
    if (!frame?._ackScheduled) ackSession(frame?.sessionId, frame?._cdp);
    return;
  }
  let next;
  try {
    if (!frame || typeof frame.data !== 'string') throw new Error('screencast payload is not base64 text');
    const jpeg = Buffer.from(frame.data, 'base64');
    const geometry = frameGeometry(frame);
    if (!geometry) throw new Error('screencast frame geometry is stale or inconclusive');
    const dimensions = encodedJpegDimensions(jpeg);
    const width = dimensions.width;
    const height = dimensions.height;
    if (!jpeg.length || jpeg.length > MAX_JPEG || jpeg[0] !== 0xff || jpeg[1] !== 0xd8
      || jpeg[jpeg.length - 2] !== 0xff || jpeg[jpeg.length - 1] !== 0xd9
      || width > MAX_WIDTH || height > MAX_HEIGHT || width * height > MAX_PIXELS) {
      throw new Error('screencast frame exceeds frozen bounds');
    }
    const rawMetadata = frame.metadata && typeof frame.metadata === 'object' ? frame.metadata : {};
    const timestamp = boundedNumber(rawMetadata.timestamp, Date.now() / 1000, Number.MAX_SAFE_INTEGER / 1_000_000);
    const descriptor = {
      target_id: geometry.targetId,
      stream_epoch: state.streamEpoch,
      frame_sequence: ++state.frameSequence,
      document_generation: geometry.documentGeneration,
      viewport_revision: geometry.viewportRevision,
      image_width: width,
      image_height: height,
      viewport_css_width: geometry.cssWidth,
      viewport_css_height: geometry.cssHeight,
      viewport_offset_x: geometry.offsetX,
      viewport_offset_y: geometry.offsetY,
      scroll_x: geometry.scrollX,
      scroll_y: geometry.scrollY,
      capture_timestamp_micros: Math.floor(timestamp * 1_000_000),
      jpeg_length: jpeg.length,
    };
    next = {
      descriptor,
      sessionId: frame.sessionId,
      cdp: frame._cdp || pageCdp,
      _ackScheduled: Boolean(frame._ackScheduled),
      payload: envelope(descriptor, jpeg),
      references: 0,
      acknowledged: false,
    };
    frameHistory.set(descriptor.frame_sequence, descriptor);
    while (frameHistory.size > MAX_FRAME_HISTORY) {
      frameHistory.delete(frameHistory.keys().next().value);
    }
    emit({ type: 'frame', descriptor });
  } catch (error) {
    // Invalid or stale packets are deliberately discarded, but their CDP
    // credit is released on the session that produced the packet.
    emit({ type: 'failed', code: 'browser_frame_invalid', message: String(error.message || error) });
    if (!frame?._ackScheduled) ackSession(frame?.sessionId, frame?._cdp);
    return;
  }
  latestFrame = next;
  for (const socket of sockets) queueFrame(socket, next);
  const delay = Math.max(0, FRAME_INTERVAL_MS - (performance.now() - lastScreencastAcknowledgement));
  setTimeout(() => {
    acknowledgeFrame(next);
    lastScreencastAcknowledgement = performance.now();
  }, delay);
}

function allowedFrameOrigin(origin) {
  if (!origin) return true;
  try {
    const parsed = new URL(origin);
    return ['http:', 'https:', 'tauri:'].includes(parsed.protocol)
      && (parsed.hostname === 'localhost' || parsed.hostname === '127.0.0.1'
        || parsed.hostname === '::1' || parsed.hostname.endsWith('.localhost'));
  } catch {
    return false;
  }
}
function startFrameServer() {
  return new Promise((resolve, reject) => {
    server = new WebSocketServer({ host: '127.0.0.1', port: 0, maxPayload: MAX_WS_PAYLOAD, perMessageDeflate: false });
    server.once('listening', () => { port = server.address().port; resolve(); });
    server.on('connection', (socket, request) => {
      const expectedHost = `127.0.0.1:${port}`;
      const expectedOrigin = `http://${expectedHost}`;
      if (sockets.size >= MAX_FRAME_PEERS
        || request.headers.host !== expectedHost
        || request.headers.origin !== expectedOrigin) {
        socket.close(1008, 'unexpected frame peer');
        return;
      }
      socket.authorized = false;
      socket.preauthTimer = setTimeout(() => socket.close(1008, 'frame authorization timeout'), PREAUTH_TIMEOUT_MS);
      sockets.add(socket);
      socket.on('close', () => cleanupSocket(socket));
      socket.on('error', () => cleanupSocket(socket));
      socket.on('message', (payload, binary) => {
        if (binary || payload.length > MAX_WS_PAYLOAD) { socket.close(1003, 'text control required'); return; }
        let value; try { value = JSON.parse(payload.toString('utf8')); } catch { socket.close(1003, 'invalid control'); return; }
        if (!socket.authorized) {
          const grant = typeof value?.grant === 'string' ? value.grant : '';
          const expiry = grants.get(grant);
          const viewId = grantViews.get(grant);
          if (!grant || !viewId || !Number.isFinite(expiry) || expiry < Date.now() || !state.viewIds.has(viewId)) {
            socket.close(1008, 'invalid grant');
            return;
          }
          grants.delete(grant);
          grantViews.delete(grant);
          clearTimeout(socket.preauthTimer);
          socket.preauthTimer = undefined;
          clearGrantTimer(viewId);
          attachedViewIds.add(viewId);
          socket.viewId = viewId;
          socket.authorized = true;
          if (latestFrame) queueFrame(socket, latestFrame);
          return;
        }
        if (!state.viewIds.has(socket.viewId)) {
          socket.close(1008, 'view revoked');
          return;
        }
        if (!value || !['ack', 'discard'].includes(value.type)
          || !Number.isSafeInteger(value.frame_sequence)
          || !releaseSocketFrame(socket, value.frame_sequence)) {
          socket.close(1008, 'invalid credit');
        }
      });
    });
  });
}

function commandProof(command) {
  if (!command || typeof command !== 'object') return null;
  // Inspection carries its frame proof one level deeper than mutations. Keep
  // the shared gateway check aware of that nesting so an old Element request
  // cannot pass merely because the outer command has no location field.
  if (command.type === 'inspect') return command.command?.location || null;
  return command.location || command.context || null;
}

function proofMatches(command) {
  if (!command || typeof command !== 'object') return false;
  const proof = commandProof(command);
  if (proof !== null) {
    if (!proof || typeof proof !== 'object' || proof.target_id !== state.targetId
      || proof.document_generation !== state.documentGeneration) return false;
    if (proof.viewport_revision !== undefined) {
      if (command.type === 'wheel') {
        if (!Number.isSafeInteger(proof.viewport_revision)
          || proof.viewport_revision < state.inputViewportRevision
          || proof.viewport_revision > state.viewportRevision) return false;
      } else if (proof.viewport_revision !== state.viewportRevision) return false;
    }
    if (proof.lease_generation !== undefined && proof.lease_generation !== state.leaseGeneration) return false;
    // Wheel coordinates survive scroll-only revisions in the current CSS
    // viewport and do not require the old displayed frame to remain retained.
    if (proof.presented_frame_sequence !== undefined && command.type !== 'wheel') {
      const presented = frameHistory.get(proof.presented_frame_sequence);
      if (!presented
        || presented.target_id !== state.targetId
        || presented.stream_epoch !== state.streamEpoch
        || presented.document_generation !== state.documentGeneration
        || presented.viewport_revision !== state.viewportRevision) return false;
    }
  }
  if (command.type === 'tab' && command.command?.target_id !== undefined) {
    if (command.command.type === 'close') return state.targets.some((target) => target.kind === 'page' && target.can_close && target.target_id === command.command.target_id);
    if (command.command.type === 'select') return state.targets.some((target) => target.kind === 'page' && target.target_id === command.command.target_id);
    return false;
  }
  return true;
}

function inspectionFrame(location) {
  if (!location || typeof location !== 'object') return null;
  const presented = frameHistory.get(location.presented_frame_sequence);
  if (!presented
    || presented.target_id !== state.targetId
    || presented.stream_epoch !== state.streamEpoch
    || presented.document_generation !== state.documentGeneration
    || presented.viewport_revision !== state.viewportRevision
    || presented.viewport_css_width !== state.viewportCssWidth
    || presented.viewport_css_height !== state.viewportCssHeight
    || presented.viewport_offset_x !== state.visualOffsetX
    || presented.viewport_offset_y !== state.visualOffsetY) return null;
  return presented;
}

function inspectionGuard(command) {
  const location = command?.location;
  if (!location || typeof location !== 'object') {
    return { status: 'rejected', code: 'invalid_inspection_location', message: 'Element inspection requires a frame-bound location proof' };
  }
  const presented = inspectionFrame(location);
  if (!presented) {
    return { status: 'stale', code: 'stale_location', message: 'The presented frame is no longer current' };
  }
  if (!Number.isFinite(command.x) || !Number.isFinite(command.y)
    || command.x < 0 || command.y < 0
    || command.x >= presented.viewport_css_width
    || command.y >= presented.viewport_css_height) {
    return { status: 'rejected', code: 'invalid_inspection_coordinates', message: 'Element inspection coordinates are outside the presented viewport' };
  }
  if (command.pointer_sample_sequence === null) return { status: 'ok', presented };
  if (!Number.isSafeInteger(command.pointer_sample_sequence)) {
    return { status: 'rejected', code: 'invalid_pointer_sample', message: 'Element inspection pointer sample is malformed' };
  }
  const cursor = state.cursor;
  if (!cursor) {
    return { status: 'unsupported', capability: 'inspection', code: 'inspection_pointer_sample_missing', message: 'Element inspection is waiting for a current cursor sample' };
  }
  if (cursor.pointer_sample_sequence !== command.pointer_sample_sequence
    || cursor.target_id !== state.targetId
    || cursor.document_generation !== state.documentGeneration
    || cursor.viewport_revision !== state.viewportRevision) {
    return { status: 'stale', code: 'stale_pointer_sample', message: 'The cursor sample no longer matches the presented page' };
  }
  return { status: 'ok', presented };
}

function inspectionStillCurrent(command, binding, expectedFrameGeneration) {
  return pageBindingIsCurrent(page, pageCdp, binding)
    && state.frameGeneration === expectedFrameGeneration
    && inspectionGuard(command).status === 'ok';
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
function queueScreencastOperation(operation) {
  const next = screencastTransition.then(operation, operation);
  screencastTransition = next.catch(() => {});
  return next;
}
async function startScreencast(expectedCdp = pageCdp, expectedBinding = pageBindingGeneration) {
  return queueScreencastOperation(async () => {
    if (!pageBindingIsCurrent(page, expectedCdp, expectedBinding) || !state.geometryFresh || state.viewIds.size === 0 || screencastActive) return;
    state.captureToken++;
    state.captureBaseline = geometrySnapshot();
    try {
      await expectedCdp.send('Page.startScreencast', {
        format: 'jpeg', quality: 80,
        maxWidth: Math.max(1, Math.floor(state.requestedCssWidth * state.devicePixelRatio)),
        maxHeight: Math.max(1, Math.floor(state.requestedCssHeight * state.devicePixelRatio)),
        everyNthFrame: 1,
      });
      if (pageBindingIsCurrent(page, expectedCdp, expectedBinding)) screencastActive = true;
    } catch (error) {
      if (pageBindingIsCurrent(page, expectedCdp, expectedBinding)) {
        state.captureBaseline = null;
        screencastActive = false;
      }
      throw error;
    }
  });
}
async function stopScreencast(expectedCdp = pageCdp) {
  return queueScreencastOperation(async () => {
    if (expectedCdp === pageCdp) screencastActive = false;
    try { await expectedCdp?.send('Page.stopScreencast'); } catch {}
  });
}
async function captureCurrentFrame(expectedCdp = pageCdp, expectedBinding = pageBindingGeneration) {
  if (!pageBindingIsCurrent(page, expectedCdp, expectedBinding) || !state.geometryFresh) return;
  const baseline = state.captureBaseline;
  try {
    const capture = await expectedCdp.send('Page.captureScreenshot', { format: 'jpeg', quality: 80, captureBeyondViewport: false });
    if (!pageBindingIsCurrent(page, expectedCdp, expectedBinding) || !baseline
      || baseline.captureToken !== state.captureToken || typeof capture.data !== 'string') return;
    enqueueFrame({
      data: capture.data, metadata: { timestamp: Date.now() / 1000 }, _cdp: expectedCdp,
      _captureToken: baseline.captureToken, _geometry: baseline,
    });
  } catch (error) {
    if (pageBindingIsCurrent(page, expectedCdp, expectedBinding)) emit({ type: 'failed', code: 'browser_frame_capture', message: String(error.message || error) });
  }
}
async function installPageObservers() {
  if (observedPage && observedPageHandlers) {
    for (const [event, handler] of observedPageHandlers) observedPage.off?.(event, handler);
  }
  if (screencastListener && pageCdp) pageCdp.off?.('Page.screencastFrame', screencastListener);
  const observed = page;
  const cdp = pageCdp;
  const targetId = state?.targetId;
  const bindingGeneration = pageBindingGeneration;
  const frameGeneration = state?.frameGeneration;
  const bindingCurrent = () => pageBindingIsCurrent(observed, cdp, bindingGeneration) && state.targetId === targetId;
  const current = () => bindingCurrent();
  const onFrameNavigated = (frame) => {
    if (!bindingCurrent() || frame !== observed.mainFrame()) return;
    void (async () => {
      // Playwright emits `framenavigated` for History API route changes too.
      // The CDP loader ID changes only when Chromium replaces the document.
      let loaderId = null;
      try { loaderId = (await cdp.send('Page.getFrameTree')).frameTree.frame.loaderId || null; } catch {}
      if (!bindingCurrent()) return;
      if (loaderId === state.loaderId) {
        const viewportChanged = await updatePageState(observed, cdp, bindingGeneration);
        await updateHistory(cdp, bindingGeneration);
        if (viewportChanged) {
          await stopScreencast(cdp);
          await startScreencast(cdp, bindingGeneration);
          await captureCurrentFrame(cdp, bindingGeneration);
          emitEvent('viewport_changed', { viewport: viewportState() });
        }
        if (bindingCurrent()) emitNavigation();
        return;
      }
      state.loaderId = loaderId;
      // A replacement document can restore Chromium's host-device input
      // mapping even though its emulated viewport and frame geometry persist.
      state.cdpInputScale = state.devicePixelRatio;
      state.documentGeneration = nextGeneration(state.documentGeneration);
      state.frameGeneration = nextGeneration(state.frameGeneration);
      resetFrameTransport();
      void dismissPendingBlocker();
      frameBarrier = frameBarrier.then(async () => {
        if (!bindingCurrent()) return;
        const viewportChanged = await updatePageState(observed, cdp, bindingGeneration);
        if (!bindingCurrent()) return;
        await updateFrameId(cdp, bindingGeneration);
        if (!bindingCurrent()) return;
        await updateHistory(cdp, bindingGeneration);
        if (!bindingCurrent()) return;
        if (viewportChanged) emitEvent('viewport_changed', { viewport: viewportState() });
        emitEvent('document_changed', { document: documentState() });
        emitNavigation();
        await stopScreencast(cdp);
        await startScreencast(cdp, bindingGeneration);
        await captureCurrentFrame(cdp, bindingGeneration);
      }).catch(() => {});
    })();
  };
  const onLoad = () => {
    if (!current()) return;
    void updatePageState(observed, cdp, bindingGeneration)
      .then((viewportChanged) => {
        if (!current()) return;
        if (viewportChanged) emitEvent('viewport_changed', { viewport: viewportState() });
        return updateHistory(cdp, bindingGeneration);
      })
      .then(async () => {
        if (!current()) return;
        emitNavigation();
        if (!state.captureBaseline) {
          await stopScreencast(cdp);
          await startScreencast(cdp, bindingGeneration);
        }
        await captureCurrentFrame(cdp, bindingGeneration);
      })
      .catch(() => {});
  };
  const onDialog = (dialog) => {
    if (!current()) { void dialog.dismiss().catch(() => {}); return; }
    const type = dialog.type();
    setBlocker('dialog', `${type}: ${dialog.message()}`, type === 'prompt' ? dialog.defaultValue() : null, async (decision, text) => {
      if (decision === 'accept') await dialog.accept(text ?? undefined);
      else await dialog.dismiss();
    });
  };
  const onFileChooser = (chooser) => {
    if (!current()) { void chooser.setFiles([]).catch(() => {}); return; }
    const blocker = setBlocker('file_chooser', 'File upload requires a local file-selection adapter', null, async () => chooser.setFiles([]));
    blocker.cancellable = true;
  };
  const onDownload = (download) => {
    if (current()) setBlocker('download', `Download requested: ${download.suggestedFilename()}`, null, async () => {});
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
    // Capture geometry and identity at CDP delivery time. The page can scroll
    // on the compositor after a command returns, so never read the mutable
    // baseline after waiting on navigation or another frame.
    const baseline = state?.captureBaseline ? { ...state.captureBaseline } : null;
    frame._cdp = cdp;
    frame._captureToken = baseline?.captureToken ?? state?.captureToken;
    frame._geometry = baseline;
    scheduleSessionAck(frame);
    if (!baseline) return;
    if (frameMetadataDiffers(frame, baseline) && !rebaseScrollFrame(frame, baseline)) {
      queueFrameGeometryRepair(observed, cdp, bindingGeneration, baseline);
      return;
    }
    const geometry = frame._geometry;
    const identity = {
      documentGeneration: geometry.documentGeneration,
      viewportRevision: geometry.viewportRevision,
      captureToken: geometry.captureToken,
    };
    if (!current()
      || identity.documentGeneration !== state.documentGeneration
      || identity.viewportRevision !== state.viewportRevision
      || identity.captureToken !== state.captureToken) return;
    enqueueFrame(frame);
  };
  cdp.on('Page.screencastFrame', screencastListener);
}
async function attach(message) {
  const { chromium } = await importPlaywright(message.playwright_core);
  await importWebSocket(message.playwright_core);
  try {
    browser = await chromium.connectOverCDP(message.cdp_endpoint);
    context = browser.contexts()[0];
    browserCdp = await browser.newBrowserCDPSession();
    await browserCdp.send('Target.setDiscoverTargets', { discover: true });
  } catch {
    throw startupError(
      'browser_helper_runtime_failed',
      'The browser helper could not attach to the owned Chromium CDP endpoint; verify the browser is live and retry',
    );
  }
  const viewport = requestedViewport(message.viewport);
  state = {
    associationKey: message.association_key, browserIncarnation: message.browser_incarnation,
    viewId: message.view_id, streamEpoch: message.stream_epoch, metadataSequence: 0,
    documentGeneration: nextGeneration(), frameGeneration: nextGeneration(), viewportRevision: 1, frameSequence: 0,
    leaseGeneration: 1, nextInputSequence: 1,
    requestedCssWidth: viewport.width, requestedCssHeight: viewport.height,
    viewportCssWidth: viewport.width, viewportCssHeight: viewport.height,
    visualOffsetX: 0, visualOffsetY: 0, visualScale: 1, pageScale: 1,
    geometryFresh: false, viewportTransition: false, captureToken: 0, captureBaseline: null,
    devicePixelRatio: viewport.dpr, scrollX: 0, scrollY: 0, inputViewportRevision: 1, targets: [], targetId: message.target_id,
    frameId: 'main', loaderId: null, url: '', title: '', frameGrant: message.frame_grant, loading: false,
    canGoBack: false, canGoForward: false, requestedUrl: null, controlled: false,
    focus: { page_focused: false, editable: false, selection_available: false, composition_active: false },
    cursor: null, pointer: null, pointerSampleSequence: 0, viewIds: new Set([message.view_id]), controllerViewId: null,
    cdpInputScale: viewport.dpr,
    pressedButtons: 0, heldKeys: new Map(),
  };
  if (typeof message.target_id !== 'string' || !message.target_id) {
    throw startupError('browser_helper_invalid_request', 'Stable CDP target identity is required');
  }
  registerViewGrant(message.view_id, message.frame_grant.grant, message.frame_grant.expires_at);
  await startFrameServer();
  await bindPage(message.target_id, false);
  await enumerateTargets();
  browserCdp.on('Target.targetCreated', () => { void refreshTargets().catch(() => {}); });
  browserCdp.on('Target.targetDestroyed', () => { void reconcileDestroyedTarget().catch(() => {}); });
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
  if (['resize', 'navigation', 'pointer', 'wheel', 'keyboard', 'text', 'composition', 'clipboard', 'tab', 'dialog', 'file', 'download', 'permission'].includes(request.command.type) && state.controllerViewId !== request.view_id) {
    return { status: 'rejected', ...base, code: 'browser_control_required', message: 'Another browser view holds the input lease' };
  }
  try {
    if (request.command.type === 'take_control') {
      if (state.controlled && state.controllerViewId !== request.view_id) {
        return { status: 'rejected', ...base, code: 'browser_control_required', message: 'Another browser view holds the input lease' };
      }
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
    if (request.command.type === 'resize') {
      requireControl(request.command);
      const previousViewport = {
        width: state.requestedCssWidth,
        height: state.requestedCssHeight,
        dpr: state.devicePixelRatio,
      };
      const expectedCdp = pageCdp;
      await stopScreencast(expectedCdp);
      invalidateViewport();
      // Rebind the observer closure as well as CDP capture. This prevents a
      // delayed callback from the retired screencast using the new baseline.
      pageBindingGeneration++;
      await installPageObservers();
      await applyRequestedViewport(request.command.viewport);
      if (state.requestedCssWidth !== previousViewport.width
        || state.requestedCssHeight !== previousViewport.height
        || state.devicePixelRatio !== previousViewport.dpr) {
        state.cdpInputScale = 1;
      }
      const changed = await updatePageState(page, expectedCdp, pageBindingGeneration);
      if (!state.geometryFresh) {
        emitEvent('viewport_changed', { viewport: viewportState() });
        return { status: 'accepted', ...base, outcome: { type: 'none' } };
      }
      await startScreencast(expectedCdp, pageBindingGeneration);
      await captureCurrentFrame(expectedCdp, pageBindingGeneration);
      if (changed) emitEvent('viewport_changed', { viewport: viewportState() });
      else emitEvent('viewport_changed', { viewport: viewportState() });
      return { status: 'accepted', ...base, outcome: { type: 'none' } };
    }
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
      const sequence = requireInputControl(request.command);
      const i = request.command.input;
      const cdpButton = i.kind === 'move'
        ? (i.buttons & 1 ? 'left' : i.buttons & 2 ? 'right' : i.buttons & 4 ? 'middle' : 'none')
        : i.button || 'none';
      const point = cdpInputPoint(i.x, i.y);
      await pageCdp.send('Input.dispatchMouseEvent', {
        type: i.kind === 'down' ? 'mousePressed' : i.kind === 'up' || i.kind === 'cancel' ? 'mouseReleased' : 'mouseMoved',
        x: point.x, y: point.y, button: cdpButton, buttons: i.buttons,
        clickCount: i.kind === 'move' ? 0 : i.click_count, modifiers: i.modifiers,
      });
      advanceInput(sequence);
      emitControl();
      state.pointer = { x: i.x, y: i.y };
      state.pressedButtons = i.buttons;
      state.pointerSampleSequence++;
      await updateCursor();
      return { status: 'accepted', ...base, outcome: { type: 'none' } };
    }
    if (request.command.type === 'wheel') {
      const sequence = requireInputControl(request.command); const i = request.command.input;
      if (!Number.isFinite(i.x) || !Number.isFinite(i.y) || i.x < 0 || i.y < 0
        || i.x >= state.viewportCssWidth || i.y >= state.viewportCssHeight) {
        return { status: 'rejected', ...base, code: 'invalid_wheel_coordinates', message: 'Wheel coordinates are outside the current browser viewport' };
      }
      const point = cdpInputPoint(i.x, i.y);
      await pageCdp.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: point.x, y: point.y, deltaX: i.delta_x_css, deltaY: i.delta_y_css, modifiers: i.modifiers });
      advanceInput(sequence); emitControl();
      return { status: 'accepted', ...base, outcome: { type: 'none' } };
    }
    if (request.command.type === 'keyboard') {
      const sequence = requireInputControl(request.command);
      const i = request.command.input;
      const text = i.kind === 'down' && (i.modifiers & 7) === 0
        ? ((i.key === 'Enter' || i.key === 'NumpadEnter') ? '\r' : i.key.length === 1 ? i.key : undefined)
        : undefined;
      await pageCdp.send('Input.dispatchKeyEvent', {
        type: i.kind === 'down' ? 'keyDown' : 'keyUp',
        key: i.key,
        code: i.code,
        location: i.location,
        modifiers: i.modifiers,
        autoRepeat: i.repeat,
        windowsVirtualKeyCode: virtualKeyCode(i.key, i.code),
        nativeVirtualKeyCode: virtualKeyCode(i.key, i.code),
        ...(text ? { text, unmodifiedText: text } : {}),
      });
      if (i.kind === 'down') state.heldKeys.set(i.code, i);
      else state.heldKeys.delete(i.code);
      advanceInput(sequence);
      emitControl();
      await updateFocus();
      emitFocus();
      return { status: 'accepted', ...base, outcome: { type: 'none' } };
    }
    if (request.command.type === 'composition') { const sequence = requireInputControl(request.command); const i = request.command.input; if (i.kind === 'commit') await pageCdp.send('Input.insertText', { text: i.text }); else await pageCdp.send('Input.imeSetComposition', { text: i.kind === 'cancel' ? '' : i.text, selectionStart: i.text.length, selectionEnd: i.text.length, replacementStart: 0, replacementEnd: 0 }); state.focus.composition_active = i.kind === 'start' || i.kind === 'update'; advanceInput(sequence); emitControl(); emitFocus(); return { status: 'accepted', ...base, outcome: { type: 'none' } }; }
    if (request.command.type === 'clipboard') { const sequence = requireInputControl(request.command); if (request.command.command.type === 'paste') { await pageCdp.send('Input.insertText', { text: request.command.command.text }); advanceInput(sequence); emitControl(); return { status: 'accepted', ...base, outcome: { type: 'clipboard', text: null } }; } const text = await page.evaluate(() => window.getSelection()?.toString().slice(0, 16384) || ''); advanceInput(sequence); emitControl(); return { status: 'accepted', ...base, outcome: { type: 'clipboard', text } }; }
    if (request.command.type === 'tab') {
      requireControl(request.command);
      const action = request.command.command;
      if (action.type === 'select') {
        await queueTargetTransition(async () => {
          await resetInputTransition();
          await bindPage(action.target_id);
        });
      } else if (action.type === 'create') {
        await queueTargetTransition(async () => {
          await resetInputTransition();
          const created = await context.newPage();
          if (action.url) await created.goto(validateNavigationUrl(action.url));
          const cdp = await context.newCDPSession(created);
          const targetId = (await cdp.send('Target.getTargetInfo')).targetInfo?.targetId;
          await cdp.detach();
          if (!targetId) throw new Error('created tab has no target');
          await bindPage(targetId);
        });
      } else if (action.type === 'close') {
        if (action.target_id === state.targetId) {
          await closeSelectedPage();
        } else {
          await queueTargetTransition(async () => {
            const closing = await pageForTarget(action.target_id);
            await closing.page.close();
          });
        }
      }
      await refreshTargets();
      return { status: 'accepted', ...base, outcome: { type: 'snapshot', snapshot: snapshot() } };
    }
    if (request.command.type === 'dialog') {
      requireControl(request.command);
      if (!pendingBlocker || pendingBlocker.blocker_id !== request.command.blocker_id || pendingBlocker.kind !== 'dialog') {
        throw Object.assign(new Error('stale dialog'), { code: 'stale_dialog' });
      }
      const blocker = pendingBlocker;
      await blocker.resolve(request.command.command.type, request.command.command.text);
      clearBlocker(blocker.blocker_id);
      return { status: 'accepted', ...base, outcome: { type: 'none' } };
    }
    if (['file', 'download', 'permission'].includes(request.command.type)) {
      requireControl(request.command);
      const blocker = pendingBlocker;
      if (blocker && blocker.blocker_id === request.command.blocker_id) {
        if (request.command.type === 'file' || request.command.type === 'download') await blocker.resolve('dismiss');
        clearBlocker(blocker.blocker_id);
      }
      return { status: 'unsupported', ...base, capability: request.command.type, message: `Browser ${request.command.type} handling is unavailable` };
    }
    if (request.command.type === 'inspect') {
      const i = request.command.command;
      const guard = inspectionGuard(i);
      if (guard.status !== 'ok') {
        return guard.status === 'stale'
          ? { ...base, ...guard, current_stream_epoch: state.streamEpoch, current_metadata_sequence: state.metadataSequence }
          : { ...base, ...guard };
      }

      // Inspection is deliberately read-only. DOM.getNodeForLocation is used
      // only to identify an inaccessible shadow boundary; it never dispatches
      // input, changes focus, or obtains the browser lease.
      const inspectedPage = page;
      const inspectedCdp = pageCdp;
      const inspectedBinding = pageBindingGeneration;
      const inspectedFrameGeneration = state.frameGeneration;
      let boundary = null;
      try {
        const hit = await inspectedCdp.send('DOM.getNodeForLocation', {
          x: i.x, y: i.y, includeUserAgentShadowDOM: false, ignorePointerEventsNone: false,
        });
        if (hit?.frameId && hit.frameId !== state.frameId) {
          return { status: 'unsupported', ...base, capability: 'inspection', code: 'inspection_frame_inaccessible', message: 'Element inspection cannot cross into a nested browsing context' };
        }
        if (Number.isSafeInteger(hit?.backendNodeId)) {
          const described = await inspectedCdp.send('DOM.describeNode', {
            backendNodeId: hit.backendNodeId, depth: 1, pierce: false,
          });
          if (described?.node?.shadowRoots?.some((root) => root.shadowRootType === 'closed')) {
            boundary = 'Element inspection cannot access a closed shadow root';
          }
        }
      } catch {
        // DOM inspection remains available through the narrow page probe when
        // the optional CDP node description is unavailable.
      }
      if (!inspectionStillCurrent(i, inspectedBinding, inspectedFrameGeneration)) {
        return { status: 'stale', ...base, current_stream_epoch: state.streamEpoch, current_metadata_sequence: state.metadataSequence, code: 'stale_location', message: 'The page or inspection geometry changed before DOM inspection' };
      }
      if (boundary) {
        return {
          status: 'accepted', ...base,
          outcome: { type: 'inspection', inspection: {
            location: i.location, frame_id: state.frameId, frame_generation: state.frameGeneration,
            pointer_sample_sequence: i.pointer_sample_sequence, bounds: null, evidence: null,
            inspectable: false, freshness: 'unavailable', limitation: boundary,
          } },
        };
      }
      let result;
      try {
        result = await inspectedPage.evaluate(({ x, y }) => {
        const e = document.elementFromPoint(x, y);
        if (!e) return { kind: 'empty', limitation: 'No page element is present at this point' };
        const tag = String(e.localName || '').toLowerCase();
        if (tag === 'iframe' || tag === 'frame') {
          return { kind: 'unsupported', limitation: 'Element inspection cannot cross into a nested browsing context' };
        }
        if (tag === 'canvas') {
          return { kind: 'unsupported', limitation: 'Canvas content has no inspectable inner DOM element' };
        }
        const r = e.getBoundingClientRect();
        const finiteRect = [r.x, r.y, r.width, r.height].every(Number.isFinite)
          && r.width >= 0 && r.height >= 0
          && r.width <= 2560 && r.height <= 1600
          && r.x >= -2560 && r.y >= -1600
          && r.x <= 5120 && r.y <= 3200;
        if (!finiteRect) return { kind: 'unsupported', limitation: 'The element geometry is outside bounded inspection limits' };
        const sensitive = (tag === 'input' && String(e.getAttribute('type') || '').toLowerCase() === 'password')
          || !!e.querySelector?.('input[type="password"]');
        const text = sensitive ? '' : (e.innerText || e.textContent || '').trim().slice(0, 1024);
        const excerpt = sensitive
          ? `<${tag}>`
          : String(e.outerHTML || `<${tag}>`)
            .replace(/\svalue\s*=\s*(['"]).*?\1/gi, ' value="[redacted]"')
            .slice(0, 1024);
        return {
          kind: 'element',
          bounds: { x: r.x, y: r.y, width: r.width, height: r.height },
          evidence: {
            tag,
            text,
            role: e.getAttribute('role')?.slice(0, 256) || null,
            name: (e.getAttribute('aria-label') || e.getAttribute('name'))?.slice(0, 256) || null,
            locators: [e.id ? `#${String(e.id).slice(0, 256)}` : tag],
            excerpt,
          },
        };
        }, { x: i.x, y: i.y });
      } catch (error) {
        if (!inspectionStillCurrent(i, inspectedBinding, inspectedFrameGeneration)) {
          return { status: 'stale', ...base, current_stream_epoch: state.streamEpoch, current_metadata_sequence: state.metadataSequence, code: 'stale_location', message: 'The page or inspection geometry changed during DOM inspection' };
        }
        throw error;
      }
      if (!inspectionStillCurrent(i, inspectedBinding, inspectedFrameGeneration)) {
        return { status: 'stale', ...base, current_stream_epoch: state.streamEpoch, current_metadata_sequence: state.metadataSequence, code: 'stale_location', message: 'The page or inspection geometry changed during DOM inspection' };
      }
      const inspectable = result?.kind === 'element';
      return {
        status: 'accepted',
        ...base,
        outcome: {
          type: 'inspection',
          inspection: {
            location: i.location,
            frame_id: state.frameId,
            frame_generation: state.frameGeneration,
            // Null is an explicit local sample. Never replace it with the
            // helper's latest cursor token.
            pointer_sample_sequence: i.pointer_sample_sequence,
            bounds: inspectable ? result.bounds : null,
            evidence: inspectable ? result.evidence : null,
            inspectable,
            freshness: inspectable ? 'fresh' : 'unavailable',
            limitation: inspectable ? null : (result?.limitation || 'No page element is present at this point'),
          },
        },
      };
    }
    if (request.command.type === 'capture') {
      const location = request.command.command.location;
      const presented = frameHistory.get(location.presented_frame_sequence);
      if (!presented
        || presented.target_id !== state.targetId
        || presented.stream_epoch !== state.streamEpoch
        || presented.document_generation !== state.documentGeneration
        || presented.viewport_revision !== state.viewportRevision
        || presented.frame_sequence !== location.presented_frame_sequence) {
        throw Object.assign(new Error('stale capture frame'), { code: 'stale_capture' });
      }
      return { status: 'accepted', ...base, outcome: { type: 'capture_prepared', capture_id: randomUUID(), descriptor: presented } };
    }
    return { status: 'unsupported', ...base, capability: request.command.type, message: 'This browser-view command is not implemented by the inline helper' };
  } catch (error) {
    if (request.command.type === 'navigation' && state.loading) {
      state.loading = false;
      emitNavigation();
    }
    const code = error?.code;
    if (code === 'stale_input_sequence') emitControl();
    if (['browser_control_required', 'stale_input_sequence', 'stale_control', 'stale_dialog', 'stale_pointer', 'stale_capture', 'browser_last_page'].includes(code)) {
      return { status: 'rejected', ...base, code, message: String(error.message || error) };
    }
    return { status: 'outcome_unknown', ...base, code: 'browser_command_uncertain', message: String(error.message || error) };
  }
}
async function detach() {
  await releaseHeldInput();
  await stopScreencast();
  grants.clear();
  for (const viewId of grantTimers.keys()) clearGrantTimer(viewId);
  viewGrants.clear();
  grantViews.clear();
  attachedViewIds.clear();
  resetFrameTransport();
  for (const socket of sockets) socket.terminate();
  sockets.clear();
  server?.close();
  await stopDummyPageServer();
  try { await browser?.disconnect(); } catch {}
  process.exit(0);
}
const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
let inputQueue = Promise.resolve();
let inputQueueDepth = 0;
let urgentInputQueue = Promise.resolve();
let urgentInputQueueDepth = 0;
async function processInputLine(line) {
  let message;
  try {
    message = JSON.parse(line);
    if (message.type === 'attach') await attach(message);
    else if (message.type === 'command') emit({ type: 'command_response', response: await command(message.request) });
    else if (message.type === 'grant') {
      registerViewGrant(message.grant.view_id, message.grant.grant, message.grant.expires_at);
    } else if (message.type === 'attach_view') {
      const hadViews = state.viewIds.size > 0;
      state.viewIds.add(message.view_id);
      registerViewGrant(message.view_id, message.frame_grant.grant, message.frame_grant.expires_at);
      if (!hadViews && state.viewIds.size > 0) await startScreencast();
    } else if (message.type === 'detach_view') {
      retireView(message.view_id, false);
    } else if (message.type === 'pause') {
      await releaseHeldInput();
      if (state.controlled) {
        state.controlled = false;
        state.controllerViewId = null;
        state.leaseGeneration++;
        state.nextInputSequence = 1;
        emitControl();
      }
      for (const socket of sockets) {
        socket.authorized = false;
        socket.terminate();
      }
      for (const grant of viewGrants.values()) grants.delete(grant);
      for (const viewId of grantTimers.keys()) clearGrantTimer(viewId);
      viewGrants.clear();
      grantViews.clear();
      attachedViewIds.clear();
      state.viewIds.clear();
      resetFrameTransport();
      await stopScreencast();
    }
    else if (message.type === 'resume') await startScreencast();
    else if (message.type === 'detach') await detach();
    else if (message.type === 'stop') await detach();
  } catch (error) {
    const rawMessage = String(error?.message || error);
    const code = typeof error?.code === 'string'
      ? error.code
      : (message?.type === 'attach' ? 'browser_helper_runtime_failed' : 'browser_helper_failed');
    const safeMessage = typeof error?.code === 'string'
      ? String(error.message || 'browser helper operation failed').slice(0, 256)
      : 'Browser helper operation failed; retry the browser view';
    emit({ type: 'failed', code, message: safeMessage });
    if (/Target page, context or browser has been closed|Target closed|Connection closed/i.test(rawMessage)) await detach();
  }
}
const MAX_COMMAND_QUEUE = 64;
const MAX_URGENT_QUEUE = 16;
function urgentInputLine(line) {
  try {
    const message = JSON.parse(line);
    if (['detach', 'detach_view', 'stop', 'pause'].includes(message.type)) return true;
    const command = message.type === 'command' ? message.request?.command : null;
    return command?.type === 'release_control'
      || command?.type === 'dialog'
      || (command?.type === 'navigation' && command.command?.type === 'stop');
  } catch {
    return false;
  }
}
input.on('line', (line) => {
  if (urgentInputLine(line)) {
    if (urgentInputQueueDepth >= MAX_URGENT_QUEUE) {
      emit({ type: 'failed', code: 'browser_command_busy', message: 'Urgent browser command queue is full; retry after the current operation settles' });
      return;
    }
    urgentInputQueueDepth++;
    urgentInputQueue = urgentInputQueue
      .then(() => processInputLine(line), () => processInputLine(line))
      .finally(() => { urgentInputQueueDepth--; });
    return;
  }
  if (inputQueueDepth >= MAX_COMMAND_QUEUE) {
    emit({ type: 'failed', code: 'browser_command_busy', message: 'Browser command queue is full; retry after the current operation settles' });
    return;
  }
  inputQueueDepth++;
  inputQueue = inputQueue
    .then(() => processInputLine(line), () => processInputLine(line))
    .finally(() => { inputQueueDepth--; });
});
process.on('uncaughtException', () => {
  emit({
    type: 'failed',
    code: 'browser_helper_crashed',
    message: 'Browser helper runtime crashed; retry the browser view',
  });
  process.exit(1);
});
