import type { BrowserViewFrameDescriptor } from "../../protocol/generated/v1";

export interface PaintedRect {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

export interface BrowserCoordinate {
  readonly x: number;
  readonly y: number;
}

export interface BrowserTransform {
  readonly frame: BrowserViewFrameDescriptor;
  readonly paintedRect: PaintedRect;
  readonly clientToImage: (clientX: number, clientY: number) => BrowserCoordinate | null;
  readonly clientToViewport: (clientX: number, clientY: number) => BrowserCoordinate | null;
  readonly clientToDocument: (clientX: number, clientY: number) => BrowserCoordinate | null;
}

function validRect(rect: PaintedRect): boolean {
  return Number.isFinite(rect.left) && Number.isFinite(rect.top) && Number.isFinite(rect.width) && Number.isFinite(rect.height) && rect.width > 0 && rect.height > 0;
}

function pointInRect(rect: PaintedRect, x: number, y: number): boolean {
  return validRect(rect) && Number.isFinite(x) && Number.isFinite(y) && x >= rect.left && y >= rect.top && x < rect.left + rect.width && y < rect.top + rect.height;
}

/**
 * Build the one transform shared by browser input and future annotation layers.
 * Coordinates in the letterbox, toolbar, or outside the painted image are rejected.
 */
export function createBrowserTransform(frame: BrowserViewFrameDescriptor, paintedRect: PaintedRect): BrowserTransform | null {
  if (!validRect(paintedRect)
    || !Number.isInteger(frame.image_width) || !Number.isInteger(frame.image_height)
    || frame.image_width <= 0 || frame.image_height <= 0
    || !Number.isFinite(frame.viewport_css_width) || !Number.isFinite(frame.viewport_css_height)
    || frame.viewport_css_width <= 0 || frame.viewport_css_height <= 0
    || !Number.isFinite(frame.viewport_offset_x) || !Number.isFinite(frame.viewport_offset_y)
    || !Number.isFinite(frame.scroll_x) || !Number.isFinite(frame.scroll_y)) return null;
  const clientToNormalized = (clientX: number, clientY: number): BrowserCoordinate | null => {
    if (!pointInRect(paintedRect, clientX, clientY)) return null;
    return { x: (clientX - paintedRect.left) / paintedRect.width, y: (clientY - paintedRect.top) / paintedRect.height };
  };
  const clientToImage = (clientX: number, clientY: number): BrowserCoordinate | null => {
    const normalized = clientToNormalized(clientX, clientY);
    if (!normalized) return null;
    return { x: normalized.x * frame.image_width, y: normalized.y * frame.image_height };
  };
  const clientToViewport = (clientX: number, clientY: number): BrowserCoordinate | null => {
    const normalized = clientToNormalized(clientX, clientY);
    if (!normalized) return null;
    return { x: frame.viewport_offset_x + normalized.x * frame.viewport_css_width, y: frame.viewport_offset_y + normalized.y * frame.viewport_css_height };
  };
  const clientToDocument = (clientX: number, clientY: number): BrowserCoordinate | null => {
    const viewport = clientToViewport(clientX, clientY);
    if (!viewport) return null;
    return { x: viewport.x + frame.scroll_x, y: viewport.y + frame.scroll_y };
  };
  return { frame, paintedRect, clientToImage, clientToViewport, clientToDocument };
}
