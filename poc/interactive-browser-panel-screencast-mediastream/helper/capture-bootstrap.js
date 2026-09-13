(() => {
  if (window.top !== window) return;
  const config = window.__pocMediaStreamCapture;
  if (!config?.ingressUrl) return;

  const CONTROL_ID = 'poc-media-stream-capture';
  const MAX_PACKET_BYTES = 2 * 1024 * 1024;
  const HEADER_BYTES = 48;
  const PACKET_MAGIC = 0x49505743;
  let reader;
  let encoder;
  let captureStream;
  let socket;
  let forceKeyframe = true;
  let sequence = 0;
  let running = false;
  let starting = false;
  let button;
  let localStatus;

  function setStatus(message, state) {
    for (const target of new Set([document.querySelector('#capture-status'), localStatus])) {
      if (!target) continue;
      target.textContent = message;
      if (state) target.dataset.state = state; else delete target.dataset.state;
    }
  }
  function failCapture(error) {
    running = false;
    starting = false;
    if (button) button.disabled = false;
    const detail = error instanceof Error ? error.message : String(error);
    setStatus(`MediaStream/WebCodecs capture failed: ${detail}`, 'error');
    console.error(`MediaStream/WebCodecs capture failed: ${detail}`);
  }
  function packet(chunk, width, height) {
    if (!Number.isSafeInteger(chunk.timestamp) || chunk.timestamp < 0 || !chunk.byteLength || chunk.byteLength > MAX_PACKET_BYTES - HEADER_BYTES) throw new Error('encoded chunk was outside packet bounds');
    const bytes = new Uint8Array(HEADER_BYTES + chunk.byteLength);
    const view = new DataView(bytes.buffer);
    view.setUint32(0, PACKET_MAGIC);
    view.setUint8(4, 1);
    view.setUint8(5, chunk.type === 'key' ? 1 : 0);
    view.setUint16(6, HEADER_BYTES);
    view.setUint32(8, ++sequence);
    view.setUint32(12, width);
    view.setUint32(16, height);
    view.setUint32(20, 1);
    view.setBigUint64(24, BigInt(chunk.timestamp));
    view.setUint32(32, chunk.duration || 0);
    view.setUint32(36, chunk.byteLength);
    view.setUint32(40, 0);
    view.setUint32(44, 0);
    chunk.copyTo(bytes.subarray(HEADER_BYTES));
    return bytes.buffer;
  }
  async function realtimeConfig(width, height, framerate) {
    for (const candidate of [
      { codec: 'vp8', width, height, bitrate: 2_500_000, framerate, latencyMode: 'realtime', hardwareAcceleration: 'prefer-hardware' },
      { codec: 'vp8', width, height, bitrate: 2_500_000, framerate, latencyMode: 'realtime', hardwareAcceleration: 'no-preference' },
      { codec: 'vp8', width, height, bitrate: 1_800_000, framerate, latencyMode: 'realtime' },
    ]) {
      const support = await VideoEncoder.isConfigSupported(candidate);
      if (support.supported) return support.config;
    }
    throw new Error('no supported realtime VP8 VideoEncoder configuration');
  }
  async function encode(track) {
    const settings = track.getSettings();
    const width = Math.max(1, Math.round(settings.width || innerWidth));
    const height = Math.max(1, Math.round(settings.height || innerHeight));
    const framerate = Math.min(30, Math.max(1, Math.round(settings.frameRate || 30)));
    const codec = await realtimeConfig(width, height, framerate);
    encoder = new VideoEncoder({
      output(chunk) {
        if (!socket || socket.readyState !== WebSocket.OPEN || socket.bufferedAmount > MAX_PACKET_BYTES) {
          forceKeyframe = true;
          return;
        }
        try { socket.send(packet(chunk, width, height)); } catch (error) { failCapture(error); }
      },
      error: failCapture,
    });
    encoder.configure(codec);
    reader = new MediaStreamTrackProcessor({ track }).readable.getReader();
    setStatus(`MediaStream live: ${width}×${height}, ${codec.codec}, realtime WebCodecs`);
    while (running) {
      const { value: frame, done } = await reader.read();
      if (done) break;
      try {
        if (encoder.encodeQueueSize < 2 && socket?.readyState === WebSocket.OPEN && socket.bufferedAmount <= MAX_PACKET_BYTES) {
          encoder.encode(frame, { keyFrame: forceKeyframe });
          forceKeyframe = false;
        } else {
          forceKeyframe = true;
        }
      } finally {
        frame.close();
      }
    }
    if (encoder?.state === 'configured') await encoder.flush();
  }
  async function startCapture() {
    if (running || starting) return;
    if (!window.VideoEncoder || !window.MediaStreamTrackProcessor || !navigator.mediaDevices?.getDisplayMedia) {
      failCapture(new Error('this Chromium build lacks MediaStreamTrackProcessor or VideoEncoder'));
      return;
    }
    starting = true;
    button.disabled = true;
    setStatus('Opening real self display capture after trusted click…');
    try {
      socket = new WebSocket(config.ingressUrl);
      socket.binaryType = 'arraybuffer';
      socket.addEventListener('message', (event) => { if (event.data === 'keyframe') forceKeyframe = true; });
      await new Promise((resolve, reject) => {
        socket.addEventListener('open', resolve, { once: true });
        socket.addEventListener('error', () => reject(new Error('ingress WebSocket failed')), { once: true });
      });
      captureStream = await navigator.mediaDevices.getDisplayMedia({
        video: { displaySurface: 'browser', frameRate: { ideal: 30, max: 30 } },
        audio: false,
        selfBrowserSurface: 'include',
        preferCurrentTab: true,
      });
      const track = captureStream.getVideoTracks()[0];
      if (!track) throw new Error('display capture returned no video track');
      running = true;
      starting = false;
      track.addEventListener('ended', () => {
        running = false;
        forceKeyframe = true;
        button.disabled = false;
        setStatus('Display capture ended.', 'error');
      });
      await encode(track);
    } catch (error) {
      failCapture(error);
    }
  }
  function stopCapture() {
    running = false;
    starting = false;
    reader?.cancel().catch(() => {});
    encoder?.close();
    captureStream?.getTracks().forEach((track) => track.stop());
    socket?.close();
  }
  function recordInstallError(error) {
    window.__pocMediaStreamCaptureInstallError = error instanceof Error ? error.message : String(error);
  }
  function install() {
    if (!document.body) throw new Error('document body was unavailable');
    if (document.getElementById(`${CONTROL_ID}-host`)) return;
    const host = document.createElement('div');
    host.id = `${CONTROL_ID}-host`;
    host.style.cssText = 'all:initial;position:fixed;right:16px;bottom:16px;z-index:2147483647;display:block';
    const root = host.attachShadow({ mode: 'open' });
    root.innerHTML = `<style>:host{all:initial}#${CONTROL_ID}{all:initial;display:block;box-sizing:border-box;border:0;border-radius:8px;padding:10px 14px;background:#1769d1;color:#fff;font:600 14px/1.2 system-ui,sans-serif;box-shadow:0 2px 12px #0006;cursor:pointer}#${CONTROL_ID}:disabled{cursor:wait;opacity:.72}.status{all:initial;display:block;max-width:260px;margin-top:8px;padding:8px 10px;border-radius:7px;background:#effaf4;color:#17633e;font:12px/1.3 system-ui,sans-serif;box-shadow:0 2px 12px #0004}.status[data-state=error]{background:#fff1ef;color:#a73c2f}</style><button id="${CONTROL_ID}" type="button">Start MediaStream capture</button><span class="status" aria-live="polite">Capture is waiting for a trusted click.</span>`;
    button = root.querySelector(`#${CONTROL_ID}`);
    localStatus = root.querySelector('.status');
    button.addEventListener('click', () => { void startCapture(); });
    document.documentElement.append(host);
  }
  try {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', () => { try { install(); } catch (error) { recordInstallError(error); } }, { once: true }); else install();
    addEventListener('beforeunload', stopCapture, { once: true });
  } catch (error) {
    recordInstallError(error);
  }
})();
