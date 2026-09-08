const $ = (selector) => document.querySelector(selector);
const spaceLabel = $('#space-label');
const controlBadge = $('#control-badge');
const status = $('#status');
const tabSelect = $('#tab-select');
const comment = $('#comment');
const selectButton = $('#select-element');
const captureButton = $('#capture');
const fullButton = $('#open-full');

function send(message) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendMessage(message, (response) => {
      if (chrome.runtime.lastError) return reject(new Error(chrome.runtime.lastError.message));
      if (!response?.ok) return reject(new Error(response?.error || 'The Space request failed.'));
      resolve(response);
    });
  });
}

function setStatus(text, kind = '') {
  status.textContent = text;
  status.className = `status ${kind}`;
}

function selectedBrowserTabId() {
  const value = Number(tabSelect.value);
  return Number.isInteger(value) ? value : undefined;
}

async function loadContext() {
  try {
    const response = await send({ type: 'COCKPIT_GET_CONTEXT' });
    const state = response.state || {};
    spaceLabel.textContent = state.spaceLabel || 'Unnamed Space';
    controlBadge.textContent = state.control === 'agent' ? 'Agent control' : 'Human control';
    controlBadge.className = `badge ${state.control === 'agent' ? 'agent' : 'human'}`;
    tabSelect.replaceChildren();
    const tabs = response.browserTabs || [];
    for (const tab of tabs) {
      const option = document.createElement('option');
      option.value = String(tab.browserTabId);
      option.textContent = tab.title ? `${tab.title} — ${tab.url}` : tab.url;
      tabSelect.append(option);
    }
    if (response.activeBrowserTabId !== null && response.activeBrowserTabId !== undefined) {
      tabSelect.value = String(response.activeBrowserTabId);
    }
    if (!tabs.length) {
      const option = document.createElement('option');
      option.textContent = 'No eligible HTTP/HTTPS tabs';
      option.value = '';
      tabSelect.append(option);
      selectButton.disabled = true;
      captureButton.disabled = true;
      setStatus('Open an HTTP or HTTPS page to begin.', 'error');
    } else {
      selectButton.disabled = false;
      captureButton.disabled = false;
      setStatus('Ready');
    }
  } catch (error) {
    setStatus(error.message, 'error');
    selectButton.disabled = true;
    captureButton.disabled = true;
  }
}

selectButton.addEventListener('click', async () => {
  selectButton.disabled = true;
  captureButton.disabled = true;
  setStatus('Click an element in the page…');
  try {
    await send({
      type: 'COCKPIT_START_PICKER',
      browserTabId: selectedBrowserTabId(),
      comment: comment.value,
    });
    window.close();
  } catch (error) {
    setStatus(error.message, 'error');
    selectButton.disabled = false;
    captureButton.disabled = false;
  }
});

captureButton.addEventListener('click', async () => {
  selectButton.disabled = true;
  captureButton.disabled = true;
  setStatus('Capturing the selected page…');
  try {
    await send({ type: 'COCKPIT_CAPTURE', browserTabId: selectedBrowserTabId() });
    window.close();
  } catch (error) {
    setStatus(error.message, 'error');
    selectButton.disabled = false;
    captureButton.disabled = false;
  }
});

fullButton.addEventListener('click', () => {
  chrome.tabs.create({ url: `${chrome.runtime.getURL('popup.html')}?full=1` });
});

loadContext();
