import { useCallback, useLayoutEffect, useRef, type MutableRefObject } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { FocusRequest } from "../../protocol/generated/v1";
import { useFocusCoordinator } from "../session/focusCoordinator";
import { useMutationCoordinator } from "../session/mutationCoordinator";
import type { PendingCreation } from "../layout/tabLayoutStore";
import { runtimeSource } from "../layout/reconcile";
import { authoritativeMutationSnapshot, describeError, type Selection } from "./model";
import type { OrderedSession } from "./useOrderedSession";

export function useSessionControl({ client, session, mountedRef, sessionObservation, requestResync }: {
  client: CockpitClient;
  session: OrderedSession;
  mountedRef: MutableRefObject<boolean>;
  sessionObservation: MutableRefObject<number>;
  requestResync(): void;
}) {
  const { stateRef, layouts, selection, tabLayout, echoAccess, dispatchOrdered, drainLayoutEffects } = session;
  const splitPlacement = useRef<Omit<PendingCreation, "token"> | null>(null);
  const { focus, panePrepared, reconcile: reconcileFocus, reset: resetFocus, retryFocus, tokenRef: focusTokenRef, getEchoes, consumeEcho, supersedeSelection } = useFocusCoordinator({
    client, stateRef, mountedRef, dispatch: dispatchOrdered, describeError, onTimeout: requestResync,
    onIntent: echo => layouts.dispatch({ type: "focus/register", ...echo }),
  });
  echoAccess.current = { get: getEchoes, consume: consumeEcho, supersede: supersedeSelection };
  const focusAndSelect = useCallback((request: FocusRequest, location: Selection, prepare?: { paneId: string }) => {
    if (location.tabId) {
      layouts.dispatch({ type: "activate-tab", tabId: location.tabId });
      if ((request.kind === "pane" || request.kind === "agent") && location.paneId) layouts.dispatch({ type: "select-leaf", tabId: location.tabId, leafId: location.paneId });
    }
    focus(request, location, prepare);
  }, [focus, layouts.dispatch]);
  const selectLeaf = useCallback((tabId: string, leafId: string) => {
    const tab = layouts.getState().tabs[tabId];
    if (!tab) return;
    if (tab.selectedLeafId !== leafId) layouts.dispatch({ type: "select-leaf", tabId, leafId });
    if (tab.terminals[leafId]) {
      const pending = stateRef.current.focusPending;
      if (!pending || ((pending.kind !== "pane" && pending.kind !== "agent") || pending.target_id !== leafId)) focus({ kind: "pane", target_id: leafId }, { spaceId: tab.spaceId, tabId, paneId: leafId });
    } else {
      const pending = stateRef.current.focusPending;
      // A remembered viewer can take DOM focus while its tab is preparing.
      // Reasserting that existing selection must not cancel the tab request.
      if (tab.selectedLeafId !== leafId || pending?.kind !== "tab" || pending.target_id !== tabId) supersedeSelection();
      requestAnimationFrame(() => {
        const host = Array.from(document.querySelectorAll<HTMLElement>("[data-leaf-id]")).find(element => element.dataset.leafId === leafId);
        if (!host || host.contains(document.activeElement) || document.activeElement?.closest('dialog[open], [role="dialog"][aria-modal="true"]') || layouts.getState().tabs[tabId]?.selectedLeafId !== leafId) return;
        (host.querySelector<HTMLElement>('.context-document, .review-diff, .browser-surface, input, [tabindex="0"]') ?? host).focus({ preventScroll: true });
      });
    }
  }, [focus, supersedeSelection, layouts.dispatch, layouts.getState]);
  const priorSelection = useRef<{ tabId: string | null; leafId: string | null; terminal: boolean }>({ tabId: null, leafId: null, terminal: false });
  useLayoutEffect(() => {
    const terminal = Boolean(selection.paneId && tabLayout?.terminals[selection.paneId]);
    const prior = priorSelection.current;
    priorSelection.current = { tabId: selection.tabId, leafId: selection.paneId, terminal };
    if (selection.tabId === prior.tabId && selection.paneId !== prior.leafId) {
      if (!terminal) supersedeSelection();
      else if (!prior.terminal && selection.paneId && tabLayout) focus({ kind: "pane", target_id: selection.paneId }, { spaceId: tabLayout.spaceId, tabId: tabLayout.tabId, paneId: selection.paneId });
    }
  }, [selection.tabId, selection.paneId, tabLayout, focus, supersedeSelection]);
  const { mutate, reset: resetMutations, retry: retryMutation, state: mutations, tokenRef: mutationTokenRef } = useMutationCoordinator({
    client, stateRef, mountedRef, sessionObservationRef: sessionObservation, dispatchSession: dispatchOrdered,
    describeError, mutationSnapshot: authoritativeMutationSnapshot, onResync: requestResync,
    onBegin: operation => {
      if (operation.request.type !== "pane_split") return;
      const placement = splitPlacement.current;
      splitPlacement.current = null;
      if (placement) layouts.dispatch({ type: "creation/begin", creation: { ...placement, token: operation.token } });
    },
    onSettled: (operation, created, snapshot, responseIsCurrent) => {
      if (operation.epoch !== stateRef.current.epoch || operation.request.type !== "pane_split") return;
      if (snapshot && responseIsCurrent) layouts.dispatch({ type: "snapshot", snapshot, echoes: getEchoes(), sync: "live" });
      layouts.dispatch({ type: "creation/settled", token: operation.token, created });
      drainLayoutEffects();
    },
  });
  const split = useCallback((tabId: string, leafId: string, direction: "right" | "down") => {
    const tab = layouts.getState().tabs[tabId];
    const sourcePaneId = tab ? runtimeSource(tab, leafId, stateRef.current.snapshot?.focused_pane_id) : null;
    if (!tab || !sourcePaneId || stateRef.current.sync !== "live") return;
    splitPlacement.current = { tabId, placeBeside: leafId, dir: direction === "right" ? "row" : "col", sourcePaneId };
    if (!mutate(`pane:${sourcePaneId}`, { type: "pane_split", pane_id: sourcePaneId, direction, ratio: null }, true)) splitPlacement.current = null;
  }, [layouts.getState, mutate]);
  useLayoutEffect(drainLayoutEffects, [layouts.state, drainLayoutEffects]);
  return { focusAndSelect, selectLeaf, split, panePrepared, retryFocus, reconcileFocus, resetFocus, focusTokenRef, mutate, retryMutation, resetMutations, mutations, mutationTokenRef };
}
