import { useCallback, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from "react";

const WRAP_KEY = "cockpit.viewer.wrap";
const MAX_TREE_WIDTH = 640;
const CHANGE_EVENT = "cockpit-viewer-layout";

/** A file list's stored width, default and lower bound; Review and Context share one, the Library has its own. */
export type TreeLayout = { key: string; fallback: number; min: number };
export const VIEWER_TREE: TreeLayout = { key: "cockpit.viewer.treeWidth", fallback: 176, min: 112 };
export const LIBRARY_TREE: TreeLayout = { key: "cockpit.library.treeWidth", fallback: 296, min: 200 };

function read(key: string): string | null {
  try { return globalThis.localStorage?.getItem(key) ?? null; } catch { return null; }
}

function write(key: string, value: string) {
  try { globalThis.localStorage?.setItem(key, value); } catch { /* preference only */ }
  window.dispatchEvent(new CustomEvent(CHANGE_EVENT, { detail: key }));
}

function clampWidth(layout: TreeLayout, value: number): number {
  return Math.round(Math.max(layout.min, Math.min(MAX_TREE_WIDTH, value)));
}

function storedWidth(layout: TreeLayout): number {
  const value = Number(read(layout.key));
  return Number.isFinite(value) && value > 0 ? clampWidth(layout, value) : layout.fallback;
}

/** One preference shared by every viewer pane, kept in step across panes. */
function useSharedPreference<T>(key: string, load: () => T): [T, (next: T, persist?: boolean) => void] {
  const [value, setValue] = useState(load);
  useEffect(() => {
    // A viewer that changes surface (a pane switching to the Library root) rereads its own preference.
    setValue(load());
    const reload = (event: Event) => { if (!(event instanceof CustomEvent) || event.detail === key) setValue(load()); };
    window.addEventListener(CHANGE_EVENT, reload);
    return () => window.removeEventListener(CHANGE_EVENT, reload);
  }, [key, load]);
  const update = useCallback((next: T, persist = true) => {
    setValue(next);
    if (persist) write(key, String(next));
  }, [key]);
  return [value, update];
}

const loadWrap = () => read(WRAP_KEY) !== "false";

/** Whether long source and diff lines wrap instead of scrolling sideways. */
export function useWrapPreference(): [boolean, () => void] {
  const [wrap, setWrap] = useSharedPreference(WRAP_KEY, loadWrap);
  return [wrap, useCallback(() => setWrap(!wrap), [setWrap, wrap])];
}

/** Width of the file list beside a viewer's content, set by its splitter. */
export function useTreeWidth(layout: TreeLayout = VIEWER_TREE) {
  const load = useCallback(() => storedWidth(layout), [layout]);
  const [width, setWidth] = useSharedPreference(layout.key, load);
  const style = { "--viewer-tree-width": `${width}px` } as CSSProperties;
  return { width, setWidth, style };
}

/** Splitter left edge, in step with the grid column in viewer.css. */
const splitterLeft = (width: number) => `calc(min(${width}px, 60%) - 3px)`;

/** The width the list actually has: the grid column caps a stored width at 60% of a narrow viewer, so the splitter's own centre is the truth. */
function shownWidth(layout: TreeLayout, splitter: HTMLElement, stored: number): number {
  return splitter.offsetWidth > 0 ? clampWidth(layout, splitter.offsetLeft + splitter.offsetWidth / 2) : stored;
}

function applyWidth(splitter: HTMLElement, width: number) {
  splitter.parentElement?.style.setProperty("--viewer-tree-width", `${width}px`);
  splitter.style.left = splitterLeft(width);
  splitter.setAttribute("aria-valuenow", String(width));
}

export function TreeSplitter({ width, onChange, layout = VIEWER_TREE }: { width: number; onChange: (width: number, persist?: boolean) => void; layout?: TreeLayout }) {
  const drag = useRef<{ pointer: number; startX: number; startWidth: number; width: number; frame: number } | null>(null);
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    const startWidth = shownWidth(layout, event.currentTarget, width);
    drag.current = { pointer: event.pointerId, startX: event.clientX, startWidth, width: startWidth, frame: 0 };
  };
  // While dragging, write the width straight to the layout, once per frame: re-rendering the
  // viewer (and a tree of thousands of rows) on every pointer move made resizing lag.
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    current.width = clampWidth(layout, current.startWidth + event.clientX - current.startX);
    if (current.frame) return;
    const splitter = event.currentTarget;
    current.frame = requestAnimationFrame(() => {
      current.frame = 0;
      applyWidth(splitter, current.width);
    });
  };
  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    cancelAnimationFrame(current.frame);
    drag.current = null;
    // The pending frame is dropped, so write the final width now: React sees no change when the drag ends where the stored width already was.
    applyWidth(event.currentTarget, current.width);
    onChange(current.width);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 48 : 16;
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      onChange(clampWidth(layout, shownWidth(layout, event.currentTarget, width) + (event.key === "ArrowRight" ? step : -step)));
    } else if (event.key === "Home") {
      event.preventDefault();
      onChange(layout.fallback);
    }
  };
  return <div className="viewer-splitter" role="separator" aria-orientation="vertical" aria-label="Resize file list" title="Drag to resize · double-click to reset"
    tabIndex={0} aria-valuemin={layout.min} aria-valuemax={MAX_TREE_WIDTH} aria-valuenow={width} style={{ left: splitterLeft(width) }}
    onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerUp}
    onDoubleClick={() => onChange(layout.fallback)} onKeyDown={onKeyDown} />;
}
