import { useEffect, useId, useState } from "react";
import type {
  WorkspaceTeardownAction,
  WorkspaceTeardownExecuteRequest,
  WorkspaceTeardownPreview,
  WorkspaceTeardownPreviewRequest,
  WorkspaceTeardownResult,
} from "../../protocol/generated/v1";
import "./setup.css";

export type TeardownClient = {
  workspaceTeardownPreview(sessionId: string, request: WorkspaceTeardownPreviewRequest): Promise<WorkspaceTeardownPreview>;
  workspaceTeardownExecute(sessionId: string, request: WorkspaceTeardownExecuteRequest): Promise<WorkspaceTeardownResult>;
};

export type TeardownDialogProps = {
  client: TeardownClient;
  sessionId: string;
  workspaceId: string;
  open: boolean;
  onClose: () => void;
  onCompleted: (result: WorkspaceTeardownResult) => void;
};

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

function actionLabel(action: WorkspaceTeardownAction): string {
  switch (action) {
    case "close_space": return "Close Space";
    case "remove_owned_worktree": return "Remove task worktree";
    case "reconcile_remove_outcome": return "Reconcile removal";
    case "remove_orphaned_companion": return "Remove orphaned companion";
    case "forget_association": return "Forget association";
  }
}

export function TeardownDialog({ client, sessionId, workspaceId, open, onClose, onCompleted }: TeardownDialogProps) {
  const titleId = useId();
  const [preview, setPreview] = useState<WorkspaceTeardownPreview | null>(null);
  const [confirmation, setConfirmation] = useState("");
  const [selectedAction, setSelectedAction] = useState<WorkspaceTeardownAction | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [receipt, setReceipt] = useState<WorkspaceTeardownResult | null>(null);

  useEffect(() => { setReceipt(null); }, [open, sessionId, workspaceId]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    setPreview(null);
    setConfirmation("");
    setSelectedAction(null);
    setError(null);
    void client.workspaceTeardownPreview(sessionId, { workspace_id: workspaceId })
      .then((next) => { if (active) setPreview(next); })
      .catch((reason: unknown) => { if (active) setError(errorMessage(reason, "Could not prepare teardown review.")); });
    return () => { active = false; };
  }, [client, open, refresh, sessionId, workspaceId]);

  if (!open) return null;
  const removeSelected = selectedAction === "remove_owned_worktree" || selectedAction === "remove_orphaned_companion";
  const canConfirm = Boolean(selectedAction)
    && (!removeSelected || confirmation === preview?.required_confirmation)
    && !busy;
  const execute = async () => {
    if (!preview || !selectedAction || !canConfirm) return;
    setBusy(true);
    setError(null);
    try {
      const result = await client.workspaceTeardownExecute(sessionId, {
        operation_id: preview.operation_id,
        workspace_id: preview.workspace_id,
        expected_endpoint_identity: preview.endpoint_identity,
        expected_checkout_path: preview.checkout_path,
        action: selectedAction,
        confirmation,
      });
      onCompleted(result);
      setReceipt(result);
      setConfirmation("");
      setSelectedAction(null);
      if (result.outcome === "completed") setPreview(null);
      else setRefresh((current) => current + 1);
    } catch (reason: unknown) {
      setError(errorMessage(reason, "Could not complete the reviewed teardown."));
    } finally {
      setBusy(false);
    }
  };

  return <div className="setup-overlay" role="presentation">
    <section className="setup-dialog setup-dialog-compact" role="dialog" aria-modal="true" aria-labelledby={titleId}>
      <header className="setup-header"><div><span className="setup-eyebrow">Task cleanup</span><h2 id={titleId}>Review teardown</h2><p>Close keeps local files. Removal is available only for a freshly verified clean, Cockpit-owned linked worktree.</p></div><button type="button" className="setup-close" onClick={onClose} disabled={busy}>Close</button></header>
      <main className="setup-body"><section className="setup-step-content">
        {receipt ? <p className="setup-inline-status" role="status">{receipt.message}</p> : null}
        {receipt?.outcome === "completed" ? <div className="setup-actions"><button type="button" onClick={onClose}>Done</button></div> : !preview && !error ? <p className="setup-empty">Preparing fresh provenance and Git status…</p> : null}
        {error ? <p className="setup-error" role="alert">{error}</p> : null}
        {preview ? <>
          <div className="setup-plan-summary">
            <div className="setup-summary-row"><span>Checkout</span><code>{preview.checkout_path}</code></div>
            <div className="setup-summary-row"><span>Ownership</span><span>{preview.ownership.replaceAll("_", " ")}</span></div>
            <div className="setup-summary-row"><span>Git status</span><span>{preview.dirty_state}</span></div>
            <div className="setup-summary-row"><span>Companion</span><code>{preview.companion_path ?? "No owned companion"}</code></div>
          </div>
          {preview.ownership === "borrowed_opened" ? <p className="setup-inline-status">This directory is borrowed. Closing its Space and removing an owned companion leave its files in place.</p> : null}
          {preview.blockers.length > 0 ? <div className="setup-diagnostics" role="status"><strong>Removal blocked</strong><ul>{preview.blockers.map((blocker) => <li key={blocker}>{blocker}</li>)}</ul></div> : null}
          {preview.warnings.length > 0 ? <div className="setup-warnings"><strong>Recovery notes</strong><ul>{preview.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul></div> : null}
          <div className="setup-progress-actions">{preview.allowed_actions.map((action) => <button key={action} type="button" className={selectedAction === action ? "setup-primary" : ""} aria-pressed={selectedAction === action} onClick={() => { setSelectedAction(action); setConfirmation(""); }} disabled={busy}>{actionLabel(action)}</button>)}</div>
          {removeSelected && preview.required_confirmation ? <label className="setup-field" htmlFor="teardown-confirmation"><span className="setup-label">Type <code>{preview.required_confirmation}</code> to remove this exact reviewed resource</span><input id="teardown-confirmation" value={confirmation} onChange={(event) => setConfirmation(event.target.value)} autoComplete="off" disabled={busy} /></label> : null}
          <div className="setup-actions"><button type="button" onClick={onClose} disabled={busy}>Cancel</button><button type="button" className={removeSelected ? "setup-primary" : ""} onClick={() => void execute()} disabled={!canConfirm}>{selectedAction ? actionLabel(selectedAction) : "Choose an action"}</button></div>
        </> : null}
      </section></main>
    </section>
  </div>;
}
