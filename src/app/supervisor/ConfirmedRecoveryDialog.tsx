import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ErrorSlot } from "../ErrorSlot";
import { UiIcon } from "../UiIcon";
import { trapDialogKeys, useRestoreFocus } from "../library/LibraryConfirmDialog";
import type { ConfirmedRecovery } from "./useLiaison";
import "./confirmedRecovery.css";

export function ConfirmedRecoveryDialog({ recovery }: { recovery: ConfirmedRecovery }) {
  const titleId = useId(), bodyId = useId();
  const [view, setView] = useState<"options" | "stopped" | "fresh">("options");
  const safeRef = useRef<HTMLButtonElement>(null), confirmRef = useRef<HTMLButtonElement>(null);
  const back = () => { if (recovery.busy) return; if (view === "options" || recovery.retry) recovery.close(); else setView("options"); };
  useRestoreFocus();
  useEffect(() => { safeRef.current?.focus(); }, [view]);
  useEffect(() => { if (recovery.busy) confirmRef.current?.focus(); }, [recovery.busy]);
  const unavailable = recovery.busy || recovery.unavailable;
  return createPortal(<div className="setup-overlay library-dialog-overlay" role="presentation">
    <section className="setup-dialog task-setup library-confirm confirmed-recovery" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={bodyId} onKeyDown={event => trapDialogKeys(event, back)}>
      <header className="task-setup-header"><h2 id={titleId}>{view === "fresh" ? "Start a new conversation?" : view === "stopped" ? "Resume this supervisor?" : "Supervisor recovery"}</h2><button type="button" className="task-setup-close" aria-label="Close" disabled={recovery.busy} onClick={back}><UiIcon name="close" /></button></header>
      <div className="task-setup-body"><div id={bodyId} className="library-confirm-body">
        {view === "stopped" ? <>
          <p className="confirmed-recovery-lead">Continue only after you have stopped the old supervisor and closed its terminal.</p>
          <p className="confirmed-recovery-fine">Tasks and history stay. Cockpit checks the old terminal before resuming.</p>
        </> : view === "fresh" ? <>
          <p className="confirmed-recovery-lead">Start a new conversation with your existing tasks and history.</p>
          <p className="confirmed-recovery-fine">The old terminal stays open. Its supervisor's Cockpit credential will no longer work; you can close the terminal yourself.</p>
        </> : <>
          <p className="confirmed-recovery-lead">The previous supervisor may still be running. Your tasks and history are kept.</p>
          <p className="confirmed-recovery-fine">{recovery.freshAvailable ? "Retry the saved conversation, or start a new one without closing the old terminal." : "Retry the saved conversation after checking the old supervisor and terminal."}</p>
          {recovery.stopConfirmable && !recovery.retry ? <button type="button" onClick={() => setView("stopped")}>I've stopped it myself…</button> : null}
        </>}
      </div></div>
      <footer className="task-setup-footer">
        <ErrorSlot placement="dialog" message={recovery.message} error={recovery.error} />
        <button ref={safeRef} type="button" disabled={recovery.busy} onClick={back}>{view === "options" || recovery.retry ? "Cancel" : "Back"}</button>
        {view === "options" && !recovery.retry && recovery.freshAvailable ? <button type="button" disabled={unavailable} onClick={() => setView("fresh")}>Start new conversation (tasks kept)</button> : null}
        <button ref={confirmRef} type="button" className="setup-primary" aria-disabled={unavailable} onClick={() => {
          if (unavailable) return;
          if (recovery.pendingAction === "fresh" || view === "fresh") void recovery.startNew();
          else if (view === "stopped") void recovery.confirm();
          else void recovery.retryResume();
        }}>{recovery.busy ? "Working…" : recovery.pendingAction === "fresh" || view === "fresh" ? "Start new conversation" : view === "stopped" && !recovery.retry ? "It's stopped — resume" : "Retry resume"}</button>
      </footer>
    </section>
  </div>, document.body);
}
