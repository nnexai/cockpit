import { useId, useMemo, type KeyboardEvent, type RefObject } from "react";
import type { OrchestrationSnapshot } from "../../protocol/generated/v1";
import { StateGlyph } from "../sidebar/StateGlyph";
import { useRovingList } from "../sidebar/useRovingList";
import { UiIcon } from "../UiIcon";
import { agentState } from "./SupervisorActions";
import { TIER_LABEL, type AttentionTier } from "./attention";
import { edgePath, GRAPH_GEOMETRY } from "./graphLayout";
import { revealNearest } from "./reveal";
import { chainIds, firstChild, nodeFacts, nodeId, type SupervisorGraphModel, type TopologyNode } from "./topology";

export type SupervisorGraphProps = {
  model: SupervisorGraphModel;
  snapshot: OrchestrationSnapshot;
  live: boolean; connected: boolean; runtimeLive: boolean;
  selectedNodeId: string | null; highlightedRunId: string | null;
  tierFor(node: TopologyNode): AttentionTier | null;
  dimFor(node: TopologyNode): string | null;
  showSubagents: boolean; onShowSubagents(next: boolean): void;
  sharedSpace: string | null | undefined;
  bottomInset: number; scrollRef: RefObject<HTMLDivElement | null>;
  onSelect(node: TopologyNode): void;
  onHover(runId: string | null): void;
  onEscape(): boolean;
};

/** Observation and selection only. No terminal or orchestration control callback is accepted. */
export function SupervisorGraph({ model, snapshot, live, connected, runtimeLive, selectedNodeId, highlightedRunId, tierFor, dimFor,
  showSubagents, onShowSubagents, sharedSpace, bottomInset, scrollRef, onSelect, onHover, onEscape }: SupervisorGraphProps) {
  const headingId = useId();
  const fresh = live && connected && runtimeLive && snapshot.runtime.status === "fresh";
  const observations = useMemo(() => new Map(fresh && snapshot.runtime.status === "fresh" ? snapshot.runtime.runs.map(run => [run.run_id, run]) : []), [fresh, snapshot.runtime]);
  const positions = useMemo(() => new Map(model.layout.positions.map(position => [position.id, position])), [model.layout]);
  const highlighted = useMemo(() => {
    const ids = new Set(selectedNodeId ? chainIds(model, selectedNodeId) : []);
    if (highlightedRunId) for (const id of chainIds(model, nodeId.run(highlightedRunId))) ids.add(id);
    return ids;
  }, [model, selectedNodeId, highlightedRunId]);
  const roving = useRovingList({ rowIds: model.layout.order, selectedId: selectedNodeId });
  const connectedCount = model.nodes.filter(node => node.kind !== "subagent" && node.run && observations.get(node.run.run_id)?.presence === "present" && observations.get(node.run.run_id)?.actual_omp).length;
  const onGraphKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const id = (event.target as HTMLElement).dataset.rowId;
    if (id === undefined || event.ctrlKey || event.altKey || event.metaKey || event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      if (onEscape()) { event.preventDefault(); event.stopPropagation(); }
      return;
    }
    const order = model.layout.order, index = order.indexOf(id);
    let target: string | null | undefined;
    switch (event.key) {
      case "ArrowLeft": target = model.layout.parentOf.get(id); break;
      case "ArrowRight": target = firstChild(model, id); break;
      case "ArrowUp": target = order[Math.max(0, index - 1)]; break;
      case "ArrowDown": target = order[Math.min(order.length - 1, index + 1)]; break;
      case "Home": target = order[0]; break;
      case "End": target = order[order.length - 1]; break;
      default: return;
    }
    event.preventDefault(); event.stopPropagation();
    if (!target || !scrollRef.current) return;
    const element = [...scrollRef.current.querySelectorAll<HTMLElement>("[data-row-id]")].find(row => row.dataset.rowId === target);
    if (element) {
      revealNearest(scrollRef.current, element, { top: GRAPH_GEOMETRY.headerHeight, bottom: bottomInset });
      element.focus({ preventScroll: true });
    }
  };
  const columnLabels = ["Supervisor", "Tasks", "Workers", "Subagents"];
  return <section className="supervisor-graph-band" aria-labelledby={headingId}>
    <div className="supervisor-graph-heading">
      <h2 id={headingId} title="Up/Down moves through nodes, Left/Right along the chain">
        Agents · {fresh ? `${connectedCount} connected` : "unobserved"} · {model.counts.subagents} subagents · {model.counts.tasks} tasks
      </h2>
      {model.hiddenCompletedTasks > 0 ? <span className="supervisor-graph-hidden-completed">{model.hiddenCompletedTasks} completed tasks hidden</span> : null}
      <label className="supervisor-graph-subagents"><input type="checkbox" checked={showSubagents} onChange={event => onShowSubagents(event.target.checked)} />Subagents</label>
    </div>
    {model.counts.workers === 0 && model.counts.supervisors > 0 ? <p className="supervisor-graph-empty">No worker agents yet</p> : null}
    <div className="supervisor-graph-scroll" role="region" aria-label="Agent graph, scrollable" ref={element => { roving.listRef.current = element; scrollRef.current = element; }}
      onFocus={roving.listProps.onFocus} onBlur={roving.listProps.onBlur} onKeyDown={onGraphKey}>
      {model.nodes.length ? <>
        <div className="supervisor-graph-columns" style={{ width: model.layout.width, height: GRAPH_GEOMETRY.headerHeight }} aria-hidden="true">
          {Array.from({ length: model.layout.columns }, (_, column) => <span key={column} className="supervisor-graph-column" style={{
            left: GRAPH_GEOMETRY.padding + column * (GRAPH_GEOMETRY.nodeWidth + GRAPH_GEOMETRY.columnGap) + 2, width: GRAPH_GEOMETRY.nodeWidth,
          }}>{columnLabels[column] ?? "Nested"}</span>)}
        </div>
        <div className="supervisor-graph-canvas" role="group" aria-label="Agent relationships" style={{ width: model.layout.width, height: model.layout.height }}>
          <svg className="supervisor-graph-edges" width={model.layout.width} height={model.layout.height} viewBox={`0 0 ${model.layout.width} ${model.layout.height}`} aria-hidden="true" focusable="false">
            {model.layout.edges.map(edge => <path key={edge.to} className={`supervisor-graph-edge is-${model.byId.get(edge.to)!.edge ?? "delegated"}${highlighted.has(edge.from) && highlighted.has(edge.to) ? " is-highlighted" : ""}`} d={edge.path} />)}
            {model.links.map(link => <path key={`${link.from}|${link.to}`} className={`supervisor-graph-edge is-assigned is-link${highlighted.has(link.from) && highlighted.has(link.to) ? " is-highlighted" : ""}`}
              d={edgePath(positions.get(link.from)!, positions.get(link.to)!)} />)}
          </svg>
          {model.nodes.map(node => {
            const position = positions.get(node.id)!;
            const facts = nodeFacts(node, { model, snapshot, live, connected, runtimeLive });
            const tier = tierFor(node), dim = dimFor(node), selected = selectedNodeId === node.id;
            const observed = observations.get(node.run?.run_id ?? node.assignedRunId ?? "");
            const shared = observed?.presence === "present" && !!sharedSpace && observed.workspace_id === sharedSpace;
            const name = [facts.title, facts.role, facts.status, facts.provenance, facts.relation, tier ? TIER_LABEL[tier] : null, dim ? `dimmed by ${dim}` : null].filter(Boolean).join(", ");
            const detail = node.subagent ? node.subagent.summary : node.run ? agentState(snapshot, node.run, connected, runtimeLive).detail : null;
            return <button key={node.id} type="button" data-row-id={node.id} data-node-kind={node.kind} tabIndex={roving.tabIndexFor(node.id)}
              aria-expanded={selected} aria-label={name} title={`${name}${detail ? `\n${detail}` : ""}`} className={`supervisor-graph-node is-${node.kind}${selected ? " is-selected" : ""}${highlighted.has(node.id) ? " is-highlighted" : ""}${node.unassigned ? " is-unassigned" : ""}${dim ? " is-dimmed" : ""}`}
              style={{ left: position.x, top: position.y, width: position.width, height: position.height }}
              onMouseEnter={() => onHover(node.run?.run_id ?? node.assignedRunId)} onMouseLeave={() => onHover(null)} onClick={() => onSelect(node)}>
              <span className="supervisor-graph-node-icon" aria-hidden="true">{facts.glyph === "document" ? <UiIcon name="file" /> : <StateGlyph shape={facts.glyph} />}</span>
              <strong className="supervisor-graph-node-title">{facts.title}</strong>
              {tier ? <span className={`supervisor-tier is-${tier}`}><span className="supervisor-tier-glyph" aria-hidden="true">{tier === "decide" ? "?" : tier === "recover" ? "!" : "i"}</span><span className="supervisor-tier-label">{TIER_LABEL[tier]}</span></span> : null}
              <span className={`supervisor-graph-node-metadata${shared ? " is-shared" : ""}`}>
                <span className="supervisor-graph-node-status">{facts.status}</span>
                <span className="supervisor-graph-node-provenance">{facts.provenance}</span>
              </span>
            </button>;
          })}
        </div>
        {bottomInset > 0 ? <div className="supervisor-graph-bottom-inset" style={{ height: bottomInset }} aria-hidden="true" /> : null}
      </> : <p className="supervisor-graph-empty">No agents in this task scope.</p>}
    </div>
  </section>;
}
