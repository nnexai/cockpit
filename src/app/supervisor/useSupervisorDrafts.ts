import { useRef, useState } from "react";
import type { TaskLane } from "../../protocol/generated/v1";
import type { ScrollOffset } from "./reveal";
import type { OrchestrationAction, Task, TaskView } from "../../protocol/generated/v1";
import type { StepDraftState } from "./stepInteractions";
import { newStepDraft } from "./stepInteractions";
import type { MutationRequestScope, RetainedDraftSubmission, SourceSubmission } from "./useSupervisor";
import type { ResumeAction, StartDraft } from "./SupervisorDialogs";

export type SupervisorViewDrafts = {
  mode: "tasks" | "graph" | "dependencies";
  offsets: { tasks: ScrollOffset | null; graph: ScrollOffset | null; dependencies: ScrollOffset | null };
  laneScroll: Partial<Record<TaskLane, number>>;
  collapsedLanes: TaskLane[];
  detailWidth: number | null;
  sheetHeight: number | null;
  queueOpenRow: string | null;
  detailTrail: string[];
};

export type TextDraft = { text: string; operation: { id: string; text: string; action?: OrchestrationAction; requestScope?: MutationRequestScope } | null; notice: string | null; error: string | null };
export type EditDraft = { title: string; description: string; revision: string; baseTask: Task; baseView: TaskView; submitted: SourceSubmission | null; reviewed: Task | null };
export type RelationDraft = { dependsOn: string[]; baseSet: string[]; baseTask: Task; baseView: TaskView; revision: string; docRevision: string; query: string; submitted: SourceSubmission | null; reviewed: Task | null };
export type FollowUpDraft = { taskId: string; title: string; description: string; waitForSource: boolean; baseSource: Task; baseView: TaskView; docRevision: string; submitted: SourceSubmission | null; reviewed: Task | null };
export type ScopeDrafts = {
  messages: Map<string, TextDraft>;
  edits: Map<string, EditDraft>;
  steps: Map<string, StepDraftState>;
  relations: Map<string, RelationDraft>;
  followUps: Map<string, FollowUpDraft>;
  selectedTask: string | null;
  selectedRun: string | null;
  selectedSubagent: string | null;
  showSubagents: boolean;
  attentionOnly: boolean;
  spaceFilter: string;
  disclosures: Record<"completed" | "history" | "diagnostics" | "archive", boolean>;
  view: SupervisorViewDrafts;
};
export function newTextDraft(): TextDraft { return { text: "", operation: null, notice: null, error: null }; }
export function newScopeDrafts(): ScopeDrafts {
  return { messages: new Map(), edits: new Map(), steps: new Map(), relations: new Map(), followUps: new Map(), selectedTask: null, selectedRun: null, selectedSubagent: null, showSubagents: true, attentionOnly: false, spaceFilter: "", disclosures: { completed: false, history: false, diagnostics: false, archive: false }, view: { mode: "tasks", offsets: { tasks: null, graph: null, dependencies: null }, laneScroll: {}, collapsedLanes: [], detailWidth: null, sheetHeight: null, queueOpenRow: null, detailTrail: [] } };
}
export function messageDraft(scope: ScopeDrafts, key: string): TextDraft {
  let draft = scope.messages.get(key);
  if (!draft) { draft = newTextDraft(); scope.messages.set(key, draft); }
  return draft;
}
export function stepDraft(scope: ScopeDrafts, taskId: string): StepDraftState {
  let draft = scope.steps.get(taskId);
  if (!draft) { draft = newStepDraft(); scope.steps.set(taskId, draft); }
  return draft;
}
export type SupervisorLifecycleDrafts = {
  start: StartDraft;
  resumes: Map<string, { action: ResumeAction; requestScope: MutationRequestScope }>;
  startUnknown: boolean;
};
export type SupervisorDraftStore = {
  scopes: Map<string | null, ScopeDrafts>;
  lifecycle: SupervisorLifecycleDrafts;
  storageWarning: string | null;
};

// This is a whitelist of user-owned form state, not a serialization of snapshots,
// clients, dispatch credentials, run records or native bindings.
type Schema = "string" | "number" | "boolean" | { nullable: Schema } | { optional: Schema }
  | { array: Schema } | { map: Schema; nullableKey?: boolean } | { set: Schema }
  | { fields: Record<string, Schema> } | { tag: string; variants: Record<string, Schema> };
const object = (fields: Record<string, Schema>): Schema => ({ fields });
const nullable = (schema: Schema): Schema => ({ nullable: schema });
const optional = (schema: Schema): Schema => ({ optional: schema });
const array = (schema: Schema): Schema => ({ array: schema });
const map = (schema: Schema, nullableKey = false): Schema => ({ map: schema, nullableKey });
const union = (tag: string, variants: Record<string, Schema>): Schema => ({ tag, variants });
const stringOrNull = nullable("string");
const requestScopeSchema = object({ stateStoreId: "string", sessionId: "string", rootId: stringOrNull, expectedRevision: nullable("number") });
const taskSchema = object({
  task_id: "string", title: "string", body: "string", description: "string", description_editable: "boolean",
  description_diagnostic: stringOrNull, steps: array(object({ step_id: stringOrNull, parent_step_id: stringOrNull,
    depth: "number", title: "string", checked: "boolean", status: "string", line: "number", source_offset: "number", diagnostic: stringOrNull })),
  step_progress: nullable(object({ done: "number", total: "number" })), steps_diagnostic: stringOrNull,
  depends_on: array("string"), follow_up_of: stringOrNull, relations_diagnostic: stringOrNull,
  checked: "boolean", line: "number", task_revision: "string", diagnostic: stringOrNull,
});
const taskViewSchema = object({
  task: taskSchema, lane: "string", current_run_id: stringOrNull,
  dependencies: object({ state: "string", unmet: array(object({ task_id: "string", reason: "string" })),
    problems: array(object({ code: "string", message: "string" })) }),
});
const targetSchema = union("target", {
  existing_space: object({ target: "string", workspace_id: "string" }),
  space_worktree: object({ target: "string", workspace_id: "string", branch: stringOrNull, base_ref: stringOrNull }),
  setup: object({ target: "string", request: union("operation", {
    open: object({ operation: "string", path: "string", label: stringOrNull, task_name: stringOrNull, focus: "boolean" }),
    create: object({ operation: "string", repository_id: "string", branch: stringOrNull, base_ref: stringOrNull,
      checkout_path: stringOrNull, label: stringOrNull, task_name: stringOrNull, artifact_url: stringOrNull,
      linked_artifact_urls: array("string"), focus: "boolean" }),
  }) }),
});
const sourceFields = { action: "string", root_id: "string", task_id: "string" } satisfies Record<string, Schema>;
const actionSchema = union("action", {
  task_update: object({ ...sourceFields, expected_task_revision: "string", title: stringOrNull, description: stringOrNull }),
  task_dependencies_set: object({ ...sourceFields, expected_task_revision: "string", expected_doc_revision: "string", depends_on: array("string") }),
  task_create: object({ ...sourceFields, title: "string", description: "string", depends_on: array("string"), follow_up_of: stringOrNull, expected_doc_revision: stringOrNull, source_revision: stringOrNull }),
  supervisor_start: object({ action: "string", operation_id: "string", mode: "string", target: nullable(targetSchema),
    label: stringOrNull, selected_root_id: stringOrNull, expected_project_key: stringOrNull }),
  run_resume: object({ action: "string", run_id: "string", operation_id: "string", expected_attachment_epoch: nullable("number"), mode: "string", decision_id: stringOrNull }),
  task_scope_update: object({ ...sourceFields, operation_id: "string", run_id: "string",
    expected_task_revision: "string", expected_doc_revision: "string", title: stringOrNull, description: stringOrNull }),
  message_send: object({ action: "string", message_id: "string", to_run_id: "string", kind: "string", text: "string", in_reply_to: stringOrNull }),
  annotate: object({ action: "string", run_id: "string", text: "string" }),
  send_back: object({ action: "string", run_id: "string", text: "string" }),
  cleanup_finish: object({ action: "string", request: object({ operation_id: "string", root_id: "string", run_id: stringOrNull }) }),
  subagent_control: object({ action: "string", run_id: "string", subagent_id: "string",
    op: union("op", { send: object({ op: "string", text: "string" }), cancel: object({ op: "string" }) }) }),
});
const scopeSchema = object({ sessionId: "string", rootId: "string", taskId: "string" });
const sourceSubmissionSchema = object({ submissionId: "string", scope: scopeSchema, action: actionSchema });
const intentFields = { kind: "string", stepId: "string" } satisfies Record<string, Schema>;
const stepSubmissionSchema = object({
  submissionId: "string", scope: scopeSchema, expectedTaskRevision: "string",
  intent: union("kind", {
    add: object({ ...intentFields, parentStepId: stringOrNull, beforeStepId: stringOrNull, title: "string" }),
    rename: object({ ...intentFields, title: "string" }),
    set_checked: object({ ...intentFields, checked: "boolean", scope: "string" }),
    move: object({ ...intentFields, parentStepId: stringOrNull, beforeStepId: stringOrNull }),
    remove: object(intentFields),
  }),
});
const stepTextSchema = object({
  text: "string", baseTaskRevision: "string", baseTitle: stringOrNull, baseParentStepId: stringOrNull,
  baseBeforeStepId: stringOrNull, retainedSubmission: nullable(stepSubmissionSchema), open: "boolean",
});
const stepOperationSchema = union("kind", {
  idle: object({ kind: "string" }),
  sending: object({ kind: "string", submitted: stepSubmissionSchema }),
  refused: object({ kind: "string", submitted: stepSubmissionSchema, operationCode: "string", message: "string" }),
  unknown: object({ kind: "string", submitted: stepSubmissionSchema, message: "string" }),
  review: object({ kind: "string", submitted: stepSubmissionSchema, currentTask: taskSchema, reason: "string" }),
});
const originalSource = { submitted: nullable(sourceSubmissionSchema), reviewed: nullable(taskSchema) };
const scopeDraftSchema = object({
  messages: map(object({ text: "string", operation: nullable(object({ id: "string", text: "string",
    action: optional(actionSchema), requestScope: optional(requestScopeSchema) })), notice: stringOrNull, error: stringOrNull })),
  edits: map(object({ title: "string", description: "string", revision: "string", baseTask: taskSchema, baseView: taskViewSchema, ...originalSource })),
  relations: map(object({ dependsOn: array("string"), baseSet: array("string"), baseTask: taskSchema,
    baseView: taskViewSchema, revision: "string", docRevision: "string", query: "string", ...originalSource })),
  followUps: map(object({ taskId: "string", title: "string", description: "string", waitForSource: "boolean",
    baseSource: taskSchema, baseView: taskViewSchema, docRevision: "string", ...originalSource })),
  steps: map(object({ add: map(stepTextSchema, true), rename: map(stepTextSchema),
    expanded: nullable({ set: "string" }), focusedStepId: stringOrNull,
    confirmation: nullable(object({ submitted: stepSubmissionSchema, baseTask: taskSchema })),
    operation: stepOperationSchema, status: nullable(object({ tone: "string", text: "string" })) })),
});
const lifecycleSchema = object({
  start: object({ label: "string", location: "string", spaceId: "string", directory: "string",
    intent: optional("string"), operation: optional(actionSchema), requestScope: optional(requestScopeSchema) }),
  resumes: map(object({ action: actionSchema, requestScope: requestScopeSchema })), startUnknown: "boolean",
});
const storeSchema = object({ scopes: map(scopeDraftSchema, true), lifecycle: lifecycleSchema });

/** Apply the same strict projection on writes and reads; unexpected fields never reach disk. */
function project(value: unknown, schema: Schema, loading: boolean): unknown {
  if (typeof schema === "string") {
    if (typeof value !== schema || schema === "number" && !Number.isFinite(value)) throw new Error("Invalid draft field");
    return value;
  }
  if ("optional" in schema) return value === undefined ? undefined : project(value, schema.optional, loading);
  if ("nullable" in schema) return value === null ? null : project(value, schema.nullable, loading);
  if ("array" in schema || "set" in schema) {
    const values = value instanceof Set && !loading ? [...value] : value;
    if (!Array.isArray(values)) throw new Error("Invalid draft collection");
    const items = values.map(item => project(item, "array" in schema ? schema.array : schema.set, loading));
    return "set" in schema && loading ? new Set(items) : items;
  }
  if ("map" in schema) {
    const entries = value instanceof Map && !loading ? [...value] : value;
    if (!Array.isArray(entries)) throw new Error("Invalid draft map");
    const items = entries.map(entry => {
      if (!Array.isArray(entry) || entry.length !== 2 ||
        !(typeof entry[0] === "string" || schema.nullableKey && entry[0] === null)) throw new Error("Invalid draft key");
      return [entry[0], project(entry[1], schema.map, loading)] as [string | null, unknown];
    });
    return loading ? new Map(items) : items;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Invalid draft object");
  const record = value as Record<string, unknown>;
  if ("tag" in schema) {
    const tag = record[schema.tag];
    if (typeof tag !== "string" || !Object.hasOwn(schema.variants, tag)) throw new Error("Invalid retained action");
    return project(value, schema.variants[tag], loading);
  }
  return Object.fromEntries(Object.entries(schema.fields).flatMap(([key, field]) => {
    const selected = project(record[key], field, loading);
    return selected === undefined ? [] : [[key, selected]];
  }));
}
export function newSupervisorDraftStore(): SupervisorDraftStore {
  return { scopes: new Map(), lifecycle: { start: { label: "", location: "existing", spaceId: "", directory: "" },
    resumes: new Map(), startUnknown: false }, storageWarning: null };
}
export const supervisorDraftStorageKey = (stateStoreId: string) => `cockpit.supervisor.drafts.v1:${stateStoreId}`;
export function serializeSupervisorDrafts(store: SupervisorDraftStore): string {
  return JSON.stringify({ version: 1, ...project(store, storeSchema, false) as object });
}
export function deserializeSupervisorDrafts(raw: string): SupervisorDraftStore {
  const value: unknown = JSON.parse(raw);
  if (!value || typeof value !== "object" || !("version" in value) || value.version !== 1) throw new Error("Unsupported supervisor drafts");
  const stored = project(value, storeSchema, true) as SupervisorDraftStore;
  for (const [key, scope] of stored.scopes) {
    // View/focus/scroll state is never replayed by loading form data.
    stored.scopes.set(key, { ...newScopeDrafts(), ...scope });
    for (const draft of scope.steps.values()) {
      draft.focusRequest = null;
      if (draft.operation.kind === "sending") {
        const message = "The change was interrupted. Read the saved task before another write.";
        draft.operation = { kind: "unknown", submitted: draft.operation.submitted, message };
        draft.status = { tone: "warning", text: message };
      }
    }
  }
  if (stored.lifecycle.start.operation) stored.lifecycle.startUnknown = true;
  stored.storageWarning = null;
  return stored;
}
export function retainedSupervisorSubmissions(store: SupervisorDraftStore): RetainedDraftSubmission[] {
  const retained: RetainedDraftSubmission[] = [];
  for (const scope of store.scopes.values()) {
    for (const drafts of [scope.edits, scope.relations, scope.followUps]) {
      for (const draft of drafts.values()) if (draft.submitted) retained.push({ kind: "source", submitted: draft.submitted });
    }
    for (const draft of scope.steps.values()) {
      if (draft.operation.kind === "unknown" || draft.operation.kind === "sending" ||
        draft.operation.kind === "review" && draft.operation.reason === "unknown") retained.push({ kind: "steps", submitted: draft.operation.submitted });
    }
  }
  return retained;
}
const memory = new Map<string, SupervisorDraftStore>();
const storageFailure = "Drafts are retained for this app session, but browser storage is unavailable. Save before restarting Cockpit.";
export function readSupervisorDraftStore(stateStoreId: string, storage: Pick<Storage, "getItem">): SupervisorDraftStore {
  const cached = memory.get(stateStoreId);
  if (cached) return cached;
  let store = newSupervisorDraftStore();
  try {
    const raw = storage.getItem(supervisorDraftStorageKey(stateStoreId));
    if (raw) store = deserializeSupervisorDrafts(raw);
  } catch { store.storageWarning = storageFailure; }
  memory.set(stateStoreId, store);
  return store;
}
export function persistSupervisorDraftStore(stateStoreId: string, store: SupervisorDraftStore, storage: Pick<Storage, "setItem">): void {
  memory.set(stateStoreId, store);
  try { storage.setItem(supervisorDraftStorageKey(stateStoreId), serializeSupervisorDrafts(store)); store.storageWarning = null; }
  catch { store.storageWarning = storageFailure; }
}
export type SupervisorDraftStorage = {
  scope(rootId: string | null): ScopeDrafts;
  lifecycle: SupervisorLifecycleDrafts;
  storageWarning: string | null;
  retainedSubmissions(): RetainedDraftSubmission[];
  changed(): void;
};
/** Store identity comes only from the authoritative snapshot, never a transport or conversation. */
export function useSupervisorDrafts(stateStoreId: string | null): SupervisorDraftStorage {
  const registry = useRef(new Map<string | null, SupervisorDraftStore>());
  const [, render] = useState(0);
  let store = registry.current.get(stateStoreId);
  if (!store) {
    if (stateStoreId) {
      try { store = readSupervisorDraftStore(stateStoreId, localStorage); }
      catch { store = memory.get(stateStoreId) ?? newSupervisorDraftStore(); store.storageWarning = storageFailure; memory.set(stateStoreId, store); }
    } else store = newSupervisorDraftStore();
    registry.current.set(stateStoreId, store);
  }
  const captured = store;
  return {
    scope: (rootId: string | null) => {
      let scope = captured.scopes.get(rootId);
      if (!scope) { scope = newScopeDrafts(); captured.scopes.set(rootId, scope); }
      return scope;
    },
    lifecycle: captured.lifecycle,
    storageWarning: captured.storageWarning,
    retainedSubmissions: () => retainedSupervisorSubmissions(captured),
    changed: () => {
      if (stateStoreId) {
        try { persistSupervisorDraftStore(stateStoreId, captured, localStorage); }
        catch { captured.storageWarning = storageFailure; }
      }
      render(value => value + 1);
    },
  };
}
