import { describe, expect, it } from "vitest";
import type { BrowserViewFrameDescriptor } from "../../protocol/generated/v1";
import { createBrowserTransform } from "./transform";

const frame: BrowserViewFrameDescriptor = {
  target_id: "target",
  stream_epoch: 1,
  frame_sequence: 1,
  document_generation: 1,
  viewport_revision: 1,
  image_width: 1600,
  image_height: 1000,
  viewport_css_width: 1185,
  viewport_css_height: 750,
  viewport_offset_x: 0,
  viewport_offset_y: 0,
  scroll_x: 20,
  scroll_y: 300,
  capture_timestamp_micros: 1,
  jpeg_length: 1,
};

describe("browser coordinate transform", () => {
  it("maps input when compositor pixels and CSS viewport have different aspect ratios", () => {
    const transform = createBrowserTransform(frame, { left: 0, top: 0, width: 160, height: 100 });

    expect(transform?.clientToImage(80, 50)).toEqual({ x: 800, y: 500 });
    expect(transform?.clientToViewport(80, 50)).toEqual({ x: 592.5, y: 375 });
    expect(transform?.clientToDocument(80, 50)).toEqual({ x: 612.5, y: 675 });
  });
});
