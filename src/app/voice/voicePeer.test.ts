import { afterEach, describe, expect, it, vi } from "vitest";
import { createVoicePeer } from "./voicePeer";

// The project's ES2022 library does not include Promise.withResolvers.
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function microphone() {
  const track = { enabled: true, stop: vi.fn() };
  const stream = {
    getTracks: () => [track],
    getAudioTracks: () => [track],
  } as unknown as MediaStream;
  return { track, stream };
}

class FakePeerConnection {
  iceGatheringState = "complete";
  localDescription: RTCSessionDescriptionInit | null = null;
  onconnectionstatechange = null;
  ontrack = null;
  addTransceiver = vi.fn();
  createDataChannel = () => ({ onerror: null, close: vi.fn() });
  createOffer = async () => ({ type: "offer" as const, sdp: "offer" });
  setLocalDescription = async (description: RTCSessionDescriptionInit) => { this.localDescription = description; };
  setRemoteDescription = vi.fn(async () => {});
  close = vi.fn();
}

afterEach(() => vi.unstubAllGlobals());

describe("voice microphone lifecycle", () => {
  it("stops a microphone granted after cancellation without opening a peer", async () => {
    const permission = deferred<MediaStream>();
    const { track, stream } = microphone();
    const signalOffer = vi.fn(async () => "answer");
    const Peer = vi.fn();
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: () => permission.promise } });
    vi.stubGlobal("RTCPeerConnection", Peer);
    const peer = createVoicePeer({ signalOffer });

    const start = peer.start();
    peer.stop();
    await expect(start).rejects.toMatchObject({ name: "AbortError" });
    permission.resolve(stream);
    await permission.promise;
    await Promise.resolve();

    expect(track.stop).toHaveBeenCalledOnce();
    expect(Peer).not.toHaveBeenCalled();
    expect(signalOffer).not.toHaveBeenCalled();
  });

  it("mutes the actual microphone, including a mute requested before permission", async () => {
    const permission = deferred<MediaStream>();
    const { track, stream } = microphone();
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: () => permission.promise } });
    vi.stubGlobal("RTCPeerConnection", FakePeerConnection);
    const peer = createVoicePeer({ signalOffer: async () => "answer" });

    const start = peer.start();
    peer.setMuted(true);
    permission.resolve(stream);
    await start;
    expect(track.enabled).toBe(false);
    peer.setMuted(false);
    expect(track.enabled).toBe(true);
    peer.setMuted(true);
    expect(track.enabled).toBe(false);
    peer.stop();
    expect(track.stop).toHaveBeenCalledOnce();
  });
});
