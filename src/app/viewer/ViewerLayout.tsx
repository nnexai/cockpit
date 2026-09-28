import { useCallback, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from "react";

const TREE_WIDTH_KEY = "cockpit.viewer.treeWidth";
const WRAP_KEY = "cockpit.viewer.wrap";
const DEFAULT_TREE_WIDTH = 176;
const MIN_TREE_WIDTH = 112;
const MAX_TREE_WIDTH = 640;
const CHANGE_EVENT = "cockpit-viewer-layout";

function read(key: string): string | null {
  try { return globalThis.localStorage?.getItem(key) ?? null; } catch { return null; }
}

function write(key: string, value: string) {
  try { globalThis.localStorage?.setItem(key, value); } catch { /* preference only */ }
  window.dispatchEvent(new CustomEvent(CHANGE_EVENT, { detail: key }));
}

function clampWidth(value: number): number {
  return Math.round(Math.max(MIN_TREE_WIDTH, Math.min(MAX_TREE_WIDTH, value)));
}

function storedWidth(): number {
  const value = Number(read(TREE_WIDTH_KEY));
  return Number.isFinite(value) && value > 0 ? clampWidth(value) : DEFAULT_TREE_WIDTH;
}

/** One preference shared by every viewer pane, kept in step across panes. */
function useSharedPreference<T>(key: string, load: () => T): [T, (next: T, persist?: boolean) => void] {
  const [value, setValue] = useState(load);
  useEffect(() => {
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
export function useTreeWidth() {
  const [width, setWidth] = useSharedPreference(TREE_WIDTH_KEY, storedWidth);
  const style = { "--viewer-tree-width": `${width}px` } as CSSProperties;
  return { width, setWidth, style };
}

/** Splitter left edge, in step with the grid column in viewer.css. */
const splitterLeft = (width: number) => `calc(min(${width}px, 60%) - 3px)`;

export function TreeSplitter({ width, onChange }: { width: number; onChange: (width: number, persist?: boolean) => void }) {
  const drag = useRef<{ pointer: number; startX: number; startWidth: number; width: number; frame: number } | null>(null);
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { pointer: event.pointerId, startX: event.clientX, startWidth: width, width, frame: 0 };
  };
  // While dragging, write the width straight to the layout, once per frame: re-rendering the
  // viewer (and a tree of thousands of rows) on every pointer move made resizing lag.
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    current.width = clampWidth(current.startWidth + event.clientX - current.startX);
    if (current.frame) return;
    const splitter = event.currentTarget;
    current.frame = requestAnimationFrame(() => {
      current.frame = 0;
      splitter.parentElement?.style.setProperty("--viewer-tree-width", `${current.width}px`);
      splitter.style.left = splitterLeft(current.width);
      splitter.setAttribute("aria-valuenow", String(current.width));
    });
  };
  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    cancelAnimationFrame(current.frame);
    drag.current = null;
    onChange(current.width);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 48 : 16;
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      onChange(clampWidth(width + (event.key === "ArrowRight" ? step : -step)));
    } else if (event.key === "Home") {
      event.preventDefault();
      onChange(DEFAULT_TREE_WIDTH);
    }
  };
  return <div className="viewer-splitter" role="separator" aria-orientation="vertical" aria-label="Resize file list" title="Drag to resize · double-click to reset"
    tabIndex={0} aria-valuemin={MIN_TREE_WIDTH} aria-valuemax={MAX_TREE_WIDTH} aria-valuenow={width} style={{ left: splitterLeft(width) }}
    onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerUp}
    onDoubleClick={() => onChange(DEFAULT_TREE_WIDTH)} onKeyDown={onKeyDown} />;
}
