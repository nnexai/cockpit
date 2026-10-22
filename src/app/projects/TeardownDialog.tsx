import { useEffect, useId, useState } from "react";
import type {
  WorkspaceTeardownAction,
  WorkspaceTeardownExecuteRequest,
  WorkspaceTeardownPreview,
  WorkspaceTeardownPreviewRequest,
  WorkspaceTeardownResult,
} from "../../protocol/generated/v1";
import { ModalSection } from "../input/ModalSection";
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
  if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") return error.message;
  return error instanceof Error && error.message ? error.message : fallback;
}

function errorCode(error: unknown): string | null {
  if (typeof error === "object" && error !== null && "operationCode" in error && typeof error.operationCode === "string") return error.operationCode;
  if (typeof error === "object" && error !== null && "code" in error && typeof error.code === "string") return error.code;
  return null;
}

function actionLabel(action: WorkspaceTeardownAction, discardCount: number): string {
  switch (action) {
    case "close_space": return "Close Space";
    case "remove_owned_worktree": return discardCount > 0 ? "Discard and remove worktree" : "Remove worktree";
    case "reconcile_remove_outcome": return "Reconcile removal";
  }
}

function confirmLabel(action: WorkspaceTeardownAction, discardCount: number): string {
  if (action === "remove_owned_worktree") {
    return discardCount > 0
      ? `Confirm discard ${discardCount} ${discardCount === 1 ? "path" : "paths"} and remove`
      : "Confirm remove worktree";
  }
  return actionLabel(action, discardCount);
}

export function TeardownDialog({ client, sessionId, workspaceId, open, onClose, onCompleted }: TeardownDialogProps) {
  const titleId = useId();
  const [preview, setPreview] = useState<WorkspaceTeardownPreview | null>(null);
  const [selectedAction, setSelectedAction] = useState<WorkspaceTeardownAction | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [receipt, setReceipt] = useState<WorkspaceTeardownResult | null>(null);

  useEffect(() => { setReceipt(null); setError(null); }, [open, sessionId, workspaceId]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    setPreview(null);
    setSelectedAction(null);
    void client.workspaceTeardownPreview(sessionId, { workspace_id: workspaceId })
      .then((next) => { if (active) setPreview(next); })
      .catch((reason: unknown) => { if (active) setError(errorMessage(reason, "Could not prepare teardown review.")); });
    return () => { active = false; };
  }, [client, open, refresh, sessionId, workspaceId]);

  if (!open) return null;
  const removeSelected = selectedAction === "remove_owned_worktree";
  const discardPaths = preview?.discard_paths.slice(0, 50) ?? [];
  const canConfirm = Boolean(preview && selectedAction)
    && (!removeSelected || Boolean(preview?.required_confirmation))
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
        confirmation: removeSelected ? preview.required_confirmation! : "",
        ...(removeSelected && preview.discard_count > 0 && preview.discard_fingerprint !== null
          ? { discard_fingerprint: preview.discard_fingerprint }
          : {}),
      });
      onCompleted(result);
      setReceipt(result);
      setSelectedAction(null);
      if (result.outcome === "completed") setPreview(null);
      else setRefresh((current) => current + 1);
    } catch (reason: unknown) {
      setError(errorMessage(reason, "Could not complete the reviewed teardown."));
      if (removeSelected || errorCode(reason) === "stale_identity") {
        setPreview(null);
        setSelectedAction(null);
        setRefresh((current) => current + 1);
      }
    } finally {
      setBusy(false);
    }
  };

  return <div className="setup-overlay" role="presentation">
    <ModalSection className="setup-dialog setup-dialog-compact" labelledBy={titleId} onDismiss={() => { if (!busy) onClose(); }}>
      <header className="setup-header"><div><span className="setup-eyebrow">Task cleanup</span><h2 id={titleId}>Review teardown</h2><p>Close keeps local files. Removal deletes the selected Space's linked worktree; uncommitted changes are discarded only after you confirm with the final button.</p></div><button type="button" className="setup-close" onClick={onClose} disabled={busy}>Close</button></header>
      <main className="setup-body"><section className="setup-step-content">
        {receipt ? <p className="setup-inline-status" role="status">{receipt.message}</p> : null}
        {receipt?.outcome === "completed" ? <div className="setup-actions"><button type="button" onClick={onClose}>Done</button></div> : !preview && !error ? <p className="setup-empty">Preparing fresh provenance and Git status…</p> : null}
        {error ? <p className="setup-error" role="alert">{error}</p> : null}
        {preview ? <>
          <div className="setup-plan-summary">
            <div className="setup-summary-row"><span>Checkout</span><code>{preview.checkout_path}</code></div>
            <div className="setup-summary-row"><span>Ownership</span><span>{preview.ownership.replaceAll("_", " ")}</span></div>
            <div className="setup-summary-row"><span>Git status</span><span>{preview.dirty_state}</span></div>
          </div>
          {preview.ownership === "borrowed_opened" ? <p className="setup-inline-status">This directory is borrowed. Closing its Space leaves its files in place.</p> : null}
          {preview.blockers.length > 0 ? <div className="setup-diagnostics" role="status"><strong>Removal blocked</strong><ul>{preview.blockers.map((blocker) => <li key={blocker}>{blocker}</li>)}</ul></div> : null}
          {preview.discard_count > 0 && preview.allowed_actions.includes("remove_owned_worktree") ? <div className="setup-diagnostics" role="status">
            <strong>{preview.discard_count} uncommitted {preview.discard_count === 1 ? "path" : "paths"} will be permanently discarded</strong>
            <ul className="setup-discard-list" tabIndex={0} aria-label="Paths to be discarded">{discardPaths.map((path) => <li key={path}><code>{path}</code></li>)}</ul>
            {preview.discard_count > discardPaths.length ? <p className="setup-inline-status">…and {preview.discard_count - discardPaths.length} more not shown.</p> : null}
          </div> : null}
          <div className="setup-progress-actions">{preview.allowed_actions.map((action) => <button key={action} type="button" className={selectedAction === action ? "setup-primary" : ""} aria-pressed={selectedAction === action} onClick={() => setSelectedAction(action)} disabled={busy}>{actionLabel(action, preview.discard_count)}</button>)}</div>
          {removeSelected && preview.required_confirmation ? <p className="setup-inline-status" id="teardown-confirm-hint">{preview.discard_count > 0 ? `This discards ${preview.discard_count} uncommitted ${preview.discard_count === 1 ? "path" : "paths"} and removes ${preview.checkout_path}.` : `This removes ${preview.checkout_path}.`}</p> : null}
          <div className="setup-actions"><button type="button" onClick={onClose} disabled={busy}>Cancel</button><button type="button" className={removeSelected ? "setup-destructive" : ""} aria-describedby={removeSelected && preview.required_confirmation ? "teardown-confirm-hint" : undefined} onClick={() => void execute()} disabled={!canConfirm}>{selectedAction ? confirmLabel(selectedAction, preview.discard_count) : "Choose an action"}</button></div>
        </> : null}
      </section></main>
    </ModalSection>
  </div>;
}
