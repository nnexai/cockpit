import { useId, useMemo } from "react";
import type { OrchestrationSnapshot, Run, Subagent } from "../../protocol/generated/v1";
import { StateGlyph, type GlyphShape } from "../sidebar/StateGlyph";
import { useRovingList } from "../sidebar/useRovingList";
import { UiIcon } from "../UiIcon";
import { agentState } from "./SupervisorActions";
import { graphLayout } from "./graphLayout";
import type { ScopeDrafts } from "./useSupervisorDrafts";

export type SupervisorGraphRow = { key: string; depth: number; run: Run; subagent: Subagent | null };
export type SupervisorGraphProps = {
  rows: SupervisorGraphRow[];
  snapshot: OrchestrationSnapshot;
  live: boolean;
  connected: boolean;
  runtimeLive: boolean;
  scope: ScopeDrafts;
  sharedSpace: string | null | undefined;
  highlightedRun: string | null;
  onHover(run: Run | null): void;
  onSelect(run: Run, subagent: Subagent | null): void;
  onSelectTask?(taskId: string): void;
  changed(): void;
};

export function SupervisorGraph({ rows, snapshot, live, connected, runtimeLive, scope, sharedSpace, highlightedRun, onHover, onSelect, onSelectTask, changed }: SupervisorGraphProps) {
  const headingId = useId();
  const fresh = live && connected && runtimeLive && snapshot.runtime.status === "fresh";
  const observations = fresh && snapshot.runtime.status === "fresh" ? snapshot.runtime.runs : [];
  const visibleRows = useMemo(() => scope.showSubagents ? rows : rows.filter(row => !row.subagent), [rows, scope.showSubagents]);
  const graph = useMemo(() => {
    const nodes = visibleRows.map(row => ({
      id: row.key,
      parentId: row.subagent
        ? row.subagent.parent_subagent_id ? `${row.run.run_id}:${row.subagent.parent_subagent_id}` : row.run.run_id
        : row.run.parent_run_id,
    }));
    const taskRefs = visibleRows.flatMap(row => {
      if (row.subagent || row.run.kind !== "worker" || snapshot.board?.root_id !== row.run.root_id) return [];
      const task = snapshot.board.tasks.find(task => task.task.task_id === row.run.task_id);
      return task ? [{ id: `task:${row.run.run_id}:${task.task.task_id}`, run: row.run, task: task.task }] : [];
    });
    const layout = graphLayout(nodes);
    const positionsById = new Map(layout.positions.map(position => [position.id, position]));
    return { ...layout, taskRefs, positionsById };
  }, [visibleRows, snapshot.board]);
  const selectedId = scope.selectedRun ? scope.selectedSubagent ? `${scope.selectedRun}:${scope.selectedSubagent}` : scope.selectedRun : null;
  const highlightedIds = new Set<string>();
  if (selectedId) highlightedIds.add(selectedId);
  if (highlightedRun) highlightedIds.add(highlightedRun);
  for (const reference of graph.taskRefs) if (reference.task.task_id === scope.selectedTask) {
    highlightedIds.add(reference.id);
    highlightedIds.add(reference.run.run_id);
  }
  const highlightedEdges = new Set<string>();
  for (const id of highlightedIds) {
    let current = id;
    const visited = new Set<string>();
    while (!visited.has(current)) {
      visited.add(current);
      const edge = graph.edges.find(edge => edge.to === current);
      if (!edge) break;
      highlightedEdges.add(`${edge.from}:${edge.to}`);
      current = edge.from;
    }
  }
  const roving = useRovingList({
    rowIds: [...visibleRows.map(row => row.key), ...(onSelectTask ? graph.taskRefs.map(reference => reference.id) : [])],
    selectedId,
    onEscape: () => {
      scope.selectedRun = null;
      scope.selectedSubagent = null;
      scope.selectedTask = null;
      changed();
      return true;
    },
  });
  const connectedCount = visibleRows.filter(row => !row.subagent && observations.some(observed => observed.run_id === row.run.run_id && observed.actual_omp && observed.presence === "present")).length;

  return <section className="supervisor-graph-band" aria-labelledby={headingId}>
    <div className="supervisor-graph-heading">
      <h2 id={headingId}>Agents · {fresh ? `${connectedCount} connected` : "unobserved"}</h2>
      <label className="supervisor-graph-subagents"><input type="checkbox" checked={scope.showSubagents} disabled={!rows.some(row => !row.subagent)} onChange={event => {
        scope.showSubagents = event.target.checked;
        if (!scope.showSubagents && scope.selectedSubagent) {
          scope.selectedSubagent = null;
          const parent = scope.selectedRun;
          requestAnimationFrame(() => roving.focusRow(parent ?? undefined));
        }
        changed();
      }} />Subagents</label>
    </div>
    <div className="supervisor-graph-scroll" ref={roving.listRef} {...roving.listProps}>
      {visibleRows.length ? <div className="supervisor-graph-canvas" role="group" aria-label="Agent relationships" style={{ width: graph.width, height: graph.height }}>
        <svg className="supervisor-graph-edges" width={graph.width} height={graph.height} viewBox={`0 0 ${graph.width} ${graph.height}`} aria-hidden="true" focusable="false">
          {graph.edges.map(edge => <path key={`${edge.from}:${edge.to}`} className={`supervisor-graph-edge${highlightedEdges.has(`${edge.from}:${edge.to}`) ? " is-highlighted" : ""}`} d={edge.path} fill="none" stroke="var(--border-strong)" />)}
        </svg>
        {visibleRows.map(row => {
          const position = graph.positionsById.get(row.key)!;
          const observed = observations.find(observation => observation.run_id === row.run.run_id);
          const state = agentState(snapshot, row.run, connected, runtimeLive);
          const rawStatus = row.subagent ? row.subagent.status : fresh && state.verified ? observed?.agent_status ?? "unknown" : "unobserved";
          const shape: GlyphShape = row.subagent
            ? rawStatus === "running" ? "working" : rawStatus === "done" ? "done" : rawStatus === "failed" ? "blocked" : "unknown"
            : rawStatus === "working" || rawStatus === "idle" || rawStatus === "blocked" || rawStatus === "done" ? rawStatus : "unknown";
          const selected = selectedId === row.key;
          const label = row.subagent?.label ?? row.run.label;
          const role = row.subagent?.role ?? (row.subagent ? "OMP subagent" : row.run.kind === "worker" ? "Task agent" : "Supervisor");
          const space = observed?.presence === "present" ? observed.workspace_label : null;
          const evidence = `${row.subagent ? "OMP events" : "Herdr"} · ${rawStatus.replaceAll("_", " ")}`;
          return <button key={row.key} type="button" data-row-id={row.key} tabIndex={roving.tabIndexFor(row.key)} aria-expanded={selected} aria-label={`${label}, ${role}, ${evidence}${space ? `, Space ${space}` : ""}`} title={row.subagent ? `${evidence} · ${row.subagent.updated_at}${row.subagent.summary ? `\n${row.subagent.summary}` : ""}` : `${state.label}\n${state.detail}`} className={`supervisor-graph-node${selected ? " is-selected" : ""}${highlightedIds.has(row.key) ? " is-highlighted" : ""}${row.subagent ? " is-subagent" : ""}`} style={{ left: position.x, top: position.y, width: position.width, height: position.height }} onMouseEnter={() => onHover(row.run)} onMouseLeave={() => onHover(null)} onClick={() => onSelect(row.run, row.subagent)}>
            <span className="supervisor-graph-node-heading"><StateGlyph shape={shape} /><strong>{label}</strong></span>
            <span className={`supervisor-graph-node-metadata${space && observed?.workspace_id === sharedSpace ? " is-shared" : ""}`} title={`${role} · ${evidence} · ${space ? `Space ${space}${observed?.tab_label ? ` · ${observed.tab_label}` : ""}` : row.subagent ? `In ${row.run.label} · no terminal` : "Space unobserved"}`}>{row.subagent ? role : row.run.kind === "worker" ? "Worker" : "Supervisor"} · {row.subagent ? "OMP " : ""}{rawStatus.replaceAll("_", " ")} · {space ?? (row.subagent ? "no terminal" : "unobserved")}</span>
          </button>;
        })}
      </div> : <p className="supervisor-graph-empty">No agents in this task scope.</p>}
      {graph.taskRefs.length ? <div className="supervisor-graph-task-links" role="group" aria-label="Worker task references">{graph.taskRefs.map(reference => {
        const className = `supervisor-graph-task${highlightedIds.has(reference.id) || highlightedIds.has(reference.run.run_id) ? " is-highlighted" : ""}${scope.selectedTask === reference.task.task_id ? " is-selected" : ""}`;
        const title = `${reference.run.label} → Task · ${reference.task.title}`;
        const content = <><UiIcon name="file" /><span>{title}</span></>;
        return onSelectTask ? <button key={reference.id} type="button" className={className} data-row-id={reference.id} tabIndex={roving.tabIndexFor(reference.id)} title={title} aria-label={`View task ${reference.task.title} assigned to ${reference.run.label}`} onMouseEnter={() => onHover(reference.run)} onMouseLeave={() => onHover(null)} onClick={() => onSelectTask(reference.task.task_id)}>{content}</button> : <span key={reference.id} className={className} title={title}>{content}</span>;
      })}</div> : null}
    </div>
  </section>;
}
