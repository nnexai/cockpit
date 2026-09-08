const $ = (selector) => document.querySelector(selector);
const space = $('#space-label');
const connection = $('#connection');
const status = $('#status');
const pending = $('#pending');
const tabs = $('#tab-select');
const annotate = $('#annotate');
const browse = $('#browse');
const capture = $('#capture');
const retry = $('#retry');
const discard = $('#discard');
const reconnect = $('#reconnect');
let context = null;

function send(message) {
  return new Promise((resolve, reject) => chrome.runtime.sendMessage(message, (result) => {
    const error = chrome.runtime.lastError;
    if (error) reject(new Error(error.message));
    else if (result?.error) reject(new Error(result.error));
    else resolve(result);
  }));
}
function setStatus(text, kind = '') { status.textContent = text; status.className = `status ${kind}`; }
function selectedTab() { const value = Number(tabs.value); return Number.isInteger(value) ? value : undefined; }
function setBusy(value) { [annotate, browse, capture, retry, discard, reconnect].forEach((button) => { button.disabled = value; }); }
function renderTabs(items) {
  tabs.replaceChildren();
  for (const tab of items || []) {
    const option = document.createElement('option'); option.value = String(tab.tab_id); option.textContent = tab.title || tab.url || `Tab ${tab.tab_id}`; tabs.append(option);
  }
  if (context?.tab_id != null) tabs.value = String(context.tab_id);
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
}
async function refresh(reload = false) {
  setBusy(true);
  try { render(await send({ type: reload ? 'reconnect' : 'context' })); setStatus(context.connected ? 'Ready to annotate the live page.' : 'Pairing is offline; drafts remain available.', context.connected ? '' : 'error'); }
  catch (error) { setStatus(error.message, 'error'); }
  finally { setBusy(false); }
}
annotate.addEventListener('click', async () => { try { await send({ type: 'mode', tab_id: selectedTab(), mode: 'annotate' }); setStatus('Annotate mode is active on the page.'); window.close(); } catch (error) { setStatus(error.message, 'error'); } });
browse.addEventListener('click', async () => { try { await send({ type: 'mode', tab_id: selectedTab(), mode: 'browse' }); setStatus('Browse mode is active.'); window.close(); } catch (error) { setStatus(error.message, 'error'); } });
capture.addEventListener('click', async () => { setBusy(true); setStatus('Capturing the visible page…'); try { const result = await send({ type: 'capture', tab_id: selectedTab() }); render(await send({ type: 'context' })); setStatus(`Saved ${result.annotation_ids.length} annotation${result.annotation_ids.length === 1 ? '' : 's'}.`); } catch (error) { setStatus(error.message, 'error'); } finally { setBusy(false); } });
retry.addEventListener('click', async () => { setBusy(true); setStatus('Retrying pending evidence…'); try { const result = await send({ type: 'save-pending' }); render(await send({ type: 'context' })); setStatus(`Saved ${result.annotation_ids.length} annotation${result.annotation_ids.length === 1 ? '' : 's'}.`); } catch (error) { setStatus(error.message, 'error'); } finally { setBusy(false); } });
discard.addEventListener('click', async () => { setBusy(true); try { await send({ type: 'discard-pending' }); render(await send({ type: 'context' })); setStatus('Pending evidence discarded.'); } catch (error) { setStatus(error.message, 'error'); } finally { setBusy(false); } });
reconnect.addEventListener('click', () => refresh(true));
tabs.addEventListener('change', () => { if (context) context.tab_id = selectedTab(); });
refresh();
