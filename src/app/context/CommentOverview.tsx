import { useLayoutEffect, useRef, type ReactNode } from "react";

export function CommentOverview({ onDismiss, children }: { onDismiss: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    dialog.showModal();
    return () => dialog.close();
  }, []);
  return <dialog ref={ref} className="comment-overview" aria-label="Comments" onCancel={(event) => { event.preventDefault(); event.stopPropagation(); onDismiss(); }} onPointerDown={(event) => {
    if (event.target !== event.currentTarget) return;
    const rect = event.currentTarget.getBoundingClientRect();
    if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) onDismiss();
  }}>{children}</dialog>;
}
