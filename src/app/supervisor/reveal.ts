export type Insets = { top: number; right: number; bottom: number; left: number };
export type ScrollOffset = { left: number; top: number };
type Rect = { left: number; top: number; width: number; height: number };

/** Minimal per-axis movement; oversized targets align their start, not their end. */
export function nearestScroll(viewport: DOMRectReadOnly | Rect, target: Rect, insets: Insets, current: ScrollOffset): ScrollOffset {
  const left = viewport.left + insets.left, top = viewport.top + insets.top;
  const right = viewport.left + viewport.width - insets.right, bottom = viewport.top + viewport.height - insets.bottom;
  const dx = target.width > right - left || target.left < left ? target.left - left
    : target.left + target.width > right ? target.left + target.width - right : 0;
  const dy = target.height > bottom - top || target.top < top ? target.top - top
    : target.top + target.height > bottom ? target.top + target.height - bottom : 0;
  return { left: Math.max(0, current.left + dx), top: Math.max(0, current.top + dy) };
}

// The client box excludes the border and scrollbars, which cannot display a target.
function scrollportRect(scroller: HTMLElement): Rect {
  const rect = scroller.getBoundingClientRect();
  return { left: rect.left + scroller.clientLeft, top: rect.top + scroller.clientTop, width: scroller.clientWidth, height: scroller.clientHeight };
}

/** Scroll this port alone; callers decide independently whether DOM focus should move. */
export function revealNearest(scroller: HTMLElement, element: HTMLElement, insets: Partial<Insets> = {}): boolean {
  if (!scroller.contains(element)) return false;
  const viewport = scrollportRect(scroller);
  if (viewport.width <= 0 || viewport.height <= 0) return false;
  const current = readOffset(scroller);
  const next = nearestScroll(viewport, element.getBoundingClientRect(), { top: 0, right: 0, bottom: 0, left: 0, ...insets }, current);
  if (next.left === current.left && next.top === current.top) return false;
  writeOffset(scroller, next);
  return scroller.scrollLeft !== current.left || scroller.scrollTop !== current.top;
}

export function isCovered(scroller: HTMLElement, element: HTMLElement, insets: Partial<Insets> = {}): boolean {
  if (!scroller.contains(element)) return false;
  const viewport = scrollportRect(scroller);
  if (viewport.width <= 0 || viewport.height <= 0) return false;
  const target = element.getBoundingClientRect();
  return target.left < viewport.left + (insets.left ?? 0) || target.top < viewport.top + (insets.top ?? 0)
    || target.left + target.width > viewport.left + viewport.width - (insets.right ?? 0)
    || target.top + target.height > viewport.top + viewport.height - (insets.bottom ?? 0);
}

export const readOffset = (element: HTMLElement): ScrollOffset => ({ left: element.scrollLeft, top: element.scrollTop });
export const writeOffset = (element: HTMLElement, offset: ScrollOffset) => { element.scrollLeft = offset.left; element.scrollTop = offset.top; };
