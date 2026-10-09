import type { Dispatch, SetStateAction, RefObject, FocusEvent, KeyboardEvent } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { OrchestrationSnapshot, Run, SessionSnapshotResponse, TaskView, RootSummary, RunObservation, Subagent, Report, TaskLane } from "../../protocol/generated/v1";
import type { ScopeDrafts, TextDraft } from "./useSupervisorDrafts";
import type { SourceSubmission, SourceResolution, SourceResolutionOutcome, TaskMutationOutcome } from "./useSupervisor";
import type { StepScope, StepSubmission, StepReadOutcome, StepUnknownResolution, StepResolutionOutcome } from "./stepInteractions";
import type { StartDraft, SupervisorDialogState } from "./SupervisorDialogs";
import type { AttentionTier, AttentionModel, SupervisorOwned } from "./attention";
import type { AgentState, Mutation, PathRowView } from "./SupervisorActions";
import type { SupervisorGraphModel, NodeSelection } from "./topology";
import type { SupervisorLayout, PanelKind, PanelPlacement, PanelBounds } from "./useSupervisorLayout";
import type { BoardNavOptions } from "./boardNavigation";
import type { QueueRowView } from "./SupervisorAttention";
import type { StripChip } from "./SupervisorTasks";
import type { ClosedTaskCount } from "./SupervisorActivity";

type Setter<T> = Dispatch<SetStateAction<T>>;
type Setters<T> = { [K in keyof T as `set${Capitalize<K & string>}`]: Setter<T[K]> };
type ViewStateValues = {
  dialog: SupervisorDialogState | null; terminalError: string | null; notice: string | null;
  hoverSpace: string | null; hoverRun: string | null; detailSection: "overview" | "activity" | "actions";
  attentionOpen: boolean; focusTier: AttentionTier | null; focusNotice: string | null;
  inlineQueueCap: number | null; graphViewportHeight: number | null;
};
export type SupervisorViewState = ViewStateValues & Setters<ViewStateValues> & {
  opened: RefObject<boolean>; initialRootSnapshot: RefObject<OrchestrationSnapshot | null>;
};
type StartStateValues = { startPending: boolean; startUnknown: boolean; pendingRestart: string | null };
export type StartAgentFlowState = StartStateValues & Setters<StartStateValues> & {
  startLock: RefObject<boolean>; startDraft: RefObject<StartDraft>; lastSession: RefObject<string>; seenStart: RefObject<number>;
};
export type DetailFocus = {
  detailInvoker: RefObject<HTMLElement | null>; panelInvoker: RefObject<HTMLElement | null>; counterInvoker: RefObject<HTMLElement | null>;
  returnFocusIntent: RefObject<{ invoker: Element | null } | null>;
  rootRef: RefObject<HTMLElement | null>; workareaRef: RefObject<HTMLDivElement | null>; graphRef: RefObject<HTMLDivElement | null>;
  laneRefs: RefObject<Partial<Record<TaskLane, HTMLUListElement | null>>>;
  focusedTask: RefObject<string | null>; focusedGraph: RefObject<string | null>;
  focusWithinDetail: RefObject<boolean>; questionHadFocus: RefObject<boolean>;
  startFocus: RefObject<{ runId: string | null; invoker: Element | null } | null>;
  createdSelection: RefObject<{ sessionId: string; rootId: string; taskId: string; focus: boolean } | null>;
  lastTaskIds: RefObject<string[]>; revealIntent: RefObject<{ focus: boolean } | null>; explicitFocus: RefObject<boolean>;
  previousGraph: RefObject<{ scope: ScopeDrafts; ids: readonly string[] } | null>;
};
type SupervisorService = {
  snapshot: OrchestrationSnapshot | null; error: string | null; connected: boolean; busy: boolean; mutateResult: Mutation; refresh(): void;
  submitStep(submitted: StepSubmission): Promise<TaskMutationOutcome<StepSubmission>>;
  submitTask(submitted: SourceSubmission): Promise<TaskMutationOutcome<SourceSubmission>>;
  readSaved(scope: StepScope): Promise<StepReadOutcome>;
  resolveUnknown(request: StepUnknownResolution): Promise<StepResolutionOutcome>;
  resolveSourceUnknown(request: SourceResolution): Promise<SourceResolutionOutcome>;
  taskWriteUnconfirmed(scope: StepScope): boolean;
};

export type SupervisorViewProps = {
  client: CockpitClient; sessionId: string; session: SessionSnapshotResponse | null; runtimeLive: boolean; active: boolean; startToken: number;
  navigationError: string | null; onClose(): void; onTerminal(run: Run, snapshot: OrchestrationSnapshot): Promise<void>; onModalChange(open: boolean): void;
};
export type SupervisorViewInputs = SupervisorViewProps & SupervisorService & SupervisorViewState & DetailFocus & StartAgentFlowState & {
  rootId: string | null; setRootId: Setter<string | null>; scope: ScopeDrafts; changed(): void;
};
export type SupervisorViewModel = {
  root: Run | null; openRoots: RootSummary[]; closedRoots: RootSummary[]; orphanedWorkers: Run[];
  live: boolean; destination: SessionSnapshotResponse["spaces"][number] | undefined; rootState: AgentState | null;
  tasks: TaskView[]; rootRuns: Run[]; selectedAgent: Run | null; observations: RunObservation[];
  observe(runId: string | null | undefined): RunObservation | undefined;
  selectedTask: TaskView | null; detailRun: Run | null; detailTask: TaskView | null; detailSubagent: Subagent | null;
  hasRetainedDraft(taskId: string): boolean; retainedDraftTaskId: string | null; detailOpen: boolean; stepTaskId: string | null;
  detailTaskScope: StepScope | null; contentReason: string | null; stepReadOnlyReason: string | null;
  sharedSpace: string | null | undefined; mode: "tasks" | "graph" | "dependencies";
  model: SupervisorGraphModel | null; selectedNode: string | null; observedCount: number; rootSpace: string | null | undefined; banner: string | null;
};
export type SupervisorViewPanel = {
  layout: SupervisorLayout; panelKind: PanelKind | null; placement: PanelPlacement | null; bounds: PanelBounds | null; bottomInset: number;
};
export type SupervisorViewNavigation = {
  navOptions: BoardNavOptions; taskIds: string[]; listRef: RefObject<HTMLDivElement | null>;
  listProps: { onFocus(event: FocusEvent<HTMLElement>): void; onBlur(event: FocusEvent<HTMLElement>): void; onKeyDown(event: KeyboardEvent<HTMLElement>): void };
  tabIndexFor(id: string): number; focusRow(id: string | undefined): void;
  rowElement(id: string | null): HTMLElement | undefined; selectedVisibleId(): string | null;
  revealSelection(focus: boolean, coveredOnly?: boolean): void; focusTaskOrStart(): void; saveOffsets(): void;
  closeAttention(): void; closePanel(): void; closeDetail(): void; escapeLayer(target?: HTMLElement): void;
  select(selection: NodeSelection, invoker: HTMLElement | null, toggle: boolean, reveal: boolean, focus?: boolean): void;
  navigateRelation(taskId: string, back?: boolean): void; switchView(next: "tasks" | "graph" | "dependencies", focus?: boolean): void;
  dimFor(tier: AttentionTier | null, runId: string | null): string | null;
  showItem(taskId: string | null, runId: string | null, invoker: HTMLElement): void;
  openPanel(kind: "history" | "diagnostics", invoker: HTMLElement): void;
};
export type SupervisorViewAttention = {
  attention: AttentionModel | null; decide: boolean; rootReceiptLabel: string | null; question: Report | null; answer: TextDraft | null;
};
export type SupervisorViewActions = {
  started(runId: string): void; start(): Promise<void>; restart(run: Run): Promise<void>;
  navigate(run: Run): Promise<void>; check(run: Run): void; edit(task: TaskView): void; editPrerequisites(task: TaskView): void;
  createFollowUp(task: TaskView): void; resumeSourceDraft(taskId: string, kind: "edit" | "relations" | "follow_up"): void;
};
export type SupervisorViewQueue = {
  rows: QueueRowView[]; expandedId: string | null; path: PathRowView[]; tier: AttentionTier | null | undefined;
  owned: SupervisorOwned | null | undefined; stateSentence: string; chips: StripChip[];
};
export type SupervisorViewContext = SupervisorViewInputs & SupervisorViewModel & SupervisorViewPanel & SupervisorViewNavigation & SupervisorViewAttention & SupervisorViewActions;
export type SupervisorViewRenderContext = SupervisorViewContext & SupervisorViewQueue & { archiveCounts: ReadonlyMap<string, ClosedTaskCount> };
