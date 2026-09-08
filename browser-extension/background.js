const EXPECTED_EXTENSION_ID = 'fblkilbfbmpndnfjaacljmcljhepakok';
const MAX_PNG_BYTES = 4 * 1024 * 1024;
const MAX_REQUEST_BYTES = 6 * 1024 * 1024;
const MAX_DRAFTS = 8;
const DRAFTS_KEY = 'cockpit.feedback.drafts';
const PENDING_KEY = 'cockpit.feedback.pending';
const PAIRING_KEY = 'cockpit.feedback.pairing';
let pairing = null;
const documentByTab = new Map();

function publicError(error) { return error instanceof Error ? error.message.slice(0, 400) : 'Request failed'; }
function isPageUrl(url) { return typeof url === 'string' && /^https?:\/\//i.test(url); }
async function readStorage(key, fallback) { const value = await chrome.storage.local.get(key); return value[key] ?? fallback; }
async function writeStorage(key, value) { await chrome.storage.local.set({ [key]: value }); }
async function loadPairing(force = false) {
  if (pairing && !force) return pairing;
  try {
    const response = await fetch(`${chrome.runtime.getURL('pairing.json')}?v=${Date.now()}`, { cache: 'no-store' });
    if (!response.ok) throw new Error('Pairing is unavailable');
    const candidate = await response.json();
    if (!candidate || typeof candidate.endpoint !== 'string' || typeof candidate.token !== 'string' || !/^[0-9a-f]{24}$/.test(candidate.association_key) || typeof candidate.browser_instance !== 'string') throw new Error('Pairing is invalid');
    pairing = candidate;
    await writeStorage(PAIRING_KEY, candidate);
    return pairing;
  } catch (error) {
    if (!pairing) pairing = await readStorage(PAIRING_KEY, null);
    if (!pairing) throw error;
    return pairing;
  }
}
async function apiRequest(path, options = {}) {
  const config = await loadPairing();
  const endpoint = String(config.endpoint).replace(/\/$/, '');
  const headers = { Authorization: `Bearer ${config.token}`, 'Content-Type': 'application/json', 'X-Cockpit-Association': config.association_key };
  const response = await fetch(`${endpoint}${path}`, { method: options.body ? 'POST' : 'GET', headers, body: options.body ? JSON.stringify(options.body) : undefined, cache: 'no-store' });
  let body = null; try { body = await response.json(); } catch { body = null; }
  if (!response.ok) throw new Error(typeof body?.message === 'string' ? body.message.slice(0, 400) : `Space request failed (${response.status})`);
  return body || {};
}
async function activeTab() {
  const result = await chrome.tabs.query({ active: true, currentWindow: true });
  const tab = result[0];
  if (!tab || !Number.isInteger(tab.id) || !isPageUrl(tab.url)) throw new Error('Select an HTTP(S) page first');
  return tab;
}
async function probeDocument(tabId) {
  const result = await chrome.scripting.executeScript({ target: { tabId, frameIds: [0] }, func: () => ({ url: location.href }) });
  const injected = result?.[0];
  if (!injected?.documentId) throw new Error('The page document could not be identified');
  documentByTab.set(tabId, injected.documentId);
  return injected.documentId;
}
async function draftFor(tabId, documentId) {
  const drafts = await readStorage(DRAFTS_KEY, []);
  return drafts.find((draft) => draft.tab_id === tabId && draft.document_id === documentId && !draft.stale) || null;
}
async function ensureContent(tab) {
  const result = await chrome.scripting.executeScript({ target: { tabId: tab.id, frameIds: [0] }, files: ['content.js'] });
  const documentId = result?.[0]?.documentId || await probeDocument(tab.id);
  documentByTab.set(tab.id, documentId);
  await chrome.tabs.sendMessage(tab.id, { type: 'init', tab_id: tab.id, document_id: documentId, draft: await draftFor(tab.id, documentId), mode: 'browse' }, { frameId: 0 });
  return documentId;
}
async function sendToPage(tab, message, expectedDocument) {
  const known = documentByTab.get(tab.id) || await ensureContent(tab);
  if (expectedDocument && known !== expectedDocument) throw new Error('The page changed; review the new document');
  const result = await chrome.tabs.sendMessage(tab.id, { ...message, document_id: known }, { frameId: 0 });
  if (result?.error) throw new Error(result.error);
  return result;
}
function validContentSender(sender, message) {
  const tabId = sender?.tab?.id;
  return sender?.id === EXPECTED_EXTENSION_ID && sender.frameId === 0 && Number.isInteger(tabId) && isPageUrl(sender.tab.url) && typeof sender.documentId === 'string' && message.document_id === sender.documentId;
}
function validExtensionSender(sender) { return sender?.id === EXPECTED_EXTENSION_ID && typeof sender.url === 'string' && sender.url.startsWith(chrome.runtime.getURL('')); }
async function saveDraft(draft) {
  if (!draft || !Number.isInteger(draft.tab_id) || typeof draft.document_id !== 'string') throw new Error('Invalid draft');
  const drafts = await readStorage(DRAFTS_KEY, []);
  const next = drafts.filter((item) => !(item.tab_id === draft.tab_id && item.document_id === draft.document_id));
  next.unshift({ ...draft, updated_at: draft.updated_at || new Date().toISOString(), stale: Boolean(draft.stale) });
  await writeStorage(DRAFTS_KEY, next.slice(0, MAX_DRAFTS));
}
function decodePng(dataUrl) {
  if (typeof dataUrl !== 'string' || !dataUrl.startsWith('data:image/png;base64,')) throw new Error('Capture was not a PNG');
  const encoded = dataUrl.slice(dataUrl.indexOf(',') + 1);
  const bytes = Math.floor(encoded.length * 3 / 4) - (encoded.endsWith('==') ? 2 : encoded.endsWith('=') ? 1 : 0);
  if (bytes <= 0 || bytes > MAX_PNG_BYTES) throw new Error('Capture exceeds the 4 MiB limit');
  const raw = atob(encoded);
  if (raw.length < 33 || raw.slice(0, 8) !== '\x89PNG\r\n\x1a\n' || raw.slice(12, 16) !== 'IHDR') throw new Error('Capture has an invalid PNG header');
  const width = raw.charCodeAt(16) * 0x1000000 + raw.charCodeAt(17) * 0x10000 + raw.charCodeAt(18) * 0x100 + raw.charCodeAt(19);
  const height = raw.charCodeAt(20) * 0x1000000 + raw.charCodeAt(21) * 0x10000 + raw.charCodeAt(22) * 0x100 + raw.charCodeAt(23);
  if (!width || !height || width > 8192 || height > 8192 || width * height > 16000000) throw new Error('Capture dimensions exceed the limit');
  return { base64: encoded, width, height };
}
async function capture(tab) {
  if (await readStorage(PENDING_KEY, null)) throw new Error('Save or discard the pending capture before capturing again');
  const config = await loadPairing();
  const documentId = await ensureContent(tab);
  await sendToPage(tab, { type: 'prepare-capture' }, documentId);
  const before = await probeDocument(tab.id);
  if (before !== documentId) { await sendToPage(tab, { type: 'cancel-capture' }, before).catch(() => {}); throw new Error('The page navigated before capture'); }
  let image;
  let finished;
  let finalized = false;
  try {
    try { image = await chrome.tabs.captureVisibleTab(tab.windowId, { format: 'png' }); }
    catch (error) { throw new Error(`Capture failed: ${publicError(error)}`); }
    const png = decodePng(image);
    const after = await probeDocument(tab.id);
    if (after !== documentId) throw new Error('The page navigated during capture; evidence was retained as a draft');
    finished = await sendToPage(tab, { type: 'finish-capture', image_width: png.width, image_height: png.height, page: { tab_id: tab.id, image_width: png.width, image_height: png.height } }, documentId);
    finalized = true;
    const submission = { association_key: config.association_key, browser_instance: config.browser_instance, capture_id: crypto.randomUUID(), page: { ...finished.page, tab_id: tab.id, image_width: png.width, image_height: png.height }, annotations: finished.annotations, png_base64: png.base64 };
    const requestBytes = new TextEncoder().encode(JSON.stringify(submission)).length;
    await writeStorage(PENDING_KEY, { submission, request_bytes: requestBytes, tab_id: tab.id, document_id: documentId, created_at: new Date().toISOString() });
    if (requestBytes > MAX_REQUEST_BYTES) throw new Error('Capture request exceeds the 6 MiB limit; pending evidence was retained');
    return savePending();
  } finally {
    if (!finalized) await sendToPage(tab, { type: 'cancel-capture' }, documentId).catch(() => {});
  }
}
async function savePending() {
  const pending = await readStorage(PENDING_KEY, null); if (!pending?.submission) throw new Error('There is no pending capture');
  if (pending.request_bytes > MAX_REQUEST_BYTES) throw new Error('Capture request exceeds the 6 MiB limit; pending evidence was retained');
  const config = await loadPairing();
  if (pending.submission.association_key !== config.association_key) throw new Error('Pending capture belongs to the previous Space pairing; reconnect that pairing to retry');
  const result = await apiRequest('/capture', { body: pending.submission });
  await writeStorage(PENDING_KEY, null);
  const drafts = await readStorage(DRAFTS_KEY, []);
  const savedIds = new Set(pending.submission.annotations.map(annotation => annotation.id));
  await writeStorage(DRAFTS_KEY, drafts.map(draft => draft.tab_id === pending.tab_id && draft.document_id === pending.document_id ? { ...draft, annotations: draft.annotations.filter(annotation => !savedIds.has(annotation.id)) } : draft).filter(draft => draft.annotations.length));
  await chrome.tabs.sendMessage(pending.tab_id, { type: 'capture-saved', document_id: pending.document_id, ids: [...savedIds] }, { documentId: pending.document_id }).catch(() => {});
  return result;
}
async function context() {
  const config = await loadPairing();
  let tab = null;
  try { tab = await activeTab(); } catch { /* A pending capture can be retried after navigation to a restricted page. */ }
  let statusResult = { association_key: config.association_key, space_label: config.space_label || 'Cockpit Space', connected: false, pending_count: 0 };
  try { statusResult = { ...statusResult, ...(await apiRequest('/status')) }; } catch { /* Offline status is shown without exposing credentials. */ }
  return { ...statusResult, tab_id: tab?.id ?? null, tabs: tab ? [{ tab_id: tab.id, title: tab.title, url: tab.url }] : [], has_pending_capture: Boolean(await readStorage(PENDING_KEY, null)) };
}
async function handle(message, sender) {
  if (message.type === 'draft') { if (!validContentSender(sender, message)) throw new Error('Unauthorized page sender'); await saveDraft(message.draft); return { saved: true }; }
  if (message.type === 'capture-page') {
    if (!validContentSender(sender, message)) throw new Error('Unauthorized page sender');
    const tab = await activeTab();
    if (tab.id !== sender.tab.id) throw new Error('Select the annotated tab before capturing');
    if (await probeDocument(tab.id) !== sender.documentId) throw new Error('The original page document is no longer active');
    return capture(tab);
  }
  if (!validExtensionSender(sender)) throw new Error('Unauthorized extension sender');
  if (message.type === 'context') return context();
  if (message.type === 'reconnect') { await loadPairing(true); return context(); }
  if (message.type === 'save-pending') return savePending();
  if (message.type === 'discard-pending') { await writeStorage(PENDING_KEY, null); return { discarded: true }; }
  const tab = Number.isInteger(message.tab_id) ? await chrome.tabs.get(message.tab_id) : await activeTab();
  if (!tab || !Number.isInteger(tab.id) || !isPageUrl(tab.url)) throw new Error('Select an HTTP(S) page first');
  const documentId = await ensureContent(tab);
  if (message.type === 'mode') return sendToPage(tab, { type: 'mode', mode: message.mode }, documentId);
  if (message.type === 'capture') return capture(tab);
  throw new Error('Unknown extension action');
}
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => { Promise.resolve().then(() => handle(message, sender)).then(sendResponse).catch((error) => sendResponse({ error: publicError(error) })); return true; });
chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (!changeInfo.status && !changeInfo.url) return;
  documentByTab.delete(tabId);
  readStorage(DRAFTS_KEY, []).then((drafts) => writeStorage(DRAFTS_KEY, drafts.map((draft) => draft.tab_id === tabId ? { ...draft, stale: true } : draft))).catch(() => {});
});
