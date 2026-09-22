import { afterEach, describe, expect, it, vi } from "vitest";
import type { BrowserViewFrameDescriptor } from "../../protocol/generated/v1";
import type { BrowserViewFramePacket } from "../../client/CockpitClient";
import { FramePresenter } from "./framePresenter";

const jpeg = Uint8Array.from([
  0xff, 0xd8,
  0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01,
  0xff, 0xd9,
]);

function descriptor(viewportRevision: number, frameSequence: number): BrowserViewFrameDescriptor {
  return {
    target_id: "target-1", stream_epoch: 1, frame_sequence: frameSequence,
    document_generation: 1, viewport_revision: viewportRevision,
    image_width: 1, image_height: 1, jpeg_length: jpeg.byteLength,
    viewport_css_width: 800, viewport_css_height: 600,
    viewport_offset_x: 0, viewport_offset_y: 0, scroll_x: viewportRevision, scroll_y: 0,
    capture_timestamp_micros: frameSequence,
  };
}

function packet(frame: BrowserViewFrameDescriptor): BrowserViewFramePacket {
  return { descriptor: frame, jpeg: jpeg.slice().buffer, ack: vi.fn(), discard: vi.fn() };
}

describe("FramePresenter metadata ordering", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("holds one newer frame until matching metadata, then presents it", async () => {
    let currentRevision = 1;
    const present = vi.fn();
    vi.stubGlobal("createImageBitmap", vi.fn(async () => ({ width: 1, height: 1, close: vi.fn() })));
    const presenter = new FramePresenter({
      shouldDefer: (frame) => frame.viewport_revision > currentRevision,
      isExpectedStale: (frame) => frame.viewport_revision < currentRevision,
      present,
    });
    const next = packet(descriptor(2, 2));

    presenter.push(next);
    expect(present).not.toHaveBeenCalled();
    expect(next.ack).not.toHaveBeenCalled();
    expect(next.discard).not.toHaveBeenCalled();

    currentRevision = 2;
    presenter.revalidate();
    await new Promise<void>((resolve) => queueMicrotask(resolve));
    expect(present).toHaveBeenCalledTimes(1);
    expect(next.ack).toHaveBeenCalledTimes(1);
    presenter.close();
  });

  it("discards an older frame instead of retaining it as a deferred candidate", () => {
    let currentRevision = 2;
    const old = packet(descriptor(1, 1));
    const presenter = new FramePresenter({
      shouldDefer: (frame) => frame.viewport_revision > currentRevision,
      isExpectedStale: (frame) => frame.viewport_revision < currentRevision,
      present: vi.fn(),
    });

    presenter.push(old);
    expect(old.discard).toHaveBeenCalledTimes(1);
    presenter.close();
  });
});
