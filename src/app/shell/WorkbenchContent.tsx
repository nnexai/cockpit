import type { CSSProperties, ReactNode } from "react";
import { UiIcon } from "../UiIcon";
import { ErrorSlot } from "../ErrorSlot";
import { ServerPopup } from "../ServerPopup";
import { BrowserCleanupNotices } from "../layout/BrowserCleanupNotices";
import { SubscriptionLimits } from "../limits/SubscriptionLimits";
import { armedPrefixHint } from "../input/shortcuts";
import { SetupDialog } from "../projects/SetupDialog";
import { TeardownDialog } from "../projects/TeardownDialog";
import { TeardownRecoveryPanel } from "../projects/TeardownRecoveryPanel";
import { AddContextDialog } from "../library/AddContextDialog";
import { LibraryView } from "../library/LibraryView";
import { SupervisorView } from "../supervisor/SupervisorView";
import { NotesView } from "../notes/NotesView";
import { Sidebar } from "../sidebar/Sidebar";
import { spaceNotesFromFailures } from "../sidebar/Spaces";
import { byId } from "./model";
import { TabStrip } from "./TabStrip";
import { CommandOverlay } from "./CommandOverlay";
import { PaneDialogOverlay } from "./PaneDialogOverlay";
import { SessionDialogOverlay } from "./SessionDialogOverlay";
import { RecoveryPanel } from "./RecoveryPanel";
import { ResourceContextMenu } from "./ResourceContextMenu";
import { WorkbenchCanvas } from "./WorkbenchCanvas";
import { SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_DEFAULT_WIDTH } from "./useSidebarState";
import type { CommandAction } from "./commands";
import type { WorkbenchProps } from "./Workbench";
import type { WorkbenchState } from "./useWorkbenchState";
import type { WorkbenchActions } from "./useWorkbenchActions";

export type WorkbenchContentProps = WorkbenchProps & WorkbenchState & WorkbenchActions & {
  commandActions: CommandAction[];
  commandFailureMessage: string;
  commandStatus?: ReactNode;
};

export function WorkbenchContent(props: WorkbenchContentProps) {
  const {
    client, state, sessions, selection, terminalMouseInput, mutations, ctx, onSession, onReconnect, onRefreshSessions, onMutate, onRetryMutation, layoutError, onDismissLayoutError,
    snapshot, shell, popup, popupPending, sidebarWidth, sidebarCollapsed, narrowViewport, drawerOpen, sidebarSession, sidebarCloseRef, spaces, spaceGit, gitBlocked, gitActions, gitPoll, editing, mutationBusy, modalOpen, allTabs,
    closeDrawer, openSessionChooser, setEditing, focusSpace, openContext, setSetupOpen, focusAgent, updateSidebarWidth, openDrawer, toggleSidebarCollapsed,
    supervisorOpen, closeSupervisor, openSupervisor, widgetDots, widgetsPending, showWidgets, tabs, browserOpen, browserReason, libraryOpen, notesOpen, closeNotes, openNotes, focusTab, toggleBrowser, closeLibrary, openLibrary, setCommandsOpen,
    supervisor, setSupervisorModal, supervisorTerminal, supervisorNavigationError, librarySpace, captureLibraryInvoker, libraryCommand,
    lifecycleError, setLifecycleError, limits, limitsOpen, setLimitsOpen, commandsOpener, dialog, setDialog, panes, localLeaves, swap, lastTerminalMessage,
    commandsOpen, commandActions, commandStatus, setSessionChooserOpen, libraryAddOpen, setLibraryAddOpen, sessionChooserOpen, setupOpen, setupParent, recoveryOpen, setRecoveryOpen, teardownSpaceId, setTeardownSpaceId,
    prefixActive, armedPrefix, commandNotice, commandFailureMessage, gitNotice, prefixHint,
  } = props;
  const workbenchStyle: CSSProperties & { "--sidebar-width": string } = { "--sidebar-width": `${sidebarWidth}px` };
  const sidebarClass = "sidebar";
  // Selection chrome follows Herdr's acknowledgement: while a focus request is in flight the sidebar keeps the confirmed row and marks the target as pending.
  const pendingSpaceId = state.focusPending?.kind === "space" ? state.focusPending.target_id : null;
  const pendingPaneId = state.focusPending?.kind === "pane" || state.focusPending?.kind === "agent" ? state.focusPending.target_id : null;
  const sidebarSelectedSpaceId = state.focusPending ? snapshot?.focused_space_id ?? null : selection.spaceId;
  const sidebarSelectedPaneId = state.focusPending ? snapshot?.focused_pane_id ?? null : selection.paneId;
  return <div className={`workbench${sidebarCollapsed ? " sidebar-collapsed" : ""}${narrowViewport && drawerOpen ? " drawer-open" : ""}`} style={workbenchStyle}>
    <div className="workbench-underlay" inert={Boolean(popup)}>
    {narrowViewport && drawerOpen ? <button type="button" className="drawer-scrim" aria-label="Close sidebar" onClick={() => closeDrawer()} /> : null}
    <aside id="cockpit-sidebar" className={sidebarClass} aria-label="Spaces and agents" role={narrowViewport && drawerOpen ? "dialog" : undefined} aria-modal={narrowViewport && drawerOpen ? "true" : undefined} aria-hidden={narrowViewport && !drawerOpen ? "true" : undefined} hidden={narrowViewport ? !drawerOpen : sidebarCollapsed}>
      <Sidebar session={sidebarSession} sync={state.sync} narrow={narrowViewport} onSession={openSessionChooser} onClose={() => closeDrawer()} closeRef={sidebarCloseRef} hasSession={state.sessionId !== null} hasSnapshot={snapshot !== null}
        spaces={{ spaces, gitStatus: spaceGit, gitBlocked, gitEntry: gitActions.forStatus, gitStatusError: gitPoll.error, onRetryGitStatus: gitPoll.refresh, onGitAction: props.runGitAction, onDismissGitProblem: gitActions.dismiss, selectedSpaceId: sidebarSelectedSpaceId, pendingSpaceId, editingId: editing?.kind === "space" ? editing.id : null, busy: mutationBusy, notes: spaceNotesFromFailures(Object.values(mutations.errors)), onEdit: (id) => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "space", id } : null); }, onSelect: focusSpace, onContext: openContext, onSetup: () => setSetupOpen(true), setupEnabled: state.sync === "live" && !modalOpen, mutate: onMutate }}
        agents={{ agents: snapshot?.agents ?? [], spaces, tabs: allTabs, selectedPaneId: sidebarSelectedPaneId, pendingPaneId, onSelect: focusAgent }} />
    </aside>
    {!narrowViewport && !sidebarCollapsed ? <div className="sidebar-resizer" role="separator" tabIndex={sidebarCollapsed ? -1 : 0} aria-label="Resize sidebar" aria-orientation="vertical" aria-valuemin={SIDEBAR_MIN_WIDTH} aria-valuemax={SIDEBAR_MAX_WIDTH} aria-valuenow={sidebarWidth}
      onKeyDown={(event) => { if (sidebarCollapsed) return; if (event.key === "Home") { event.preventDefault(); updateSidebarWidth(SIDEBAR_DEFAULT_WIDTH); } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") { event.preventDefault(); updateSidebarWidth(sidebarWidth + (event.key === "ArrowLeft" ? -8 : 8)); } }}
      onPointerDown={(event) => { if (sidebarCollapsed || event.button !== 0) return; event.preventDefault(); const start = event.clientX; const width = sidebarWidth; const move = (next: PointerEvent) => updateSidebarWidth(width + next.clientX - start); const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); }; window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop); }} /> : null}
    <main className="main-workarea">
      {!selection.spaceId ? <button type="button" className="drawer-toggle" aria-expanded={drawerOpen} aria-controls="cockpit-sidebar" aria-label="Open sidebar" onClick={narrowViewport ? openDrawer : toggleSidebarCollapsed}><UiIcon name="sidebar" /> <span>Sidebar</span></button> : null}
      {selection.spaceId ? <TabStrip supervisorOpen={supervisorOpen} onSupervisor={() => { if (supervisorOpen) closeSupervisor(); else openSupervisor(); }} widgetDots={widgetDots} widgetsPending={widgetsPending} onWidgets={showWidgets} sidebarOpen={narrowViewport ? drawerOpen : !sidebarCollapsed} onToggleSidebar={narrowViewport ? (drawerOpen ? () => closeDrawer() : openDrawer) : toggleSidebarCollapsed} tabs={tabs} selectedTabId={selection.tabId} editingId={editing?.kind === "tab" ? editing.id : null} busy={mutationBusy} browserOpen={browserOpen} browserDisabledReason={browserOpen ? null : browserReason} libraryOpen={libraryOpen} notesOpen={notesOpen} onNotesToggle={notesOpen ? closeNotes : openNotes} onEdit={id => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "tab", id } : null); }} onSelect={focusTab} onContext={openContext} onCreate={() => { if (selection.spaceId) onMutate("tab:new", { type: "tab_create", space_id: selection.spaceId, label: null }, true); }} onBrowserToggle={toggleBrowser} onLibraryToggle={() => { if (libraryOpen) closeLibrary(); else openLibrary(); }} onCommands={() => setCommandsOpen(true)} mutate={onMutate} /> : <div className="tab-toolbar"><button type="button" className="tab-strip-action" onClick={() => openSupervisor()}>Supervisor</button><button type="button" className="tab-strip-action" onClick={() => setCommandsOpen(true)}>Commands</button></div>}
      <div className="workarea-content">
        {supervisor && state.sessionId ? <SupervisorView key={state.sessionId} client={client} sessionId={state.sessionId} session={snapshot} runtimeLive={state.sync === "live"} active={supervisorOpen} startToken={supervisor.startSessionId === state.sessionId ? supervisor.startToken : 0} onClose={closeSupervisor} onModalChange={setSupervisorModal} onTerminal={(run, orchestration) => supervisorTerminal(orchestration.runtime.status === "fresh" ? orchestration.runtime.runs.find(observation => observation.run_id === run.run_id)?.pane_id ?? "" : "", run, orchestration)} navigationError={supervisorNavigationError} /> : null}
        {supervisorOpen ? null : notesOpen ? <NotesView key={`${state.sessionId}:${selection.spaceId}`} client={client} space={librarySpace} onClose={closeNotes} /> : libraryOpen ? <LibraryView client={client} onClose={closeLibrary} onCaptureInvoker={captureLibraryInvoker} command={libraryCommand} space={librarySpace} /> : <WorkbenchCanvas {...props} />}
      </div>
      <div className="workarea-statusbar">
      <BrowserCleanupNotices ctx={ctx} activeTabId={selection.tabId} fallback={<ErrorSlot placement="pane" message={[lifecycleError, layoutError].filter(Boolean).join(" · ")} actions={lifecycleError || layoutError ? <>
        <button type="button" onClick={onReconnect}>Resync</button>
        <button type="button" onClick={() => { setLifecycleError(null); onDismissLayoutError(); }}>Dismiss</button>
      </> : null} />} />
      {!limits.absent ? <SubscriptionLimits snapshot={limits.snapshot} link={limits.link} now={limits.now} open={limitsOpen}
        onOpenChange={setLimitsOpen} suspended={modalOpen} commandOpener={commandsOpener} /> : null}
      </div>
    </main>
    <ResourceContextMenu {...props} />
    {dialog ? <PaneDialogOverlay dialog={dialog} panes={panes} tabs={allTabs} spaces={spaces} busy={mutationBusy} onDismiss={() => setDialog(null)} mutate={onMutate} leafChoices={localLeaves.map(leaf => ({ id: leaf.id, title: byId(panes, leaf.id)?.title || leaf.kind }))} onSwap={swap} confirmMove={pane => { const message = lastTerminalMessage(pane, "Moving"); return !message || window.confirm(message); }} /> : null}
    {commandsOpen ? <CommandOverlay actions={commandActions.map((action) => ({ ...action, run: () => { setCommandsOpen(false); action.run(); } }))} statusContent={commandStatus} onSwitchSession={() => { setCommandsOpen(false); void onRefreshSessions().catch(() => undefined).finally(() => setSessionChooserOpen(true)); }} onDismiss={() => setCommandsOpen(false)} /> : null}
    {libraryAddOpen ? <AddContextDialog client={client} onClose={() => setLibraryAddOpen(false)} onOpenItem={(itemId) => openLibrary({ kind: "open", itemId })} space={librarySpace} /> : null}
    {sessionChooserOpen ? <SessionDialogOverlay sessions={sessions} currentSessionId={state.sessionId} onRefresh={onRefreshSessions} onSession={onSession} onDismiss={() => setSessionChooserOpen(false)} /> : null}
    {state.sessionId ? <SetupDialog client={client} sessionId={state.sessionId} open={setupOpen} selectedParent={setupParent} parentSpaceId={selection.spaceId} onClose={() => setSetupOpen(false)} onCompleted={onReconnect} /> : null}
    {state.sessionId ? <TeardownRecoveryPanel client={client} sessionId={state.sessionId} open={recoveryOpen} onClose={() => setRecoveryOpen(false)} /> : null}
    {state.sessionId && teardownSpaceId ? <TeardownDialog client={client} sessionId={state.sessionId} workspaceId={teardownSpaceId} open onClose={() => setTeardownSpaceId(null)} onCompleted={onReconnect} /> : null}
    {prefixActive ? <div className="prefix-indicator" role="status"><span>{armedPrefix.label} · {armedPrefix.origin === "herdr" ? "Herdr commands · Esc cancels" : armedPrefixHint()}</span></div> : commandNotice || commandFailureMessage ? <div className="prefix-indicator is-notice" role="alert">{commandNotice || commandFailureMessage}</div> : gitNotice && gitNotice.sessionId === state.sessionId ? <div className="prefix-indicator is-notice" role="alert">{gitNotice.text}</div> : popupPending ? <div className="prefix-indicator" role="status">Opening Herdr popup…</div> : prefixHint ? <div className="prefix-indicator is-notice" role="status">{prefixHint}</div> : null}
    <RecoveryPanel state={state} mutations={mutations} onReconnect={onReconnect} onRetryMutation={onRetryMutation} />
    </div>
    {popup && state.sessionId ? <ServerPopup key={`${state.sessionId}:${popup.terminal_id}`} client={client} sessionId={state.sessionId} popup={popup} live={state.sync === "live" && shell?.status === "live"} error={shell?.error ?? state.syncError?.message ?? null} focusEpoch={state.epoch} terminalMouseInput={terminalMouseInput} onReconnect={onReconnect} /> : null}
  </div>;
}
