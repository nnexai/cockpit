(() => {
  if (globalThis.__cockpitFeedbackLoaded) return;
  globalThis.__cockpitFeedbackLoaded = true;
  const runtime = chrome.runtime;
  const MAX_POINTS = 8192; const MAX_ANNOTATIONS = 64; const MAX_ANCHORS_PER_ANNOTATION = 4; const GEOMETRY_EPSILON = 1; const FREEHAND_TOLERANCE = 1.5;
  const state = { documentId: null, tabId: null, mode: 'browse', tool: 'freehand', annotations: [], selected: null, editing: false, composing: false, alignmentDirty: false, reviewed: false, alignmentReason: null, captureAnyway: false, drawing: null, capture: null };
  const anchorRefs = new Map();
  let capturedPointerId = null;
  let geometryBaseline = null;
  const colors = ['#d62828', '#1769aa', '#2a9d55', '#c27803', '#7c3aed'];
  const uid = () => (crypto.randomUUID ? crypto.randomUUID() : `${Date.now()}-${Math.random().toString(16).slice(2)}`);
  const bounded = (value, limit) => String(value || '').replace(/[\u0000-\u001f\u007f]/g, ' ').trim().slice(0, limit);
  const send = (message) => new Promise((resolve, reject) => runtime.sendMessage({ ...message, document_id: state.documentId }, (result) => {
    const error = runtime.lastError;
    if (error) reject(new Error(error.message)); else if (result?.error) reject(new Error(result.error)); else resolve(result);
  }));
  const viewport = () => ({ width: Math.max(1, innerWidth), height: Math.max(1, innerHeight), scroll_x: scrollX, scroll_y: scrollY, device_pixel_ratio: devicePixelRatio || 1, visual_scale: visualViewport?.scale || 1 });
  const point = (event) => ({ x: event.pageX, y: event.pageY });
  function simplifyFreehand(points, tolerance = FREEHAND_TOLERANCE) {
    if (!Array.isArray(points) || points.length <= 2) return Array.isArray(points) ? points.slice() : [];
    const keep = new Uint8Array(points.length); keep[0] = 1; keep[points.length - 1] = 1;
    const squaredTolerance = tolerance * tolerance;
    const ranges = [[0, points.length - 1]];
    while (ranges.length) {
      const [start, end] = ranges.pop();
      const first = points[start]; const last = points[end];
      const dx = last.x - first.x; const dy = last.y - first.y; const lengthSquared = dx * dx + dy * dy;
      let index = -1; let maximum = squaredTolerance;
      for (let current = start + 1; current < end; current += 1) {
        const candidate = points[current];
        let distanceSquared;
        if (lengthSquared === 0) {
          const offsetX = candidate.x - first.x; const offsetY = candidate.y - first.y;
          distanceSquared = offsetX * offsetX + offsetY * offsetY;
        } else {
          const projection = Math.max(0, Math.min(1, ((candidate.x - first.x) * dx + (candidate.y - first.y) * dy) / lengthSquared)); const projectedX = first.x + projection * dx; const projectedY = first.y + projection * dy;
          const offsetX = candidate.x - projectedX; const offsetY = candidate.y - projectedY; distanceSquared = offsetX * offsetX + offsetY * offsetY;
        }
        if (distanceSquared > maximum) { maximum = distanceSquared; index = current; }
      }
      if (index !== -1) { keep[index] = 1; ranges.push([start, index], [index, end]); }
    }
    return points.filter((_, index) => keep[index]);
  }
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
    const token = element.getAttribute('data-cockpit-anchor') || uid();
    if (!element.hasAttribute('data-cockpit-anchor')) element.setAttribute('data-cockpit-anchor', token);
    return { document_id: state.documentId, frame_id: 0, dom_token: token, tag: bounded(element.localName, 40), text: bounded(element.textContent, 600), role, name, locators: locatorFor(element), excerpt: bounded(element.textContent, 240), rect: { x: rect.left + scrollX, y: rect.top + scrollY, width: rect.width, height: rect.height } };
  }
  const host = document.createElement('cockpit-feedback-overlay');
  host.setAttribute('popover', 'manual');
  host.style.cssText = 'all:initial;position:fixed!important;inset:0!important;z-index:2147483647!important;pointer-events:none!important;display:block!important;isolation:isolate!important;';
  const shadow = host.attachShadow({ mode: 'closed' });
  shadow.innerHTML = `<style>
    :host{all:initial!important;position:fixed!important;inset:0!important;z-index:2147483647!important;display:block!important;pointer-events:none!important;isolation:isolate!important}[hidden]{display:none!important}.controls,.editor,.add-text,.hover-outline{position:fixed;z-index:2147483647;pointer-events:auto;background:#fff;color:#17212b;border:1px solid #91a0ae;border-radius:6px;box-shadow:0 4px 16px #0004;font:12px/1 system-ui,sans-serif}.controls{top:8px;right:8px;display:flex;flex-wrap:nowrap;gap:3px;align-items:center;max-width:calc(100vw - 16px);padding:4px;overflow-x:auto;scrollbar-width:none}.controls::-webkit-scrollbar{display:none}.controls button,.editor button,.add-text{box-sizing:border-box;font:inherit;border:1px solid #91a0ae;border-radius:4px;background:#fff;color:inherit;cursor:pointer}.controls button{height:28px}.controls button{display:inline-flex;flex:0 0 28px;align-items:center;justify-content:center;padding:0}.controls .icon{width:16px;height:16px;display:block;fill:none;stroke:currentColor;stroke-width:1.5;stroke-linecap:round;stroke-linejoin:round}.controls button:hover:not(:disabled){background:#edf2f8}.controls button:focus-visible{outline:2px solid #1769aa;outline-offset:1px}.controls button:disabled{opacity:.45;cursor:default}.controls button.active,.controls .capture-action{background:#245fa8;border-color:#245fa8;color:#fff}.controls .review{background:#fff2c2;border-color:#c27803;color:#17212b}.controls .close{margin-left:1px}.hint{position:absolute;top:calc(100% + 4px);right:0;max-width:min(360px,calc(100vw - 16px));padding:5px 7px;border:1px solid #91a0ae;border-radius:4px;background:#fff;color:#526170;box-shadow:0 2px 8px #0003;font-size:11px;line-height:1.25;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.marks{position:fixed;inset:0;pointer-events:none;overflow:visible}.mark{fill:none;stroke:#d62828;stroke-width:5;stroke-linecap:round;stroke-linejoin:round;vector-effect:non-scaling-stroke}.mark.region{fill:#d6282815;stroke-width:3}.comment{position:fixed;max-width:min(260px,calc(100vw - 24px));padding:6px 8px;border-radius:4px;background:#fff;border:1px solid #d62828;color:#18212b;box-shadow:0 1px 4px #0003;white-space:pre-wrap;overflow-wrap:anywhere;font:13px/1.4 system-ui,sans-serif;cursor:pointer}.comment.selected{outline:2px solid #1769aa}.editor{width:min(260px,calc(100vw - 40px));padding:8px;z-index:2}.editor textarea{display:block;box-sizing:border-box;width:100%;min-height:70px;margin-bottom:6px;resize:vertical;border:0;background:#fff;color:#17212b;font:14px/1.4 system-ui,sans-serif;outline:none}.editor footer{display:flex;justify-content:space-between;gap:8px}.hover-outline{display:none;border:2px solid #1769aa;border-radius:3px;background:#1769aa12;box-shadow:0 0 0 1px #fff,0 2px 8px #1769aa55;pointer-events:none}.review{background:#fff2c2!important;border-color:#c27803!important}</style>
    <svg class="marks" aria-hidden="true"></svg><div class="hover-outline" aria-hidden="true"></div><div class="controls" aria-label="Cockpit annotation tools"><button data-tool="select" type="button" aria-label="Select tool" title="Select tool (V)"><svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="m4 2 7 6-3 .4 2 4-1.6.8-2-4-2 2z"/></svg></button><button data-tool="freehand" type="button" aria-label="Freehand tool" title="Freehand tool (F)"><svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M3 12c1.5-4 3-7 5-7 1.4 0 1.4 2 2.5 2 1 0 1.5-1.5 2.5-3"/></svg></button><button data-tool="element" type="button" aria-label="Element tool" title="Element tool (E)"><svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="4"/><path d="M8 1v3M8 12v3M1 8h3M12 8h3"/></svg></button><button data-tool="region" type="button" aria-label="Region tool" title="Region tool (R)"><svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><rect x="2.5" y="2.5" width="11" height="11" rx="1" stroke-dasharray="2 1.5"/></svg></button><button class="remove" type="button" aria-label="Remove selected mark" title="Remove selected mark (Delete)" disabled><svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5h10M6 2.5h4M5 4.5l.6 9h4.8l.6-9M7 7v4M9 7v4"/></svg></button><button class="capture-action" type="button" aria-label="Capture and save" title="Capture and save"><svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 5h2l1-2h5l1 2h2v8h-11z"/><circle cx="8" cy="9" r="2.5"/></svg></button><button class="capture-anyway" type="button" hidden aria-label="Capture current view anyway" title="Capture current view as shown"><svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 5h2l1-2h5l1 2h2v8h-11z"/><path d="M8 6.5v3M8 11.5v.1"/></svg></button><button class="review" type="button" hidden aria-label="Review positions" title="Review positions"><svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="6"/><path d="m5 8 2 2 4-4"/></svg></button><span class="hint" role="status" hidden></span><button class="close" type="button" aria-label="Hide annotation tools" title="Hide annotation tools (Escape)"><svg class="icon" viewBox="0 0 16 16" aria-hidden="true"><path d="m4 4 8 8M12 4l-8 8"/></svg></button></div><button class="add-text" type="button" hidden>+ Text</button><div class="editor" hidden><textarea maxlength="4000" aria-label="Annotation comment" placeholder="Add an optional comment…"></textarea><footer><span>Shift+Enter for a new line</span><button class="done" type="button">Done</button></footer></div>`;
  document.documentElement.append(host);
  if (typeof host.showPopover === 'function') { try { host.showPopover(); } catch {} }
  const controls = shadow.querySelector('.controls'); const textarea = shadow.querySelector('textarea'); const hint = shadow.querySelector('.hint'); const marks = shadow.querySelector('.marks'); const reviewButton = shadow.querySelector('.review'); const hoverOutline = shadow.querySelector('.hover-outline');
  controls.setAttribute('role', 'toolbar');
  hint.style.position = 'fixed'; hint.style.top = '40px'; hint.style.right = '8px';
  const editor = shadow.querySelector('.editor');
  const addText = shadow.querySelector('.add-text');
  const captureButton = shadow.querySelector('.capture-action');
  const captureAnywayButton = shadow.querySelector('.capture-anyway');
  const removeButton = shadow.querySelector('.remove');
  const surface = document.createElement('div');
  surface.style.cssText = 'position:fixed;inset:0;pointer-events:none;touch-action:none';
  shadow.prepend(surface);
  const closeButton = shadow.querySelector('.close');
  const observedAnchors = new Set();
  let hoveredElement = null;
  let lastPointer = null;
  let layoutObserver = null;
  function copyRect(rect) {
    if (!rect) return null;
    const values = [rect.x, rect.y, rect.width, rect.height];
    return values.every(Number.isFinite) ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height } : null;
  }
  function rectFor(element) {
    if (!element || element.nodeType !== 1 || !element.isConnected) return null;
    try {
      const rect = element.getBoundingClientRect();
      return copyRect({ x: rect.left + scrollX, y: rect.top + scrollY, width: rect.width, height: rect.height });
    } catch {
      return null;
    }
  }
  function closeEnough(left, right) {
    return Number.isFinite(left) && Number.isFinite(right) && Math.abs(left - right) <= GEOMETRY_EPSILON;
  }
  function sameRect(left, right) {
    return Boolean(left && right) && closeEnough(left.x, right.x) && closeEnough(left.y, right.y) && closeEnough(left.width, right.width) && closeEnough(left.height, right.height);
  }
  function sameViewport(left, right) {
    return Boolean(left && right) && ['width', 'height', 'scroll_x', 'scroll_y', 'device_pixel_ratio', 'visual_scale'].every((key) => closeEnough(left[key], right[key]));
  }
  function overlayNode(node) {
    if (!node) return false;
    if (node === host || host.contains(node)) return true;
    return typeof node.getRootNode === 'function' && node.getRootNode() === shadow;
  }
  function evidenceMatches(element, evidence) {
    if (!element || overlayNode(element) || !evidence) return false;
    if (evidence.document_id && evidence.document_id !== state.documentId) return false;
    if (evidence.frame_id !== undefined && evidence.frame_id !== 0) return false;
    if (typeof evidence.dom_token !== 'string' || !evidence.dom_token || element.getAttribute('data-cockpit-anchor') !== evidence.dom_token) return false;
    if (evidence.tag && element.localName !== evidence.tag) return false;
    if (evidence.role && (bounded(element.getAttribute('role'), 80) || null) !== evidence.role) return false;
    if (evidence.name && (bounded(element.getAttribute('aria-label') || element.getAttribute('title'), 160) || null) !== evidence.name) return false;
    return true;
  }
  function restoreElementAnchor(annotation) {
    const evidence = annotation?.element;
    const locators = Array.isArray(evidence?.locators) ? evidence.locators.slice(0, 8) : [];
    for (const locator of locators) {
      if (typeof locator !== 'string' || !locator.trim()) continue;
      let matches;
      try { matches = document.querySelectorAll(locator); } catch { continue; }
      if (matches.length === 1 && evidenceMatches(matches[0], evidence)) return matches[0];
    }
    return null;
  }
  function elementAtPagePoint(position) {
    if (!position || !Number.isFinite(position.x) || !Number.isFinite(position.y)) return null;
    const x = position.x - scrollX; const y = position.y - scrollY;
    if (x < 0 || y < 0 || x > innerWidth || y > innerHeight) return null;
    let element;
    try { element = document.elementsFromPoint(x, y).find((candidate) => !overlayNode(candidate)); } catch { return null; }
    return element && !overlayNode(element) ? element : null;
  }
  function anchorSamples(annotation) {
    if (annotation.kind === 'region' && annotation.bounds) {
      const bounds = annotation.bounds;
      const inset = Math.min(2, Math.max(0, Math.min(bounds.width, bounds.height) / 4));
      return [
        { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 },
        { x: bounds.x + inset, y: bounds.y + inset },
        { x: bounds.x + bounds.width - inset, y: bounds.y + inset },
        { x: bounds.x + bounds.width - inset, y: bounds.y + bounds.height - inset },
      ];
    }
    const points = Array.isArray(annotation.points) ? annotation.points : [];
    if (!points.length) return [];
    return [points[0], points[Math.floor(points.length / 2)], points[points.length - 1]];
  }
  function collectAnchors(annotation) {
    if (annotation.kind === 'element') {
      const element = restoreElementAnchor(annotation);
      return element ? [element] : [];
    }
    const elements = [];
    for (const sample of anchorSamples(annotation).slice(0, MAX_ANCHORS_PER_ANNOTATION)) {
      const element = elementAtPagePoint(sample);
      if (element && !elements.includes(element)) elements.push(element);
    }
    return elements;
  }
  function setAnchors(annotationId, elements) {
    const unique = [];
    for (const element of elements || []) if (element && !overlayNode(element) && !unique.includes(element)) unique.push(element);
    anchorRefs.set(annotationId, unique.slice(0, MAX_ANCHORS_PER_ANNOTATION));
    syncObservedAnchors();
  }
  function ensureAnchors(annotation) {
    if (!anchorRefs.has(annotation.id)) setAnchors(annotation.id, collectAnchors(annotation));
    return anchorRefs.get(annotation.id);
  }
  function syncObservedAnchors() {
    if (!layoutObserver) return;
    const current = new Set();
    for (const annotation of state.annotations) for (const element of ensureAnchors(annotation)) current.add(element);
    for (const element of observedAnchors) {
      if (!current.has(element)) { layoutObserver.unobserve(element); observedAnchors.delete(element); }
    }
    for (const element of current) {
      if (observedAnchors.has(element)) continue;
      layoutObserver.observe(element); observedAnchors.add(element);
    }
  }
  function geometryEntries(useStoredBounds = false) {
    const entries = [];
    for (const annotation of state.annotations) {
      const elements = ensureAnchors(annotation);
      if (!elements.length && annotation.kind === 'element') entries.push({ annotationId: annotation.id, element: null, rect: null });
      for (const element of elements) {
        const stored = useStoredBounds && annotation.kind === 'element' ? copyRect(annotation.bounds) : null;
        entries.push({ annotationId: annotation.id, element, rect: stored || rectFor(element) });
      }
    }
    syncObservedAnchors();
    return entries;
  }
  function makeGeometryBaseline(useStoredBounds = false, baselineViewport = null) {
    return { viewport: baselineViewport || viewport(), anchors: geometryEntries(useStoredBounds) };
  }
  function geometryStatus(baseline, currentViewport = viewport()) {
    if (!baseline) return { viewportChanged: false, layoutChanged: false, disconnected: false };
    let layoutChanged = false; let disconnected = false;
    for (const entry of baseline.anchors) {
      if (!entry.element || !entry.element.isConnected) { layoutChanged = true; disconnected = true; continue; }
      const rect = rectFor(entry.element);
      if (!rect || !sameRect(rect, entry.rect)) layoutChanged = true;
    }
    return { viewportChanged: !sameViewport(currentViewport, baseline.viewport), layoutChanged, disconnected };
  }
  function forgetAnnotation(annotationId) {
    anchorRefs.delete(annotationId);
    if (geometryBaseline) geometryBaseline.anchors = geometryBaseline.anchors.filter((entry) => entry.annotationId !== annotationId);
    syncObservedAnchors();
  }
  function setHint(text, visible = true) { hint.textContent = text; hint.hidden = !visible; }
  function setMode(mode, visible = mode === 'annotate') { const nextMode = mode === 'annotate' ? 'annotate' : 'browse'; if (state.mode === 'annotate' && nextMode === 'browse') { releaseDrawingPointer(); state.drawing = null; hoveredElement = null; } state.mode = nextMode; surface.style.pointerEvents = state.mode === 'annotate' ? 'auto' : 'none'; controls.hidden = !visible; for (const button of shadow.querySelectorAll('[data-tool]')) { const active = button.dataset.tool === state.tool && state.mode === 'annotate'; button.classList.toggle('active', active); button.setAttribute('aria-pressed', String(active)); } setHint(state.mode === 'annotate' ? 'Draw, pick an element, or drag a region. V Select, E Element, R Region, F Freehand.' : '', false); render(); }
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
        shape.style.pointerEvents = state.mode === 'annotate' && state.tool === 'select' ? 'stroke' : 'none';
        shape.addEventListener('pointerdown', event => { event.stopPropagation(); event.preventDefault(); select(annotation.id); });
      }
      if (annotation.comment) {
        const label = document.createElement('div'); label.className = `comment${annotation.id === state.selected ? ' selected' : ''}`; label.textContent = annotation.comment; label.style.borderColor = annotation.color;
        label.addEventListener('click', event => { event.stopPropagation(); select(annotation.id, true); });
        label.style.pointerEvents = state.mode === 'annotate' && state.tool === 'select' ? 'auto' : 'none';
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
    captureAnywayButton.hidden = !state.alignmentDirty || state.alignmentReason === 'disconnected' || Boolean(state.capture);
    captureAnywayButton.disabled = Boolean(state.capture);
    if (hoveredElement && state.tool === 'element' && state.mode === 'annotate' && hoveredElement.isConnected) {
      const rect = hoveredElement.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        hoverOutline.hidden = false; hoverOutline.style.display = 'block';
        hoverOutline.style.left = `${rect.left}px`; hoverOutline.style.top = `${rect.top}px`;
        hoverOutline.style.width = `${rect.width}px`; hoverOutline.style.height = `${rect.height}px`;
      } else { hoverOutline.hidden = true; hoverOutline.style.display = 'none'; }
    } else { hoverOutline.hidden = true; hoverOutline.style.display = 'none'; }
  }
  function select(id, edit = false) { state.selected = id; state.editing = edit; textarea.value = state.annotations.find(item => item.id === id)?.comment || ''; render(); if (edit) textarea.focus(); }
  function persist() { send({ type: 'draft', draft: { tab_id: state.tabId, document_id: state.documentId, url: location.href, title: bounded(document.title, 300), annotations: state.annotations, viewport: viewport(), alignment_dirty: state.alignmentDirty, updated_at: new Date().toISOString() } }).catch(error => setHint(`Draft not saved: ${error.message}`)); }
  function add(annotation, anchor = null) {
    if (state.annotations.length >= MAX_ANNOTATIONS) { setHint('The 64 annotation limit has been reached.'); return; }
    state.annotations.push(annotation);
    setAnchors(annotation.id, anchor ? [anchor] : collectAnchors(annotation));
    if (!geometryBaseline) geometryBaseline = makeGeometryBaseline(false);
    else {
      const elements = anchorRefs.get(annotation.id) || [];
      if (!elements.length && annotation.kind === 'element') geometryBaseline.anchors.push({ annotationId: annotation.id, element: null, rect: null });
      for (const element of elements) geometryBaseline.anchors.push({ annotationId: annotation.id, element, rect: rectFor(element) });
    }
    select(annotation.id, annotation.kind !== 'freehand'); persist();
  }
  function finishFreehand() { const drawing = state.drawing; releaseDrawingPointer(); state.drawing = null; if (!drawing || drawing.points.length < 2) { render(); return; } add({ id: uid(), document_id: state.documentId, frame_id: 0, kind: 'freehand', comment: '', color: colors[state.annotations.length % colors.length], points: simplifyFreehand(drawing.points), bounds: null, element: null }); }
  function finishRegion(end) { const drawing = state.drawing; releaseDrawingPointer(); state.drawing = null; if (!drawing) return; const x = Math.min(drawing.start.x, end.x), y = Math.min(drawing.start.y, end.y); const width = Math.abs(end.x - drawing.start.x), height = Math.abs(end.y - drawing.start.y); if (width < 2 || height < 2) { render(); return; } add({ id: uid(), document_id: state.documentId, frame_id: 0, kind: 'region', comment: '', color: colors[state.annotations.length % colors.length], points: [{ x, y }, { x: x + width, y: y + height }], bounds: { x, y, width, height }, element: null }); }
  function eligibleElement(element) {
    if (!element || element.nodeType !== Node.ELEMENT_NODE || overlayNode(element)) return false;
    if (['html', 'head', 'body', 'script', 'style', 'meta', 'link', 'noscript'].includes(element.localName)) return false;
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }
  function pageElementAt(clientX, clientY) {
    surface.style.pointerEvents = 'none';
    let element = null;
    try { element = document.elementsFromPoint(clientX, clientY).find(eligibleElement) || null; } catch {}
    surface.style.pointerEvents = state.mode === 'annotate' ? 'auto' : 'none';
    return element;
  }
  function refreshElementPreview(clientX = lastPointer?.x, clientY = lastPointer?.y) {
    if (state.mode !== 'annotate' || state.tool !== 'element' || !Number.isFinite(clientX) || !Number.isFinite(clientY)) {
      hoveredElement = null; render(); return null;
    }
    const next = pageElementAt(clientX, clientY);
    hoveredElement = next;
    render();
    return next;
  }
  function pick(event) {
    surface.style.pointerEvents = 'none';
    const element = hoveredElement && hoveredElement.isConnected && hoveredElement.getBoundingClientRect().left <= event.clientX && hoveredElement.getBoundingClientRect().right >= event.clientX && hoveredElement.getBoundingClientRect().top <= event.clientY && hoveredElement.getBoundingClientRect().bottom >= event.clientY
      ? hoveredElement
      : document.elementFromPoint(event.clientX, event.clientY);
    surface.style.pointerEvents = 'auto';
    if (!eligibleElement(element)) return;
    const evidence = elementEvidence(element);
    add({ id: uid(), document_id: state.documentId, frame_id: 0, kind: 'element', comment: '', color: colors[state.annotations.length % colors.length], points: [{ x: evidence.rect.x + evidence.rect.width / 2, y: evidence.rect.y + evidence.rect.height / 2 }], bounds: evidence.rect, element: evidence }, element);
  }
  surface.addEventListener('pointerdown', (event) => {
    if (state.mode !== 'annotate') return;
    lastPointer = { x: event.clientX, y: event.clientY };
    if (state.tool === 'select') { state.editing = false; render(); return; }
    event.preventDefault();
    state.editing = false;
    capturedPointerId = event.pointerId;
    surface.setPointerCapture(event.pointerId);
    if (state.tool === 'element') { pick(event); releaseDrawingPointer(); hoveredElement = null; render(); return; }
    state.drawing = state.tool === 'freehand' ? { points: [point(event)], pointerId: event.pointerId } : { start: point(event), pointerId: event.pointerId };
  });
  surface.addEventListener('pointermove', (event) => { lastPointer = { x: event.clientX, y: event.clientY }; if (state.mode !== 'annotate') return; if (state.tool === 'element' && !state.drawing) { refreshElementPreview(event.clientX, event.clientY); return; } if (!state.drawing) return; if (state.tool === 'freehand' && state.drawing.points.length < MAX_POINTS) state.drawing.points.push(point(event)); else if (state.tool === 'region') state.drawing.end = point(event); render(); });
  surface.addEventListener('pointerup', (event) => { if (!state.drawing) return; if (state.tool === 'freehand') finishFreehand(); else finishRegion(point(event)); });
  surface.addEventListener('pointercancel', () => { releaseDrawingPointer(); state.drawing = null; render(); });
  for (const button of shadow.querySelectorAll('[data-tool]')) button.addEventListener('click', () => { releaseDrawingPointer(); state.drawing = null; state.tool = button.dataset.tool; setMode('annotate'); });
  function removeAnnotation(id) {
    state.annotations = state.annotations.filter((item) => item.id !== id);
    forgetAnnotation(id);
    if (!state.annotations.length) {
      geometryBaseline = null; state.alignmentDirty = false; state.reviewed = false; state.alignmentReason = null; state.captureAnyway = false;
    }
  }
  shadow.querySelector('.remove').addEventListener('click', () => { if (!state.selected) return; removeAnnotation(state.selected); state.selected = null; textarea.value = ''; render(); persist(); });
  textarea.addEventListener('input', () => { const selected = state.annotations.find(item => item.id === state.selected); if (selected) { selected.comment = bounded(textarea.value, 4000); render(); persist(); } });
  addText.addEventListener('click', () => select(state.selected, true));
  function finishEditing() { state.editing = false; hoveredElement = null; render(); persist(); }
  function releaseDrawingPointer() { const pointerId = capturedPointerId ?? state.drawing?.pointerId; if (Number.isFinite(pointerId) && surface.hasPointerCapture(pointerId)) surface.releasePointerCapture(pointerId); capturedPointerId = null; }
  function closeControls() { releaseDrawingPointer(); state.drawing = null; state.editing = false; hoveredElement = null; setMode('browse', false); editor.hidden = true; addText.hidden = true; persist(); }
  shadow.querySelector('.done').addEventListener('click', finishEditing);
  editor.addEventListener('keydown', event => {
    event.stopPropagation();
    if (!event.isComposing && (event.key === 'Escape' || (event.key === 'Enter' && !event.shiftKey))) { event.preventDefault(); finishEditing(); }
  });
  function editableTarget(target, path = []) {
    return path.some((node) => node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement || node instanceof HTMLSelectElement || node?.isContentEditable)
      || target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || target?.isContentEditable;
  }
  function cancelCurrentInteraction() {
    if (state.drawing || capturedPointerId != null) { releaseDrawingPointer(); state.drawing = null; hoveredElement = null; render(); return true; }
    if (state.editing) { finishEditing(); return true; }
    if (hoveredElement) { hoveredElement = null; render(); return true; }
    return false;
  }
  addEventListener('compositionstart', () => { state.composing = true; }, true);
  addEventListener('compositionend', () => { state.composing = false; }, true);
  addEventListener('keydown', (event) => {
    const path = typeof event.composedPath === 'function' ? event.composedPath() : [];
    if (state.mode !== 'annotate' || controls.hidden || state.composing || event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Escape') {
      if (path.includes(host) && !state.editing) { event.preventDefault(); event.stopPropagation(); closeControls(); return; }
      event.preventDefault(); event.stopPropagation();
      if (!cancelCurrentInteraction()) closeControls();
      return;
    }
    if (path.includes(host) || event.target === host || editableTarget(event.target, path)) return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    const tool = { v: 'select', e: 'element', r: 'region', f: 'freehand' }[String(event.key || '').toLowerCase()];
    if (tool) { event.preventDefault(); event.stopPropagation(); releaseDrawingPointer(); state.drawing = null; state.tool = tool; setMode('annotate'); return; }
    if ((event.key === 'Delete' || event.key === 'Backspace') && state.selected) { event.preventDefault(); event.stopPropagation(); removeAnnotation(state.selected); state.selected = null; textarea.value = ''; render(); persist(); return; }
    if (event.key === 'Enter' && state.selected) { event.preventDefault(); event.stopPropagation(); select(state.selected, true); }
  }, true);
  async function startCapture(allowLayoutDrift = false) {
    if (state.capture || (allowLayoutDrift && (!state.alignmentDirty || state.alignmentReason === 'disconnected'))) return;
    state.captureAnyway = allowLayoutDrift;
    captureButton.disabled = true; captureAnywayButton.disabled = true;
    setHint(allowLayoutDrift ? 'Capturing this view as shown; layout movement is allowed for this capture, but viewport or navigation changes still cancel.' : 'Capturing and saving…');
    try { const result = await send({ type: 'capture-page' }); setHint(`Saved ${result.annotation_ids.length} annotation${result.annotation_ids.length === 1 ? '' : 's'}. Ready in Cockpit feedback.`); }
    catch (error) { setHint(error.message); }
    finally { state.captureAnyway = false; captureButton.disabled = false; captureAnywayButton.disabled = false; render(); }
  }
  captureButton.addEventListener('click', () => startCapture(false));
  captureAnywayButton.addEventListener('click', () => startCapture(true));
  closeButton.addEventListener('click', closeControls);
  reviewButton.addEventListener('click', () => {
    geometryBaseline = makeGeometryBaseline(false);
    const status = geometryStatus(geometryBaseline);
    if (status.disconnected) {
      state.alignmentDirty = true; state.reviewed = false; state.alignmentReason = 'disconnected';
      setHint('A marked element is no longer available. Remove it or add the mark again.');
    } else {
      state.alignmentDirty = false; state.reviewed = true; state.alignmentReason = null;
      setHint('Positions reviewed. Capture when ready.');
    }
    render(); persist();
  });
  function markDirty(reason = 'layout') {
    const capture = state.capture;
    if (capture && reason === 'layout' && capture.allowLayoutDrift) return;
    if (capture) capture.invalidated = true;
    if (!state.annotations.length) return;
    const rank = { layout: 1, viewport: 2, disconnected: 3 };
    const changed = !state.alignmentDirty || state.reviewed || (rank[reason] || 0) > (rank[state.alignmentReason] || 0);
    state.alignmentDirty = true; state.reviewed = false; state.alignmentReason = reason;
    if (changed) {
      setHint(reason === 'disconnected' ? 'A marked element disappeared. Remove it or add the mark again.' : reason === 'viewport' ? 'The viewport changed. Review positions before capture.' : 'The page moved. Review positions before capture, or capture anyway as shown.');
      render(); persist();
    }
  }
  function observePageChange() {
    if (!state.annotations.length) return;
    const status = geometryStatus(geometryBaseline);
    if (!status.viewportChanged && !status.layoutChanged) return;
    markDirty(status.disconnected ? 'disconnected' : status.viewportChanged ? 'viewport' : 'layout');
  }
  function inspectCapture(capture) {
    if (state.capture !== capture) return;
    if (location.href !== capture.url) { capture.invalidated = true; return; }
    const status = geometryStatus(capture.geometryBaseline);
    if (status.viewportChanged || status.disconnected || (status.layoutChanged && !capture.allowLayoutDrift)) {
      markDirty(status.disconnected ? 'disconnected' : status.viewportChanged ? 'viewport' : 'layout');
    }
  }
  function watchCapture(capture) {
    const tick = () => { if (state.capture !== capture) return; inspectCapture(capture); if (state.capture === capture) capture.monitorFrame = requestAnimationFrame(tick); };
    capture.monitorFrame = requestAnimationFrame(tick);
  }
  function stopCaptureWatch(capture) {
    if (capture?.monitorFrame != null && typeof cancelAnimationFrame === 'function') cancelAnimationFrame(capture.monitorFrame);
  }
  addEventListener('resize', () => { observePageChange(); refreshElementPreview(); }, true); addEventListener('scroll', () => { observePageChange(); refreshElementPreview(); }, true); addEventListener('zoom', () => { observePageChange(); refreshElementPreview(); }, true);
  const observer = new MutationObserver((mutations) => {
    if (mutations.some((mutation) => {
      if (overlayNode(mutation.target)) return false;
      const nodes = [...mutation.addedNodes, ...mutation.removedNodes];
      return !nodes.length || nodes.some((node) => !overlayNode(node));
    })) { observePageChange(); refreshElementPreview(); }
  });
  observer.observe(document.documentElement, { subtree: true, childList: true, attributes: true, characterData: true });
  layoutObserver = typeof ResizeObserver === 'function' ? new ResizeObserver(() => { observePageChange(); refreshElementPreview(); }) : null;
  syncObservedAnchors();
  function exportAnnotations(imageWidth, imageHeight) { const capture = state.capture; const sx = imageWidth / capture.viewport.width, sy = imageHeight / capture.viewport.height; const mapPoint = (p) => ({ x: (p.x - capture.viewport.scroll_x) * sx, y: (p.y - capture.viewport.scroll_y) * sy }); return capture.annotations.map((item) => ({ id: item.id, kind: item.kind, comment: bounded(item.comment, 4000), color: item.color, points: item.points.map(mapPoint), bounds: item.bounds ? { x: (item.bounds.x - capture.viewport.scroll_x) * sx, y: (item.bounds.y - capture.viewport.scroll_y) * sy, width: item.bounds.width * sx, height: item.bounds.height * sy } : null, element: item.element })); }
  async function onMessage(message) {
    if (!message || message.type === 'init') {
      const fresh = !state.documentId;
      if (message?.document_id) state.documentId = message.document_id;
      if (message?.tab_id != null) state.tabId = message.tab_id;
      if (fresh) {
        const draft = message?.draft && message.draft.document_id === state.documentId ? message.draft : null;
        state.annotations = draft?.annotations || [];
        anchorRefs.clear(); state.captureAnyway = false;
        state.alignmentDirty = Boolean(draft?.alignment_dirty); state.reviewed = !state.alignmentDirty; state.alignmentReason = state.alignmentDirty ? 'layout' : null;
        geometryBaseline = state.annotations.length ? makeGeometryBaseline(true, draft?.viewport || null) : null;
        const status = geometryStatus(geometryBaseline);
        if (status.disconnected) { state.alignmentDirty = true; state.reviewed = false; state.alignmentReason = 'disconnected'; }
        else if (status.viewportChanged || status.layoutChanged) { state.alignmentDirty = true; state.reviewed = false; state.alignmentReason = status.viewportChanged ? 'viewport' : 'layout'; }
      }
      setMode(fresh ? 'browse' : state.mode, false); return { document_id: state.documentId };
    }
    if (message.document_id !== state.documentId) throw new Error('The original page document is no longer active');
    if (message.type === 'capture-saved') {
      const ids = new Set(message.ids);
      state.annotations = state.annotations.filter(annotation => !ids.has(annotation.id));
      for (const id of ids) forgetAnnotation(id);
      state.selected = null; textarea.value = '';
      if (!state.annotations.length) { geometryBaseline = null; state.alignmentDirty = false; state.reviewed = false; state.alignmentReason = null; state.captureAnyway = false; }
      render(); persist(); return { saved: true };
    }
    if (message.type === 'mode') { setMode(message.mode, message.mode === 'annotate'); return { document_id: state.documentId }; }
    if (message.type === 'prepare-capture') {
      const allowLayoutDrift = state.captureAnyway;
      state.captureAnyway = false;
      if (!state.annotations.length) throw new Error('Add a mark or comment before capturing');
      observePageChange();
      const status = geometryStatus(geometryBaseline);
      if (status.disconnected) throw new Error('A marked element is no longer available. Remove it or add the mark again');
      if (state.alignmentDirty && !state.reviewed && !allowLayoutDrift) throw new Error('Review changed positions before capture');
      const vp = viewport();
      const visible = p => p.x >= vp.scroll_x && p.y >= vp.scroll_y && p.x <= vp.scroll_x + vp.width && p.y <= vp.scroll_y + vp.height;
      if (state.annotations.some(annotation => annotation.points.some(p => !visible(p)) || (annotation.bounds && (!visible(annotation.bounds) || !visible({ x: annotation.bounds.x + annotation.bounds.width, y: annotation.bounds.y + annotation.bounds.height }))))) throw new Error('Some marks are outside the viewport. Scroll back or remove them before capturing');
      const capture = { annotations: structuredClone(state.annotations), viewport: vp, geometryBaseline: makeGeometryBaseline(false, vp), allowLayoutDrift, url: location.href, title: bounded(document.title, 300), document_id: state.documentId, captured_at: new Date().toISOString(), invalidated: false, monitorFrame: null };
      state.capture = capture;
      controls.hidden = true;
      editor.hidden = true; addText.hidden = true;
      watchCapture(capture);
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return { document_id: state.documentId, annotation_count: capture.annotations.length };
    }
    if (message.type === 'finish-capture') {
      const capture = state.capture;
      if (!capture) throw new Error('The page moved during capture. Review positions and capture again');
      inspectCapture(capture);
      if (capture.invalidated) throw new Error('The page moved during capture. Review positions and capture again');
      const vp = viewport();
      const visible = p => p.x >= vp.scroll_x && p.y >= vp.scroll_y && p.x <= vp.scroll_x + vp.width && p.y <= vp.scroll_y + vp.height;
      if (capture.annotations.some(annotation => annotation.points.some(p => !visible(p)) || (annotation.bounds && (!visible(annotation.bounds) || !visible({ x: annotation.bounds.x + annotation.bounds.width, y: annotation.bounds.y + annotation.bounds.height }))))) throw new Error('Some marks are outside the viewport. Scroll back or remove them before capturing');
      const result = { page: { ...message.page, url: capture.url, title: capture.title, document_id: capture.document_id, captured_at: capture.captured_at, viewport: capture.viewport }, annotations: exportAnnotations(message.image_width, message.image_height) };
      stopCaptureWatch(capture); state.capture = null; controls.hidden = false; render(); persist(); return result;
    }
    if (message.type === 'cancel-capture') { stopCaptureWatch(state.capture); state.capture = null; state.captureAnyway = false; controls.hidden = false; render(); return { document_id: state.documentId }; }
    return { document_id: state.documentId };
  }
  runtime.onMessage.addListener((message, sender, sendResponse) => { Promise.resolve().then(() => onMessage(message, sender)).then(sendResponse).catch((error) => sendResponse({ error: bounded(error.message, 400) })); return true; });
  setMode('browse', false); render();
})();
