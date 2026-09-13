import type {
  BrowserViewFrameDescriptor,
  BrowserViewFrameEnvelopeV2,
} from "../../protocol/generated/v1";
import type { BrowserViewFramePacket } from "../../client/CockpitClient";

export const IBFV_V2_MAGIC = 0x4942_4656;
export const IBFV_V2_VERSION = 2;
export const IBFV_V2_HEADER_BYTES = 96;
export const IBFV_V2_DEFAULT_LIMITS: BrowserViewFrameEnvelopeV2 = {
  magic: IBFV_V2_MAGIC,
  version: IBFV_V2_VERSION,
  header_bytes: IBFV_V2_HEADER_BYTES,
  max_width: 2560,
  max_height: 1600,
  max_pixels: 2560 * 1600,
  max_jpeg_bytes: 6 * 1024 * 1024,
};

type Ackable = Pick<BrowserViewFramePacket, "ack" | "discard">;

export interface ParsedBrowserFrame {
  readonly streamEpoch: number;
  readonly frameSequence: number;
  readonly documentGeneration: number;
  readonly viewportRevision: number;
  readonly imageWidth: number;
  readonly imageHeight: number;
  readonly viewportCssWidth: number;
  readonly viewportCssHeight: number;
  readonly viewportOffsetX: number;
  readonly viewportOffsetY: number;
  readonly scrollX: number;
  readonly scrollY: number;
  readonly captureTimestampMicros: number;
  readonly jpeg: ArrayBuffer;
}

export interface FrameValidationContext {
  readonly streamEpoch: number;
  readonly targetId: string;
  readonly displayedTargetId: string | null;
  readonly documentGeneration: number;
  readonly viewportRevision: number;
  readonly viewportCssWidth: number;
  readonly viewportCssHeight: number;
}

export type FrameRejectionReason =
  | "truncated"
  | "invalid_envelope"
  | "invalid_jpeg"
  | "invalid_dimensions"
  | "identity_mismatch";

export class BrowserFrameError extends Error {
  readonly reason: FrameRejectionReason;

  constructor(reason: FrameRejectionReason, message: string) {
    super(message);
    this.name = "BrowserFrameError";
    this.reason = reason;
  }
}

function asBytes(data: ArrayBuffer | ArrayBufferView): Uint8Array {
  if (data instanceof Uint8Array) return data;
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
}

function hasJpegMarkers(bytes: Uint8Array): boolean {
  return bytes.length >= 4 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[bytes.length - 2] === 0xff && bytes[bytes.length - 1] === 0xd9;
}

function uint64(view: DataView, offset: number): number {
  const value = view.getBigUint64(offset, false);
  const number = Number(value);
  if (!Number.isSafeInteger(number)) throw new BrowserFrameError("invalid_envelope", "Browser frame sequence exceeds JavaScript's safe integer range");
  return number;
}

function finite(value: number): number {
  if (!Number.isFinite(value)) throw new BrowserFrameError("invalid_envelope", "Browser frame geometry is not finite");
  return value;
}

/** Parse the fixed, big-endian IBFV v2 envelope and return a bounded JPEG payload. */
export function parseBrowserViewFrame(
  data: ArrayBuffer | ArrayBufferView,
  limits: BrowserViewFrameEnvelopeV2 = IBFV_V2_DEFAULT_LIMITS,
): ParsedBrowserFrame {
  const bytes = asBytes(data);
  if (bytes.byteLength < IBFV_V2_HEADER_BYTES) throw new BrowserFrameError("truncated", "Browser frame envelope is truncated");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(0, false) !== limits.magic || view.getUint16(4, false) !== limits.version || view.getUint16(6, false) !== limits.header_bytes) {
    throw new BrowserFrameError("invalid_envelope", "Browser frame envelope version or magic is invalid");
  }
  const headerBytes = view.getUint16(6, false);
  if (headerBytes < IBFV_V2_HEADER_BYTES || headerBytes > bytes.byteLength) throw new BrowserFrameError("invalid_envelope", "Browser frame header length is invalid");
  const streamEpoch = uint64(view, 8);
  const frameSequence = uint64(view, 16);
  const documentGeneration = uint64(view, 24);
  const viewportRevision = uint64(view, 32);
  const imageWidth = view.getUint32(40, false);
  const imageHeight = view.getUint32(44, false);
  const viewportCssWidth = finite(view.getFloat32(48, false));
  const viewportCssHeight = finite(view.getFloat32(52, false));
  const viewportOffsetX = finite(view.getFloat32(56, false));
  const viewportOffsetY = finite(view.getFloat32(60, false));
  const scrollX = finite(view.getFloat32(64, false));
  const scrollY = finite(view.getFloat32(68, false));
  const captureTimestampMicros = uint64(view, 72);
  const jpegLength = view.getUint32(80, false);
  if (imageWidth === 0 || imageHeight === 0 || imageWidth > limits.max_width || imageHeight > limits.max_height || imageWidth * imageHeight > limits.max_pixels) {
    throw new BrowserFrameError("invalid_dimensions", "Browser frame dimensions exceed bounds");
  }
  if (jpegLength === 0 || jpegLength > limits.max_jpeg_bytes || headerBytes + jpegLength > bytes.byteLength) {
    throw new BrowserFrameError("invalid_envelope", "Browser frame JPEG length exceeds bounds");
  }
  const jpeg = bytes.slice(headerBytes, headerBytes + jpegLength);
  if (!hasJpegMarkers(jpeg)) throw new BrowserFrameError("invalid_jpeg", "Browser frame does not contain a complete JPEG payload");
  if (view.getUint32(84, false) !== 0) throw new BrowserFrameError("invalid_envelope", "Browser frame flags are unsupported");
  for (let offset = 88; offset < IBFV_V2_HEADER_BYTES; offset += 4) {
    if (view.getUint32(offset, false) !== 0) throw new BrowserFrameError("invalid_envelope", "Browser frame reserved bytes are non-zero");
  }
  if (headerBytes + jpegLength !== bytes.byteLength) throw new BrowserFrameError("invalid_envelope", "Browser frame contains trailing bytes");
  return { streamEpoch, frameSequence, documentGeneration, viewportRevision, imageWidth, imageHeight, viewportCssWidth, viewportCssHeight, viewportOffsetX, viewportOffsetY, scrollX, scrollY, captureTimestampMicros, jpeg: jpeg.buffer };
}

export function validateFrameDescriptor(descriptor: BrowserViewFrameDescriptor, context: FrameValidationContext, limits: BrowserViewFrameEnvelopeV2 = IBFV_V2_DEFAULT_LIMITS): void {
  if (descriptor.stream_epoch !== context.streamEpoch || descriptor.target_id !== context.targetId || descriptor.target_id !== context.displayedTargetId || descriptor.document_generation !== context.documentGeneration || descriptor.viewport_revision !== context.viewportRevision || Math.abs(descriptor.viewport_css_width - context.viewportCssWidth) > 0.01 || Math.abs(descriptor.viewport_css_height - context.viewportCssHeight) > 0.01) {
    throw new BrowserFrameError("identity_mismatch", "Browser frame is for a stale target, document, epoch, or viewport");
  }
  if (!Number.isInteger(descriptor.image_width) || !Number.isInteger(descriptor.image_height) || descriptor.image_width <= 0 || descriptor.image_height <= 0 || descriptor.image_width > limits.max_width || descriptor.image_height > limits.max_height || descriptor.image_width * descriptor.image_height > limits.max_pixels || descriptor.jpeg_length <= 0 || descriptor.jpeg_length > limits.max_jpeg_bytes) {
    throw new BrowserFrameError("invalid_dimensions", "Browser frame descriptor exceeds bounds");
  }
}

export function packetBytes(packet: BrowserViewFramePacket): ArrayBuffer {
  const value = packet.jpeg;
  return value.slice(0);
}


export interface FramePresenterOptions {
  readonly limits?: BrowserViewFrameEnvelopeV2;
  readonly validate?: (descriptor: BrowserViewFrameDescriptor) => void;
  readonly present: (image: ImageBitmap | HTMLImageElement, descriptor: BrowserViewFrameDescriptor) => void;
  readonly onError?: (error: BrowserFrameError | Error) => void;
}

type ActiveFrame = {
  packet: BrowserViewFramePacket;
  descriptor: BrowserViewFrameDescriptor;
  jpeg: ArrayBuffer;
};

function sameNumericDescriptor(left: ParsedBrowserFrame, right: BrowserViewFrameDescriptor): boolean {
  return left.streamEpoch === right.stream_epoch &&
    left.frameSequence === right.frame_sequence &&
    left.documentGeneration === right.document_generation &&
    left.viewportRevision === right.viewport_revision &&
    left.imageWidth === right.image_width &&
    left.imageHeight === right.image_height &&
    Math.abs(left.viewportCssWidth - right.viewport_css_width) <= 0.01 &&
    Math.abs(left.viewportCssHeight - right.viewport_css_height) <= 0.01;
}

function packetJpeg(packet: BrowserViewFramePacket, limits: BrowserViewFrameEnvelopeV2): ArrayBuffer {
  const bytes = packetBytes(packet);
  const view = new DataView(bytes);
  if (bytes.byteLength >= 4 && view.getUint32(0, false) === IBFV_V2_MAGIC) {
    const parsed = parseBrowserViewFrame(bytes, limits);
    if (!sameNumericDescriptor(parsed, packet.descriptor)) throw new BrowserFrameError("identity_mismatch", "Browser frame envelope does not match its descriptor");
    return parsed.jpeg;
  }
  if (bytes.byteLength !== packet.descriptor.jpeg_length || bytes.byteLength > limits.max_jpeg_bytes || !hasJpegMarkers(new Uint8Array(bytes))) throw new BrowserFrameError("invalid_jpeg", "Browser frame JPEG is invalid");
  return bytes;
}

/**
 * Decodes at most one packet at a time and keeps one latest replacement. A packet's
 * transport credit is released only after `present` returns, or on deliberate discard.
 * Frame sequence ordering is enforced independently of receive order.
 */
export class FramePresenter {
  private readonly limits: BrowserViewFrameEnvelopeV2;
  private readonly validate?: FramePresenterOptions["validate"];
  private readonly present: FramePresenterOptions["present"];
  private readonly onError?: FramePresenterOptions["onError"];
  private active: ActiveFrame | null = null;
  private pending: ActiveFrame | null = null;
  private lastPresentedSequence: number | null = null;
  private closed = false;
  private readonly released = new WeakSet<BrowserViewFramePacket>();

  private discard(packet: BrowserViewFramePacket): void {
    if (this.released.has(packet)) return;
    this.released.add(packet);
    packet.discard();
  }

  private ack(packet: BrowserViewFramePacket): void {
    if (this.released.has(packet)) return;
    this.released.add(packet);
    packet.ack();
  }

  constructor(options: FramePresenterOptions) {
    this.limits = options.limits ?? IBFV_V2_DEFAULT_LIMITS;
    this.validate = options.validate;
    this.present = options.present;
    this.onError = options.onError;
  }

  push(packet: BrowserViewFramePacket): void {
    if (this.closed) { this.discard(packet); return; }
    let jpeg: ArrayBuffer;
    try {
      this.validate?.(packet.descriptor);
      validateFrameDescriptor(packet.descriptor, {
        streamEpoch: packet.descriptor.stream_epoch,
        targetId: packet.descriptor.target_id,
        displayedTargetId: packet.descriptor.target_id,
        documentGeneration: packet.descriptor.document_generation,
        viewportRevision: packet.descriptor.viewport_revision,
        viewportCssWidth: packet.descriptor.viewport_css_width,
        viewportCssHeight: packet.descriptor.viewport_css_height,
      }, this.limits);
      jpeg = packetJpeg(packet, this.limits);
    } catch (error) {
      this.discard(packet);
      this.onError?.(error instanceof BrowserFrameError ? error : new BrowserFrameError("invalid_envelope", String(error)));
      return;
    }
    const sequence = packet.descriptor.frame_sequence;
    if (this.lastPresentedSequence !== null && sequence <= this.lastPresentedSequence
      || this.active && sequence <= this.active.descriptor.frame_sequence
      || this.pending && sequence <= this.pending.descriptor.frame_sequence) {
      this.discard(packet);
      return;
    }
    const frame = { packet, descriptor: packet.descriptor, jpeg };
    if (this.active) {
      if (this.pending) this.discard(this.pending.packet);
      this.pending = frame;
      return;
    }
    this.active = frame;
    void this.decodeActive();
  }

  private async decodeActive(): Promise<void> {
    const current = this.active;
    if (!current || this.closed) return;
    try {
      const blob = new Blob([current.jpeg], { type: "image/jpeg" });
      let image: ImageBitmap | HTMLImageElement;
      if (typeof globalThis.createImageBitmap === "function") {
        image = await globalThis.createImageBitmap(blob);
      } else {
        const url = URL.createObjectURL(blob);
        const element = new Image();
        element.src = url;
        if (typeof element.decode === "function") await element.decode();
        else await new Promise<void>((resolve, reject) => { element.onload = () => resolve(); element.onerror = () => reject(new Error("Browser JPEG decode failed")); });
        image = element;
        URL.revokeObjectURL(url);
      }
      if (this.closed || this.active !== current) {
        if ("close" in image) image.close();
        this.discard(current.packet);
        return;
      }
      const decodedWidth = "naturalWidth" in image ? image.naturalWidth : image.width;
      const decodedHeight = "naturalHeight" in image ? image.naturalHeight : image.height;
      if (decodedWidth !== current.descriptor.image_width || decodedHeight !== current.descriptor.image_height) throw new BrowserFrameError("invalid_dimensions", "Decoded browser frame dimensions do not match descriptor");
      this.present(image, current.descriptor);
      this.lastPresentedSequence = current.descriptor.frame_sequence;
      this.ack(current.packet);
      if ("close" in image) image.close();
    } catch (error) {
      this.discard(current.packet);
      this.onError?.(error instanceof BrowserFrameError ? error : new Error(error instanceof Error ? error.message : String(error)));
    } finally {
      if (this.active === current) this.active = null;
      if (!this.closed && this.pending) {
        const next = this.pending;
        this.pending = null;
        this.active = next;
        void this.decodeActive();
      }
    }
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    if (this.pending) this.discard(this.pending.packet);
    this.pending = null;
    if (this.active) this.discard(this.active.packet);
    this.active = null;
  }
}
