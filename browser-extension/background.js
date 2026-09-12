const EXPECTED_EXTENSION_ID = 'fblkilbfbmpndnfjaacljmcljhepakok';
const MAX_PNG_BYTES = 4 * 1024 * 1024;
const MAX_REQUEST_BYTES = 6 * 1024 * 1024;
const MAX_DRAFTS = 8;
const MAX_DRAFT_SUMMARIES = 8;
const MAX_ANNOTATION_SUMMARIES = 32;
const MAX_SUMMARY_TEXT = 240;
const NETWORK_TIMEOUT_MS = 5000;
const DOCUMENT_PROBE_TIMEOUT_MS = 2000;
const ACTIVATION_GENERATIONS = new Map();
const DRAFTS_KEY = 'cockpit.feedback.drafts';
const CONSUMED_KEY = 'cockpit.feedback.consumed';
const MAX_CONSUMED_MARKERS = MAX_DRAFTS * 8;
const MAX_CONSUMED_IDS_PER_MARKER = 128;
const PENDING_KEY = 'cockpit.feedback.pending';
const PAIRING_KEY = 'cockpit.feedback.pairing';
let pairing = null;
const documentByTab = new Map();
let storageTail = Promise.resolve();
let workerTail = Promise.resolve();
let pendingSaveInFlight = null;

function enqueueStorage(task) {
  const result = storageTail.then(task, task);
  storageTail = result.catch(() => {});
  return result;
}
function enqueueWorker(task) {
  const result = workerTail.then(task, task);
  workerTail = result.catch(() => {});
  return result;
}
function publicError(error) { return error instanceof Error ? error.message.slice(0, 400) : 'Request failed'; }
function isPageUrl(url) { return typeof url === 'string' && /^https?:\/\//i.test(url); }
function withDeadline(task, timeoutMs, timeoutMessage) {
  let timer;
  const operation = Promise.resolve().then(task);
  const deadline = new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error(timeoutMessage)), timeoutMs);
  });
  return Promise.race([operation, deadline]).finally(() => clearTimeout(timer));
}
async function fetchJsonWithDeadline(url, options = {}) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), NETWORK_TIMEOUT_MS);
  try {
    const response = await fetch(url, { ...options, signal: controller.signal });
    let body = null;
    try {
      body = await response.json();
    } catch (error) {
      if (error?.name === 'AbortError') throw error;
    }
    return { response, body };
  } catch (error) {
    if (error?.name === 'AbortError') throw new Error(`Network request timed out after ${NETWORK_TIMEOUT_MS}ms`);
    throw error;
  } finally {
    clearTimeout(timer);
  }
}
async function readStorage(key, fallback) {
  return enqueueStorage(async () => {
    const value = await chrome.storage.local.get(key);
    return value[key] ?? fallback;
  });
}
function writeStorage(key, value) {
  return enqueueStorage(() => chrome.storage.local.set({ [key]: value }));
}
function normalizeConsumedMarkers(value) {
  const markers = new Map();
  for (const marker of Array.isArray(value) ? value : []) {
    if (!Number.isInteger(marker?.tab_id) || typeof marker?.document_id !== 'string') continue;
    const key = draftKey(marker.tab_id, marker.document_id);
    const current = markers.get(key) || { tab_id: marker.tab_id, document_id: marker.document_id, ids: [] };
    const seen = new Set(current.ids);
    for (const id of Array.isArray(marker.ids) ? marker.ids : []) {
      if (typeof id !== 'string' || !id || seen.has(id)) continue;
      seen.add(id);
      current.ids.push(id);
    }
    if (current.ids.length > MAX_CONSUMED_IDS_PER_MARKER) current.ids = current.ids.slice(-MAX_CONSUMED_IDS_PER_MARKER);
    if (current.ids.length) markers.set(key, current);
  }
  return [...markers.values()];
}
function consumedMarkersEqual(left, right) {
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
  return left.every((marker, index) => {
    const other = right[index];
    return marker?.tab_id === other?.tab_id
      && marker?.document_id === other?.document_id
      && Array.isArray(marker?.ids)
      && Array.isArray(other?.ids)
      && marker.ids.length === other.ids.length
      && marker.ids.every((id, idIndex) => id === other.ids[idIndex]);
  });
}
function boundConsumedMarkers(markers, drafts = [], pending = null) {
  const normalized = normalizeConsumedMarkers(markers);
  if (normalized.length <= MAX_CONSUMED_MARKERS) return normalized;
  const protectedKeys = new Set(
    (Array.isArray(drafts) ? drafts : [])
      .filter(hasDraftAnnotations)
      .map((draft) => draftKey(draft.tab_id, draft.document_id)),
  );
  if (Number.isInteger(pending?.tab_id) && typeof pending?.document_id === 'string') protectedKeys.add(draftKey(pending.tab_id, pending.document_id));
  const unprotected = normalized.filter((marker) => !protectedKeys.has(draftKey(marker.tab_id, marker.document_id)));
  const keep = new Set(unprotected.slice(-Math.max(0, MAX_CONSUMED_MARKERS - protectedKeys.size)).map((marker) => draftKey(marker.tab_id, marker.document_id)));
  return normalized.filter((marker) => protectedKeys.has(draftKey(marker.tab_id, marker.document_id)) || keep.has(draftKey(marker.tab_id, marker.document_id)));
}
function consumedIdsFor(markers, tabId, documentId) {
  const marker = (Array.isArray(markers) ? markers : []).find((item) => item.tab_id === tabId && item.document_id === documentId);
  return marker ? new Set(marker.ids) : null;
}
function mergeConsumedMarkers(markers, tabId, documentId, ids) {
  const key = draftKey(tabId, documentId);
  const normalized = normalizeConsumedMarkers(markers);
  const existing = normalized.find((marker) => draftKey(marker.tab_id, marker.document_id) === key);
  const next = normalized.filter((marker) => draftKey(marker.tab_id, marker.document_id) !== key);
  const incoming = ids instanceof Set ? [...ids] : Array.isArray(ids) ? ids : [];
  const uniqueIds = [...new Set([...(existing?.ids || []), ...incoming].filter((id) => typeof id === 'string' && id))];
  if (uniqueIds.length) next.push({ tab_id: tabId, document_id: documentId, ids: uniqueIds });
  return normalizeConsumedMarkers(next);
}
async function mutateFeedbackStorage(update) {
  return enqueueStorage(async () => {
    const draftsValue = await chrome.storage.local.get(DRAFTS_KEY);
    const pendingValue = await chrome.storage.local.get(PENDING_KEY);
    const consumedValue = await chrome.storage.local.get(CONSUMED_KEY);
    const consumed = normalizeConsumedMarkers(consumedValue[CONSUMED_KEY]);
    const next = await update({
      drafts: Array.isArray(draftsValue[DRAFTS_KEY]) ? draftsValue[DRAFTS_KEY] : [],
      pending: pendingValue[PENDING_KEY] ?? null,
      consumed,
    });
    if (!next || !Array.isArray(next.drafts)) throw new Error('Feedback storage update was invalid');
    const nextConsumed = boundConsumedMarkers(next.consumed === undefined ? consumed : next.consumed, next.drafts, next.pending);
    await chrome.storage.local.set({
      [DRAFTS_KEY]: next.drafts,
      [PENDING_KEY]: next.pending ?? null,
      [CONSUMED_KEY]: nextConsumed,
    });
    return { ...next, consumed: nextConsumed };
  });
}
async function loadPairing(force = false) {
  if (pairing && !force) return pairing;
  try {
    const { response, body } = await fetchJsonWithDeadline(`${chrome.runtime.getURL('pairing.json')}?v=${Date.now()}`, { cache: 'no-store' });
    if (!response.ok) throw new Error('Pairing is unavailable');
    const candidate = body;
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
  const { response, body } = await fetchJsonWithDeadline(`${endpoint}${path}`, { method: options.body ? 'POST' : 'GET', headers, body: options.body ? JSON.stringify(options.body) : undefined, cache: 'no-store' });
  if (!response.ok) throw new Error(typeof body?.message === 'string' ? body.message.slice(0, 400) : `Space request failed (${response.status})`);
  return body || {};
}
async function activeTab() {
  const result = await chrome.tabs.query({ active: true, currentWindow: true });
  const tab = result[0];
  if (!tab || !Number.isInteger(tab.id) || !isPageUrl(tab.url)) throw new Error('Select an HTTP(S) page first');
  return tab;
}
function validExtensionSender(sender) {
  if (sender?.id !== EXPECTED_EXTENSION_ID || sender?.tab != null) return false;
  const extensionOrigin = `chrome-extension://${EXPECTED_EXTENSION_ID}`;
  const url = sender?.url;
  return (typeof url === 'string' && (url === extensionOrigin || url.startsWith(`${extensionOrigin}/`))) || sender?.origin === extensionOrigin;
}
async function probeDocument(tabId) {
  const result = await withDeadline(
    () => chrome.scripting.executeScript({ target: { tabId, frameIds: [0] }, func: () => ({ url: location.href }) }),
    DOCUMENT_PROBE_TIMEOUT_MS,
    'The page document probe timed out',
  );
  const injected = result?.[0];
  if (!injected?.documentId) throw new Error('The page document could not be identified');
  documentByTab.set(tabId, injected.documentId);
  return injected.documentId;
}
async function draftFor(tabId, documentId) {
  const drafts = await readStorage(DRAFTS_KEY, []);
  return (Array.isArray(drafts) ? drafts : []).find((draft) => draft.tab_id === tabId && draft.document_id === documentId && !draft.stale) || null;
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
async function openToolbar(tab) {
  if (!tab || !Number.isInteger(tab.id) || !isPageUrl(tab.url)) throw new Error('Select an HTTP(S) page first');
  const documentId = await ensureContent(tab);
  return sendToPage(tab, { type: 'mode', mode: 'annotate' }, documentId);
}
function validContentSender(sender, message) {
  const tabId = sender?.tab?.id;
  return sender?.id === EXPECTED_EXTENSION_ID && sender.frameId === 0 && Number.isInteger(tabId) && isPageUrl(sender.tab.url) && typeof sender.documentId === 'string' && message.document_id === sender.documentId;
}
function draftKey(tabId, documentId) { return `${tabId}:${documentId}`; }
async function saveDraft(draft) {
  if (!draft || !Number.isInteger(draft.tab_id) || typeof draft.document_id !== 'string') throw new Error('Invalid draft');
  return mutateFeedbackStorage(async ({ drafts, pending, consumed }) => {
    const existing = drafts.find((item) => item.tab_id === draft.tab_id && item.document_id === draft.document_id);
    const incomingAnnotations = Array.isArray(draft.annotations) ? draft.annotations : [];
    const consumedIds = consumedIdsFor(consumed, draft.tab_id, draft.document_id);
    const annotations = consumedIds ? incomingAnnotations.filter((annotation) => !consumedIds.has(annotation?.id)) : incomingAnnotations;
    if (!annotations.length) {
      return { drafts: drafts.filter((item) => !(item.tab_id === draft.tab_id && item.document_id === draft.document_id)), pending, consumed };
    }
    if (!existing && drafts.length >= MAX_DRAFTS) throw new Error(`Maximum of ${MAX_DRAFTS} unfinished drafts reached; discard a stale draft before creating another`);
    const next = drafts.filter((item) => !(item.tab_id === draft.tab_id && item.document_id === draft.document_id));
    next.unshift({
      ...draft,
      annotations,
      updated_at: draft.updated_at || new Date().toISOString(),
      stale: Boolean(draft.stale || existing?.stale),
    });
    return { drafts: next, pending, consumed };
  });
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
async function verifyVisibleCaptureTarget(tab, expectedDocument) {
  const visible = await activeTab();
  if (visible.id !== tab.id) throw new Error('Select the annotated tab before capturing');
  const documentId = await probeDocument(tab.id);
  if (expectedDocument && documentId !== expectedDocument) throw new Error('The original page document is no longer active');
  const stillVisible = await activeTab();
  if (stillVisible.id !== tab.id) throw new Error('The visible tab changed before capture');
  return documentId;
}
function activationGenerationFor(windowId) {
  return ACTIVATION_GENERATIONS.get(windowId) || 0;
}
async function capture(tab, expectedDocument = null) {
  if (await readStorage(PENDING_KEY, null)) throw new Error('Save or discard the pending capture before capturing again');
  const config = await loadPairing();
  const documentId = await ensureContent(tab);
  if (expectedDocument && documentId !== expectedDocument) throw new Error('The original page document is no longer active');
  let image;
  let finished;
  let finalized = false;
  try {
    await sendToPage(tab, { type: 'prepare-capture' }, documentId);
    const before = await verifyVisibleCaptureTarget(tab, documentId);
    if (before !== documentId) throw new Error('The page navigated before capture');
    const activationGeneration = activationGenerationFor(tab.windowId);
    try { image = await chrome.tabs.captureVisibleTab(tab.windowId, { format: 'png' }); }
    catch (error) { throw new Error(`Capture failed: ${publicError(error)}`); }
    if (activationGeneration !== activationGenerationFor(tab.windowId)) throw new Error('The visible tab changed during capture; evidence was retained as a draft');
    const png = decodePng(image);
    const after = await probeDocument(tab.id);
    if (after !== documentId) throw new Error('The page navigated during capture; evidence was retained as a draft');
    const visibleAfter = await activeTab();
    if (visibleAfter.id !== tab.id) throw new Error('The visible tab changed during capture; evidence was retained as a draft');
    finished = await sendToPage(tab, { type: 'finish-capture', image_width: png.width, image_height: png.height, page: { tab_id: tab.id, image_width: png.width, image_height: png.height } }, documentId);
    finalized = true;
    const submission = { association_key: config.association_key, browser_instance: config.browser_instance, capture_id: crypto.randomUUID(), page: { ...finished.page, tab_id: tab.id, image_width: png.width, image_height: png.height }, annotations: finished.annotations, png_base64: png.base64 };
    const requestBytes = new TextEncoder().encode(JSON.stringify(submission)).length;
    await mutateFeedbackStorage(async ({ drafts, pending }) => {
      if (pending) throw new Error('Save or discard the pending capture before capturing again');
      return { drafts, pending: { submission, request_bytes: requestBytes, tab_id: tab.id, document_id: documentId, created_at: new Date().toISOString() } };
    });
    if (requestBytes > MAX_REQUEST_BYTES) throw new Error('Capture request exceeds the 6 MiB limit; pending evidence was retained');
    return savePending();
  } finally {
    if (!finalized) await sendToPage(tab, { type: 'cancel-capture' }, documentId).catch(() => {});
  }
}
async function savePendingOnce() {
  const pending = await readStorage(PENDING_KEY, null); if (!pending?.submission) throw new Error('There is no pending capture');
  if (pending.request_bytes > MAX_REQUEST_BYTES) throw new Error('Capture request exceeds the 6 MiB limit; pending evidence was retained');
  const config = await loadPairing();
  if (pending.submission.association_key !== config.association_key) throw new Error('Pending capture belongs to the previous Space pairing; reconnect that pairing to retry');
  const submission = {
    ...pending.submission,
    annotations: pending.submission.annotations.map(annotation => ({
      ...annotation,
      element: annotation.element ? {
        tag: annotation.element.tag,
        text: annotation.element.text,
        role: annotation.element.role,
        name: annotation.element.name,
        locators: annotation.element.locators,
        excerpt: annotation.element.excerpt,
      } : null,
    })),
  };
  const result = await apiRequest('/capture', { body: submission });
  const savedIds = new Set((Array.isArray(pending.submission.annotations) ? pending.submission.annotations : []).map(annotation => annotation.id));
  const captureId = pending.submission.capture_id;
  await mutateFeedbackStorage(async ({ drafts, pending: currentPending, consumed }) => {
    const nextDrafts = drafts.map((draft) => {
      if (draft.tab_id !== pending.tab_id || draft.document_id !== pending.document_id) return draft;
      const annotations = (Array.isArray(draft.annotations) ? draft.annotations : []).filter(annotation => !savedIds.has(annotation.id));
      return annotations.length ? { ...draft, annotations } : null;
    }).filter(Boolean);
    const nextConsumed = mergeConsumedMarkers(consumed, pending.tab_id, pending.document_id, savedIds);
    const isCurrent = typeof captureId === 'string' && currentPending?.submission?.capture_id === captureId;
    return { drafts: nextDrafts, pending: isCurrent ? null : currentPending, consumed: nextConsumed };
  });
  await chrome.tabs.sendMessage(pending.tab_id, { type: 'capture-saved', document_id: pending.document_id, ids: [...savedIds] }, { documentId: pending.document_id }).catch(() => {});
  return result;
}
function savePending() {
  if (pendingSaveInFlight) return pendingSaveInFlight;
  const result = savePendingOnce();
  pendingSaveInFlight = result;
  result.then(() => { if (pendingSaveInFlight === result) pendingSaveInFlight = null; }, () => { if (pendingSaveInFlight === result) pendingSaveInFlight = null; });
  return result;
}
function summaryText(value, fallback = '') {
  return typeof value === 'string' ? value.replace(/[\u0000-\u001f\u007f]/g, ' ').trim().slice(0, MAX_SUMMARY_TEXT) : fallback;
}
function summarizeAnnotation(annotation) {
  const element = annotation?.element;
  return {
    kind: summaryText(annotation?.kind, 'mark'),
    comment: summaryText(annotation?.comment),
    target: element ? summaryText(element.name || element.text || element.tag) : '',
  };
}
function summarizeDraft(draft) {
  const annotations = Array.isArray(draft.annotations) ? draft.annotations : [];
  return {
    tab_id: draft.tab_id,
    document_id: draft.document_id,
    title: summaryText(draft.title, 'Untitled page'),
    url: summaryText(draft.url, 'Unknown URL'),
    updated_at: summaryText(draft.updated_at),
    stale: Boolean(draft.stale),
    annotation_count: annotations.length,
    annotations: annotations.slice(0, MAX_ANNOTATION_SUMMARIES).map(summarizeAnnotation),
    annotations_truncated: annotations.length > MAX_ANNOTATION_SUMMARIES,
  };
}
function summarizeDrafts(drafts, stale) {
  return (Array.isArray(drafts) ? drafts : [])
    .filter((draft) => Boolean(draft?.stale) === stale)
    .slice(0, MAX_DRAFT_SUMMARIES)
    .map(summarizeDraft);
}
function hasDraftAnnotations(draft) {
  return Array.isArray(draft?.annotations) && draft.annotations.length > 0;
}
function reconcileDraftList(drafts, liveTabIds, liveDocuments) {
  const inspectLiveTabs = liveTabIds !== null;
  const next = [];
  let changed = false;
  for (const draft of Array.isArray(drafts) ? drafts : []) {
    if (!hasDraftAnnotations(draft)) {
      changed = true;
      continue;
    }
    const replaced = inspectLiveTabs && !draft.stale && (
      !liveTabIds.has(draft.tab_id)
      || (liveDocuments.has(draft.tab_id) && liveDocuments.get(draft.tab_id) !== draft.document_id)
    );
    if (replaced && !draft.stale) {
      next.push({ ...draft, stale: true });
      changed = true;
    } else {
      next.push(draft);
    }
  }
  return { drafts: next, changed };
}
function reconcileConsumedList(consumed, liveTabIds, liveDocuments, drafts = [], pending = null) {
  const normalized = normalizeConsumedMarkers(consumed);
  const inspectLiveTabs = liveTabIds !== null;
  const retained = normalized.filter((marker) => !inspectLiveTabs
    || (liveTabIds.has(marker.tab_id) && (!liveDocuments.has(marker.tab_id) || liveDocuments.get(marker.tab_id) === marker.document_id)));
  const bounded = boundConsumedMarkers(retained, drafts, pending);
  return { markers: bounded, changed: !consumedMarkersEqual(consumed, bounded) };
}
async function reconcileStoredDrafts() {
  const stored = await readStorage(DRAFTS_KEY, []);
  const storedConsumed = await readStorage(CONSUMED_KEY, []);
  const storedPending = await readStorage(PENDING_KEY, null);
  const drafts = Array.isArray(stored) ? stored : [];
  const consumed = Array.isArray(storedConsumed) ? storedConsumed : [];
  const markerCandidates = boundConsumedMarkers(consumed, drafts, storedPending);
  const persist = async (liveTabIds, liveDocuments) => {
    const initialDrafts = reconcileDraftList(drafts, liveTabIds, liveDocuments);
    const initialConsumed = reconcileConsumedList(consumed, liveTabIds, liveDocuments, drafts, storedPending);
    if (!initialDrafts.changed && !initialConsumed.changed) return drafts;
    const updated = await mutateFeedbackStorage(async ({ drafts: currentDrafts, pending, consumed: currentConsumed }) => {
      const nextDrafts = reconcileDraftList(currentDrafts, liveTabIds, liveDocuments).drafts;
      const nextConsumed = reconcileConsumedList(currentConsumed, liveTabIds, liveDocuments, nextDrafts, pending).markers;
      return { drafts: nextDrafts, pending, consumed: nextConsumed };
    });
    return updated.drafts;
  };
  if (!drafts.some(hasDraftAnnotations) && !markerCandidates.length) return persist(null, new Map());
  let tabs;
  try {
    tabs = await chrome.tabs.query({});
  } catch {
    return persist(null, new Map());
  }
  if (!Array.isArray(tabs)) return persist(null, new Map());
  const liveTabIds = new Set(tabs.filter((tab) => Number.isInteger(tab?.id)).map((tab) => tab.id));
  for (const tabId of documentByTab.keys()) {
    if (!liveTabIds.has(tabId)) documentByTab.delete(tabId);
  }
  const tabsToProbe = new Set([
    ...drafts
      .filter((draft) => hasDraftAnnotations(draft) && !draft.stale && liveTabIds.has(draft.tab_id))
      .map((draft) => draft.tab_id),
    ...markerCandidates
      .filter((marker) => liveTabIds.has(marker.tab_id))
      .map((marker) => marker.tab_id),
  ]);
  const liveDocuments = new Map();
  for (const tabId of tabsToProbe) {
    try {
      liveDocuments.set(tabId, await probeDocument(tabId));
    } catch {
      documentByTab.delete(tabId);
    }
  }
  return persist(liveTabIds, liveDocuments);
}
async function discardStaleDraft(tabId, documentId) {
  if (!Number.isInteger(tabId) || typeof documentId !== 'string') throw new Error('Invalid stale draft');
  await mutateFeedbackStorage(async ({ drafts, pending }) => {
    const draft = drafts.find((item) => item.tab_id === tabId && item.document_id === documentId);
    if (!draft) throw new Error('That stale draft is no longer available');
    if (!draft.stale) throw new Error('Only stale drafts can be discarded from recovery');
    return { drafts: drafts.filter((item) => !(item.tab_id === tabId && item.document_id === documentId)), pending };
  });
  return { discarded: true };
}
async function markReplacedDraftsStale(tabId) {
  let currentDocument;
  try {
    currentDocument = await probeDocument(tabId);
  } catch {
    documentByTab.delete(tabId);
    return;
  }
  await mutateFeedbackStorage(async ({ drafts, pending, consumed }) => {
    const next = drafts
      .filter(hasDraftAnnotations)
      .map((draft) => draft.tab_id === tabId && !draft.stale && draft.document_id !== currentDocument ? { ...draft, stale: true } : draft);
    const nextConsumed = consumed.filter((marker) => marker.tab_id !== tabId || marker.document_id === currentDocument);
    return { drafts: next, pending, consumed: nextConsumed };
  });
}
async function markClosedDraftsStale(tabId) {
  if (!Number.isInteger(tabId)) return;
  documentByTab.delete(tabId);
  await mutateFeedbackStorage(async ({ drafts, pending, consumed }) => {
    const next = drafts
      .filter(hasDraftAnnotations)
      .map((draft) => draft.tab_id === tabId && !draft.stale ? { ...draft, stale: true } : draft);
    const nextConsumed = consumed.filter((marker) => marker.tab_id !== tabId);
    return { drafts: next, pending, consumed: nextConsumed };
  });
}
async function context(forcePairing = false) {
  const drafts = await reconcileStoredDrafts();
  const pendingCapture = await readStorage(PENDING_KEY, null);
  let tab = null;
  try { tab = await activeTab(); } catch { /* A pending capture can be retried after navigation to a restricted page. */ }
  let config = null;
  let error = null;
  try { config = await loadPairing(forcePairing); } catch (failure) { error = publicError(failure); }
  let statusResult = {
    association_key: config?.association_key || null,
    space_label: config?.space_label || 'Cockpit Space',
    connected: false,
    pending_count: 0,
    error,
  };
  if (config) {
    try {
      statusResult = { ...statusResult, ...(await apiRequest('/status')), error: null };
    } catch (failure) {
      statusResult.error = publicError(failure);
    }
  }
  return {
    ...statusResult,
    tab_id: tab?.id ?? null,
    tabs: tab ? [{ tab_id: tab.id, title: tab.title, url: tab.url }] : [],
    has_pending_capture: Boolean(pendingCapture),
    draft_count: drafts.length,
    active_drafts: summarizeDrafts(drafts, false),
    stale_drafts: summarizeDrafts(drafts, true),
  };
}
async function handle(message, sender) {
  if (!message || typeof message.type !== 'string') throw new Error('Unknown extension action');
  if (message.type === 'draft') { if (!validContentSender(sender, message)) throw new Error('Unauthorized page sender'); await saveDraft(message.draft); return { saved: true }; }
  if (message.type === 'capture-page') {
    if (!validContentSender(sender, message)) throw new Error('Unauthorized page sender');
    const tab = await activeTab();
    if (tab.id !== sender.tab.id) throw new Error('Select the annotated tab before capturing');
    if (await probeDocument(tab.id) !== sender.documentId) throw new Error('The original page document is no longer active');
    return capture(tab, sender.documentId);
  }
  if (!validExtensionSender(sender)) throw new Error('Unauthorized extension sender');
  if (message.type === 'context') return context();
  if (message.type === 'reconnect') return context(true);
  if (message.type === 'save-pending') return savePending();
  if (message.type === 'discard-pending') {
    await mutateFeedbackStorage(async ({ drafts }) => ({ drafts, pending: null }));
    return { discarded: true };
  }
  if (message.type === 'discard-draft') return discardStaleDraft(message.tab_id, message.document_id);
  const tab = Number.isInteger(message.tab_id) ? await chrome.tabs.get(message.tab_id) : await activeTab();
  if (!tab || !Number.isInteger(tab.id) || !isPageUrl(tab.url)) throw new Error('Select an HTTP(S) page first');
  const documentId = await ensureContent(tab);
  if (message.type === 'mode') return sendToPage(tab, { type: 'mode', mode: message.mode }, documentId);
  if (message.type === 'capture') return capture(tab, documentId);
  throw new Error('Unknown extension action');
}
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => { enqueueWorker(() => handle(message, sender)).then(sendResponse).catch((error) => sendResponse({ error: publicError(error) })); return true; });
chrome.commands?.onCommand.addListener((command, tab) => {
  if (command !== 'open-annotation-toolbar') return;
  enqueueWorker(() => openToolbar(tab)).catch(() => {});
});
chrome.tabs.onActivated.addListener(({ windowId }) => {
  if (!Number.isInteger(windowId)) return;
  ACTIVATION_GENERATIONS.set(windowId, activationGenerationFor(windowId) + 1);
});
chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (!changeInfo.status && !changeInfo.url) return;
  enqueueWorker(() => markReplacedDraftsStale(tabId)).catch(() => {});
});
chrome.tabs.onRemoved.addListener((tabId) => {
  enqueueWorker(() => markClosedDraftsStale(tabId)).catch(() => {});
});
