import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

export function CommentEditor({ label, onDismiss, children }: { label: string; onDismiss: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  const [sheet, setSheet] = useState(false);
  useLayoutEffect(() => {
    const dialog = ref.current;
    const pane = dialog?.closest(".context-viewer, .review-pane");
    if (!dialog || !pane) return;
    const resize = () => setSheet(window.innerWidth <= 600 || pane.getBoundingClientRect().width <= 520);
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(pane);
    window.addEventListener("resize", resize);
    return () => { observer.disconnect(); window.removeEventListener("resize", resize); };
  }, []);
  useLayoutEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    const active = document.activeElement;
    const textarea = active instanceof HTMLTextAreaElement && dialog.contains(active) ? active : null;
    const selection = textarea ? [textarea.selectionStart, textarea.selectionEnd] : null;
    if (dialog.open) dialog.close();
    if (sheet) dialog.showModal(); else dialog.show();
    if (active instanceof HTMLElement && dialog.contains(active)) active.focus({ preventScroll: true });
    if (textarea && selection) textarea.setSelectionRange(selection[0], selection[1]);
  }, [sheet]);
  return <dialog ref={ref} className={`comment-editor${sheet ? " is-sheet" : ""}`} aria-label={label} onCancel={(event) => { event.preventDefault(); event.stopPropagation(); onDismiss(); }}>{children}</dialog>;
}
