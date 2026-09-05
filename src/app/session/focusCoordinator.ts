import { useCallback, useRef, useState, type Dispatch, type MutableRefObject } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { FocusRequest, SessionSnapshotResponse } from "../../protocol/generated/v1";
import type { SessionAction, SessionState } from "./sessionStore";

export const FOCUS_FALLBACK_MS = 500;

export function scheduleFocusFallback(isCurrent: () => boolean, onDelayed: () => void, onResync: () => void): () => void {
  const timer = globalThis.setTimeout(() => {
    if (!isCurrent()) return;
    onDelayed();
    onResync();
  }, FOCUS_FALLBACK_MS);
  return () => globalThis.clearTimeout(timer);
}

export type FocusLocation = { spaceId: string | null; tabId: string | null; paneId: string | null };
type StatusError = { message: string; code?: string };
type FocusIntent = { epoch: number; token: number; request: FocusRequest; location: FocusLocation };

export type FocusCoordinatorOptions = {
  client: CockpitClient;
  stateRef: MutableRefObject<SessionState>;
  mountedRef: MutableRefObject<boolean>;
  dispatch: Dispatch<SessionAction>;
  describeError(error: unknown, fallback: string): StatusError;
  onTimeout(): void;
};

export function useFocusCoordinator({ client, stateRef, mountedRef, dispatch, describeError, onTimeout }: FocusCoordinatorOptions) {
  const tokenRef = useRef(0);
  const intentRef = useRef<FocusIntent | null>(null);
  const fallbackCancelRef = useRef<(() => void) | null>(null);
  const [focusDelayed, setFocusDelayed] = useState(false);

  const reset = useCallback(() => {
    tokenRef.current += 1;
    intentRef.current = null;
    fallbackCancelRef.current?.();
    fallbackCancelRef.current = null;
    setFocusDelayed(false);
  }, []);

  const focus = useCallback((request: FocusRequest, location: FocusLocation) => {
    const current = stateRef.current;
    const sessionId = current.sessionId;
    if (!sessionId) return;
    const epoch = current.epoch;
    const token = tokenRef.current + 1;
    tokenRef.current = token;
    fallbackCancelRef.current?.();
    fallbackCancelRef.current = null;
    setFocusDelayed(false);
    intentRef.current = { epoch, token, request, location };
    dispatch({ type: "focus/request", epoch, sessionId, request, token });
    void client.focus(sessionId, request).then((response) => {
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId || tokenRef.current !== token) return;
      if (!response.accepted) {
        dispatch({ type: "focus/error", epoch, sessionId, token, code: "focus_rejected", message: "Herdr did not accept this focus request" });
        return;
      }
      if (!stateRef.current.focusPending) return;
      fallbackCancelRef.current = scheduleFocusFallback(
        () => mountedRef.current && stateRef.current.epoch === epoch && stateRef.current.sessionId === sessionId && tokenRef.current === token && stateRef.current.focusPending !== null,
        () => setFocusDelayed(true),
        () => {
          fallbackCancelRef.current = null;
          setFocusDelayed(false);
          dispatch({ type: "focus/error", epoch, sessionId, token, code: "focus_timeout", message: "Herdr did not confirm this focus request" });
          onTimeout();
        },
      );
    }, (error: unknown) => {
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId || tokenRef.current !== token) return;
      const described = describeError(error, "Could not focus resource");
      intentRef.current = { epoch, token, request, location };
      fallbackCancelRef.current?.();
      fallbackCancelRef.current = null;
      setFocusDelayed(false);
      dispatch({ type: "focus/error", epoch, sessionId, token, code: described.code ?? "focus_error", message: described.message });
    });
  }, [client, describeError, dispatch, mountedRef, onTimeout, stateRef]);

  const retryFocus = useCallback(() => {
    const intent = intentRef.current;
    if (intent) focus(intent.request, intent.location);
  }, [focus]);

  const reconcile = useCallback((state: SessionState, selection: FocusLocation, controlPaneId: string | null): string | null | undefined => {
    const intent = intentRef.current;
    if (state.sync === "live" && intent && intent.epoch === state.epoch && intent.token === state.focusToken && !state.focusPending && !state.focusError) {
      fallbackCancelRef.current?.();
      fallbackCancelRef.current = null;
      setFocusDelayed(false);
      intentRef.current = null;
      return selection.paneId;
    }
    if (!intent && controlPaneId !== null && selection.paneId !== controlPaneId) return null;
    return undefined;
  }, []);

  return { focus, focusDelayed, reconcile, reset, retryFocus, tokenRef };
}

export function focusRequestForSnapshot(snapshot: SessionSnapshotResponse): FocusRequest | null {
  if (snapshot.focused_pane_id) return { kind: "pane", target_id: snapshot.focused_pane_id };
  if (snapshot.focused_tab_id) return { kind: "tab", target_id: snapshot.focused_tab_id };
  if (snapshot.focused_space_id) return { kind: "space", target_id: snapshot.focused_space_id };
  return null;
}
