import { useRef, type KeyboardEvent, type PointerEvent } from "react";

interface Props {
  label: string;
  /** Element whose height this splitter controls; measured when a drag or key press starts. */
  target: (splitter: HTMLElement) => HTMLElement | null;
  /** +1 when dragging down grows the target (splitter below it), -1 when dragging up grows it (splitter above it). */
  grow: 1 | -1;
  min: number;
  max: number;
  onChange: (height: number) => void;
  onReset: () => void;
  className?: string;
}

/** Full-width horizontal divider, the same grab-anywhere-on-the-border resize as pane splits. */
export function RowSplitter({ label, target, grow, min, max, onChange, onReset, className }: Props) {
  const drag = useRef<{ pointer: number; y: number; height: number } | null>(null);
  const clamp = (height: number) => Math.round(Math.min(max, Math.max(min, height)));
  const measure = (splitter: HTMLElement) => target(splitter)?.getBoundingClientRect().height ?? min;
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { pointer: event.pointerId, y: event.clientY, height: measure(event.currentTarget) };
    document.body.classList.add("is-resizing-panes");
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    onChange(clamp(current.height + (event.clientY - current.y) * grow));
  };
  const end = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current?.pointer !== event.pointerId) return;
    drag.current = null;
    document.body.classList.remove("is-resizing-panes");
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 48 : 16;
    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      event.preventDefault();
      const direction = (event.key === "ArrowDown" ? 1 : -1) * grow;
      onChange(clamp(measure(event.currentTarget) + direction * step));
    } else if (event.key === "Home") {
      event.preventDefault();
      onReset();
    }
  };
  return <div className={`supervisor-row-splitter ${className ?? ""}`} role="separator" aria-orientation="horizontal" aria-label={label} title="Drag to resize · double-click to reset"
    tabIndex={0} onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={end} onPointerCancel={end} onLostPointerCapture={end}
    onDoubleClick={onReset} onKeyDown={onKeyDown} />;
}
