import type { AttentionKind } from "../../protocol/generated/v1";
import { agentState, questionForRun, questionLabel, recoveryActions, taskStatus, TextAction, type PathRowView } from "./SupervisorActions";
import { messageDraft } from "./useSupervisorDrafts";
import { deriveAttention, TIER_ORDER, type LocalCondition } from "./attention";
import type { QueueAction, QueueRowView } from "./SupervisorAttention";
import type { StripChip } from "./SupervisorTasks";
import { nodeFacts, nodeSelection, pathNodes } from "./topology";
import { retirementView } from "./retirementView";
import { dependentCount } from "./dependencies";
import type { SupervisorViewInputs, SupervisorViewModel, SupervisorViewContext, SupervisorViewAttention, SupervisorViewQueue } from "./supervisorViewTypes";

const ATTENTION_PROBLEM: Record<AttentionKind, string> = {
  needs_input: "Needs your answer",
  runtime_blocked: "Blocked without a question",
  dispatch_unknown: "Dispatch unconfirmed",
  exited_without_report: "Exited without a Result",
  idle_without_report: "Idle without a Result",
  brief_unread: "Brief not read yet",
  plan_changed: "Plan changed",
  intent_conflict: "Task changed during acceptance",
  awaits_prepare: "Waiting for supervisor",
  awaits_execute: "Waiting for supervisor",
  to_accept: "Supervisor is reviewing",
  retirement_unconfirmed: "Worker retirement unconfirmed",
};

export function useSupervisorViewAttention(context: SupervisorViewInputs & SupervisorViewModel): SupervisorViewAttention {
  const { snapshot, root, startUnknown, terminalError, navigationError, error, connected, notice, orphanedWorkers, rootRuns, runtimeLive, tasks, rootState, scope } = context;
  const assignmentIntents = snapshot?.assignment_intents.filter(intent => intent.root_id === root?.run_id) ?? [];
  const local: LocalCondition[] = [];
  if (startUnknown) local.push({ kind: "start_unknown" });
  if (terminalError) local.push({ kind: "terminal_error", message: terminalError });
  if (navigationError) local.push({ kind: "navigation_error" });
  if (error && connected) local.push({ kind: "change_unconfirmed", message: error });
  if (notice) local.push({ kind: "notice", message: notice });
  for (const run of orphanedWorkers) local.push({ kind: "orphaned_worker", runId: run.run_id });
  if (snapshot) for (const run of rootRuns) if (run.stage !== "closed" && !["proposed", "awaiting_prepare"].includes(run.stage) && ["failure", "missing", "unknown"].includes(agentState(snapshot, run, connected, runtimeLive).kind)) local.push({ kind: "agent_status", runId: run.run_id, taskId: run.task_id });
  if (root?.last_report?.outcome === "failed") local.push({ kind: "root_failed_report", runId: root.run_id });
  for (const intent of assignmentIntents) local.push({ kind: "assignment", taskId: intent.task_id, state: intent.state });
  if (root && snapshot?.board?.unidentified_items) local.push({ kind: "unidentified_items", count: snapshot.board.unidentified_items });
  for (const task of tasks) {
    if (task.dependencies.state === "invalid" || task.task.relations_diagnostic) local.push({ kind: "relation_diagnostic", taskId: task.task.task_id, cause: task.dependencies.problems.map(problem => problem.message).join(" · ") || task.task.relations_diagnostic || "Unreadable prerequisites." });
    const currentWorker = snapshot?.runs.find(run => run.run_id === task.current_run_id);
    if (currentWorker && currentWorker.stage !== "closed" && !task.task.checked && (currentWorker.grants.some(grant => grant.scope === "execute") || currentWorker.stage === "working" || currentWorker.stage === "reported") && ["blocked", "invalid"].includes(task.dependencies.state)) local.push({ kind: "dependency_regression", taskId: task.task.task_id });
  }
  const attention = snapshot ? deriveAttention({ snapshot, rootId: root?.run_id ?? null, local }) : null;
  const decide = attention?.items.some(item => item.tier === "decide" && item.runId === root?.run_id) ?? false;
  const rootQuestion = root && snapshot ? questionForRun(snapshot, root) : null;
  const rootReceiptLabel = root && rootQuestion && rootQuestion.receipt.status !== "unresolved"
    && rootState?.kind === "ready" && !rootState.blocked && attention?.tierForRun(root.run_id) !== "recover"
    ? questionLabel(rootQuestion) : null;
  const question = decide && rootQuestion?.receipt.status === "unresolved"
    && root?.last_report?.kind === "needs_input" && root.last_report.message_id === rootQuestion.question_message_id ? root.last_report : null;
  const answer = question && root ? messageDraft(scope, `answer:${root.run_id}:${question.message_id}`) : null;
  return { attention, decide, rootReceiptLabel, question, answer };
}

export function supervisorViewQueueRows(context: SupervisorViewContext): SupervisorViewQueue {
  const { attention, snapshot, tasks, question, answer, root, changed, busy, live, rootState, mutateResult, connected, refresh, setRootId, setStartUnknown, setNotice, runtimeLive, editPrerequisites, switchView, showItem, attentionOpen, setAttentionOpen, select, startPending, pendingRestart, navigate, check, setDialog, restart, scope, mode, rootRuns, model, selectedNode, selectedTask, detailRun, detailSubagent, detailTask } = context;
  const rows: QueueRowView[] = attention?.items.map(item => {
    const run = snapshot!.runs.find(run => run.run_id === item.runId);
    const retirement = run ? retirementView(run) : null;
    const retirementUnconfirmed = item.sources.some(source => source.origin === "core" && source.kind === "retirement_unconfirmed");
    const task = tasks.find(task => task.task.task_id === item.taskId || task.current_run_id === item.runId || retirement && task.task.task_id === run?.task_id);
    const actions: QueueAction[] = []; const bodies = [];
    let title = task?.task.title ?? run?.label ?? "Supervisor";
    for (const source of item.sources) {
      if (source.origin === "core") {
        const problem = source.kind === "retirement_unconfirmed" ? retirement?.label ?? ATTENTION_PROBLEM[source.kind] : ATTENTION_PROBLEM[source.kind];
        if (source === item.sources[0]) title = `${problem} · ${title}`;
        if (source.kind === "needs_input" && question && answer && root) bodies.push(<section key="question" className="supervisor-needs-you" aria-label="Needs you"><p id={`supervisor-question-${question.message_id}`} className="supervisor-exact-text">{question.summary}</p><TextAction label="Answer" submitLabel="Send answer" describedBy={`supervisor-question-${question.message_id}`} draft={answer} changed={changed} busy={busy || !live || !rootState?.verified} submit={(text, message_id) => mutateResult({ action: "message_send", message_id, to_run_id: root.run_id, kind: "answer", text, in_reply_to: question.message_id })} success="Answer sent. Waiting for the agent." /></section>);
        if (source.kind === "retirement_unconfirmed" && retirement) bodies.push(<p key="retirement" className="supervisor-exact-text">{retirement.detail}</p>);
        if (source.kind === "intent_conflict") for (const intent of snapshot!.intents.filter(intent => intent.root_id === root?.run_id && intent.state === "conflict" && (intent.run_id === item.runId || intent.task_id === item.taskId))) {
          actions.push({ key: `apply:${intent.intent_id}`, label: "Apply acceptance to current task", primary: true, disabled: busy || !live, onActivate: () => void mutateResult({ action: "intent_resolve", intent_id: intent.intent_id, apply: true }) }, { key: `keep:${intent.intent_id}`, label: "Keep current task unchanged", disabled: busy || !connected, onActivate: () => void mutateResult({ action: "intent_resolve", intent_id: intent.intent_id, apply: false }) });
          bodies.push(<p key={intent.intent_id}>Review the current task before applying acceptance. Keeping it leaves Markdown unchanged.</p>);
        }
      } else {
        const condition = source.condition;
        if (condition.kind === "assignment" && root) {
          const canonical = tasks.find(task => task.task.task_id === condition.taskId);
          title = `${condition.state === "conflict" ? "Task changed elsewhere · Not assigned" : "Task assignment pending"} · ${canonical?.task.title ?? "Task"}`;
          if (canonical) bodies.push(<details key="canonical"><summary>Current task</summary><p className="supervisor-exact-text">{canonical.task.body}</p></details>);
          if (condition.state === "conflict") actions.push({ key: "assign", label: "Assign current task", primary: true, disabled: busy || !live || !canonical || !!canonical.task.diagnostic || root.stage === "closed", onActivate: () => void mutateResult({ action: "task_assignment_resolve", root_id: root.run_id, task_id: condition.taskId, expected_task_revision: canonical?.task.task_revision ?? null, assign: true }) }, { key: "unassign", label: "Keep unassigned", disabled: busy || !connected, onActivate: () => void mutateResult({ action: "task_assignment_resolve", root_id: root.run_id, task_id: condition.taskId, expected_task_revision: null, assign: false }) });
          else actions.push({ key: "assignment-check", label: "Check assignment status", primary: true, disabled: busy, onActivate: refresh });
        } else if (condition.kind === "orphaned_worker" && run) {
          title = `Worker agent needs control · ${run.label}`; bodies.push(<p key="orphan">Closing this supervisor did not stop its workers. Tasks and history are kept.</p>);
          actions.push({ key: "saved-context", label: "View saved task context", onActivate: () => setRootId(run.root_id), disabled: busy });
        } else if (condition.kind === "start_unknown") {
          title = "Previous start unconfirmed"; bodies.push(<p key="start">Review the tracked agents before another start; a terminal may already have opened.</p>);
          actions.push({ key: "start-check", label: "Check status", primary: true, onActivate: refresh, disabled: busy }, { key: "start-reviewed", label: "I have reviewed the previous start", disabled: busy, onActivate: () => { setStartUnknown(false); setNotice("Previous start reviewed. Any existing terminal and tracking remain unchanged."); } });
        } else if (condition.kind === "terminal_error" || condition.kind === "change_unconfirmed" || condition.kind === "navigation_error") {
          title = condition.kind === "navigation_error" ? "Could not open terminal. Its current focus or location was not confirmed." : condition.message;
          actions.push({ key: "check", label: "Check status", primary: true, onActivate: refresh, disabled: busy });
        } else if (condition.kind === "notice") { title = condition.message; actions.push({ key: "dismiss", label: "Dismiss notice", onActivate: () => setNotice(null) }); }
        else if (condition.kind === "root_failed_report" && run) { title = `Reported failure · ${run.label}`; bodies.push(<p key="failure" className="supervisor-exact-text">{run.last_report?.summary}</p>); }
        else if (condition.kind === "unidentified_items" && root && snapshot?.board) { title = `${condition.count} task-file items need identity markers`; actions.push({ key: "identify", label: "Identify task-file items", primary: true, disabled: busy || !connected, onActivate: () => void mutateResult({ action: "tasks_assign_ids", root_id: root.run_id, expected_doc_revision: snapshot.board!.doc_revision }) }); }
        else if (condition.kind === "agent_status" && run && snapshot) title = `${agentState(snapshot, run, connected, runtimeLive).label} · ${run.label}`;
        else if (condition.kind === "relation_diagnostic" && task) {
          title = `Prerequisites need fixing · ${task.task.title}`; bodies.push(<p key="relations">{condition.cause}</p>);
          actions.push({ key: "fix-prerequisites", label: "Fix prerequisites…", primary: true, disabled: busy || !live || root?.stage === "closed", onActivate: () => editPrerequisites(task) });
        } else if (condition.kind === "dependency_regression" && task) {
          title = `Prerequisites changed during work · ${task.task.title}`;
          bodies.push(<p key="regression">The current agent was not automatically stopped. New guarded work, execution and acceptance wait for the supervisor to resolve prerequisites.</p>);
          actions.push({ key: "dependencies", label: "Show dependencies", onActivate: invoker => { switchView("dependencies"); showItem(task.task.task_id, task.current_run_id, invoker); } });
        }
      }
    }
    if (retirementUnconfirmed && retirement && run && task && task.current_run_id !== run.run_id) actions.push({
      key: "saved-retirement", label: "View saved worker details",
      onActivate: invoker => { if (attentionOpen) setAttentionOpen(false); select({ run: run.run_id, subagent: null }, invoker, false, false); },
    });
    if (item.tier === "recover" && run && snapshot && !retirementUnconfirmed) {
      const state = agentState(snapshot, run, connected, runtimeLive);
      const recovery = recoveryActions(run, state, { busy: busy || startPending || pendingRestart === run.run_id, connected, runtimeLive: runtimeLive && snapshot.runtime.status === "fresh" });
      for (const action of recovery) actions.push({
        key: action.kind, label: action.label, primary: action.primary || action.kind === "terminal" && !recovery.some(item => item.primary), disabled: action.disabled, ariaDisabled: action.ariaDisabled,
        reason: action.reason, consequence: action.consequence,
        onActivate: () => {
          if (action.disabled) return;
          if (action.kind === "terminal") void navigate(run);
          else if (action.kind === "check") check(run);
          else if (action.kind === "close") setDialog({ mode: "close", run });
          else void restart(run);
        },
      });
      bodies.push(<p key="recovery-detail" className="supervisor-muted">{state.detail}</p>);
    }
    if (task && dependentCount(task, tasks)) bodies.push(<p key="holds">{dependentCount(task, tasks)} tasks are waiting on this</p>);
    return {
      id: item.id, tier: item.tier, title, since: item.since,
      body: bodies.length ? <>{bodies}</> : undefined, actions,
      showIn: retirementUnconfirmed ? task ? {
        key: "show", label: "Show completed task",
        onActivate: invoker => { scope.disclosures.completed = true; if (mode !== "tasks") switchView("tasks"); showItem(task.task.task_id, item.runId, invoker); },
      } : run ? {
        key: "show", label: "View saved worker details",
        onActivate: invoker => { if (attentionOpen) setAttentionOpen(false); select({ run: run.run_id, subagent: null }, invoker, false, false); },
      } : null : task || run && rootRuns.includes(run) || root ? {
        key: "show", label: mode === "graph" ? "Show in Graph" : mode === "dependencies" ? "Show in Dependencies" : "Show in Tasks",
        onActivate: invoker => showItem(task?.task.task_id ?? item.taskId, item.runId ?? root?.run_id ?? null, invoker),
      } : null,
    };
  }) ?? [];
  const expandedId = scope.view.queueOpenRow && rows.some(row => row.id === scope.view.queueOpenRow) ? scope.view.queueOpenRow : scope.view.queueOpenRow === null ? rows.find(row => row.tier === "decide")?.id ?? null : null;
  const path: PathRowView[] = model && selectedNode ? pathNodes(model, selectedNode, true).map((node, depth) => {
    const facts = nodeFacts(node, { model, snapshot: snapshot!, live: !!live, connected, runtimeLive });
    return { key: node.id, role: facts.role, label: facts.title, facts: [facts.status, facts.provenance, facts.relation].filter((fact): fact is string => !!fact), current: node.id === selectedNode, depth, subagent: node.kind === "subagent", onActivate: () => select(nodeSelection(node), null, false, true) };
  }) : [];
  const tier = selectedTask ? attention?.tierForTask(selectedTask) : detailRun ? attention?.tierForRun(detailRun.run_id) : null;
  const owned = detailRun ? attention?.ownedForRun(detailRun.run_id) : null;
  const detailState = detailRun && snapshot ? agentState(snapshot, detailRun, connected, runtimeLive) : null;
  const detailQuestion = detailRun && !detailSubagent && snapshot ? questionForRun(snapshot, detailRun) : null;
  const stateSentence = detailQuestion
    ? detailState && (detailState.kind !== "ready" || detailState.blocked || tier === "recover") ? detailState.label : questionLabel(detailQuestion)
    : owned ? owned.kind === "to_accept" ? "Supervisor is reviewing this result" : "Waiting for supervisor"
    : detailState?.label ?? (detailTask ? taskStatus(detailTask, undefined, snapshot!) : "No current worker");
  const chips: StripChip[] = model && snapshot ? model.nodes.filter(node => node.run && !node.subagent && node.run.stage !== "closed").map(node => { const facts = nodeFacts(node, { model, snapshot, live: !!live, connected, runtimeLive }); return { runId: node.run!.run_id, label: facts.title, glyph: facts.glyph === "document" ? "unknown" : facts.glyph, status: facts.status, tier: attention?.tierForRun(node.run!.run_id) ?? null, selected: scope.selectedRun === node.run!.run_id }; }).sort((a, b) => (a.tier ? TIER_ORDER[a.tier] : 3) - (b.tier ? TIER_ORDER[b.tier] : 3)) : [];

  return { rows, expandedId, path, tier, owned, stateSentence, chips };
}
