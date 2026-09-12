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
const lastAction = document.querySelector('#last-action');

let currentSnapshot;
let inputQueue = Promise.resolve();
let inputBusy = false;
let moveTimer;
let pendingMove;
let pollTimer;
let selfTestStarted = false;
let selfTestRequested = false;

function setStatus(message, kind = 'idle') {
  status.textContent = message;
  status.dataset.kind = kind;
}

function reportAction(message) {
  lastAction.textContent = message;
}

function applySnapshot(snapshot) {
  if (!snapshot?.pngDataUrl) throw new Error('Snapshot did not include a PNG data URL');
  currentSnapshot = snapshot;
  image.src = snapshot.pngDataUrl;
  image.hidden = false;
  emptyState.hidden = true;
  title.textContent = snapshot.title || 'Untitled page';
  dimensions.textContent = `${snapshot.width} × ${snapshot.height}`;
  pageUrl.textContent = snapshot.url;
  if (document.activeElement !== address) address.value = snapshot.url;
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
    reportAction('Chromium fixture started');
    schedulePoll();
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

function schedulePoll() {
  clearTimeout(pollTimer);
  pollTimer = setTimeout(async () => {
    try {
      if (!inputQueuePending()) applySnapshot(await command('browser_snapshot'));
    } catch (error) {
      setStatus(String(error), 'error');
    }
    schedulePoll();
  }, 900);
}

function inputQueuePending() {
  return inputBusy;
}

function queueInput(event, description, quiet = false, strict = false) {
  const operation = inputQueue.then(async () => {
    inputBusy = true;
    const snapshot = await command('browser_input', { event });
    applySnapshot(snapshot);
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

function modifierMask(event) {
  return (event.altKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) |
    (event.metaKey ? 4 : 0) | (event.shiftKey ? 8 : 0);
}
async function runSelfTest() {
  if (selfTestStarted || !selfTestRequested || !currentSnapshot) return;
  selfTestStarted = true;
  setStatus('Self-test running…');
  try {
    const click = async (x, y) => {
      await queueInput({ kind: 'mouseDown', x, y, button: 'left', modifiers: 0 }, 'Self-test pointer down', true, true);
      await queueInput({ kind: 'mouseUp', x, y, button: 'left', modifiers: 0 }, 'Self-test pointer up', true, true);
    };
    await click(300, 255);
    const focused = await command('browser_inspect');
    if (focused.activeElement !== 'INPUT#name') throw new Error(`input focus was ${focused.activeElement}`);
    for (const [key, code] of [['A', 'KeyA'], ['d', 'KeyD'], ['a', 'KeyA']]) {
      await queueInput({ kind: 'keyDown', key, code, text: key, modifiers: 0 }, `Self-test key down ${key}`, true, true);
      await queueInput({ kind: 'keyUp', key, code, modifiers: 0 }, `Self-test key up ${key}`, true, true);
    }
    await click(820, 255);
    const result = await command('browser_inspect');
    fixtureStatus.textContent = result.fixtureStatus || '—';
    activeElement.textContent = result.activeElement;
    if (!result.fixtureStatus?.includes('Hello, Ada')) throw new Error(`fixture status was ${result.fixtureStatus || 'empty'}`);
    await command('browser_self_test_report', { success: true, detail: 'pointer focus, typing, and button activation' });
    setStatus('Self-test passed', 'ok');
    reportAction('Native self-test passed');
  } catch (error) {
    try { await command('browser_self_test_report', { success: false, detail: String(error).slice(0, 480) }); } catch {}
    setStatus(`Self-test failed: ${error}`, 'error');
    reportAction('Native self-test failed');
  }
}

function imageCoordinates(event) {
  if (!currentSnapshot) return null;
  const rect = image.getBoundingClientRect();
  if (!rect.width || !rect.height || event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) return null;
  return {
    x: Math.max(0, Math.min(currentSnapshot.width, (event.clientX - rect.left) * currentSnapshot.width / rect.width)),
    y: Math.max(0, Math.min(currentSnapshot.height, (event.clientY - rect.top) * currentSnapshot.height / rect.height)),
  };
}

function buttonName(button) {
  return button === 1 ? 'middle' : button === 2 ? 'right' : 'left';
}

let activePointer = false;
let lastPointer = null;
let activeButton = 'left';

panel.addEventListener('pointermove', (event) => {
  const coordinates = imageCoordinates(event);
  if (!coordinates) return;
  lastPointer = coordinates;
  pendingMove = { kind: 'mouseMove', ...coordinates, modifiers: modifierMask(event) };
  if (!moveTimer) {
    moveTimer = setTimeout(() => {
      moveTimer = undefined;
      const move = pendingMove;
      pendingMove = undefined;
      if (move) queueInput(move, 'Pointer moved', true);
    }, 45);
  }
});

panel.addEventListener('pointerdown', (event) => {
  const coordinates = imageCoordinates(event);
  if (!coordinates) return;
  panel.focus({ preventScroll: true });
  lastPointer = coordinates;
  activePointer = true;
  activeButton = buttonName(event.button);
  panel.setPointerCapture?.(event.pointerId);
  queueInput({ kind: 'mouseDown', ...coordinates, button: activeButton, modifiers: modifierMask(event) }, 'Pointer pressed');
});

function finishPointer(event, description) {
  if (!activePointer || !lastPointer) return;
  const coordinates = imageCoordinates(event) || lastPointer;
  activePointer = false;
  panel.releasePointerCapture?.(event.pointerId);
  queueInput({ kind: 'mouseUp', ...coordinates, button: activeButton, modifiers: modifierMask(event) }, description);
  lastPointer = null;
}

panel.addEventListener('pointerup', (event) => finishPointer(event, 'Pointer released'));
panel.addEventListener('pointercancel', (event) => finishPointer(event, 'Pointer cancelled'));
panel.addEventListener('lostpointercapture', (event) => finishPointer(event, 'Pointer capture lost'));

panel.addEventListener('wheel', (event) => {
  const coordinates = imageCoordinates(event);
  if (!coordinates) return;
  event.preventDefault();
  queueInput({ kind: 'wheel', ...coordinates, deltaX: event.deltaX, deltaY: event.deltaY, modifiers: modifierMask(event) }, 'Wheel scrolled');
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
    reportAction('Navigated to local fixture');
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
    setStatus('Ready', 'ok');
    reportAction('Inspected fixed page state');
  } catch (error) {
    setStatus(String(error), 'error');
    reportAction('Inspect failed');
  }
});

if (eventApi?.listen) {
  void eventApi.listen('interactive-browser-panel-self-test', () => {
    selfTestRequested = true;
    void runSelfTest();
  });
}

window.addEventListener('beforeunload', () => {
  // Tauri also stops the helper from its app-exit hook; this is only a best-effort UI signal.
  void command('browser_stop').catch(() => {});
});

start();
