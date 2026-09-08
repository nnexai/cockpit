(() => {
  if (globalThis.__cockpitFeedbackLoaded) return;
  globalThis.__cockpitFeedbackLoaded = true;
  const runtime = chrome.runtime;
  const MAX_POINTS = 8192; const MAX_ANNOTATIONS = 64;
  const state = { documentId: null, tabId: null, mode: 'browse', tool: 'freehand', annotations: [], selected: null, editing: false, alignmentDirty: false, reviewed: false, drawing: null, capture: null };
  const colors = ['#d62828', '#1769aa', '#2a9d55', '#c27803', '#7c3aed'];
  const uid = () => (crypto.randomUUID ? crypto.randomUUID() : `${Date.now()}-${Math.random().toString(16).slice(2)}`);
  const bounded = (value, limit) => String(value || '').replace(/[\u0000-\u001f\u007f]/g, ' ').trim().slice(0, limit);
  const send = (message) => new Promise((resolve, reject) => runtime.sendMessage({ ...message, document_id: state.documentId }, (result) => {
    const error = runtime.lastError;
    if (error) reject(new Error(error.message)); else if (result?.error) reject(new Error(result.error)); else resolve(result);
  }));
  const viewport = () => ({ width: Math.max(1, innerWidth), height: Math.max(1, innerHeight), scroll_x: scrollX, scroll_y: scrollY, device_pixel_ratio: devicePixelRatio || 1, visual_scale: visualViewport?.scale || 1 });
  const point = (event) => ({ x: event.pageX, y: event.pageY });
  const cssEscape = (value) => globalThis.CSS?.escape ? CSS.escape(value) : String(value).replace(/[^a-zA-Z0-9_-]/g, (character) => `\\${character}`);
  function locatorFor(element) {
    const result = [];
    if (element.id) result.push(`#${cssEscape(element.id)}`);
    const testId = element.getAttribute('data-testid'); if (testId) result.push(`[data-testid="${cssEscape(testId)}"]`);
    let current = element;
    while (current && current.nodeType === Node.ELEMENT_NODE && result.length < 6) {
      const tag = current.localName;
      if (!tag || tag === 'html' || tag === 'body') break;
      const parent = current.parentElement;
      const index = parent ? [...parent.children].filter((child) => child.localName === tag).indexOf(current) + 1 : 1;
      result.push(`${tag}:nth-of-type(${index})`); current = parent;
    }
    return [...new Set(result)].slice(0, 8);
  }
  function elementEvidence(element) {
    const rect = element.getBoundingClientRect();
    const role = bounded(element.getAttribute('role'), 80) || null;
    const name = bounded(element.getAttribute('aria-label') || element.getAttribute('title'), 160) || null;
    return { tag: bounded(element.localName, 40), text: bounded(element.textContent, 600), role, name, locators: locatorFor(element), excerpt: bounded(element.textContent, 240), rect: { x: rect.left + scrollX, y: rect.top + scrollY, width: rect.width, height: rect.height } };
  }
  const host = document.createElement('cockpit-feedback-overlay');
  host.style.cssText = 'all:initial;position:fixed;inset:0;z-index:2147483647;pointer-events:none;display:block;';
  const shadow = host.attachShadow({ mode: 'closed' });
  shadow.innerHTML = `<style>
    :host{all:initial}[hidden]{display:none!important}.controls,.editor,.add-text{position:fixed;pointer-events:auto;background:#fff;color:#17212b;border:1px solid #9aa9b7;border-radius:8px;box-shadow:0 2px 14px #0003;font:13px system-ui,sans-serif}.controls{top:12px;right:12px;display:flex;flex-wrap:wrap;gap:6px;align-items:center;max-width:min(440px,calc(100vw - 40px));padding:8px}.controls button,.controls select,.editor button,.add-text{font:inherit;border:1px solid #91a0ae;border-radius:4px;padding:6px 9px;background:#fff;color:inherit;cursor:pointer}.controls button.active,.controls .capture-action{background:#245fa8;color:#fff}.marks{position:fixed;inset:0;pointer-events:none;overflow:visible}.mark{fill:none;stroke:#d62828;stroke-width:5;stroke-linecap:round;stroke-linejoin:round;vector-effect:non-scaling-stroke}.mark.region{fill:#d6282815;stroke-width:3}.comment{position:fixed;max-width:min(260px,calc(100vw - 24px));padding:6px 8px;border-radius:4px;background:#fff;border:1px solid #d62828;color:#18212b;box-shadow:0 1px 4px #0003;white-space:pre-wrap;overflow-wrap:anywhere;font:13px/1.4 system-ui,sans-serif;cursor:pointer}.comment.selected{outline:2px solid #1769aa}.editor{width:min(260px,calc(100vw - 40px));padding:8px;z-index:2}.editor textarea{display:block;box-sizing:border-box;width:100%;min-height:70px;margin-bottom:6px;resize:vertical;border:0;background:#fff;color:#17212b;font:14px/1.4 system-ui,sans-serif;outline:none}.editor footer{display:flex;justify-content:space-between;gap:8px}.hint{width:100%;color:#526170;font-size:12px}.review{background:#fff2c2!important;border-color:#c27803!important}</style>
    <svg class="marks" aria-hidden="true"></svg><div class="controls" aria-label="Cockpit annotation tools"><select class="mode" aria-label="Page mode"><option value="browse">Browse</option><option value="annotate">Annotate</option></select><button data-tool="freehand" type="button">Freehand</button><button data-tool="element" type="button">Element</button><button data-tool="region" type="button">Region</button><button class="remove" type="button" disabled>Remove mark</button><button class="capture-action" type="button">Capture and save</button><button class="review" type="button" hidden>Review positions</button><span class="hint" role="status"></span></div><button class="add-text" type="button" hidden>+ Text</button><div class="editor" hidden><textarea maxlength="4000" aria-label="Annotation comment" placeholder="Add an optional comment…"></textarea><footer><span>Shift+Enter for a new line</span><button class="done" type="button">Done</button></footer></div>`;
  document.documentElement.append(host);
  const controls = shadow.querySelector('.controls'); const modeSelect = shadow.querySelector('.mode'); const textarea = shadow.querySelector('textarea'); const hint = shadow.querySelector('.hint'); const marks = shadow.querySelector('.marks'); const reviewButton = shadow.querySelector('.review');
  const editor = shadow.querySelector('.editor');
  const addText = shadow.querySelector('.add-text');
  const captureButton = shadow.querySelector('.capture-action');
  const removeButton = shadow.querySelector('.remove');
  const surface = document.createElement('div');
  surface.style.cssText = 'position:fixed;inset:0;pointer-events:none;touch-action:none';
  shadow.prepend(surface);
  function setHint(text) { hint.textContent = text; }
  function setMode(mode) { state.mode = mode === 'annotate' ? 'annotate' : 'browse'; surface.style.pointerEvents = state.mode === 'annotate' ? 'auto' : 'none'; modeSelect.value = state.mode; controls.hidden = false; for (const button of shadow.querySelectorAll('[data-tool]')) button.classList.toggle('active', button.dataset.tool === state.tool && state.mode === 'annotate'); setHint(state.mode === 'annotate' ? 'Draw, pick an element, or drag a region.' : 'Browse mode: page interactions pass through.'); render(); }
  function markBounds(annotation) {
    if (annotation.bounds) return annotation.bounds;
    const first = annotation.points[0] || { x: scrollX, y: scrollY };
    let left = first.x, top = first.y, right = first.x, bottom = first.y;
    for (const point of annotation.points) { left = Math.min(left, point.x); top = Math.min(top, point.y); right = Math.max(right, point.x); bottom = Math.max(bottom, point.y); }
    return { x: left, y: top, width: right - left, height: bottom - top };
  }
  function positionBeside(element, bounds, vp) {
    element.style.left = `${Math.max(8, Math.min(bounds.x - vp.scroll_x, vp.width - element.offsetWidth - 12))}px`;
    element.style.top = `${Math.max(8, Math.min(bounds.y + bounds.height - vp.scroll_y + 8, vp.height - element.offsetHeight - 12))}px`;
  }
  function render() {
    shadow.querySelectorAll('.comment').forEach((item) => item.remove());
    marks.replaceChildren(); const vp = viewport(); marks.setAttribute('viewBox', `0 0 ${vp.width} ${vp.height}`); marks.setAttribute('width', vp.width); marks.setAttribute('height', vp.height);
    for (const annotation of state.annotations) {
      const points = annotation.points.map((p) => `${p.x - vp.scroll_x},${p.y - vp.scroll_y}`).join(' ');
      if (annotation.kind === 'freehand' && annotation.points.length > 1) { const line = document.createElementNS('http://www.w3.org/2000/svg', 'polyline'); line.setAttribute('points', points); line.setAttribute('class', 'mark'); line.style.stroke = annotation.color; marks.append(line); }
      if ((annotation.kind === 'region' || annotation.kind === 'element') && annotation.bounds) { const rect = document.createElementNS('http://www.w3.org/2000/svg', 'rect'); rect.setAttribute('x', annotation.bounds.x - vp.scroll_x); rect.setAttribute('y', annotation.bounds.y - vp.scroll_y); rect.setAttribute('width', annotation.bounds.width); rect.setAttribute('height', annotation.bounds.height); rect.setAttribute('class', 'mark region'); rect.style.stroke = annotation.color; marks.append(rect); }
      const shape = marks.lastElementChild;
      if (shape) {
        shape.style.pointerEvents = state.mode === 'annotate' ? 'stroke' : 'none';
        shape.addEventListener('pointerdown', event => { event.stopPropagation(); event.preventDefault(); select(annotation.id); });
      }
      if (annotation.comment) {
        const label = document.createElement('div'); label.className = `comment${annotation.id === state.selected ? ' selected' : ''}`; label.textContent = annotation.comment; label.style.borderColor = annotation.color;
        label.addEventListener('click', event => { event.stopPropagation(); select(annotation.id, true); });
        label.style.pointerEvents = state.mode === 'annotate' ? 'auto' : 'none';
        shadow.append(label); positionBeside(label, markBounds(annotation), vp);
      }
    }
    if (state.drawing) {
      const preview = document.createElementNS('http://www.w3.org/2000/svg', 'polyline');
      const drawing = state.drawing;
      const end = drawing.end || drawing.start;
      const points = drawing.points || [drawing.start, { x: end.x, y: drawing.start.y }, end, { x: drawing.start.x, y: end.y }, drawing.start];
      preview.setAttribute('class', state.tool === 'region' ? 'mark region' : 'mark');
      preview.setAttribute('points', points.map(p => `${p.x - vp.scroll_x},${p.y - vp.scroll_y}`).join(' '));
      preview.style.stroke = colors[state.annotations.length % colors.length];
      marks.append(preview);
    }
    const selected = state.annotations.find(annotation => annotation.id === state.selected);
    const editable = selected && state.mode === 'annotate' && !state.capture && !state.drawing;
    editor.hidden = !editable || !state.editing;
    addText.hidden = !editable || state.editing || Boolean(selected?.comment);
    removeButton.disabled = !selected;
    if (editable) positionBeside(state.editing ? editor : addText, markBounds(selected), vp);
    reviewButton.hidden = !state.alignmentDirty; reviewButton.classList.toggle('review', state.alignmentDirty);
  }
  function select(id, edit = false) { state.selected = id; state.editing = edit; textarea.value = state.annotations.find(item => item.id === id)?.comment || ''; render(); if (edit) textarea.focus(); }
  function persist() { send({ type: 'draft', draft: { tab_id: state.tabId, document_id: state.documentId, url: location.href, title: bounded(document.title, 300), annotations: state.annotations, viewport: viewport(), alignment_dirty: state.alignmentDirty, updated_at: new Date().toISOString() } }).catch(error => setHint(`Draft not saved: ${error.message}`)); }
  function add(annotation) { if (state.annotations.length >= MAX_ANNOTATIONS) { setHint('The 64 annotation limit has been reached.'); return; } state.annotations.push(annotation); select(annotation.id, annotation.kind !== 'freehand'); persist(); }
  function finishFreehand() { const drawing = state.drawing; state.drawing = null; if (!drawing || drawing.points.length < 2) { render(); return; } add({ id: uid(), kind: 'freehand', comment: '', color: colors[state.annotations.length % colors.length], points: drawing.points, bounds: null, element: null }); }
  function finishRegion(end) { const drawing = state.drawing; state.drawing = null; if (!drawing) return; const x = Math.min(drawing.start.x, end.x), y = Math.min(drawing.start.y, end.y); const width = Math.abs(end.x - drawing.start.x), height = Math.abs(end.y - drawing.start.y); if (width < 2 || height < 2) { render(); return; } add({ id: uid(), kind: 'region', comment: '', color: colors[state.annotations.length % colors.length], points: [{ x, y }, { x: x + width, y: y + height }], bounds: { x, y, width, height }, element: null }); }
  function pick(event) {
    surface.style.pointerEvents = 'none';
    const element = document.elementFromPoint(event.clientX, event.clientY);
    surface.style.pointerEvents = 'auto';
    if (!element || host.contains(element)) return;
    const evidence = elementEvidence(element);
    add({ id: uid(), kind: 'element', comment: '', color: colors[state.annotations.length % colors.length], points: [{ x: evidence.rect.x + evidence.rect.width / 2, y: evidence.rect.y + evidence.rect.height / 2 }], bounds: evidence.rect, element: { tag: evidence.tag, text: evidence.text, role: evidence.role, name: evidence.name, locators: evidence.locators, excerpt: evidence.excerpt } });
  }
  surface.addEventListener('pointerdown', (event) => {
    if (state.mode !== 'annotate') return;
    event.preventDefault();
    state.editing = false;
    surface.setPointerCapture(event.pointerId);
    if (state.tool === 'element') { pick(event); return; }
    state.drawing = state.tool === 'freehand' ? { points: [point(event)] } : { start: point(event) };
  });
  surface.addEventListener('pointermove', (event) => { if (!state.drawing || state.mode !== 'annotate') return; if (state.tool === 'freehand' && state.drawing.points.length < MAX_POINTS) state.drawing.points.push(point(event)); else if (state.tool === 'region') state.drawing.end = point(event); render(); });
  surface.addEventListener('pointerup', (event) => { if (!state.drawing) return; if (state.tool === 'freehand') finishFreehand(); else finishRegion(point(event)); });
  surface.addEventListener('pointercancel', () => { state.drawing = null; render(); });
  modeSelect.addEventListener('change', () => { setMode(modeSelect.value); persist(); });
  for (const button of shadow.querySelectorAll('[data-tool]')) button.addEventListener('click', () => { state.tool = button.dataset.tool; setMode('annotate'); });
  shadow.querySelector('.remove').addEventListener('click', () => { if (!state.selected) return; state.annotations = state.annotations.filter((item) => item.id !== state.selected); state.selected = null; textarea.value = ''; render(); persist(); });
  textarea.addEventListener('input', () => { const selected = state.annotations.find((item) => item.id === state.selected); if (selected) { selected.comment = bounded(textarea.value, 4000); render(); persist(); } });
  addText.addEventListener('click', () => select(state.selected, true));
  function finishEditing() { state.editing = false; render(); persist(); }
  shadow.querySelector('.done').addEventListener('click', finishEditing);
  editor.addEventListener('keydown', event => {
    event.stopPropagation();
    if (!event.isComposing && (event.key === 'Escape' || (event.key === 'Enter' && !event.shiftKey))) { event.preventDefault(); finishEditing(); }
  });
  captureButton.addEventListener('click', async () => {
    captureButton.disabled = true; setHint('Capturing and saving…');
    try { const result = await send({ type: 'capture-page' }); setHint(`Saved ${result.annotation_ids.length} annotation${result.annotation_ids.length === 1 ? '' : 's'}. Ready in Cockpit feedback.`); }
    catch (error) { setHint(error.message); }
    finally { captureButton.disabled = false; }
  });
  reviewButton.addEventListener('click', () => { state.alignmentDirty = false; state.reviewed = true; setHint('Positions reviewed. Capture when ready.'); render(); persist(); });
  function markDirty() { if (state.capture) state.capture.invalidated = true; if (!state.annotations.length) return; state.alignmentDirty = true; state.reviewed = false; setHint('The page moved. Review positions before capture.'); render(); persist(); }
  addEventListener('resize', markDirty, true); addEventListener('scroll', markDirty, true); addEventListener('zoom', markDirty, true);
  const observer = new MutationObserver((mutations) => { if (mutations.some((mutation) => !host.contains(mutation.target))) markDirty(); }); observer.observe(document.documentElement, { subtree: true, childList: true, attributes: true, characterData: true });
  function exportAnnotations(imageWidth, imageHeight) { const capture = state.capture; const sx = imageWidth / capture.viewport.width, sy = imageHeight / capture.viewport.height; const mapPoint = (p) => ({ x: (p.x - capture.viewport.scroll_x) * sx, y: (p.y - capture.viewport.scroll_y) * sy }); return capture.annotations.map((item) => ({ id: item.id, kind: item.kind, comment: bounded(item.comment, 4000), color: item.color, points: item.points.map(mapPoint), bounds: item.bounds ? { x: (item.bounds.x - capture.viewport.scroll_x) * sx, y: (item.bounds.y - capture.viewport.scroll_y) * sy, width: item.bounds.width * sx, height: item.bounds.height * sy } : null, element: item.element })); }
  async function onMessage(message) {
    if (!message || message.type === 'init') { const fresh = !state.documentId; if (message?.document_id) state.documentId = message.document_id; if (message?.tab_id != null) state.tabId = message.tab_id; if (fresh && message?.draft && message.draft.document_id === state.documentId) { state.annotations = message.draft.annotations || []; state.alignmentDirty = Boolean(message.draft.alignment_dirty); } setMode(fresh ? (message?.mode || 'browse') : state.mode); return { document_id: state.documentId }; }
    if (message.document_id !== state.documentId) throw new Error('The original page document is no longer active');
    if (message.type === 'capture-saved') { const ids = new Set(message.ids); state.annotations = state.annotations.filter(annotation => !ids.has(annotation.id)); state.selected = null; textarea.value = ''; render(); persist(); return { saved: true }; }
    if (message.type === 'mode') { setMode(message.mode); return { document_id: state.documentId }; }
    if (message.type === 'prepare-capture') {
      if (!state.annotations.length) throw new Error('Add a mark or comment before capturing');
      if (state.alignmentDirty && !state.reviewed) throw new Error('Review changed positions before capture');
      const vp = viewport();
      const visible = p => p.x >= vp.scroll_x && p.y >= vp.scroll_y && p.x <= vp.scroll_x + vp.width && p.y <= vp.scroll_y + vp.height;
      if (state.annotations.some(annotation => annotation.points.some(p => !visible(p)) || (annotation.bounds && (!visible(annotation.bounds) || !visible({ x: annotation.bounds.x + annotation.bounds.width, y: annotation.bounds.y + annotation.bounds.height }))))) throw new Error('Some marks are outside the viewport. Scroll back or remove them before capturing');
      state.capture = { annotations: structuredClone(state.annotations), viewport: vp, url: location.href, title: bounded(document.title, 300), document_id: state.documentId, captured_at: new Date().toISOString(), invalidated: false };
      controls.hidden = true;
      editor.hidden = true; addText.hidden = true;
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return { document_id: state.documentId, annotation_count: state.capture.annotations.length };
    }
    if (message.type === 'finish-capture') {
      if (!state.capture || state.capture.invalidated) throw new Error('The page moved during capture. Review positions and capture again');
      const result = { page: { ...message.page, url: state.capture.url, title: state.capture.title, document_id: state.capture.document_id, captured_at: state.capture.captured_at, viewport: state.capture.viewport }, annotations: exportAnnotations(message.image_width, message.image_height) };
      state.capture = null; controls.hidden = false; render(); persist(); return result;
    }
    if (message.type === 'cancel-capture') { state.capture = null; controls.hidden = false; render(); return { document_id: state.documentId }; }
    return { document_id: state.documentId };
  }
  runtime.onMessage.addListener((message, sender, sendResponse) => { Promise.resolve().then(() => onMessage(message, sender)).then(sendResponse).catch((error) => sendResponse({ error: bounded(error.message, 400) })); return true; });
  setMode('browse'); render();
})();
