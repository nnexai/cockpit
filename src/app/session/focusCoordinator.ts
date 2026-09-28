import { useCallback, useRef, useState, type Dispatch, type MutableRefObject } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { FocusRequest } from "../../protocol/generated/v1";
import type { SessionAction, SessionState } from "./sessionStore";

export const FOCUS_FALLBACK_MS = 500;
/** How long a tab switch waits for the target pane to attach at its own size before Herdr is asked to focus it anyway. */
export const FOCUS_PREPARE_MS = 300;

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
  const prepareGateRef = useRef<{ token: number; paneId: string; release: () => void } | null>(null);
  const [focusDelayed, setFocusDelayed] = useState(false);

  const reset = useCallback(() => {
    tokenRef.current += 1;
    intentRef.current = null;
    queuedIntentRef.current = null;
    prepareGateRef.current = null;
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

  /** With `prepare`, the focus request waits until that pane has attached and painted its first frame (or the wait times out), so its size settles before Herdr shows the tab. */
  const focus = useCallback((request: FocusRequest, location: FocusLocation, prepare?: { paneId: string }) => {
    const current = stateRef.current;
    const sessionId = current.sessionId;
    if (!sessionId) return;
    const epoch = current.epoch;
    const token = tokenRef.current + 1;
    tokenRef.current = token;
    fallbackCancelRef.current?.();
    fallbackCancelRef.current = null;
    prepareGateRef.current = null;
    setFocusDelayed(false);
    const intent = { epoch, sessionId, token, request, location };
    intentRef.current = intent;
    dispatch({ type: "focus/request", epoch, sessionId, request, token });
    const dispatchIntent = () => {
      if (!isCurrent(intent)) return;
      if (inFlightRef.current.has(sessionId)) {
        queuedIntentRef.current = intent;
        return;
      }
      sendFocus(intent);
    };
    if (!prepare) {
      dispatchIntent();
      return;
    }
    let released = false;
    const release = () => {
      if (released) return;
      released = true;
      globalThis.clearTimeout(timer);
      if (prepareGateRef.current?.token === token) prepareGateRef.current = null;
      dispatchIntent();
    };
    const timer = globalThis.setTimeout(release, FOCUS_PREPARE_MS);
    prepareGateRef.current = { token, paneId: prepare.paneId, release };
  }, [client, describeError, dispatch, mountedRef, onTimeout, stateRef]);

  /** Reports that a pane painted its first frame; releases a focus request waiting on it. */
  const panePrepared = useCallback((paneId: string) => {
    const gate = prepareGateRef.current;
    if (gate && gate.paneId === paneId) gate.release();
  }, []);

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

  return { focus, focusDelayed, panePrepared, reconcile, reset, retryFocus, tokenRef };
}
