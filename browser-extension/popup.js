const $ = (selector) => document.querySelector(selector);
const space = $('#space-label');
const connection = $('#connection');
const status = $('#status');
const pending = $('#pending');
const tabs = $('#tab-select');
const annotate = $('#annotate');
const capture = $('#capture');
const retry = $('#retry');
const discard = $('#discard');
const reconnect = $('#reconnect');
const draftCount = $('#draft-count');
const activeDrafts = $('#active-drafts');
const staleRecovery = $('#stale-recovery');
const staleCount = $('#stale-count');
const staleDrafts = $('#stale-drafts');
let context = null;

function send(message) {
  return new Promise((resolve, reject) => chrome.runtime.sendMessage(message, (result) => {
    const error = chrome.runtime.lastError;
    if (error) reject(new Error(error.message));
    else if (result?.error) reject(new Error(result.error));
    else resolve(result);
  }));
}
function setStatus(text, kind = '') {
  status.textContent = text;
  status.className = `status ${kind}`;
}
function setBusy(value) {
  [annotate, capture, retry, discard, reconnect].forEach((button) => { button.disabled = value; });
  document.querySelectorAll('.drafts button').forEach((button) => { button.disabled = value; });
}
function selectedTab() { const value = Number(tabs.value); return Number.isInteger(value) ? value : undefined; }
function renderTabs(items) {
  tabs.replaceChildren();
  for (const tab of items || []) {
    const option = document.createElement('option'); option.value = String(tab.tab_id); option.textContent = tab.title || tab.url || `Tab ${tab.tab_id}`; tabs.append(option);
  }
  if (context?.tab_id != null) tabs.value = String(context.tab_id);
}
function draftTitle(draft) { return draft.title || draft.url || `Tab ${draft.tab_id}`; }
function appendDraft(root, draft, stale) {
  const card = document.createElement('article');
  card.className = `draft-card${stale ? ' stale' : ''}`;
  const heading = document.createElement('div'); heading.className = 'draft-heading';
  const title = document.createElement('strong'); title.textContent = draftTitle(draft); heading.append(title);
  if (stale) {
    const button = document.createElement('button'); button.type = 'button'; button.className = 'secondary danger'; button.textContent = 'Discard stale draft';
    button.addEventListener('click', async () => {
      setBusy(true); setStatus('Discarding stale draft…');
      try { await send({ type: 'discard-draft', tab_id: draft.tab_id, document_id: draft.document_id }); render(await send({ type: 'context' })); setStatus('Stale draft discarded.'); }
      catch (error) { setStatus(error.message, 'error'); }
      finally { setBusy(false); }
    });
    heading.append(button);
  }
  card.append(heading);
  const meta = document.createElement('p'); meta.className = 'draft-meta'; meta.textContent = `${draft.url || 'Unknown URL'} · document ${draft.document_id || 'unknown'}`; card.append(meta);
  const list = document.createElement('ul'); list.className = 'draft-annotations';
  const annotations = Array.isArray(draft.annotations) ? draft.annotations : [];
  for (const annotation of annotations) {
    const item = document.createElement('li');
    const kind = document.createElement('strong'); kind.textContent = annotation.kind || 'mark'; item.append(kind);
    const summary = annotation.comment || annotation.target || 'Mark without comment'; item.append(document.createTextNode(` — ${summary}`));
    list.append(item);
  }
  if (draft.annotations_truncated) {
    const item = document.createElement('li'); item.className = 'muted'; item.textContent = `+ ${Math.max(0, draft.annotation_count - annotations.length)} more marks`; list.append(item);
  }
  card.append(list);
  root.append(card);
}
function renderDrafts(result) {
  const active = Array.isArray(result.active_drafts) ? result.active_drafts : [];
  const stale = Array.isArray(result.stale_drafts) ? result.stale_drafts : [];
  draftCount.textContent = String(result.draft_count ?? active.length + stale.length);
  activeDrafts.replaceChildren();
  if (!active.length) {
    const empty = document.createElement('p'); empty.className = 'note'; empty.textContent = 'No active page drafts retained.'; activeDrafts.append(empty);
  } else active.forEach((draft) => appendDraft(activeDrafts, draft, false));
  staleDrafts.replaceChildren();
  staleCount.textContent = String(stale.length);
  staleRecovery.hidden = !stale.length;
  stale.forEach((draft) => appendDraft(staleDrafts, draft, true));
}
function render(result) {
  context = result;
  pending.textContent = result.has_pending_capture ? 'One capture waiting to be saved' : (result.pending_count ? `${result.pending_count} pending annotation${result.pending_count === 1 ? '' : 's'}` : 'No pending capture');
  space.textContent = result.space_label || 'Cockpit Space';
  connection.textContent = result.connected ? 'Connected' : 'Offline';
  connection.className = `badge ${result.connected ? 'connected' : 'offline'}`;
  retry.hidden = !result.has_pending_capture;
  discard.hidden = !result.has_pending_capture;
  renderTabs(result.tabs);
  if (result.error) setStatus(result.error, 'error');
  renderDrafts(result);
}
async function refresh(reload = false) {
  setBusy(true);
  try {
    const result = await send({ type: reload ? 'reconnect' : 'context' });
    render(result);
    if (!result.error) setStatus(result.connected ? 'Ready to annotate the live page.' : 'Pairing is offline; drafts remain available.', result.connected ? '' : 'error');
  } catch (error) { setStatus(error.message, 'error'); }
  finally { setBusy(false); }
}
annotate.addEventListener('click', async () => { try { await send({ type: 'mode', tab_id: selectedTab(), mode: 'annotate' }); setStatus('Annotate mode is active on the page.'); window.close(); } catch (error) { setStatus(error.message, 'error'); } });
capture.addEventListener('click', async () => { setBusy(true); setStatus('Capturing the visible page…'); try { const result = await send({ type: 'capture', tab_id: selectedTab() }); render(await send({ type: 'context' })); setStatus(`Saved ${result.annotation_ids.length} annotation${result.annotation_ids.length === 1 ? '' : 's'}.`); } catch (error) { setStatus(error.message, 'error'); } finally { setBusy(false); } });
retry.addEventListener('click', async () => { setBusy(true); setStatus('Retrying pending evidence…'); try { const result = await send({ type: 'save-pending' }); render(await send({ type: 'context' })); setStatus(`Saved ${result.annotation_ids.length} annotation${result.annotation_ids.length === 1 ? '' : 's'}.`); } catch (error) { setStatus(error.message, 'error'); } finally { setBusy(false); } });
discard.addEventListener('click', async () => { setBusy(true); try { await send({ type: 'discard-pending' }); render(await send({ type: 'context' })); setStatus('Pending evidence discarded.'); } catch (error) { setStatus(error.message, 'error'); } finally { setBusy(false); } });
reconnect.addEventListener('click', () => refresh(true));
tabs.addEventListener('change', () => { if (context) context.tab_id = selectedTab(); });
refresh();
