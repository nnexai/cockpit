import { useId, useMemo, useState, type KeyboardEvent, type RefObject } from "react";
import type { OrchestrationSnapshot, TaskView } from "../../protocol/generated/v1";
import { useRovingList } from "../sidebar/useRovingList";
import { UiIcon } from "../UiIcon";
import { taskStatus } from "./SupervisorActions";
import { dependencyChain, dependencySummary, dependentCount, uniqueTask } from "./dependencies";
import { dependencyLayout } from "./dependencyLayout";
import type { AttentionTier } from "./attention";

export function DependenciesSection({ view, tasks, snapshot, onNavigate, onEdit, writable }: { view: TaskView; tasks: readonly TaskView[]; snapshot: OrchestrationSnapshot; onNavigate(taskId: string): void; onEdit(task: TaskView): void; writable: boolean }) {
  const dependents = tasks.filter(task => task.task.depends_on.includes(view.task.task_id));
  const followUps = tasks.filter(task => task.task.follow_up_of === view.task.task_id);
  const source = view.task.follow_up_of;
  if (!view.task.depends_on.length && !dependents.length && !followUps.length && !source && !view.dependencies.problems.length && !view.task.relations_diagnostic) return null;
  const row = (id: string, label: string) => {
    const target = label === "Prerequisite" && id === view.task.task_id ? null : uniqueTask(tasks, id), unmet = view.dependencies.unmet.find(item => item.task_id === id);
    const cause = id === view.task.task_id ? "Waits on itself" : unmet?.reason === "missing" ? "Not in this task file" : unmet?.reason === "ambiguous" ? "Prerequisite ID is duplicated in the task file" : "Unresolvable task identity";
    return <li key={id}>{target ? <button type="button" className="supervisor-path-row" onClick={() => onNavigate(id)} aria-label={`${label}: ${target.task.title}, ${target.task.checked ? "Accepted" : taskStatus(target, snapshot.runs.find(run => run.run_id === target.current_run_id), snapshot)}`}><strong>{target.task.checked ? "✓ " : "○ "}{target.task.title}</strong><span>{target.task.checked ? "Accepted" : taskStatus(target, snapshot.runs.find(run => run.run_id === target.current_run_id), snapshot)}</span></button> : <div className="supervisor-dependency-missing"><span>{source === id && label === "Follow-up of" ? "Follow-up of a removed task" : cause} · {id}</span>{writable && label === "Prerequisite" ? <button type="button" onClick={() => onEdit(view)}>Fix prerequisites…</button> : null}</div>}</li>;
  };
  const list = (name: string, ids: readonly string[], label: string) => ids.length ? <section><h4>{name}</h4><ul aria-label={name}>{ids.slice(0, 5).map(id => row(id, label))}</ul>{ids.length > 5 ? <details><summary>Show {ids.length - 5} more</summary><ul>{ids.slice(5).map(id => row(id, label))}</ul></details> : null}</section> : null;
  return <section className="supervisor-detail-group supervisor-dependencies" aria-label="Dependencies"><h3>Dependencies</h3>
    {view.task.depends_on.length ? <p>{view.dependencies.state === "invalid" ? "Prerequisites need fixing; acceptance count is not trustworthy." : `${view.task.depends_on.length - view.dependencies.unmet.length} of ${view.task.depends_on.length} prerequisites accepted.`} A Result alone does not satisfy a prerequisite.</p> : null}
    {view.task.relations_diagnostic ? <p className="supervisor-warning">{view.task.relations_diagnostic}</p> : null}
    {view.dependencies.problems.map((problem, index) => <p className="supervisor-warning" key={`${problem.code}-${index}`}>{problem.message}</p>)}
    {view.dependencies.state === "invalid" && writable ? <button type="button" onClick={() => onEdit(view)}>Fix prerequisites…</button> : null}
    {list("Prerequisites · waits for all", view.task.depends_on, "Prerequisite")}
    {list(`Dependents · ${dependentCount(view, tasks)} waiting on this`, dependents.map(task => task.task.task_id), "Dependent")}
    {list("Follow-ups · created from this task", followUps.map(task => task.task.task_id), "Follow-up")}
    {source ? list("Follow-up of · provenance only, not a prerequisite", [source], "Follow-up of") : null}
  </section>;
}

export function SupervisorDependencies({ tasks, snapshot, selectedTaskId, showCompleted, onShowCompleted, narrow, scrollRef, attentionOnly, tierFor, dimFor, onSelect, onEscape, bottomInset }: {
  tasks: readonly TaskView[]; snapshot: OrchestrationSnapshot; selectedTaskId: string | null; showCompleted: boolean; onShowCompleted(): void; narrow: boolean;
  scrollRef: RefObject<HTMLDivElement | null>; attentionOnly: boolean; tierFor(task: TaskView): AttentionTier | null; dimFor(task: TaskView): string | null;
  onSelect(task: TaskView, invoker: HTMLElement): void; onEscape(): boolean; bottomInset: number;
}) {
  const marker = useId().replaceAll(":", ""), [hover, setHover] = useState<string | null>(null);
  const model = useMemo(() => dependencyLayout(tasks, showCompleted), [tasks, showCompleted]);
  const ids = model.positions.map(position => `task:${position.view.task.task_id}`);
  const roving = useRovingList({ rowIds: ids, selectedId: selectedTaskId ? `task:${selectedTaskId}` : null, onEscape });
  const highlighted = dependencyChain(tasks, hover ? [hover] : selectedTaskId ? [selectedTaskId] : []);
  const attentionChain = attentionOnly ? dependencyChain(tasks, tasks.filter(task => tierFor(task)).map(task => task.task.task_id)) : null;
  const waiting = tasks.filter(task => !task.task.checked && ["blocked", "invalid"].includes(task.dependencies.state)).length;
  const node = (position: typeof model.positions[number]) => {
    const view = position.view, id = view.task.task_id, worker = snapshot.runs.find(run => run.run_id === view.current_run_id);
    const tier = tierFor(view), dim = attentionChain && !attentionChain.has(id) ? "attention filter" : dimFor(view);
    const status = view.task.checked ? "Accepted" : view.dependencies.state === "invalid" ? "Prerequisites need fixing" : view.dependencies.state === "blocked" ? `Waiting on ${view.dependencies.unmet.length}` : worker ? taskStatus(view, worker, snapshot) : "Ready";
    return <button type="button" key={id} data-row-id={`task:${id}`} tabIndex={roving.tabIndexFor(`task:${id}`)} className={`supervisor-graph-node supervisor-dependency-node${selectedTaskId === id ? " is-selected" : ""}${highlighted.has(id) ? " is-highlighted" : ""}${dim ? " is-dimmed" : ""}${view.task.checked ? " is-accepted" : ""}${position.cycle || view.dependencies.state === "invalid" ? " is-diagnosed" : ""}`} style={narrow ? undefined : { left: position.x, top: position.y }} aria-expanded={selectedTaskId === id} aria-controls="supervisor-detail" aria-label={`${view.task.title}, ${status}${tier ? `, ${tier}` : ""}${dim ? `, dimmed by ${dim}` : ""}`} onClick={event => onSelect(view, event.currentTarget)} onMouseEnter={() => setHover(id)} onMouseLeave={() => setHover(null)}><span className="supervisor-dependency-title">{view.task.checked ? "✓" : view.dependencies.state === "invalid" ? <UiIcon name="info" /> : "○"} {view.task.title}{tier ? ` · ${tier === "notice" ? "Notice" : tier === "recover" ? "Recover" : "Decide"}` : ""}</span><span>{status} · {worker?.label ?? "not assigned"}{tier && dependentCount(view, tasks) ? ` · holds ${dependentCount(view, tasks)}` : ""}{view.task.follow_up_of ? ` · ↳ follow-up of ${uniqueTask(tasks, view.task.follow_up_of)?.task.title ?? "removed task"}` : ""}</span></button>;
  };
  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!narrow && (event.key === "ArrowLeft" || event.key === "ArrowRight" || event.key === "ArrowUp" || event.key === "ArrowDown") && !event.nativeEvent.isComposing && !event.altKey && !event.ctrlKey && !event.metaKey) {
      const id = (event.target as HTMLElement).dataset.rowId?.slice(5), current = model.positions.find(position => position.view.task.task_id === id);
      if (!current) return;
      event.preventDefault();
      let candidates: typeof model.positions;
      if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
        const relatives = event.key === "ArrowLeft" ? current.view.task.depends_on : tasks.filter(task => task.task.depends_on.includes(id!)).map(task => task.task.task_id);
        candidates = model.positions.filter(position => relatives.includes(position.view.task.task_id)).sort((a, b) => Math.abs(a.y - current.y) - Math.abs(b.y - current.y) || a.view.task.line - b.view.task.line);
      } else candidates = model.positions.filter(position => position.level === current.level && position.independent === current.independent && (event.key === "ArrowUp" ? position.y < current.y : position.y > current.y)).sort((a, b) => Math.abs(a.y - current.y) - Math.abs(b.y - current.y));
      if (candidates[0]) roving.focusRow(`task:${candidates[0].view.task.task_id}`);
      return;
    }
    roving.listProps.onKeyDown(event);
  };
  const positions = new Map(model.positions.map(position => [position.view.task.task_id, position]));
  return <section className="supervisor-dependency-surface" aria-label="Dependencies"><header className="supervisor-graph-heading"><h2>Dependencies · {tasks.length} tasks · {waiting} waiting · {tasks.filter(task => tierFor(task)).length} need attention</h2>{model.hiddenCompleted ? <button type="button" onClick={onShowCompleted}>{model.hiddenCompleted} completed tasks hidden · Show completed</button> : null}</header>{tasks.some(view => !uniqueTask(tasks, view.task.task_id)) ? <p className="supervisor-warning">Task identities are duplicated. Ambiguous tasks are not selectable; repair the canonical task file before editing their relationships.</p> : null}<div ref={scrollRef} className="supervisor-dependency-scroll" style={{ paddingBottom: bottomInset }}><div ref={roving.listRef} {...roving.listProps} onKeyDown={keys} className={narrow ? "supervisor-dependency-outline" : "supervisor-dependency-canvas"} style={narrow ? undefined : { width: model.width, minHeight: model.height }}>
    {narrow ? <>{[...new Set(model.positions.map(position => position.independent ? -1 : position.level))].map(level => <section key={level}><h3>{level === -1 ? "Independent" : `Level ${level + 1}`}</h3><ul>{model.positions.filter(position => (position.independent ? -1 : position.level) === level).map(position => <li key={position.view.task.task_id}>{node(position)}{position.view.task.depends_on.length ? <p>After: {position.view.task.depends_on.map(id => uniqueTask(tasks, id)?.task.title ?? `Unavailable task identity: ${id}`).join(" · ")}</p> : null}{position.cycle ? <p className="supervisor-warning">Dependency cycle · repair this group's prerequisites.</p> : null}</li>)}</ul></section>)}</> : <><svg width={model.width} height={model.height} aria-hidden="true"><defs><marker id={marker} markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto"><path d="M0,0 L6,3 L0,6" fill="currentColor" /></marker><marker id={`${marker}-provenance`} markerWidth="6" markerHeight="6" refX="3" refY="3"><circle cx="3" cy="3" r="2" fill="currentColor" /></marker></defs>{model.cycleGroups.map((group, index) => <rect key={`cycle-${index}`} className="supervisor-dependency-cycle" x={group.x} y={group.y} width={252} height={group.height} rx={6} />)}{model.edges.map(edge => {
      if (edge.provenance && edge.from !== selectedTaskId && edge.to !== selectedTaskId) return null;
      const from = positions.get(edge.from)!, to = positions.get(edge.to)!;
      return <path key={`${edge.from}-${edge.to}-${edge.provenance}`} className={`supervisor-dependency-edge${edge.provenance ? " is-provenance" : ""}${positions.get(edge.from)!.view.task.checked ? " is-satisfied" : ""}${highlighted.has(edge.from) && highlighted.has(edge.to) ? " is-highlighted" : ""}`} d={`M${from.x + 240},${from.y + 24} C${from.x + 256},${from.y + 24} ${to.x - 16},${to.y + 24} ${to.x},${to.y + 24}`} markerStart={edge.provenance ? `url(#${marker}-provenance)` : undefined} markerEnd={edge.provenance ? undefined : `url(#${marker})`} />;
    })}</svg><div className="supervisor-dependency-columns">{[...new Set(model.positions.filter(position => !position.independent).map(position => position.level))].map(level => <span key={level} className="supervisor-dependency-level" style={{ left: 16 + level * 272 }}>Level {level + 1}{level === 0 ? " · prerequisite side" : ""}</span>)}</div>{model.positions.some(position => position.independent) ? <h3 className="supervisor-dependency-independent" style={{ top: model.independentTop }}>Independent · {model.positions.filter(position => position.independent).length}</h3> : null}{model.positions.map(node)}{model.positions.filter(position => position.view.dependencies.unmet.some(item => item.reason !== "unchecked")).map(position => <span key={`missing-${position.view.task.task_id}`} className="supervisor-dependency-stub" style={{ left: position.x, top: position.y + 52 }}>{dependencySummary(position.view, tasks)}</span>)}</>}
  </div></div></section>;
}
