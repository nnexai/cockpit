import type { CSSProperties } from "react";
import { UiIcon } from "../UiIcon";
import { ClosedTracking } from "./SupervisorActivity";
import { SupervisorSummary, AttentionQueue } from "./SupervisorAttention";
import { SupervisorDialogs, TaskSourceDialog } from "./SupervisorDialogs";
import { rootAttentionSummary } from "./attention";
import { queueCap } from "./useSupervisorLayout";
import { SupervisorViewWorkarea } from "./supervisorViewWorkarea";
import { SupervisorViewDetails } from "./supervisorViewDetails";
import type { SupervisorViewRenderContext } from "./supervisorViewTypes";

export function SupervisorViewSurface({ view }: { view: SupervisorViewRenderContext }) {
  const { rootRef, active, mode, layout, placement, focusedTask, focusedGraph, focusWithinDetail, questionHadFocus, dialog, escapeLayer, openRoots, root, busy, startPending, saveOffsets, setRootId, setTerminalError, setAttentionOpen, snapshot, scope, openPanel, closedRoots, changed, rootState, destination, start, startDraft, setDialog, onClose, archiveCounts, workareaRef, bounds, bottomInset, error, refresh, rootReceiptLabel, live, observedCount, banner, attention, counterInvoker, setFocusTier, focusTier, navigate, rows, expandedId, inlineQueueCap, model, focusNotice, taskWriteUnconfirmed, submitTask, readSaved, resolveSourceUnknown, createdSelection, setDetailSection, setFocusNotice, returnFocusIntent, runtimeLive, session, connected, mutateResult, started, setStartUnknown, check, startUnknown } = view;
  const startDisabled = busy || startPending || !snapshot || !connected || startUnknown || rootState?.kind === "starting";
  return <section ref={rootRef} className="supervisor-view" hidden={!active} aria-label="Supervisor"
    data-view={mode} data-narrow={layout.narrow} data-short={layout.short} data-compact={layout.compact}
    data-panel={placement ?? "none"} data-queue={layout.queueMode}
    onFocusCapture={event => {
      const target = event.target as HTMLElement;
      if (target.closest("li.supervisor-task")) focusedTask.current = target.closest("li.supervisor-task")?.querySelector<HTMLElement>("[data-row-id]")?.dataset.rowId ?? null;
      focusedGraph.current = target.closest<HTMLElement>(".supervisor-graph-node")?.dataset.rowId ?? null;
      focusWithinDetail.current = !!target.closest(".supervisor-detail-panel");
      questionHadFocus.current = !!target.closest('[aria-label="Needs you"]');
    }}
    onKeyDown={event => {
      if (event.key !== "Escape" || event.nativeEvent.isComposing || event.defaultPrevented || dialog ||
        (event.target as HTMLElement).closest("input,textarea,select,[contenteditable=true]")) return;
      event.preventDefault(); event.stopPropagation(); escapeLayer(event.target as HTMLElement);
    }}>
    <header className="supervisor-header"><span className="supervisor-brand"><UiIcon name="branch" /><strong>Supervisor</strong></span>{openRoots.length > 1 ? <label className="supervisor-agent-label"><select aria-label="Agent" value={root?.stage !== "closed" ? root?.run_id ?? "" : ""} disabled={busy || startPending || !!dialog} onChange={event => { saveOffsets(); setRootId(event.target.value); setTerminalError(null); setAttentionOpen(false); }}><option value="" disabled>Choose an agent</option>{openRoots.map(summary => { const counts = rootAttentionSummary(snapshot!, summary.root_id); return <option key={summary.root_id} value={summary.root_id}>{summary.label} · {counts.decide} need you{counts.recover ? " · ⚠ recover" : ""}</option>; })}</select></label> : null}<nav className="supervisor-header-panels" aria-label="Supervisor panels"><button type="button" aria-label="Activity" title="Activity" aria-pressed={scope.disclosures.history} onClick={event => openPanel("history", event.currentTarget)}><UiIcon name="comment" /></button><button type="button" aria-label="Diagnostics" title="Diagnostics" aria-pressed={scope.disclosures.diagnostics} onClick={event => openPanel("diagnostics", event.currentTarget)}><UiIcon name="info" /></button>{closedRoots.length ? <button type="button" aria-label={`Closed tracking · ${closedRoots.length}`} title={`Closed tracking · ${closedRoots.length}`} aria-expanded={scope.disclosures.archive} onClick={() => { scope.disclosures.archive = !scope.disclosures.archive; changed(); }}><UiIcon name="folder" /></button> : null}</nav><button type="button" className={root?.stage !== "closed" && rootState?.verified ? "supervisor-start-secondary" : "supervisor-primary"} title={destination ? `New tab in ${destination.label}` : "Choose an available location when starting."} data-start-agent disabled={startDisabled} onClick={() => void start()}><UiIcon name="plus" />{startPending ? "Starting…" : "Start agent"}</button><button type="button" className="supervisor-start-options" aria-label="Start options…" title="Start options…" disabled={startDisabled} onClick={() => { if (!startDraft.current.spaceId && destination) startDraft.current.spaceId = destination.id; setDialog({ mode: "start" }); }}><UiIcon name="more" /><span>Start options…</span></button><button type="button" aria-label="Hide Supervisor" title="Hide Supervisor" onClick={onClose}><UiIcon name="close" /></button></header>
    {scope.disclosures.archive && snapshot ? <ClosedTracking closedRoots={closedRoots} runs={snapshot.runs} loadedRootId={snapshot.board?.root_id ?? null} taskCount={snapshot.board?.tasks.length ?? 0} taskCounts={archiveCounts} busy={busy} dialogOpen={!!dialog} onView={id => { saveOffsets(); setRootId(id); scope.disclosures.archive = false; changed(); }} returnTo={root?.stage === "closed" && openRoots.length ? { label: openRoots[0].label, onActivate: () => setRootId(openRoots[0].root_id) } : null} /> : null}
    <div ref={workareaRef} className="supervisor-workarea" style={{ "--detail-size": `${bounds?.value ?? 340}px`, "--sheet-size": `${bottomInset}px` } as CSSProperties}><div className="supervisor-content">
      {!snapshot ? <div className="supervisor-empty"><UiIcon name="branch" /><h2>{error ? "Could not load Supervisor" : "Loading Supervisor…"}</h2>{error ? <><p>Your terminals are unchanged. The Supervisor connection could not be established.</p><button type="button" onClick={refresh}>Retry load</button></> : <p role="status">Reading saved tasks and fresh agent observations.</p>}</div> : <>
        <SupervisorSummary
          rootLabel={root?.label ?? "No supervisor selected"} stateLabel={rootReceiptLabel ?? rootState?.label ?? "Start an agent"}
          stateGlyph={rootState?.blocked ? "blocked" : rootState?.verified ? "live" : "unknown"}
          observedLine={live ? `${observedCount} agents observed · ${snapshot.runtime.status === "fresh" ? new Date(snapshot.runtime.observed_at).toLocaleTimeString() : ""}` : "Agents · unobserved"}
          banner={banner} counts={attention?.counts ?? { decide: 0, recover: 0, notice: 0 }} queueMode={layout.queueMode}
          onCounter={(tier, invoker) => {
            counterInvoker.current = invoker;
            if (layout.queueMode === "overlay") { setFocusTier(null); setAttentionOpen(true); }
            else setFocusTier(tier);
          }}
          shortcut={root && rootState?.terminal ? { key: "terminal", label: "Open terminal", disabled: busy || !live, onActivate: () => void navigate(root) } : null}
        />
        {rows.length > 0 && layout.queueMode === "inline" ? <AttentionQueue rows={rows} expandedId={expandedId} onExpand={id => { scope.view.queueOpenRow = id ?? "collapsed"; changed(); }} capPx={inlineQueueCap ?? (queueCap(layout, mode) || null)} variant="inline" focusTier={focusTier} onFocusedTier={() => setFocusTier(null)} /> : null}
        {root && model ? <SupervisorViewWorkarea view={view} /> : <div className="supervisor-empty"><UiIcon name="branch" /><h2>Start an agent to manage your tasks.</h2></div>}
      </>}
    </div>
    <SupervisorViewDetails view={view} /></div>
    <div className="supervisor-focus-notice" role="status">{focusNotice}</div>
    {dialog && snapshot && active ? dialog.mode === "edit" || dialog.mode === "relations" || dialog.mode === "follow_up" ? <TaskSourceDialog key={`${dialog.mode}:${dialog.scope.rootId}:${dialog.scope.taskId}`} dialog={dialog} snapshot={snapshot} changed={changed} busy={busy} available={!!live} writeUnconfirmed={taskWriteUnconfirmed(dialog.draft.submitted?.scope ?? dialog.scope)} submitTask={submitTask} readSaved={readSaved} resolveUnknown={resolveSourceUnknown} onSaved={(task, created, reconciled) => { if (dialog.mode === "edit") scope.edits.delete(dialog.scope.taskId); else if (dialog.mode === "relations") scope.relations.delete(dialog.scope.taskId); else scope.followUps.delete(dialog.scope.taskId); if (created) { createdSelection.current = { sessionId: dialog.scope.sessionId, rootId: dialog.scope.rootId, taskId: task.task_id, focus: document.activeElement instanceof HTMLElement && !!document.activeElement.closest('[role="dialog"]') }; scope.selectedTask = task.task_id; scope.selectedRun = null; scope.selectedSubagent = null; scope.view.detailTrail = []; setDetailSection("overview"); setFocusNotice(`${reconciled ? "Showing saved task" : "Follow-up created"} · ${task.title}`); } changed(); }} onDiscard={() => { if (dialog.mode === "edit") scope.edits.delete(dialog.scope.taskId); else if (dialog.mode === "relations") scope.relations.delete(dialog.scope.taskId); else scope.followUps.delete(dialog.scope.taskId); changed(); }} onClose={() => setDialog(null)} onReturnFocus={invoker => { returnFocusIntent.current = { invoker }; }} /> : <SupervisorDialogs dialog={dialog} snapshot={snapshot} spaces={runtimeLive ? session?.spaces ?? [] : []} startDraft={startDraft.current} changed={changed} busy={busy} available={connected && (dialog.mode === "close" || runtimeLive && snapshot.runtime.status === "fresh")} mutateResult={mutateResult} onStarted={started} onStartUnconfirmed={() => setStartUnknown(true)} onCheck={check} onClose={() => setDialog(null)} onReturnFocus={invoker => { returnFocusIntent.current = { invoker }; }} /> : null}
  </section>;
}
