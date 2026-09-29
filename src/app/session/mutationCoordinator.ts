import { useCallback, useReducer, useRef, type Dispatch, type MutableRefObject } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { CreatedPane, ResourceMutationRequest, SessionSnapshotResponse } from "../../protocol/generated/v1";
import type { SessionAction, SessionState } from "./sessionStore";

type StatusError = { message: string; code?: string };
export type MutationOperation = {
  epoch: number;
  token: number;
  key: string;
  request: ResourceMutationRequest;
  focusFromSnapshot: boolean;
};
type MutationFailure = StatusError & { operation: MutationOperation };
export type MutationCoordinatorState = {
  token: number;
  pending: MutationOperation | null;
  errors: Record<string, MutationFailure>;
};
export type MutationCoordinatorAction =
  | { type: "begin"; operation: MutationOperation }
  | { type: "succeed"; epoch: number; token: number }
  | { type: "fail"; epoch: number; token: number; error: StatusError }
  | { type: "clear"; key: string }
  | { type: "reset" };

export const initialMutationCoordinatorState: MutationCoordinatorState = { token: 0, pending: null, errors: {} };

export function mutationCoordinatorReducer(state: MutationCoordinatorState, action: MutationCoordinatorAction): MutationCoordinatorState {
  if (action.type === "reset") return initialMutationCoordinatorState;
  if (action.type === "begin") {
    if (state.pending) return state;
    const errors = { ...state.errors };
    delete errors[action.operation.key];
    return { token: action.operation.token, pending: action.operation, errors };
  }
  if (action.type === "clear") {
    const errors = { ...state.errors };
    delete errors[action.key];
    return { ...state, errors };
  }
  const pending = state.pending;
  if (!pending || pending.epoch !== action.epoch || pending.token !== action.token) return state;
  if (action.type === "succeed") return { ...state, pending: null };
  return { ...state, pending: null, errors: { ...state.errors, [pending.key]: { ...action.error, operation: pending } } };
}

export type MutationCoordinatorOptions = {
  client: CockpitClient;
  stateRef: MutableRefObject<SessionState>;
  mountedRef: MutableRefObject<boolean>;
  sessionObservationRef: MutableRefObject<number>;
  dispatchSession: Dispatch<SessionAction>;
  describeError(error: unknown, fallback: string): StatusError;
  mutationSnapshot(sessionId: string, response: unknown): SessionSnapshotResponse;
  onBegin(operation: MutationOperation): void;
  onSettled(operation: MutationOperation, created: CreatedPane | null, snapshot?: SessionSnapshotResponse, responseIsCurrent?: boolean): void;
  onResync(): void;
};

export type MutationCoordinator = {
  mutate(key: string, request: ResourceMutationRequest, focusFromSnapshot?: boolean): boolean;
  reset(): void;
  retry(operation: MutationOperation): boolean;
  state: MutationCoordinatorState;
  tokenRef: MutableRefObject<number>;
};

export function useMutationCoordinator({ client, stateRef, mountedRef, sessionObservationRef, dispatchSession, describeError, mutationSnapshot, onBegin, onSettled, onResync }: MutationCoordinatorOptions): MutationCoordinator {
  const [state, dispatch] = useReducer(mutationCoordinatorReducer, initialMutationCoordinatorState);
  const tokenRef = useRef(0);
  const pendingRef = useRef(false);

  const reset = useCallback(() => {
    tokenRef.current += 1;
    pendingRef.current = false;
    dispatch({ type: "reset" });
  }, []);

  const mutate = useCallback((key: string, request: ResourceMutationRequest, focusFromSnapshot = false): boolean => {
    const current = stateRef.current;
    const sessionId = current.sessionId;
    if (!sessionId || pendingRef.current) return false;
    const epoch = current.epoch;
    const observation = sessionObservationRef.current;
    const streamGeneration = current.generation;
    const streamSequence = current.sequence;
    const token = tokenRef.current + 1;
    tokenRef.current = token;
    pendingRef.current = true;
    const operation: MutationOperation = { epoch, token, key, request, focusFromSnapshot };
    dispatch({ type: "begin", operation });
    onBegin(operation);
    void client.mutate(sessionId, request).then((response) => {
      const snapshot = mutationSnapshot(sessionId, response);
      const operationIsCurrent = mountedRef.current && stateRef.current.epoch === epoch
        && stateRef.current.sessionId === sessionId && tokenRef.current === token;
      const responseIsCurrent = operationIsCurrent && observation === sessionObservationRef.current
        && stateRef.current.generation === streamGeneration
        && stateRef.current.sequence === streamSequence;
      onSettled(operation, response.created, snapshot, responseIsCurrent);
      if (!operationIsCurrent) return;
      pendingRef.current = false;
      dispatch({ type: "succeed", epoch, token });
      // Use the mutation snapshot only until an ordered stream event supersedes it.
      // A response that arrived behind the stream cannot overwrite a newer user action.
      if (responseIsCurrent) dispatchSession({ type: "snapshot/authoritative", epoch, sessionId, snapshot });
      dispatchSession({ type: "snapshot/request", epoch, sessionId });
      onResync();
    }).catch((error: unknown) => {
      onSettled(operation, null);
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId || tokenRef.current !== token) return;
      pendingRef.current = false;
      const errorState = describeError(error, "Could not update Herdr resource");
      dispatch({ type: "fail", epoch, token, error: errorState });
      if (errorState.code === "mutation_applied_snapshot_failed" || errorState.code === "request_outcome_unknown") {
        onResync();
      }
    });
    return true;
  }, [client, describeError, dispatchSession, mountedRef, mutationSnapshot, onBegin, onSettled, onResync, sessionObservationRef, stateRef]);

  const retry = useCallback((operation: MutationOperation) => mutate(operation.key, operation.request, operation.focusFromSnapshot), [mutate]);

  return { mutate, reset, retry, state, tokenRef };
}
