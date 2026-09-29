import type { ReactNode } from "react";
import { trapModalTab, useModalFocus } from "./modal";

/**
 * A modal dialog section: focus enters on mount, Tab stays inside, Escape
 * dismisses and focus returns to the opener. Mount it only while open.
 */
export function ModalSection({ className, labelledBy, onDismiss, children }: { className: string; labelledBy: string; onDismiss: () => void; children: ReactNode }) {
  const ref = useModalFocus<HTMLElement>(onDismiss);
  return <section ref={ref} className={className} role="dialog" aria-modal="true" aria-labelledby={labelledBy} onKeyDown={(event) => trapModalTab(event, ref.current)}>{children}</section>;
}
