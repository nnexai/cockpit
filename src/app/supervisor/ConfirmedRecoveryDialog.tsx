import { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { ErrorSlot } from "../ErrorSlot";
import { UiIcon } from "../UiIcon";
import { trapDialogKeys, useRestoreFocus } from "../library/LibraryConfirmDialog";
import type { ConfirmedRecovery } from "./useLiaison";
import "./confirmedRecovery.css";

export function ConfirmedRecoveryDialog({ recovery }: { recovery: ConfirmedRecovery }) {
  const titleId = useId(), bodyId = useId();
  const safeRef = useRef<HTMLButtonElement>(null), confirmRef = useRef<HTMLButtonElement>(null);
  useRestoreFocus();
  useEffect(() => { safeRef.current?.focus(); }, []);
  useEffect(() => { if (recovery.busy) confirmRef.current?.focus(); }, [recovery.busy]);
  return createPortal(<div className="setup-overlay library-dialog-overlay" role="presentation">
    <section className="setup-dialog task-setup library-confirm confirmed-recovery" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={bodyId} onKeyDown={event => trapDialogKeys(event, recovery.close)}>
      <header className="task-setup-header"><h2 id={titleId}>Resume this supervisor?</h2><button type="button" className="task-setup-close" aria-label="Close" disabled={recovery.busy} onClick={recovery.close}><UiIcon name="close" /></button></header>
      <div className="task-setup-body"><div id={bodyId} className="library-confirm-body">
        <p className="confirmed-recovery-lead">Continue only after you have stopped the old supervisor and closed its terminal.</p>
        <p className="confirmed-recovery-fine">Tasks and history stay. Cockpit checks that the old terminal is gone before starting a replacement.</p>
      </div></div>
      <footer className="task-setup-footer">
        <ErrorSlot placement="dialog" message={recovery.message} error={recovery.error} />
        <button ref={safeRef} type="button" disabled={recovery.busy} onClick={recovery.close}>Cancel</button>
        <button ref={confirmRef} type="button" className="setup-primary" aria-disabled={recovery.busy || recovery.unavailable} onClick={() => { if (!recovery.busy && !recovery.unavailable) void recovery.confirm(); }}>{recovery.busy ? "Resuming…" : recovery.retry ? "Retry resume" : "It's stopped — resume"}</button>
      </footer>
    </section>
  </div>, document.body);
}
