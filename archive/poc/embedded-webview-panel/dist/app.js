const invoke = window.__TAURI__?.core?.invoke;
const eventApi = window.__TAURI__?.event;
const frame = document.querySelector('#browser-frame');
const emptyState = document.querySelector('#empty-state');
const status = document.querySelector('#status');
const address = document.querySelector('#url');
const title = document.querySelector('#page-title');
const dimensions = document.querySelector('#dimensions');
const pageUrl = document.querySelector('#page-url');
const activeElement = document.querySelector('#active-element');
const fixtureStatus = document.querySelector('#fixture-status');
const greetingResult = document.querySelector('#greeting-result');
const selectedText = document.querySelector('#selected-text');
const lastAction = document.querySelector('#last-action');

const fixtureUrl = new URL('fixture/index.html', document.baseURI).href;
let selfTestRequested = false;
let selfTestStarted = false;

function setStatus(message, kind = 'idle') {
  status.textContent = message;
  status.dataset.kind = kind;
}

function reportAction(message) {
  lastAction.textContent = message;
}

function command(name, args = {}) {
  if (!invoke) return Promise.reject(new Error('Tauri command API is unavailable'));
  return invoke(name, args);
}

function normalizeHttpUrl(value) {
  if (typeof value !== 'string') throw new Error('URL must be text');
  let candidate = value.trim();
  if (!candidate || candidate.length > 2048) throw new Error('URL must be between 1 and 2048 characters');
  if (!/^[a-z][a-z\d+.-]*:\/\//i.test(candidate)) candidate = `https://${candidate}`;
  let parsed;
  try {
    parsed = new URL(candidate);
  } catch {
    throw new Error('URL must be an absolute HTTP(S) URL or a bare domain');
  }
  if (!['http:', 'https:'].includes(parsed.protocol)) {
    throw new Error('Navigation only supports http:// and https:// URLs');
  }
  if (!parsed.hostname || parsed.username || parsed.password) {
    throw new Error('Navigation URL must not contain credentials');
  }
  return parsed.href;
}

function currentFrameUrl() {
  return frame.src || fixtureUrl;
}

function updateFrameMetadata() {
  const url = currentFrameUrl();
  pageUrl.textContent = url;
  address.value = url;
  dimensions.textContent = `${frame.clientWidth} × ${frame.clientHeight}`;
  try {
    title.textContent = frame.contentDocument?.title || 'Untitled page';
  } catch {
    title.textContent = 'External page';
  }
}

function describeActiveElement(element) {
  if (!element || element === element.ownerDocument?.body) return 'BODY';
  const id = element.id ? `#${element.id}` : '';
  return `${element.tagName}${id}`;
}

function inspectFrame() {
  try {
    const doc = frame.contentDocument;
    const win = frame.contentWindow;
    if (!doc || !win) throw new Error('frame document is unavailable');
    const result = doc.querySelector('#result');
    const source = doc.querySelector('#selection-source');
    if (!result || !source) throw new Error('page is not the local inspection fixture');
    const selection = win.getSelection()?.toString().trim() || '';
    return {
      title: doc.title,
      url: win.location.href,
      activeElement: describeActiveElement(doc.activeElement),
      fixtureStatus: result.dataset.changed === 'true' ? 'greeting changed' : 'waiting for greeting',
      greeting: result.textContent.trim(),
      scrollY: Math.round(win.scrollY),
      selectedText: selection,
    };
  } catch (error) {
    throw new Error(`Inspection unavailable for this page (cross-origin or inaccessible): ${error.message}`);
  }
}

function showInspection(result) {
  pageUrl.textContent = result.url;
  title.textContent = result.title || 'Untitled page';
  activeElement.textContent = result.activeElement;
  fixtureStatus.textContent = result.fixtureStatus;
  greetingResult.textContent = result.greeting;
  scrollY.textContent = String(result.scrollY);
  selectedText.textContent = result.selectedText || '—';
}

function loadedFixtureDocument() {
  try {
    const doc = frame.contentDocument;
    if (!doc || doc.readyState !== 'complete') return null;
    if (!doc.querySelector('#name') || !doc.querySelector('#greet') ||
        !doc.querySelector('#result') || !doc.querySelector('#selection-source')) {
      return null;
    }
    return doc;
  } catch {
    return null;
  }
}

async function runSelfTest() {
  if (selfTestStarted || !loadedFixtureDocument()) return;
  selfTestStarted = true;
  let success = false;
  let detail = '';
  try {
    const doc = frame.contentDocument;
    const win = frame.contentWindow;
    const name = doc?.querySelector('#name');
    const greet = doc?.querySelector('#greet');
    const result = doc?.querySelector('#result');
    const source = doc?.querySelector('#selection-source');
    if (!doc || !win || !name || !greet || !result || !source) {
      throw new Error('fixture DOM did not load');
    }

    name.focus();
    name.value = 'Ada';
    name.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: 'Ada' }));
    greet.click();
    win.scrollTo(0, 320);
    const range = doc.createRange();
    range.selectNodeContents(source);
    const selection = win.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);

    const active = doc.activeElement === name;
    const greeted = result.textContent.includes('Hello, Ada!') && result.dataset.changed === 'true';
    const scrolled = win.scrollY > 0;
    const selected = selection.toString().includes('real text selection');
    success = active && greeted && scrolled && selected;
    detail = `activeElement=${describeActiveElement(doc.activeElement)}, greeting=${greeted}, scrollY=${Math.round(win.scrollY)}, selectedText=${selected}`;
  } catch (error) {
    detail = error instanceof Error ? error.message : String(error);
  }

  try {
    await command('browser_self_test_report', { success, detail });
    setStatus(success ? 'Self-test passed' : 'Self-test failed', success ? 'ok' : 'error');
    reportAction(success ? 'Native iframe self-test passed' : `Native iframe self-test failed: ${detail}`);
  } catch (error) {
    setStatus('Self-test report failed', 'error');
    reportAction(error.message);
  }
}

function maybeRunSelfTest() {
  if (selfTestRequested && !selfTestStarted && loadedFixtureDocument()) {
    void runSelfTest();
  }
}

frame.addEventListener('load', () => {
  frame.classList.add('loaded');
  emptyState.hidden = true;
  updateFrameMetadata();
  setStatus('Page ready', 'ok');
  reportAction('Page loaded in native WebView');
  maybeRunSelfTest();
});

window.addEventListener('resize', updateFrameMetadata);

document.querySelector('#navigation').addEventListener('submit', (event) => {
  event.preventDefault();
  try {
    const url = normalizeHttpUrl(address.value);
    frame.src = url;
    address.value = url;
    pageUrl.textContent = url;
    title.textContent = 'Loading page…';
    setStatus('Navigating…');
    reportAction(`Navigating to ${url}`);
  } catch (error) {
    setStatus(error.message, 'error');
    reportAction('Navigation rejected');
  }
});

document.querySelector('#reload').addEventListener('click', () => {
  try {
    frame.contentWindow.location.reload();
  } catch {
    frame.src = currentFrameUrl();
  }
  setStatus('Refreshing…');
  reportAction('Refreshing embedded page');
});

document.querySelector('#inspect').addEventListener('click', () => {
  try {
    showInspection(inspectFrame());
    setStatus('Inspection complete', 'ok');
    reportAction('Read active element, greeting, scroll, and selection');
  } catch (error) {
    reportAction(error.message);
    activeElement.textContent = 'Unavailable';
    fixtureStatus.textContent = 'Unavailable';
    greetingResult.textContent = 'Unavailable';
    scrollY.textContent = '—';
    selectedText.textContent = 'Unavailable';
  }
});

if (eventApi?.listen) {
  void eventApi.listen('embedded-webview-panel-self-test', () => {
    selfTestRequested = true;
    maybeRunSelfTest();
  });
}

async function initialize() {
  address.value = fixtureUrl;
  pageUrl.textContent = fixtureUrl;
  try {
    selfTestRequested = await command('browser_self_test_enabled');
  } catch {
    selfTestRequested = false;
  }
  maybeRunSelfTest();
}

void initialize();
