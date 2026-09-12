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

const FRAME_MAGIC = 0x49504246; // "IPBF"
const FRAME_VERSION = 1;
const FRAME_HEADER_BYTES = 48;
const MAX_FRAME_BYTES = 6 * 1024 * 1024;
const SAFE_CURSORS = new Set(['default', 'auto', 'pointer', 'text', 'crosshair', 'move', 'not-allowed', 'wait', 'grab', 'grabbing', 'cell', 'help', 'progress', 'zoom-in', 'zoom-out', 'col-resize', 'row-resize', 'e-resize', 'w-resize', 'n-resize', 's-resize']);

let currentSnapshot;
let frameSocket;
let reconnectTimer;
let latestFramePacket;
let frameAnimation;
let newestFrameSequence = 0;
let displayedFrameSequence = 0;
let activeObjectUrl;
let pendingObjectUrl;
let pendingObjectSequence = 0;
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
  if (!validDimension(frame?.pixelWidth) || !validDimension(frame?.pixelHeight) ||
    !viewport || !validDimension(viewport.width) || !validDimension(viewport.height)) return null;
  return {
    pixelWidth: Math.round(frame.pixelWidth),
    pixelHeight: Math.round(frame.pixelHeight),
    viewport: {
      width: Math.round(viewport.width), height: Math.round(viewport.height),
      scale: Number.isFinite(viewport.scale) && viewport.scale > 0 ? viewport.scale : 1,
      offsetX: Number.isFinite(viewport.offsetX) ? viewport.offsetX : 0,
      offsetY: Number.isFinite(viewport.offsetY) ? viewport.offsetY : 0,
    },
  };
}

function applyGeometry(frame) {
  const geometry = normalizeGeometry(frame);
  if (!geometry) return false;
  currentSnapshot = { ...currentSnapshot, ...geometry };
  panel.style.aspectRatio = `${geometry.pixelWidth} / ${geometry.pixelHeight}`;
  dimensions.textContent = `${geometry.pixelWidth} × ${geometry.pixelHeight} px · ${geometry.viewport.width} × ${geometry.viewport.height} CSS`;
  return true;
}

function applySnapshot(snapshot) {
  if (!snapshot?.streamUrl) throw new Error('Start response omitted the binary frame stream URL');
  currentSnapshot = { ...currentSnapshot, ...snapshot };
  applyGeometry(snapshot);
  title.textContent = snapshot.title || 'Untitled page';
  pageUrl.textContent = snapshot.url || '—';
  applyCursor(snapshot.cursor);
  if (document.activeElement !== address) address.value = snapshot.url || '';
}

function parseFrame(buffer) {
  if (!(buffer instanceof ArrayBuffer) || buffer.byteLength < FRAME_HEADER_BYTES) return null;
  const view = new DataView(buffer);
  if (view.getUint32(0) !== FRAME_MAGIC || view.getUint8(4) !== FRAME_VERSION || view.getUint16(6) !== FRAME_HEADER_BYTES) return null;
  const jpegLength = view.getUint32(40);
  if (!jpegLength || jpegLength > MAX_FRAME_BYTES || buffer.byteLength !== FRAME_HEADER_BYTES + jpegLength) return null;
  const frame = {
    sequence: view.getUint32(8),
    pixelWidth: view.getUint32(12),
    pixelHeight: view.getUint32(16),
    viewport: {
      width: view.getUint32(20), height: view.getUint32(24), scale: view.getFloat32(28),
      offsetX: view.getFloat32(32), offsetY: view.getFloat32(36),
    },
    jpeg: buffer.slice(FRAME_HEADER_BYTES),
  };
  return normalizeGeometry(frame) ? frame : null;
}

function acknowledgeFrame(socket, sequence) {
  if (frameSocket === socket && socket.readyState === WebSocket.OPEN) socket.send(`ack:${sequence}`);
}

function scheduleFrameRender() {
  if (frameAnimation !== undefined) return;
  frameAnimation = requestAnimationFrame(() => {
    frameAnimation = undefined;
    const packet = latestFramePacket;
    latestFramePacket = undefined; // one newest frame per animation frame, never a queue
    // A scheduled RAF can outlive a socket reconnect. Its ACK must never be
    // sent to a newer connection, which has its own replayed current frame.
    if (!packet || frameSocket !== packet.socket) return;
    const frame = parseFrame(packet.buffer);
    if (!frame) return;
    // A reconnect can replay the current frame. ACK it on that same socket even
    // when its pixels are already displayed so its delivery slot cannot stall.
    if (frame.sequence <= newestFrameSequence) {
      acknowledgeFrame(packet.socket, frame.sequence);
      return;
    }
    newestFrameSequence = frame.sequence;
    if (pendingObjectUrl) URL.revokeObjectURL(pendingObjectUrl);
    pendingObjectUrl = URL.createObjectURL(new Blob([frame.jpeg], { type: 'image/jpeg' }));
    pendingObjectSequence = frame.sequence;
    image.dataset.frameSequence = String(frame.sequence);
    image.dataset.frameGeometry = JSON.stringify({ pixelWidth: frame.pixelWidth, pixelHeight: frame.pixelHeight, viewport: frame.viewport });
    image.src = pendingObjectUrl;
    acknowledgeFrame(packet.socket, frame.sequence);
  });
}

image.addEventListener('load', () => {
  const sequence = Number(image.dataset.frameSequence);
  if (!Number.isInteger(sequence) || sequence !== pendingObjectSequence || sequence < displayedFrameSequence) return;
  let geometry;
  try { geometry = JSON.parse(image.dataset.frameGeometry || ''); } catch { return; }
  if (!applyGeometry(geometry)) return;
  if (activeObjectUrl && activeObjectUrl !== pendingObjectUrl) URL.revokeObjectURL(activeObjectUrl);
  activeObjectUrl = pendingObjectUrl;
  pendingObjectUrl = undefined;
  displayedFrameSequence = sequence;
  image.hidden = false;
  emptyState.hidden = true;
});

image.addEventListener('error', () => {
  if (pendingObjectUrl) URL.revokeObjectURL(pendingObjectUrl);
  pendingObjectUrl = undefined;
});

function connectFrameStream(streamUrl) {
  if (!streamUrl || frameSocket?.url === streamUrl && [WebSocket.OPEN, WebSocket.CONNECTING].includes(frameSocket.readyState)) return;
  clearTimeout(reconnectTimer);
  frameSocket?.close();
  const socket = new WebSocket(streamUrl);
  socket.binaryType = 'arraybuffer';
  frameSocket = socket;
  socket.addEventListener('open', () => { if (frameSocket === socket) reportAction('Binary frame stream connected'); });
  socket.addEventListener('message', (event) => {
    if (frameSocket !== socket || !(event.data instanceof ArrayBuffer)) return;
    latestFramePacket = { buffer: event.data, socket }; // overwrite instead of retaining stale JPEGs
    scheduleFrameRender();
  });
  socket.addEventListener('close', () => {
    if (latestFramePacket?.socket === socket) latestFramePacket = undefined;
    if (frameSocket !== socket) return;
    frameSocket = undefined;
    if (currentSnapshot?.streamUrl) reconnectTimer = setTimeout(() => connectFrameStream(currentSnapshot.streamUrl), 500);
  });
  socket.addEventListener('error', () => socket.close());
}

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
    connectFrameStream(snapshot.streamUrl);
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
function dispatchPanelPointer(type, x, y, options = {}) {
  return panel.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, ...panelPoint(x, y), button: options.button ?? -1, buttons: options.buttons ?? 0 }));
}
function dispatchPanelWheel(x, y, deltaY) {
  return panel.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, ...panelPoint(x, y), deltaMode: 0, deltaY }));
}
async function waitForInputIdle() {
  while (true) {
    await inputQueue;
    if (!inputBusy && !lowPriorityRunning && dragFrame === undefined && wheelFrame === undefined) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}
async function clickPanel(x, y) { dispatchPanelPointer('pointerdown', x, y, { button: 0, buttons: 1 }); dispatchPanelPointer('pointerup', x, y, { button: 0, buttons: 0 }); await waitForInputIdle(); }
async function typePanelKey(key, code) { panel.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, key, code })); panel.dispatchEvent(new KeyboardEvent('keyup', { bubbles: true, cancelable: true, key, code })); await waitForInputIdle(); }

async function waitForRenderedFrame(timeoutMs = 8_000) {
  const deadline = performance.now() + timeoutMs;
  while (displayedFrameSequence === 0) {
    if (performance.now() >= deadline) throw new Error('timed out waiting for a rendered binary frame');
    await new Promise((resolve) => requestAnimationFrame(resolve));
  }
  if (!image.complete || image.naturalWidth !== currentSnapshot.pixelWidth || image.naturalHeight !== currentSnapshot.pixelHeight) {
    throw new Error('rendered binary frame dimensions disagreed with current geometry');
  }
}

async function runSelfTest() {
  if (selfTestStarted || !selfTestRequested || !currentSnapshot?.viewport) return;
  selfTestStarted = true;
  setStatus('Self-test running…');
  try {
    await waitForRenderedFrame();
    if (!Number.isFinite(currentSnapshot.viewport.offsetX) || !Number.isFinite(currentSnapshot.viewport.offsetY)) {
      throw new Error('rendered binary frame omitted viewport offsets');
    }
    let focused;
    for (const [x, y] of [[300, 230], [300, 255], [300, 280], [300, 305]]) {
      await clickPanel(x, y); focused = await command('browser_inspect'); if (focused.activeElement === 'INPUT#name') break;
    }
    if (focused?.activeElement !== 'INPUT#name') throw new Error(`input focus was ${focused?.activeElement || 'unknown'}`);
    for (const [key, code] of [['A', 'KeyA'], ['d', 'KeyD'], ['a', 'KeyA']]) await typePanelKey(key, code);
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
    await command('browser_self_test_report', { success: true, detail: 'live binary frame surface pointer, keyboard, wheel, selection, navigation control, and inspect handlers' });
    setStatus('Self-test passed', 'ok'); reportAction('Native self-test passed');
  } catch (error) {
    try { await command('browser_self_test_report', { success: false, detail: String(error).slice(0, 480) }); } catch {}
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
window.addEventListener('beforeunload', () => { clearTimeout(reconnectTimer); frameSocket?.close(); if (activeObjectUrl) URL.revokeObjectURL(activeObjectUrl); if (pendingObjectUrl) URL.revokeObjectURL(pendingObjectUrl); void command('browser_stop').catch(() => {}); });
start();
