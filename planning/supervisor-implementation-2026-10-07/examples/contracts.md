# Example: shared TypeScript contracts (sketch for PLAN.md §5)

These are illustrative sketches. They are not compiled and are not product code. The exported names and shapes are binding for the slices. Function bodies are omitted or reduced to the essential rule.

## C1 `src/app/supervisor/attention.ts` (S1)

```ts
import type { AttentionKind, OrchestrationSnapshot, Run, TaskView } from "../../protocol/generated/v1";

export type AttentionTier = "decide" | "recover" | "notice";
export const TIER_ORDER: Readonly<Record<AttentionTier, number>> = { decide: 0, recover: 1, notice: 2 };
export const TIER_LABEL: Readonly<Record<AttentionTier, "Decide" | "Recover" | "Notice">> = { decide: "Decide", recover: "Recover", notice: "Notice" };

export type LocalCondition =
  | { kind: "start_unknown" }
  | { kind: "terminal_error"; message: string }
  | { kind: "navigation_error" }
  | { kind: "change_unconfirmed"; message: string }
  | { kind: "notice"; message: string }
  | { kind: "orphaned_worker"; runId: string }
  | { kind: "agent_status"; runId: string; taskId: string | null }   // agentState failure|missing|unknown
  | { kind: "root_failed_report"; runId: string }
  | { kind: "assignment"; taskId: string; state: "pending" | "conflict" }
  | { kind: "unidentified_items"; count: number };

export type AttentionSource =
  | { origin: "core"; kind: AttentionKind; messageSeq: number | null; since: string }
  | { origin: "local"; condition: LocalCondition };

export type AttentionSubject = { kind: "run"; runId: string } | { kind: "task"; taskId: string } | { kind: "workarea" };

export type AttentionItem = {
  id: string;                       // `${tier}:run:${enc(runId)}` | `${tier}:task:${enc(taskId)}` | `${tier}:local:${kind}`
  tier: AttentionTier;
  subject: AttentionSubject;
  runId: string | null;
  taskId: string | null;
  sources: readonly [AttentionSource, ...AttentionSource[]];  // precedence order (PLAN D3)
  since: string | null;             // earliest core since; null for local-only (no invented age)
};

export type SupervisorOwned = { kind: "awaits_prepare" | "awaits_execute" | "to_accept" | "needs_input"; runId: string; taskId: string | null; since: string };

export type AttentionModel = {
  items: readonly AttentionItem[];
  counts: Readonly<Record<AttentionTier, number>>;
  total: number;
  supervisorOwned: readonly SupervisorOwned[];
  tierForRun(runId: string): AttentionTier | null;
  tierForTask(task: TaskView): AttentionTier | null;
  itemsForRun(runId: string): readonly AttentionItem[];
  itemsForTask(task: TaskView): readonly AttentionItem[];
  ownedForRun(runId: string): SupervisorOwned | null;
};

/** Core kinds only; "supervisor" = ordinary state, never a queue item (DESIGN F2, Q1, Q2). */
export function coreTier(kind: AttentionKind, run: Run | undefined): AttentionTier | "supervisor" {
  switch (kind) {
    case "needs_input": return run && run.run_id === run.root_id ? "decide" : "supervisor";
    case "awaits_prepare": case "awaits_execute": case "to_accept": return "supervisor";
    case "runtime_blocked": case "dispatch_unknown": case "exited_without_report": return "recover";
    case "idle_without_report": case "brief_unread": case "plan_changed": case "intent_conflict": return "notice";
  }
}

export declare function deriveAttention(input: { snapshot: OrchestrationSnapshot; rootId: string | null; local: readonly LocalCondition[] }): AttentionModel;
export declare function rootAttentionSummary(snapshot: OrchestrationSnapshot, rootId: string): Record<AttentionTier, number>;
```

## C2 `graphLayout.ts` and `topology.ts` (S2)

```ts
// graphLayout.ts
export const GRAPH_GEOMETRY = { nodeWidth: 240, nodeHeight: 48, columnGap: 32, rowPitch: 56, padding: 8, headerHeight: 28 } as const;
export type GraphNode = { id: string; parentId: string | null; minColumn?: number; order?: readonly (string | number)[]; gapBefore?: number };
export type GraphPosition = { id: string; column: number; row: number; x: number; y: number; width: number; height: number };
export type GraphEdge = { from: string; to: string; path: string };
export type GraphLayout = {
  positions: GraphPosition[]; edges: GraphEdge[]; width: number; height: number; columns: number;
  order: string[];                                   // preorder = reading order
  parentOf: ReadonlyMap<string, string | null>;      // effective, after missing-parent and cycle pruning
};
export declare function graphLayout(nodes: readonly GraphNode[]): GraphLayout;
export declare function edgePath(from: GraphPosition, to: GraphPosition): string; // straight if level, else midpoint cubic

// topology.ts
import type { GlyphShape } from "../sidebar/StateGlyph";
import type { OrchestrationSnapshot, Run, Subagent, TaskView } from "../../protocol/generated/v1";
const enc = encodeURIComponent;
export const nodeId = {
  run: (runId: string) => `run:${enc(runId)}`,
  task: (taskId: string) => `task:${enc(taskId)}`,
  subagent: (runId: string, subagentId: string) => `sub:${enc(runId)}:${enc(subagentId)}`,
};
export type TopologyNodeKind = "supervisor" | "task" | "worker" | "subagent";
export type TopologyEdgeKind = "task" | "assigned" | "delegated" | "subagent" | "unassigned";
export type TopologyNode = {
  id: string; kind: TopologyNodeKind; parentId: string | null; edge: TopologyEdgeKind | null; minColumn: number;
  run: Run | null; task: TaskView | null; subagent: Subagent | null;
  assignedRunId: string | null;   // task: open current run in this root
  linkedTaskId: string | null;    // worker: open task whose current run this is
  unassigned: boolean;            // task without an open worker (dotted)
};
export type TopologyLink = { from: string; to: string; kind: "assigned" }; // task → nested worker (non-tree)
export type SupervisorGraphModel = {
  nodes: readonly TopologyNode[];                    // reading order (= layout.order)
  byId: ReadonlyMap<string, TopologyNode>;
  children: ReadonlyMap<string, readonly string[]>;  // from effective parentOf, ordered
  links: readonly TopologyLink[];
  layout: GraphLayout;
  hiddenCompletedTasks: number;
  counts: { supervisors: number; tasks: number; workers: number; subagents: number };
};
export declare function buildSupervisorGraph(input: { snapshot: OrchestrationSnapshot; root: Run | null; rootId: string; tasks: readonly TaskView[]; includeSubagents: boolean }): SupervisorGraphModel;
export declare function firstChild(model: SupervisorGraphModel, id: string): string | null;
export declare function chainIds(model: SupervisorGraphModel, id: string): ReadonlySet<string>;
export declare function pathNodes(model: SupervisorGraphModel, id: string, includeChildren: boolean): readonly TopologyNode[];
export type NodeSelection = { task: string } | { run: string; subagent: string | null };
export declare function selectionNodeId(s: { selectedTask: string | null; selectedRun: string | null; selectedSubagent: string | null }): string | null;
export declare function nodeSelection(node: TopologyNode): NodeSelection;
export type NodeFacts = { glyph: GlyphShape | "document"; title: string; role: string; status: string; provenance: string; relation: string | null };
export declare function nodeFacts(node: TopologyNode, ctx: { model: SupervisorGraphModel; snapshot: OrchestrationSnapshot; live: boolean; connected: boolean; runtimeLive: boolean }): NodeFacts;
```

## C3 `SupervisorGraph.tsx` props (S6)

```ts
export type SupervisorGraphProps = {
  model: SupervisorGraphModel;
  snapshot: OrchestrationSnapshot; live: boolean; connected: boolean; runtimeLive: boolean;
  selectedNodeId: string | null;
  highlightedRunId: string | null;
  tierFor(node: TopologyNode): AttentionTier | null;
  dimFor(node: TopologyNode): string | null;        // dim reason or null
  showSubagents: boolean; onShowSubagents(next: boolean): void;
  sharedSpace: string | null | undefined;
  bottomInset: number;                               // sheet spacer px
  scrollRef: React.RefObject<HTMLDivElement | null>;
  onSelect(node: TopologyNode): void;
  onHover(runId: string | null): void;
  onEscape(): boolean;
};
// Deliberately no terminal callback: the graph cannot change Herdr focus (G14).
```

## C4, C5, C6 (S3)

```ts
// PanelSplitter.tsx
export type PanelSplitterProps = {
  label: string; controls: string; orientation: "vertical" | "horizontal";
  value: number; min: number; max: number; grow: 1 | -1;
  onChange(px: number): void; onReset(): void; className?: string;
};
// renders: role=separator aria-orientation aria-label aria-controls aria-valuemin/max/now aria-valuetext=`${value} px` tabIndex=0

// useSupervisorLayout.ts
export type SupervisorLayout = { measured: boolean; width: number; height: number; narrow: boolean; short: boolean; compact: boolean; queueMode: "inline" | "overlay" };
export type PanelKind = "details" | "activity" | "diagnostics" | "attention";
export type PanelPlacement = "side" | "sheet" | "overlay";
export type PanelBounds = { orientation: "vertical" | "horizontal"; min: number; max: number; value: number; defaultValue: number };
export declare function useSupervisorLayout(ref: React.RefObject<HTMLElement | null>): SupervisorLayout;
export function panelPlacement(l: SupervisorLayout, view: "tasks" | "graph", panel: PanelKind): PanelPlacement {
  if (panel === "attention") return "overlay";
  if (!l.narrow) return "side";
  return panel === "details" && view === "graph" && l.height >= 560 ? "sheet" : "overlay";
}
export function panelBounds(l: SupervisorLayout, p: PanelPlacement, saved: { detailWidth: number | null; sheetHeight: number | null }): PanelBounds | null {
  const clamp = (v: number, min: number, max: number) => Math.round(Math.min(max, Math.max(min, v)));
  if (p === "side") { const max = Math.floor(l.width * 0.5); return { orientation: "vertical", min: 280, max, defaultValue: 340, value: clamp(saved.detailWidth ?? 340, 280, max) }; }
  if (p === "sheet") { const max = Math.floor(l.height * 0.75), def = Math.round(l.height * 0.5); return { orientation: "horizontal", min: 160, max, defaultValue: def, value: clamp(saved.sheetHeight ?? def, 160, max) }; }
  return null;
}
export const queueCap = (l: SupervisorLayout, view: "tasks" | "graph") => Math.floor(l.height * (view === "graph" ? 0.3 : 0.4));

// reveal.ts
export type Insets = { top: number; right: number; bottom: number; left: number };
export type ScrollOffset = { left: number; top: number };
export declare function nearestScroll(viewport: DOMRectReadOnly | { left: number; top: number; width: number; height: number }, target: { left: number; top: number; width: number; height: number }, insets: Insets, current: ScrollOffset): ScrollOffset;
export declare function revealNearest(scroller: HTMLElement, element: HTMLElement, insets?: Partial<Insets>): boolean; // writes only scroller.scrollLeft/Top
export declare function isCovered(scroller: HTMLElement, element: HTMLElement, insets?: Partial<Insets>): boolean;
export const readOffset = (el: HTMLElement): ScrollOffset => ({ left: el.scrollLeft, top: el.scrollTop });
export const writeOffset = (el: HTMLElement, o: ScrollOffset) => { el.scrollLeft = o.left; el.scrollTop = o.top; };
```

## C7 `boardNavigation.ts` (S4)

```ts
export type BoardArrangement = "lanes" | "stacked";
export type BoardNavOptions = { arrangement: BoardArrangement; completedOpen: boolean; collapsedLanes: readonly TaskLane[] };
export declare function visibleTaskIds(tasks: readonly TaskView[], options: BoardNavOptions): string[];
export declare function taskNeighbor(tasks: readonly TaskView[], id: string, key: string, options: BoardNavOptions): string | undefined;
```

## C8–C12 presentational view-models

```ts
// SupervisorActions.tsx (S5): additive props on SupervisorActions
export type StateBlockView = { sentence: string; tierLabel: "Decide" | "Recover" | "Notice" | null; waitingSince: string | null };
export type PathRowView = { key: string; role: string; label: string; facts: string[]; current: boolean; depth: number; subagent: boolean; onActivate(): void };
export type CrossView = { label: "Show in Graph" | "Show in Tasks"; onActivate(): void } | null;
export declare function canRestart(state: AgentState, flags: { busy: boolean; connected: boolean; runtimeLive: boolean }, run: Run): boolean;

// SupervisorAttention.tsx (S9)
export type QueueAction = { key: string; label: string; onActivate(invoker: HTMLElement): void; disabled?: boolean; reason?: string; primary?: boolean; consequence?: string };
export type QueueRowView = { id: string; tier: AttentionTier; title: string; since: string | null; body?: React.ReactNode; actions: QueueAction[]; showIn: QueueAction | null };

// SupervisorActivity.tsx (S10)
export type ActivityLink = { kind: "task"; taskId: string } | { kind: "run"; runId: string };
export type ActivityRow = { id: string; day: string; actor: string; what: string; at: string; stale: boolean; link: ActivityLink | null; detail: string };

// SupervisorTasks.tsx (S11)
export type StripChip = { runId: string; label: string; glyph: GlyphShape; status: string; tier: AttentionTier | null; selected: boolean };
```

## C13 `ScopeDrafts.view` (S8)

```ts
export type SupervisorViewDrafts = {
  mode: "tasks" | "graph";
  offsets: { tasks: ScrollOffset | null; graph: ScrollOffset | null };
  laneScroll: Partial<Record<TaskLane, number>>;
  collapsedLanes: TaskLane[];
  detailWidth: number | null;
  sheetHeight: number | null;
  queueOpenRow: string | null;
};
// newScopeDrafts(): view = { mode: "tasks", offsets: { tasks: null, graph: null }, laneScroll: {}, collapsedLanes: [], detailWidth: null, sheetHeight: null, queueOpenRow: null }
// disclosures: Record<"completed" | "history" | "diagnostics" | "archive", boolean>   // "agents" removed
```
