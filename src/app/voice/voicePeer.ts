export type VoicePeerOptions = {
  signalOffer: (offer: string, signal: AbortSignal) => Promise<string>;
  onConnectionStateChange?: (state: RTCPeerConnectionState) => void;
  onRemoteStream?: (stream: MediaStream) => void;
  onError?: (error: Error) => void;
};

export type VoicePeer = {
  start(): Promise<void>;
  stop(): void;
  setMuted(muted: boolean): void;
};

const ICE_GATHERING_TIMEOUT_MS = 5_000;

function abortError(): DOMException {
  return new DOMException("Voice peer stopped", "AbortError");
}

function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      // Observe the operation even when it cannot itself be cancelled.
      promise.catch(() => {});
      reject(abortError());
      return;
    }
    const onAbort = () => reject(abortError());
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", onAbort);
        resolve(value);
      },
      (error: unknown) => {
        signal.removeEventListener("abort", onAbort);
        reject(error);
      },
    );
  });
}

function waitForIceGathering(peer: RTCPeerConnection, signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.reject(abortError());
  if (peer.iceGatheringState === "complete") return Promise.resolve();
  return new Promise((resolve, reject) => {
    const finish = (aborted: boolean) => {
      clearTimeout(timer);
      peer.removeEventListener("icegatheringstatechange", onStateChange);
      signal.removeEventListener("abort", onAbort);
      if (aborted) reject(abortError());
      else resolve();
    };
    const onStateChange = () => {
      if (peer.iceGatheringState === "complete") finish(false);
    };
    const onAbort = () => finish(true);
    const timer = setTimeout(() => finish(false), ICE_GATHERING_TIMEOUT_MS);
    peer.addEventListener("icegatheringstatechange", onStateChange);
    signal.addEventListener("abort", onAbort, { once: true });
    if (signal.aborted) finish(true);
    else if (peer.iceGatheringState === "complete") finish(false);
  });
}

/** A single-use peer. Call stop on screen teardown, including when the app backgrounds. */
export function createVoicePeer(options: VoicePeerOptions): VoicePeer {
  const controller = new AbortController();
  const remoteStreams = new Set<MediaStream>();
  const remoteTracks = new Set<MediaStreamTrack>();
  let localStream: MediaStream | null = null;
  let peer: RTCPeerConnection | null = null;
  let channel: RTCDataChannel | null = null;
  let started = false;
  let muted = false;

  const stop = () => {
    if (controller.signal.aborted) return;
    controller.abort();
    if (peer) {
      peer.onconnectionstatechange = null;
      peer.ontrack = null;
    }
    if (channel) {
      channel.onerror = null;
      channel.close();
      channel = null;
    }
    peer?.close();
    peer = null;
    localStream?.getTracks().forEach((track) => track.stop());
    localStream = null;
    remoteTracks.forEach((track) => track.stop());
    remoteStreams.forEach((stream) => stream.getTracks().forEach((track) => track.stop()));
    remoteTracks.clear();
    remoteStreams.clear();
  };

  const fail = (cause: unknown) => {
    if (controller.signal.aborted) return;
    const error = cause instanceof Error ? cause : new Error(String(cause));
    stop();
    options.onError?.(error);
  };

  return {
    async start() {
      if (started) throw new Error("Voice peer has already been started");
      started = true;
      if (controller.signal.aborted) throw abortError();
      try {
        const media = navigator.mediaDevices.getUserMedia({
          audio: { echoCancellation: true },
          video: false,
        }).then((stream) => {
          if (controller.signal.aborted) {
            stream.getTracks().forEach((track) => track.stop());
          } else {
            localStream = stream;
            stream.getAudioTracks().forEach((track) => { track.enabled = !muted; });
          }
          return stream;
        });
        const stream = await abortable(media, controller.signal);
        if (controller.signal.aborted) throw abortError();
        const audioTrack = stream.getAudioTracks()[0];
        if (!audioTrack) throw new Error("Microphone did not provide an audio track");

        const connection = new RTCPeerConnection();
        peer = connection;
        connection.onconnectionstatechange = () => {
          if (controller.signal.aborted) return;
          options.onConnectionStateChange?.(connection.connectionState);
          if (connection.connectionState === "failed") fail(new Error("Voice peer connection failed"));
        };
        connection.ontrack = (event) => {
          if (controller.signal.aborted) {
            event.track.stop();
            return;
          }
          remoteTracks.add(event.track);
          const remoteStream = event.streams[0] ?? new MediaStream([event.track]);
          remoteStreams.add(remoteStream);
          options.onRemoteStream?.(remoteStream);
        };
        connection.addTransceiver(audioTrack, { direction: "sendrecv", streams: [stream] });
        channel = connection.createDataChannel("oai-events");
        channel.onerror = () => fail(new Error("Voice event channel failed"));

        const offer = await abortable(connection.createOffer(), controller.signal);
        await abortable(connection.setLocalDescription(offer), controller.signal);
        await waitForIceGathering(connection, controller.signal);
        const sdp = connection.localDescription?.sdp;
        if (!sdp) throw new Error("Voice peer did not produce an SDP offer");
        const answer = await abortable(options.signalOffer(sdp, controller.signal), controller.signal);
        if (controller.signal.aborted) throw abortError();
        await abortable(connection.setRemoteDescription({ type: "answer", sdp: answer }), controller.signal);
        if (controller.signal.aborted) throw abortError();
      } catch (error) {
        fail(error);
        throw error;
      }
    },
    stop,
    setMuted(value) {
      muted = value;
      localStream?.getAudioTracks().forEach((track) => { track.enabled = !value; });
    },
  };
}
