import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { UiIcon } from "../UiIcon";
import { errorText } from "./libraryState";
import "../projects/setup.css";
import "../projects/taskSetup.css";
import "./library.css";

const FOCUSABLE = "button:not(:disabled), input:not(:disabled), select:not(:disabled), a[href], summary, [tabindex]:not([tabindex='-1'])";

/** Escape closes, Tab stays inside, and focus returns to the opener on close. */
export function trapDialogKeys(event: KeyboardEvent<HTMLElement>, onClose: () => void): void {
  if (event.nativeEvent.isComposing) return;
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    onClose();
    return;
  }
  if (event.key !== "Tab") return;
  const focusable = [...event.currentTarget.querySelectorAll<HTMLElement>(FOCUSABLE)];
  if (focusable.length === 0) return;
  const current = focusable.indexOf(document.activeElement as HTMLElement);
  if (event.shiftKey && current <= 0) { event.preventDefault(); focusable.at(-1)?.focus(); }
  else if (!event.shiftKey && current === focusable.length - 1) { event.preventDefault(); focusable[0]?.focus(); }
}

/** Remembers the opener when mounted and focuses it again when unmounted. */
export function useRestoreFocus(): void {
  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    return () => { if (opener?.isConnected) opener.focus({ preventScroll: true }); };
  }, []);
}

/**
 * Compact confirmation (design §4.10): focus starts on the safe button, the
 * destructive button names its effect, and a failure stays inline.
 */
export function LibraryConfirmDialog({ title, body, safeLabel, confirmLabel, destructive = false, onConfirm, onClose }: {
  title: string;
  body: ReactNode;
  safeLabel: string;
  confirmLabel: string;
  destructive?: boolean;
  /** Resolves when the action was accepted; a rejection keeps the dialog open with the reason. */
  onConfirm: () => Promise<void>;
  onClose: () => void;
}) {
  const titleId = useId();
  const bodyId = useId();
  const safeRef = useRef<HTMLButtonElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useRestoreFocus();
  useEffect(() => { safeRef.current?.focus(); }, []);
  const confirm = async () => {
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
    } catch (cause) {
      setError(errorText(cause, "The action could not be completed."));
      setBusy(false);
    }
  };
  // On the body: a Context viewer is a size container and would clip a fixed overlay to its pane.
  return createPortal(<div className="setup-overlay library-dialog-overlay" role="presentation">
    <section className="setup-dialog task-setup library-confirm" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={bodyId} onKeyDown={(event) => trapDialogKeys(event, onClose)}>
      <header className="task-setup-header"><h2 id={titleId}>{title}</h2><button type="button" className="task-setup-close" onClick={onClose} aria-label="Close" disabled={busy}><UiIcon name="close" /></button></header>
      <div className="task-setup-body">
        <div id={bodyId} className="library-confirm-body">{body}</div>
        {error ? <p className="task-setup-note is-error" role="alert">{error}</p> : null}
      </div>
      <footer className="task-setup-footer">
        <button ref={safeRef} type="button" onClick={onClose} disabled={busy}>{safeLabel}</button>
        <button type="button" className={destructive ? "library-destructive" : "setup-primary"} onClick={() => void confirm()} disabled={busy}>{busy ? "Working…" : confirmLabel}</button>
      </footer>
    </section>
  </div>, document.body);
}
