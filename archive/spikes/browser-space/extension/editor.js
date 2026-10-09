const canvas = document.querySelector('#canvas');
const context = canvas.getContext('2d');
const status = document.querySelector('#status');
const target = document.querySelector('#target');
const comment = document.querySelector('#comment');
const color = document.querySelector('#color');
const width = document.querySelector('#width');
const widthValue = document.querySelector('#width-value');
const save = document.querySelector('#save');
const clear = document.querySelector('#clear');

let capture = null;
let strokes = [];
let activeStroke = null;
let image = null;
const captureId = new URLSearchParams(location.search).get('capture');

function send(message) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendMessage(message, (response) => {
      if (chrome.runtime.lastError) return reject(new Error(chrome.runtime.lastError.message));
      if (!response?.ok) return reject(new Error(response?.error || 'The Space request failed.'));
      resolve(response);
    });
  });
}

function setStatus(message, kind = '') {
  status.textContent = message;
  status.className = `status ${kind}`;
}

function redraw() {
  if (!image) return;
  context.clearRect(0, 0, canvas.width, canvas.height);
  context.drawImage(image, 0, 0);
  for (const stroke of strokes) {
    if (!Array.isArray(stroke.points) || stroke.points.length < 1) continue;
    context.save();
    context.strokeStyle = stroke.color;
    context.lineWidth = stroke.width;
    context.lineJoin = 'round';
    context.lineCap = 'round';
    context.beginPath();
    context.moveTo(stroke.points[0].x, stroke.points[0].y);
    for (const point of stroke.points.slice(1)) context.lineTo(point.x, point.y);
    context.stroke();
    context.restore();
  }
}

function pointFromEvent(event) {
  const rect = canvas.getBoundingClientRect();
  return {
    x: Math.max(0, Math.min(canvas.width, (event.clientX - rect.left) * canvas.width / rect.width)),
    y: Math.max(0, Math.min(canvas.height, (event.clientY - rect.top) * canvas.height / rect.height)),
  };
}

function persistDraft() {
  if (!captureId) return;
  send({ type: 'COCKPIT_DRAFT', captureId, draft: { comment: comment.value, strokes } }).catch(() => {});
}

canvas.addEventListener('pointerdown', (event) => {
  if (!image) return;
  canvas.setPointerCapture(event.pointerId);
  activeStroke = { points: [pointFromEvent(event)], color: color.value, width: Number(width.value) };
  strokes.push(activeStroke);
  redraw();
});
canvas.addEventListener('pointermove', (event) => {
  if (!activeStroke) return;
  activeStroke.points.push(pointFromEvent(event));
  redraw();
});
canvas.addEventListener('pointerup', () => { activeStroke = null; persistDraft(); });
canvas.addEventListener('pointercancel', () => { activeStroke = null; persistDraft(); });

width.addEventListener('input', () => { widthValue.textContent = `${width.value}px`; });
clear.addEventListener('click', async () => {
  strokes = [];
  activeStroke = null;
  redraw();
  if (captureId) await send({ type: 'COCKPIT_DRAFT', captureId, draft: { comment: comment.value, strokes } }).catch(() => {});
});
comment.addEventListener('input', () => {
  if (!captureId) return;
  send({ type: 'COCKPIT_DRAFT', captureId, draft: { comment: comment.value, strokes } }).catch(() => {});
});

save.addEventListener('click', async () => {
  if (!capture || !image) {
    setStatus('No screenshot draft is loaded.', 'error');
    return;
  }
  save.disabled = true;
  clear.disabled = true;
  setStatus('Saving feedback…');
  try {
    await send({ type: 'COCKPIT_SAVE_DRAWING', captureId: capture.captureId, comment: comment.value, strokes });
    setStatus('Saved to this Space.', 'saved');
    target.textContent = `${capture.title || 'Page'} — ${capture.url}`;
  } catch (error) {
    setStatus(error.message, 'error');
    // Keep both the visible form and the session draft on failure.
    await send({ type: 'COCKPIT_DRAFT', captureId: capture.captureId, draft: { comment: comment.value, strokes } }).catch(() => {});
    save.disabled = false;
    clear.disabled = false;
    comment.focus();
  }
});

async function load() {
  if (!captureId) {
    setStatus('This editor link has no screenshot draft.', 'error');
    return;
  }
  try {
    const pending = await send({ type: 'COCKPIT_DRAFT', captureId });
    const result = await new Promise((resolve, reject) => {
      chrome.storage.session.get('cockpit.space.pendingCapture', (value) => {
        if (chrome.runtime.lastError) reject(new Error(chrome.runtime.lastError.message));
        else resolve(value['cockpit.space.pendingCapture']);
      });
    });
    capture = result;
    if (!capture || capture.captureId !== captureId) throw new Error('This screenshot draft is no longer available.');
    target.textContent = `${capture.title || 'Page'} — ${capture.url}`;
    image = new Image();
    image.onload = () => {
      canvas.width = image.naturalWidth;
      canvas.height = image.naturalHeight;
      redraw();
      setStatus('Draw on the screenshot, add a comment, and save.');
    };
    image.onerror = () => setStatus('The screenshot image could not be loaded.', 'error');
    image.src = capture.image;
    if (pending.draft) {
      comment.value = String(pending.draft.comment || '').slice(0, 4000);
      strokes = Array.isArray(pending.draft.strokes) ? pending.draft.strokes : [];
    }
  } catch (error) {
    setStatus(error.message, 'error');
  }
}

load();
