import type {
  OrchestrationAction, OrchestrationMutationRequest, OrchestrationMutationResponse,
  OrchestrationSnapshotRequest, OrchestrationSnapshot, OrchestrationWaitRequest, OrchestrationWaitResponse,
  Grant, TaskIntent,
} from "../protocol/generated/v1";
import {
  wireOrchestrationAction, wireOrchestrationMutationRequest, wireOrchestrationMutationResponse,
  wireOrchestrationSnapshotRequest, wireOrchestrationSnapshot, wireOrchestrationWaitRequest, wireOrchestrationWaitResponse,
  type TypedWirePolicy, type WireOwners,
} from "../protocol/generated/validate";
import { CockpitClientError, validateSessionId } from "./CockpitClient";
import { parseWorkspaceSetupRequest } from "./projectProtocol";
import { definePolicy, parseWire, rootMessage } from "./wire";

const encoder = new TextEncoder();
const boundedText = (max: number) => (value: string) => value.length <= max
  && encoder.encode(value).byteLength <= max && !value.includes("\0");
const text = boundedText(16 * 1024), label = boundedText(256), path = boundedText(4096);
const id = (value: string) => value.length > 0 && value.length <= 512
  && encoder.encode(value).byteLength <= 512 && !/[\x00-\x1f\x7f]/.test(value);
const hash = (value: string) => /^[a-f0-9]{64}$/.test(value);
const uuid = (value: string) => /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value);
const timestamp = (value: string) => value.length <= 64
  && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(value) && Number.isFinite(Date.parse(value));
const session = (value: string) => {
  if (!id(value)) return false;
  try { validateSessionId(value); return true; } catch { return false; }
};
const stepTitle = (value: string) => {
  if (!value.trim() || /[\r\n\0]/.test(value)) return false;
  let scalars = 0;
  for (const scalar of value) {
    const code = scalar.codePointAt(0)!;
    if (++scalars > 200 || (code >= 0xd800 && code <= 0xdfff)) return false;
  }
  return true;
};
type StringNames<T> = { [K in keyof T]: T[K] extends string | null ? K : never }[keyof T] & string;
type StringFields = { readonly [O in keyof WireOwners]?: readonly StringNames<WireOwners[O]>[] };
// These groups name semantic refinements only; generated descriptors own field presence and types.
function refine(rule: (value: string) => boolean, owners: StringFields): NonNullable<TypedWirePolicy["fields"]> {
  return Object.fromEntries((Object.entries(owners) as [string, readonly string[]][]).flatMap(([owner, fields]) =>
    fields.map(field => [`${owner}.${field}`, (value: string | null) => value === null || rule(value)])));
}
const provenance = (value: Grant | TaskIntent) => value.origin === "supervisor"
  ? value.supervisor_run_id !== null && value.omp_session_id !== null
  : value.supervisor_run_id === null && value.omp_session_id === null;
const labels = rootMessage("Invalid orchestration", {
  OrchestrationAction: "action", OrchestrationSnapshotRequest: "snapshot request", OrchestrationSnapshot: "snapshot",
  OrchestrationMutationRequest: "mutation request", OrchestrationMutationResponse: "mutation response",
  OrchestrationActionResult: "mutation result", OrchestrationWaitRequest: "wait request", OrchestrationWaitResponse: "wait response",
});
const POLICY = definePolicy({
  message: failure => {
    if (failure.refinement === "assignment_revision") return "Invalid orchestration assignment revision";
    if (failure.refinement === "answer_correlation") return "Invalid orchestration answer correlation";
    if (failure.path.some(frame => frame.type === "OrchestrationActionResult" && frame.variant === undefined
      && (frame.field === "result" || (failure.array && frame.field === undefined))))
      return "Invalid orchestration mutation response";
    return labels(failure);
  },
  wire: {
    complete: new Set(["OrchestrationMutationRequest"]),
    raw: new Set(["OrchestrationAction", "OrchestrationSnapshotRequest", "OrchestrationMutationRequest", "OrchestrationSnapshot",
      "OrchestrationMutationResponse", "OrchestrationWaitRequest", "OrchestrationWaitResponse"]),
    exact: new Set(["OrchestrationAction", "OrchestrationSnapshotRequest", "OrchestrationMutationRequest", "OrchestrationWaitRequest",
      "DispatchTarget", "WorkspaceSetupRequest", "SubagentOp", "NativeProcessIdentity", "NativeShellIdentity", "RetirementIdentity",
      "RetirementState", "RunRetirement", "NativeStopReceipt", "AnswerReceipt", "QuestionReceipt", "QuestionStatus", "TaskAssignmentIntent"]),
    order: {
      OrchestrationMutationRequest: ["session_id", "expected_revision", "action", "keys"],
      OrchestrationMutationResponse: ["revision", "result:shallow", "result"],
    },
    lengths: {
      "OrchestrationAction[task_create].depends_on": { max: 32 }, "OrchestrationAction[task_dependencies_set].depends_on": { max: 32 },
      "WorkspaceSetupRequest[create].linked_artifact_urls": { max: 4 },
    },
    fields: {
      ...refine(id, {
        ErrorResponse: ["code"], "ActorRef[run]": ["run_id"], Report: ["message_id"],
        "DispatchTarget[existing_space]": ["workspace_id"], "DispatchTarget[space_worktree]": ["workspace_id"],
        TaskStep: ["step_id", "parent_step_id"], Task: ["task_id", "follow_up_of", "diagnostic"],
        TaskDependencyBlocker: ["task_id"], TaskBoard: ["root_id"], TaskView: ["current_run_id"],
        SetupSummary: ["operation_id", "workspace_id", "repository_id", "project_workspace_id"],
        RunLocation: ["endpoint_identity", "workspace_id", "tab_id", "pane_id", "launch_tag", "boot_id", "terminal_id", "native_session_id"],
        NativeProcessIdentity: ["kernel_boot_id"], RetirementIdentity: ["launch_tag", "endpoint_identity", "workspace_id", "tab_id", "pane_id", "terminal_id", "herdr_boot_id", "omp_session_id"],
        RunRetirement: ["result_message_id"], Grant: ["grant_id", "supervisor_run_id", "omp_session_id"],
        Run: ["run_id", "root_id", "parent_run_id", "task_id", "bound_omp_session", "supersedes_run_id"], DispatchState: ["launch_tag", "endpoint_identity"],
        Message: ["message_id", "to_run_id", "in_reply_to", "escalated_from", "from_subagent_id", "woken_omp_session"],
        AnswerReceipt: ["message_id"], QuestionStatus: ["run_id", "question_message_id"], Subagent: ["run_id", "subagent_id", "parent_subagent_id", "bound_omp_session"],
        TaskIntent: ["intent_id", "root_id", "task_id", "run_id", "supervisor_run_id", "omp_session_id", "result_message_id"], TaskAssignmentIntent: ["root_id"],
        RunObservation: ["run_id", "workspace_id", "pane_id", "tab_id"], "RuntimeObservation[fresh]": ["endpoint_identity"], Attention: ["run_id", "task_id"],
        RootSummary: ["root_id"], UnmanagedAgent: ["workspace_id", "tab_id", "pane_id"], OrchestrationSnapshotRequest: ["root_id"],
        "OrchestrationAction[task_create]": ["root_id"], "OrchestrationAction[task_assign]": ["root_id"], "OrchestrationAction[task_assignment_resolve]": ["root_id"],
        "OrchestrationAction[task_update]": ["root_id", "task_id"], "OrchestrationAction[task_dependencies_set]": ["root_id", "task_id"],
        "OrchestrationAction[task_step_add]": ["root_id", "task_id"], "OrchestrationAction[task_step_rename]": ["root_id", "task_id"],
        "OrchestrationAction[task_step_set_checked]": ["root_id", "task_id"], "OrchestrationAction[task_step_move]": ["root_id", "task_id"], "OrchestrationAction[task_step_remove]": ["root_id", "task_id"],
        "OrchestrationAction[tasks_assign_ids]": ["root_id"], "OrchestrationAction[run_bind_session]": ["omp_session_id"],
        "OrchestrationAction[run_propose]": ["task_id", "parent_run_id", "supersedes_run_id"], "OrchestrationAction[grant_prepare]": ["run_id"], "OrchestrationAction[grant_execute]": ["run_id"],
        "OrchestrationAction[accept]": ["run_id"], "OrchestrationAction[send_back]": ["run_id"], "OrchestrationAction[cancel_run]": ["run_id"], "OrchestrationAction[retry_launch]": ["run_id"],
        "OrchestrationAction[reconcile_run]": ["run_id"], "OrchestrationAction[intent_resolve]": ["intent_id"], "OrchestrationAction[report]": ["message_id", "to_run_id"],
        "OrchestrationAction[message_send]": ["message_id", "to_run_id", "in_reply_to"], "OrchestrationAction[annotate]": ["run_id"], "OrchestrationAction[inbox_woken]": ["omp_session_id"],
        "OrchestrationAction[subagent_update]": ["subagent_id", "parent_subagent_id"], "OrchestrationAction[subagent_control]": ["run_id", "subagent_id"],
        "OrchestrationActionResult[run]": ["run_id"], "OrchestrationActionResult[task_assigned]": ["to_run_id"], "OrchestrationActionResult[message]": ["to_run_id"],
      }),
      ...refine(text, {
        ErrorResponse: ["message"], Report: ["summary", "plan"], PlanRecord: ["text"], Run: ["prepare_brief"], Annotation: ["text"], Message: ["text"],
        "SubagentOp[send]": ["text"], Subagent: ["summary"], SubagentControlState: ["error"], "RetirementState[unknown]": ["detail"],
        "OrchestrationAction[task_create]": ["description"], "OrchestrationAction[task_assign]": ["description"], "OrchestrationAction[task_update]": ["description"],
        "OrchestrationAction[run_propose]": ["prepare_brief"], "OrchestrationAction[grant_execute]": ["note"], "OrchestrationAction[send_back]": ["text"],
        "OrchestrationAction[report]": ["summary", "plan"], "OrchestrationAction[message_send]": ["text"], "OrchestrationAction[annotate]": ["text"], "OrchestrationAction[subagent_update]": ["summary"], "OrchestrationAction[subagent_control_done]": ["error"],
      }),
      ...refine(label, {
        Run: ["label"], Subagent: ["role", "label"], RootSummary: ["label"], "OrchestrationAction[task_create]": ["title"], "OrchestrationAction[task_assign]": ["title"],
        "OrchestrationAction[task_update]": ["title"], "OrchestrationAction[supervisor_start]": ["label"], "OrchestrationAction[run_adopt]": ["label"], "OrchestrationAction[run_propose]": ["label"], "OrchestrationAction[subagent_update]": ["role", "label"],
      }),
      ...refine(path, { TaskBoard: ["path"], SetupSummary: ["checkout_path", "branch", "base"], "DispatchTarget[space_worktree]": ["branch", "base_ref"] }),
      ...refine(hash, {
        Task: ["task_revision"], TaskBoard: ["doc_revision"], PlanRecord: ["plan_revision"], NativeShellIdentity: ["argv_digest"], RunRetirement: ["task_revision"], Grant: ["plan_revision"], Run: ["task_revision_at_propose"], TaskIntent: ["expected_task_revision"],
        OrchestrationSnapshot: ["tasks_token"], OrchestrationWaitRequest: ["after_tasks_token"], OrchestrationWaitResponse: ["tasks_token"],
        "OrchestrationAction[task_create]": ["expected_doc_revision", "source_revision"], "OrchestrationAction[task_assignment_resolve]": ["expected_task_revision"], "OrchestrationAction[task_update]": ["expected_task_revision"],
        "OrchestrationAction[task_dependencies_set]": ["expected_task_revision", "expected_doc_revision"], "OrchestrationAction[task_step_add]": ["expected_task_revision"], "OrchestrationAction[task_step_rename]": ["expected_task_revision"],
        "OrchestrationAction[task_step_set_checked]": ["expected_task_revision"], "OrchestrationAction[task_step_move]": ["expected_task_revision"], "OrchestrationAction[task_step_remove]": ["expected_task_revision"], "OrchestrationAction[tasks_assign_ids]": ["expected_doc_revision"],
        "OrchestrationAction[grant_prepare]": ["plan_revision"], "OrchestrationAction[grant_execute]": ["plan_revision"], "OrchestrationAction[accept]": ["expected_task_revision"], "OrchestrationActionResult[task_ids]": ["doc_revision"],
      }),
      ...refine(uuid, {
        RunRetirement: ["retirement_id"], TaskAssignmentIntent: ["task_id"], "OrchestrationAction[task_create]": ["task_id", "follow_up_of"], "OrchestrationAction[task_assign]": ["task_id"], "OrchestrationAction[task_assignment_resolve]": ["task_id"],
        "OrchestrationAction[task_step_add]": ["step_id", "parent_step_id", "before_step_id"], "OrchestrationAction[task_step_rename]": ["step_id"], "OrchestrationAction[task_step_set_checked]": ["step_id"],
        "OrchestrationAction[task_step_move]": ["step_id", "parent_step_id", "before_step_id"], "OrchestrationAction[task_step_remove]": ["step_id"], "OrchestrationAction[retirement_native_receipt]": ["retirement_id"],
      }),
      ...refine(timestamp, {
        Report: ["at"], PlanRecord: ["created_at"], RunRetirement: ["created_at", "updated_at"], Grant: ["granted_at"], Run: ["created_at", "updated_at"], DispatchState: ["updated_at"], Annotation: ["at"],
        Message: ["created_at", "acked_at"], AnswerReceipt: ["created_at", "acked_at"], QuestionStatus: ["asked_at"], Subagent: ["updated_at"], SubagentControlState: ["at"],
        RunObservation: ["state_changed_at"], "RuntimeObservation[fresh]": ["observed_at"], Attention: ["since"], UnmanagedAgent: ["state_changed_at"],
        "RetirementState[native_stop_offered]": ["offered_at"], "RetirementState[native_stop_deferred]": ["offered_at", "at"], "RetirementState[native_stop_requested]": ["at"], "RetirementState[native_stopped]": ["at"],
        "RetirementState[close_intent]": ["at"], "RetirementState[retired]": ["at"], "RetirementState[retained]": ["at"], "RetirementState[unknown]": ["at"],
      }),
      ...refine(session, { OrchestrationSnapshotRequest: ["session_id"], OrchestrationMutationRequest: ["session_id"], OrchestrationSnapshot: ["session_id"], Run: ["session_id"], RunLocation: ["session_id"], RetirementIdentity: ["session_id"] }),
      ...refine(stepTitle, { "OrchestrationAction[task_step_add]": ["title"], "OrchestrationAction[task_step_rename]": ["title"] }),
      ...refine(value => /^(?:0|[1-9]\d*)$/.test(value) && (value.length < 20 || (value.length === 20 && value <= "18446744073709551615")), { NativeShellIdentity: ["executable_device", "executable_inode"] }),
      "Task.depends_on": values => values.every(id), "SetupSummary.effects": values => values.every(text), "SetupSummary.warnings": values => values.every(text),
      "OrchestrationAction[task_create].depends_on": values => values.every(uuid), "OrchestrationAction[task_dependencies_set].depends_on": values => values.every(uuid),
      "OrchestrationAction[inbox_pull].limit": value => value <= 100, "OrchestrationWaitRequest.timeout_ms": value => value <= 30_000,
      "NativeStopReceipt[refused].text": boundedText(1024),
      "DispatchTarget[setup].request": value => {
        try {
          const request = parseWorkspaceSetupRequest(value);
          if ((request.label !== null && !label(request.label)) || (request.task_name !== null && !label(request.task_name))) return false;
          return request.operation === "open" ? path(request.path) : id(request.repository_id)
            && [request.branch, request.base_ref, request.checkout_path, request.artifact_url].every(value => value === null || path(value))
            && request.linked_artifact_urls.every(path);
        } catch { return false; }
      },
    },
    checks: {
      TaskStepProgress: { progress: value => value.done <= value.total }, Grant: { provenance },
      Run: { grant_root: value => value.grants.every(grant => grant.origin !== "supervisor" || grant.supervisor_run_id === value.root_id) },
      RunRetirement: { identity: value => value.identity !== null || (value.state.state === "retained" && value.state.reason === "identity_incomplete") },
      "QuestionReceipt[answer_delivered]": { stage: value => value.answer.stage !== "acked" }, "QuestionReceipt[answer_acknowledged]": { stage: value => value.answer.stage === "acked" },
      TaskIntent: { provenance, root: value => value.origin !== "supervisor" || value.supervisor_run_id === value.root_id },
      RunObservation: { presence: value => !value.actual_omp || value.presence === "present" },
      "OrchestrationAction[task_assignment_resolve]": { assignment_revision: value => !value.assign || value.expected_task_revision !== null },
      "OrchestrationAction[message_send]": { answer_correlation: value => value.kind === "answer" ? value.in_reply_to !== null : value.in_reply_to === null },
    },
  } satisfies TypedWirePolicy,
});
function malformed(label: string): never { throw new CockpitClientError("malformed_response", `Invalid orchestration ${label}`); }
export function parseOrchestrationAction(value: unknown): OrchestrationAction { return parseWire(value, wireOrchestrationAction, POLICY); }
export function parseOrchestrationSnapshotRequest(value: unknown): OrchestrationSnapshotRequest { return parseWire(value, wireOrchestrationSnapshotRequest, POLICY); }
export function parseOrchestrationMutationRequest(value: unknown): OrchestrationMutationRequest { return parseWire(value, wireOrchestrationMutationRequest, POLICY); }
export function parseOrchestrationWaitRequest(value: unknown): OrchestrationWaitRequest { return parseWire(value, wireOrchestrationWaitRequest, POLICY); }
export function parseOrchestrationSnapshot(value: unknown): OrchestrationSnapshot {
  const snapshot = parseWire(value, wireOrchestrationSnapshot, POLICY);
  if (snapshot.runs.some(run => run.session_id !== snapshot.session_id
    || (run.location !== null && run.location.session_id !== snapshot.session_id))) malformed("run session identity");
  return snapshot;
}
export function matchOrchestrationSnapshot(value: unknown, request: OrchestrationSnapshotRequest): OrchestrationSnapshot {
  const snapshot = parseOrchestrationSnapshot(value);
  if (snapshot.session_id !== request.session_id || (request.root_id !== null && snapshot.board !== null && snapshot.board.root_id !== request.root_id)) malformed("snapshot identity");
  return snapshot;
}
export function parseOrchestrationMutationResponse(value: unknown): OrchestrationMutationResponse { return parseWire(value, wireOrchestrationMutationResponse, POLICY); }
export function parseOrchestrationWaitResponse(value: unknown): OrchestrationWaitResponse { return parseWire(value, wireOrchestrationWaitResponse, POLICY); }
