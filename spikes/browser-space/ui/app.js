(() => {
  'use strict';

  const TOKEN_KEY = 'cockpit-browser-space-ui-token';
  const state = {
    value: null,
    annotations: [],
    refreshInFlight: false,
    actionInFlight: false,
    inspectInFlight: false,
    inspect: null,
  };

  const $ = (id) => document.getElementById(id);
  const setText = (id, value) => {
    const node = $(id);
    if (node) node.textContent = value == null ? '' : String(value);
  };
  const text = (value, fallback = '—') => value == null || value === '' ? fallback : String(value);
  function readToken() {
    let token = '';
    let fragmentToken = '';
    const fragment = location.hash.slice(1);
    if (fragment) {
      try {
        const params = new URLSearchParams(fragment);
        fragmentToken = /^token=/i.test(fragment) ? (params.get('token') || '') : decodeURIComponent(fragment);
      } catch {
        fragmentToken = '';
      }
      try {
        history.replaceState(null, document.title, `${location.pathname}${location.search}`);
      } catch {
        // URL cleanup is best effort when history is unavailable.
      }
    }
    try {
      token = sessionStorage.getItem(TOKEN_KEY) || '';
    } catch {
      // A blocked storage implementation does not prevent using a fresh fragment token.
    }
    if (fragmentToken) {
      token = fragmentToken;
      try {
        sessionStorage.setItem(TOKEN_KEY, token);
      } catch {
        // Keep the token in memory for this page load.
      }
    }
    return token;
  }

  const token = readToken();
  const endpoint = location.origin;

  function setConnection(kind, message) {
    const status = $('connection-status');
    if (!status) return;
    status.dataset.state = kind;
    setText('connection-text', message);
  }

  function setBusy(busy) {
    state.actionInFlight = busy;
    ['open-preview', 'show-browser', 'grant-control', 'take-control'].forEach((id) => {
      const button = $(id);
      if (button) button.disabled = busy;
    });
  }

  async function request(path, options = {}) {
    if (!token) throw new Error('missing UI token');
    const headers = new Headers(options.headers || {});
    headers.set('Authorization', `Bearer ${token}`);
    if (options.body !== undefined) headers.set('Content-Type', 'application/json');
    const response = await fetch(`${endpoint}${path}`, { ...options, headers });
    if (!response.ok) {
      const error = new Error(`request failed (${response.status})`);
      error.status = response.status;
      throw error;
    }
    return response.json();
  }

  function setActionError(message) {
    setText('control-note', message);
    $('control-note').dataset.state = 'error';
  }

  function clearActionError() {
    $('control-note').dataset.state = '';
    setText('control-note', 'Control changes are sent to the browser-space server and confirmed by state.');
  }

  function showToast(message) {
    const toast = $('toast');
    toast.textContent = message;
    toast.hidden = false;
    window.setTimeout(() => { toast.hidden = true; }, 2600);
  }

  async function post(path, body) {
    return request(path, { method: 'POST', body: JSON.stringify(body) });
  }

  function renderState(next) {
    state.value = next;
    const control = next.control === 'agent' ? 'AGENT' : next.control === 'human' ? 'HUMAN' : text(next.control, 'UNKNOWN').toUpperCase();
    setText('space-heading', text(next.spaceLabel, 'Unnamed Space'));
    setText('space-id', next.spaceId ? `ID ${next.spaceId}` : 'Space ID unavailable');
    setText('session-name', text(next.session));
    setText('browser-instance', next.instanceId ? `#${next.instanceId}` : 'Dedicated instance');
    setText('control-owner', control);
    $('space-state').textContent = 'CONNECTED';
    $('space-state').dataset.state = 'connected';
    $('grant-control').disabled = state.actionInFlight || next.control === 'agent';
    $('take-control').disabled = state.actionInFlight || next.control !== 'agent';

    const tabs = Array.isArray(next.tabs) ? next.tabs : [];
    setText('tab-count', `${tabs.length} ${tabs.length === 1 ? 'tab' : 'tabs'}`);
    renderTabs(tabs);

    if (next.connectionPath) {
      const exportLine = `export COCKPIT_BROWSER_CONNECTION=${shellQuote(next.connectionPath)}`;
      $('env-export').textContent = exportLine;
      const fixtureUrl = shellQuote(next.fixtureUrl || '<fixture-url-from-state>');
      $('cli-example').textContent = [
        `bun run agent.ts status --connection ${shellQuote(next.connectionPath)}`,
        `bun run agent.ts inspect TAB_ID --connection ${shellQuote(next.connectionPath)}`,
        `bun run agent.ts fill TAB_ID '[data-testid=\"card-number\"]' '4242 4242 4242 4242' ${fixtureUrl}`,
        `bun run agent.ts click TAB_ID '[data-testid=\"confirm-payment\"]' ${fixtureUrl}`,
      ].join('\n');
      $('copy-export').disabled = false;
    }
  }

  function shellQuote(value) {
    return `'${String(value).replaceAll("'", "'\\''")}'`;
  }

  function renderTabs(tabs) {
    const list = $('tab-list');
    list.replaceChildren();
    if (!tabs.length) {
      const empty = document.createElement('div');
      empty.className = 'empty-state';
      empty.textContent = 'No eligible browser tabs are currently attached.';
      list.append(empty);
      return;
    }
    tabs.forEach((tab, index) => {
      const row = document.createElement('article');
      row.className = 'tab-row';
      row.dataset.tabId = text(tab.id, `tab-${index}`);

      const marker = document.createElement('span');
      marker.className = 'tab-marker';
      marker.textContent = String(index + 1).padStart(2, '0');
      marker.setAttribute('aria-hidden', 'true');
      row.append(marker);

      const details = document.createElement('div');
      details.className = 'tab-details';
      const title = document.createElement('strong');
      title.textContent = text(tab.title, 'Untitled page');
      const url = document.createElement('span');
      url.className = 'tab-url';
      url.textContent = text(tab.url, 'URL unavailable');
      details.append(title, url);
      row.append(details);

      const inspect = document.createElement('button');
      inspect.className = 'small-button';
      inspect.type = 'button';
      inspect.textContent = 'Inspect';
      inspect.dataset.tabId = text(tab.id, '');
      inspect.disabled = !tab.id;
      inspect.addEventListener('click', () => inspectTab(tab));
      row.append(inspect);
      list.append(row);
    });
  }

  async function inspectTab(tab) {
    if (!tab.id || state.inspectInFlight) return;
    state.inspectInFlight = true;
    setConnection('pending', 'Inspecting browser tab…');
    try {
      const result = await post('/api/inspect', { tabId: tab.id });
      state.inspect = result;
      renderInspect(result);
      setConnection('connected', 'Connected');
    } catch {
      setActionError('Inspect failed; the tab may have closed or become unavailable.');
      setConnection('error', 'Connected with errors');
    } finally {
      state.inspectInFlight = false;
    }
  }

  function renderInspect(result) {
    $('inspect-output').hidden = false;
    setText('inspect-url', text(result.url));
    setText('inspect-title', text(result.title));
    const list = $('element-list');
    list.replaceChildren();
    const elements = Array.isArray(result.elements) ? result.elements : [];
    if (!elements.length) {
      const empty = document.createElement('p');
      empty.className = 'muted';
      empty.textContent = 'No bounded interactive elements returned.';
      list.append(empty);
      return;
    }
    elements.forEach((element) => {
      const item = document.createElement('div');
      item.className = 'element-evidence';
      const identity = document.createElement('code');
      identity.textContent = text(element.selector, 'selector unavailable');
      const description = document.createElement('span');
      description.textContent = `${text(element.tag, 'element').toLowerCase()}${element.role ? ` · ${element.role}` : ''} · ${text(element.text, 'no visible text')}`;
      item.append(identity, description);
      list.append(item);
    });
  }

  function annotationValue(annotation, key) {
    if (annotation && annotation[key] !== undefined) return annotation[key];
    if (annotation && annotation.data && annotation.data[key] !== undefined) return annotation.data[key];
    return undefined;
  }

  function renderAnnotations(annotations) {
    const list = $('feedback-list');
    list.replaceChildren();
    const values = Array.isArray(annotations) ? annotations : [];
    setText('feedback-count', `${values.length} ${values.length === 1 ? 'item' : 'items'}`);
    if (!values.length) {
      const empty = document.createElement('div');
      empty.className = 'empty-state';
      empty.textContent = 'No saved feedback yet. Use the extension to select an element or draw on a page.';
      list.append(empty);
      return;
    }
    values.forEach((annotation) => list.append(renderAnnotation(annotation)));
  }

  function renderAnnotation(annotation) {
    const card = document.createElement('article');
    card.className = 'feedback-card';
    const kind = annotationValue(annotation, 'kind') === 'drawing' ? 'SCREENSHOT / DRAWING' : 'ELEMENT FEEDBACK';
    const badge = document.createElement('span');
    badge.className = 'feedback-kind';
    badge.textContent = kind;
    card.append(badge);

    const comment = document.createElement('p');
    comment.className = 'feedback-comment';
    comment.textContent = text(annotationValue(annotation, 'comment'), 'No comment');
    card.append(comment);

    const title = document.createElement('div');
    title.className = 'feedback-context';
    title.textContent = `${text(annotationValue(annotation, 'title'), 'Untitled page')} · ${text(annotationValue(annotation, 'url'), 'URL unavailable')}`;
    card.append(title);

    const element = annotationValue(annotation, 'element');
    if (element) {
      const evidence = document.createElement('div');
      evidence.className = 'annotation-evidence';
      const selector = document.createElement('code');
      selector.textContent = text(element.selector, 'selector unavailable');
      const target = document.createElement('span');
      target.textContent = `${text(element.tag, 'element').toLowerCase()} · ${text(element.text, 'no visible text')}`;
      evidence.append(selector, target);
      card.append(evidence);
    }

    const image = annotationValue(annotation, 'image');
    const strokes = annotationValue(annotation, 'strokes');
    if (typeof image === 'string' && /^data:image\/(?:png|jpeg);base64,[a-z0-9+/=]+$/i.test(image)) {
      const figure = document.createElement('figure');
      figure.className = 'screenshot-evidence';
      const stage = document.createElement('div');
      stage.className = 'screenshot-stage';
      const img = document.createElement('img');
      img.src = image;
      img.alt = 'Screenshot captured from the associated browser tab';
      const viewport = annotationValue(annotation, 'viewport') || {};
      const dpr = Number(viewport.devicePixelRatio) > 0 ? Number(viewport.devicePixelRatio) : 1;
      const fallbackWidth = Number(viewport.width) > 0 ? Number(viewport.width) * dpr : 1;
      const fallbackHeight = Number(viewport.height) > 0 ? Number(viewport.height) * dpr : 1;
      const strokeOverlay = Array.isArray(strokes) && strokes.length ? renderStrokes(strokes, fallbackWidth, fallbackHeight) : null;
      stage.append(img);
      if (strokeOverlay) {
        stage.append(strokeOverlay);
        img.addEventListener('load', () => {
          const exactOverlay = renderStrokes(strokes, img.naturalWidth || fallbackWidth, img.naturalHeight || fallbackHeight);
          strokeOverlay.replaceWith(exactOverlay);
        }, { once: true });
      }
      figure.append(stage);
      const caption = document.createElement('figcaption');
      caption.textContent = `${strokes?.length || 0} stroke${strokes?.length === 1 ? '' : 's'} · captured viewport evidence`;
      figure.append(caption);
      card.append(figure);
    } else if (Array.isArray(strokes) && strokes.length) {
      const strokeSummary = document.createElement('div');
      strokeSummary.className = 'stroke-summary';
      strokeSummary.textContent = `${strokes.length} drawing stroke${strokes.length === 1 ? '' : 's'} saved with this annotation.`;
      card.append(strokeSummary);
    }

    const created = annotationValue(annotation, 'createdAt');
    if (created) {
      const time = document.createElement('time');
      time.className = 'feedback-time';
      time.dateTime = String(created);
      time.textContent = String(created);
      card.append(time);
    }
    return card;
  }

  function renderStrokes(strokes, width, height) {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.classList.add('stroke-overlay');
    const canvasWidth = Number.isFinite(Number(width)) && Number(width) > 0 ? Number(width) : 1;
    const canvasHeight = Number.isFinite(Number(height)) && Number(height) > 0 ? Number(height) : 1;
    svg.setAttribute('viewBox', `0 0 ${canvasWidth} ${canvasHeight}`);
    svg.setAttribute('preserveAspectRatio', 'none');
    strokes.forEach((stroke) => {
      if (!stroke || !Array.isArray(stroke.points) || stroke.points.length < 2) return;
      const points = stroke.points.filter((point) => Number.isFinite(Number(point.x)) && Number.isFinite(Number(point.y)));
      if (points.length < 2) return;
      const polyline = document.createElementNS('http://www.w3.org/2000/svg', 'polyline');
      polyline.setAttribute('points', points.map((point) => `${Number(point.x)},${Number(point.y)}`).join(' '));
      const color = typeof stroke.color === 'string' && /^#[0-9a-f]{3,8}$/i.test(stroke.color) ? stroke.color : '#ffb454';
      polyline.setAttribute('stroke', color);
      polyline.setAttribute('stroke-width', String(Math.min(Math.max(Number(stroke.width) || 2, 1), 8)));
      polyline.setAttribute('fill', 'none');
      polyline.setAttribute('vector-effect', 'non-scaling-stroke');
      polyline.setAttribute('stroke-linecap', 'round');
      polyline.setAttribute('stroke-linejoin', 'round');
      svg.append(polyline);
    });
    return svg;
  }

  async function refresh(force = false) {
    if (state.refreshInFlight || (state.actionInFlight && !force)) return;
    state.refreshInFlight = true;
    if (!state.value) setConnection('pending', 'Connecting…');
    try {
      const [snapshot, annotationResponse] = await Promise.all([
        request('/api/state'),
        request('/api/annotations'),
      ]);
      renderState(snapshot);
      state.annotations = Array.isArray(annotationResponse.annotations) ? annotationResponse.annotations : (snapshot.annotations || []);
      renderAnnotations(state.annotations);
      setConnection('connected', 'Connected');
    } catch (error) {
      if (!state.value) {
        setConnection('error', error.message === 'missing UI token' ? 'UI token required' : 'Disconnected');
        $('space-state').textContent = error.message === 'missing UI token' ? 'AUTH ERROR' : 'DISCONNECTED';
        $('space-state').dataset.state = 'error';
      } else {
        setConnection('error', 'Disconnected · retrying');
        $('space-state').textContent = 'STALE';
        $('space-state').dataset.state = 'stale';
      }
    } finally {
      state.refreshInFlight = false;
    }
  }

  async function openBrowser(url) {
    if (state.actionInFlight) return;
    setBusy(true);
    clearActionError();
    setConnection('pending', 'Opening browser tab…');
    try {
      const body = url ? { url } : {};
      await post('/api/open', body);
      await refresh(true);
    } catch {
      setActionError('Browser request did not complete. Refresh state before retrying; a tab may already have opened.');
      setConnection('error', 'Browser outcome unconfirmed');
    } finally {
      setBusy(false);
    }
  }

  async function setControl(control) {
    if (state.actionInFlight) return;
    setBusy(true);
    clearActionError();
    setConnection('pending', control === 'agent' ? 'Granting agent control…' : 'Taking back control…');
    try {
      await post('/api/control', { control });
      await refresh(true);
    } catch {
      setActionError('Control request did not complete. Current ownership is unconfirmed until state refreshes.');
      setConnection('error', 'Control outcome unconfirmed');
    } finally {
      setBusy(false);
    }
  }

  $('open-preview').addEventListener('click', () => {
    const fixtureUrl = state.value && state.value.fixtureUrl;
    if (!fixtureUrl) {
      setActionError('Preview is unavailable until the server advertises fixtureUrl.');
      return;
    }
    openBrowser(fixtureUrl);
  });
  $('show-browser').addEventListener('click', () => openBrowser());
  $('grant-control').addEventListener('click', () => setControl('agent'));
  $('take-control').addEventListener('click', () => setControl('human'));
  $('close-inspect').addEventListener('click', () => { $('inspect-output').hidden = true; });
  $('copy-export').addEventListener('click', async () => {
    if (!$('env-export').textContent) return;
    try {
      await navigator.clipboard.writeText($('env-export').textContent);
      showToast('Descriptor export copied');
    } catch {
      setActionError('Clipboard access was denied; select the export manually.');
    }
  });

  if (!token) {
    setConnection('error', 'UI token required');
    $('space-state').textContent = 'AUTH ERROR';
    $('space-state').dataset.state = 'error';
  } else {
    refresh();
    window.setInterval(refresh, 5000);
  }
})();
