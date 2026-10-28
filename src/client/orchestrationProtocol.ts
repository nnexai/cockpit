import type {
  OrchestrationAction, OrchestrationMutationRequest, OrchestrationMutationResponse,
  OrchestrationSnapshotRequest, OrchestrationSnapshot, OrchestrationWaitRequest, OrchestrationWaitResponse,
  LiaisonLookupRequest, LiaisonLookupResponse, CheckOutcome, Integration, SourceOutput, CleanupItem, OwnedResourceIdentity,
} from "../protocol/generated/v1";
import {
  wireOrchestrationAction, wireOrchestrationMutationRequest, wireOrchestrationMutationResponse,
  wireOrchestrationSnapshotRequest, wireOrchestrationSnapshot, wireOrchestrationWaitRequest, wireOrchestrationWaitResponse,
  wireLiaisonLookupRequest, wireLiaisonLookupResponse, type TypedWirePolicy, type WireOwners,
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
const absolutePath = (value: string) => path(value) && value.startsWith("/")
  && !/[\x00-\x1f\x7f]/.test(value) && !value.split("/").some(part => part === "." || part === "..");
const decimalU64 = (value: string) => /^(?:0|[1-9]\d*)$/.test(value)
  && (value.length < 20 || (value.length === 20 && value <= "18446744073709551615"));
const epoch = (value: number | null) => value === null || (Number.isSafeInteger(value) && value > 0);
const hash = (value: string) => /^[a-f0-9]{64}$/.test(value);
const gitObject = (value: string) => /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(value);
const nonemptyText = (max: number) => (value: string) => boundedText(max)(value) && value.trim().length > 0;
const launchText = (value: string) => nonemptyText(4096)(value) && !/[\x00-\x1f\x7f]/.test(value);
const relativePath = (value: string) => path(value) && value.length > 0 && !value.startsWith("/")
  && !value.split("/").some(part => part === "" || part === "." || part === "..");
const sameStrings = (left: readonly string[], right: readonly string[]) =>
  left.length === right.length && left.every((value, index) => value === right[index]);
const validChecks = (checks: readonly CheckOutcome[]) =>
  new Set(checks.map(check => check.contract)).size === checks.length;
const sameSource = (left: SourceOutput, right: SourceOutput) => {
  if (left.kind !== right.kind || left.result_message_id !== right.result_message_id
    || left.task_revision !== right.task_revision || left.focused_checks.length !== right.focused_checks.length
    || !left.focused_checks.every((check, index) => {
      const other = right.focused_checks[index]!;
      return check.contract === other.contract && check.command_or_scenario === other.command_or_scenario
        && check.outcome === other.outcome && check.evidence === other.evidence && check.limitation === other.limitation;
    })) return false;
  return left.kind === "code" && right.kind === "code"
    ? left.repository_key === right.repository_key && left.checkout_key === right.checkout_key
      && left.base_commit === right.base_commit && left.head_commit === right.head_commit && sameStrings(left.commits, right.commits)
    : left.kind === "non_code" && right.kind === "non_code" && sameStrings(left.artifacts, right.artifacts);
};
const integratedChecks = (integration: Integration) => integration.state !== "integrated"
  || (integration.checks.length > 0 && integration.checks.every(check => check.outcome === "passed" && !check.limitation?.trim())
    && (integration.method !== "recorded_adaptation" || integration.source.focused_checks.length > 0)
    && integration.source.focused_checks.every(source => integration.checks.some(check => check.contract === source.contract)));
function cleanupCounts(value: {
  items: readonly { cleanup: CleanupItem }[]; removed_count: number; waiting_count: number; kept_count: number; unknown_count: number;
}): boolean {
  let removed = 0, waiting = 0, kept = 0, unknown = 0;
  for (const item of value.items) {
    switch (item.cleanup.state) {
      case "removed": removed++; break;
      case "kept": kept++; break;
      case "unknown": unknown++; break;
      default: waiting++; break;
    }
  }
  return value.removed_count === removed && value.waiting_count === waiting && value.kept_count === kept && value.unknown_count === unknown;
}
function completeGitIdentity(resource: OwnedResourceIdentity): boolean {
  if (resource.kind !== "worktree" && resource.kind !== "branch") return true;
  const pin = resource.identity;
  return !!(pin.machine_id && pin.repository_key && pin.repository_root && pin.repository_device && pin.repository_inode
    && pin.common_dir && pin.common_device && pin.common_inode && pin.checkout_device && pin.checkout_inode
    && pin.checkout_key && pin.creation_tip);
}
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
type StringNames<T> = { [K in keyof T]-?: Exclude<T[K], undefined> extends string | null ? K : never }[keyof T] & string;
type StringFields = { readonly [O in keyof WireOwners]?: readonly StringNames<WireOwners[O]>[] };
// These groups name semantic refinements only; generated descriptors own field presence and types.
function refine(rule: (value: string) => boolean, owners: StringFields): NonNullable<TypedWirePolicy["fields"]> {
  return Object.fromEntries((Object.entries(owners) as [string, readonly string[]][]).flatMap(([owner, fields]) =>
    fields.map(field => [`${owner}.${field}`, (value: string | null) => value === null || rule(value)])));
}
const labels = rootMessage("Invalid orchestration", {
  OrchestrationAction: "action", OrchestrationSnapshotRequest: "snapshot request", OrchestrationSnapshot: "snapshot",
  OrchestrationMutationRequest: "mutation request", OrchestrationMutationResponse: "mutation response",
  OrchestrationActionResult: "mutation result", OrchestrationWaitRequest: "wait request", OrchestrationWaitResponse: "wait response",
  LiaisonLookupRequest: "liaison lookup request", LiaisonLookupResponse: "liaison lookup response",
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
    complete: new Set(["OrchestrationMutationRequest",
      "Run", "NativeProcessIdentity", "NativeShellIdentity", "ReplacementRetirementAuthority",
      "ConversationIdentity", "TaskSchedule", "OwnedProcessScope", "AttemptBadge", "RunResumeResult", "RunAttachResult",
      "ProjectIdentity", "ProjectSource", "CheckoutIdentity", "DirectoryIdentity", "CheckoutGitIdentity",
      "ResumeRecord", "ResumeCause", "SupervisorStartResult",
      "WorkerStartResult", "TaskScopeUpdateResult",
      "AgentLaunchReceipt",
      "Report", "SetupSummary", "AttachmentReference", "CheckOutcome", "IntegrationMapping", "IntegrationRecord", "IntegrationRecordResult",
      "SourceOutput[code]", "SourceOutput[non_code]", "Integration[pending]", "Integration[not_applicable]", "Integration[integrated]",
      "ResourceSourceIdentity", "ResourceRequest", "ResourceRequestRecord", "ResourceRelease", "ResourceBuildRegistration",
      "OrchestrationAction[integration_record]", "OrchestrationAction[resource_request]",
      "OrchestrationAction[resource_release]", "OrchestrationAction[resource_build_register]",
      "OrchestrationActionResult[integration_recorded]", "OrchestrationActionResult[resource]",
      "OwnedGitIdentity", "OwnedSpaceIdentity", "OwnedFilesystemIdentity", "OwnedFixtureIdentity",
      "OwnedResourceIdentity[worker_terminal]", "OwnedResourceIdentity[worktree]", "OwnedResourceIdentity[branch]",
      "OwnedResourceIdentity[space]", "OwnedResourceIdentity[scratch]", "OwnedResourceIdentity[fixture_set]",
      "OwnedAttemptDirectoryIdentity", "OwnedResourceIdentity[attempt_directory]",
      "CleanupReason", "CleanupEffectReceipt", "CleanupItem", "OwnedResourceRecord", "ResourceRegister",
      "CleanupFinishResult", "CleanupResourceSummary", "RunCleanupSummary", "RestorationSummary",
      "OrchestrationAction[resource_register]", "OrchestrationAction[cleanup_finish]",
      "OrchestrationActionResult[resource_registered]", "OrchestrationActionResult[cleanup_finished]",
      "LiaisonLookupRequest", "LiaisonLookupResponse", "LiaisonCandidate",
      "OrchestrationAction[supervisor_start]", "OrchestrationAction[run_attach]",
      "OrchestrationAction[worker_start]", "OrchestrationAction[task_scope_update]", "OrchestrationAction[task_schedule]", "OrchestrationAction[task_schedule_cancel]", "OrchestrationAction[worker_replace]",
      "OrchestrationAction[run_register]", "OrchestrationAction[task_unassign]",
      "OrchestrationActionResult[supervisor_started]", "OrchestrationActionResult[worker_started]", "OrchestrationActionResult[task_scope_updated]",
      "OrchestrationActionResult[task_scheduled]", "OrchestrationActionResult[worker_replaced]",
      "OrchestrationActionResult[run_resumed]", "OrchestrationActionResult[run_attached]", "OrchestrationActionResult[task_unassigned]"]),
    // run_resume uses generated required fields plus exact keys: complete also requires
    // own optional fields, which would turn its explicit stop confirmation into a requirement.
    raw: new Set(["OrchestrationSnapshotRequest", "OrchestrationWaitRequest", "OrchestrationWaitResponse",
      "LiaisonLookupRequest", "LiaisonLookupResponse"]),
    absent: new Set(["TaskOwner.conversation", "TaskOwnership.schedule", "ExecutionAttachment.process_scope", "DispatchState.scope_unit",
      "DispatchState.latch", "DispatchState.latch_state", "DispatchState.source_location",
      "DispatchState.move_operation_id", "DispatchState.staging_workspace_id",
      "DispatchState.pre_move_scope",
      "RunLocation.peer_process", "RetirementIdentity.herdr_peer",
      "OrchestrationActionResult[run_registered].conversation",
      "RunRetirement.result_message_id", "RunRetirement.task_revision", "RunRetirement.replacement",
      "OrchestrationConfiguration.initial_prompt_args", "OrchestrationConfiguration.hook_args",
      "OrchestrationConfiguration.max_parallel", "OrchestrationConfiguration.process_scope"]),
    exact: new Set(["OrchestrationAction", "OrchestrationSnapshotRequest", "OrchestrationMutationRequest", "OrchestrationWaitRequest",
      "OrchestrationSnapshot", "OrchestrationMutationResponse", "OrchestrationActionResult", "OrchestrationWaitResponse",
      "DispatchTarget", "WorkspaceSetupRequest", "NativeProcessIdentity", "NativeShellIdentity", "RetirementIdentity",
      "RetirementState", "RunRetirement", "AnswerReceipt", "QuestionReceipt", "QuestionStatus", "TaskAssignmentIntent",
      "Run", "RunLocation", "ProjectIdentity", "ProjectSource", "CheckoutIdentity", "DirectoryIdentity", "CheckoutGitIdentity",
      "ExecutionAttachment", "ConversationIdentity", "TaskOwnership", "TaskOwner", "OrchestrationConfiguration", "ResumeRecord", "ResumeCause", "SupervisorStartResult", "RunResumeResult", "RunAttachResult",
      "WorkerStartResult", "TaskScopeUpdateResult", "TaskSchedule", "ReplacementPlacement", "AttemptBadge", "OwnedProcessScope",
      "AgentLaunchReceipt",
      "ReplacementRetirementAuthority",
      "AttachmentReference", "CheckOutcome", "SourceOutput", "Integration", "IntegrationMapping", "IntegrationRecord", "IntegrationRecordResult",
      "ResourceSourceIdentity", "ResourceRequest", "ResourceRequestRecord", "ResourceRelease", "ResourceBuildRegistration",
      "OwnedGitIdentity", "OwnedSpaceIdentity", "OwnedFilesystemIdentity", "OwnedFixtureIdentity", "OwnedResourceIdentity",
      "OwnedAttemptDirectoryIdentity",
      "CleanupReason", "CleanupEffectReceipt", "CleanupItem", "OwnedResourceRecord", "ResourceRegister", "CleanupFinish",
      "CleanupFinishResult", "CleanupResourceSummary", "RunCleanupSummary", "RestorationSummary",
      "LiaisonLookupRequest", "LiaisonLookupResponse", "LiaisonCandidate",
      "ErrorResponse", "RootSummary", "TaskBoard", "TaskProject", "TaskView", "Task", "TaskStep", "TaskStepProgress", "TaskDependencies",
      "TaskDependencyBlocker", "SetupSummary", "DispatchState", "Annotation", "ActorRef", "Report",
      "Message", "TaskIntent", "RunObservation", "RuntimeObservation", "Attention", "UnmanagedAgent"]),
    order: {
      OrchestrationMutationRequest: ["session_id", "expected_revision", "action", "keys"],
      OrchestrationMutationResponse: ["revision", "result:shallow", "result", "keys"],
    },
    lengths: {
      "OrchestrationAction[task_create].depends_on": { max: 32 }, "OrchestrationAction[task_dependencies_set].depends_on": { max: 32 },
      "WorkspaceSetupRequest[create].linked_artifact_urls": { max: 4 },
      "SourceOutput[code].focused_checks": { max: 128 }, "SourceOutput[non_code].focused_checks": { max: 128 },
      "SourceOutput[non_code].artifacts": { max: 256 }, "Integration[integrated].checks": { max: 128 },
      "Integration[integrated].mappings": { max: 256 }, "IntegrationRecordResult.checks": { max: 128 },
      "OrchestrationConfiguration.extra_args": { max: 16 }, "OrchestrationConfiguration.resume_args": { max: 16 },
      "OrchestrationConfiguration.routes": { max: 256 },
      "OrchestrationConfiguration.initial_prompt_args": { max: 16 }, "OrchestrationConfiguration.hook_args": { max: 16 },
    },
    fields: {
      ...refine(id, {
        ErrorResponse: ["code"], "ActorRef[run]": ["run_id"], Report: ["message_id"],
        "DispatchTarget[existing_space]": ["workspace_id"], "DispatchTarget[space_worktree]": ["workspace_id"],
        "DispatchTarget[owned_attempt_directory]": ["source_workspace_id"],
        TaskStep: ["step_id", "parent_step_id"], Task: ["task_id", "follow_up_of", "diagnostic"],
        TaskDependencyBlocker: ["task_id"], TaskBoard: ["root_id"], TaskView: ["current_run_id"],
        SetupSummary: ["operation_id", "workspace_id", "repository_id", "project_workspace_id"],
        RunLocation: ["endpoint_identity", "workspace_id", "tab_id", "pane_id", "launch_tag", "boot_id", "terminal_id", "native_session_id"],
        NativeProcessIdentity: ["kernel_boot_id"], RetirementIdentity: ["launch_tag", "endpoint_identity", "workspace_id", "tab_id", "pane_id", "terminal_id", "herdr_boot_id"],
        RunRetirement: ["result_message_id"],
        ReplacementRetirementAuthority: ["previous_run_id", "replacement_run_id"],
        Run: ["run_id", "root_id", "parent_run_id", "task_id"], DispatchState: ["launch_tag", "endpoint_identity", "scope_unit", "staging_workspace_id"],
        ProjectIdentity: ["key", "repository_id", "source_workspace_id"], CheckoutIdentity: ["key"],
        ExecutionAttachment: ["machine_id"], ConversationIdentity: ["harness", "session_id"], TaskOwnership: ["root_id", "task_id"], TaskOwner: ["run_id"], "OrchestrationActionResult[run_registered]": ["run_id"], "OrchestrationActionResult[task_unassigned]": ["task_id", "previous_run_id"], ResumeRecord: ["run_id"], ResumeCause: ["code"],
        "LiaisonLookupResponse[unavailable]": ["code"],
        SupervisorStartResult: ["root_id"], RunResumeResult: ["run_id"], RunAttachResult: ["run_id"],
        TaskScopeUpdateResult: ["instruction_message_id"],
        TaskSchedule: ["principal_key"], AttemptBadge: ["run_id"], OwnedProcessScope: ["unit", "invocation_id", "machine_id", "kernel_boot_id"],
        AgentLaunchReceipt: ["nonce"],
        "OrchestrationAction[worker_replace]": ["expected_run_id"],
        "OrchestrationActionResult[worker_replaced]": ["previous_run_id"],
        AttachmentReference: ["run_id"], "SourceOutput[code]": ["repository_key", "checkout_key", "result_message_id"],
        "SourceOutput[non_code]": ["result_message_id"],
        "Integration[not_applicable]": ["result_message_id"],
        "Integration[integrated]": ["accepted_result_message_id", "target_repository_key", "integrator_run_id"],
        IntegrationRecord: ["run_id", "expected_result_message_id"], IntegrationRecordResult: ["run_id"],
        ResourceRequest: ["resource_key"], ResourceRequestRecord: ["resource_key", "machine_id"],
        ResourceSourceIdentity: ["checkout_key"],
        OwnedSpaceIdentity: ["endpoint_identity", "workspace_id"],
        OwnedFilesystemIdentity: ["machine_id"], CleanupReason: ["code"],
        OwnedResourceRecord: ["root_id", "run_id", "creation_operation_id", "integration_result_id"],
        ResourceRegister: ["run_id"], CleanupFinish: ["root_id", "run_id"], RunCleanupSummary: ["run_id"],
        "OrchestrationAction[supervisor_start]": ["selected_root_id"],
        "OrchestrationAction[run_resume]": ["run_id"], "OrchestrationAction[task_unassign]": ["root_id", "task_id"],
        Message: ["message_id", "to_run_id", "in_reply_to", "escalated_from"],
        AnswerReceipt: ["message_id"], QuestionStatus: ["run_id", "question_message_id"],
        TaskIntent: ["intent_id", "root_id", "task_id", "run_id", "supervisor_run_id", "result_message_id"], TaskAssignmentIntent: ["root_id"],
        RunObservation: ["run_id", "workspace_id", "pane_id", "tab_id"], "RuntimeObservation[fresh]": ["endpoint_identity"], Attention: ["run_id", "task_id"],
        RootSummary: ["root_id"], UnmanagedAgent: ["workspace_id", "tab_id", "pane_id"],
        "OrchestrationAction[task_create]": ["root_id"], "OrchestrationAction[task_assign]": ["root_id"], "OrchestrationAction[task_assignment_resolve]": ["root_id"],
        "OrchestrationAction[task_update]": ["root_id", "task_id"], "OrchestrationAction[task_dependencies_set]": ["root_id", "task_id"],
        "OrchestrationAction[task_step_add]": ["root_id", "task_id"], "OrchestrationAction[task_step_rename]": ["root_id", "task_id"],
        "OrchestrationAction[task_step_set_checked]": ["root_id", "task_id"], "OrchestrationAction[task_step_move]": ["root_id", "task_id"], "OrchestrationAction[task_step_remove]": ["root_id", "task_id"],
        "OrchestrationAction[tasks_assign_ids]": ["root_id"],
        "OrchestrationAction[accept]": ["run_id"], "OrchestrationAction[send_back]": ["run_id"], "OrchestrationAction[cancel_run]": ["run_id"],
        "OrchestrationAction[reconcile_run]": ["run_id"], "OrchestrationAction[intent_resolve]": ["intent_id"], "OrchestrationAction[report]": ["message_id", "to_run_id"],
        "OrchestrationAction[message_send]": ["message_id", "to_run_id", "in_reply_to"], "OrchestrationAction[annotate]": ["run_id"],
        "OrchestrationActionResult[run]": ["run_id"], "OrchestrationActionResult[task_assigned]": ["to_run_id"], "OrchestrationActionResult[message]": ["to_run_id"],
      }),
      ...refine(value => id(value) && value.trim().length > 0, {
        OrchestrationSnapshotRequest: ["root_id"], OrchestrationWaitRequest: ["root_id"],
      }),
      ...refine(text, {
        ErrorResponse: ["message"], ResumeCause: ["message"], "LiaisonLookupResponse[unavailable]": ["message"],
        Report: ["summary", "plan"], Run: ["brief"], Annotation: ["text"], Message: ["text"],
        "RetirementState[unknown]": ["detail"],
        "OrchestrationAction[task_create]": ["description"], "OrchestrationAction[task_assign]": ["description"], "OrchestrationAction[task_update]": ["description"],
        "OrchestrationAction[worker_start]": ["brief"], "OrchestrationAction[task_scope_update]": ["description"], "OrchestrationAction[send_back]": ["text"],
        TaskSchedule: ["brief"], "OrchestrationAction[task_schedule]": ["brief"], "OrchestrationAction[worker_replace]": ["reason"],
        "OrchestrationAction[report]": ["summary", "plan"], "OrchestrationAction[message_send]": ["text"], "OrchestrationAction[annotate]": ["text"],
        CheckOutcome: ["evidence"], IntegrationMapping: ["explanation"], ResourceRequestRecord: ["blocked_reason"],
        CleanupReason: ["message"], CleanupEffectReceipt: ["evidence"],
      }),
      ...refine(label, {
        Run: ["label"], RootSummary: ["label"], "OrchestrationAction[task_create]": ["title"], "OrchestrationAction[task_assign]": ["title"],
        "OrchestrationAction[task_update]": ["title"], "OrchestrationAction[supervisor_start]": ["label"], "OrchestrationAction[run_adopt]": ["label"],
        "OrchestrationAction[worker_start]": ["label"], "OrchestrationAction[task_scope_update]": ["title"],
        TaskSchedule: ["label"], "OrchestrationAction[task_schedule]": ["label"],
      }),
      ...refine(path, { TaskBoard: ["path"], SetupSummary: ["checkout_path", "branch", "base"], "DispatchTarget[space_worktree]": ["branch", "base_ref"] }),
      ...refine(absolutePath, {
        "ProjectSource[git]": ["common_dir"], "ProjectSource[directory]": ["root"], CheckoutIdentity: ["path"],
        CheckoutGitIdentity: ["common_dir", "worktree_git_dir"],
        OwnedGitIdentity: ["checkout_path"], OwnedSpaceIdentity: ["checkout_path"],
        OwnedFilesystemIdentity: ["root", "manifest_path"],
        OwnedProcessScope: ["control_group"],
        AgentLaunchReceipt: ["latch"],
        OwnedAttemptDirectoryIdentity: ["root"],
        "DispatchTarget[owned_attempt_directory]": ["source_root"],
        "ReplacementPlacement[isolated_directory]": ["directory"],
      }),
      ...refine(value => value === "" || id(value), { OwnedGitIdentity: ["machine_id", "repository_key", "checkout_key"] }),
      ...refine(value => value === "" || absolutePath(value), { OwnedGitIdentity: ["repository_root", "common_dir"] }),
      ...refine(value => value === "" || decimalU64(value), {
        OwnedGitIdentity: ["repository_device", "repository_inode", "common_device", "common_inode", "checkout_device", "checkout_inode"],
      }),
      "OwnedGitIdentity.creation_tip": value => value === "" || gitObject(value),
      ...refine(nonemptyText(1024), { CheckOutcome: ["contract"] }),
      ...refine(nonemptyText(4096), { CheckOutcome: ["command_or_scenario"], "Integration[not_applicable]": ["reason"],
        ResourceSourceIdentity: ["input_configuration", "toolchain_fingerprint"] }),
      ...refine(boundedText(4096), { CheckOutcome: ["limitation"] }),
      ...refine(gitObject, { SetupSummary: ["base_commit"], "SourceOutput[code]": ["base_commit", "head_commit"],
        "Integration[integrated]": ["target_commit"], ResourceSourceIdentity: ["source_commit", "source_tree"] }),
      ...refine(gitObject, { "ReplacementPlacement[isolated_worktree]": ["base_commit"] }),
      "Integration[integrated].target_ref": value => path(value) && value.startsWith("refs/heads/") && value.length > "refs/heads/".length,
      "OwnedGitIdentity.branch_ref": value => value === null
        || (path(value) && value.startsWith("refs/heads/") && value.length > "refs/heads/".length),
      ...refine(hash, {
        Task: ["task_revision"], TaskBoard: ["doc_revision"], NativeShellIdentity: ["argv_digest"],
        RunRetirement: ["task_revision"], Run: ["task_revision_at_start"], TaskIntent: ["expected_task_revision"],
        CleanupEffectReceipt: ["identity_digest"],
        OwnedResourceRecord: ["resource_id"], CleanupResourceSummary: ["resource_id"],
        TaskScopeUpdateResult: ["task_revision", "doc_revision"],
        "SourceOutput[code]": ["task_revision"], "SourceOutput[non_code]": ["task_revision"],
        "Integration[integrated]": ["accepted_task_revision"], IntegrationRecord: ["expected_task_revision"],
        "OrchestrationAction[report]": ["original_task_revision"], "OrchestrationAction[message_send]": ["original_task_revision"],
        OrchestrationSnapshot: ["tasks_token", "state_store_id"], OrchestrationWaitRequest: ["after_tasks_token"], OrchestrationWaitResponse: ["tasks_token"],
        "OrchestrationAction[task_create]": ["expected_doc_revision", "source_revision"], "OrchestrationAction[task_assignment_resolve]": ["expected_task_revision"], "OrchestrationAction[task_update]": ["expected_task_revision"],
        "OrchestrationAction[task_dependencies_set]": ["expected_task_revision", "expected_doc_revision"], "OrchestrationAction[task_step_add]": ["expected_task_revision"], "OrchestrationAction[task_step_rename]": ["expected_task_revision"],
        "OrchestrationAction[task_step_set_checked]": ["expected_task_revision"], "OrchestrationAction[task_step_move]": ["expected_task_revision"], "OrchestrationAction[task_step_remove]": ["expected_task_revision"], "OrchestrationAction[tasks_assign_ids]": ["expected_doc_revision"],
        "OrchestrationAction[worker_start]": ["expected_task_revision", "expected_doc_revision"],
        "OrchestrationAction[task_scope_update]": ["expected_task_revision", "expected_doc_revision"],
        "OrchestrationAction[task_schedule]": ["expected_task_revision", "expected_doc_revision"],
        "OrchestrationAction[accept]": ["expected_task_revision"], "OrchestrationActionResult[task_ids]": ["doc_revision"],
      }),
      ...refine(uuid, {
        RunRetirement: ["retirement_id"], TaskAssignmentIntent: ["task_id"], "OrchestrationAction[task_create]": ["task_id", "follow_up_of"], "OrchestrationAction[task_assign]": ["task_id"], "OrchestrationAction[task_assignment_resolve]": ["task_id"],
        "OrchestrationAction[task_step_add]": ["step_id", "parent_step_id", "before_step_id"], "OrchestrationAction[task_step_rename]": ["step_id"], "OrchestrationAction[task_step_set_checked]": ["step_id"],
        "OrchestrationAction[task_step_move]": ["step_id", "parent_step_id", "before_step_id"], "OrchestrationAction[task_step_remove]": ["step_id"],
        Run: ["checkout_claim_id"], ExecutionAttachment: ["attachment_id"], ResumeRecord: ["operation_id", "decision_id"],
        SupervisorStartResult: ["operation_id", "resume_operation_id"],
        WorkerStartResult: ["operation_id", "run_id"], TaskScopeUpdateResult: ["operation_id"],
        RetirementIdentity: ["attachment_id"],
        ReplacementRetirementAuthority: ["operation_id"],
        AttachmentReference: ["attachment_id"], "Integration[integrated]": ["operation_id"],
        IntegrationRecord: ["operation_id"], IntegrationRecordResult: ["operation_id"],
        ResourceRequest: ["operation_id"], ResourceRequestRecord: ["request_id"],
        ResourceRelease: ["request_id"], ResourceBuildRegistration: ["request_id"],
        "OrchestrationAction[report]": ["original_claim_id"], "OrchestrationAction[message_send]": ["original_claim_id"],
        OwnedGitIdentity: ["creation_operation_id"], OwnedSpaceIdentity: ["creation_operation_id"],
        OwnedFilesystemIdentity: ["creation_operation_id"], OwnedFixtureIdentity: ["creation_operation_id"],
        OwnedAttemptDirectoryIdentity: ["creation_operation_id"],
        CleanupEffectReceipt: ["effect_id"], CleanupItem: ["operation_id"],
        ResourceRegister: ["operation_id"], CleanupFinish: ["operation_id"], CleanupFinishResult: ["operation_id"],
        LiaisonLookupRequest: ["selected_root_id"], LiaisonCandidate: ["root_id"],
        "LiaisonLookupResponse[resolved]": ["preferred_root_id"],
        "OrchestrationAction[supervisor_start]": ["operation_id"], "OrchestrationAction[run_resume]": ["operation_id", "confirmed_stopped_attachment_id"],
        "OrchestrationAction[run_attach]": ["operation_id", "attachment_id"], "OrchestrationAction[run_register]": ["operation_id"], "OrchestrationAction[task_unassign]": ["operation_id"], TaskOwner: ["claim_operation_id"],
        DispatchState: ["move_operation_id"],
        "OrchestrationAction[worker_start]": ["operation_id", "root_id", "task_id"],
        "OrchestrationAction[task_scope_update]": ["operation_id", "root_id", "task_id", "run_id"],
        TaskSchedule: ["operation_id"],
        "OrchestrationAction[task_schedule]": ["operation_id", "root_id", "task_id"],
        "OrchestrationAction[task_schedule_cancel]": ["operation_id", "root_id", "task_id"],
        "OrchestrationAction[worker_replace]": ["operation_id", "root_id", "task_id"],
        "OrchestrationActionResult[task_scheduled]": ["operation_id", "task_id"],
      }),
      ...refine(timestamp, {
        Report: ["at"], RunRetirement: ["created_at", "updated_at"],
        Run: ["created_at", "updated_at"], DispatchState: ["updated_at"], Annotation: ["at"],
        Message: ["created_at", "acked_at"], AnswerReceipt: ["created_at", "acked_at"], QuestionStatus: ["asked_at"],
        RunObservation: ["state_changed_at"], "RuntimeObservation[fresh]": ["observed_at"], Attention: ["since"], UnmanagedAgent: ["state_changed_at"],
        "RetirementState[close_intent]": ["at"], "RetirementState[retired]": ["at"], "RetirementState[retained]": ["at"], "RetirementState[unknown]": ["at"],
        ExecutionAttachment: ["updated_at"], ResumeRecord: ["updated_at"], LiaisonCandidate: ["updated_at"],
        "Integration[integrated]": ["verified_at"], ResourceRequestRecord: ["updated_at"],
        CleanupEffectReceipt: ["updated_at"], CleanupItem: ["updated_at"],
        TaskSchedule: ["scheduled_at"], OwnedProcessScope: ["captured_at"],
      }),
      ...refine(session, { OrchestrationSnapshotRequest: ["session_id"], OrchestrationMutationRequest: ["session_id"], OrchestrationSnapshot: ["session_id"], Run: ["session_id"], RunLocation: ["session_id"], RetirementIdentity: ["session_id"], LiaisonLookupRequest: ["session_id"], OwnedSpaceIdentity: ["session_id"] }),
      ...refine(stepTitle, { "OrchestrationAction[task_step_add]": ["title"], "OrchestrationAction[task_step_rename]": ["title"] }),
      ...refine(decimalU64, { NativeShellIdentity: ["executable_device", "executable_inode"], DirectoryIdentity: ["device", "inode"],
        OwnedFilesystemIdentity: ["device", "inode", "manifest_device", "manifest_inode"], OwnedProcessScope: ["cgroup_device", "cgroup_inode"],
        AgentLaunchReceipt: ["latch_device", "latch_inode"], OwnedAttemptDirectoryIdentity: ["device", "inode"] }),
      "ExecutionAttachment.epoch": epoch,
      "RetirementIdentity.attachment_epoch": epoch,
      "ResumeRecord.previous_attachment_epoch": epoch, "ResumeRecord.replacement_attachment_epoch": epoch,
      "OrchestrationConfiguration.harness_kind": launchText,
      "OrchestrationConfiguration.command": value => launchText(value) && !value.startsWith("-"),
      "OrchestrationConfiguration.extra_args": values => values.every(launchText),
      "OrchestrationConfiguration.resume_args": values => values === null || values.every(launchText),
      "OrchestrationConfiguration.initial_prompt_args": values => values === null || values.every(launchText),
      "OrchestrationConfiguration.hook_args": values => values === null || values.every(launchText),
      "AttemptBadge.attempt": value => value > 0,
      "OrchestrationAction[run_resume].expected_attachment_epoch": epoch, "OrchestrationAction[run_attach].expected_attachment_epoch": epoch,
      "WorkerStartResult.attempt": value => value > 0,
      "AttachmentReference.attachment_epoch": value => epoch(value),
      "SourceOutput[code].commits": values => values.every(gitObject) && new Set(values).size === values.length,
      "SourceOutput[non_code].artifacts": values => values.every(nonemptyText(4096)),
      "IntegrationMapping.source_commits": values => values.length > 0 && values.every(gitObject) && new Set(values).size === values.length,
      "IntegrationMapping.target_commits": values => values.length > 0 && values.every(gitObject) && new Set(values).size === values.length,
      "IntegrationMapping.source_paths": values => values.every(relativePath) && new Set(values).size === values.length,
      "IntegrationMapping.target_paths": values => values.every(relativePath) && new Set(values).size === values.length,
      "Task.depends_on": values => values.every(id), "SetupSummary.effects": values => values.every(text), "SetupSummary.warnings": values => values.every(text),
      "OrchestrationAction[task_create].depends_on": values => values.every(uuid), "OrchestrationAction[task_dependencies_set].depends_on": values => values.every(uuid),
      "OrchestrationAction[inbox_pull].limit": value => value <= 100, "OrchestrationWaitRequest.timeout_ms": value => value <= 30_000,
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
      TaskStepProgress: { progress: value => value.done <= value.total },
      OrchestrationConfiguration: { resume_args: value => value.resume_args === null
        || (value.resume_args.filter(arg => arg === "{session_id}").length === 1
          && value.resume_args.every(arg => !arg.includes("{session_id}") || arg === "{session_id}")) },
      Run: {
        resume_identity: value => value.resume === null || value.resume.run_id === value.run_id,
        output_identity: value => value.source_output === null ? value.result?.source_output == null
          : value.result !== null && value.result.kind === "result"
            && value.source_output.result_message_id === value.result.message_id
            && value.result.source_output !== null && sameSource(value.source_output, value.result.source_output),
        source_checkout: value => value.source_output?.kind !== "code" || (value.source_output.repository_key === value.project?.key
          && value.source_output.checkout_key === value.checkout?.key && value.source_output.base_commit === value.setup?.base_commit),
        integration_identity: value => value.integration.state === "pending" || (value.source_output !== null
          && (value.integration.state === "not_applicable"
            ? value.source_output.kind === "non_code" && value.integration.result_message_id === value.source_output.result_message_id
            : sameSource(value.integration.source, value.source_output) && integratedChecks(value.integration))),
      },
      RunResumeResult: { resume_identity: value => value.resume.run_id === value.run_id },
      Report: { source_identity: value => value.source_output === null
        ? value.kind !== "result" || value.outcome !== "succeeded"
        : value.kind === "result" && value.source_output.result_message_id === value.message_id },
      "OrchestrationAction[report]": { source_identity: value => value.source_output === null
        ? value.kind !== "result" || value.outcome !== "succeeded"
        : value.kind === "result" && value.source_output.result_message_id === value.message_id },
      "SourceOutput[code]": { checks: value => validChecks(value.focused_checks),
        range: value => value.commits.length === 0 ? value.base_commit === value.head_commit
          : value.commits.at(-1) === value.head_commit && !value.commits.includes(value.base_commit) },
      "SourceOutput[non_code]": { checks: value => validChecks(value.focused_checks) },
      "Integration[integrated]": {
        source_identity: value => value.source.kind === "code" && value.accepted_result_message_id === value.source.result_message_id
          && value.accepted_task_revision === value.source.task_revision,
        checks: value => validChecks(value.checks),
        mappings: value => {
          if (value.method === "ancestor") return value.mappings.length === 0;
          if (value.source.kind !== "code" || value.mappings.length === 0) return false;
          const source = value.mappings.flatMap(mapping => mapping.source_commits);
          const target = value.mappings.flatMap(mapping => mapping.target_commits);
          return sameStrings(source, value.source.commits) && new Set(target).size === target.length
            && (value.method !== "recorded_adaptation" || value.mappings.every(mapping => !!mapping.explanation?.trim()));
        },
      },
      IntegrationRecord: { identity: value => value.integration.state === "pending"
        || (value.integration.state === "not_applicable"
          ? value.integration.result_message_id === value.expected_result_message_id
          : value.integration.operation_id === value.operation_id
            && value.integration.accepted_result_message_id === value.expected_result_message_id
            && value.integration.accepted_task_revision === value.expected_task_revision) },
      IntegrationRecordResult: {
        identity: value => value.integration.state !== "integrated" || value.integration.operation_id === value.operation_id,
        checks: value => validChecks(value.checks) && integratedChecks(value.integration),
      },
      OwnedFixtureIdentity: { creation_identity: value => value.creation_operation_id === value.filesystem.creation_operation_id },
      ResourceRegister: { creation_identity: value => completeGitIdentity(value.resource) },
      OwnedResourceRecord: {
        kind: value => value.kind === value.identity.kind,
        authority: value => (value.ownership === "owned") === (value.authority === "owned_when_safe"),
        physical_identity: value => completeGitIdentity(value.identity) || (value.ownership === "unknown"
          && value.authority === "authority_unproven" && value.cleanup.state === "unknown"
          && value.cleanup.reason?.code === "creation_identity_unavailable"),
      },
      CleanupResourceSummary: { public_evidence: value => value.cleanup.effects.every(effect => effect.evidence === null) },
      CleanupFinishResult: { counts: cleanupCounts },
      RunCleanupSummary: { counts: cleanupCounts },
      RestorationSummary: { attachments: value => value.reattached <= value.total_attachments },
      "LiaisonLookupResponse[resolved]": {
        root_identity: value => {
          const roots = new Set<string>();
          for (const root of value.roots) {
            if (roots.has(root.root_id)) return false;
            roots.add(root.root_id);
          }
          return value.preferred_root_id === null || roots.has(value.preferred_root_id);
        },
      },
      RunRetirement: {
        identity: value => value.identity !== null || (value.state.state === "retained" && value.state.reason === "identity_incomplete"),
        authority: value => value.trigger === "replacement"
          ? value.result_message_id === null && value.task_revision === null && value.replacement !== null
          : value.result_message_id !== null && value.task_revision !== null && value.replacement === null,
      },
      "QuestionReceipt[answer_delivered]": { stage: value => value.answer.stage !== "acked" }, "QuestionReceipt[answer_acknowledged]": { stage: value => value.answer.stage === "acked" },
      TaskIntent: {
        provenance: value => value.origin === "supervisor"
          ? value.supervisor_run_id !== null && value.conversation !== null
          : value.supervisor_run_id === null && value.conversation === null,
        root: value => value.origin !== "supervisor" || value.supervisor_run_id === value.root_id,
      },
      RunObservation: { presence: value => !value.actual_agent || value.presence === "present" },
      "OrchestrationAction[task_assignment_resolve]": { assignment_revision: value => !value.assign || value.expected_task_revision !== null },
      "OrchestrationAction[task_scope_update]": { scope: value => value.title !== null || value.description !== null },
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
    || (run.attachment?.location != null && run.attachment.location.session_id !== snapshot.session_id))) malformed("run session identity");
  return snapshot;
}
export function matchOrchestrationSnapshot(value: unknown, request: OrchestrationSnapshotRequest): OrchestrationSnapshot {
  const snapshot = parseOrchestrationSnapshot(value);
  if (snapshot.session_id !== request.session_id || (request.root_id !== null && snapshot.board !== null && snapshot.board.root_id !== request.root_id)) malformed("snapshot identity");
  return snapshot;
}
export function parseOrchestrationMutationResponse(value: unknown): OrchestrationMutationResponse { return parseWire(value, wireOrchestrationMutationResponse, POLICY); }
export function parseOrchestrationWaitResponse(value: unknown): OrchestrationWaitResponse { return parseWire(value, wireOrchestrationWaitResponse, POLICY); }
export function parseLiaisonLookupRequest(value: unknown): LiaisonLookupRequest { return parseWire(value, wireLiaisonLookupRequest, POLICY); }
export function parseLiaisonLookupResponse(value: unknown): LiaisonLookupResponse { return parseWire(value, wireLiaisonLookupResponse, POLICY); }
