import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient, CockpitStatus } from "../client/CockpitClient";
import type { SessionSummary } from "../protocol/generated/v1";
import { releaseViewers } from "./layout/viewerLifecycle";
import { LibraryView } from "./library/LibraryView";
import { CompatibilityNotice, OpenLibraryButton } from "./shell/CompatibilityNotice";
import { describeError, type StatusError } from "./shell/model";
import { Workbench } from "./shell/Workbench";
import { useOrderedSession } from "./shell/useOrderedSession";
import { useSessionControl } from "./shell/useSessionControl";
import { useAutoResync, useResyncControl } from "./shell/useSessionRecovery";
import { useSessionStream, type SessionStreamHandle } from "./shell/useSessionStream";

export function App({ client }: { client: CockpitClient }) {
  const [status, setStatus] = useState<CockpitStatus | null>(null);
  const [statusError, setStatusError] = useState<StatusError | null>(null);
  const [statusAttempt, setStatusAttempt] = useState(0);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [sessionsAttempt, setSessionsAttempt] = useState(0);
  const [sessionsLoaded, setSessionsLoaded] = useState(false);
  const [sessionsError, setSessionsError] = useState<StatusError | null>(null);
  const session = useOrderedSession(client);
  const { state, tabLayout, selection, ctx, ctxRef, registerTransient, dispatchOrdered, announcement, setAnnouncement, lifecycleError, setLifecycleError } = session;
  const sessionStream = useRef<SessionStreamHandle | null>(null);
  const sessionObservation = useRef(0);
  const sessionListRequest = useRef(0);
  const mountedRef = useRef(true);
  const recovery = useResyncControl();
  const { resyncAttempt, recoveryResyncRef, autoResyncAttempts, clearRecoveryTimers, requestResync } = recovery;
  const { focusAndSelect, selectLeaf, split, panePrepared, retryFocus, reconcileFocus, resetFocus, focusTokenRef, mutate, retryMutation, resetMutations, mutations, mutationTokenRef } = useSessionControl({ client, session, mountedRef, sessionObservation, requestResync });
  const resetSessionRuntime = useCallback(() => {
    sessionObservation.current += 1;
    sessionStream.current?.close();
    sessionStream.current = null;
    resetFocus();
    clearRecoveryTimers();
    autoResyncAttempts.current = 0;
    recoveryResyncRef.current = false;
    resetMutations();
  }, [clearRecoveryTimers, resetFocus, resetMutations]);
  const switchSession = useCallback((id: string) => {
    resetSessionRuntime();
    const outgoing = ctxRef.current;
    void releaseViewers(outgoing, "all").catch(error => setLifecycleError(describeError(error, "Could not release viewers").message));
    dispatchOrdered({ type: "switch", sessionId: id });
  }, [resetSessionRuntime, dispatchOrdered]);
  const refreshSessions = useCallback(async () => {
    const request = ++sessionListRequest.current;
    setSessionsError(null);
    try {
      const response = await client.sessions();
      if (!mountedRef.current || sessionListRequest.current !== request) return;
      setSessions(response.sessions);
      setSessionsLoaded(true);
    } catch (error: unknown) {
      if (!mountedRef.current || sessionListRequest.current !== request) return;
      setSessionsError(describeError(error, "Could not list Herdr sessions"));
      setSessionsLoaded(true);
    }
  }, [client]);
  useEffect(() => { mountedRef.current = true; return () => { mountedRef.current = false; clearRecoveryTimers(); resetFocus(); }; }, [clearRecoveryTimers, resetFocus]);
  useEffect(() => { let active = true; setStatus(null); setStatusError(null); void client.status().then((next) => { if (active) setStatus(next); }, (error: unknown) => { if (active) setStatusError(describeError(error, "Could not read Cockpit status")); }); return () => { active = false; }; }, [client, statusAttempt]);
  const compatible = status?.herdr.status === "compatible";
  const sessionAvailable = sessionsLoaded && sessions.some((session) => session.id === state.sessionId);
  useEffect(() => {
    if (!compatible) return;
    void refreshSessions();
    return () => { sessionListRequest.current += 1; };
  }, [refreshSessions, compatible, statusAttempt, sessionsAttempt]);
  useEffect(() => {
    if (!compatible || !sessionsLoaded) return;
    if (sessions.length === 0) {
      resetSessionRuntime();
      return;
    }
    if (!state.sessionId || !sessions.some((session) => session.id === state.sessionId)) {
      const preferred = sessions.find((session) => session.is_default) ?? sessions[0];
      switchSession(preferred.id);
    }
  }, [compatible, sessionsLoaded, sessions, state.sessionId, resetSessionRuntime, switchSession]);
  useSessionStream({ client, session, compatible, sessionAvailable, resyncAttempt, recoveryResyncRef, focusTokenRef, mutationTokenRef, retryFocus, sessionObservation, sessionStream });
  useAutoResync(recovery, session, mountedRef);
  useEffect(() => {
    reconcileFocus(state, selection, selection.paneId && tabLayout?.terminals[selection.paneId] ? selection.paneId : null);
  }, [state.snapshot, state.sync, state.epoch, state.focusPending, state.focusToken, state.focusError, selection.paneId, tabLayout, reconcileFocus]);

  // With no session the Library opens full-screen in place of the notice screens.
  const [noSessionLibraryOpen, setNoSessionLibraryOpen] = useState(false);
  const returnToLibraryOpener = useRef(false);
  useEffect(() => {
    if (noSessionLibraryOpen || !returnToLibraryOpener.current) return;
    returnToLibraryOpener.current = false;
    document.querySelector<HTMLElement>("[data-library-opener]")?.focus({ preventScroll: true });
  }, [noSessionLibraryOpen]);
  const openNoSessionLibrary = () => setNoSessionLibraryOpen(true);
  const noSessionLibrary = noSessionLibraryOpen
    ? <div className="app-shell"><LibraryView client={client} fullScreen onClose={() => { returnToLibraryOpener.current = true; setNoSessionLibraryOpen(false); }} /></div>
    : null;
  const explicitResync = () => {
    clearRecoveryTimers();
    autoResyncAttempts.current = 0;
    void refreshSessions();
    requestResync();
  };
  if ((!status || !compatible) && (statusError || status) && noSessionLibrary) return noSessionLibrary;
  if (!status || !compatible) return <div className="app-shell">{statusError || (status && !compatible) ? <CompatibilityNotice status={status} error={statusError} retry={() => setStatusAttempt((value) => value + 1)} onOpenLibrary={openNoSessionLibrary} /> : <main className="compatibility-main" aria-live="polite"><section className="notice notice-loading" role="status"><p className="eyebrow">Cockpit</p><h1>Connecting to Herdr</h1><p>Reading compatibility status...</p></section></main>}</div>;
  if (((sessionsError && sessions.length === 0) || (sessionsLoaded && sessions.length === 0)) && noSessionLibrary) return noSessionLibrary;
  if (sessionsError && sessions.length === 0) return <div className="app-shell"><CompatibilityNotice status={status} error={sessionsError} retry={() => setSessionsAttempt((value) => value + 1)} onOpenLibrary={openNoSessionLibrary} /></div>;
  if (sessionsLoaded && sessions.length === 0) return <div className="app-shell"><main className="compatibility-main"><section className="notice"><h1>No Herdr sessions</h1><p>Create or start a session, then refresh the list.</p><div className="notice-actions"><button type="button" className="action-button" onClick={() => setSessionsAttempt((value) => value + 1)}>Refresh sessions</button><OpenLibraryButton onOpen={openNoSessionLibrary} /></div></section></main></div>;
  return <div className="app-shell"><div className="sr-only" role="status" aria-live="polite">{announcement}</div><Workbench key={state.epoch} client={client} state={state} sessions={sessions} selection={selection} terminalMouseInput={status.capabilities.terminal_mouse_input} mutations={mutations} ctx={ctx} tabLayout={tabLayout} registerTransient={registerTransient} onSession={switchSession} onFocus={focusAndSelect} onSelectLeaf={selectLeaf} onSplit={split} onPanePrepared={panePrepared} onReconnect={explicitResync} onRetry={retryFocus} onRefreshSessions={refreshSessions} onOpenSession={() => { void refreshSessions(); }} onMutate={mutate} onRetryMutation={retryMutation} layoutError={lifecycleError} onDismissLayoutError={() => setLifecycleError(null)} onWidgetAnnouncement={setAnnouncement} /></div>;
}
