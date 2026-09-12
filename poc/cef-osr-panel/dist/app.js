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

let currentSnapshot;
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

const SAFE_CURSORS = new Set(['default', 'auto', 'pointer', 'text', 'crosshair', 'move', 'not-allowed', 'wait', 'grab', 'grabbing', 'cell', 'help', 'progress', 'zoom-in', 'zoom-out', 'col-resize', 'row-resize', 'e-resize', 'w-resize', 'n-resize', 's-resize']);

function setStatus(message, kind = 'idle') {
  status.textContent = message;
  status.dataset.kind = kind;
}

function reportAction(message) {
  lastAction.textContent = message;
}
function applyCursor(cursor) {
  panel.style.cursor = SAFE_CURSORS.has(cursor) ? cursor : 'default';
}

function applySnapshot(snapshot) {
  if (!snapshot?.jpegDataUrl) throw new Error('Snapshot did not include a JPEG data URL');
  currentSnapshot = snapshot;
  image.src = snapshot.jpegDataUrl;
  image.hidden = false;
  emptyState.hidden = true;
  title.textContent = snapshot.title || 'Untitled page';
  dimensions.textContent = `${snapshot.width} × ${snapshot.height}`;
  pageUrl.textContent = snapshot.url;
  applyCursor(snapshot.cursor);
  if (document.activeElement !== address) address.value = snapshot.url;
}

function applyScreencastFrame(frame) {
  if (!frame?.jpegDataUrl) return;
  image.src = frame.jpegDataUrl;
  image.hidden = false;
  emptyState.hidden = true;
  dimensions.textContent = `${frame.width} × ${frame.height}`;
  if (currentSnapshot) currentSnapshot = { ...currentSnapshot, width: frame.width, height: frame.height };
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
    applySnapshot(await command('browser_start'));
    setStatus('Ready', 'ok');
    reportAction('CEF OnPaint frame stream started');
    if (await command('browser_self_test_enabled')) {
      selfTestRequested = true;
      void runSelfTest();
    }
  } catch (error) {
    setStatus(String(error), 'error');
    emptyState.textContent = `Unable to start browser: ${error}`;
    reportAction('Startup failed');
  }
}


function inputQueuePending() {
  return inputBusy;
}

function queueInput(event, description, quiet = false, strict = false, frameDelay = 90) {
  const operation = inputQueue.then(async () => {
    inputBusy = true;
    const ack = await command('browser_input', { event });
    applyInputAck(ack);
    if (!quiet) {
      setStatus('Ready', 'ok');
      reportAction(description);
    }
  });
  inputQueue = operation.catch((error) => {
    setStatus(String(error), 'error');
    reportAction(`${description} failed`);
  }).finally(() => {
    inputBusy = false;
  });
  return strict ? operation : inputQueue;
}
function queueLatestInput(event, description) {
  const slot = event.kind === 'wheel' ? 'wheel' : 'pointerMove';
  pendingLowPriority[slot] = { event, description, sequence: ++lowPrioritySequence };
  if (lowPriorityRunning) return;
  lowPriorityRunning = true;
  void (async () => {
    while (pendingLowPriority.pointerMove || pendingLowPriority.wheel) {
      const slotToRun = !pendingLowPriority.wheel ||
        (pendingLowPriority.pointerMove && pendingLowPriority.pointerMove.sequence < pendingLowPriority.wheel.sequence)
        ? 'pointerMove'
        : 'wheel';
      const next = pendingLowPriority[slotToRun];
      pendingLowPriority[slotToRun] = undefined;
      await queueInput(next.event, next.description, true, false, 40);
    }
    lowPriorityRunning = false;
  })();
}

function modifierMask(event) {
  return (event.altKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) |
    (event.metaKey ? 4 : 0) | (event.shiftKey ? 8 : 0);
}
function panelPoint(x, y) {
  const rect = image.getBoundingClientRect();
  return {
    clientX: rect.left + x * rect.width / currentSnapshot.width,
    clientY: rect.top + y * rect.height / currentSnapshot.height,
  };
}

function dispatchPanelPointer(type, x, y, options = {}) {
  const point = panelPoint(x, y);
  return panel.dispatchEvent(new PointerEvent(type, {
    bubbles: true,
    cancelable: true,
    pointerId: 1,
    ...point,
    button: options.button ?? -1,
    buttons: options.buttons ?? 0,
  }));
}

function dispatchPanelWheel(x, y, deltaY) {
  const point = panelPoint(x, y);
  return panel.dispatchEvent(new WheelEvent('wheel', {
    bubbles: true,
    cancelable: true,
    ...point,
    deltaMode: 0,
    deltaY,
  }));
}

async function waitForInputIdle() {
  while (true) {
    await inputQueue;
    if (!inputBusy && !lowPriorityRunning && dragFrame === undefined && wheelFrame === undefined) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

async function clickPanel(x, y) {
  dispatchPanelPointer('pointerdown', x, y, { button: 0, buttons: 1 });
  dispatchPanelPointer('pointerup', x, y, { button: 0, buttons: 0 });
  await waitForInputIdle();
}

async function typePanelKey(key, code) {
  panel.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, key, code }));
  panel.dispatchEvent(new KeyboardEvent('keyup', { bubbles: true, cancelable: true, key, code }));
  await waitForInputIdle();
}

async function runSelfTest() {
  if (selfTestStarted || !selfTestRequested || !currentSnapshot) return;
  selfTestStarted = true;
  setStatus('Self-test running…');
  try {
    let focused;
    for (const [x, y] of [[300, 230], [300, 255], [300, 280], [300, 305]]) {
      await clickPanel(x, y);
      focused = await command('browser_inspect');
      if (focused.activeElement === 'INPUT#name') break;
    }
    if (focused?.activeElement !== 'INPUT#name') throw new Error(`input focus was ${focused?.activeElement || 'unknown'}`);
    for (const [key, code] of [['A', 'KeyA'], ['d', 'KeyD'], ['a', 'KeyA']]) {
      await typePanelKey(key, code);
    }
    let result;
    for (const [x, y] of [[820, 230], [820, 255], [820, 280], [820, 305], [800, 255], [840, 255]]) {
      await clickPanel(x, y);
      result = await command('browser_inspect');
      if (result.fixtureStatus?.includes('Hello, Ada')) break;
    }
    if (!result?.fixtureStatus?.includes('Hello, Ada')) throw new Error(`fixture status was ${result?.fixtureStatus || 'empty'}`);

    const beforeScroll = await command('browser_inspect');
    dispatchPanelPointer('pointermove', 500, 600, { buttons: 0 });
    dispatchPanelWheel(500, 600, 500);
    await waitForInputIdle();
    const afterScroll = await command('browser_inspect');
    if (afterScroll.scrollY <= beforeScroll.scrollY) throw new Error(`scrollY stayed at ${beforeScroll.scrollY}`);

    dispatchPanelPointer('pointermove', 210, 120, { buttons: 0 });
    dispatchPanelPointer('pointerdown', 210, 120, { button: 0, buttons: 1 });
    dispatchPanelPointer('pointermove', 620, 120, { button: 0, buttons: 1 });
    dispatchPanelPointer('pointerup', 620, 120, { button: 0, buttons: 0 });
    await waitForInputIdle();
    applySnapshot(await command('browser_snapshot'));
    result = await command('browser_inspect');
    if (!result.selectedText?.length) throw new Error('drag input did not select fixture text');

    fixtureStatus.textContent = result.fixtureStatus || '—';
    activeElement.textContent = result.activeElement || '—';
    scrollY.textContent = String(result.scrollY ?? 0);
    selectedText.textContent = result.selectedText || '—';
    await command('browser_self_test_report', { success: true, detail: 'panel pointer, keyboard, wheel, and selection handlers' });
    setStatus('Self-test passed', 'ok');
    reportAction('Native self-test passed');
  } catch (error) {
    try { await command('browser_self_test_report', { success: false, detail: String(error).slice(0, 480) }); } catch {}
    setStatus(`Self-test failed: ${error}`, 'error');
    reportAction('Native self-test failed');
  }
}


function clampedImageCoordinates(event) {
  if (!currentSnapshot) return null;
  const rect = image.getBoundingClientRect();
  if (!rect.width || !rect.height) return null;
  return {
    x: Math.max(0, Math.min(currentSnapshot.width, (event.clientX - rect.left) * currentSnapshot.width / rect.width)),
    y: Math.max(0, Math.min(currentSnapshot.height, (event.clientY - rect.top) * currentSnapshot.height / rect.height)),
  };
}

function imageCoordinates(event) {
  const coordinates = clampedImageCoordinates(event);
  if (!coordinates || event.clientX < image.getBoundingClientRect().left || event.clientX > image.getBoundingClientRect().right || event.clientY < image.getBoundingClientRect().top || event.clientY > image.getBoundingClientRect().bottom) return null;
  return coordinates;
}

function buttonName(button) {
  return button === 1 ? 'middle' : button === 2 ? 'right' : 'left';
}
function buttonMask(button) {
  return button === 'left' ? 1 : button === 'right' ? 2 : 4;
}
let activePointer = false;
let lastPointer = null;
let activeButton = 'left';

function scheduleAnimation(callback) {
  return typeof requestAnimationFrame === 'function' && typeof cancelAnimationFrame === 'function'
    ? requestAnimationFrame(callback)
    : setTimeout(callback, 16);
}
function cancelAnimation(id) {
  if (typeof requestAnimationFrame === 'function' && typeof cancelAnimationFrame === 'function') cancelAnimationFrame(id);
  else clearTimeout(id);
}


function flushDragMove() {
  dragFrame = undefined;
  const move = pendingDragMove;
  pendingDragMove = undefined;
  if (move) queueLatestInput(move, 'Pointer moved');
}

function flushWheel() {
  wheelFrame = undefined;
  // A wheel's target must follow the most recent pointer move in Chromium.
  if (pendingDragMove) {
    if (dragFrame !== undefined) {
      cancelAnimation(dragFrame);
      dragFrame = undefined;
    }
    flushDragMove();
  }
  const wheel = pendingWheel;
  pendingWheel = undefined;
  if (wheel) queueLatestInput(wheel, 'Wheel scrolled');
}

panel.addEventListener('pointermove', (event) => {
  const coordinates = activePointer ? clampedImageCoordinates(event) : imageCoordinates(event);
  if (!coordinates) return;
  lastPointer = coordinates;
  pendingDragMove = { kind: 'mouseMove', ...coordinates, buttons: event.buttons, modifiers: modifierMask(event) };
  if (dragFrame === undefined) dragFrame = scheduleAnimation(flushDragMove);
});

panel.addEventListener('pointerdown', (event) => {
  const coordinates = imageCoordinates(event);
  if (!coordinates) return;
  panel.focus({ preventScroll: true });
  lastPointer = coordinates;
  activePointer = true;
  activeButton = buttonName(event.button);
  try { panel.setPointerCapture?.(event.pointerId); } catch {}
  queueInput({ kind: 'mouseDown', ...coordinates, button: activeButton, buttons: event.buttons, modifiers: modifierMask(event) }, 'Pointer pressed');
});

function finishPointer(event, description) {
  if (!activePointer || !lastPointer) return;
  if (dragFrame !== undefined) {
    cancelAnimation(dragFrame);
    dragFrame = undefined;
  }
  pendingDragMove = undefined;
  pendingLowPriority.pointerMove = undefined;
  const coordinates = clampedImageCoordinates(event) || lastPointer;
  activePointer = false;
  try { panel.releasePointerCapture?.(event.pointerId); } catch {}
  // Always send the final endpoint before release, including an outside-image drag.
  queueInput({ kind: 'mouseMove', ...coordinates, buttons: event.buttons || buttonMask(activeButton), modifiers: modifierMask(event) }, 'Pointer moved', true);
  queueInput({ kind: 'mouseUp', ...coordinates, button: activeButton, buttons: 0, modifiers: modifierMask(event) }, description);
  lastPointer = null;
}

panel.addEventListener('pointerup', (event) => finishPointer(event, 'Pointer released'));
panel.addEventListener('pointercancel', (event) => finishPointer(event, 'Pointer cancelled'));
panel.addEventListener('lostpointercapture', (event) => finishPointer(event, 'Pointer capture lost'));

panel.addEventListener('wheel', (event) => {
  const coordinates = imageCoordinates(event);
  if (!coordinates) return;
  event.preventDefault();
  const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? (currentSnapshot?.height || 720) : 1;
  const deltaX = event.deltaX * unit;
  const deltaY = event.deltaY * unit;
  pendingWheel = {
    kind: 'wheel', ...coordinates,
    deltaX: Math.max(-10000, Math.min(10000, (pendingWheel?.deltaX || 0) + deltaX)),
    deltaY: Math.max(-10000, Math.min(10000, (pendingWheel?.deltaY || 0) + deltaY)),
    modifiers: modifierMask(event),
  };
  if (wheelFrame === undefined) wheelFrame = scheduleAnimation(flushWheel);
}, { passive: false });

panel.addEventListener('keydown', (event) => {
  if (event.isComposing) return;
  event.preventDefault();
  queueInput({
    kind: 'keyDown', key: event.key, code: event.code,
    text: event.key.length === 1 ? event.key : undefined,
    modifiers: modifierMask(event),
  }, `Key down: ${event.key}`);
});

panel.addEventListener('keyup', (event) => {
  if (event.isComposing) return;
  event.preventDefault();
  queueInput({ kind: 'keyUp', key: event.key, code: event.code, modifiers: modifierMask(event) }, `Key up: ${event.key}`);
});

// The ordinary keydown/keyup path above handles ASCII input. For an IME, send the
// completed composition as one bounded text key event instead of evaluating script.
panel.addEventListener('compositionend', (event) => {
  if (!event.data) return;
  queueInput({ kind: 'keyDown', key: event.data.slice(0, 128), code: 'Unidentified', text: event.data.slice(0, 16) }, 'Text composition committed');
  queueInput({ kind: 'keyUp', key: event.data.slice(0, 128), code: 'Unidentified' }, 'Text composition released');
});

document.querySelector('#navigation').addEventListener('submit', async (event) => {
  event.preventDefault();
  setStatus('Navigating…');
  try {
    applySnapshot(await command('browser_navigate', { url: address.value.trim() }));
    setStatus('Ready', 'ok');
    reportAction('Navigated to URL');
    panel.focus({ preventScroll: true });
  } catch (error) {
    setStatus(String(error), 'error');
    reportAction('Navigation rejected');
  }
});

document.querySelector('#reload').addEventListener('click', async () => {
  setStatus('Refreshing…');
  try {
    applySnapshot(await command('browser_reload'));
    setStatus('Ready', 'ok');
    reportAction('Fixture refreshed');
  } catch (error) {
    setStatus(String(error), 'error');
    reportAction('Refresh failed');
  }
});

document.querySelector('#inspect').addEventListener('click', async () => {
  setStatus('Inspecting…');
  try {
    const result = await command('browser_inspect');
    title.textContent = result.title || 'Untitled page';
    pageUrl.textContent = result.url;
    activeElement.textContent = result.activeElement;
    fixtureStatus.textContent = result.fixtureStatus || '—';
    scrollY.textContent = String(result.scrollY ?? 0);
    selectedText.textContent = result.selectedText || '—';
    setStatus('Ready', 'ok');
    reportAction('Inspected fixed page state');
  } catch (error) {
    setStatus(String(error), 'error');
    reportAction('Inspect failed');
  }
});

if (eventApi?.listen) {
  void eventApi.listen('cef-osr-panel-frame', (event) => {
    applyScreencastFrame(event?.payload);
  });
  void eventApi.listen('cef-osr-panel-self-test', () => {
    selfTestRequested = true;
    void runSelfTest();
  });
}

window.addEventListener('beforeunload', () => {
  // Tauri also stops the helper from its app-exit hook; this is only a best-effort UI signal.
  void command('browser_stop').catch(() => {});
});

start();
