import { useCallback, useEffect, useRef, useState } from "react";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { OrchestrationAction, OrchestrationActionResult, OrchestrationSnapshot, Task } from "../../protocol/generated/v1";
import type { StepSubmission, StepScope, StepReadOutcome, StepUnknownResolution, StepResolutionOutcome } from "./stepInteractions";
import { taskContentReason, uniqueTask } from "./dependencies";

export type TaskMutationOutcome<TSubmission> =
  | { kind: "not_sent"; reason: string }
  | { kind: "refused"; operationCode: string; message: string }
  | { kind: "confirmed"; task: Task }
  | { kind: "unknown"; submitted: TSubmission; message: string };
export type SourceAction = Extract<OrchestrationAction, { action: "task_update" | "task_dependencies_set" | "task_create" }>;
export type SourceSubmission = Readonly<{ submissionId: string; scope: StepScope; action: SourceAction }>;
type Submission = SourceSubmission | StepSubmission;
type UnknownRecord = { kind: "source" | "steps"; payload: string; read: Task | null; readSerial: number; missingRead: boolean; client: CockpitClient };
export type SourceResolution = Readonly<{ originalSubmitted: SourceSubmission; reviewedTaskRevision: string | null; decision: { kind: "use_saved" | "keep_saved" | "retry_same" } | { kind: "apply_reviewed"; submitted: SourceSubmission } }>;
export type SourceResolutionOutcome = { kind: "resolved"; task: Task } | { kind: "not_resolved"; message: string } | { kind: "applied"; outcome: TaskMutationOutcome<SourceSubmission> };

const LOCAL_FENCES: Partial<Record<OrchestrationAction["action"], true>> = {
  task_create: true, task_update: true, task_dependencies_set: true,
  task_step_add: true, task_step_rename: true, task_step_set_checked: true,
  task_step_move: true, task_step_remove: true,
  task_assign: true, task_assignment_resolve: true, tasks_assign_ids: true,
  grant_prepare: true, grant_execute: true, accept: true, message_send: true, annotate: true,
};
// Only codes whose core paths guarantee refusal BEFORE publication belong here.
const PREPUBLICATION_REFUSALS: Partial<Record<string, true>> = {
  actor_forbidden: true, caller_unbound: true, caller_mismatch: true, attempt_stale: true,
  session_mismatch: true, root_not_found: true, run_not_found: true, invalid_stage: true,
  invalid_identity: true, message_too_large: true, task_not_found: true, task_id_duplicate: true,
  task_checked: true, intent_conflict: true, task_revision_conflict: true, task_blocked: true,
  task_dependencies_invalid: true, task_relations_invalid: true, invalid_task: true,
  task_id_conflict: true, task_description_ambiguous: true, task_relationships_live: true,
  task_patch_invalid: true, task_step_patch_invalid: true, tasks_full: true, task_steps_invalid: true,
  task_step_cycle: true, task_step_not_found: true, task_step_unsafe: true,
  task_step_title_invalid: true, task_step_destination_invalid: true, task_step_id_conflict: true,
  task_step_scope_invalid: true, task_step_limit: true,
};
export const taskScopeKey = (scope: StepScope) => JSON.stringify([scope.sessionId, scope.rootId, scope.taskId]);
export function stepAction(submitted: StepSubmission): OrchestrationAction {
  const base = { root_id: submitted.scope.rootId, task_id: submitted.scope.taskId, expected_task_revision: submitted.expectedTaskRevision };
  const intent = submitted.intent;
  switch (intent.kind) {
    case "add": return { action: "task_step_add", ...base, step_id: intent.stepId, parent_step_id: intent.parentStepId, before_step_id: intent.beforeStepId, title: intent.title };
    case "rename": return { action: "task_step_rename", ...base, step_id: intent.stepId, title: intent.title };
    case "set_checked": return { action: "task_step_set_checked", ...base, step_id: intent.stepId, checked: intent.checked, scope: intent.scope };
    case "move": return { action: "task_step_move", ...base, step_id: intent.stepId, parent_step_id: intent.parentStepId, before_step_id: intent.beforeStepId };
    case "remove": return { action: "task_step_remove", ...base, step_id: intent.stepId };
  }
}
export function acceptsSupervisorSnapshot(next: OrchestrationSnapshot, sessionId: string, rootId: string | null, revisionFloor: number): boolean {
  return next.session_id === sessionId && next.revision >= revisionFloor && (!rootId || next.board === null || next.board.root_id === rootId);
}

/** One writer for the mounted workarea; uncertainty belongs to the original task. */
export function useSupervisor(client: CockpitClient, sessionId: string, rootId: string | null, active: boolean) {
  const [snapshot, setSnapshot] = useState<OrchestrationSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [observationError, setObservationError] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const [busy, setBusy] = useState(false);
  const [refreshToken, setRefreshToken] = useState(0);
  const [, uncertaintyChanged] = useState(0);
  const generation = useRef(0), scopeGeneration = useRef(0), floor = useRef(0), pending = useRef(false);
  const identity = useRef({ client, sessionId, rootId, active, connected });
  if (identity.current.client !== client || identity.current.sessionId !== sessionId || identity.current.rootId !== rootId) scopeGeneration.current++;
  identity.current = { client, sessionId, rootId, active, connected };
  const current = useRef<OrchestrationSnapshot | null>(null);
  const unknown = useRef(new Map<string, UnknownRecord>());
  const refresh = useCallback(() => setRefreshToken(value => value + 1), []);
  useEffect(() => {
    floor.current = 0; current.current = null; setSnapshot(null);
    setError(null); setObservationError(null); setConnected(false);
  }, [client, sessionId, rootId]);
  useEffect(() => {
    const scope = ++generation.current;
    let cancelled = false, timer = 0;
    const sleep = () => new Promise<void>(resolve => { timer = window.setTimeout(resolve, 1500); });
    const live = () => !cancelled && generation.current === scope;
    if (!active) { setConnected(false); return () => { cancelled = true; }; }
    const observe = async () => {
      while (live()) {
        try {
          const next = await client.orchestrationSnapshot({ session_id: sessionId, root_id: rootId });
          if (!live()) return;
          if (!acceptsSupervisorSnapshot(next, sessionId, rootId, floor.current)) { await sleep(); continue; }
          floor.current = next.revision; current.current = next; setSnapshot(next); setConnected(true); setObservationError(null);
          const wait = await client.orchestrationWait({ after_revision: next.revision, after_tasks_token: next.tasks_token, timeout_ms: 5000 });
          if (!live()) return;
          floor.current = Math.max(floor.current, wait.revision);
        } catch (failure) {
          if (!live()) return;
          setConnected(false); setObservationError(failure instanceof Error ? failure.message : "Supervisor connection unavailable"); await sleep();
        }
      }
    };
    void observe();
    return () => { cancelled = true; generation.current++; window.clearTimeout(timer); };
  }, [client, sessionId, rootId, active, refreshToken]);
  const mutateResult = useCallback(async (action: OrchestrationAction): Promise<OrchestrationActionResult | null> => {
    const observed = current.current;
    if (!identity.current.active || !observed || pending.current) return null;
    if (action.action === "task_update" || action.action === "task_dependencies_set" || action.action === "task_create" || action.action.startsWith("task_step")) {
      setError("Task source changes require the typed task writer and retained submission identity.");
      return null;
    }
    const captured = identity.current;
    const sameScope = () => captured.client === identity.current.client && captured.sessionId === identity.current.sessionId && captured.rootId === identity.current.rootId;
    pending.current = true; setBusy(true); setError(null);
    try {
      const response = await captured.client.orchestrationMutate({ session_id: captured.sessionId, expected_revision: LOCAL_FENCES[action.action] ? null : observed.revision, action });
      if (!sameScope()) return null;
      floor.current = Math.max(floor.current, response.revision); refresh(); return response.result;
    } catch (failure) {
      if (sameScope()) { setError(failure instanceof Error ? failure.message : "The action was not confirmed. Review current state before retrying."); refresh(); }
      return null;
    } finally { pending.current = false; setBusy(false); }
  }, [refresh]);
  const submit = async <T extends Submission>(submitted: T, kind: UnknownRecord["kind"], action: OrchestrationAction, replacing?: UnknownRecord): Promise<TaskMutationOutcome<T>> => {
    const captured = identity.current, observed = current.current, key = taskScopeKey(submitted.scope);
    const capturedScopeGeneration = scopeGeneration.current;
    if ("root_id" in action && (action.root_id !== submitted.scope.rootId || "task_id" in action && action.task_id !== submitted.scope.taskId)) return { kind: "not_sent", reason: "Submission scope does not match its task payload." };
    if (!captured.active || !observed || observed.session_id !== submitted.scope.sessionId || observed.board?.root_id !== submitted.scope.rootId || captured.sessionId !== submitted.scope.sessionId || captured.rootId !== submitted.scope.rootId) return { kind: "not_sent", reason: "Return to the original task and reconnect before saving." };
    if (pending.current) return { kind: "not_sent", reason: "Wait for the current operation." };
    if (unknown.current.has(key) && unknown.current.get(key) !== replacing) return { kind: "not_sent", reason: "A change to this task is unconfirmed. Read and explicitly resolve it first." };
    if (!captured.connected || observed.runtime.status !== "fresh") return { kind: "not_sent", reason: "Reconnect and check current agent status before saving." };
    const target = uniqueTask(observed.board?.tasks ?? [], submitted.scope.taskId);
    if ("action" in submitted && (submitted.action.root_id !== submitted.scope.rootId || submitted.action.task_id !== submitted.scope.taskId)) return { kind: "not_sent", reason: "The source operation must target its original task and root." };
    if (kind === "steps" && target?.task.step_progress === null) return { kind: "not_sent", reason: "The saved checklist structure is unsafe to edit. Resolve its source diagnostics first." };
    if (kind === "steps" || action.action === "task_update") {
      const reason = taskContentReason(observed, target, true);
      if (reason) return { kind: "not_sent", reason };
    }
    // Own an immutable copy of exactly what was sent; local drafts remain writable.
    const retained = structuredClone(submitted);
    const sentAction = "action" in retained ? retained.action : stepAction(retained);
    pending.current = true; setBusy(true);
    const uncertain = (message: string): TaskMutationOutcome<T> => {
      unknown.current.set(key, { kind, payload: JSON.stringify(retained), read: null, readSerial: 0, missingRead: false, client: captured.client }); uncertaintyChanged(value => value + 1);
      return { kind: "unknown", submitted: retained, message };
    };
    try {
      const response = await captured.client.orchestrationMutate({ session_id: retained.scope.sessionId, expected_revision: null, action: sentAction });
      if (capturedScopeGeneration !== scopeGeneration.current) return uncertain("The scope changed before confirmation. Check the original saved task.");
      if (response.result.result !== "task" || response.result.task.task_id !== retained.scope.taskId) return uncertain("The task response was not confirmed. Check saved state before another write.");
      if (replacing && unknown.current.get(key) === replacing) unknown.current.delete(key);
      floor.current = Math.max(floor.current, response.revision); uncertaintyChanged(value => value + 1); refresh();
      return { kind: "confirmed", task: response.result.task };
    } catch (failure) {
      const message = failure instanceof Error ? failure.message : "The task change was not confirmed.";
      if (capturedScopeGeneration !== scopeGeneration.current) return uncertain("The scope changed before confirmation. Check the original saved task.");
      if (failure instanceof CockpitClientError && failure.operationCode && PREPUBLICATION_REFUSALS[failure.operationCode]) {
        if (replacing && unknown.current.get(key) === replacing) unknown.current.delete(key);
        uncertaintyChanged(value => value + 1); refresh();
        return { kind: "refused", operationCode: failure.operationCode, message };
      }
      return uncertain(message);
    } finally { pending.current = false; setBusy(false); }
  };
  const readSaved = async (scope: StepScope): Promise<StepReadOutcome> => {
    const record = unknown.current.get(taskScopeKey(scope));
    const readSerial = record ? ++record.readSerial : 0;
    if (record) { record.read = null; record.missingRead = false; }
    try {
      const next = await (record?.client ?? client).orchestrationSnapshot({ session_id: scope.sessionId, root_id: scope.rootId });
      if (next.session_id !== scope.sessionId || next.board?.root_id !== scope.rootId) return { kind: "unavailable", message: "The original task document is unavailable." };
      const matches = next.board.tasks.filter(view => view.task.task_id === scope.taskId);
      if (matches.length === 0) {
        if (record && unknown.current.get(taskScopeKey(scope)) === record && record.readSerial === readSerial) record.missingRead = true;
        return { kind: "missing" };
      }
      if (matches.length !== 1) return { kind: "unavailable", message: "The task identity is ambiguous." };
      const task = matches[0].task;
      if (record && unknown.current.get(taskScopeKey(scope)) === record && record.readSerial === readSerial) record.read = task;
      return { kind: "found", task };
    } catch (failure) { return { kind: "unavailable", message: failure instanceof Error ? failure.message : "Saved task could not be read." }; }
  };
  const resolutionCheck = (submitted: Submission, kind: UnknownRecord["kind"], revision: string) => {
    const record = unknown.current.get(taskScopeKey(submitted.scope));
    if (!record || record.kind !== kind || record.payload !== JSON.stringify(submitted)) return { code: "different_source_operation" as const, message: "The retained operation is different. Its uncertainty gate is unchanged." };
    if (pending.current) return { code: "busy" as const, message: "Wait for the current operation. The uncertainty gate is retained." };
    if (!record.read) return { code: "read_required" as const, message: "Read the original saved task before resolving this operation." };
    if (record.read.task_revision !== revision) return { code: "resolution_stale" as const, message: "Read and review the original saved task again." };
    return record;
  };
  const resolveUnknown = async (request: StepUnknownResolution): Promise<StepResolutionOutcome> => {
    const base = { originalScope: request.originalScope, originalSubmissionId: request.originalSubmissionId };
    if (taskScopeKey(request.originalScope) !== taskScopeKey(request.originalSubmitted.scope) || request.originalSubmissionId !== request.originalSubmitted.submissionId) return { kind: "not_resolved", ...base, code: "different_source_operation", message: "Original operation identity does not match." };
    const checked = resolutionCheck(request.originalSubmitted, "steps", request.reviewedTaskRevision);
    if ("code" in checked) return { kind: "not_resolved", ...base, ...checked };
    if (request.decision.kind === "apply_reviewed") {
      if (checked.read!.step_progress === null || checked.read!.checked) return { kind: "not_resolved", ...base, code: "resolution_stale", message: "The reviewed saved checklist is read-only or structurally unsafe. Keep the saved state or resolve its source diagnostics; the original uncertainty gate is retained." };
      const next = request.decision.submitted;
      if (taskScopeKey(next.scope) !== taskScopeKey(request.originalScope) || next.expectedTaskRevision !== request.reviewedTaskRevision) return { kind: "not_resolved", ...base, code: "resolution_stale", message: "Reviewed submission must target the original task and read revision." };
      return { kind: "applied", ...base, submitted: next, outcome: await submit(next, "steps", stepAction(next), checked) };
    }
    unknown.current.delete(taskScopeKey(request.originalScope)); uncertaintyChanged(value => value + 1);
    return { kind: "resolved", ...base, task: checked.read! };
  };
  const resolveSourceUnknown = async (request: SourceResolution): Promise<SourceResolutionOutcome> => {
    if (request.decision.kind === "retry_same") {
      const key = taskScopeKey(request.originalSubmitted.scope), record = unknown.current.get(key);
      if (!record || record.kind !== "source" || record.payload !== JSON.stringify(request.originalSubmitted) || request.originalSubmitted.action.action !== "task_create" || !record.missingRead || pending.current) return { kind: "not_resolved", message: "Read the original task file and confirm the retained creation identity is missing before retrying the exact submission." };
      return { kind: "applied", outcome: await submit(request.originalSubmitted, "source", request.originalSubmitted.action, record) };
    }
    if (!request.reviewedTaskRevision) return { kind: "not_resolved", message: "Read and review the original saved task first." };
    const checked = resolutionCheck(request.originalSubmitted, "source", request.reviewedTaskRevision);
    if ("code" in checked) return { kind: "not_resolved", message: checked.message };
    if (request.decision.kind === "apply_reviewed") {
      const next = request.decision.submitted;
      if (taskScopeKey(next.scope) !== taskScopeKey(request.originalSubmitted.scope) || next.action.action !== request.originalSubmitted.action.action || ("expected_task_revision" in next.action && next.action.expected_task_revision !== request.reviewedTaskRevision)) return { kind: "not_resolved", message: "Reviewed change must target the original operation and saved revision." };
      return { kind: "applied", outcome: await submit(next, "source", next.action, checked) };
    }
    unknown.current.delete(taskScopeKey(request.originalSubmitted.scope)); uncertaintyChanged(value => value + 1);
    return { kind: "resolved", task: checked.read! };
  };
  return { snapshot, error: error ?? observationError, connected, busy, mutateResult, refresh,
    submitStep: (submitted: StepSubmission) => submit(submitted, "steps", stepAction(submitted)),
    submitTask: (submitted: SourceSubmission) => submit(submitted, "source", submitted.action),
    readSaved, resolveUnknown, resolveSourceUnknown,
    taskWriteUnconfirmed: (scope: StepScope) => unknown.current.has(taskScopeKey(scope)),
  };
}
