import { useCallback, useEffect, useRef, useState } from "react";
import { CockpitClientError, type CockpitClient } from "../client/CockpitClient";
import type { ContextSplitDirection, PanePresentation } from "../protocol/generated/v1";
import { createContextViewState, type ContextViewState } from "./context/ContextViewer";

export type PaneRendererState = {
  presentation: PanePresentation;
  choice: "context" | "review" | "terminal" | null;
  view: ContextViewState;
  inspectionError: string | null;
  actionError: string | null;
  outcomeUnknown: boolean;
};

export function isGraphicalContext(state: PaneRendererState | undefined): boolean {
  return state?.presentation.renderer === "context" && state.choice !== "terminal";
}

export function isGraphicalReview(state: PaneRendererState | undefined): boolean {
  return state?.presentation.renderer === "review" && state.choice !== "terminal";
}


export function usePaneRenderers(
  client: CockpitClient,
  sessionId: string | null,
  visiblePaneIds: string[],
  allPaneIds: string[],
  live: boolean,
  refreshEpoch: number,
  onResync: () => void,
) {
  const [panes, setPanes] = useState<Record<string, PaneRendererState>>({});
  const [busy, setBusy] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const currentSession = useRef(sessionId);
  currentSession.current = sessionId;
  const launching = useRef(false);
  const panesRef = useRef(panes);
  panesRef.current = panes;
  const visibleKey = visiblePaneIds.join("\0");
  const allKey = allPaneIds.join("\0");

  useEffect(() => { setPanes({}); }, [sessionId]);
  useEffect(() => {
    if (!live) return;
    const existing = new Set(allPaneIds);
    setPanes((current) => {
      const removed = Object.keys(current).filter((id) => !existing.has(id));
      if (removed.length === 0) return current;
      const next = { ...current };
      for (const id of removed) delete next[id];
      return next;
    });
  }, [allKey, live]);

  const accept = useCallback((presentation: PanePresentation, signal: AbortSignal) => {
    if (signal.aborted || presentation.session_id !== currentSession.current) return;
    setPanes((current) => {
      if (signal.aborted || presentation.session_id !== currentSession.current) return current;
      const previous = current[presentation.pane_id];
      const retained = previous?.presentation.binding_id === presentation.binding_id;
      return { ...current, [presentation.pane_id]: {
        presentation,
        choice: retained ? previous.choice : null,
        view: retained ? previous.view : createContextViewState(),
        inspectionError: null,
        actionError: retained ? previous.actionError : null,
        outcomeUnknown: retained ? previous.outcomeUnknown : false,
      } };
    });
  }, []);

  useEffect(() => {
    if (!sessionId || !live || visiblePaneIds.length === 0) return;
    const abort = new AbortController();
    let timer: number | undefined;
    const poll = async () => {
      for (let index = 0; index < visiblePaneIds.length && !abort.signal.aborted; index += 3) {
        await Promise.all(visiblePaneIds.slice(index, index + 3).map(async (paneId) => {
          try {
            const presentation = await client.inspectPane(sessionId, paneId, abort.signal);
            accept(presentation, abort.signal);
          } catch (error) {
            if (abort.signal.aborted) return;
            setPanes((current) => {
              if (abort.signal.aborted || currentSession.current !== sessionId) return current;
              const previous = current[paneId];
              return previous ? { ...current, [paneId]: { ...previous, inspectionError: error instanceof Error ? error.message : "Context could not be loaded" } } : current;
            });
          }
        }));
      }
      if (!abort.signal.aborted) timer = window.setTimeout(() => { void poll(); }, 2500);
    };
    void poll();
    return () => { abort.abort(); clearTimeout(timer); };
  }, [client, sessionId, visibleKey, allKey, live, refreshEpoch, refresh, accept]);

  const choose = (paneId: string, choice: "context" | "review" | "terminal") => {
    setPanes((current) => {
      const previous = current[paneId];
      if (!previous || (choice === "context" && previous.presentation.renderer !== "context")) return current;
      return { ...current, [paneId]: { ...previous, choice } };
    });
  };

  const updateView = (paneId: string, bindingId: string, view: ContextViewState) => {
    setPanes((current) => {
      const previous = current[paneId];
      if (!previous || previous.presentation.binding_id !== bindingId) return current;
      return { ...current, [paneId]: { ...previous, view } };
    });
  };

  const open = async (paneId: string, direction: ContextSplitDirection, kind: "context" | "review" | "files" = "context") => {
    if (!sessionId || !live || launching.current) return;
    const previous = panesRef.current[paneId];
    if (previous?.outcomeUnknown && !window.confirm("The previous launch outcome is unknown. Check the existing panes before opening another pane. Open another?")) return;
    const requestedSession = sessionId;
    let requestedBinding = previous?.presentation.binding_id;
    launching.current = true;
    setBusy(true);
    try {
      const presentation = await client.inspectPane(sessionId, paneId);
      if (currentSession.current !== requestedSession) return;
      requestedBinding = presentation.binding_id;
      if (kind === "review") {
        const root = presentation.roots.find(candidate => candidate.root_id === presentation.default_root_id)
          ?? presentation.roots.find(candidate => candidate.kind === "repository");
        if (!root || !presentation.can_open_review) throw new Error(presentation.reason || "Review requires an enabled Reviewr plugin and a Git checkout");
        await client.openReview(sessionId, { pane_id: paneId, binding_id: presentation.binding_id, repository_id: root.repository_id, direction });
      } else {
        const root = kind === "files"
          ? presentation.roots.find(candidate => candidate.root_id === presentation.files_root_id)
          : presentation.roots.find(candidate => candidate.root_id === presentation.default_root_id);
        const available = kind === "files" ? presentation.can_open_files : presentation.can_open_context;
        if (!root || !available) throw new Error(presentation.reason || "Files require an enabled file-viewer plugin and a verified browsing root");
        await client.openContext(sessionId, { pane_id: paneId, binding_id: presentation.binding_id, root_id: root.root_id, direction });
      }
      if (currentSession.current !== requestedSession) return;
      setPanes((current) => {
        const source = current[paneId];
        if (currentSession.current !== requestedSession || source?.presentation.binding_id !== requestedBinding) return current;
        return source ? { ...current, [paneId]: { ...source, actionError: null, outcomeUnknown: false } } : current;
      });
      onResync();
    } catch (error) {
      if (currentSession.current !== requestedSession) return;
      setPanes((current) => {
        const pane = current[paneId];
        if (!pane || currentSession.current !== requestedSession || pane.presentation.binding_id !== requestedBinding) return current;
        const code = error instanceof CockpitClientError ? error.operationCode : undefined;
        return { ...current, [paneId]: { ...pane, actionError: error instanceof Error ? error.message : "Context could not be opened", outcomeUnknown: code === "request_outcome_unknown" || code === "mutation_applied_snapshot_failed" } };
      });
    } finally {
      launching.current = false;
      setBusy(false);
      setRefresh((value) => value + 1);
    }
  };

  return { panes, busy, choose, updateView, open, refresh: () => setRefresh((value) => value + 1) };
}
