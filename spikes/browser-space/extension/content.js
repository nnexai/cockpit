(() => {
  if (globalThis.__cockpitSpacePickerLoaded) return;
  globalThis.__cockpitSpacePickerLoaded = true;

  const runtime = chrome.runtime;
  let picker = null;

  function send(message) {
    return new Promise((resolve, reject) => {
      runtime.sendMessage(message, (response) => {
        if (runtime.lastError) return reject(new Error(runtime.lastError.message));
        if (!response?.ok) return reject(new Error(response?.error || 'The Space request failed.'));
        resolve(response);
      });
    });
  }

  function viewport() {
    return {
      width: Math.max(0, Math.round(window.innerWidth)),
      height: Math.max(0, Math.round(window.innerHeight)),
      scrollX: window.scrollX,
      scrollY: window.scrollY,
      devicePixelRatio: window.devicePixelRatio || 1,
    };
  }

  function cssEscape(value) {
    if (globalThis.CSS?.escape) return CSS.escape(value);
    return String(value).replace(/[^a-zA-Z0-9_-]/g, (character) => `\\${character}`);
  }

  function selectorFor(element) {
    if (element.id) return `#${cssEscape(element.id)}`;
    const testId = element.getAttribute('data-testid');
    if (testId) return `[data-testid="${cssEscape(testId)}"]`;
    const parts = [];
    let current = element;
    while (current && current.nodeType === Node.ELEMENT_NODE && parts.length < 6) {
      let part = current.localName;
      if (!part) break;
      const classes = [...current.classList].filter(Boolean).slice(0, 2);
      if (classes.length) part += `.${classes.map(cssEscape).join('.')}`;
      const parent = current.parentElement;
      if (parent) {
        const siblings = [...parent.children].filter((sibling) => sibling.localName === current.localName);
        if (siblings.length > 1) part += `:nth-of-type(${siblings.indexOf(current) + 1})`;
      }
      parts.unshift(part);
      if (current.id) break;
      current = parent;
    }
    return parts.join(' > ');
  }

  function metadata(element) {
    const rect = element.getBoundingClientRect();
    return {
      tag: element.localName || element.nodeName,
      text: (element.innerText || element.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 1000),
      role: element.getAttribute('role') || '',
      selector: selectorFor(element).slice(0, 1000),
      rect: {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
      },
    };
  }

  function removePicker() {
    if (!picker) return;
    document.removeEventListener('pointermove', picker.move, true);
    document.removeEventListener('click', picker.click, true);
    picker.host.remove();
    picker = null;
  }

  function openForm(element) {
    if (!picker) return;
    picker.selected = metadata(element);
    picker.outline.style.display = 'block';
    const rect = element.getBoundingClientRect();
    picker.outline.style.left = `${rect.left}px`;
    picker.outline.style.top = `${rect.top}px`;
    picker.outline.style.width = `${rect.width}px`;
    picker.outline.style.height = `${rect.height}px`;
    picker.help.textContent = `${picker.selected.tag} — ${picker.selected.text || 'no text'}`;
    picker.form.style.display = 'flex';
    picker.input.focus();
  }

  function installPicker(initialComment) {
    removePicker();
    const host = document.createElement('div');
    host.id = 'cockpit-space-picker';
    host.style.cssText = 'all:initial;position:fixed;inset:0;z-index:2147483647;pointer-events:none;';
    const root = host.attachShadow({ mode: 'closed' });
    const style = document.createElement('style');
    style.textContent = `
      .outline { display:none; position:fixed; border:2px solid #246fca; background:#246fca22; pointer-events:none; }
      .panel { position:fixed; right:18px; bottom:18px; width:310px; padding:13px; border:1px solid #a9b9c9; border-radius:8px; background:#fff; box-shadow:0 8px 30px #12263b44; color:#172433; font:14px/1.4 system-ui,sans-serif; pointer-events:auto; }
      .title { margin:0 0 5px; font-weight:700; } .help { margin:0 0 9px; color:#506070; font-size:12px; }
      textarea { width:100%; box-sizing:border-box; resize:vertical; border:1px solid #aebdca; border-radius:4px; padding:7px; font:inherit; }
      .buttons { display:flex; gap:7px; margin-top:9px; } button { border:0; border-radius:4px; padding:7px 10px; color:#fff; background:#246fca; font:600 13px system-ui,sans-serif; cursor:pointer; } button.cancel { color:#31506d; background:#e8eef4; } button:disabled { opacity:.55; cursor:wait; }
      .error { min-height:18px; margin:7px 0 0; color:#a32929; font-size:12px; }
    `;
    root.append(style);
    const outline = document.createElement('div');
    outline.className = 'outline';
    const panel = document.createElement('section');
    panel.className = 'panel';
    panel.style.display = 'none';
    const title = document.createElement('p');
    title.className = 'title';
    title.textContent = 'Comment on this element';
    const help = document.createElement('p');
    help.className = 'help';
    help.textContent = 'Move over the page, then click an element.';
    const input = document.createElement('textarea');
    input.rows = 3;
    input.maxLength = 4000;
    input.placeholder = 'What should change?';
    input.value = String(initialComment || '').slice(0, 4000);
    const buttons = document.createElement('div');
    buttons.className = 'buttons';
    const save = document.createElement('button');
    save.textContent = 'Save feedback';
    const cancel = document.createElement('button');
    cancel.className = 'cancel';
    cancel.textContent = 'Cancel';
    const error = document.createElement('p');
    error.className = 'error';
    error.setAttribute('role', 'alert');
    buttons.append(save, cancel);
    panel.append(title, help, input, buttons, error);
    root.append(outline, panel);
    document.documentElement.append(host);
    const state = {
      host, root, outline, panel, form: panel, input, help, save, cancel, error, selected: null,
      move(event) {
        if (host.contains(event.target)) return;
        const element = document.elementFromPoint(event.clientX, event.clientY);
        if (!element || element === host) return;
        const rect = element.getBoundingClientRect();
        outline.style.display = 'block';
        outline.style.left = `${rect.left}px`;
        outline.style.top = `${rect.top}px`;
        outline.style.width = `${rect.width}px`;
        outline.style.height = `${rect.height}px`;
      },
      click(event) {
        if (host.contains(event.target)) return;
        const element = document.elementFromPoint(event.clientX, event.clientY);
        if (!element || element === host) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        openForm(element);
      },
    };
    picker = state;
    document.addEventListener('pointermove', state.move, true);
    document.addEventListener('click', state.click, true);
    cancel.addEventListener('click', removePicker);
    save.addEventListener('click', async () => {
      if (!state.selected) {
        error.textContent = 'Select an element first.';
        return;
      }
      save.disabled = true;
      cancel.disabled = true;
      error.textContent = '';
      try {
        await send({ type: 'COCKPIT_ELEMENT_ANNOTATION', comment: input.value, element: state.selected, viewport: viewport() });
        error.className = 'error saved';
        error.textContent = 'Saved to this Space.';
        setTimeout(removePicker, 700);
      } catch (saveError) {
        // Keep the form and its comment so a transient server error cannot lose a draft.
        error.className = 'error';
        error.textContent = saveError.message;
        save.disabled = false;
        cancel.disabled = false;
        input.focus();
      }
    });
  }

  runtime.onMessage.addListener((message, sender, sendResponse) => {
    if (message?.type === 'COCKPIT_PING') {
      sendResponse({ ok: true });
      return false;
    }
    if (message?.type === 'COCKPIT_GET_VIEWPORT') {
      sendResponse({ ok: true, viewport: viewport() });
      return false;
    }
    if (message?.type === 'COCKPIT_START_PICKER') {
      installPicker(message.initialComment);
      sendResponse({ ok: true });
      return false;
    }
    return false;
  });
})();
