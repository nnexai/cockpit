import { UiIcon } from "../UiIcon";
import { SupervisorGraph } from "./SupervisorGraph";
import { SupervisorDependencies } from "./SupervisorDependencies";
import { AgentsStrip, TaskCard } from "./SupervisorTasks";
import { taskLanes, taskNeighbor } from "./boardNavigation";
import { nodeSelection, type TopologyNode } from "./topology";
import { taskStatus } from "./SupervisorActions";
import { readOffset, revealNearest } from "./reveal";
import type { TaskLane, TaskView } from "../../protocol/generated/v1";
import type { SupervisorViewRenderContext } from "./supervisorViewTypes";

export function SupervisorViewWorkarea({ view }: { view: SupervisorViewRenderContext }) {
  const { tasks, scope, layout, changed, observe, attention, tabIndexFor, dimFor, hoverRun, selectedAgent, rootSpace, sharedSpace, focusedTask, select, rowElement, setHoverRun, setHoverSpace, laneRefs, mode, switchView, observations, rootRuns, connected, runtimeLive, selectedNode, detailRun, revealIntent, setDetailSection, bottomInset, graphRef, escapeLayer, observedCount, chips, rootRef, listRef, listProps, navOptions, busy, live, rootState, navigate } = view;
  const snapshot = view.snapshot!, root = view.root!, model = view.model!;
  const renderTask = (task: TaskView) => {
    const worker = snapshot!.runs.find(run => run.run_id === task.current_run_id), observed = observe(worker?.run_id);
    const tier = attention?.tierForTask(task) ?? null;
    return <TaskCard key={task.task.task_id} rowId={task.task.task_id} tabIndex={tabIndexFor(task.task.task_id)} view={{ task, worker, observed, snapshot: snapshot!, live: !!live, status: taskStatus(task, worker, snapshot!), tier, dimReason: dimFor(tier, worker?.run_id ?? null), selected: scope.selectedTask === task.task.task_id, linked: !!worker && (hoverRun === worker.run_id || selectedAgent?.run_id === worker.run_id), subagentCount: snapshot!.subagents.filter(agent => agent.run_id === worker?.run_id).length, showSpace: !!scope.spaceFilter || observed?.workspace_id !== rootSpace, sharedSpace }} onFocus={() => { focusedTask.current = task.task.task_id; }} onSelect={() => select({ task: task.task.task_id }, rowElement(task.task.task_id) ?? null, true, false)} onHover={entering => { setHoverRun(entering ? worker?.run_id ?? null : null); setHoverSpace(entering ? observed?.workspace_id ?? null : null); }} />;
  };
  const renderLane = (lane: TaskLane, label: string) => {
    const laneTasks = tasks.filter(task => task.lane === lane);
    const open = lane === "accepted" ? scope.disclosures.completed : laneTasks.length > 0 && !scope.view.collapsedLanes.includes(lane);
    const list = <ul ref={element => { laneRefs.current[lane] = element; }} className="supervisor-task-list" hidden={layout.narrow ? !open : lane === "accepted" && !open} onScroll={event => { if (!layout.narrow) scope.view.laneScroll[lane] = event.currentTarget.scrollTop; }}>{laneTasks.map(renderTask)}</ul>;
    if (layout.narrow) return <details key={lane} className={`supervisor-lane-group is-${lane}`} open={open} onToggle={event => { const next = event.currentTarget.open; if (next === open) return; if (lane === "accepted") scope.disclosures.completed = next; else scope.view.collapsedLanes = next ? scope.view.collapsedLanes.filter(item => item !== lane) : [...scope.view.collapsedLanes.filter(item => item !== lane), lane]; changed(); }}><summary className="supervisor-lane-header"><span className="supervisor-lane-dot" /><strong>{label}</strong><span className="supervisor-lane-count">{laneTasks.length}</span><UiIcon name={open ? "down" : "right"} /></summary>{list}</details>;
    return <section key={lane} className={`supervisor-lane is-${lane}${lane === "accepted" && !open ? " is-collapsed" : ""}`} aria-label={`${label} tasks`}><header className="supervisor-lane-header"><span className="supervisor-lane-dot" /><strong>{label}</strong><span className="supervisor-lane-count">{laneTasks.length}</span>{lane === "accepted" ? <button type="button" aria-label={open ? "Hide completed tasks" : "Show completed tasks"} aria-expanded={open} onClick={() => { scope.disclosures.completed = !open; changed(); }}><UiIcon name={open ? "down" : "right"} /></button> : null}</header>{list}{!laneTasks.length ? <p className="supervisor-lane-empty">No tasks</p> : null}</section>;
  };
        return <><div className="supervisor-viewbar"><div className="supervisor-viewbar-switch" role="group" aria-label="Workarea view"><button type="button" data-view-segment="tasks" aria-pressed={mode === "tasks"} onClick={() => switchView("tasks")}><UiIcon name="grid" />Tasks {tasks.filter(task => task.lane !== "accepted").length}</button><button type="button" data-view-segment="graph" aria-pressed={mode === "graph"} onClick={() => switchView("graph")}><UiIcon name="branch" />Graph {model.nodes.length}</button><button type="button" data-view-segment="dependencies" aria-pressed={mode === "dependencies"} aria-label={`Dependencies, ${tasks.filter(task => !task.task.checked && ["blocked", "invalid"].includes(task.dependencies.state)).length} tasks waiting`} onClick={() => switchView("dependencies")}>Dependencies {tasks.filter(task => !task.task.checked && ["blocked", "invalid"].includes(task.dependencies.state)).length || ""}</button></div><div className="supervisor-viewbar-filters" aria-label="Task filters"><button type="button" aria-pressed={scope.attentionOnly} onClick={() => { scope.attentionOnly = !scope.attentionOnly; changed(); }}>Attention · {attention?.total ?? 0}</button><label>Space<select aria-label="Task Space" value={scope.spaceFilter} onChange={event => { scope.spaceFilter = event.target.value; changed(); }}><option value="">All Spaces</option>{[...new Map(observations.filter(item => item.workspace_id && item.workspace_label && rootRuns.some(run => run.run_id === item.run_id)).map(item => [item.workspace_id!, item.workspace_label!])).entries()].map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></label></div></div>
        {mode === "graph" ? <SupervisorGraph
          model={model} snapshot={snapshot} live={!!live} connected={connected} runtimeLive={runtimeLive}
          selectedNodeId={selectedNode} highlightedRunId={hoverRun ?? detailRun?.run_id ?? null}
          tierFor={node => node.task ? attention?.tierForTask(node.task) ?? null : node.run ? attention?.tierForRun(node.run.run_id) ?? null : null}
          dimFor={node => dimFor(node.task ? attention?.tierForTask(node.task) ?? null : node.run ? attention?.tierForRun(node.run.run_id) ?? null : null, node.run?.run_id ?? node.assignedRunId)}
          showSubagents={scope.showSubagents}
          onShowSubagents={next => {
            scope.showSubagents = next;
            if (!next && scope.selectedSubagent && scope.selectedRun) {
              scope.selectedSubagent = null; revealIntent.current = { focus: true }; setDetailSection("overview");
            }
            changed();
          }}
          sharedSpace={sharedSpace} bottomInset={bottomInset} scrollRef={graphRef}
          onSelect={(node: TopologyNode) => select(nodeSelection(node), rowElement(node.id) ?? null, true, false)}
          onHover={id => { setHoverRun(id); setHoverSpace(observe(id)?.workspace_id ?? null); }}
          onEscape={() => { escapeLayer(); return true; }}
        /> : mode === "dependencies" ? <SupervisorDependencies tasks={tasks} snapshot={snapshot} selectedTaskId={scope.selectedTask} showCompleted={scope.disclosures.completed} onShowCompleted={() => { scope.disclosures.completed = true; changed(); }} narrow={layout.narrow} scrollRef={graphRef} attentionOnly={scope.attentionOnly} tierFor={task => attention?.tierForTask(task) ?? null} dimFor={task => scope.spaceFilter && observe(task.current_run_id)?.workspace_id !== scope.spaceFilter ? "Space filter" : null} onSelect={(task, invoker) => select({ task: task.task.task_id }, invoker, true, false)} onEscape={() => { escapeLayer(); return true; }} bottomInset={bottomInset} /> : <section className="supervisor-board-surface" aria-label="Tasks">
          {!layout.narrow && !layout.short ? <AgentsStrip
            heading={live ? `Agents · ${observedCount} observed` : "Agents · unobserved"} chips={chips}
            subagentCount={snapshot.subagents.filter(agent => rootRuns.some(run => run.run_id === agent.run_id && run.stage !== "closed")).length}
            onSelect={(id, invoker) => select({ run: id, subagent: null }, invoker, true, false)}
            onGraph={() => {
              switchView("graph");
              requestAnimationFrame(() => rootRef.current?.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')?.focus({ preventScroll: true }));
            }}
          /> : null}
          {tasks.length ? <div ref={listRef} {...listProps} className="supervisor-board"
            onScroll={event => { scope.view.offsets.tasks = readOffset(event.currentTarget); }}
            onKeyDown={event => {
              const id = (event.target as HTMLElement).dataset.rowId;
              if (id && !event.ctrlKey && !event.altKey && !event.metaKey && !event.nativeEvent.isComposing) {
                const next = taskNeighbor(tasks, id, event.key, navOptions);
                if (next) {
                  event.preventDefault();
                  const element = rowElement(next);
                  const scroller = layout.narrow ? listRef.current : element?.closest<HTMLUListElement>(".supervisor-task-list");
                  if (element && scroller) { revealNearest(scroller, element); element.focus({ preventScroll: true }); }
                  return;
                }
              }
              listProps.onKeyDown(event);
            }}>
            {taskLanes.map(({ lane, label }) => renderLane(lane, label))}
          </div> : <div className="supervisor-empty">
            <h2>No tasks yet</h2>
            <button type="button" disabled={busy || !live || !rootState?.terminal} onClick={() => void navigate(root)}>Open terminal</button>
          </div>}
        </section>}
        </>;
}
