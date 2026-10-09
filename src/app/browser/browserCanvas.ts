import type { BrowserPoint, BrowserViewDraftAnnotation, BrowserViewSnapshot } from "../../protocol/generated/v1";
import type { BrowserViewFramePacket } from "../../client/CockpitClient";
import { kindFor } from "./browserPaneModel";

// WebKitGTK composites GPU-backed canvases as separate layers and can show a
// stale or partly swapped buffer for a frame whenever other panes repaint.
// A CPU-backed, opaque canvas is painted with the rest of the page instead.
export const frameContext = (canvas: HTMLCanvasElement | null | undefined): CanvasRenderingContext2D | null =>
  canvas?.getContext("2d", { alpha: false, willReadFrequently: true }) ?? null;

export async function composeAnnotatedPng(source: HTMLCanvasElement | null, snapshot: BrowserViewSnapshot | null, annotationsToPaint: BrowserViewDraftAnnotation[], pinnedDescriptor: BrowserViewFramePacket["descriptor"]): Promise<string | null> {
    if (!source || !snapshot?.document || !snapshot.viewport) return null;
    // The live canvas keeps the viewport's backing size; captures keep the pinned frame's pixels.
    const output = document.createElement("canvas"); output.width = pinnedDescriptor.image_width; output.height = pinnedDescriptor.image_height;
    const drawing = output.getContext("2d"); if (!drawing) return null;
    drawing.drawImage(source, 0, 0, output.width, output.height);
    const map = (point: BrowserPoint): BrowserPoint => ({
      x: (point.x - pinnedDescriptor.scroll_x - pinnedDescriptor.viewport_offset_x) / pinnedDescriptor.viewport_css_width * output.width,
      y: (point.y - pinnedDescriptor.scroll_y - pinnedDescriptor.viewport_offset_y) / pinnedDescriptor.viewport_css_height * output.height,
    });
    for (const annotation of annotationsToPaint) {
      drawing.strokeStyle = annotation.color; drawing.fillStyle = annotation.color; drawing.lineWidth = kindFor(annotation) === "freehand" ? 3 : 2;
      drawing.lineCap = "round"; drawing.lineJoin = "round";
      if (kindFor(annotation) === "freehand") {
        const points = annotation.points.map(map); if (points.length < 2) continue;
        drawing.beginPath(); drawing.moveTo(points[0].x, points[0].y); for (const point of points.slice(1)) drawing.lineTo(point.x, point.y); drawing.stroke();
      } else if (annotation.bounds) {
        const start = map({ x: annotation.bounds.x, y: annotation.bounds.y }); const end = map({ x: annotation.bounds.x + annotation.bounds.width, y: annotation.bounds.y + annotation.bounds.height });
        drawing.globalAlpha = 0.09; drawing.fillRect(start.x, start.y, end.x - start.x, end.y - start.y); drawing.globalAlpha = 1; drawing.strokeRect(start.x, start.y, end.x - start.x, end.y - start.y);
      }
      const anchor = annotation.bounds ? { x: annotation.bounds.x, y: annotation.bounds.y } : annotation.points[0];
      const label = annotation.comment?.trim().replace(/\s+/g, " ").slice(0, 240);
      if (anchor && label) {
        const point = map(anchor);
        const x = Math.max(2, Math.min(output.width - 2, point.x + 8));
        const y = Math.max(14, Math.min(output.height - 2, point.y + 16));
        drawing.font = "600 14px sans-serif"; drawing.textBaseline = "alphabetic"; drawing.lineWidth = 4;
        drawing.strokeStyle = "#0c1016"; drawing.strokeText(label, x, y); drawing.fillStyle = annotation.color; drawing.fillText(label, x, y);
      }
    }
    const blob = await new Promise<Blob | null>((resolve) => output.toBlob(resolve, "image/png"));
    if (!blob) return null;
    const data = await new Promise<string>((resolve, reject) => { const reader = new FileReader(); reader.onload = () => typeof reader.result === "string" ? resolve(reader.result) : reject(new Error("PNG data is unavailable")); reader.onerror = () => reject(reader.error ?? new Error("Could not read PNG")); reader.readAsDataURL(blob); });
    const comma = data.indexOf(","); return comma >= 0 ? data.slice(comma + 1) : null;
}
