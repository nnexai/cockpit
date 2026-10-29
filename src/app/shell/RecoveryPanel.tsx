import type { ResourceMutationRequest } from "../../protocol/generated/v1";
import type { SessionState } from "../session/sessionStore";
import type { MutationCoordinatorState, MutationOperation } from "../session/mutationCoordinator";
import { plainError } from "./plainErrors";

export function mutationFailureCanRetry(request: ResourceMutationRequest, code: string | undefined): boolean {
  if (code === "mutation_applied_snapshot_failed" || code === "request_outcome_unknown") return false;
  return request.type === "space_rename" || request.type === "space_move_block" || request.type === "tab_rename" || request.type === "tab_move" || request.type === "pane_rename";
}

export function RecoveryPanel({ state, mutations, onReconnect, onRetryMutation }: { state: SessionState; mutations: MutationCoordinatorState; onReconnect: () => void; onRetryMutation: (operation: MutationOperation) => void }) {
  const failures = Object.values(mutations.errors);
  if (!state.syncError && failures.length === 0) return null;
  return <aside className="recovery-panel" aria-label="Recovery" role="alert">
    {state.syncError ? <div><span title={plainError(state.syncError.code, state.syncError.message).detail}>{plainError(state.syncError.code, state.syncError.message).text}</span><button type="button" onClick={onReconnect}>Resync</button></div> : null}
    {failures.map((failure) => <div key={`${failure.operation.key}:${failure.operation.token}`}><span title={failure.detail ?? plainError(failure.code, failure.message).detail}>{plainError(failure.code, failure.message).text}</span>{mutationFailureCanRetry(failure.operation.request, failure.code) ? <button type="button" onClick={() => onRetryMutation(failure.operation)}>Retry</button> : null}<button type="button" onClick={onReconnect}>Resync</button></div>)}

  </aside>;
}
