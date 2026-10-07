import { useCallback, useEffect, useRef, type KeyboardEvent, type PointerEvent } from "react";

export type PanelSplitterProps = {
  label: string;
  controls: string;
  orientation: "vertical" | "horizontal";
  value: number;
  min: number;
  max: number;
  grow: 1 | -1;
  onChange(px: number): void;
  onReset(): void;
  className?: string;
};

/** Controlled details divider: resizing changes only panel geometry, never selection or focus. */
export function PanelSplitter({ label, controls, orientation, value, min, max, grow, onChange, onReset, className }: PanelSplitterProps) {
  const drag = useRef<{ pointer: number; coordinate: number; value: number; element: HTMLDivElement } | null>(null);
  const clamp = (px: number) => Math.round(Math.min(max, Math.max(min, px)));
  const endDrag = useCallback(() => {
    const current = drag.current;
    if (!current) return;
    drag.current = null;
    document.body.classList.remove("is-resizing-panes");
    if (current.element.hasPointerCapture(current.pointer)) current.element.releasePointerCapture(current.pointer);
  }, []);
  useEffect(() => endDrag, [endDrag]);

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || drag.current) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { pointer: event.pointerId, coordinate: orientation === "vertical" ? event.clientX : event.clientY, value, element: event.currentTarget };
    document.body.classList.add("is-resizing-panes");
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    const coordinate = orientation === "vertical" ? event.clientX : event.clientY;
    onChange(clamp(current.value + (coordinate - current.coordinate) * grow));
  };
  const end = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current?.pointer === event.pointerId) endDrag();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.ctrlKey || event.altKey || event.metaKey || event.nativeEvent.isComposing) return;
    const backward = orientation === "vertical" ? "ArrowLeft" : "ArrowUp";
    const forward = orientation === "vertical" ? "ArrowRight" : "ArrowDown";
    if (event.key === backward || event.key === forward) {
      event.preventDefault();
      onChange(clamp(value + (event.key === forward ? 1 : -1) * grow * (event.shiftKey ? 48 : 16)));
    } else if (event.key === "Home") {
      event.preventDefault();
      onReset();
    }
  };
  return <div className={`supervisor-panel-splitter${className ? ` ${className}` : ""}`} role="separator" aria-orientation={orientation}
    aria-label={label} aria-controls={controls} aria-valuemin={min} aria-valuemax={max} aria-valuenow={value} aria-valuetext={`${value} px`}
    title="Drag to resize · double-click to reset" tabIndex={0} onPointerDown={onPointerDown} onPointerMove={onPointerMove}
    onPointerUp={end} onPointerCancel={end} onLostPointerCapture={end} onDoubleClick={onReset} onKeyDown={onKeyDown} />;
}
