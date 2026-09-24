// Annotation tool icons and the colour picker used by the browser pane toolbar.
import { useCallback, useEffect, useRef, useState } from "react";

export const COLORS = ["#d62828", "#1769aa", "#2a9d55", "#c27803", "#7c3aed"] as const;
export const annotationIconPaths = {
  browse: "M2 1.5 14 9.5 9.2 10.6 12 14.8 10.1 16 7.3 11.8 4.5 14.8Z",
  select: "M14.5 8a6.5 6.5 0 1 1-13 0 6.5 6.5 0 0 1 13 0Zm-9 0 2.1 2.1L11.8 6.5",
  freehand: "M3 12c1.5-4 3-7 5-7 1.4 0 1.4 2 2.5 2 1 0 1.5-1.5 2.5-3",
  element: "M8 1v3m0 8v3M1 8h3m8 0h3M12 8a4 4 0 1 1-8 0 4 4 0 0 1 8 0Z",
  region: "M2.5 2.5h11v11h-11Z",
  remove: "M3 4.5h10M6 2.5h4M5 4.5l.6 9h4.8l.6-9M7 7v4M9 7v4",
  notes: "M14 11a3 3 0 0 1-3 3H6l-3 2v-8a3 3 0 0 1 3-3h5a3 3 0 0 1 3 3Z",
  feedback: "M14.5 1.5 7 9m7.5-7.5-4.5 13L7 9 1.5 6.5Z",
  expand: "M6 1H1v5m8-5h5v5M1 9v5h5m8-5v5H9",
} as const;
export type AnnotationIconName = keyof typeof annotationIconPaths;
export const AnnotationIcon = ({ name }: { name: AnnotationIconName }) => <svg className={`browser-annotation-icon browser-annotation-icon-${name}`} viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.35" strokeLinecap="round" strokeLinejoin="round"><path d={annotationIconPaths[name]} /></svg>;
export function BrowserColorPicker({ color, onChange }: { color: string; onChange: (color: typeof COLORS[number]) => void }) {
  const pickerRef = useRef<HTMLDetailsElement>(null);
  const [open, setOpen] = useState(false);
  const close = useCallback((restoreFocus: boolean) => {
    const picker = pickerRef.current;
    if (!picker) return;
    picker.open = false;
    if (restoreFocus) picker.querySelector("summary")?.focus();
  }, []);
  useEffect(() => {
    if (!open) return;
    // A dismissing click outside the popover must not also reach the live page.
    const onPointerDown = (event: globalThis.PointerEvent) => {
      const target = event.target as Node | null;
      if (!target || pickerRef.current?.contains(target)) return;
      if (target instanceof Element && target.closest(".browser-surface")) {
        event.preventDefault();
        event.stopPropagation();
        globalThis.document.addEventListener("pointerup", (up) => up.stopPropagation(), { capture: true, once: true });
      }
      close(false);
    };
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      close(true);
    };
    globalThis.document.addEventListener("pointerdown", onPointerDown, true);
    globalThis.document.addEventListener("keydown", onKeyDown, true);
    return () => {
      globalThis.document.removeEventListener("pointerdown", onPointerDown, true);
      globalThis.document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open, close]);
  return <details ref={pickerRef} className="browser-color-picker" onToggle={(event) => setOpen(event.currentTarget.open)}>
    <summary aria-label={`Annotation color, ${color}`} title={`Annotation color: ${color}`}><span style={{ backgroundColor: color }} /></summary>
    <div className="browser-color-options" role="group" aria-label="Annotation color">
      {COLORS.map((candidate) => <button key={candidate} type="button" className={color === candidate ? "is-active" : undefined} aria-label={`Use ${candidate} annotation color`} aria-pressed={color === candidate} title={`Use ${candidate} annotation color`} style={{ backgroundColor: candidate }} onClick={() => { onChange(candidate); close(true); }} />)}
    </div>
  </details>;
}

const TOOLS = ["browse", "select", "freehand", "element", "region"] as const;

export function AnnotationToolButtons({ tool, onChange }: { tool: typeof TOOLS[number]; onChange: (tool: typeof TOOLS[number]) => void }) {
  return <>{TOOLS.map((candidate) => {
    const label = candidate[0].toUpperCase() + candidate.slice(1);
    return <button key={candidate} type="button" className={tool === candidate ? "is-active" : undefined} aria-pressed={tool === candidate} aria-label={label} title={`${label} tool`} onClick={() => onChange(candidate)}>
      <AnnotationIcon name={candidate} />
    </button>;
  })}</>;
}
