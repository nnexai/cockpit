import { useLayoutEffect, useRef, useState } from "react";
import type { OrchestrationSnapshot, Run, RunObservation, TaskView } from "../../protocol/generated/v1";
import { StateGlyph, type GlyphShape } from "../sidebar/StateGlyph";
import { UiIcon } from "../UiIcon";
import { ProvenancePair, reportAge } from "./SupervisorActions";
import { TIER_LABEL, type AttentionTier } from "./attention";
import { taskLanes } from "./boardNavigation";
import { dependencySummary, uniqueTask } from "./dependencies";

export type TaskCardView = {
  task: TaskView; worker: Run | undefined; observed: RunObservation | undefined; snapshot: OrchestrationSnapshot;
  live: boolean; status: string; tier: AttentionTier | null; dimReason: string | null; selected: boolean; linked: boolean;
  subagentCount: number; showSpace: boolean; sharedSpace: string | null | undefined;
};
export function TaskCard({ view: v, rowId, tabIndex, onFocus, onSelect, onHover }: {
  view: TaskCardView; rowId: string; tabIndex: number; onFocus(): void; onSelect(): void; onHover(entering: boolean): void;
}) {
  const laneLabel = taskLanes.find(lane => lane.lane === v.task.lane)?.label ?? v.task.lane;
  const status = v.status.toLowerCase() === laneLabel.toLowerCase() ? null : v.status;
  const dependencyLine = dependencySummary(v.task, v.snapshot.board?.tasks ?? []);
  const source = v.task.task.follow_up_of ? uniqueTask(v.snapshot.board?.tasks ?? [], v.task.task.follow_up_of) : null;
  const followUpLine = v.task.task.follow_up_of ? `Follow-up of ${source ? `“${source.task.title}”` : "a removed task"}` : null;
  const progress = v.task.task.step_progress;
  const stepLine = !progress ? "Steps unavailable" : progress.total ? `Steps ${progress.done}/${progress.total}` : v.task.task.steps.some(step => !step.step_id) ? "Checklist not tracked" : null;
  const label = [v.task.task.title, v.status, v.worker?.label, v.subagentCount && `+${v.subagentCount} subagents`, dependencyLine, followUpLine, stepLine && progress?.total ? `${progress.done} of ${progress.total} steps complete` : stepLine, v.tier && TIER_LABEL[v.tier], v.dimReason && `dimmed by ${v.dimReason}`].filter(Boolean).join(", ");
  return <li className={`supervisor-task supervisor-card${v.selected ? " is-selected" : ""}${v.dimReason ? " is-dimmed" : ""}${v.linked ? " is-linked" : ""}`} onMouseEnter={() => onHover(true)} onMouseLeave={() => onHover(false)}>
    <button type="button" className="supervisor-task-toggle supervisor-card-toggle" data-row-id={rowId} tabIndex={tabIndex} aria-label={label} aria-expanded={v.selected} aria-controls="supervisor-detail" onFocus={onFocus} onClick={onSelect}>
      <span className="supervisor-card-heading"><span className="supervisor-task-title">{v.task.task.title}</span>{v.tier ? <span className={`supervisor-attention-badge is-${v.tier}`}><StateGlyph shape={v.tier === "decide" ? "blocked" : v.tier === "recover" ? "unknown" : "idle"} />{TIER_LABEL[v.tier]}</span> : null}</span>
      <div className="supervisor-card-meta">
        {status ? <span className="supervisor-task-stage">{status}</span> : null}
        {v.worker ? <span className="supervisor-worker-chip">{v.worker.label}</span> : null}
        {v.subagentCount ? <span className="supervisor-card-subagents">+{v.subagentCount} subagents</span> : null}
      {dependencyLine ? <span className={`supervisor-card-dependencies${v.task.dependencies.state === "invalid" ? " is-diagnosed" : ""}`} aria-hidden="true">{dependencyLine}</span> : null}
      {followUpLine ? <span className="supervisor-card-followup" aria-hidden="true">↳ {followUpLine}</span> : null}
      {stepLine ? <span className="supervisor-card-steps" aria-hidden="true">{stepLine}{progress && progress.total > 0 ? <span className="supervisor-step-meter"><span style={{ width: `${Math.min(100, progress.done / progress.total * 100)}%` }} /></span> : null}</span> : null}
      <div className="supervisor-task-evidence">
        {v.worker?.stage === "closed" && v.worker.last_report ? <span
          className="supervisor-pair-reported" title={new Date(v.worker.last_report.at).toLocaleString()}
          aria-label={`Reported by ${v.worker.label}: ${reportAge(v.worker.last_report)} · ${new Date(v.worker.last_report.at).toLocaleString()}`}>
          reported <time dateTime={v.worker.last_report.at}>{reportAge(v.worker.last_report)}</time>
        </span> : v.worker ? <ProvenancePair run={v.worker} report={v.worker.last_report} observed={v.observed} snapshot={v.snapshot} live={v.live} compact /> : null}
        {v.showSpace && v.observed?.workspace_label ? <span
          className={`supervisor-location${v.sharedSpace === v.observed.workspace_id ? " is-shared" : ""}`}
          title={v.observed.tab_label ? `${v.observed.workspace_label} · ${v.observed.tab_label}` : v.observed.workspace_label}>
          <UiIcon name="folder" />{v.observed.workspace_label}
        </span> : null}
      </div>
      </div>
      {v.worker?.last_report?.outcome === "failed" ? <span className="supervisor-card-failure supervisor-exact-text">{v.worker.last_report.summary}</span> : null}
    </button>
  </li>;
}
export type StripChip = { runId: string; label: string; glyph: GlyphShape; status: string; tier: AttentionTier | null; selected: boolean };
export function AgentsStrip({ heading, chips, subagentCount, onSelect, onGraph }: {
  heading: string; chips: readonly StripChip[]; subagentCount: number; onSelect(runId: string, invoker: HTMLElement): void; onGraph(): void;
}) {
  const strip = useRef<HTMLDivElement>(null);
  const measure = useRef<HTMLDivElement>(null);
  const [visibleCount, setVisibleCount] = useState(chips.length);
  useLayoutEffect(() => {
    const update = () => {
      const width = strip.current?.getBoundingClientRect().width ?? 0;
      if (!width || !measure.current) { setVisibleCount(chips.length); return; }
      const sizes = [...measure.current.children].map(child => child.getBoundingClientRect().width + 6);
      let occupied = 0, count = 0;
      // Reserve the disclosure chip before admitting each next complete chip.
      const reserve = 76;
      for (const size of sizes) { if (occupied + size > width - reserve) break; occupied += size; count++; }
      setVisibleCount(count);
    };
    update();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    if (strip.current) observer?.observe(strip.current);
    return () => observer?.disconnect();
  }, [chips]);
  const chip = (item: StripChip, measuring: boolean) => <button key={item.runId} type="button" tabIndex={measuring ? -1 : undefined} className={`supervisor-strip-chip${item.selected ? " is-selected" : ""}${item.tier ? ` is-${item.tier}` : ""}`} aria-label={`${item.label}, ${item.status}${item.tier ? `, ${TIER_LABEL[item.tier]}` : ""}`} onClick={event => onSelect(item.runId, event.currentTarget)}><StateGlyph shape={item.glyph} /><span>{item.label}</span></button>;
  return <section className="supervisor-strip" aria-label="Agents"><span className="supervisor-strip-heading">{heading}</span><div className="supervisor-strip-chips" ref={strip}>{chips.slice(0, visibleCount).map(item => chip(item, false))}{visibleCount < chips.length ? <button type="button" className="supervisor-strip-more" onClick={onGraph}>+{chips.length - visibleCount} more</button> : null}</div>{subagentCount ? <span className="supervisor-strip-subagents">+{subagentCount} subagents</span> : null}<button type="button" className="supervisor-strip-graph" onClick={onGraph}>Graph <UiIcon name="right" /></button><div ref={measure} className="supervisor-strip-measure" aria-hidden="true">{chips.map(item => chip(item, true))}</div></section>;
}
