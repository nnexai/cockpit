import { useState } from "react";
import type { LibraryOperation, LibraryReportRow } from "../../protocol/generated/v1";
import { REPORT_OUTCOMES, reportOutcomeLabel, reportSummary } from "./libraryState";

/**
 * Provider refresh report (design §4.9). It stays until dismissed or the next
 * refresh; a Library refresh never writes to a Space.
 */
export function RefreshReport({ operation, verb, error, onCancel, onDismiss, onOpenItem, onRetry }: {
  operation: LibraryOperation;
  /** `Refresh` for provider refreshes, `Replace` for a confirmed edited-file replace. */
  verb: "Refresh" | "Replace";
  error: string | null;
  onCancel: () => void;
  onDismiss: () => void;
  onOpenItem: (itemId: string) => void;
  onRetry: (itemIds: string[]) => void;
}) {
  const [shown, setShown] = useState(false);
  const phase = operation.phases.find((candidate) => candidate.phase === "library");
  const report = operation.report;
  if (!operation.finished) {
    const total = phase?.total ?? null;
    return <div className="context-notice library-report" role="status">
      <span className="library-spinner" aria-hidden="true" />
      <span>{verb === "Refresh" ? `Refreshing${total !== null ? ` ${total} ${total === 1 ? "item" : "items"}` : ""}… ${phase?.done ?? 0} done` : "Replacing with the source version…"}</span>
      {operation.cancel_requested ? <span>Cancelling after the item in flight…</span> : <button type="button" onClick={onCancel}>Cancel</button>}
      {error ? <span className="library-report-error">{error}</span> : null}
    </div>;
  }
  const failed = phase?.state === "failed" ? phase.error : null;
  const rows = report?.rows ?? [];
  const retryIds = rows.filter((row) => row.outcome === "failed" && row.item_id).map((row) => row.item_id!);
  const rowLabel = (row: LibraryReportRow) => row.item_id
    ? <button type="button" className="library-report-link" onClick={() => onOpenItem(row.item_id!)}>{row.title}</button>
    : <span>{row.title}</span>;
  return <div className="library-report-block">
    <div className={`context-notice library-report${failed ? " context-notice-error" : ""}`} role="status">
      <strong>{verb} {phase?.state === "cancelled" ? "cancelled" : failed ? "failed" : "finished"}:</strong>
      <span>{failed ? failed.message : report ? reportSummary(report) : "done"}</span>
      <span className="library-report-note">Spaces aren't changed.</span>
      <span className="context-toolbar-spacer" />
      {rows.length > 0 ? <button type="button" aria-expanded={shown} onClick={() => setShown((value) => !value)}>{shown ? "Hide" : "Show"}</button> : null}
      {retryIds.length > 0 ? <button type="button" onClick={() => onRetry(retryIds)}>Retry failed</button> : null}
      <button type="button" onClick={onDismiss}>Dismiss</button>
    </div>
    {shown && rows.length > 0 ? <div className="library-report-rows">
      {REPORT_OUTCOMES.map((outcome) => {
        const group = rows.filter((row) => row.outcome === outcome);
        if (group.length === 0) return null;
        return <section key={outcome} aria-label={reportOutcomeLabel(outcome)}>
          <h3>{group.length} {reportOutcomeLabel(outcome)}</h3>
          <ul>{group.map((row, index) => <li key={`${row.item_id ?? row.follow_id ?? row.title}:${index}`} className={row.outcome === "failed" ? "is-failed" : undefined}>
            {rowLabel(row)}
            {row.reason ? <span className="library-report-reason">{row.outcome === "failed" ? `${row.reason}. Previous content kept.` : row.reason}</span> : null}
            {row.outcome === "failed" && row.item_id ? <button type="button" onClick={() => onRetry([row.item_id!])}>Retry</button> : null}
          </li>)}</ul>
        </section>;
      })}
      {report?.truncated_rows ? <p className="library-report-reason">Only the first rows are listed.</p> : null}
    </div> : null}
  </div>;
}
