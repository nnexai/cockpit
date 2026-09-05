import { useCallback, useReducer, useRef, type Dispatch, type MutableRefObject } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ResourceMutationRequest } from "../../protocol/generated/v1";
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
  focusTokenRef: MutableRefObject<number>;
  dispatchSession: Dispatch<SessionAction>;
  describeError(error: unknown, fallback: string): StatusError;
  mutationSnapshot(sessionId: string, response: unknown): { focused_pane_id: string | null };
  onResync(): void;
};

export function useMutationCoordinator({ client, stateRef, mountedRef, sessionObservationRef, focusTokenRef, dispatchSession, describeError, mutationSnapshot, onResync }: MutationCoordinatorOptions) {
  const [state, dispatch] = useReducer(mutationCoordinatorReducer, initialMutationCoordinatorState);
  const tokenRef = useRef(0);
  const pendingRef = useRef(false);
  const focusIntentRef = useRef<{ epoch: number; token: number; paneId: string } | null>(null);

  const reset = useCallback(() => {
    tokenRef.current += 1;
    pendingRef.current = false;
    focusIntentRef.current = null;
    dispatch({ type: "reset" });
  }, []);

  const mutate = useCallback((key: string, request: ResourceMutationRequest, focusFromSnapshot = false): boolean => {
    const current = stateRef.current;
    const sessionId = current.sessionId;
    if (!sessionId || pendingRef.current) return false;
    const epoch = current.epoch;
    const observation = sessionObservationRef.current;
    const focusToken = focusTokenRef.current;
    const token = tokenRef.current + 1;
    tokenRef.current = token;
    pendingRef.current = true;
    const operation: MutationOperation = { epoch, token, key, request, focusFromSnapshot };
    dispatch({ type: "begin", operation });
    void client.mutate(sessionId, request).then((response) => {
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId || tokenRef.current !== token) return;
      const snapshot = mutationSnapshot(sessionId, response);
      pendingRef.current = false;
      dispatch({ type: "succeed", epoch, token });
      focusIntentRef.current = focusFromSnapshot && snapshot.focused_pane_id && observation === sessionObservationRef.current && focusToken === focusTokenRef.current
        ? { epoch, token, paneId: snapshot.focused_pane_id }
        : null;
      dispatchSession({ type: "snapshot/request", epoch, sessionId });
      onResync();
    }).catch((error: unknown) => {
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId || tokenRef.current !== token) return;
      pendingRef.current = false;
      const errorState = describeError(error, "Could not update Herdr resource");
      dispatch({ type: "fail", epoch, token, error: errorState });
      if (errorState.code === "mutation_applied_snapshot_failed" || errorState.code === "request_outcome_unknown") {
        focusIntentRef.current = null;
        onResync();
      }
    });
    return true;
  }, [client, describeError, dispatchSession, focusTokenRef, mountedRef, mutationSnapshot, onResync, sessionObservationRef, stateRef]);

  const retry = useCallback((operation: MutationOperation) => mutate(operation.key, operation.request, operation.focusFromSnapshot), [mutate]);
  const consumeFocusedPane = useCallback((epoch: number, focusedPaneId: string | null): string | null => {
    const intent = focusIntentRef.current;
    if (!intent?.paneId || intent.epoch !== epoch || intent.token !== tokenRef.current) return null;
    focusIntentRef.current = null;
    return focusedPaneId === intent.paneId ? intent.paneId : null;
  }, []);

  return { consumeFocusedPane, mutate, reset, retry, state, tokenRef };
}
