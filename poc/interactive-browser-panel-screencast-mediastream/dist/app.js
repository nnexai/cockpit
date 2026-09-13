const invoke = window.__TAURI__?.core?.invoke;
const eventApi = window.__TAURI__?.event;
const panel = document.querySelector('#browser-panel');
const canvas = document.querySelector('#video');
const context = canvas.getContext('2d', { alpha: false, desynchronized: true });
const emptyState = document.querySelector('#empty-state');
const status = document.querySelector('#status');
const fields = Object.fromEntries(['url', 'page-title', 'dimensions', 'page-url', 'active-element', 'fixture-status', 'scroll-y', 'selected-text', 'last-action'].map((id) => [id, document.querySelector(`#${id}`)]));
const PACKET_MAGIC = 0x49505743; const HEADER_BYTES = 48; const MAX_PACKET_BYTES = 2 * 1024 * 1024;
const SAFE_CURSORS = new Set(['default','auto','pointer','text','crosshair','move','not-allowed','wait','grab','grabbing','cell','help','progress','zoom-in','zoom-out','col-resize','row-resize','e-resize','w-resize','n-resize','s-resize']);
let snapshot; let socket; let reconnect; let decoder; let decoderWidth = 0; let decoderHeight = 0; let decoding = []; let decoderEpoch = 0; let newestSequence = 0; let renderedFrames = 0; let inputQueue = Promise.resolve(); let pendingPointerMove; let pointerMoveScheduled = false; let selfTestStarted = false;
function setStatus(message, kind = 'idle') { status.textContent = message; status.dataset.kind = kind; }
function action(message) { fields['last-action'].textContent = message; }
function command(name, args = {}) { if (!invoke) return Promise.reject(new Error('Tauri command bridge is unavailable')); return invoke(name, args); }
function validDimension(value) { return Number.isFinite(value) && value > 0 && value <= 16384; }
function applySnapshot(value) { snapshot = value; fields.url.value = value.url; fields['page-title'].textContent = value.title || 'Untitled Chromium page'; fields.dimensions.textContent = `${value.pixelWidth}×${value.pixelHeight} · VP8 WebCodecs`; fields['page-url'].textContent = value.url; panel.style.cursor = SAFE_CURSORS.has(value.cursor) ? value.cursor : 'default'; }
function parsePacket(data) {
  if (!(data instanceof ArrayBuffer) || data.byteLength < HEADER_BYTES || data.byteLength > MAX_PACKET_BYTES) throw new Error('MediaStream packet exceeded native bounds');
  const view = new DataView(data); if (view.getUint32(0) !== PACKET_MAGIC || view.getUint8(4) !== 1 || view.getUint16(6) !== HEADER_BYTES) throw new Error('invalid MediaStream packet header');
  const sequence = view.getUint32(8), width = view.getUint32(12), height = view.getUint32(16), codec = view.getUint32(20), timestamp = view.getBigUint64(24), payloadLength = view.getUint32(36), keyframe = Boolean(view.getUint8(5) & 1);
  if (!sequence || !validDimension(width) || !validDimension(height) || codec !== 1 || !payloadLength || data.byteLength !== HEADER_BYTES + payloadLength || timestamp > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('invalid VP8 MediaStream packet fields');
  return { sequence, width, height, keyframe, timestamp: Number(timestamp), bytes: new Uint8Array(data, HEADER_BYTES, payloadLength) };
}
function acknowledge(sequence) { if (socket?.readyState === WebSocket.OPEN) socket.send(`ack:${sequence}`); }
function failDecoder(error) { const detail = error instanceof Error ? error.message : String(error); setStatus(`VideoDecoder failed: ${detail}`, 'error'); emptyState.textContent = `Native WebKit VideoDecoder self-test failed: ${detail}`; emptyState.hidden = false; discardDecoder(); }
function discardDecoder() { decoderEpoch += 1; try { decoder?.close(); } catch {} decoder = undefined; decoderWidth = 0; decoderHeight = 0; decoding = []; }
function resetStream() { discardDecoder(); newestSequence = 0; }
function ensureDecoder(packet) {
  if (!window.VideoDecoder) throw new Error('this native WebKit runtime does not expose VideoDecoder');
  if (decoder && decoderWidth === packet.width && decoderHeight === packet.height) return;
  if (!packet.keyframe) throw new Error('decoder configuration requires a VP8 keyframe');
  discardDecoder();
  decoderWidth = packet.width; decoderHeight = packet.height;
  const epoch = decoderEpoch;
  decoder = new VideoDecoder({ output(frame) { const sequence = decoding.shift(); try { if (epoch !== decoderEpoch) return; canvas.width = frame.displayWidth || frame.codedWidth; canvas.height = frame.displayHeight || frame.codedHeight; context.drawImage(frame, 0, 0, canvas.width, canvas.height); newestSequence = sequence || newestSequence; renderedFrames += 1; emptyState.hidden = true; if (sequence) acknowledge(sequence); } finally { frame.close(); } }, error(error) { if (epoch === decoderEpoch) failDecoder(error); } });
  decoder.configure({ codec: 'vp8', codedWidth: packet.width, codedHeight: packet.height, optimizeForLatency: true });
}
function receivePacket(event) {
  try {
    const packet = parsePacket(event.data);
    if (packet.keyframe && packet.sequence <= newestSequence) resetStream();
    else if (packet.sequence <= newestSequence) { acknowledge(packet.sequence); return; }
    ensureDecoder(packet);
    decoding.push(packet.sequence);
    decoder.decode(new EncodedVideoChunk({ type: packet.keyframe ? 'key' : 'delta', timestamp: packet.timestamp, data: packet.bytes }));
  } catch (error) { failDecoder(error); socket?.close(); }
}
function connect(streamUrl) {
  clearTimeout(reconnect); socket?.close(); const active = new WebSocket(streamUrl); active.binaryType = 'arraybuffer'; socket = active;
  active.addEventListener('open', () => setStatus('Receiving bounded VP8 WebCodecs packets…', 'ready'));
  active.addEventListener('message', receivePacket);
  active.addEventListener('error', () => { if (socket === active) setStatus('MediaStream packet WebSocket error', 'error'); });
  active.addEventListener('close', () => { if (socket !== active) return; discardDecoder(); reconnect = setTimeout(() => { if (snapshot?.streamUrl) connect(snapshot.streamUrl); }, 500); });
}
async function inspect() { const value = await command('browser_inspect'); fields['active-element'].textContent = value.activeElement; fields['fixture-status'].textContent = value.fixtureStatus; fields['scroll-y'].textContent = String(value.scrollY); fields['selected-text'].textContent = value.selectedText || '—'; fields['page-url'].textContent = value.url; fields['page-title'].textContent = value.title || 'Untitled Chromium page'; panel.style.cursor = SAFE_CURSORS.has(value.cursor) ? value.cursor : 'default'; return value; }
async function start() { try { if (!window.VideoDecoder) throw new Error('this native WebKit runtime does not expose VideoDecoder'); const value = await command('browser_start'); applySnapshot(value); connect(value.streamUrl); action('MediaStream capture started'); } catch (error) { failDecoder(error); } }
function modifierMask(event) { return (event.altKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.metaKey ? 4 : 0) | (event.shiftKey ? 8 : 0); }
function point(event) { const rect = panel.getBoundingClientRect(); const viewport = snapshot?.viewport || { width: 1024, height: 720 }; return { x: Math.max(0, Math.min(viewport.width, (event.clientX - rect.left) * viewport.width / rect.width)), y: Math.max(0, Math.min(viewport.height, (event.clientY - rect.top) * viewport.height / rect.height)) }; }
async function sendInput(event, label) { const ack = await command('browser_input', { event }); panel.style.cursor = SAFE_CURSORS.has(ack.cursor) ? ack.cursor : 'default'; action(label); }
function queueInput(event, label) { inputQueue = inputQueue.then(() => sendInput(event, label)).catch((error) => setStatus(`Input failed: ${error.message || error}`, 'error')); return inputQueue; }
function queuePointerMove(event, label) {
  pendingPointerMove = { event, label };
  if (pointerMoveScheduled) return inputQueue;
  pointerMoveScheduled = true;
  inputQueue = inputQueue.then(async () => {
    const move = pendingPointerMove; pendingPointerMove = undefined; pointerMoveScheduled = false;
    if (move) await sendInput(move.event, move.label);
  }).catch((error) => setStatus(`Input failed: ${error.message || error}`, 'error'));
  return inputQueue;
}
function button(button) { return button === 1 ? 'middle' : button === 2 ? 'right' : 'left'; }
panel.addEventListener('pointerdown', (event) => { event.preventDefault(); panel.focus(); panel.setPointerCapture(event.pointerId); const value = point(event); queueInput({ kind: 'mouseDown', ...value, button: button(event.button), buttons: event.buttons, modifiers: modifierMask(event) }, 'Pointer pressed'); });
panel.addEventListener('pointerup', (event) => { event.preventDefault(); const value = point(event); queueInput({ kind: 'mouseUp', ...value, button: button(event.button), buttons: event.buttons, modifiers: modifierMask(event) }, 'Pointer released'); });
panel.addEventListener('pointermove', (event) => { const value = point(event); queuePointerMove({ kind: 'mouseMove', ...value, buttons: event.buttons, modifiers: modifierMask(event) }, 'Pointer moved'); });
panel.addEventListener('wheel', (event) => { event.preventDefault(); const value = point(event); queueInput({ kind: 'wheel', ...value, deltaX: event.deltaX, deltaY: event.deltaY, modifiers: modifierMask(event) }, 'Scrolled Chromium'); }, { passive: false });
panel.addEventListener('keydown', (event) => { if (event.isComposing) return; event.preventDefault(); queueInput({ kind: 'keyDown', key: event.key, code: event.code, text: event.key.length === 1 ? event.key : undefined, modifiers: modifierMask(event) }, `Key down: ${event.key}`); });
panel.addEventListener('keyup', (event) => { if (event.isComposing) return; event.preventDefault(); queueInput({ kind: 'keyUp', key: event.key, code: event.code, modifiers: modifierMask(event) }, `Key up: ${event.key}`); });
document.querySelector('#navigation').addEventListener('submit', async (event) => { event.preventDefault(); try { applySnapshot(await command('browser_navigate', { url: fields.url.value })); action('Navigated Chromium'); } catch (error) { setStatus(`Navigation failed: ${error.message || error}`, 'error'); } });
document.querySelector('#reload').addEventListener('click', async () => { try { applySnapshot(await command('browser_reload')); action('Reloaded Chromium'); } catch (error) { setStatus(`Reload failed: ${error.message || error}`, 'error'); } });
document.querySelector('#inspect').addEventListener('click', () => { void inspect().then(() => action('Inspected Chromium state')).catch((error) => setStatus(`Inspect failed: ${error.message || error}`, 'error')); });
async function waitForRenderedFrameAfter(count, timeoutMs = 8_000) { const deadline = performance.now() + timeoutMs; while (renderedFrames <= count) { if (performance.now() > deadline) throw new Error('no decoded MediaStream/WebCodecs frame arrived'); await new Promise((resolve) => setTimeout(resolve, 25)); } }
async function selfTest() {
  if (selfTestStarted) return;
  selfTestStarted = true;
  try {
    if (!window.VideoDecoder) throw new Error('native WebKit does not provide VideoDecoder');
    await waitForRenderedFrameAfter(0);
    const beforeReload = renderedFrames;
    applySnapshot(await command('browser_reload'));
    await waitForRenderedFrameAfter(beforeReload);
    const fixture = await inspect();
    if (!fixture.title.includes('Interactive MediaStream Fixture')) throw new Error('reload did not preserve fixture state');
    const beforeNavigation = renderedFrames;
    const navigated = await command('browser_navigate', { url: 'https://google.de' });
    applySnapshot(navigated);
    await waitForRenderedFrameAfter(beforeNavigation);
    const target = await inspect();
    const googleHost = new URL(target.url).hostname;
    if (!target.title.includes('Google') || !/(^|\.)google\.(de|com)$/.test(googleHost)) throw new Error(`navigation did not reach Google host (${target.title}, ${target.url})`);
    await command('browser_self_test_report', { success: true, detail: 'MediaStream getDisplayMedia → realtime VP8 VideoEncoder → bounded WebSocket → native VideoDecoder rendered fresh frames after reload and google.de navigation' });
  } catch (error) {
    const detail = `MediaStream/WebCodecs self-test failed: ${error instanceof Error ? error.message : String(error)}`;
    failDecoder(detail);
    await command('browser_self_test_report', { success: false, detail }).catch(() => {});
  }
}
if (eventApi?.listen) void eventApi.listen('interactive-browser-panel-self-test', selfTest);
window.addEventListener('beforeunload', () => { clearTimeout(reconnect); socket?.close(); try { decoder?.close(); } catch {} void command('browser_stop').catch(() => {}); });
start();
