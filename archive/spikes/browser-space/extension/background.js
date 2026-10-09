importScripts('config.js');

const config = globalThis.COCKPIT_SPIKE;
if (!config || !config.endpoint || !config.token) throw new Error('Cockpit Space runtime config is missing');
const endpoint = String(config.endpoint).replace(/\/$/, '');
const extensionToken = String(config.token);
const API = Object.freeze({ state: '/api/state', control: '/api/control', annotations: '/api/annotations' });
const lastEligibleByWindow = new Map();
const pendingTargets = new Map();
const pendingElementKey = (tabId) => `cockpit.space.pendingElement.${tabId}`;
const pendingCaptureKey = 'cockpit.space.pendingCapture';
const draftKey = (captureId) => `cockpit.space.draft.${captureId}`;
let controllerOrigin = null;
try { controllerOrigin = new URL(endpoint).origin; } catch { controllerOrigin = null; }

function isWebUrl(url) { return typeof url === 'string' && /^https?:\/\//i.test(url); }
function isEligibleUrl(url) {
  if (!isWebUrl(url)) return false;
  try { return !controllerOrigin || new URL(url).origin !== controllerOrigin; } catch { return false; }
}
function publicError(error) {
  const message = error instanceof Error ? error.message : String(error || 'Unknown error');
  return message.replace(extensionToken, '[redacted]');
}
async function apiRequest(route, body) {
  const response = await fetch(`${endpoint}${route}`, {
    method: body === undefined ? 'GET' : 'POST',
    headers: { Authorization: `Bearer ${extensionToken}`, ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  let value = {};
  try { value = text ? JSON.parse(text) : {}; } catch { /* server error remains generic */ }
  if (!response.ok) throw new Error(value?.error || `Space server returned HTTP ${response.status}`);
  return value;
}
async function eligibleTab(tabId) {
  try {
    const tab = await chrome.tabs.get(tabId);
    return isEligibleUrl(tab.url) ? tab : null;
  } catch { return null; }
}
async function pageTargets() {
  const targets = await chrome.debugger.getTargets();
  return targets.filter((target) => target.type === 'page' && Number.isInteger(target.tabId));
}
async function targetForTab(tabId) {
  return (await pageTargets()).find((target) => target.tabId === tabId) || null;
}
async function chooseTab(requestedTabId) {
  if (requestedTabId !== undefined && requestedTabId !== null) {
    const tab = await eligibleTab(Number(requestedTabId));
    if (!tab) throw new Error('Choose an HTTP or HTTPS page first.');
    const target = await targetForTab(tab.id);
    if (!target) throw new Error('The selected page is not available to the Space.');
    lastEligibleByWindow.set(tab.windowId, tab.id);
    return { tab, targetId: target.id };
  }
  let candidates = (await chrome.tabs.query({ active: true, currentWindow: true })).filter((tab) => isEligibleUrl(tab.url));
  if (!candidates.length) {
    candidates = [];
    for (const tabId of lastEligibleByWindow.values()) {
      const tab = await eligibleTab(tabId);
      if (tab) candidates.push(tab);
    }
  }
  if (!candidates.length) candidates = (await chrome.tabs.query({})).filter((tab) => isEligibleUrl(tab.url));
  candidates.sort((a, b) => a.id - b.id);
  const tab = candidates[candidates.length - 1];
  if (!tab) throw new Error('Open an HTTP or HTTPS page before capturing feedback.');
  const target = await targetForTab(tab.id);
  if (!target) throw new Error('The selected page is not available to the Space.');
  lastEligibleByWindow.set(tab.windowId, tab.id);
  return { tab, targetId: target.id };
}
async function ensureContentScript(tabId) {
  try { await chrome.tabs.sendMessage(tabId, { type: 'COCKPIT_PING' }); }
  catch {
    await chrome.scripting.executeScript({ target: { tabId }, files: ['content.js'] });
  }
}
async function sendToPage(tabId, message) {
  await ensureContentScript(tabId);
  return chrome.tabs.sendMessage(tabId, message);
}
async function pageDocumentId(tabId) {
  const results = await chrome.scripting.executeScript({
    target: { tabId, frameIds: [0] },
    func: () => document.visibilityState,
  });
  const documentId = results[0]?.documentId;
  if (typeof documentId !== 'string' || !documentId) throw new Error('The browser did not provide a page identity.');
  return documentId;
}
async function setHumanControl() { await apiRequest(API.control, { control: 'human' }); }
async function pendingForTab(tabId) {
  const memory = pendingTargets.get(tabId);
  if (memory) return memory;
  const key = pendingElementKey(tabId);
  const stored = await chrome.storage.session.get(key);
  return stored[key] || null;
}
async function currentTargetForPending(pending) {
  const tab = await eligibleTab(pending.browserTabId);
  if (!tab) throw new Error('The page is no longer available.');
  if (tab.url !== pending.url) throw new Error('The page changed; start feedback again on the current page.');
  const target = await targetForTab(tab.id);
  if (!target || target.id !== pending.targetId) throw new Error('The page target changed; start feedback again.');
  const documentId = await pageDocumentId(tab.id);
  if (documentId !== pending.documentId) throw new Error('The page document changed; start feedback again.');
  return { tab, targetId: target.id };
}
function viewportFrom(value) {
  const input = value && typeof value === 'object' ? value : {};
  const number = (key, fallback = 0) => { const n = Number(input[key]); return Number.isFinite(n) ? n : fallback; };
  return { width: Math.max(0, Math.round(number('width'))), height: Math.max(0, Math.round(number('height'))), scrollX: number('scrollX'), scrollY: number('scrollY'), devicePixelRatio: Math.max(0.1, number('devicePixelRatio', 1)) };
}
function validElement(value) {
  const element = value?.element;
  const rect = element?.rect;
  if (!element || typeof element !== 'object' || !rect || typeof rect !== 'object') return null;
  return {
    tag: String(element.tag ?? '').slice(0, 100), text: String(element.text ?? '').slice(0, 1000), selector: String(element.selector ?? '').slice(0, 1000),
    rect: { x: Number(rect.x) || 0, y: Number(rect.y) || 0, width: Math.max(0, Number(rect.width) || 0), height: Math.max(0, Number(rect.height) || 0) },
  };
}
async function startPicker(browserTabId, initialComment) {
  const { tab, targetId } = await chooseTab(browserTabId);
  await setHumanControl();
  await chrome.windows.update(tab.windowId, { focused: true });
  await chrome.tabs.update(tab.id, { active: true });
  try {
    await ensureContentScript(tab.id);
    const documentId = await pageDocumentId(tab.id);
    const pending = { browserTabId: tab.id, targetId, documentId, url: tab.url };
    pendingTargets.set(tab.id, pending);
    await chrome.storage.session.set({ [pendingElementKey(tab.id)]: pending });
    await sendToPage(tab.id, { type: 'COCKPIT_START_PICKER', initialComment: String(initialComment || '').slice(0, 4000) });
  } catch (error) {
    pendingTargets.delete(tab.id);
    await chrome.storage.session.remove(pendingElementKey(tab.id));
    throw new Error(`Could not open the page picker: ${publicError(error)}`);
  }
  return { tabId: tab.id };
}
async function viewportForTab(tabId) {
  const result = await sendToPage(tabId, { type: 'COCKPIT_GET_VIEWPORT' });
  return viewportFrom(result?.viewport);
}
async function capture(browserTabId) {
  const { tab, targetId } = await chooseTab(browserTabId);
  await setHumanControl();
  await chrome.windows.update(tab.windowId, { focused: true });
  await chrome.tabs.update(tab.id, { active: true });
  const viewport = await viewportForTab(tab.id);
  const documentId = await pageDocumentId(tab.id);
  const image = await chrome.tabs.captureVisibleTab(tab.windowId, { format: 'png' });
  if (image.length > 4_000_000) throw new Error('Screenshot exceeds this spike’s 4 MB capture limit. Reduce the browser window size and capture again.');
  const captureId = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  const pending = { captureId, browserTabId: tab.id, targetId, documentId, url: tab.url, title: tab.title || '', viewport, image };
  await chrome.storage.session.set({ [pendingCaptureKey]: pending });
  await chrome.tabs.create({ url: `${chrome.runtime.getURL('editor.html')}?capture=${encodeURIComponent(captureId)}` });
  return { captureId };
}
async function saveElement(sender, value) {
  const browserTabId = sender.tab?.id;
  const pending = await pendingForTab(browserTabId);
  if (!pending || pending.browserTabId !== browserTabId) throw new Error('This picker is no longer active.');
  if (sender.documentId !== pending.documentId) throw new Error('The page document changed; start feedback again.');
  const { tab, targetId } = await currentTargetForPending(pending);
  const element = validElement(value);
  if (!element) throw new Error('The selected element metadata is invalid.');
  const result = await apiRequest(API.annotations, { tabId: targetId, url: tab.url, title: tab.title || '', comment: String(value.comment ?? '').slice(0, 4000), kind: 'element', element, viewport: viewportFrom(value.viewport) });
  pendingTargets.delete(browserTabId);
  await chrome.storage.session.remove(pendingElementKey(browserTabId));
  return result;
}
async function saveDrawing(value) {
  const stored = await chrome.storage.session.get(pendingCaptureKey);
  const pending = stored[pendingCaptureKey];
  if (!pending || pending.captureId !== value.captureId) throw new Error('This screenshot draft is no longer available.');
  const { tab, targetId } = await currentTargetForPending(pending);
  if (typeof pending.image !== 'string' || !/^data:image\/(png|jpeg);base64,/.test(pending.image)) throw new Error('The screenshot image is invalid.');
  const strokes = Array.isArray(value.strokes) ? value.strokes.slice(0, 500) : [];
  const result = await apiRequest(API.annotations, { tabId: targetId, url: tab.url, title: tab.title || pending.title || '', comment: String(value.comment ?? '').slice(0, 4000), kind: 'drawing', viewport: viewportFrom(pending.viewport), image: pending.image, strokes });
  await chrome.storage.session.remove([pendingCaptureKey, draftKey(pending.captureId)]);
  return result;
}
async function context() {
  const state = await apiRequest(API.state);
  const browserTabs = [];
  for (const tab of (await chrome.tabs.query({})).filter((candidate) => isEligibleUrl(candidate.url))) {
    const target = await targetForTab(tab.id);
    if (target) browserTabs.push({ browserTabId: tab.id, targetId: target.id, url: tab.url, title: tab.title || '' });
  }
  const active = (await chrome.tabs.query({ active: true, currentWindow: true })).find((tab) => isEligibleUrl(tab.url));
  const remembered = active ? lastEligibleByWindow.get(active.windowId) : undefined;
  const activeBrowserTabId = active?.id ?? browserTabs.find((tab) => tab.browserTabId === remembered)?.browserTabId ?? browserTabs.at(-1)?.browserTabId ?? null;
  if (active) lastEligibleByWindow.set(active.windowId, active.id);
  return { state, browserTabs, activeBrowserTabId };
}
function isExtensionSender(sender) { return sender?.id === chrome.runtime.id && typeof sender.url === 'string' && sender.url.startsWith(chrome.runtime.getURL('')); }
function isContentSender(sender) { return sender?.id === chrome.runtime.id && sender.frameId === 0 && Number.isInteger(sender.tab?.id) && isEligibleUrl(sender.tab.url); }
async function handleMessage(message, sender) {
  if (!message || typeof message.type !== 'string') throw new Error('Unknown extension message.');
  if (message.type === 'COCKPIT_GET_CONTEXT') {
    if (!isExtensionSender(sender)) throw new Error('Only the extension UI may request Space state.');
    return context();
  }
  if (message.type === 'COCKPIT_START_PICKER') {
    if (!isExtensionSender(sender)) throw new Error('Only the extension UI may start a picker.');
    return startPicker(message.browserTabId, message.comment);
  }
  if (message.type === 'COCKPIT_CAPTURE') {
    if (!isExtensionSender(sender)) throw new Error('Only the extension UI may capture a page.');
    return capture(message.browserTabId);
  }
  if (message.type === 'COCKPIT_ELEMENT_ANNOTATION') {
    if (!isContentSender(sender)) throw new Error('Only a page picker may save an element.');
    const pending = await pendingForTab(sender.tab.id);
    if (!pending || pending.browserTabId !== sender.tab.id) throw new Error('This picker is no longer active.');
    return saveElement(sender, message);
  }
  if (message.type === 'COCKPIT_SAVE_DRAWING') {
    if (!isExtensionSender(sender)) throw new Error('Only the screenshot editor may save a drawing.');
    return saveDrawing(message);
  }
  if (message.type === 'COCKPIT_DRAFT') {
    if (!isExtensionSender(sender)) throw new Error('Only the screenshot editor may access drafts.');
    const key = draftKey(String(message.captureId || ''));
    if (message.draft === undefined) {
      const draft = await chrome.storage.session.get(key);
      return { draft: draft[key] || null };
    }
    await chrome.storage.session.set({ [key]: message.draft });
    return { ok: true };
  }
  throw new Error('Unknown extension message.');
}
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  handleMessage(message, sender).then((result) => sendResponse({ ok: true, ...result })).catch((error) => sendResponse({ ok: false, error: publicError(error) }));
  return true;
});
chrome.tabs.onActivated.addListener(async ({ tabId, windowId }) => {
  const tab = await eligibleTab(tabId);
  if (tab) lastEligibleByWindow.set(windowId, tabId);
});
chrome.tabs.onUpdated.addListener(async (tabId, changeInfo, tab) => {
  if (changeInfo.status === 'complete' && isEligibleUrl(tab.url)) lastEligibleByWindow.set(tab.windowId, tabId);
});
