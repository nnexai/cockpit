import { useState } from "react";
import type { LibraryOperation, LibraryReportRow } from "../../protocol/generated/v1";
import { REPORT_OUTCOMES, reportOutcomeLabel, reportSummary } from "./libraryState";
import { UiIcon } from "../UiIcon";
import { ErrorSlot } from "../ErrorSlot";

/**
 * Provider refresh report (design §4.9). It stays until dismissed or the next
 * refresh; a Library refresh never writes to a Space. A followed space or query
 * that read only part of its source explains it: nothing is marked removed at
 * source or dropped until a refresh reads all of it.
 */
export function RefreshReport({ operation, verb, error, onCancel, onDismiss, onOpenItem, onRetry, onRetryOperation, onRetryFollow }: {
  operation: LibraryOperation;
  /** `Refresh` for provider refreshes, `Replace` for a confirmed edited-file replace, `Keep` for `Keep in Library`. */
  verb: "Refresh" | "Replace" | "Keep";
  error: string | null;
  onCancel: () => void;
  onDismiss: () => void;
  onOpenItem: (itemId: string) => void;
  onRetry: (itemIds: string[]) => void;
  onRetryOperation?: () => void;
  /** Refreshes one followed space again; failed follow rows offer it when set. */
  onRetryFollow?: (followId: string) => void;
}) {
  const [shown, setShown] = useState(false);
  const phase = operation.phases.find((candidate) => candidate.phase === "library");
  const report = operation.report;
  if (!operation.finished) {
    const total = phase?.total ?? null;
    return <ErrorSlot placement="pane" className="library-report" error={Boolean(error)}
      message={error ?? (verb === "Refresh" ? `Refreshing${total !== null ? ` ${total} ${total === 1 ? "item" : "items"}` : ""}… ${phase?.done ?? 0} done` : verb === "Keep" ? "Keeping in Library…" : "Replacing with the source version…")}
      actions={operation.cancel_requested ? <span>Cancelling after the item in flight…</span> : <button type="button" onClick={onCancel}>Cancel</button>} />;
  }
  const failed = phase?.state === "failed" ? phase.error ?? { message: phase.message ?? "The operation failed." } : null;
  const rows = report?.rows ?? [];
  const retryIds = rows.filter((row) => row.outcome === "failed" && row.item_id).map((row) => row.item_id!);
  // One retry starts one operation: failed items, else the one followed space that failed; several failed spaces retry per row.
  const failedFollowIds = onRetryFollow && retryIds.length === 0 ? [...new Set(rows.filter((row) => row.outcome === "failed" && !row.item_id && row.follow_id).map((row) => row.follow_id!))] : [];
  const limitedFollows = rows.filter((row) => row.outcome === "partial" && !row.item_id && row.follow_id);
  const rowLabel = (row: LibraryReportRow) => row.item_id
    ? <button type="button" className="library-report-link" onClick={() => onOpenItem(row.item_id!)}>{row.title}</button>
    : <span>{row.title}</span>;
  return <div className="library-report-block">
    <ErrorSlot placement="pane" className="library-report" error={Boolean(failed || error || rows.some((row) => row.outcome === "failed"))}
      message={<><strong>{verb === "Keep" ? (phase?.state === "cancelled" ? "Keep cancelled:" : failed ? "Keep failed:" : "Kept in Library:") : `${verb} ${phase?.state === "cancelled" ? "cancelled" : failed ? "failed" : "finished"}:`}</strong> {error ?? failed?.message ?? (report ? reportSummary(report) : "done")} <span className="library-report-note">Spaces aren't changed.</span></>}
      actions={<>
        {rows.length > 0 ? <button type="button" aria-expanded={shown} onClick={() => setShown((value) => !value)}>{shown ? "Hide" : "Show"}</button> : null}
        {retryIds.length > 0 || failedFollowIds.length === 1 ? <button type="button" onClick={() => {
          if (retryIds.length > 0) onRetry(retryIds); else onRetryFollow?.(failedFollowIds[0]!);
        }}>Retry failed</button> : failed && onRetryOperation ? <button type="button" onClick={onRetryOperation}>Retry</button> : null}
        <button type="button" onClick={onDismiss}>Dismiss</button>
      </>} />
    {shown && rows.length > 0 ? <div className="library-report-rows">
      {limitedFollows.map((row, index) => <p key={`${row.follow_id}:${index}`} className="context-notice context-notice-warning library-report-limit" role="status">
        <UiIcon name="half-ring" />
        <span>{`${row.title}: ${row.reason ?? "partial"}. Only part of the source was read, so nothing is marked removed at source or dropped until a refresh reads all of it.`}</span>
      </p>)}
      {REPORT_OUTCOMES.map((outcome) => {
        const group = rows.filter((row) => row.outcome === outcome);
        if (group.length === 0) return null;
        return <section key={outcome} aria-label={reportOutcomeLabel(outcome)}>
          <h3>{group.length} {reportOutcomeLabel(outcome)}</h3>
          <ul>{group.map((row, index) => <li key={`${row.item_id ?? row.follow_id ?? row.title}:${index}`} className={row.outcome === "failed" ? "is-failed" : undefined}>
            {rowLabel(row)}
            {row.reason ? <span className="library-report-reason">{row.outcome === "failed" ? `${row.reason}. Previous content kept.` : row.reason}</span> : null}
            {row.outcome === "failed" && row.item_id ? <button type="button" onClick={() => onRetry([row.item_id!])}>Retry</button> : null}
            {row.outcome === "failed" && !row.item_id && row.follow_id && onRetryFollow ? <button type="button" onClick={() => onRetryFollow(row.follow_id!)}>Retry</button> : null}
          </li>)}</ul>
        </section>;
      })}
      {report?.truncated_rows ? <p className="library-report-reason">Only the first rows are listed.</p> : null}
    </div> : null}
  </div>;
}
