import { useCallback, useRef, useState, type Dispatch, type MutableRefObject } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { FocusRequest } from "../../protocol/generated/v1";
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
type FocusIntent = { epoch: number; sessionId: string; token: number; request: FocusRequest; location: FocusLocation };

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
  const inFlightRef = useRef(new Map<string, FocusIntent>());
  const queuedIntentRef = useRef<FocusIntent | null>(null);
  const fallbackCancelRef = useRef<(() => void) | null>(null);
  const [focusDelayed, setFocusDelayed] = useState(false);

  const reset = useCallback(() => {
    tokenRef.current += 1;
    intentRef.current = null;
    queuedIntentRef.current = null;
    fallbackCancelRef.current?.();
    fallbackCancelRef.current = null;
    setFocusDelayed(false);
  }, []);

  const isCurrent = (intent: FocusIntent): boolean =>
    mountedRef.current &&
    stateRef.current.epoch === intent.epoch &&
    stateRef.current.sessionId === intent.sessionId &&
    tokenRef.current === intent.token;

  const sendFocus = (intent: FocusIntent): void => {
    inFlightRef.current.set(intent.sessionId, intent);
    void client.focus(intent.sessionId, intent.request).then((response) => {
      if (!isCurrent(intent)) return;
      if (!response.accepted) {
        dispatch({ type: "focus/error", epoch: intent.epoch, sessionId: intent.sessionId, token: intent.token, code: "focus_rejected", message: "Herdr did not accept this focus request" });
        return;
      }
      if (!stateRef.current.focusPending) return;
      fallbackCancelRef.current = scheduleFocusFallback(
        () => isCurrent(intent) && stateRef.current.focusPending !== null,
        () => setFocusDelayed(true),
        () => {
          fallbackCancelRef.current = null;
          setFocusDelayed(false);
          dispatch({ type: "focus/error", epoch: intent.epoch, sessionId: intent.sessionId, token: intent.token, code: "focus_timeout", message: "Herdr did not confirm this focus request" });
          onTimeout();
        },
      );
    }, (error: unknown) => {
      if (!isCurrent(intent)) return;
      const described = describeError(error, "Could not focus resource");
      intentRef.current = intent;
      fallbackCancelRef.current?.();
      fallbackCancelRef.current = null;
      setFocusDelayed(false);
      dispatch({ type: "focus/error", epoch: intent.epoch, sessionId: intent.sessionId, token: intent.token, code: described.code ?? "focus_error", message: described.message });
    }).finally(() => {
      if (inFlightRef.current.get(intent.sessionId) !== intent) return;
      inFlightRef.current.delete(intent.sessionId);
      const queued = queuedIntentRef.current;
      if (!queued || queued.sessionId !== intent.sessionId) return;
      queuedIntentRef.current = null;
      if (isCurrent(queued)) sendFocus(queued);
    });
  };

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
    const intent = { epoch, sessionId, token, request, location };
    intentRef.current = intent;
    dispatch({ type: "focus/request", epoch, sessionId, request, token });
    if (inFlightRef.current.has(sessionId)) {
      queuedIntentRef.current = intent;
      return;
    }
    sendFocus(intent);
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
