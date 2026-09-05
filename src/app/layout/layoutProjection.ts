import type { LayoutRect, ResourceMutationRequest, TabLayout } from "../../protocol/generated/v1";

export type ResizeHandle = {
  id: string;
  paneId: string;
  axis: "x" | "y";
  negativeDirection: "left" | "up";
  positiveDirection: "right" | "down";
  coordinate: number;
  start: number;
  length: number;
};

const EPSILON = 0.000001;
const edge = (rect: LayoutRect, axis: "x" | "y") => axis === "x" ? rect.x + rect.width : rect.y + rect.height;
const overlap = (aStart: number, aLength: number, bStart: number, bLength: number) => {
  const start = Math.max(aStart, bStart);
  return { start, length: Math.min(aStart + aLength, bStart + bLength) - start };
};

export function deriveResizeHandles(layout: TabLayout | undefined): ResizeHandle[] {
  if (!layout || layout.zoomed) return [];
  const handles: ResizeHandle[] = [];
  for (let leftIndex = 0; leftIndex < layout.panes.length; leftIndex += 1) {
    for (let rightIndex = leftIndex + 1; rightIndex < layout.panes.length; rightIndex += 1) {
      const first = layout.panes[leftIndex];
      const second = layout.panes[rightIndex];
      const verticalOverlap = overlap(first.rect.y, first.rect.height, second.rect.y, second.rect.height);
      if (verticalOverlap.length > EPSILON) {
        const firstBefore = Math.abs(edge(first.rect, "x") - second.rect.x) <= EPSILON;
        const secondBefore = Math.abs(edge(second.rect, "x") - first.rect.x) <= EPSILON;
        if (firstBefore || secondBefore) {
          const pane = firstBefore ? first : second;
          handles.push({ id: `x:${pane.pane_id}:${firstBefore ? second.pane_id : first.pane_id}`, paneId: pane.pane_id, axis: "x", negativeDirection: "left", positiveDirection: "right", coordinate: edge(pane.rect, "x"), start: verticalOverlap.start, length: verticalOverlap.length });
        }
      }
      const horizontalOverlap = overlap(first.rect.x, first.rect.width, second.rect.x, second.rect.width);
      if (horizontalOverlap.length > EPSILON) {
        const firstBefore = Math.abs(edge(first.rect, "y") - second.rect.y) <= EPSILON;
        const secondBefore = Math.abs(edge(second.rect, "y") - first.rect.y) <= EPSILON;
        if (firstBefore || secondBefore) {
          const pane = firstBefore ? first : second;
          handles.push({ id: `y:${pane.pane_id}:${firstBefore ? second.pane_id : first.pane_id}`, paneId: pane.pane_id, axis: "y", negativeDirection: "up", positiveDirection: "down", coordinate: edge(pane.rect, "y"), start: horizontalOverlap.start, length: horizontalOverlap.length });
        }
      }
    }
  }
  return handles;
}

export function resizeRequest(handle: ResizeHandle, pixelDelta: number, canvasPixels: number): ResourceMutationRequest | null {
  if (!Number.isFinite(pixelDelta) || !Number.isFinite(canvasPixels) || canvasPixels <= 0 || Math.abs(pixelDelta) < 1) return null;
  const amount = Math.abs(pixelDelta) / canvasPixels;
  if (!Number.isFinite(amount) || amount <= 0) return null;
  return { type: "pane_resize", pane_id: handle.paneId, direction: pixelDelta < 0 ? handle.negativeDirection : handle.positiveDirection, amount };
}

export function projectedPaneIds(paneIds: string[], layout: TabLayout | undefined, fallbackPaneId: string | null): string[] {
  if (!layout?.zoomed) return paneIds;
  const focusedPaneId = paneIds.includes(layout.focused_pane_id ?? "")
    ? layout.focused_pane_id
    : paneIds.includes(fallbackPaneId ?? "") ? fallbackPaneId : paneIds[0] ?? null;
  return focusedPaneId ? [focusedPaneId] : [];
}

export function projectedPaneRect(layout: TabLayout | undefined, paneId: string): LayoutRect | undefined {
  if (!layout) return undefined;
  if (layout.zoomed) return layout.area;
  return layout.panes.find((candidate) => candidate.pane_id === paneId)?.rect;
}

export function tabDropInsertionIndex(sourceIndex: number, targetIndex: number, afterTarget: boolean): number | null {
  if (sourceIndex < 0 || targetIndex < 0 || sourceIndex === targetIndex) return null;
  const insertionIndex = targetIndex + (afterTarget ? 1 : 0);
  const finalIndex = insertionIndex > sourceIndex ? insertionIndex - 1 : insertionIndex;
  return finalIndex === sourceIndex ? null : insertionIndex;
}
