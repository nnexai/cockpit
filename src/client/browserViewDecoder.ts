import {
  CockpitClientError, matchBrowserViewEvent, parseBrowserViewEvent,
  parseBrowserViewFrameDescriptor,
  type BrowserViewEvent, type BrowserViewFrameDescriptor,
  type BrowserViewFramePacket, type BrowserViewSnapshot,
} from "./CockpitClient";

export interface BrowserFrameEnvelope {
  readonly data: ArrayBuffer;
  readonly frameSequence: number;
}
export type BrowserFrameRelease = (kind: "ack" | "discard") => void;

/** Owns decoded identity/order only; socket readiness, cancellation and release stay in the adapters. */
export function browserViewDecoder(
  snapshot: BrowserViewSnapshot,
  backend: "browser" | "native",
  onEvent: (event: BrowserViewEvent) => void,
  onFrame: (frame: BrowserViewFramePacket) => void,
) {
  const identity = snapshot.identity;
  let targetId: string | undefined = snapshot.displayed_target_id ?? (backend === "browser" ? "" : undefined);
  let attached = false;
  let lastFrameSequence = 0;
  const deliver = (descriptor: BrowserViewFrameDescriptor, jpeg: ArrayBuffer, release: BrowserFrameRelease) => {
    try { onFrame({ descriptor, jpeg, ack: () => release("ack"), discard: () => release("discard") }); }
    catch (cause) {
      release("discard");
      throw new CockpitClientError("transport_error", backend === "browser" ? "Browser view frame handler failed" : "Native browser frame handler failed", { cause });
    }
  };
  return {
    get attached() { return attached; },
    event(value: unknown): boolean {
      const event = matchBrowserViewEvent(parseBrowserViewEvent(value), identity);
      if (backend === "browser") {
        if (event.type === "attached") {
          if (attached) return false;
        } else if (!attached) return false;
      }
      if (event.type === "attached") {
        targetId = event.snapshot.displayed_target_id ?? (backend === "browser" ? "" : undefined);
        attached = true;
      } else if (event.type === "targets_changed") targetId = event.displayed_target_id ?? (backend === "browser" ? "" : undefined);
      else if (event.type === "document_changed") targetId = event.document?.target_id ?? (backend === "browser" ? "" : undefined);
      onEvent(event);
      return event.type === "attached";
    },
    envelope(data: unknown): BrowserFrameEnvelope {
      if (!(data instanceof ArrayBuffer)) throw new CockpitClientError("malformed_response", "Browser frame is not binary");
      if (data.byteLength < 96 || data.byteLength > 96 + 6 * 1024 * 1024) throw new CockpitClientError("malformed_response", "Browser frame exceeds bounds");
      const view = new DataView(data);
      const jpegLength = view.getUint32(80, false);
      if (view.getUint32(0, false) !== 0x49424656 || view.getUint16(4, false) !== 2 || view.getUint16(6, false) !== 96 || view.getBigUint64(8, false) !== BigInt(identity.stream_epoch) || jpegLength === 0 || data.byteLength !== 96 + jpegLength) {
        throw new CockpitClientError("malformed_response", "Browser frame envelope is invalid");
      }
      const jpeg = new Uint8Array(data, 96, 2);
      if (jpeg[0] !== 0xff || jpeg[1] !== 0xd8) throw new CockpitClientError("malformed_response", "Browser frame payload is not JPEG");
      return { data, frameSequence: Number(view.getBigUint64(16, false)) };
    },
    browserFrame(frame: BrowserFrameEnvelope, release: BrowserFrameRelease) {
      let descriptor: BrowserViewFrameDescriptor;
      let jpeg: ArrayBuffer;
      try {
        if (!targetId) throw new CockpitClientError("malformed_response", "Browser frame target identity is unavailable");
        const { data } = frame;
        const view = new DataView(data);
        descriptor = parseBrowserViewFrameDescriptor({
          target_id: targetId,
          stream_epoch: Number(view.getBigUint64(8, false)),
          frame_sequence: frame.frameSequence,
          document_generation: Number(view.getBigUint64(24, false)),
          viewport_revision: Number(view.getBigUint64(32, false)),
          image_width: view.getUint32(40, false),
          image_height: view.getUint32(44, false),
          viewport_css_width: view.getFloat32(48, false),
          viewport_css_height: view.getFloat32(52, false),
          viewport_offset_x: view.getFloat32(56, false),
          viewport_offset_y: view.getFloat32(60, false),
          scroll_x: view.getFloat32(64, false),
          scroll_y: view.getFloat32(68, false),
          capture_timestamp_micros: Number(view.getBigUint64(72, false)),
          jpeg_length: view.getUint32(80, false),
        });
        jpeg = data.slice(96);
      } catch (error) { release("discard"); throw error; }
      deliver(descriptor, jpeg, release);
    },
    nativeDescriptor(value: unknown): BrowserViewFrameDescriptor {
      const descriptor = parseBrowserViewFrameDescriptor(value);
      if (targetId !== undefined && descriptor.target_id !== targetId) throw new CockpitClientError("malformed_response", "Native browser frame target identity does not match");
      if (descriptor.stream_epoch !== identity.stream_epoch) throw new CockpitClientError("malformed_response", "Native browser frame stream identity does not match");
      if (descriptor.frame_sequence <= lastFrameSequence) throw new CockpitClientError("malformed_response", "Native browser frame sequence is out of order");
      return descriptor;
    },
    nativeFrame(value: unknown, descriptor: BrowserViewFrameDescriptor, release: BrowserFrameRelease) {
      let bytes: Uint8Array;
      if (value instanceof ArrayBuffer) bytes = new Uint8Array(value);
      else if (value instanceof Uint8Array) bytes = value;
      else throw new CockpitClientError("malformed_response", "Native browser frame payload is not binary");
      if (bytes.byteLength === 0 || bytes.byteLength > 6 * 1024 * 1024 || bytes.byteLength !== descriptor.jpeg_length || bytes[0] !== 0xff || bytes[1] !== 0xd8) {
        throw new CockpitClientError("malformed_response", "Native browser frame payload is invalid");
      }
      const jpeg = value instanceof ArrayBuffer ? value
        : bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength && bytes.buffer instanceof ArrayBuffer ? bytes.buffer
        : bytes.slice().buffer;
      lastFrameSequence = descriptor.frame_sequence;
      deliver(descriptor, jpeg, release);
    },
  };
}
