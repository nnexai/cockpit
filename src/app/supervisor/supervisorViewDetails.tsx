import { UiIcon } from "../UiIcon";
import { PanelSplitter } from "./PanelSplitter";
import { AttentionQueue } from "./SupervisorAttention";
import { SupervisorActions } from "./SupervisorActions";
import { SupervisorActivity, SupervisorDiagnostics, activityRows } from "./SupervisorActivity";
import { TIER_LABEL } from "./attention";
import { stepDraft } from "./useSupervisorDrafts";
import type { SupervisorViewRenderContext } from "./supervisorViewTypes";

export function SupervisorViewDetails({ view }: { view: SupervisorViewRenderContext }) {
  const { snapshot, active, panelKind, bounds, placement, scope, changed, rows, expandedId, focusTier, setFocusTier, closeAttention, navigateRelation, tasks, closeDetail, closePanel, detailTask, detailSubagent, detailRun, detailSection, setDetailSection, busy, live, mutateResult, navigate, edit, setDialog, stateSentence, tier, owned, path, resumeSourceDraft, editPrerequisites, createFollowUp, detailTaskScope, taskWriteUnconfirmed, stepReadOnlyReason, submitStep, readSaved, resolveUnknown, mode, switchView, rootRuns, select, connected, setFocusNotice } = view;
    return snapshot && active && panelKind ? <>{bounds ? <PanelSplitter label="Resize details" controls="supervisor-detail" orientation={bounds.orientation} value={bounds.value} min={bounds.min} max={bounds.max} grow={-1} onChange={value => { if (placement === "sheet") scope.view.sheetHeight = value; else scope.view.detailWidth = value; changed(); }} onReset={() => { if (placement === "sheet") scope.view.sheetHeight = null; else scope.view.detailWidth = null; changed(); }} className={placement === "sheet" ? "supervisor-sheet-splitter" : undefined} /> : null}<aside className={`supervisor-detail-panel${placement === "sheet" ? " supervisor-sheet" : ""}`} id="supervisor-detail" aria-label={panelKind === "details" ? "Selected details" : panelKind === "attention" ? "Attention" : panelKind === "diagnostics" ? "Diagnostics" : "Activity"}>
      {panelKind === "attention" ? <AttentionQueue
        rows={rows} expandedId={expandedId}
        onExpand={id => { scope.view.queueOpenRow = id ?? "collapsed"; changed(); }}
        capPx={null} variant="overlay" focusTier={focusTier}
        onFocusedTier={() => setFocusTier(null)} onClose={closeAttention}
      /> : <>
        <header className="supervisor-detail-header">
          {panelKind === "details" && scope.view.detailTrail.length ? <button type="button" data-relation-back onClick={() => navigateRelation(scope.view.detailTrail.at(-1)!, true)}>‹ {tasks.find(task => task.task.task_id === scope.view.detailTrail.at(-1))?.task.title ?? "Back"}</button> : null}
          <button type="button"
            aria-label={panelKind === "details" ? "Close details" : `Close ${panelKind === "activity" ? "activity" : "diagnostics"} panel`}
            onClick={panelKind === "details" ? closeDetail : closePanel}><UiIcon name="back" /></button>
          <strong>{panelKind === "details" ? detailTask?.task.title ?? detailSubagent?.label ?? detailRun?.label ?? "Task unavailable · drafts kept" : panelKind === "activity" ? "Activity" : "Diagnostics"}</strong>
        </header>
        {panelKind === "details" ? <>
          <nav className="supervisor-segments" aria-label="Detail sections">
            {(["overview", "activity", "actions"] as const).map(section =>
              <button type="button" key={section} aria-pressed={detailSection === section}
                onClick={() => setDetailSection(section)}>{section[0].toUpperCase() + section.slice(1)}</button>)}
          </nav>
          <div className="supervisor-detail-scroll">
            <SupervisorActions
              section={detailSection} snapshot={snapshot} run={detailRun} task={detailTask}
              subagent={detailSubagent} scope={scope} changed={changed} busy={busy} live={!!live}
              mutateResult={mutateResult} onTerminal={run => void navigate(run)} onEditTask={edit}
              onCloseTracking={run => setDialog({ mode: "close", run })}
              onCancelSubagent={(run, subagent) => setDialog({ mode: "subagent_cancel", run, subagent })}
              stateBlock={{ sentence: stateSentence, tierLabel: tier ? TIER_LABEL[tier] : null, waitingSince: owned?.since ?? null }}
              path={path}
              onResumeSourceDraft={resumeSourceDraft}
              onNavigateTask={navigateRelation} onEditPrerequisites={editPrerequisites} onCreateFollowUp={createFollowUp}
              taskWriteUnconfirmed={!!detailTaskScope && taskWriteUnconfirmed(detailTaskScope)}
              steps={detailTaskScope ? { scope: detailTaskScope, task: detailTask?.task ?? null, draft: stepDraft(scope, detailTaskScope.taskId), writable: !stepReadOnlyReason, readOnlyReason: stepReadOnlyReason, busy, taskWriteUnconfirmed: taskWriteUnconfirmed(detailTaskScope), onDraftChanged: changed, submit: submitStep, readSaved, resolveUnknown, onCloseDetails: closeDetail } : undefined}
              crossView={null}
              crossViews={(["tasks", "graph", "dependencies"] as const).filter(next => next !== mode).map(next => ({ label: next === "tasks" ? "Show in Tasks" : next === "graph" ? "Show in Graph" : "Show in Dependencies", onActivate: () => { if (next === "tasks" && detailTask?.task.checked) scope.disclosures.completed = true; switchView(next, true); } }))}
              acceptanceConflict={snapshot.intents.some(intent => intent.state === "conflict" &&
                (intent.task_id === detailTask?.task.task_id || intent.run_id === detailRun?.run_id))}
            />
          </div>
        </> : <div className="supervisor-detail-scroll">
          {panelKind === "activity" ? <SupervisorActivity rows={activityRows(snapshot, rootRuns, tasks)}
            onLink={link => {
              if (link.kind === "task") select({ task: link.taskId }, null, false, true);
              else select({ run: link.runId, subagent: null }, null, false, true);
            }} /> : <SupervisorDiagnostics snapshot={snapshot} rootRuns={rootRuns} busy={busy} connected={connected}
            onIdentify={() => {
              if (snapshot.board) void mutateResult({ action: "tasks_assign_ids", root_id: snapshot.board.root_id, expected_doc_revision: snapshot.board.doc_revision });
            }}
            onCopyPath={() => setFocusNotice("Canonical task path copied")} />}
        </div>}
      </>}
    </aside></> : null;
}
