const invoke = window.__TAURI__?.core?.invoke;
const eventApi = window.__TAURI__?.event;
const panel = document.querySelector('#browser-panel');
const image = document.querySelector('#screenshot');
const emptyState = document.querySelector('#empty-state');
const status = document.querySelector('#status');
const address = document.querySelector('#url');
const title = document.querySelector('#page-title');
const dimensions = document.querySelector('#dimensions');
const pageUrl = document.querySelector('#page-url');
const activeElement = document.querySelector('#active-element');
const fixtureStatus = document.querySelector('#fixture-status');
const scrollY = document.querySelector('#scroll-y');
const selectedText = document.querySelector('#selected-text');
const lastAction = document.querySelector('#last-action');

const SAFE_CURSORS = new Set(['default', 'auto', 'pointer', 'text', 'crosshair', 'move', 'not-allowed', 'wait', 'grab', 'grabbing', 'cell', 'help', 'progress', 'zoom-in', 'zoom-out', 'col-resize', 'row-resize', 'e-resize', 'w-resize', 'n-resize', 's-resize']);

let currentSnapshot;
let streamAbort;
let reconnectTimer;
let streamGeneration = 0;
let streamBuffer = new Uint8Array();
let streamDiagnostic = 'not connected';
let pendingPacket;
let frameAnimation;
let displayedFrameSequence = 0;
let activeObjectUrl;
let pendingObjectUrl;
let pendingImageGeometry;
let renderedMjpegParts = 0;
let inputQueue = Promise.resolve();
let inputBusy = false;
let dragFrame;
let pendingDragMove;
let wheelFrame;
let pendingWheel;
let selfTestStarted = false;
let selfTestRequested = false;
let pendingLowPriority = { pointerMove: undefined, wheel: undefined };
let lowPrioritySequence = 0;
let lowPriorityRunning = false;

function setStatus(message, kind = 'idle') { status.textContent = message; status.dataset.kind = kind; }
function reportAction(message) { lastAction.textContent = message; }
function applyCursor(cursor) { panel.style.cursor = SAFE_CURSORS.has(cursor) ? cursor : 'default'; }
function validDimension(value) { return Number.isFinite(value) && value > 0 && value <= 16_384; }
function normalizeGeometry(frame) {
  const viewport = frame?.viewport;
  if (!Number.isSafeInteger(frame?.sequence) || frame.sequence <= 0 || !validDimension(frame?.pixelWidth) || !validDimension(frame?.pixelHeight) || !viewport || !validDimension(viewport.width) || !validDimension(viewport.height)) return null;
  return { sequence: frame.sequence, pixelWidth: Math.round(frame.pixelWidth), pixelHeight: Math.round(frame.pixelHeight), viewport: { width: Math.round(viewport.width), height: Math.round(viewport.height), scale: Number.isFinite(viewport.scale) && viewport.scale > 0 ? viewport.scale : 1, offsetX: Number.isFinite(viewport.offsetX) ? viewport.offsetX : 0, offsetY: Number.isFinite(viewport.offsetY) ? viewport.offsetY : 0 } };
}
function applyGeometry(frame) {
  const geometry = normalizeGeometry(frame); if (!geometry) return false;
  currentSnapshot = { ...currentSnapshot, ...geometry };
  panel.style.aspectRatio = `${geometry.pixelWidth} / ${geometry.pixelHeight}`;
  dimensions.textContent = `${geometry.pixelWidth} × ${geometry.pixelHeight} px · ${geometry.viewport.width} × ${geometry.viewport.height} CSS · @${geometry.viewport.scale.toFixed(2)} (${geometry.viewport.offsetX.toFixed(0)}, ${geometry.viewport.offsetY.toFixed(0)})`;
  return true;
}
function appendBytes(left, right) { const combined = new Uint8Array(left.byteLength + right.byteLength); combined.set(left); combined.set(right, left.byteLength); return combined; }
function indexOfBytes(bytes, needle, start = 0) { outer: for (let i = start; i <= bytes.byteLength - needle.byteLength; i += 1) { for (let j = 0; j < needle.byteLength; j += 1) if (bytes[i + j] !== needle[j]) continue outer; return i; } return -1; }
function headerNumber(headers, name, min, max, integral = false) { const line = headers.split(/\r?\n/).find((entry) => entry.toLowerCase().startsWith(`${name}:`)); const value = Number(line?.slice(name.length + 1).trim()); return Number.isFinite(value) && (!integral || Number.isInteger(value)) && value >= min && value <= max ? value : null; }
function parsePacket(headers, jpeg) {
  const frame = { sequence: headerNumber(headers, 'x-frame-sequence', 1, 0xffffffff, true), pixelWidth: headerNumber(headers, 'x-pixel-width', 1, 16384, true), pixelHeight: headerNumber(headers, 'x-pixel-height', 1, 16384, true), viewport: { width: headerNumber(headers, 'x-viewport-width', 1, 16384, true), height: headerNumber(headers, 'x-viewport-height', 1, 16384, true), scale: headerNumber(headers, 'x-viewport-scale', Number.MIN_VALUE, 100), offsetX: headerNumber(headers, 'x-viewport-offset-x', -100000, 100000), offsetY: headerNumber(headers, 'x-viewport-offset-y', -100000, 100000) } };
  const geometry = normalizeGeometry(frame);
  return geometry && jpeg[0] === 0xff && jpeg[1] === 0xd8 ? { geometry, jpeg } : null;
}
function discardPendingRender() {
  if (frameAnimation !== undefined) cancelAnimationFrame(frameAnimation);
  frameAnimation = undefined; pendingPacket = undefined; pendingImageGeometry = undefined;
  if (pendingObjectUrl) URL.revokeObjectURL(pendingObjectUrl);
  pendingObjectUrl = undefined;
}
function scheduleFrameRender() {
  if (frameAnimation !== undefined) return;
  frameAnimation = requestAnimationFrame(() => {
    frameAnimation = undefined;
    const packet = pendingPacket; pendingPacket = undefined;
    if (!packet || packet.geometry.sequence <= displayedFrameSequence) return;
    if (pendingObjectUrl) URL.revokeObjectURL(pendingObjectUrl);
    const generation = streamGeneration;
    const objectUrl = URL.createObjectURL(new Blob([packet.jpeg], { type: 'image/jpeg' }));
    pendingObjectUrl = objectUrl;
    const decoder = new Image();
    decoder.onload = () => {
      if (generation !== streamGeneration || pendingObjectUrl !== objectUrl) return;
      if (decoder.naturalWidth !== packet.geometry.pixelWidth || decoder.naturalHeight !== packet.geometry.pixelHeight) return;
      if (activeObjectUrl) URL.revokeObjectURL(activeObjectUrl);
      activeObjectUrl = objectUrl; pendingObjectUrl = undefined;
      displayedFrameSequence = packet.geometry.sequence; renderedMjpegParts += 1; applyGeometry(packet.geometry);
      image.src = objectUrl; image.hidden = false; emptyState.hidden = true;
    };
    decoder.onerror = () => { if (generation === streamGeneration && pendingObjectUrl === objectUrl) { URL.revokeObjectURL(objectUrl); pendingObjectUrl = undefined; } };
    decoder.src = objectUrl;
  });
}
function consumeMjpeg(chunk, boundary) {
  streamBuffer = appendBytes(streamBuffer, chunk);
  const marker = new TextEncoder().encode(`--${boundary}\r\n`);
  const endHeaders = new TextEncoder().encode('\r\n\r\n');
  while (true) {
    const start = indexOfBytes(streamBuffer, marker);
    if (start < 0) { if (streamBuffer.byteLength > 6 * 1024 * 1024 + 4096) throw new Error('MJPEG partial frame exceeded limit'); return; }
    if (start > 0) streamBuffer = streamBuffer.slice(start);
    const headerEnd = indexOfBytes(streamBuffer, endHeaders, marker.byteLength);
    if (headerEnd < 0) { if (streamBuffer.byteLength > marker.byteLength + 4096) throw new Error('MJPEG headers exceeded limit'); return; }
    const headers = new TextDecoder('ascii').decode(streamBuffer.slice(marker.byteLength, headerEnd));
    const length = headerNumber(headers, 'content-length', 1, 6 * 1024 * 1024, true);
    if (length === null) throw new Error('MJPEG part had an invalid Content-Length');
    const bodyStart = headerEnd + endHeaders.byteLength; const bodyEnd = bodyStart + length;
    if (streamBuffer.byteLength < bodyEnd + 2) return;
    if (streamBuffer[bodyEnd] !== 13 || streamBuffer[bodyEnd + 1] !== 10) throw new Error('MJPEG part trailer was invalid');
    const packet = parsePacket(headers, streamBuffer.slice(bodyStart, bodyEnd));
    streamBuffer = streamBuffer.slice(bodyEnd + 2);
    if (!packet) throw new Error('MJPEG part had invalid JPEG or geometry headers');
    pendingPacket = packet; // one replaceable unrendered JPEG, never a FIFO
    scheduleFrameRender();
  }
}
async function connectMjpegStream(streamUrl) {
  clearTimeout(reconnectTimer); streamAbort?.abort(); discardPendingRender(); streamBuffer = new Uint8Array();
  const generation = ++streamGeneration; const controller = new AbortController(); streamAbort = controller;
  try {
    streamDiagnostic = 'fetching';
    const response = await fetch(streamUrl, { cache: 'no-store', signal: controller.signal });
    const contentType = response.headers.get('content-type') || '';
    streamDiagnostic = `response ${response.status} ${contentType} body=${Boolean(response.body)}`;
    // WebKitGTK rejects fetch() of multipart/x-mixed-replace before exposing a
    // body. The helper keeps multipart bytes but uses octet-stream and this
    // explicit boundary header as a native fetch compatibility workaround.
    const boundary = response.headers.get('x-mjpeg-boundary');
    if (!response.ok || contentType !== 'application/octet-stream' || !boundary || !/^[A-Za-z0-9-]{1,80}$/.test(boundary) || !response.body) throw new Error(`MJPEG response was ${response.status}`);
    reportAction('MJPEG stream connected');
    const reader = response.body.getReader();
    streamDiagnostic = 'reading';
    while (generation === streamGeneration) { const { done, value } = await reader.read(); if (generation !== streamGeneration) return; if (done) throw new Error('MJPEG stream ended'); if (value) consumeMjpeg(value, boundary); }
  } catch (error) {
    if (controller.signal.aborted || generation !== streamGeneration) return;
    streamDiagnostic = `error ${String(error)}`;
    reportAction(`MJPEG reconnecting: ${String(error)}`);
    reconnectTimer = setTimeout(() => { void connectMjpegStream(streamUrl); }, 500);
  }
}
function applySnapshot(snapshot) {
  if (!snapshot?.streamUrl) throw new Error('Start response omitted the loopback MJPEG URL');
  currentSnapshot = { ...currentSnapshot, ...snapshot }; applyGeometry({ ...snapshot, sequence: snapshot.sequence || 1 });
  title.textContent = snapshot.title || 'Untitled page'; pageUrl.textContent = snapshot.url || '—'; applyCursor(snapshot.cursor);
  if (document.activeElement !== address) address.value = snapshot.url || '';
}
image.addEventListener('error', () => { reportAction('MJPEG image display failed'); });

function applyInputAck(ack) {
  if (!ack || typeof ack.cursor !== 'string') throw new Error('Input acknowledgement omitted cursor');
  applyCursor(ack.cursor);
}

async function command(name, args = {}) {
  if (!invoke) throw new Error('Tauri global API is unavailable; launch this page through Tauri');
  return invoke(name, args);
}

async function start() {
  setStatus('Launching…');
  try {
    const snapshot = await command('browser_start');
    applySnapshot(snapshot);
    connectMjpegStream(snapshot.streamUrl);
    setStatus('Ready', 'ok');
    reportAction('Chromium control channel started');
    if (await command('browser_self_test_enabled')) { selfTestRequested = true; void runSelfTest(); }
  } catch (error) {
    setStatus(String(error), 'error');
    emptyState.textContent = `Unable to start browser: ${error}`;
    reportAction('Startup failed');
  }
}

function queueInput(event, description, quiet = false, strict = false) {
  const operation = inputQueue.then(async () => {
    inputBusy = true;
    applyInputAck(await command('browser_input', { event }));
    if (!quiet) { setStatus('Ready', 'ok'); reportAction(description); }
  });
  inputQueue = operation.catch((error) => { setStatus(String(error), 'error'); reportAction(`${description} failed`); }).finally(() => { inputBusy = false; });
  return strict ? operation : inputQueue;
}

function queueLatestInput(event, description) {
  const slot = event.kind === 'wheel' ? 'wheel' : 'pointerMove';
  pendingLowPriority[slot] = { event, description, sequence: ++lowPrioritySequence };
  if (lowPriorityRunning) return;
  lowPriorityRunning = true;
  void (async () => {
    while (pendingLowPriority.pointerMove || pendingLowPriority.wheel) {
      const slotToRun = !pendingLowPriority.wheel || (pendingLowPriority.pointerMove && pendingLowPriority.pointerMove.sequence < pendingLowPriority.wheel.sequence) ? 'pointerMove' : 'wheel';
      const next = pendingLowPriority[slotToRun];
      pendingLowPriority[slotToRun] = undefined;
      await queueInput(next.event, next.description, true);
    }
    lowPriorityRunning = false;
  })();
}

function modifierMask(event) { return (event.altKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.metaKey ? 4 : 0) | (event.shiftKey ? 8 : 0); }
function panelPoint(x, y) {
  const rect = image.getBoundingClientRect();
  const viewport = currentSnapshot?.viewport;
  return { clientX: rect.left + x * rect.width / viewport.width, clientY: rect.top + y * rect.height / viewport.height };
}
function dispatchPanelPointer(type, x, y, options = {}) { return panel.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, ...panelPoint(x, y), button: options.button ?? -1, buttons: options.buttons ?? 0 })); }
function dispatchPanelWheel(x, y, deltaY) { return panel.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, ...panelPoint(x, y), deltaMode: 0, deltaY })); }

async function waitForInputIdle() {
  while (true) {
    await inputQueue;
    if (!inputBusy && !lowPriorityRunning && dragFrame === undefined && wheelFrame === undefined) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}
async function clickPanel(x, y) { dispatchPanelPointer('pointerdown', x, y, { button: 0, buttons: 1 }); dispatchPanelPointer('pointerup', x, y, { button: 0, buttons: 0 }); await waitForInputIdle(); }
async function typePanelKey(key, code) { panel.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, key, code })); panel.dispatchEvent(new KeyboardEvent('keyup', { bubbles: true, cancelable: true, key, code })); await waitForInputIdle(); }

async function waitUntil(predicate, description, timeoutMs = 8_000) {
  const deadline = performance.now() + timeoutMs;
  while (!predicate()) {
    if (performance.now() >= deadline) throw new Error(`timed out waiting for ${description}`);
    await new Promise((resolve) => requestAnimationFrame(resolve));
  }
}

async function waitForRenderedFrame() {
  await waitUntil(() => renderedMjpegParts > 0 && image.complete && image.naturalWidth > 0 && image.naturalHeight > 0, 'a rendered MJPEG part');
  if (!currentSnapshot || image.naturalWidth !== currentSnapshot.pixelWidth || image.naturalHeight !== currentSnapshot.pixelHeight) throw new Error('rendered MJPEG dimensions disagreed with correlated part geometry');
}

async function runSelfTest() {
  if (selfTestStarted || !selfTestRequested || !currentSnapshot?.viewport) return;
  selfTestStarted = true;
  setStatus('Self-test running…');
  try {
    await waitForRenderedFrame();
    if (!image.src.startsWith('blob:')) throw new Error('frontend did not atomically render the parsed MJPEG part');
    if (!Number.isFinite(currentSnapshot.viewport.offsetX) || !Number.isFinite(currentSnapshot.viewport.offsetY)) throw new Error('MJPEG headers omitted viewport offsets');
    let focused;
    for (const [x, y] of [[300, 230], [300, 255], [300, 280], [300, 305]]) {
      await clickPanel(x, y); focused = await command('browser_inspect'); if (focused.activeElement === 'INPUT#name') break;
    }
    if (focused?.activeElement !== 'INPUT#name') throw new Error(`input focus was ${focused?.activeElement || 'unknown'}`);
    const beforeFrames = renderedMjpegParts;
    for (const [key, code] of [['A', 'KeyA'], ['d', 'KeyD'], ['a', 'KeyA']]) await typePanelKey(key, code);
    await waitUntil(() => renderedMjpegParts > beforeFrames, 'updated correlated MJPEG part');
    let result;
    for (const [x, y] of [[820, 230], [820, 255], [820, 280], [820, 305], [800, 255], [840, 255]]) {
      await clickPanel(x, y); result = await command('browser_inspect'); if (result.fixtureStatus?.includes('Hello, Ada')) break;
    }
    if (!result?.fixtureStatus?.includes('Hello, Ada')) throw new Error(`fixture status was ${result?.fixtureStatus || 'empty'}`);
    const beforeScroll = await command('browser_inspect');
    dispatchPanelPointer('pointermove', 500, 600, { buttons: 0 }); dispatchPanelWheel(500, 600, 500); await waitForInputIdle();
    const afterScroll = await command('browser_inspect');
    if (afterScroll.scrollY <= beforeScroll.scrollY) throw new Error(`scrollY stayed at ${beforeScroll.scrollY}`);
    dispatchPanelPointer('pointermove', 210, 120, { buttons: 0 }); dispatchPanelPointer('pointerdown', 210, 120, { button: 0, buttons: 1 }); dispatchPanelPointer('pointermove', 620, 120, { button: 0, buttons: 1 }); dispatchPanelPointer('pointerup', 620, 120, { button: 0, buttons: 0 });
    await waitForInputIdle();
    result = await command('browser_inspect');
    if (!result.selectedText?.length) throw new Error('drag input did not select fixture text');
    applySnapshot(await command('browser_reload'));
    result = await command('browser_inspect');
    if (result.url !== currentSnapshot.url) throw new Error('reload did not preserve the inspected page URL');
    fixtureStatus.textContent = result.fixtureStatus || '—'; activeElement.textContent = result.activeElement || '—'; scrollY.textContent = String(result.scrollY ?? 0); selectedText.textContent = result.selectedText || '—';
    await command('browser_self_test_report', { success: true, detail: 'correlated MJPEG parts and headers, pointer, keyboard, wheel, selection, reload, and inspect handlers' });
    setStatus('Self-test passed', 'ok'); reportAction('Native self-test passed');
  } catch (error) {
    try { await command('browser_self_test_report', { success: false, detail: `${String(error).slice(0, 400)} [stream=${streamDiagnostic}]` }); } catch {}
    setStatus(`Self-test failed: ${error}`, 'error'); reportAction('Native self-test failed');
  }
}

function clampedImageCoordinates(event) {
  const viewport = currentSnapshot?.viewport;
  const rect = image.getBoundingClientRect();
  if (!viewport || !rect.width || !rect.height) return null;
  return { x: Math.max(0, Math.min(viewport.width, (event.clientX - rect.left) * viewport.width / rect.width)), y: Math.max(0, Math.min(viewport.height, (event.clientY - rect.top) * viewport.height / rect.height)) };
}
function imageCoordinates(event) {
  const coordinates = clampedImageCoordinates(event);
  const rect = image.getBoundingClientRect();
  if (!coordinates || event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) return null;
  return coordinates;
}
function buttonName(button) { return button === 1 ? 'middle' : button === 2 ? 'right' : 'left'; }
function buttonMask(button) { return button === 'left' ? 1 : button === 'right' ? 2 : 4; }
let activePointer = false;
let lastPointer = null;
let activeButton = 'left';
function scheduleAnimation(callback) { return requestAnimationFrame(callback); }
function cancelAnimation(id) { cancelAnimationFrame(id); }
function flushDragMove() { dragFrame = undefined; const move = pendingDragMove; pendingDragMove = undefined; if (move) queueLatestInput(move, 'Pointer moved'); }
function flushWheel() {
  wheelFrame = undefined;
  if (pendingDragMove) { if (dragFrame !== undefined) { cancelAnimation(dragFrame); dragFrame = undefined; } flushDragMove(); }
  const wheel = pendingWheel; pendingWheel = undefined; if (wheel) queueLatestInput(wheel, 'Wheel scrolled');
}
panel.addEventListener('pointermove', (event) => {
  const coordinates = activePointer ? clampedImageCoordinates(event) : imageCoordinates(event); if (!coordinates) return;
  lastPointer = coordinates; pendingDragMove = { kind: 'mouseMove', ...coordinates, buttons: event.buttons, modifiers: modifierMask(event) };
  if (dragFrame === undefined) dragFrame = scheduleAnimation(flushDragMove);
});
panel.addEventListener('pointerdown', (event) => {
  const coordinates = imageCoordinates(event); if (!coordinates) return;
  panel.focus({ preventScroll: true }); lastPointer = coordinates; activePointer = true; activeButton = buttonName(event.button);
  try { panel.setPointerCapture?.(event.pointerId); } catch {}
  queueInput({ kind: 'mouseDown', ...coordinates, button: activeButton, buttons: event.buttons, modifiers: modifierMask(event) }, 'Pointer pressed');
});
function finishPointer(event, description) {
  if (!activePointer || !lastPointer) return;
  if (dragFrame !== undefined) { cancelAnimation(dragFrame); dragFrame = undefined; }
  pendingDragMove = undefined; pendingLowPriority.pointerMove = undefined;
  const coordinates = clampedImageCoordinates(event) || lastPointer; activePointer = false;
  try { panel.releasePointerCapture?.(event.pointerId); } catch {}
  queueInput({ kind: 'mouseMove', ...coordinates, buttons: event.buttons || buttonMask(activeButton), modifiers: modifierMask(event) }, 'Pointer moved', true);
  queueInput({ kind: 'mouseUp', ...coordinates, button: activeButton, buttons: 0, modifiers: modifierMask(event) }, description); lastPointer = null;
}
panel.addEventListener('pointerup', (event) => finishPointer(event, 'Pointer released'));
panel.addEventListener('pointercancel', (event) => finishPointer(event, 'Pointer cancelled'));
panel.addEventListener('lostpointercapture', (event) => finishPointer(event, 'Pointer capture lost'));
panel.addEventListener('wheel', (event) => {
  const coordinates = imageCoordinates(event); if (!coordinates) return;
  event.preventDefault();
  const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? currentSnapshot.viewport.height : 1;
  pendingWheel = { kind: 'wheel', ...coordinates, deltaX: Math.max(-10000, Math.min(10000, (pendingWheel?.deltaX || 0) + event.deltaX * unit)), deltaY: Math.max(-10000, Math.min(10000, (pendingWheel?.deltaY || 0) + event.deltaY * unit)), modifiers: modifierMask(event) };
  if (wheelFrame === undefined) wheelFrame = scheduleAnimation(flushWheel);
}, { passive: false });
panel.addEventListener('keydown', (event) => { if (event.isComposing) return; event.preventDefault(); queueInput({ kind: 'keyDown', key: event.key, code: event.code, text: event.key.length === 1 ? event.key : undefined, modifiers: modifierMask(event) }, `Key down: ${event.key}`); });
panel.addEventListener('keyup', (event) => { if (event.isComposing) return; event.preventDefault(); queueInput({ kind: 'keyUp', key: event.key, code: event.code, modifiers: modifierMask(event) }, `Key up: ${event.key}`); });
panel.addEventListener('compositionend', (event) => { if (!event.data) return; queueInput({ kind: 'keyDown', key: event.data.slice(0, 128), code: 'Unidentified', text: event.data.slice(0, 16) }, 'Text composition committed'); queueInput({ kind: 'keyUp', key: event.data.slice(0, 128), code: 'Unidentified' }, 'Text composition released'); });

document.querySelector('#navigation').addEventListener('submit', async (event) => {
  event.preventDefault(); setStatus('Navigating…');
  try { applySnapshot(await command('browser_navigate', { url: address.value.trim() })); setStatus('Ready', 'ok'); reportAction('Navigated to URL'); panel.focus({ preventScroll: true }); }
  catch (error) { setStatus(String(error), 'error'); reportAction('Navigation rejected'); }
});
document.querySelector('#reload').addEventListener('click', async () => {
  setStatus('Refreshing…');
  try { applySnapshot(await command('browser_reload')); setStatus('Ready', 'ok'); reportAction('Fixture refreshed'); }
  catch (error) { setStatus(String(error), 'error'); reportAction('Refresh failed'); }
});
document.querySelector('#inspect').addEventListener('click', async () => {
  setStatus('Inspecting…');
  try {
    const result = await command('browser_inspect'); title.textContent = result.title || 'Untitled page'; pageUrl.textContent = result.url; activeElement.textContent = result.activeElement; fixtureStatus.textContent = result.fixtureStatus || '—'; scrollY.textContent = String(result.scrollY ?? 0); selectedText.textContent = result.selectedText || '—'; setStatus('Ready', 'ok'); reportAction('Inspected fixed page state');
  } catch (error) { setStatus(String(error), 'error'); reportAction('Inspect failed'); }
});
if (eventApi?.listen) void eventApi.listen('interactive-browser-panel-self-test', () => { selfTestRequested = true; void runSelfTest(); });
window.addEventListener('beforeunload', () => { clearTimeout(reconnectTimer); streamAbort?.abort(); if (activeObjectUrl) URL.revokeObjectURL(activeObjectUrl); if (pendingObjectUrl) URL.revokeObjectURL(pendingObjectUrl); void command('browser_stop').catch(() => {}); });
start();
