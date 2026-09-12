import { useEffect, useId, useState } from "react";
import type {
  WorkspaceTeardownRecovery,
  WorkspaceTeardownRecoveryList,
} from "../../protocol/generated/v1";
import { TeardownDialog, type TeardownClient } from "./TeardownDialog";
import "./setup.css";

export type TeardownRecoveryClient = TeardownClient & {
  workspaceTeardownRecoveries(sessionId: string): Promise<WorkspaceTeardownRecoveryList>;
};

export type TeardownRecoveryPanelProps = {
  client: TeardownRecoveryClient;
  sessionId: string;
  open: boolean;
  onClose: () => void;
};

function errorMessage(error: unknown): string {
  return error instanceof Error && error.message ? error.message : "Could not load pending cleanup.";
}

function stateLabel(recovery: WorkspaceTeardownRecovery): string {
  switch (recovery.state) {
    case "pending": return "Removal dispatch needs reconciliation";
    case "outcome_unknown": return "Removal outcome unknown";
    case "orphaned_companion": return "Companion cleanup pending";
  }
}

/**
 * Recovery is intentionally independent of the current Space list. The
 * selected entry opens the normal fresh-proof dialog using its durable
 * workspace identity, so an absent Space cannot hide an orphaned cleanup.
 */
export function TeardownRecoveryPanel({ client, sessionId, open, onClose }: TeardownRecoveryPanelProps) {
  const titleId = useId();
  const [list, setList] = useState<WorkspaceTeardownRecoveryList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<WorkspaceTeardownRecovery | null>(null);
  const [refresh, setRefresh] = useState(0);

  useEffect(() => {
    if (!open || selected) return;
    let active = true;
    setList(null);
    setError(null);
    void client.workspaceTeardownRecoveries(sessionId)
      .then((next) => { if (active) setList(next); })
      .catch((reason: unknown) => { if (active) setError(errorMessage(reason)); });
    return () => { active = false; };
  }, [client, open, refresh, selected, sessionId]);

  if (!open) return null;
  if (selected) {
    return <TeardownDialog
      client={client}
      sessionId={sessionId}
      workspaceId={selected.workspace_id}
      open
      onClose={() => setSelected(null)}
      onCompleted={() => setRefresh((current) => current + 1)}
    />;
  }
  return <div className="setup-overlay" role="presentation">
    <section className="setup-dialog setup-dialog-compact" role="dialog" aria-modal="true" aria-labelledby={titleId}>
      <header className="setup-header"><div><span className="setup-eyebrow">Task cleanup</span><h2 id={titleId}>Pending cleanup</h2><p>These records remain available after a Space closes or the app restarts.</p></div><button type="button" className="setup-close" onClick={onClose}>Close</button></header>
      <main className="setup-body"><section className="setup-step-content">
        {!list && !error ? <p className="setup-empty">Loading durable cleanup records…</p> : null}
        {error ? <p className="setup-error" role="alert">{error}</p> : null}
        {list?.recoveries.length === 0 ? <p className="setup-empty">No pending cleanup records.</p> : null}
        {list?.recoveries.map((recovery) => <div className="setup-plan-summary" key={recovery.operation_id}>
          <div className="setup-summary-row"><span>Checkout</span><code>{recovery.checkout_path}</code></div>
          <div className="setup-summary-row"><span>Status</span><span>{stateLabel(recovery)}</span></div>
          <div className="setup-actions"><button type="button" className="setup-primary" onClick={() => setSelected(recovery)}>Review recovery</button></div>
        </div>)}
      </section></main>
    </section>
  </div>;
}
