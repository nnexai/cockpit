import { useCallback, useReducer, useRef, useState, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import { initialSessionState, sessionReducer, type SessionAction, type SessionState } from "../session/sessionStore";
import { useTabLayouts, type LeafCtx, type TabLayouts, type TabLayoutState } from "../layout/tabLayoutStore";
import type { FocusEcho } from "../layout/reconcile";
import { getViewerClientId, releaseViewers } from "../layout/viewerLifecycle";
import { retireTabBrowser } from "../layout/browserLifecycle";
import { describeError, type Selection } from "./model";

export interface OrderedSession {
  state: SessionState;
  stateRef: MutableRefObject<SessionState>;
  layouts: TabLayouts;
  tabLayout: TabLayoutState | null;
  selection: Selection;
  ctx: LeafCtx;
  ctxRef: MutableRefObject<LeafCtx>;
  registerTransient(cancel: () => void): () => void;
  echoAccess: MutableRefObject<{ get(): FocusEcho[]; consume(token: number): void; supersede(): void }>;
  drainLayoutEffects(): void;
  dispatchOrdered(action: SessionAction): void;
  announcement: string;
  setAnnouncement: Dispatch<SetStateAction<string>>;
  lifecycleError: string | null;
  setLifecycleError: Dispatch<SetStateAction<string | null>>;
}

/** Own both reducers and deliver each authoritative observation to layouts before effects drain. */
export function useOrderedSession(client: CockpitClient): OrderedSession {
  const [state, dispatch] = useReducer(sessionReducer, initialSessionState);
  const layouts = useTabLayouts(state.sessionId ?? "");
  const tabLayout = layouts.state.activeTabId ? layouts.state.tabs[layouts.state.activeTabId] ?? null : null;
  const selection: Selection = { spaceId: layouts.state.activeSpaceId, tabId: layouts.state.activeTabId, paneId: tabLayout?.selectedLeafId ?? null };
  const ctx: LeafCtx = { client, sessionId: state.sessionId ?? "", serverInstance: layouts.state.serverInstance, clientId: getViewerClientId(), getState: layouts.getState, dispatch: layouts.dispatch };
  const ctxRef = useRef(ctx);
  ctxRef.current = ctx;
  const transientCallbacks = useRef(new Set<() => void>());
  const registerTransient = useCallback((cancel: () => void) => { transientCallbacks.current.add(cancel); return () => { transientCallbacks.current.delete(cancel); }; }, []);
  const [announcement, setAnnouncement] = useState("");
  const [lifecycleError, setLifecycleError] = useState<string | null>(null);
  const echoAccess = useRef<{ get(): FocusEcho[]; consume(token: number): void; supersede(): void }>({ get: () => [], consume: () => undefined, supersede: () => undefined });
  const stateRef = useRef(state);
  stateRef.current = state;
  const drainLayoutEffects = useCallback(() => {
    const viewerTabs = new Set<string>();
    for (const effect of layouts.takeEffects()) {
      if (effect.type === "cancel-transient") { transientCallbacks.current.forEach(cancel => cancel()); if (effect.reason === "external-focus") echoAccess.current.supersede(); }
      if (effect.type === "consume-echo") echoAccess.current.consume(effect.token);
      if (effect.type === "announce") setAnnouncement(effect.text);
      if (effect.type === "viewer-release") viewerTabs.add(effect.tabId);
      if (effect.type === "browser-retire") void retireTabBrowser(ctxRef.current, effect.tabId, effect.associationKey, effect.serverInstance).catch(error => setLifecycleError(describeError(error, "Could not retire browser").message));
    }
    if (viewerTabs.size) void releaseViewers(ctxRef.current, [...viewerTabs]).catch(error => setLifecycleError(describeError(error, "Could not release viewers").message));
  }, [layouts.takeEffects]);
  const dispatchOrdered = useCallback((action: SessionAction) => {
    const previous = stateRef.current;
    const next = sessionReducer(previous, action);
    stateRef.current = next;
    dispatch(action);
    if (next.snapshot && next.snapshot !== previous.snapshot
      && (next.sync === "live" || action.type === "snapshot/authoritative")) {
      layouts.dispatch({ type: "snapshot", snapshot: next.snapshot, echoes: echoAccess.current.get(), sync: "live" });
      drainLayoutEffects();
    }
  }, [layouts.dispatch, drainLayoutEffects]);
  return { state, stateRef, layouts, tabLayout, selection, ctx, ctxRef, registerTransient, echoAccess, drainLayoutEffects, dispatchOrdered, announcement, setAnnouncement, lifecycleError, setLifecycleError };
}
