import { useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent, type RefObject } from "react";

const FOCUSABLE = "button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex='-1'])";

export function nextModalFocusIndex(current: number, count: number, shiftKey: boolean): number {
  if (count <= 0) return -1;
  if (current < 0) return shiftKey ? count - 1 : 0;
  return (current + (shiftKey ? count - 1 : 1)) % count;
}

/**
 * Focus the first control on mount, close on Escape, and give focus back to the
 * opener on unmount. Escape is a window (bubble) listener, so a menu or picker
 * inside the dialog that handles its own Escape closes first.
 */
export function useModalFocus<T extends HTMLElement>(onDismiss: () => void): RefObject<T | null> {
  const ref = useRef<T | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const dismissRef = useRef(onDismiss);
  dismissRef.current = onDismiss;
  useEffect(() => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    ref.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) {
        event.preventDefault();
        dismissRef.current();
      }
    };
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("keydown", escape);
      opener.current?.focus();
    };
  }, []);
  return ref;
}

/** Keep Tab and Shift+Tab inside `root`. */
export function trapModalTab(event: ReactKeyboardEvent<HTMLElement>, root: HTMLElement | null): void {
  if (event.key !== "Tab") return;
  const controls = [...(root?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])];
  if (controls.length === 0) return;
  event.preventDefault();
  controls[nextModalFocusIndex(controls.indexOf(document.activeElement as HTMLElement), controls.length, event.shiftKey)]?.focus();
}
