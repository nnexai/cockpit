import type { CSSProperties, ReactNode } from "react";
import { UiIcon } from "../UiIcon";
import { ErrorSlot } from "../ErrorSlot";
import { ServerPopup } from "../ServerPopup";
import { BrowserCleanupNotices } from "../layout/BrowserCleanupNotices";
import { SubscriptionLimits } from "../limits/SubscriptionLimits";
import { armedPrefixHint, withShortcut } from "../input/shortcuts";
import { SetupDialog } from "../projects/SetupDialog";
import { TeardownDialog } from "../projects/TeardownDialog";
import { TeardownRecoveryPanel } from "../projects/TeardownRecoveryPanel";
import { AddContextDialog } from "../library/AddContextDialog";
import { LibraryView } from "../library/LibraryView";
import { SupervisorView } from "../supervisor/SupervisorView";
import { ConfirmedRecoveryDialog } from "../supervisor/ConfirmedRecoveryDialog";
import { NotesView } from "../notes/NotesView";
import { Sidebar } from "../sidebar/Sidebar";
import { spaceNotesFromFailures } from "../sidebar/Spaces";
import { spaceStatus } from "../sidebar/spaceTree";
import { byId } from "./model";
import { TabStrip, type TabStripSpaceContext } from "./TabStrip";
import { CommandOverlay } from "./CommandOverlay";
import { QuickSwitcher } from "./QuickSwitcher";
import { switchRows } from "./quickSwitch";
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
import { QuickAskDialog } from "../quickAsk/QuickAskDialog";
import type { QuickAskState } from "../quickAsk/useQuickAsk";

export type WorkbenchContentProps = WorkbenchProps & WorkbenchState & WorkbenchActions & {
  quickTerminalOpen: boolean;
  commandActions: CommandAction[];
  commandFailureMessage: string;
  commandStatus?: ReactNode;
  ask: QuickAskState;
};

export function WorkbenchContent(props: WorkbenchContentProps) {
  const {
    client, state, sessions, selection, terminalMouseInput, mutations, ctx, onSession, onReconnect, onRefreshSessions, onMutate, onRetryMutation, layoutError, onDismissLayoutError,
    snapshot, shell, popup, popupPending, sidebarWidth, sidebarCollapsed, narrowViewport, drawerOpen, sidebarSession, sidebarCloseRef, spaces, spaceGit, spaceServices, gitBlocked, gitActions, gitPoll, editing, mutationBusy, modalOpen, allTabs,
    closeDrawer, openSessionChooser, setEditing, focusSpace, focusService, openContext, setSetupOpen, focusAgent, updateSidebarWidth, openDrawer, toggleSidebarCollapsed,
    overviewOpen, closeOverview, widgetDots, widgetsPending, showWidgets, tabs, browserOpen, browserReason, libraryOpen, notesOpen, closeNotes, openNotes, focusTab, toggleBrowser, closeLibrary, openLibrary, setCommandsOpen,
    liaison, supervisorNavigationError, librarySpace, captureLibraryInvoker, libraryCommand,
    lifecycleError, setLifecycleError, limits, limitsOpen, setLimitsOpen, commandsOpener, dialog, setDialog, panes, localLeaves, swap, lastTerminalMessage,
    commandsOpen, switcherOpen, setSwitcherOpen, setSwitchTarget, confirmedSpaceId, confirmedPaneId, commandActions, commandStatus, setSessionChooserOpen, libraryAddOpen, setLibraryAddOpen, sessionChooserOpen, setupOpen, setupParent, recoveryOpen, setRecoveryOpen, teardownSpaceId, setTeardownSpaceId,
    prefixActive, armedPrefix, commandNotice, commandFailureMessage, gitNotice, prefixHint,
  } = props;
  const workbenchStyle: CSSProperties & { "--sidebar-width": string } = { "--sidebar-width": `${sidebarWidth}px` };
  const sidebarClass = "sidebar";
  // Selection chrome follows Herdr's acknowledgement: while a focus request is in flight the sidebar keeps the confirmed row and marks the target as pending.
  const pendingSpaceId = state.focusPending?.kind === "space" ? state.focusPending.target_id : null;
  const pendingPaneId = state.focusPending?.kind === "pane" || state.focusPending?.kind === "agent" ? state.focusPending.target_id : null;
  const sidebarSelectedSpaceId = state.focusPending ? snapshot?.focused_space_id ?? null : selection.spaceId;
  const sidebarSelectedPaneId = state.focusPending ? snapshot?.focused_pane_id ?? null : selection.paneId;
  const pendingServiceKey = [...spaceServices.values()].flat().find(service =>
    (service.tab_id !== null && state.focusPending?.kind === "tab" && state.focusPending.target_id === service.tab_id)
    || (state.focusPending?.kind === "pane" && state.focusPending.target_id === service.pane_id))?.key ?? null;
  const activeSpace = byId(spaces, selection.spaceId);
  const activeCheckout = activeSpace ? spaceGit.get(activeSpace.id)?.checkout : undefined;
  const activeSpaceContext: TabStripSpaceContext | null = activeSpace ? {
    label: activeSpace.label,
    branch: activeCheckout ? (activeCheckout.state === "branch" ? activeCheckout.branch : null) : activeSpace.git?.branch ?? null,
    status: spaceStatus(activeSpace.agent_status),
  } : null;
  const agentBlocked = (snapshot?.agents ?? []).some(agent => spaceStatus(agent.status || "unknown").className === "blocked");
  return <div className={`workbench${sidebarCollapsed ? " sidebar-collapsed" : ""}${narrowViewport && drawerOpen ? " drawer-open" : ""}`} style={workbenchStyle}>
    <div className="workbench-underlay" inert={Boolean(popup) || props.quickTerminalOpen}>
    {narrowViewport && drawerOpen ? <button type="button" className="drawer-scrim" aria-label="Close sidebar" onClick={() => closeDrawer()} /> : null}
    <aside id="cockpit-sidebar" className={sidebarClass} aria-label="Spaces and agents" role={narrowViewport && drawerOpen ? "dialog" : undefined} aria-modal={narrowViewport && drawerOpen ? "true" : undefined} aria-hidden={narrowViewport && !drawerOpen ? "true" : undefined} hidden={narrowViewport ? !drawerOpen : sidebarCollapsed}>
      <Sidebar key={`${state.sessionId}:${snapshot?.server_instance}`} session={sidebarSession} sync={state.sync} narrow={narrowViewport} onSession={openSessionChooser} onClose={() => closeDrawer()} closeRef={sidebarCloseRef} hasSession={state.sessionId !== null} hasSnapshot={snapshot !== null}
        spaces={{ spaces, tags: props.spaceTags, tagEditorRequest: props.tagEditorRequest, editorSuspended: modalOpen, services: spaceServices, pendingServiceKey, onSelectService: focusService, gitStatus: spaceGit, gitBlocked, gitEntry: gitActions.forStatus, gitStatusError: gitPoll.error, onRetryGitStatus: gitPoll.refresh, onGitAction: props.runGitAction, onDismissGitProblem: gitActions.dismiss, selectedSpaceId: sidebarSelectedSpaceId, pendingSpaceId, editingId: editing?.kind === "space" ? editing.id : null, busy: mutationBusy, notes: spaceNotesFromFailures(Object.values(mutations.errors)), onEdit: (id) => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "space", id } : null); }, onSelect: focusSpace, onContext: openContext, onSetup: () => setSetupOpen(true), setupEnabled: state.sync === "live" && !modalOpen, mutate: onMutate }}
        agents={{ snapshot, attention: props.attention, live: state.sync === "live", spaces, tabs: allTabs, selectedPaneId: sidebarSelectedPaneId, pendingPaneId, onSelect: row => { if (state.sync === "live") props.attention.activate(row); focusAgent(row.agent); } }} />
    </aside>
    {!narrowViewport && !sidebarCollapsed ? <div className="sidebar-resizer" role="separator" tabIndex={sidebarCollapsed ? -1 : 0} aria-label="Resize sidebar" aria-orientation="vertical" aria-valuemin={SIDEBAR_MIN_WIDTH} aria-valuemax={SIDEBAR_MAX_WIDTH} aria-valuenow={sidebarWidth}
      onKeyDown={(event) => { if (sidebarCollapsed) return; if (event.key === "Home") { event.preventDefault(); updateSidebarWidth(SIDEBAR_DEFAULT_WIDTH); } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") { event.preventDefault(); updateSidebarWidth(sidebarWidth + (event.key === "ArrowLeft" ? -8 : 8)); } }}
      onPointerDown={(event) => { if (sidebarCollapsed || event.button !== 0) return; event.preventDefault(); const start = event.clientX; const width = sidebarWidth; const move = (next: PointerEvent) => updateSidebarWidth(width + next.clientX - start); const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); }; window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop); }} /> : null}
    <main className="main-workarea">
      {!selection.spaceId ? <button type="button" className="drawer-toggle" aria-expanded={drawerOpen} aria-controls="cockpit-sidebar" aria-label="Open sidebar" onClick={narrowViewport ? openDrawer : toggleSidebarCollapsed}><UiIcon name="sidebar" /> <span>Sidebar</span></button> : null}
      {selection.spaceId ? <TabStrip supervisorAction={liaison.actionLabel} supervisorActionDisabled={!liaison.actionEnabled} supervisorActionReason={liaison.actionReason} liaisonTab={liaison.tabId && liaison.root?.attachment?.location?.tab_id === liaison.tabId ? { id: liaison.tabId, launchTag: liaison.root.attachment.location.launch_tag } : null} waitingTabId={liaison.waiting ? liaison.tabId : null} onSupervisor={() => { void liaison.activate(); }} widgetDots={widgetDots} widgetsPending={widgetsPending} onWidgets={showWidgets} sidebarOpen={narrowViewport ? drawerOpen : !sidebarCollapsed} spaceContext={activeSpaceContext} agentBlocked={agentBlocked} onToggleSidebar={narrowViewport ? (drawerOpen ? () => closeDrawer() : openDrawer) : toggleSidebarCollapsed} tabs={tabs} selectedTabId={selection.tabId} editingId={editing?.kind === "tab" ? editing.id : null} busy={mutationBusy} browserOpen={browserOpen} browserDisabledReason={browserOpen ? null : browserReason} libraryOpen={libraryOpen} notesOpen={notesOpen} onNotesToggle={notesOpen ? closeNotes : openNotes} onEdit={id => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "tab", id } : null); }} onSelect={focusTab} onContext={openContext} onCreate={() => { if (selection.spaceId) onMutate("tab:new", { type: "tab_create", space_id: selection.spaceId, label: null }, true); }} onBrowserToggle={toggleBrowser} onLibraryToggle={() => { if (libraryOpen) closeLibrary(); else openLibrary(); }} onCommands={() => setCommandsOpen(true)} onQuickSwitch={() => setSwitcherOpen(true)} mutate={onMutate} /> : <div className="tab-toolbar"><div className="tab-strip-actions"><button type="button" className="tab-icon-button" aria-label="Commands" title={withShortcut("Commands", "help")} onClick={() => setCommandsOpen(true)}><UiIcon name="command" /></button></div></div>}
      <div className="workarea-content">
        {overviewOpen ? <SupervisorView key={`${liaison.snapshot?.state_store_id}:${liaison.root?.run_id}`} tasks={liaison.tasks} runs={liaison.snapshot?.runs.filter(run => run.root_id === liaison.root?.run_id) ?? []} snapshot={liaison.snapshot} rootId={liaison.root?.run_id ?? null} fresh={liaison.fresh} waiting={liaison.waiting} status={liaison.status} onClose={() => { closeOverview(); if (selection.paneId !== liaison.paneId && liaison.startAvailable) void liaison.start(); }} /> : null}
        {notesOpen ? <NotesView key={`${state.sessionId}:${selection.spaceId}`} client={client} space={librarySpace} onClose={closeNotes} /> : libraryOpen ? <LibraryView client={client} onClose={closeLibrary} onCaptureInvoker={captureLibraryInvoker} command={libraryCommand} space={librarySpace} /> : <WorkbenchCanvas {...props} />}
        <span className="sr-only" role="status" aria-live="polite">{liaison.waiting ? "Supervisor is waiting for you." : ""}</span>
      </div>
      <div className="workarea-statusbar">
      <BrowserCleanupNotices ctx={ctx} activeTabId={selection.tabId} fallback={<ErrorSlot placement="pane" message={[lifecycleError, layoutError, liaison.error, supervisorNavigationError].filter(Boolean).join(" · ")} actions={lifecycleError || layoutError ? <>
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
    {props.askOpen ? <QuickAskDialog state={props.ask} onClose={() => props.setAskOpen(false)} /> : null}
    {liaison.recovery ? <ConfirmedRecoveryDialog recovery={liaison.recovery} /> : null}
    {switcherOpen ? <QuickSwitcher rows={switchRows({ spaces, agents: snapshot?.agents ?? [], tabs: allTabs, currentSpaceId: confirmedSpaceId, currentPaneId: confirmedPaneId, live: state.sync === "live", busy: mutationBusy })}
      hasSession={state.sessionId !== null} loading={snapshot === null} live={state.sync === "live"}
      onChoose={target => { setSwitchTarget(target); setSwitcherOpen(false); }} onDismiss={() => setSwitcherOpen(false)} /> : null}
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
