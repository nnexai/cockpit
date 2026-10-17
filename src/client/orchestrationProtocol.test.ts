import { describe, expect, it, vi } from "vitest";
import type {
  CheckoutIdentity, ExecutionAttachment, NativeProcessIdentity, NativeShellIdentity, OrchestrationAction,
  OrchestrationSnapshot, ProjectIdentity, ResumeRecord, RetirementIdentity, RetirementState, RunRetirement, ReplacementRetirementAuthority, Task, TaskDependencies,
  CheckOutcome, Integration, ResourceRequestRecord, SourceOutput, CleanupItem, OwnedGitIdentity, OwnedFilesystemIdentity,
  OwnedResourceIdentity, OwnedResourceRecord, CleanupFinishResult, OwnedProcessScope, AgentLaunchReceipt, OwnedAttemptDirectoryIdentity,
} from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";
import { createBrowserClient } from "./browser";
import { createNativeClient } from "./native";
import {
  matchOrchestrationSnapshot, parseOrchestrationAction, parseOrchestrationMutationRequest,
  parseOrchestrationMutationResponse, parseOrchestrationSnapshot, parseOrchestrationSnapshotRequest,
  parseOrchestrationWaitRequest, parseOrchestrationWaitResponse, parseProjectRootLookupRequest, parseProjectRootLookupResponse,
} from "./orchestrationProtocol";

const hash = "a".repeat(64);
const at = "2026-10-05T12:00:00Z";
const assignmentTaskId = "7b613f19-4a52-41fa-8864-a880cd69ef50";
const prerequisiteId = "06c97d2c-8ed7-4231-b0e7-8fd6fd89c649";
const parentStepId = "d6bdcdd4-7f90-4d02-8b8a-9dcda7157018";
const childStepId = "78ff0733-7730-438c-b793-f866d562db95";
const processIdentity: NativeProcessIdentity = { pid: 42, start_ticks: 1234, kernel_boot_id: "kernel-boot" };
const shellIdentity: NativeShellIdentity = {
  process: { ...processIdentity, pid: 7, start_ticks: 100 },
  executable_device: "2049", executable_inode: "18446744073709551615", argv_digest: hash,
};
const project: ProjectIdentity = {
  key: "project-key", source: { kind: "git", common_dir: "/repo/.git" },
  repository_id: "repo", source_workspace_id: "project-space",
};
const checkout: CheckoutIdentity = {
  key: "checkout-key", path: "/repo",
  directory_identity: { device: "2049", inode: "18446744073709551615" },
  git: { common_dir: "/repo/.git", worktree_git_dir: "/repo/.git/worktrees/worker" },
};
const attachment: ExecutionAttachment = {
  attachment_id: assignmentTaskId, epoch: 1, state: "connected", launch_mode: "shell_agent",
  location: { endpoint_identity: "endpoint", session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "pane",
    launch_tag: "launch", boot_id: "boot", terminal_id: "terminal", native_session_id: null, peer_process: null },
  conversation: { harness: "omp", session_id: "opaque-conversation" }, process: processIdentity, shell: shellIdentity, machine_id: "machine",
  process_scope: null, launch_attempt: 1, updated_at: at,
};
const resume: ResumeRecord = {
  operation_id: prerequisiteId, run_id: "run", previous_attachment_epoch: 1, phase: "blocked",
  cause: { code: "writer_unverified", message: "Writer exit cannot be verified", class: "recover" },
  decision_id: null, replacement_attachment_epoch: null, updated_at: at,
};
const startAction: Extract<OrchestrationAction, { action: "supervisor_start" }> = {
  action: "supervisor_start", operation_id: assignmentTaskId, mode: "reuse_or_resume",
  target: null, label: null, selected_root_id: null, expected_project_key: null,
};
const workerStartAction: Extract<OrchestrationAction, { action: "worker_start" }> = {
  action: "worker_start", operation_id: assignmentTaskId, root_id: prerequisiteId, task_id: parentStepId,
  expected_task_revision: hash, expected_doc_revision: hash,
  target: { target: "existing_space", workspace_id: "space" }, label: null, brief: "Implement the assigned task",
  cleanup_policy: "owned_when_safe",
};
const scopeUpdateAction: Extract<OrchestrationAction, { action: "task_scope_update" }> = {
  action: "task_scope_update", operation_id: childStepId, root_id: prerequisiteId, task_id: parentStepId,
  run_id: assignmentTaskId, expected_task_revision: hash, expected_doc_revision: hash,
  title: null, description: "Revised task scope",
};
const workerStarted = {
  result: "worker_started" as const,
  start: { operation_id: assignmentTaskId, run_id: childStepId, attempt: 1, duplicate: false },
};
const scopeUpdated = {
  result: "task_scope_updated" as const,
  update: { operation_id: childStepId, state: "applied" as const, task_revision: hash,
    doc_revision: hash, instruction_message_id: "scope-instruction", duplicate: false },
};
const retirementIdentity: RetirementIdentity = {
  attachment_id: assignmentTaskId, attachment_epoch: 1, launch_mode: "shell_agent",
  run_attempt: 1, launch_attempt: 1, launch_tag: "launch", endpoint_identity: "endpoint",
  session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "pane",
  terminal_id: "terminal", herdr_boot_id: "boot", herdr_peer: null, conversation: attachment.conversation, process: processIdentity, shell: shellIdentity,
};
const retirementStates: RetirementState[] = [
  { state: "waiting", blockers: [] },
  { state: "waiting", blockers: ["open_descendant_runs"] },
  { state: "waiting", blockers: ["resource_admissions"] },
  { state: "close_intent", at },
  ...(["closed_by_cockpit", "already_absent", "absent_after_uncertain_close"] as const)
    .map(terminal => ({ state: "retired" as const, at, terminal })),
  ...([
    "identity_incomplete", "identity_changed", "endpoint_changed", "native_process_unverifiable",
    "process_pane_mismatch", "worker_unresponsive", "worker_busy_timeout", "user_activity", "native_refused",
    "shared_tab", "tab_renamed", "pane_moved", "foreground_process", "observation_unavailable", "herdr_refused",
  ] as const).flatMap(reason => [false, true].map(stopped => ({
    state: "retained" as const, at, reason, stopped,
  }))),
  { state: "unknown", at, phase: "terminal_close", detail: "Unconfirmed" },
];
function retirementRecord(state: RetirementState, identity: RetirementIdentity | null = retirementIdentity): RunRetirement {
  return {
    retirement_id: assignmentTaskId, trigger: "accept", result_message_id: "result",
    task_revision: hash, replacement: null, identity, state, created_at: at, updated_at: at,
  };
}
const target = { target: "setup" as const, request: { operation: "open" as const, path: "/repo", label: null, task_name: null, focus: false } };
const task: Task = {
  task_id: "task", title: "Title",
  body: "Body\n<!-- cockpit-depends-on: 06c97d2c-8ed7-4231-b0e7-8fd6fd89c649 -->\n- [x] Parent\n  - [ ] Child",
  description: "Body", description_editable: true, description_diagnostic: null,
  steps: [
    { step_id: parentStepId, parent_step_id: null, depth: 0, title: "Parent", checked: true,
      status: "partial", line: 5, source_offset: 94, diagnostic: null },
    { step_id: childStepId, parent_step_id: parentStepId, depth: 1, title: "Child", checked: false,
      status: "open", line: 6, source_offset: 108, diagnostic: null },
  ],
  step_progress: { done: 0, total: 1 }, steps_diagnostic: null,
  depends_on: [prerequisiteId], follow_up_of: assignmentTaskId, relations_diagnostic: null,
  checked: false, line: 3, task_revision: hash, diagnostic: null,
};
const dependencies: TaskDependencies = {
  state: "blocked", unmet: [{ task_id: prerequisiteId, reason: "unchecked" }], problems: [],
};
const report = { message_id: "report", kind: "initialized" as const, outcome: "succeeded" as const, summary: "Initialized",
  plan: "Work plan", source_output: null, at };
const baseCommit = "b".repeat(40), headCommit = "c".repeat(40), targetCommit = "d".repeat(40);
const focusedCheck: CheckOutcome = {
  contract: "decoder", command_or_scenario: "bun run test:client", outcome: "passed", evidence: "Client boundary passed", limitation: null,
};
const codeOutput: Extract<SourceOutput, { kind: "code" }> = {
  kind: "code", repository_key: project.key, checkout_key: checkout.key, base_commit: baseCommit, head_commit: headCommit,
  commits: [headCommit], task_revision: hash, result_message_id: "result", focused_checks: [focusedCheck],
};
const nonCodeOutput: Extract<SourceOutput, { kind: "non_code" }> = {
  kind: "non_code", result_message_id: "result", task_revision: hash, artifacts: ["design.html"], focused_checks: [focusedCheck],
};
const integrated: Extract<Integration, { state: "integrated" }> = {
  state: "integrated", operation_id: prerequisiteId, accepted_result_message_id: "result", accepted_task_revision: hash,
  source: codeOutput, target_repository_key: project.key, target_ref: "refs/heads/main", target_commit: targetCommit,
  method: "recorded_adaptation",
  mappings: [{ source_commits: [headCommit], target_commits: [targetCommit], source_paths: ["src/client/example.ts"],
    target_paths: ["src/client/example.ts"], explanation: "Adapted against current target API" }],
  checks: [{ ...focusedCheck, evidence: `Client boundary passed at ${targetCommit}` }],
  integrator_run_id: "root", verified_at: at,
};
const integrationRecord = {
  operation_id: prerequisiteId, run_id: "run", expected_result_message_id: "result", expected_task_revision: hash, integration: integrated,
};
const resourceRequest = {
  operation_id: parentStepId, resource_key: "/repo/target",
  source_identity: { checkout_key: checkout.key, source_commit: headCommit, source_tree: baseCommit,
    input_configuration: "release", toolchain_fingerprint: "rustc-1.90" },
  holder: { run_id: "run", attachment_id: attachment.attachment_id, attachment_epoch: 1 },
};
const resourceRecord: ResourceRequestRecord = {
  request_id: parentStepId, sequence: 1, resource_key: resourceRequest.resource_key, source_identity: resourceRequest.source_identity,
  holder: resourceRequest.holder, state: "held", build_processes: [shellIdentity],
  machine_id: "machine", blocked_reason: null, updated_at: at,
};
const ownedGit: OwnedGitIdentity = {
  creation_operation_id: prerequisiteId, machine_id: "machine", repository_key: project.key, repository_root: "/repo",
  repository_device: "2049", repository_inode: "400", common_dir: "/repo/.git", common_device: "2049", common_inode: "401",
  checkout_path: "/repo/worktrees/worker", checkout_device: "2049", checkout_inode: "402", checkout_key: checkout.key,
  branch_ref: "refs/heads/worker", creation_tip: headCommit,
};
const ownedFilesystem: OwnedFilesystemIdentity = {
  creation_operation_id: prerequisiteId, machine_id: "machine", root: "/tmp/disposable", device: "2049", inode: "403",
  manifest_path: "/tmp/disposable/manifest.json", manifest_device: "2049", manifest_inode: "404",
};
const ownedAttemptDirectory: OwnedAttemptDirectoryIdentity = {
  creation_operation_id: prerequisiteId, root: `/state/orchestration/attempts/${childStepId}`, device: "2049", inode: "405",
};
const ownedIdentities: OwnedResourceIdentity[] = [
  { kind: "worker_terminal", identity: retirementIdentity }, { kind: "worktree", identity: ownedGit },
  { kind: "branch", identity: ownedGit },
  { kind: "space", identity: { creation_operation_id: prerequisiteId, endpoint_identity: "endpoint", session_id: "session",
    workspace_id: "space", checkout_path: "/repo/worktrees/worker" } },
  { kind: "scratch", identity: ownedFilesystem },
  { kind: "fixture_set", identity: { adapter: "ui_polish_runtime", creation_operation_id: prerequisiteId,
    filesystem: ownedFilesystem, processes: [processIdentity] } },
  { kind: "attempt_directory", identity: ownedAttemptDirectory },
];
const cleanupItem: CleanupItem = {
  operation_id: null, state: "waiting_integration", reason: { class: "transient", code: "integration_pending",
    message: "Accepted source awaits integration" }, effects: [], updated_at: at,
};
const ownedResource: OwnedResourceRecord = {
  resource_id: "f".repeat(64), root_id: "root", run_id: "run", kind: "worktree", ownership: "owned",
  creation_operation_id: prerequisiteId, identity: ownedIdentities[1]!, authority: "owned_when_safe",
  consumers: [resourceRequest.holder], integration_result_id: "result", cleanup: cleanupItem,
};
const cleanupFinished: CleanupFinishResult = {
  operation_id: parentStepId, items: [ownedResource], removed_count: 0, waiting_count: 1, kept_count: 0, unknown_count: 0, duplicate: false,
};
const message = {
  message_id: "message", to_run_id: "run", seq: 1, from: { type: "run" as const, run_id: "root" },
  kind: "report" as const, text: "Initialized", in_reply_to: null, report, stale: false, escalated_from: null,
  stage: "read" as const, created_at: at, acked_at: null,
};
const snapshot: OrchestrationSnapshot = {
  state_store_id: hash, cleanup: [], restoration: { reattached: 1, total_attachments: 1, cleanup_pending: 0 },
  session_id: "session", revision: 2, tasks_token: hash,
  roots: [{ root_id: "root", label: "Supervisor", kind: "supervisor", open_runs: 1, needs_you: 1 }],
  board: { root_id: "root", path: "/tasks/root.md", doc_revision: hash, unidentified_items: 0,
    diagnostics: [{ code: "task_id_duplicate", message: "Duplicate" }], tasks: [{ task, lane: "working", current_run_id: "run", dependencies, scheduled: false, attempt: null }] },
  runs: [{
    session_id: "session", brief: "Implement the assigned task", run_id: "run", kind: "worker", label: "Worker", root_id: "root",
    parent_run_id: "root", task_id: "task", attempt: 1, task_revision_at_start: hash, stage: "working", close_reason: null,
    assignment_generation: 1, launch_config: { harness_kind: "omp", command: "omp", extra_args: [], resume_args: null,
      initial_prompt_args: null, hook_args: null, max_parallel: 4, process_scope: "auto", inbox_prompt_hints: false, routes: [] },
    dispatch: { step: "launched", launch_attempt: 1, error: { code: "launch_unknown", message: "Review" }, updated_at: at,
      launch_tag: "launch", endpoint_identity: "endpoint", recovery: null, agent_started: true, scope_unit: null, pre_move_scope: null,
      latch: null, latch_state: null, source_location: null, move_operation_id: null, staging_workspace_id: null },
    target, setup: { operation_id: "operation", generation: 1, workspace_id: "space", checkout_path: "/repo", repository_id: "repo",
      branch: "task", base: "main", base_commit: baseCommit, ownership: "owned_worktree", effects: ["Create"], warnings: ["Warning"], project_workspace_id: null },
    cleanup_policy: "owned_when_safe",
    last_report: report, result: null, source_output: null, integration: { state: "pending" },
    annotations: [{ by: { type: "operator" }, text: "Note", at }],
    project, checkout, attachment, checkout_claim_id: childStepId, resume, retirement: null,
    created_at: at, updated_at: at,
  }],
  messages: [message],
  task_ownership: [{ root_id: "root", task_id: "task", generation: 1, owner: { run_id: "run", conversation: attachment.conversation!, claim_operation_id: prerequisiteId }, schedule: null }],
  intents: [{ intent_id: "intent", root_id: "root", task_id: "task", run_id: "run", expected_task_revision: hash, state: "pending",
    origin: null, supervisor_run_id: null, conversation: null, result_message_id: null }],
  assignment_intents: [{ root_id: "root", task_id: assignmentTaskId, state: "conflict" }],
  questions: [{ run_id: "run", question_message_id: "question", asked_at: at, receipt: { status: "unresolved" } }],
  runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: at,
    runs: [{ run_id: "run", presence: "present", actual_agent: true, workspace_id: "space", workspace_label: "Space", tab_id: "tab", tab_label: "Worker", pane_id: "moved-pane", agent_status: "idle", state_changed_at: at }] },
  unmanaged_agents: [{ workspace_id: "space", workspace_label: "Space", tab_id: "other-tab", tab_label: "Unmanaged", pane_id: "other-pane",
    agent_name: "omp", agent_status: "idle", state_changed_at: at }],
  attention: [{ kind: "to_accept", run_id: "run", task_id: "task", message_seq: 1, since: at }],
};
const codeRun = {
  ...snapshot.runs[0]!, stage: "reported" as const, source_output: codeOutput,
  result: { ...report, message_id: "result", kind: "result" as const, source_output: codeOutput },
};
const actions: OrchestrationAction[] = [
  { action: "task_create", root_id: "root", task_id: assignmentTaskId, title: "Title", description: "Body",
    depends_on: [prerequisiteId], follow_up_of: prerequisiteId, expected_doc_revision: hash, source_revision: hash },
  { action: "task_assign", root_id: "root", task_id: assignmentTaskId, title: "Title", description: "Body" },
  { action: "task_assignment_resolve", root_id: "root", task_id: assignmentTaskId, expected_task_revision: hash, assign: true },
  { action: "task_update", root_id: "root", task_id: "task", expected_task_revision: hash, title: null, description: "Body" },
  { action: "task_dependencies_set", root_id: "root", task_id: "task", expected_task_revision: hash, expected_doc_revision: hash, depends_on: [prerequisiteId] },
  { action: "task_step_add", root_id: "root", task_id: "task", expected_task_revision: hash, step_id: childStepId, parent_step_id: parentStepId, before_step_id: null, title: "Child" },
  { action: "task_step_rename", root_id: "root", task_id: "task", expected_task_revision: hash, step_id: childStepId, title: "Renamed" },
  { action: "task_step_set_checked", root_id: "root", task_id: "task", expected_task_revision: hash, step_id: childStepId, checked: true, scope: "leaf" },
  { action: "task_step_move", root_id: "root", task_id: "task", expected_task_revision: hash, step_id: childStepId, parent_step_id: null, before_step_id: parentStepId },
  { action: "task_step_remove", root_id: "root", task_id: "task", expected_task_revision: hash, step_id: childStepId },
  { action: "tasks_assign_ids", root_id: "root", expected_doc_revision: hash },
  startAction,
  { action: "run_resume", run_id: "run", operation_id: prerequisiteId, expected_attachment_epoch: 1 },
  { action: "run_attach", operation_id: prerequisiteId, attachment_id: assignmentTaskId, expected_attachment_epoch: 1, conversation: attachment.conversation! },
  { action: "run_register", operation_id: prerequisiteId },
  { action: "task_unassign", operation_id: prerequisiteId, root_id: "root", task_id: "task", expected_assignment_generation: 1 },
  { action: "run_adopt", label: "Adopted", location: attachment.location! },
  workerStartAction,
  { action: "task_schedule", operation_id: assignmentTaskId, root_id: prerequisiteId, task_id: parentStepId,
    target: null, brief: "Implement after dependencies", cleanup_policy: "owned_when_safe", label: null,
    expected_task_revision: hash, expected_doc_revision: hash },
  { action: "task_schedule_cancel", operation_id: assignmentTaskId, root_id: prerequisiteId, task_id: parentStepId },
  { action: "worker_replace", operation_id: assignmentTaskId, root_id: prerequisiteId, task_id: parentStepId,
    expected_run_id: "run", expected_assignment_generation: 1, reason: "Stalled worker" },
  scopeUpdateAction,
  { action: "accept", run_id: "run", expected_task_revision: hash },
  { action: "send_back", run_id: "run", text: "Retry" },
  { action: "cancel_run", run_id: "run" },
  { action: "reconcile_run", run_id: "run", recovery: "accept_existing_worktree" },
  { action: "intent_resolve", intent_id: "intent", apply: false },
  { action: "report", message_id: "result", kind: "result", outcome: "succeeded", summary: "Done", plan: null, source_output: codeOutput, to_run_id: null },
  { action: "integration_record", record: integrationRecord },
  { action: "resource_request", request: resourceRequest },
  { action: "resource_release", release: { request_id: parentStepId, expected_holder: resourceRequest.holder } },
  { action: "resource_build_register", registration: { request_id: parentStepId, expected_holder: resourceRequest.holder, process: processIdentity } },
  { action: "resource_register", request: { operation_id: prerequisiteId, run_id: "run", resource: ownedIdentities[4]! } },
  { action: "cleanup_finish", request: { operation_id: parentStepId, root_id: "root", run_id: null } },
  { action: "message_send", message_id: "message", to_run_id: "run", kind: "instruction", text: "Work", in_reply_to: null },
  { action: "annotate", run_id: "run", text: "Note" },
  { action: "inbox_pull", after_seq: 0, limit: 100 },
  { action: "inbox_ack", through_seq: 1 },
];

// Exercise required fields recursively, including required-nullable fields in every
// nested DTO. The paths identify the boundary that failed instead of hiding it in a cast.
function fieldPaths(value: unknown, prefix: (string | number)[] = []): (string | number)[][] {
  if (Array.isArray(value)) return value.flatMap((item, index) => fieldPaths(item, [...prefix, index]));
  if (typeof value !== "object" || value === null) return [];
  return Object.entries(value).flatMap(([key, item]) => [[...prefix, key], ...fieldPaths(item, [...prefix, key])]);
}
function replaceField(value: unknown, path: (string | number)[], replacement: unknown, remove = false): unknown {
  if (path.length === 0) return structuredClone(replacement);
  const copy = structuredClone(value);
  let owner = copy as Record<string | number, unknown>;
  for (const key of path.slice(0, -1)) owner = owner[key] as Record<string | number, unknown>;
  const key = path[path.length - 1]!;
  if (remove) delete owner[key]; else owner[key] = replacement;
  return copy;
}
const defaultedSnapshotFields: Readonly<Record<string, true>> = {
  "runs.0.attachment.process_scope": true,
  "runs.0.attachment.location.peer_process": true,
  "runs.0.dispatch.scope_unit": true,
  "runs.0.dispatch.pre_move_scope": true,
  "runs.0.dispatch.latch": true,
  "runs.0.dispatch.latch_state": true,
  "runs.0.dispatch.source_location": true,
  "runs.0.dispatch.move_operation_id": true,
  "runs.0.dispatch.staging_workspace_id": true,
  "task_ownership.0.owner.conversation": true,
  "task_ownership.0.schedule": true,
  "runs.0.launch_config.initial_prompt_args": true,
  "runs.0.launch_config.hook_args": true,
  "runs.0.launch_config.max_parallel": true,
  "runs.0.launch_config.process_scope": true,
};

describe("orchestration protocol boundary", () => {
  it("decodes queued schedules, explicit replacement and current task attempt badges", () => {
    const scheduleAction = actions.find(action => action.action === "task_schedule")!;
    const replaceAction = actions.find(action => action.action === "worker_replace")!;
    expect(parseOrchestrationAction(scheduleAction)).toEqual(scheduleAction);
    expect(parseOrchestrationAction(replaceAction)).toEqual(replaceAction);
    expect(parseOrchestrationAction({ ...scheduleAction, expected_task_revision: null, expected_doc_revision: null })).toBeDefined();
    for (const [action, field, invalid] of [
      [scheduleAction, "expected_task_revision", "bad"], [scheduleAction, "task_id", "not-a-uuid"],
      [replaceAction, "expected_run_id", ""], [replaceAction, "reason", "bad\0reason"],
      [replaceAction, "expected_assignment_generation", -1],
    ] as const) {
      expect(() => parseOrchestrationAction({ ...action, [field]: invalid })).toThrow(CockpitClientError);
    }
    const schedule = { operation_id: assignmentTaskId, principal_key: "conversation-key", target: null,
      brief: "Implement when ready", cleanup_policy: "owned_when_safe" as const, label: null, scheduled_at: at };
    const dto = { ...snapshot, task_ownership: [{ ...snapshot.task_ownership[0]!, schedule }] };
    expect(parseOrchestrationSnapshot(dto).task_ownership[0]!.schedule).toEqual(schedule);
    for (const state of ["starting", "running", "stalled"] as const) {
      const badge = { run_id: "run", attempt: 2, state };
      const view = { ...snapshot.board!.tasks[0]!, scheduled: true, attempt: badge };
      const board = { ...snapshot.board!, tasks: [view] };
      expect(parseOrchestrationSnapshot({ ...snapshot, board }).board!.tasks[0]).toEqual(view);
      for (const invalid of [{ ...badge, attempt: 0 }, { ...badge, state: "queued" }, { ...badge, run_id: "" }]) {
        expect(() => parseOrchestrationSnapshot({ ...snapshot, board: { ...board, tasks: [{ ...view, attempt: invalid }] } }))
          .toThrow(CockpitClientError);
      }
    }
    for (const placement of [{ placement: "same_checkout" }, { placement: "isolated_worktree", base_commit: baseCommit },
      { placement: "isolated_directory", directory: ownedAttemptDirectory.root }]) {
      const response = { revision: 3, result: { result: "worker_replaced", start: workerStarted.start,
        previous_run_id: "run", assignment_generation: 2, placement } };
      expect(parseOrchestrationMutationResponse(response)).toEqual(response);
      expect(() => parseOrchestrationMutationResponse(replaceField(response, ["result", "previous_run_id"], ""))).toThrow(CockpitClientError);
    }
    expect(() => parseOrchestrationMutationResponse({ revision: 3, result: { result: "worker_replaced",
      start: workerStarted.start, previous_run_id: "run", assignment_generation: 2,
      placement: { placement: "isolated_worktree", base_commit: "bad" } } })).toThrow(CockpitClientError);
  });

  it("decodes isolated directory attempts with their own four-field identity and preserves outputs-kept facts", () => {
    for (const source_workspace_id of ["source-space", null]) {
      const action = { ...workerStartAction, target: { target: "owned_attempt_directory",
        source_root: "/notes/source", source_workspace_id } };
      expect(parseOrchestrationAction(action)).toEqual(action);
      expect(() => parseOrchestrationAction(replaceField(action, ["target", "source_root"], "../source"))).toThrow(CockpitClientError);
      expect(() => parseOrchestrationAction(replaceField(action, ["target", "target"], "future_directory"))).toThrow(CockpitClientError);
    }
    const record = { ...ownedResource, kind: "attempt_directory", identity: { kind: "attempt_directory", identity: ownedAttemptDirectory },
      cleanup: { ...cleanupItem, state: "kept", reason: { class: "authority", code: "outputs_kept",
        message: "Publish unique outputs before removing the empty attempt directory" } } };
    const response = { revision: 3, result: { result: "resource_registered", resource: record, duplicate: false } };
    expect(parseOrchestrationMutationResponse(response)).toEqual(response);
    for (const [field, invalid] of [
      ["creation_operation_id", "not-a-uuid"], ["root", "../source"], ["device", "01"], ["inode", "-1"],
    ] as const) {
      expect(() => parseOrchestrationMutationResponse(replaceField(response, ["result", "resource", "identity", "identity", field], invalid)))
        .toThrow(CockpitClientError);
    }
    for (const extra of [{ machine_id: "machine" }, { manifest_path: "/state/manifest" }, { creation_id: assignmentTaskId }]) {
      expect(() => parseOrchestrationMutationResponse(replaceField(response, ["result", "resource", "identity", "identity"],
        { ...ownedAttemptDirectory, ...extra }))).toThrow(CockpitClientError);
    }
    const replacement = { revision: 4, result: { result: "worker_replaced", start: workerStarted.start,
      previous_run_id: "run", assignment_generation: 2, placement: { placement: "isolated_directory", directory: ownedAttemptDirectory.root } } };
    expect(parseOrchestrationMutationResponse(replacement)).toEqual(replacement);
    expect(() => parseOrchestrationMutationResponse(replaceField(replacement, ["result", "placement", "directory"], "../attempt"))).toThrow(CockpitClientError);
  });

  it("decodes pinned latch and moved-space receipts without accepting incomplete proof or future states", () => {
    const latch: AgentLaunchReceipt = { latch: "/state/launch-latch", nonce: "launch-nonce",
      latch_device: "29", latch_inode: "123" };
    const dispatch = { ...snapshot.runs[0]!.dispatch!, latch, source_location: attachment.location!,
      move_operation_id: assignmentTaskId, staging_workspace_id: "staging-space" };
    for (const latch_state of ["awaiting", "release_intent", "released", "unknown"] as const) {
      const receipt = { ...dispatch, latch_state };
      expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "dispatch"], receipt))
        .runs[0]!.dispatch).toEqual(receipt);
    }
    const dto = replaceField(snapshot, ["runs", 0, "dispatch"], { ...dispatch, latch_state: "released" });
    for (const field of fieldPaths(latch)) {
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "dispatch", "latch", ...field], undefined, true)))
        .toThrow(CockpitClientError);
    }
    for (const [path, invalid] of [
      [["latch", "latch"], "../latch"], [["latch", "nonce"], ""], [["latch", "latch_inode"], "01"],
      [["latch"], { ...latch, private_capability: "secret" }], [["latch_state"], "future_state"],
      [["step"], "future_step"], [["source_location", "pane_id"], ""],
      [["move_operation_id"], "not-a-uuid"], [["staging_workspace_id"], ""],
    ] as const) {
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "dispatch", ...path], invalid)))
        .toThrow(CockpitClientError);
    }
    for (const field of ["latch", "latch_state", "source_location", "move_operation_id", "staging_workspace_id"] as const) {
      expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "dispatch", field], undefined, true))
        .runs[0]!.dispatch![field]).toBeNull();
    }
  });

  it("preserves owned process scope pins and defaults absent new conversation or scope facts", () => {
    const scope: OwnedProcessScope = { kind: "systemd_user_scope", unit: "cockpit-worker.scope",
      invocation_id: "invocation", control_group: "/user.slice/cockpit-worker.scope",
      cgroup_device: "29", cgroup_inode: "123", machine_id: "machine", kernel_boot_id: "kernel-boot",
      member: processIdentity, captured_at: at };
    const dto = replaceField(snapshot, ["runs", 0, "attachment", "process_scope"], scope);
    expect(parseOrchestrationSnapshot(dto).runs[0]!.attachment!.process_scope).toEqual(scope);
    const preMoveScope = { ...scope, captured_at: "2026-10-05T11:59:00Z" };
    const withPreMoveScope = replaceField(dto, ["runs", 0, "dispatch", "pre_move_scope"], preMoveScope);
    const parsedScopes = parseOrchestrationSnapshot(withPreMoveScope).runs[0]!;
    expect(parsedScopes.dispatch!.pre_move_scope).toEqual(preMoveScope);
    expect(parsedScopes.attachment!.process_scope).toEqual(scope);
    expect(parseOrchestrationSnapshot(replaceField(withPreMoveScope, ["runs", 0, "dispatch", "pre_move_scope"], undefined, true))
      .runs[0]!.dispatch!.pre_move_scope).toBeNull();
    for (const [field, invalid] of [
      ["kind", "process_group"], ["unit", ""], ["control_group", "../scope"], ["cgroup_inode", "01"],
      ["machine_id", ""], ["captured_at", "yesterday"],
    ] as const) {
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "attachment", "process_scope", field], invalid)))
        .toThrow(CockpitClientError);
    }
    for (const path of fieldPaths(scope)) {
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "attachment", "process_scope", ...path], undefined, true)))
        .toThrow(CockpitClientError);
    }
    expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "attachment", "process_scope"], undefined, true))
      .runs[0]!.attachment!.process_scope).toBeNull();
    const registered = { revision: 3, result: { result: "run_registered", run_id: "run", assignment_generation: null, duplicate: false } };
    expect(parseOrchestrationMutationResponse(registered)).toEqual({ ...registered, result: { ...registered.result, conversation: null } });
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["task_ownership", 0, "owner", "conversation"], undefined, true))
      .task_ownership[0]!.owner!.conversation).toBeNull();
    for (const max_parallel of [0, 4]) {
      expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "launch_config", "max_parallel"], max_parallel))
        .runs[0]!.launch_config.max_parallel).toBe(max_parallel);
    }
  });

  it("roundtrips every closed owned-resource identity and rejects mismatched kinds or private capabilities", () => {
    for (const resource of ownedIdentities) {
      const action = { action: "resource_register", request: { operation_id: prerequisiteId, run_id: "run", resource } };
      expect(parseOrchestrationAction(action)).toEqual(action);
      const record = { ...ownedResource, kind: resource.kind, identity: resource,
        creation_operation_id: resource.kind === "worker_terminal" ? resource.identity.launch_tag : resource.identity.creation_operation_id };
      const response = { revision: 3, result: { result: "resource_registered", resource: record, duplicate: false } };
      expect(parseOrchestrationMutationResponse(response)).toEqual(response);
      expect(() => parseOrchestrationMutationResponse(replaceField(response, ["result", "resource", "kind"],
        resource.kind === "scratch" ? "worktree" : "scratch"))).toThrow(CockpitClientError);
      for (const dto of [action, response]) {
        const parse = dto === action ? parseOrchestrationAction : parseOrchestrationMutationResponse;
        for (const field of fieldPaths(dto)) {
          if (field[field.length - 1] === "herdr_peer") {
            expect(parse(replaceField(dto, field, undefined, true))).toEqual(dto);
          } else {
            expect(() => parse(replaceField(dto, field, undefined, true)), field.join(".")).toThrow(CockpitClientError);
          }
        }
        for (const ownerPath of [[], ...fieldPaths(dto)]) {
          const owner = ownerPath.reduce<unknown>((value, key) => (value as Record<string | number, unknown>)[key], dto);
          if (owner === null || typeof owner !== "object" || Array.isArray(owner)) continue;
          expect(() => parse(replaceField(dto, ownerPath, { ...owner, secret: "private" }))).toThrow(CockpitClientError);
        }
      }
    }
    const fixture = { action: "resource_register", request: { operation_id: prerequisiteId, run_id: "run", resource: ownedIdentities[5] } };
    for (const [field, value] of [
      [["identity", "adapter"], "arbitrary_script"], [["identity", "filesystem", "creation_operation_id"], childStepId],
      [["identity", "filesystem", "root"], "/tmp/../persistent"], [["identity", "processes", 0, "pid"], -1],
    ] as const) expect(() => parseOrchestrationAction(replaceField(fixture, ["request", "resource", ...field], value))).toThrow(CockpitClientError);
  });

  it("preserves borrowed and unknown resources without granting owned cleanup authority", () => {
    for (const ownership of ["borrowed", "unknown"]) {
      const resource = { ...ownedResource, ownership, authority: "authority_unproven",
        cleanup: { ...cleanupItem, state: "kept", reason: { class: "authority", code: "not_owned", message: "Ownership unproven" } } };
      const response = { revision: 3, result: { result: "resource_registered", resource, duplicate: true } };
      expect(parseOrchestrationMutationResponse(response)).toEqual(response);
      expect(() => parseOrchestrationMutationResponse(replaceField(response, ["result", "resource", "authority"], "owned_when_safe")))
        .toThrow(CockpitClientError);
    }
    const response = { revision: 3, result: { result: "resource_registered", resource: ownedResource, duplicate: false } };
    for (const [field, value] of [
      [["authority"], "authority_unproven"], [["resource_id"], prerequisiteId], [["identity", "identity", "creation_tip"], "short"],
      [["identity", "identity", "repository_device"], "01"], [["identity", "identity", "checkout_path"], "relative/path"],
    ] as const) expect(() => parseOrchestrationMutationResponse(replaceField(response, ["result", "resource", ...field], value)))
      .toThrow(CockpitClientError);
  });

  it("keeps unavailable Git creation pins explicit Unknown and never treats them as registration authority", () => {
    const unknownGit: OwnedGitIdentity = { ...ownedGit, machine_id: "", repository_key: "", repository_root: "",
      repository_device: "", repository_inode: "", common_dir: "", common_device: "", common_inode: "",
      checkout_device: "", checkout_inode: "", checkout_key: "", creation_tip: "", branch_ref: null };
    const resource = { ...ownedResource, ownership: "unknown", authority: "authority_unproven",
      identity: { kind: "worktree", identity: unknownGit },
      cleanup: { ...cleanupItem, state: "unknown", reason: { class: "authority",
        code: "creation_identity_unavailable", message: "Creation identity is unavailable" } } };
    const response = { revision: 3, result: { result: "resource_registered", resource, duplicate: false } };
    expect(parseOrchestrationMutationResponse(response)).toEqual(response);
    expect(() => parseOrchestrationAction({ action: "resource_register",
      request: { operation_id: prerequisiteId, run_id: "run", resource: resource.identity } })).toThrow(CockpitClientError);
    for (const patch of [
      { ownership: "owned", authority: "owned_when_safe" }, { ownership: "borrowed" },
      { cleanup: { ...resource.cleanup, state: "waiting" } },
      { cleanup: { ...resource.cleanup, reason: { ...resource.cleanup.reason, code: "not_owned" } } },
    ]) expect(() => parseOrchestrationMutationResponse({ ...response,
      result: { ...response.result, resource: { ...resource, ...patch } } })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationMutationResponse(replaceField(response,
      ["result", "resource", "identity", "identity", "checkout_path"], ""))).toThrow(CockpitClientError);
    expect(() => parseOrchestrationMutationResponse(replaceField(response,
      ["result", "resource", "identity", "identity", "creation_operation_id"], ""))).toThrow(CockpitClientError);
  });
  it("decodes canonical ownership and neutral registration without inferring process presence", () => {
    const conversation = { harness: "custom", session_id: "resumable/non-uuid-session" };
    const ownership = { ...snapshot.task_ownership[0]!, owner: { ...snapshot.task_ownership[0]!.owner!, conversation } };
    const dto = { ...snapshot, task_ownership: [ownership], runtime: { status: "unavailable" as const, error: { code: "offline", message: "Unavailable" } } };
    expect(parseOrchestrationSnapshot(dto).task_ownership).toEqual([ownership]);
    expect(parseOrchestrationSnapshot({ ...dto, task_ownership: [{ ...ownership, generation: 2, owner: null }] }).task_ownership[0]!.owner).toBeNull();
    const registered = { revision: 3, result: { result: "run_registered", run_id: "run", conversation, assignment_generation: 1, duplicate: false } };
    const unassigned = { revision: 4, result: { result: "task_unassigned", task_id: "task", assignment_generation: 2, previous_run_id: "run", duplicate: false } };
    expect(parseOrchestrationMutationResponse(registered)).toEqual(registered);
    expect(parseOrchestrationMutationResponse(unassigned)).toEqual(unassigned);
    for (const path of [["task_ownership", 0, "generation"], ["runs", 0, "assignment_generation"]]) {
      for (const invalid of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, "1"]) {
        expect(() => parseOrchestrationSnapshot(replaceField(dto, path, invalid))).toThrow(CockpitClientError);
      }
    }
    for (const field of ["harness", "session_id"]) {
      for (const invalid of ["", "bad\nidentity", "é".repeat(257), null]) {
        expect(() => parseOrchestrationSnapshot(replaceField(dto, ["task_ownership", 0, "owner", "conversation", field], invalid))).toThrow(CockpitClientError);
      }
    }
    expect(() => parseOrchestrationSnapshot(replaceField(dto, ["task_ownership", 0, "owner", "claim_operation_id"], "not-a-uuid"))).toThrow(CockpitClientError);
  });

  it("keeps configured launch argv literal with one whole resume placeholder", () => {
    const config = { ...snapshot.runs[0]!.launch_config, harness_kind: "custom", command: "/usr/bin/custom-agent", extra_args: ["--literal", "value with spaces"], resume_args: ["resume", "{session_id}"] };
    const dto = replaceField(snapshot, ["runs", 0, "launch_config"], config);
    expect(parseOrchestrationSnapshot(dto).runs[0]!.launch_config).toEqual(config);
    for (const invalid of [[], ["resume"], ["{session_id}", "{session_id}"], ["--resume={session_id}"], ["{session_id}", "bad\0arg"], Array(17).fill("{session_id}")]) {
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "launch_config", "resume_args"], invalid))).toThrow(CockpitClientError);
    }
    for (const removed of [{ omp_extension: "/extension" }, { model: "old-model" }]) {
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "launch_config"], { ...config, ...removed }))).toThrow(CockpitClientError);
    }
  });

  it("decodes independent cleanup states with exact counters and retains typed reasons without raw summary evidence", () => {
    const states = ["pending", "running", "waiting", "waiting_integration", "reconciling", "removed", "kept", "stalled", "unknown"] as const;
    const items = states.map((state, index) => ({ ...ownedResource, resource_id: index.toString(16).repeat(64),
      cleanup: { ...cleanupItem, state } }));
    const cleanup = { ...cleanupFinished, items, removed_count: 1, waiting_count: 6, kept_count: 1, unknown_count: 1 };
    const response = { revision: 3, result: { result: "cleanup_finished", cleanup } };
    expect(parseOrchestrationMutationResponse(response)).toEqual(response);
    const summary = { run_id: "run", items: items.map(item => ({ resource_id: item.resource_id,
      kind: item.kind, ownership: item.ownership, cleanup: item.cleanup })), removed_count: 1, waiting_count: 6, kept_count: 1, unknown_count: 1 };
    const dto = { ...snapshot, cleanup: [summary], restoration: { ...snapshot.restoration, cleanup_pending: 7 } };
    expect(parseOrchestrationSnapshot(dto)).toEqual(dto);
    for (const field of ["removed_count", "waiting_count", "kept_count", "unknown_count"]) {
      expect(() => parseOrchestrationMutationResponse(replaceField(response, ["result", "cleanup", field], 0))).toThrow(CockpitClientError);
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["cleanup", 0, field], 0))).toThrow(CockpitClientError);
    }
    for (const state of ["intent", "applied", "absent", "refused", "unknown"]) {
      const effect = { effect_id: assignmentTaskId, kind: ownedResource.kind, identity_digest: ownedResource.resource_id,
        state, evidence: "Adapter process receipt", updated_at: at };
      const scoped = { ...ownedResource, cleanup: { ...cleanupItem, effects: [effect] } };
      expect(parseOrchestrationMutationResponse({ revision: 3, result: { result: "resource_registered", resource: scoped, duplicate: false } }))
        .toBeDefined();
      const publicItem = { resource_id: scoped.resource_id, kind: scoped.kind, ownership: scoped.ownership,
        cleanup: { ...scoped.cleanup, effects: [{ ...effect, evidence: null }] } };
      const publicDto = { ...snapshot, cleanup: [{ run_id: "run", items: [publicItem],
        removed_count: 0, waiting_count: 1, kept_count: 0, unknown_count: 0 }] };
      expect(parseOrchestrationSnapshot(publicDto).cleanup[0]!.items[0]!.cleanup.reason).toEqual(cleanupItem.reason);
      expect(() => parseOrchestrationSnapshot(replaceField(publicDto, ["cleanup", 0, "items", 0, "cleanup", "effects", 0, "evidence"],
        effect.evidence))).toThrow(CockpitClientError);
      expect(() => parseOrchestrationSnapshot(replaceField(publicDto, ["cleanup", 0, "items", 0, "identity"], ownedResource.identity)))
        .toThrow(CockpitClientError);
      expect(() => parseOrchestrationSnapshot(replaceField(publicDto, ["cleanup", 0, "items", 0, "cleanup", "effects", 0, "identity_digest"], "bad-digest")))
        .toThrow(CockpitClientError);
      expect(parseOrchestrationSnapshot(replaceField(publicDto, ["cleanup", 0, "items", 0, "cleanup", "effects", 0, "identity_digest"], hash)))
        .toBeDefined();
    }
  });

  it("decodes cleanup finish and durable-store/restoration identity without inferred native attachment proof", () => {
    for (const run_id of [null, "run"]) {
      const action = { action: "cleanup_finish", request: { operation_id: parentStepId, root_id: "root", run_id } };
      expect(parseOrchestrationAction(action)).toEqual(action);
    }
    for (const [field, value] of [
      [["state_store_id"], "session"], [["restoration", "reattached"], 2],
      [["restoration", "total_attachments"], Number.MAX_SAFE_INTEGER + 1],
      [["restoration", "cleanup_pending"], -1], [["restoration", "secret"], "private"],
    ] as const) expect(() => parseOrchestrationSnapshot(replaceField(snapshot, [...field], value))).toThrow(CockpitClientError);
    const disconnected = { ...snapshot, runs: [], restoration: { reattached: 0, total_attachments: 0, cleanup_pending: 0 } };
    expect(parseOrchestrationSnapshot(disconnected).state_store_id).toBe(snapshot.state_store_id);
    for (const field of ["state_store_id", "restoration", "cleanup"]) {
      expect(() => parseOrchestrationSnapshot(replaceField(disconnected, [field], undefined, true))).toThrow(CockpitClientError);
    }
  });

  it("keeps accepted source separate from integration and decodes all actual target mapping methods", () => {
    const accepted = { ...codeRun, stage: "closed", close_reason: "accepted" };
    expect(parseOrchestrationSnapshot({ ...snapshot, runs: [accepted] }).runs[0]!.integration).toEqual({ state: "pending" });
    for (const method of ["ancestor", "patch_equivalent", "recorded_adaptation"] as const) {
      const integration = { ...integrated, method, mappings: method === "ancestor" ? [] : integrated.mappings };
      const dto = { ...snapshot, runs: [{ ...accepted, integration }] };
      expect(parseOrchestrationSnapshot(dto).runs[0]!.integration).toEqual(integration);
      expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "integration", "checks", 0, "evidence"], null))
        .runs[0]!.integration).toEqual({ ...integration, checks: [{ ...focusedCheck, evidence: null }] });
      for (const field of ["source_output", "integration"]) {
        expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, field], undefined, true))).toThrow(CockpitClientError);
      }
    }
    const integration = { state: "not_applicable", reason: "Accepted design artifact", result_message_id: "result" };
    const nonCode = { ...accepted, source_output: nonCodeOutput, result: { ...codeRun.result, source_output: nonCodeOutput }, integration };
    expect(parseOrchestrationSnapshot({ ...snapshot, runs: [nonCode] }).runs[0]!.integration).toEqual(integration);
    expect(() => parseOrchestrationSnapshot({ ...snapshot, runs: [{ ...accepted, integration }] })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationSnapshot({ ...snapshot, runs: [{ ...nonCode, integration: integrated }] })).toThrow(CockpitClientError);
  });

  it("rejects wrong Result, scope, checkout and integration identities instead of accepting a merged flag", () => {
    const dto = { ...snapshot, runs: [{ ...codeRun, integration: integrated }] };
    for (const [field, value] of [
      [["source_output", "result_message_id"], "old-result"], [["source_output", "task_revision"], "e".repeat(64)],
      [["source_output", "repository_key"], "other-repository"], [["source_output", "checkout_key"], "other-checkout"],
      [["source_output", "base_commit"], headCommit], [["source_output", "head_commit"], targetCommit],
      [["result", "source_output"], { ...codeOutput, focused_checks: [{ ...focusedCheck, evidence: "Different source output" }] }],
      [["integration", "accepted_result_message_id"], "old-result"], [["integration", "accepted_task_revision"], "e".repeat(64)],
      [["integration", "source", "checkout_key"], "other-checkout"], [["integration", "source", "kind"], "non_code"],
      [["integration", "checks", 0, "outcome"], "failed"],
      [["integration", "checks", 0, "limitation"], "Not fully verified"],
      [["integration"], { merged: true }], [["integration"], { state: "pending", merged: true }],
    ] as const) expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, ...field], value)), field.join("."))
      .toThrow(CockpitClientError);
  });

  it("validates complete integration mappings and retains failed checks only in pending receipts", () => {
    const action = { action: "integration_record", record: integrationRecord };
    for (const [field, value] of [
      [["operation_id"], assignmentTaskId], [["expected_result_message_id"], "old-result"], [["expected_task_revision"], "e".repeat(64)],
      [["integration", "mappings"], []], [["integration", "mappings", 0, "source_commits"], []],
      [["integration", "mappings", 0, "source_commits"], [targetCommit]],
      [["integration", "mappings", 0, "target_commits"], [targetCommit, targetCommit]],
      [["integration", "mappings", 0, "source_paths"], ["/repo/file"]], [["integration", "mappings", 0, "target_paths"], ["../file"]],
      [["integration", "mappings", 0, "explanation"], null], [["integration", "mappings", 0, "explanation"], " "],
      [["integration", "target_ref"], "main"], [["integration", "target_commit"], "short-id"],
    ] as const) expect(() => parseOrchestrationAction({ ...action, record: replaceField(integrationRecord, [...field], value) }))
      .toThrow(CockpitClientError);
    const failed = { ...focusedCheck, outcome: "failed", evidence: `Failed at ${targetCommit}` };
    expect(parseOrchestrationAction({ ...action, record: { ...integrationRecord,
      integration: { ...integrated, checks: [failed] } } })).toBeDefined();
    const response = { revision: 3, result: { result: "integration_recorded", receipt: {
      operation_id: prerequisiteId, run_id: "run", integration: { state: "pending" }, checks: [failed], duplicate: false,
    } } };
    expect(parseOrchestrationMutationResponse(response)).toEqual(response);
    expect(() => parseOrchestrationMutationResponse(replaceField(response, ["result", "receipt", "integration"],
      { ...integrated, checks: [failed] }))).toThrow(CockpitClientError);
    expect(() => parseOrchestrationMutationResponse(replaceField(response, ["result", "receipt", "integration"],
      { ...integrated, operation_id: assignmentTaskId }))).toThrow(CockpitClientError);
  });

  it("requires Result-only source metadata, full Git IDs and explicit focused check outcomes", () => {
    const action = { action: "report", message_id: "result", kind: "result", outcome: "succeeded",
      summary: "Done", plan: null, source_output: codeOutput, to_run_id: null };
    expect(parseOrchestrationAction(action)).toEqual(action);
    expect(parseOrchestrationAction({ ...action, source_output: nonCodeOutput })).toBeDefined();
    expect(parseOrchestrationAction({ ...action, outcome: "failed", source_output: null })).toBeDefined();
    for (const patch of [
      { source_output: null }, { kind: "progress" }, { message_id: "old-result" },
      { source_output: { ...codeOutput, head_commit: "C".repeat(40) } },
      { source_output: { ...codeOutput, commits: [headCommit, headCommit] } },
      { source_output: { ...codeOutput, focused_checks: [focusedCheck, focusedCheck] } },
      { source_output: { ...codeOutput, focused_checks: [{ ...focusedCheck, outcome: "skipped" }] } },
      { source_output: { ...codeOutput, focused_checks: [{ ...focusedCheck, contract: " " }] } },
    ]) expect(() => parseOrchestrationAction({ ...action, ...patch })).toThrow(CockpitClientError);
    const sha256 = { ...codeOutput, base_commit: "1".repeat(64), head_commit: "2".repeat(64), commits: ["2".repeat(64)] };
    expect(parseOrchestrationAction({ ...action, source_output: sha256 })).toBeDefined();
    expect(parseOrchestrationAction({ ...action, source_output: { ...codeOutput, head_commit: baseCommit, commits: [] } })).toBeDefined();
    for (const outcome of ["passed", "failed", "not_run"]) {
      expect(parseOrchestrationAction({ ...action, source_output: { ...nonCodeOutput,
        focused_checks: [{ ...focusedCheck, outcome, evidence: null, limitation: "Not exercised" }] } })).toBeDefined();
    }
  });

  it("allows absent original provenance and validates retained revision and claim identities", () => {
    const reportAction = { action: "report", message_id: "result", kind: "result", outcome: "succeeded",
      summary: "Done", plan: null, source_output: codeOutput, to_run_id: null };
    const sendAction = { action: "message_send", message_id: "question", to_run_id: "root",
      kind: "observation", text: "Retained work update", in_reply_to: null };
    for (const action of [reportAction, sendAction]) {
      expect(parseOrchestrationAction(action)).toEqual(action);
      for (const provenance of [
        { original_task_revision: hash }, { original_claim_id: childStepId },
        { original_task_revision: hash, original_claim_id: childStepId },
      ]) expect(parseOrchestrationAction({ ...action, ...provenance })).toEqual({ ...action, ...provenance });
      for (const provenance of [
        { original_task_revision: "old-scope" }, { original_task_revision: "A".repeat(64) },
        { original_claim_id: "wrong-claim" }, { original_claim_id: "" }, { original_claim_id: 1 },
      ]) expect(() => parseOrchestrationAction({ ...action, ...provenance })).toThrow(CockpitClientError);
    }
  });

  it("decodes cleared SendBack output without mistaking stale Result events for current source", () => {
    expect(parseOrchestrationAction({ action: "send_back", run_id: "run", text: "Revise scope" })).toBeDefined();
    expect(parseOrchestrationAction(scopeUpdateAction)).toEqual(scopeUpdateAction);
    const revisedSource = { ...codeOutput, task_revision: "e".repeat(64), result_message_id: "revised-result" };
    const revisedRun = { ...codeRun, source_output: revisedSource,
      result: { ...codeRun.result, message_id: "revised-result", source_output: revisedSource } };
    const revisedIntegration = { ...integrated, source: revisedSource,
      accepted_result_message_id: revisedSource.result_message_id, accepted_task_revision: revisedSource.task_revision };
    const revised = parseOrchestrationSnapshot({ ...snapshot, runs: [{ ...revisedRun, integration: revisedIntegration }] });
    expect(revised.runs[0]!.task_revision_at_start).toBe(hash);
    expect(revised.runs[0]!.source_output!.task_revision).toBe(revisedSource.task_revision);
    expect(revised.runs[0]!.integration).toEqual(revisedIntegration);
    const oldResult = { ...message, stale: true, report: codeRun.result };
    const afterSendBack = { ...snapshot, messages: [oldResult], runs: [{ ...codeRun,
      stage: "working", result: null, source_output: null, integration: { state: "pending" } }] };
    expect(parseOrchestrationSnapshot(afterSendBack).messages[0]!.report).toEqual(codeRun.result);
    for (const source_output of [codeOutput, { ...codeOutput, task_revision: "e".repeat(64) }]) {
      expect(() => parseOrchestrationSnapshot(replaceField(afterSendBack, ["runs", 0, "source_output"], source_output)))
        .toThrow(CockpitClientError);
    }
    expect(() => parseOrchestrationSnapshot(replaceField(afterSendBack, ["runs", 0, "integration"], integrated))).toThrow(CockpitClientError);
  });

  it("decodes durable resource states and rejects unsafe sequence, source and holder proof", () => {
    for (const state of ["queued", "held", "released", "blocked"]) {
      const response = { revision: 3, result: { result: "resource", request: { ...resourceRecord, state,
        blocked_reason: state === "blocked" ? "Process exit is unknown" : null }, duplicate: false } };
      expect(parseOrchestrationMutationResponse(response)).toEqual(response);
    }
    const response = { revision: 3, result: { result: "resource", request: resourceRecord, duplicate: true } };
    for (const [field, value] of [
      [["sequence"], Number.MAX_SAFE_INTEGER + 1], [["request_id"], "not-a-uuid"], [["state"], "granted"],
      [["holder", "attachment_id"], "not-a-uuid"], [["holder", "attachment_epoch"], 0],
      [["source_identity", "source_commit"], "short"], [["source_identity", "source_tree"], "A".repeat(40)],
      [["source_identity", "input_configuration"], ""], [["machine_id"], ""],
      [["build_processes", 0, "process", "start_ticks"], Number.MAX_SAFE_INTEGER + 1],
      [["build_processes", 0, "argv_digest"], "bad"], [["updated_at"], "yesterday"],
    ] as const) expect(() => parseOrchestrationMutationResponse(replaceField(response, ["result", "request", ...field], value)))
      .toThrow(CockpitClientError);
    expect(parseOrchestrationMutationResponse(replaceField(response, ["result", "request", "sequence"], Number.MAX_SAFE_INTEGER))).toBeDefined();
  });

  it("rejects private capabilities and missing fields throughout public W3 proof summaries", () => {
    const response = { revision: 3, result: { result: "resource", request: resourceRecord, duplicate: false } };
    const source = { ...snapshot, runs: [{ ...codeRun, integration: integrated }] };
    for (const [dto, parse] of [
      [response, parseOrchestrationMutationResponse], [source, parseOrchestrationSnapshot],
      [{ action: "integration_record", record: integrationRecord }, parseOrchestrationAction],
      [{ action: "resource_request", request: resourceRequest }, parseOrchestrationAction],
    ] as const) {
      for (const field of fieldPaths(dto)) {
        if (Object.hasOwn(defaultedSnapshotFields, field.join("."))) {
          const expected = field.join(".") === "task_ownership.0.owner.conversation" ? replaceField(dto, field, null) : dto;
          expect(parse(replaceField(dto, field, undefined, true)), field.join(".")).toEqual(expected);
        } else {
          expect(() => parse(replaceField(dto, field, undefined, true)), field.join(".")).toThrow(CockpitClientError);
        }
      }
      for (const ownerPath of [[], ...fieldPaths(dto)]) {
        const owner = ownerPath.reduce<unknown>((value, key) => (value as Record<string | number, unknown>)[key], dto);
        if (owner === null || typeof owner !== "object" || Array.isArray(owner)) continue;
        for (const field of ["credential_id", "secret", "job_auth", "output_facts", "source_claim_id", "request_digest"]) {
          expect(() => parse(replaceField(dto, ownerPath, { ...owner, [field]: "private" }))).toThrow(CockpitClientError);
        }
      }
    }
  });

  it("requires explicit Answer correlation and forbids it on non-Answer sends", () => {
    const answer = { action: "message_send", message_id: "answer", to_run_id: "run", kind: "answer", text: "Use this checkout", in_reply_to: "question" };
    expect(parseOrchestrationAction(answer)).toEqual(answer);
    for (const link of [null, "", undefined, 1]) {
      expect(() => parseOrchestrationAction({ ...answer, in_reply_to: link })).toThrow(CockpitClientError);
    }
    for (const kind of ["instruction", "cancel_request", "report"]) {
      expect(() => parseOrchestrationAction({ ...answer, kind })).toThrow(CockpitClientError);
      expect(parseOrchestrationAction({ ...answer, kind, in_reply_to: null })).toMatchObject({ kind, in_reply_to: null });
    }
  });

  it("decodes strict question receipts and rejects contradictory or missing delivery facts", () => {
    const answer = { sender: { type: "operator" }, message_id: "answer", seq: 3, stage: "stored", created_at: at, acked_at: null };
    const question = { run_id: "run", question_message_id: "question", asked_at: at, receipt: { status: "unresolved" } };
    const receipts = [
      { status: "unresolved" },
      ...(["stored", "read"] as const).map(stage => ({ status: "answer_delivered", answer: { ...answer, stage } })),
      { status: "answer_acknowledged", answer: { ...answer, stage: "acked", acked_at: at } },
      { status: "answer_acknowledged", answer: { ...answer, stage: "acked" } },
    ];
    for (const receipt of receipts) {
      const dto = { ...snapshot, questions: [{ ...question, receipt }] };
      expect(parseOrchestrationSnapshot(dto).questions[0]!.receipt).toEqual(receipt);
      for (const path of fieldPaths(dto.questions[0])) {
        expect(() => parseOrchestrationSnapshot(replaceField(dto, ["questions", 0, ...path], undefined, true))).toThrow(CockpitClientError);
      }
    }
    for (const receipt of [
      { status: "unresolved", answer },
      { status: "answer_delivered", answer: { ...answer, stage: "acked" } },
      { status: "answer_acknowledged", answer },
      { status: "answer_delivered", answer: { ...answer, extra: true } },
      { status: "processed", answer },
    ]) expect(() => parseOrchestrationSnapshot({ ...snapshot, questions: [{ ...question, receipt }] })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationSnapshot({ ...snapshot, questions: [{ ...question, extra: true }] })).toThrow(CockpitClientError);
  });

  it("accepts populated DTOs and defaults only the newly optional saved fields", () => {
    expect(parseOrchestrationSnapshot(snapshot)).toEqual(snapshot);
    for (const path of fieldPaths(snapshot)) {
      if (Object.hasOwn(defaultedSnapshotFields, path.join("."))) {
        const expected = path.join(".") === "task_ownership.0.owner.conversation" ? replaceField(snapshot, path, null) : snapshot;
        expect(parseOrchestrationSnapshot(replaceField(snapshot, path, undefined, true)), path.join("."))
          .toEqual(expected);
      } else {
        expect(() => parseOrchestrationSnapshot(replaceField(snapshot, path, undefined, true)), path.join(".")).toThrow(CockpitClientError);
      }
      expect(() => parseOrchestrationSnapshot(replaceField(snapshot, path, {})), path.join(".")).toThrow(CockpitClientError);
    }
    expect(parseOrchestrationSnapshot({ ...snapshot, runtime: { status: "unavailable", error: { code: "offline", message: "Offline" } } }).runtime.status).toBe("unavailable");
    expect(() => parseOrchestrationSnapshot({ ...snapshot, runtime: { status: "unavailable", error: { code: "offline", message: 4 } } })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationSnapshot({ ...snapshot, messages: Array(1) })).toThrow(CockpitClientError);
  });

  it("requires explicit nullable current run revision and cleanup policy", () => {
    for (const field of ["cleanup_policy", "task_revision_at_start"]) {
      const nullable = replaceField(snapshot, ["runs", 0, field], null);
      expect(parseOrchestrationSnapshot(nullable), field).toBeDefined();
      expect(() => parseOrchestrationSnapshot(replaceField(nullable, ["runs", 0, field], undefined, true)), field)
        .toThrow(CockpitClientError);
    }
  });

  it("rejects removed current permission fields, stages, reports and attention", () => {
    const plan = { plan_revision: hash, text: "Old permission plan", created_at: at };
    const grants = [{ grant_id: "grant", scope: "prepare", plan_revision: hash, origin: "browser",
      supervisor_run_id: null, omp_session_id: null, granted_at: at }];
    for (const [field, value] of [
      ["prepare_brief", "Prepare"], ["task_revision_at_propose", hash],
      ["prepare_plan", plan], ["init_receipt", report], ["work_plan", plan], ["grants", grants],
      ["supersedes_run_id", "old-run"],
      ["history", { original_stage: "ready", prepare_plan: plan, init_receipt: report,
        work_plan: plan, grants, supersedes_run_id: "old-run" }],
    ] as const) {
      for (const oldValue of [value, null]) {
        expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, field], oldValue)), field).toThrow(CockpitClientError);
      }
    }
    for (const stage of ["proposed", "awaiting_prepare", "preparing", "initializing", "ready"]) {
      expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "stage"], stage)), stage).toThrow(CockpitClientError);
    }
    for (const lane of ["setup", "queued", "starting"]) {
      expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["board", "tasks", 0, "lane"], lane))).toThrow(CockpitClientError);
    }
    for (const kind of ["awaits_prepare", "awaits_execute"]) {
      expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["attention", 0, "kind"], kind))).toThrow(CockpitClientError);
    }
    for (const path of [["runs", 0, "last_report"], ["messages", 0, "report"]]) {
      expect(() => parseOrchestrationSnapshot(replaceField(snapshot, [...path, "kind"], "ready"))).toThrow(CockpitClientError);
    }
    expect(parseOrchestrationAction({ action: "report", message_id: "initialized", kind: "initialized",
      outcome: "succeeded", summary: "Checkout initialized", plan: null, source_output: null, to_run_id: null })).toBeDefined();
    expect(() => parseOrchestrationAction({ action: "report", message_id: "ready", kind: "ready",
      outcome: "succeeded", summary: "Ready", plan: null, source_output: null, to_run_id: null })).toThrow(CockpitClientError);
    for (const stage of ["queued", "starting", "working", "reported", "active", "closed"]) {
      expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "stage"], stage)).runs[0]!.stage).toBe(stage);
    }
    for (const lane of ["blocked", "ready", "working", "review", "accepted"]) {
      expect(parseOrchestrationSnapshot(replaceField(snapshot, ["board", "tasks", 0, "lane"], lane)).board!.tasks[0]!.lane).toBe(lane);
    }
  });

  it("requires one-start and scope-update identities, dual CAS fences and bounded public input", () => {
    for (const action of [workerStartAction, scopeUpdateAction]) {
      for (const field of ["operation_id", "root_id", "task_id", ...action.action === "task_scope_update" ? ["run_id"] : []]) {
        for (const value of ["", "not-a-uuid", "root", null]) {
          expect(() => parseOrchestrationAction({ ...action, [field]: value }), `${action.action}.${field}`).toThrow(CockpitClientError);
        }
      }
      for (const field of ["expected_task_revision", "expected_doc_revision"]) {
        for (const value of [null, "", "bad", "A".repeat(64)]) {
          expect(() => parseOrchestrationAction({ ...action, [field]: value })).toThrow(CockpitClientError);
        }
      }
      for (const field of ["credential_id", "secret", "secret_digest", "auth", "job_auth", "actor", "origin", "body"]) {
        expect(() => parseOrchestrationAction({ ...action, [field]: "private" })).toThrow(CockpitClientError);
      }
    }
    for (const cleanup_policy of [null, "always", "never", "owned", {}]) {
      expect(() => parseOrchestrationAction({ ...workerStartAction, cleanup_policy })).toThrow(CockpitClientError);
    }
    for (const target of [null, { target: "unknown" }, { target: "existing_space", workspace_id: "" }]) {
      expect(() => parseOrchestrationAction({ ...workerStartAction, target })).toThrow(CockpitClientError);
    }
    expect(parseOrchestrationAction({ ...workerStartAction, label: "é".repeat(128), brief: "é".repeat(8192) })).toBeDefined();
    for (const [field, value] of [["label", "é".repeat(129)], ["brief", "é".repeat(8193)], ["brief", "bad\0text"]] as const) {
      expect(() => parseOrchestrationAction({ ...workerStartAction, [field]: value })).toThrow(CockpitClientError);
    }
    for (const patch of [
      { title: "New title", description: null }, { title: null, description: "" },
      { title: "é".repeat(128), description: "é".repeat(8192) },
    ]) expect(parseOrchestrationAction({ ...scopeUpdateAction, ...patch })).toBeDefined();
    for (const patch of [
      { title: null, description: null }, { title: "é".repeat(129) }, { description: "é".repeat(8193) },
      { title: "bad\0title" }, { description: "bad\0description" },
    ]) expect(() => parseOrchestrationAction({ ...scopeUpdateAction, ...patch })).toThrow(CockpitClientError);
    expect(parseOrchestrationAction({ ...workerStartAction, label: null })).toBeDefined();
    expect(() => parseOrchestrationAction(replaceField(workerStartAction, ["label"], undefined, true))).toThrow(CockpitClientError);
    for (const field of ["title", "description"]) {
      const action = { ...scopeUpdateAction, title: "Revised title", description: "Revised description", [field]: null };
      expect(parseOrchestrationAction(action)).toEqual(action);
      expect(() => parseOrchestrationAction(replaceField(action, [field], undefined, true))).toThrow(CockpitClientError);
    }
  });

  it("rejects removed permission-era mutation actions instead of translating them", () => {
    for (const action of [
      { action: "run_propose", task_id: "task", parent_run_id: null, label: null, target, prepare_brief: "Prepare", supersedes_run_id: null },
      { action: "grant_prepare", run_id: "run", plan_revision: hash },
      { action: "grant_execute", run_id: "run", plan_revision: hash, note: null },
      { action: "run_bind_session", omp_session_id: "omp" },
      { action: "retry_launch", run_id: "run" },
    ]) {
      expect(() => parseOrchestrationAction(action), action.action).toThrow(CockpitClientError);
      expect(() => parseOrchestrationMutationRequest({ session_id: "session", expected_revision: 2, action })).toThrow(CockpitClientError);
    }
  });

  it("keeps converted unknown identities nullable without inventing connection evidence", () => {
    const converted: ExecutionAttachment = {
      ...attachment, state: "disconnected", launch_mode: null, location: null, conversation: null,
      process: null, shell: null, machine_id: null, launch_attempt: 0,
    };
    const run = { ...snapshot.runs[0]!, project: null, checkout: null, checkout_claim_id: null, resume: null, attachment: converted };
    const dto = { ...snapshot, runs: [run], runtime: { status: "unavailable", error: { code: "offline", message: "Offline" } } };
    expect(parseOrchestrationSnapshot(dto).runs[0]).toEqual(run);
    expect(parseOrchestrationSnapshot({ ...dto, runs: [{ ...run, attachment: null }] }).runs[0]!.attachment).toBeNull();
    for (const field of ["launch_mode", "location", "conversation", "process", "shell", "machine_id"] as const) {
      expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "attachment", field], null))
        .runs[0]!.attachment![field]).toBeNull();
    }
    for (const state of ["reserved", "connecting", "connected", "disconnected", "quiescing", "revoked"] as const) {
      expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "attachment", "state"], state))
        .runs[0]!.attachment!.state).toBe(state);
    }
    const directoryProject = { ...project, source: { kind: "directory", root: "/repo" }, repository_id: null, source_workspace_id: null };
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "project"], directoryProject))
      .runs[0]!.project).toEqual(directoryProject);
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "checkout", "git"], null))
      .runs[0]!.checkout!.git).toBeNull();
  });

  it("requires explicit nullable identity fields instead of filling missing saved evidence", () => {
    for (const path of [
      ["project"], ["checkout"], ["attachment"], ["checkout_claim_id"], ["resume"],
      ["project", "repository_id"], ["project", "source_workspace_id"], ["checkout", "git"],
      ["attachment", "location"], ["attachment", "conversation"], ["attachment", "process"],
      ["attachment", "shell"], ["attachment", "machine_id"], ["attachment", "launch_mode"],
      ["attachment", "location", "boot_id"], ["attachment", "location", "terminal_id"],
      ["attachment", "location", "native_session_id"], ["attachment", "process", "kernel_boot_id"],
      ["attachment", "shell", "process", "kernel_boot_id"],
      ["resume", "previous_attachment_epoch"], ["resume", "cause"], ["resume", "decision_id"],
      ["resume", "replacement_attachment_epoch"],
    ]) {
      const nullable = replaceField(snapshot, ["runs", 0, ...path], null);
      expect(parseOrchestrationSnapshot(nullable), path.join(".")).toBeDefined();
      expect(() => parseOrchestrationSnapshot(replaceField(nullable, ["runs", 0, ...path], undefined, true)), path.join("."))
        .toThrow(CockpitClientError);
    }
  });

  it("preserves saved attachment receipts independently from observed presence and pinned retirement", () => {
    const dto = { ...snapshot, runs: [{ ...snapshot.runs[0]!, retirement: retirementRecord({ state: "waiting", blockers: [] }) }] };
    const observed = parseOrchestrationSnapshot(dto);
    expect(observed.runs[0]!.attachment!.location!.pane_id).toBe("pane");
    expect(observed.runtime).toMatchObject({ status: "fresh", runs: [{ pane_id: "moved-pane" }] });
    for (const presence of ["missing", "endpoint_changed", "unobserved"] as const) {
      const changed = replaceField(replaceField(dto, ["runtime", "runs", 0, "presence"], presence),
        ["runtime", "runs", 0, "actual_agent"], false);
      expect(parseOrchestrationSnapshot(changed).runs[0]!.attachment).toEqual(attachment);
    }
    const replacement = { ...attachment, attachment_id: childStepId, epoch: 2, location: { ...attachment.location!, pane_id: "replacement" } };
    const changed = parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "attachment"], replacement));
    expect(changed.runs[0]!.attachment).toEqual(replacement);
    expect(changed.runs[0]!.retirement!.identity).toEqual(retirementIdentity);
    expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "resume", "run_id"], "foreign-run"))).toThrow(CockpitClientError);
  });

  it("rejects removed top-level physical mirrors even when null or consistent with the attachment", () => {
    for (const [field, value] of [
      ["location", attachment.location], ["bound_omp_session", attachment.conversation],
      ["bound_omp_process", attachment.process], ["launch_shell_identity", attachment.shell],
    ] as const) {
      for (const mirror of [value, null]) {
        expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, field], mirror)), field).toThrow(CockpitClientError);
      }
    }
  });

  it("rejects unknown fields and private credential material throughout public snapshot records", () => {
    for (const ownerPath of [[], ...fieldPaths(snapshot)]) {
      const owner = ownerPath.reduce<unknown>((value, key) => (value as Record<string | number, unknown>)[key], snapshot);
      if (owner === null || typeof owner !== "object" || Array.isArray(owner)) continue;
      for (const field of ["extra", "credential_id", "secret", "secret_digest", "auth", "job_auth", "credential_file"]) {
        const dto = replaceField(snapshot, ownerPath, { ...owner, [field]: "private" });
        expect(() => parseOrchestrationSnapshot(dto), `${ownerPath.join(".")}.${field}`).toThrow(CockpitClientError);
      }
    }
  });

  it("validates nested project, checkout, attachment and resume semantics", () => {
    for (const [path, bad] of [
      [["project", "key"], ""],
      [["project", "repository_id"], "bad\nid"], [["project", "source_workspace_id"], ""],
      [["project", "source", "kind"], "unknown"], [["project", "source", "common_dir"], "relative"],
      [["project", "source", "common_dir"], "/repo/../other"], [["project", "source", "common_dir"], "/repo/./.git"],
      [["project", "source", "common_dir"], `/${"é".repeat(2048)}`],
      [["checkout", "key"], ""],
      [["checkout", "path"], "repo"], [["checkout", "path"], "/repo\nother"],
      [["checkout", "directory_identity", "device"], "01"], [["checkout", "directory_identity", "inode"], "18446744073709551616"],
      [["checkout", "directory_identity", "device"], "-1"], [["checkout", "directory_identity", "inode"], ""],
      [["checkout", "git", "common_dir"], ""], [["checkout", "git", "worktree_git_dir"], "../repo"],
      [["attachment", "attachment_id"], "not-a-uuid"], [["attachment", "state"], "alive"],
      [["attachment", "conversation", "session_id"], ""], [["attachment", "conversation", "harness"], "bad\nid"], [["attachment", "machine_id"], "bad\nid"],
      [["attachment", "updated_at"], "yesterday"], [["attachment", "launch_attempt"], 0x100000000],
      [["attachment", "launch_attempt"], -1], [["attachment", "process", "pid"], 0x100000000],
      [["attachment", "process", "start_ticks"], Number.MAX_SAFE_INTEGER + 1],
      [["attachment", "location", "endpoint_identity"], ""], [["attachment", "location", "native_session_id"], ""],
      [["resume", "operation_id"], "not-a-uuid"], [["resume", "run_id"], ""],
      [["resume", "phase"], "ready"], [["resume", "decision_id"], "not-a-uuid"],
      [["resume", "phase"], "waiting_quiescence"],
      [["resume", "updated_at"], "yesterday"], [["resume", "cause", "code"], ""],
      [["resume", "cause", "message"], "é".repeat(8193)], [["resume", "cause", "message"], "bad\0cause"],
      [["resume", "cause", "class"], "warning"], [["checkout_claim_id"], "not-a-uuid"],
    ] as const) expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, ...path], bad)), path.join("."))
      .toThrow(CockpitClientError);
    for (const path of [
      ["attachment", "epoch"], ["resume", "previous_attachment_epoch"], ["resume", "replacement_attachment_epoch"],
    ]) {
      for (const bad of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1, Infinity, "1"]) {
        expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, ...path], bad))).toThrow(CockpitClientError);
      }
      expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, ...path], Number.MAX_SAFE_INTEGER))).toBeDefined();
    }
    for (const phase of ["reconciling", "reattached", "launching", "connected", "blocked"]) {
      expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "resume", "phase"], phase))
        .runs[0]!.resume!.phase).toBe(phase);
    }
    const decision = { ...resume, decision_id: childStepId, cause: { ...resume.cause!, class: "decision" }, replacement_attachment_epoch: 2 };
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "resume"], decision)).runs[0]!.resume).toEqual(decision);
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "resume", "cause"], null)).runs[0]!.resume!.cause).toBeNull();
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "checkout", "directory_identity"], { device: "0", inode: "9007199254740993" }))).toBeDefined();
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "checkout", "path"], `/${"é".repeat(2047)}a`))).toBeDefined();
    expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "project", "source"], { kind: "directory", root: "../repo" }))).toThrow(CockpitClientError);
  });

  it("validates wave-one action identities and epochs without accepting caller auth", () => {
    const waveOne = actions.filter(value => ["supervisor_start", "run_resume", "run_attach", "run_register", "task_unassign"].includes(value.action));
    for (const action of waveOne) {
      for (const field of ["operation_id", ...action.action === "run_attach" ? ["attachment_id"] : []]) {
        for (const value of ["", "not-a-uuid", "é".repeat(257)]) {
          expect(() => parseOrchestrationAction({ ...action, [field]: value })).toThrow(CockpitClientError);
        }
      }
      for (const field of ["credential_id", "secret", "secret_digest", "auth", "job_auth", "attachment_epoch"]) {
        expect(() => parseOrchestrationAction({ ...action, [field]: "private" })).toThrow(CockpitClientError);
      }
      if (action.action === "run_resume" || action.action === "run_attach") {
        for (const expected_attachment_epoch of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1, "1"]) {
          expect(() => parseOrchestrationAction({ ...action, expected_attachment_epoch })).toThrow(CockpitClientError);
        }
        expect(parseOrchestrationAction({ ...action, expected_attachment_epoch: Number.MAX_SAFE_INTEGER })).toBeDefined();
      }
    }
    expect(parseOrchestrationAction({ action: "run_resume", run_id: "run", operation_id: assignmentTaskId,
      expected_attachment_epoch: null })).toBeDefined();
    expect(parseOrchestrationAction({ ...startAction, mode: "another", selected_root_id: "root", expected_project_key: project.key })).toBeDefined();
    for (const invalid of [
      { ...startAction, mode: "unknown" }, { ...startAction, selected_root_id: "" },
      { ...startAction, expected_project_key: "" }, { ...startAction, expected_project_key: "é".repeat(257) },
      { action: "run_resume", run_id: "run", operation_id: assignmentTaskId, expected_attachment_epoch: null, mode: "unknown", decision_id: null },
      { action: "run_resume", run_id: "", operation_id: assignmentTaskId, expected_attachment_epoch: null },
      { action: "run_resume", run_id: "run", operation_id: assignmentTaskId, expected_attachment_epoch: null, mode: "replace_existing", decision_id: "bad" },
      { action: "run_attach", operation_id: assignmentTaskId, attachment_id: prerequisiteId, expected_attachment_epoch: 1, conversation: { harness: "omp", session_id: "" } },
    ]) expect(() => parseOrchestrationAction(invalid)).toThrow(CockpitClientError);
  });

  it("decodes strict project lookup records with honest unavailable and disconnected states", () => {
    const request = { session_id: "session", workspace_id: "space", selected_root_id: assignmentTaskId };
    const resolved = { status: "resolved", project, lookup_revision: 2, preferred_root_id: assignmentTaskId,
      roots: [{ root_id: assignmentTaskId, connection: "disconnected", resume: "blocked", cause: resume.cause, updated_at: at }] };
    const unavailable = { status: "unavailable", code: "project_unavailable", message: "Directory identity could not be verified" };
    expect(parseProjectRootLookupRequest(request)).toEqual(request);
    expect(parseProjectRootLookupRequest({ ...request, selected_root_id: null })).toBeDefined();
    for (const response of [resolved, unavailable]) {
      expect(parseProjectRootLookupResponse(response)).toEqual(response);
      for (const path of fieldPaths(response)) {
        expect(() => parseProjectRootLookupResponse(replaceField(response, path, undefined, true))).toThrow(CockpitClientError);
        expect(() => parseProjectRootLookupResponse(replaceField(response, path, {}))).toThrow(CockpitClientError);
      }
      for (const ownerPath of [[], ...fieldPaths(response)]) {
        const owner = ownerPath.reduce<unknown>((value, key) => (value as Record<string | number, unknown>)[key], response);
        if (owner === null || typeof owner !== "object" || Array.isArray(owner)) continue;
        expect(() => parseProjectRootLookupResponse(replaceField(response, ownerPath, { ...owner, auth: "private" }))).toThrow(CockpitClientError);
      }
    }
    for (const path of fieldPaths(request)) {
      expect(() => parseProjectRootLookupRequest(replaceField(request, path, undefined, true))).toThrow(CockpitClientError);
    }
    for (const invalid of [{ ...request, session_id: "../session" }, { ...request, workspace_id: "" },
      { ...request, selected_root_id: "" }, { ...request, selected_root_id: "root" },
      { ...request, selected_root_id: "not-a-uuid" }, { ...request, auth: "private" }]) {
      expect(() => parseProjectRootLookupRequest(invalid)).toThrow(CockpitClientError);
    }
    for (const [path, value] of [
      [["lookup_revision"], Number.MAX_SAFE_INTEGER + 1], [["lookup_revision"], -1],
      [["preferred_root_id"], ""], [["roots", 0, "root_id"], ""], [["roots", 0, "connection"], "live"],
      [["preferred_root_id"], "root"], [["preferred_root_id"], prerequisiteId],
      [["roots", 0, "root_id"], "root"], [["roots", 0, "root_id"], "not-a-uuid"],
      [["roots", 0, "resume"], "retry_anyway"], [["roots", 0, "updated_at"], "yesterday"],
      [["roots", 0, "cause", "code"], ""], [["roots", 0, "cause", "message"], "é".repeat(8193)],
    ] as const) expect(() => parseProjectRootLookupResponse(replaceField(resolved, [...path], value))).toThrow(CockpitClientError);
    for (const connection of ["connected", "reconnecting", "disconnected", "unknown"]) {
      for (const availability of ["available", "in_progress", "blocked"]) {
        expect(parseProjectRootLookupResponse({ ...resolved, preferred_root_id: null,
          roots: [{ ...resolved.roots[0]!, connection, resume: availability, cause: null }] })).toBeDefined();
      }
    }
    expect(parseProjectRootLookupResponse({ ...resolved, roots: [], preferred_root_id: null, lookup_revision: Number.MAX_SAFE_INTEGER })).toBeDefined();
    expect(() => parseProjectRootLookupResponse({ ...resolved, roots: [] })).toThrow(CockpitClientError);
    expect(() => parseProjectRootLookupResponse({ ...resolved, roots: [resolved.roots[0]!, resolved.roots[0]!] })).toThrow(CockpitClientError);
    expect(() => parseProjectRootLookupResponse({ ...resolved, preferred_root_id: null,
      roots: [resolved.roots[0]!, { ...resolved.roots[0]!, connection: "connected" }] })).toThrow(CockpitClientError);
    const second = { ...resolved.roots[0]!, root_id: prerequisiteId };
    expect(parseProjectRootLookupResponse({ ...resolved, roots: [resolved.roots[0]!, second], preferred_root_id: prerequisiteId })).toBeDefined();
    expect(() => parseProjectRootLookupResponse({ ...unavailable, code: "" })).toThrow(CockpitClientError);
    expect(() => parseProjectRootLookupResponse({ ...unavailable, message: "bad\0cause" })).toThrow(CockpitClientError);
  });

  it("preserves canonical body, editable prose, raw checks and effective checklist/dependency facts", () => {
    const view = parseOrchestrationSnapshot(snapshot).board!.tasks[0]!;
    expect(view.task.body).toBe(task.body);
    expect(view.task.description).toBe("Body");
    expect(view.task.steps[0]).toMatchObject({ checked: true, status: "partial" });
    expect(view.task.steps[1]).toMatchObject({ parent_step_id: parentStepId, source_offset: 108, status: "open" });
    expect(view.task.step_progress).toEqual({ done: 0, total: 1 });
    expect(view.task.follow_up_of).toBe(assignmentTaskId);
    // A successful Result is not acceptance and cannot replace the native dependency projection.
    expect(view.dependencies).toEqual(dependencies);
    const response = parseOrchestrationMutationResponse({ revision: 3, result: { result: "task", task } });
    expect(response.result).toEqual({ result: "task", task });
  });

  it("keeps diagnosed external checklists visible without applying authoring limits to source reads", () => {
    const external: Task = {
      ...task, description_editable: false, description_diagnostic: "description_unsafe",
      steps: [{ ...task.steps[0]!, step_id: null, parent_step_id: null, depth: 5,
        title: "é".repeat(201), diagnostic: "step_depth" }],
      step_progress: null, steps_diagnostic: "steps_invalid", relations_diagnostic: "relations_invalid",
    };
    const dto = replaceField(snapshot, ["board", "tasks", 0, "task"], external);
    expect(parseOrchestrationSnapshot(dto).board!.tasks[0]!.task).toEqual(external);
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["board", "tasks", 0, "task", "steps"], []))
      .board!.tasks[0]!.task.steps).toEqual([]);
    for (const status of ["open", "partial", "done"]) {
      expect(parseOrchestrationSnapshot(replaceField(snapshot, ["board", "tasks", 0, "task", "steps", 0, "status"], status))
        .board!.tasks[0]!.task.steps[0]!.status).toBe(status);
    }
  });

  it("decodes each dependency state and blocker reason without deriving it from lane or Result", () => {
    for (const state of ["none", "satisfied", "blocked", "invalid"] as const) {
      const facts: TaskDependencies = state === "none" || state === "satisfied"
        ? { state, unmet: [], problems: [] }
        : { state, unmet: [
          { task_id: prerequisiteId, reason: "unchecked" },
          { task_id: "deleted", reason: "missing" },
          { task_id: "duplicate", reason: "ambiguous" },
        ], problems: state === "invalid" ? [{ code: "dependency_cycle", message: "Cycle" }] : [] };
      const dto = replaceField(snapshot, ["board", "tasks", 0, "dependencies"], facts);
      expect(parseOrchestrationSnapshot(dto).board!.tasks[0]!.dependencies).toEqual(facts);
    }
  });

  it("rejects malformed checklist, progress, relation and binding facts", () => {
    const taskPath = ["board", "tasks", 0, "task"] as const;
    for (const [path, bad] of [
      [["description"], null], [["description_editable"], "true"], [["description_diagnostic"], false],
      [["steps"], Array(1)], [["steps", 0, "step_id"], ""], [["steps", 0, "parent_step_id"], 4],
      [["steps", 0, "depth"], -1], [["steps", 0, "depth"], 0x100000000],
      [["steps", 0, "title"], null], [["steps", 0, "checked"], 1], [["steps", 0, "status"], "complete"],
      [["steps", 0, "line"], 1.5], [["steps", 0, "source_offset"], "108"],
      [["steps", 0, "source_offset"], 0x100000000], [["steps", 0, "diagnostic"], {}],
      [["step_progress"], { done: 2, total: 1 }], [["step_progress"], { done: -1, total: 1 }],
      [["step_progress"], { done: 0, total: 0x100000000 }], [["steps_diagnostic"], []],
      [["depends_on"], null], [["depends_on"], [""]], [["follow_up_of"], 1], [["relations_diagnostic"], {}],
    ] as const) {
      expect(() => parseOrchestrationSnapshot(replaceField(snapshot, [...taskPath, ...path], bad)), path.join("."))
        .toThrow(CockpitClientError);
      expect(() => parseOrchestrationMutationResponse({
        revision: 3, result: { result: "task", task: replaceField(task, [...path], bad) },
      }), path.join(".")).toThrow(CockpitClientError);
    }
    for (const facts of [
      { ...dependencies, state: "ready" }, { ...dependencies, unmet: [{ task_id: prerequisiteId, reason: "done" }] },
      { ...dependencies, unmet: [{ task_id: "", reason: "missing" }] },
      { ...dependencies, problems: [{ code: "dependency_cycle", message: 4 }] },
    ]) expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["board", "tasks", 0, "dependencies"], facts)))
      .toThrow(CockpitClientError);
  });

  it("requires stable creation identity and revision fences and rejects writable canonical body aliases", () => {
    const create = { action: "task_create", root_id: "root", task_id: assignmentTaskId, title: "Title",
      description: "Prose", depends_on: [], follow_up_of: null, expected_doc_revision: null, source_revision: null };
    expect(parseOrchestrationAction(create)).toEqual(create);
    for (const task_id of ["", "task", "not-a-uuid"]) expect(() => parseOrchestrationAction({ ...create, task_id }))
      .toThrow(CockpitClientError);
    for (const invalid of [
      { ...create, depends_on: ["invalid"] }, { ...create, follow_up_of: "invalid" },
      { ...create, expected_doc_revision: "bad" }, { ...create, source_revision: "bad" },
      { ...create, depends_on: Array(33).fill(prerequisiteId) },
    ]) expect(() => parseOrchestrationAction(invalid)).toThrow(CockpitClientError);
    expect(parseOrchestrationAction({ ...create, depends_on: Array(32).fill(prerequisiteId) })).toBeDefined();
    for (const action of actions.filter(action => ["task_create", "task_assign", "task_update"].includes(action.action))) {
      expect(() => parseOrchestrationAction({ ...action, body: "Raw canonical replacement" })).toThrow(CockpitClientError);
      expect(() => parseOrchestrationAction(replaceField(action, ["description"], undefined, true))).toThrow(CockpitClientError);
    }
    const set = { action: "task_dependencies_set", root_id: "root", task_id: "task",
      expected_task_revision: hash, expected_doc_revision: hash, depends_on: [] };
    expect(parseOrchestrationAction(set)).toEqual(set);
    expect(() => parseOrchestrationAction({ ...set, depends_on: Array(33).fill(prerequisiteId) })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationAction({ ...set, expected_doc_revision: null })).toThrow(CockpitClientError);
  });

  it("validates Unicode-scalar step titles, stable identities and explicit check scope", () => {
    const add = { action: "task_step_add", root_id: "root", task_id: "task", expected_task_revision: hash,
      step_id: childStepId, parent_step_id: null, before_step_id: null, title: "😀".repeat(200) };
    expect(parseOrchestrationAction(add)).toEqual(add);
    expect(parseOrchestrationAction({ ...add, title: "é".repeat(200) })).toBeDefined();
    for (const title of ["😀".repeat(201), "é".repeat(201), "", " \t ", "line\nbreak", "bad\0title", "\ud800"]) {
      expect(() => parseOrchestrationAction({ ...add, title })).toThrow(CockpitClientError);
      expect(() => parseOrchestrationAction({ action: "task_step_rename", root_id: "root", task_id: "task",
        expected_task_revision: hash, step_id: childStepId, title })).toThrow(CockpitClientError);
    }
    for (const field of ["step_id", "parent_step_id", "before_step_id"]) {
      expect(() => parseOrchestrationAction({ ...add, [field]: "invalid" })).toThrow(CockpitClientError);
    }
    const checked = { action: "task_step_set_checked", root_id: "root", task_id: "task",
      expected_task_revision: hash, step_id: parentStepId, checked: true, scope: "subtree" };
    expect(parseOrchestrationAction(checked)).toEqual(checked);
    expect(parseOrchestrationAction({ ...checked, scope: "leaf" })).toBeDefined();
    for (const scope of [null, "all", "children"]) expect(() => parseOrchestrationAction({ ...checked, scope })).toThrow(CockpitClientError);
  });

  it("decodes explicit project-bound worktree targets and rejects malformed or unknown targets", () => {
    const worktree = { target: "space_worktree", workspace_id: "project-space", branch: "feature", base_ref: "main" };
    const dto = replaceField(snapshot, ["runs", 0, "target"], worktree);
    const bound = replaceField(dto, ["runs", 0, "setup", "project_workspace_id"], "project-space");
    expect(parseOrchestrationSnapshot(bound).runs[0]!.target).toEqual(worktree);
    expect(parseOrchestrationSnapshot(bound).runs[0]!.setup!.project_workspace_id).toBe("project-space");
    expect(parseOrchestrationAction({
      ...workerStartAction, target: { ...worktree, branch: null, base_ref: null },
    })).toMatchObject({ target: { target: "space_worktree", workspace_id: "project-space" } });
    for (const invalid of [
      { ...worktree, target: "unknown" }, { ...worktree, workspace_id: "" },
      { ...worktree, branch: 1 }, { ...worktree, base_ref: {} }, { ...worktree, extra: true },
      { target: "space_worktree", workspace_id: "project-space", branch: null },
    ]) expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "target"], invalid))).toThrow(CockpitClientError);
  });

  it("decodes native peer identity receipts without inventing legacy boot evidence", () => {
    const peer: NativeProcessIdentity = { ...processIdentity, pid: 314, start_ticks: 2718 };
    const location = { ...attachment.location!, boot_id: null, peer_process: peer };
    const identity = { ...retirementIdentity, herdr_boot_id: null, herdr_peer: peer };
    const retirement = retirementRecord({ state: "waiting", blockers: [] }, identity);
    const run = { ...snapshot.runs[0]!, attachment: { ...attachment, location }, retirement };
    const dto = { ...snapshot, runs: [run] };
    const decoded = parseOrchestrationSnapshot(dto).runs[0]!;
    expect(decoded.attachment!.location!.peer_process).toEqual(peer);
    expect(decoded.attachment!.location!.boot_id).toBeNull();
    expect(decoded.retirement!.identity!.herdr_peer).toEqual(peer);
    expect(decoded.retirement!.identity!.herdr_boot_id).toBeNull();
    const action = { action: "run_adopt", label: "Adopted", location };
    expect(parseOrchestrationAction(action)).toEqual(action);
    const withoutLocationPeer = replaceField(dto, ["runs", 0, "attachment", "location", "peer_process"], undefined, true);
    const legacy = replaceField(withoutLocationPeer, ["runs", 0, "retirement", "identity", "herdr_peer"], undefined, true);
    const restored = parseOrchestrationSnapshot(legacy).runs[0]!;
    expect(restored.attachment!.location!.peer_process).toBeNull();
    expect(restored.retirement!.identity!.herdr_peer).toBeNull();
    for (const path of [["attachment", "location", "peer_process"], ["retirement", "identity", "herdr_peer"]]) {
      for (const field of fieldPaths(peer)) {
        expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, ...path, ...field], undefined, true)))
          .toThrow(CockpitClientError);
      }
      for (const invalid of [{ ...peer, pid: -1 }, { ...peer, start_ticks: 1.5 }, { ...peer, kernel_boot_id: "" },
        { ...peer, invented_boot: "unsupported" }]) {
        expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, ...path], invalid))).toThrow(CockpitClientError);
      }
    }
  });

  it("decodes replacement retirement authority without inventing accepted Result pins", () => {
    const replacement: ReplacementRetirementAuthority = { operation_id: prerequisiteId, previous_run_id: "run",
      previous_generation: 1, replacement_run_id: childStepId };
    const retirement: RunRetirement = { ...retirementRecord({ state: "waiting", blockers: [] }),
      trigger: "replacement", result_message_id: null, task_revision: null, replacement };
    const dto = replaceField(snapshot, ["runs", 0, "retirement"], retirement);
    expect(parseOrchestrationSnapshot(dto).runs[0]!.retirement).toEqual(retirement);
    for (const field of ["result_message_id", "task_revision"] as const) {
      expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", field], undefined, true))
        .runs[0]!.retirement![field]).toBeNull();
    }
    for (const path of fieldPaths(replacement)) {
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", "replacement", ...path], undefined, true)))
        .toThrow(CockpitClientError);
    }
    for (const [path, invalid] of [
      [["replacement"], null], [["replacement", "operation_id"], "not-a-uuid"],
      [["replacement", "previous_run_id"], ""], [["replacement", "replacement_run_id"], ""],
      [["replacement", "previous_generation"], -1], [["replacement", "previous_generation"], Number.MAX_SAFE_INTEGER + 1],
      [["replacement"], { ...replacement, secret: "private" }],
      [["result_message_id"], "fabricated-result"], [["task_revision"], hash], [["trigger"], "future_retirement"],
    ] as const) {
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", ...path], invalid)))
        .toThrow(CockpitClientError);
    }
    const accepted = retirementRecord({ state: "waiting", blockers: [] });
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "retirement"],
      { ...accepted, replacement: undefined })).runs[0]!.retirement).toEqual(accepted);
    for (const trigger of ["accept", "accept_recovery", "operator_conflict_resolution"] as const) {
      for (const changed of [{ result_message_id: null }, { task_revision: null }, { replacement }]) {
        expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "retirement"], { ...accepted, trigger, ...changed })))
          .toThrow(CockpitClientError);
      }
    }
  });

  it.each(retirementStates)("decodes strict retirement state $state", state => {
    const retirement = retirementRecord(state);
    const dto = replaceField(snapshot, ["runs", 0, "retirement"], retirement);
    expect(parseOrchestrationSnapshot(dto).runs[0]!.retirement).toEqual(retirement);
    for (const path of fieldPaths(retirement)) {
      if (path.join(".") === "replacement" || path.join(".") === "identity.herdr_peer") {
        expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", ...path], undefined, true))
          .runs[0]!.retirement).toEqual(retirement);
      } else {
        expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", ...path], undefined, true)), path.join("."))
          .toThrow(CockpitClientError);
      }
    }
    for (const path of [[], ["identity"], ["identity", "process"], ["state"]]) {
      const owner = path.reduce<unknown>((value, key) => (value as Record<string, unknown>)[key], retirement);
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", ...path], {
        ...(owner as Record<string, unknown>), extra: true,
      }))).toThrow(CockpitClientError);
    }
    expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", "state", "state"], "unknown_state")))
      .toThrow(CockpitClientError);
    if (state.state === "waiting") {
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", "state", "blockers"],
        ["future_resource_admission"]))).toThrow(CockpitClientError);
    }
  });

  it("permits null retirement identity only for identity-incomplete retention", () => {
    for (const state of retirementStates) {
      const retirement = retirementRecord(state, null);
      const dto = replaceField(snapshot, ["runs", 0, "retirement"], retirement);
      if (state.state === "retained" && state.reason === "identity_incomplete") {
        expect(parseOrchestrationSnapshot(dto).runs[0]!.retirement).toEqual(retirement);
      } else {
        expect(() => parseOrchestrationSnapshot(dto), state.state).toThrow(CockpitClientError);
      }
    }
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "attachment", "process"], null)).runs[0]!.attachment!.process).toBeNull();
  });

  it("preserves explicit launch mode without backfilling unknown retirement identity", () => {
    const retirement = retirementRecord({ state: "retained", at, reason: "user_activity", stopped: false });
    const dto = replaceField(snapshot, ["runs", 0, "retirement"], retirement);
    for (const launch_mode of ["direct_process", "shell_agent", null]) {
      expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "attachment", "launch_mode"], launch_mode))
        .runs[0]!.attachment!.launch_mode).toBe(launch_mode);
      expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", "identity", "launch_mode"], launch_mode))
        .runs[0]!.retirement!.identity!.launch_mode).toBe(launch_mode);
    }
    const converted = { ...retirement, trigger: "accept_recovery",
      identity: { ...retirementIdentity, attachment_id: null, attachment_epoch: null, launch_mode: null } };
    expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement"], converted))
      .runs[0]!.retirement).toEqual(converted);
    for (const path of [["identity", "attachment_id"], ["identity", "attachment_epoch"], ["identity", "launch_mode"], ["identity", "conversation"], ["identity", "process"], ["identity", "shell"]]) {
      const nullable = replaceField(dto, ["runs", 0, "retirement", ...path], null);
      expect(parseOrchestrationSnapshot(nullable)).toBeDefined();
      expect(() => parseOrchestrationSnapshot(replaceField(nullable, ["runs", 0, "retirement", ...path], undefined, true)))
        .toThrow(CockpitClientError);
    }
    for (const launch_mode of ["unknown", "shell", "", 1, {}]) {
      expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "attachment", "launch_mode"], launch_mode)))
        .toThrow(CockpitClientError);
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", "identity", "launch_mode"], launch_mode)))
        .toThrow(CockpitClientError);
    }
    for (const [path, value] of [
      [["identity", "attachment_id"], "not-a-uuid"], [["identity", "attachment_epoch"], 0],
      [["identity", "attachment_epoch"], -1], [["identity", "attachment_epoch"], 1.5],
      [["identity", "attachment_epoch"], Number.MAX_SAFE_INTEGER + 1],
      [["identity", "attachment_epoch"], "1"],
    ] as const) expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", ...path], value)))
      .toThrow(CockpitClientError);
    expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", "identity", "attachment_epoch"], Number.MAX_SAFE_INTEGER)))
      .toBeDefined();
  });

  it("validates retirement identity counters, timestamps and tagged enum members", () => {
    const retirement = retirementRecord({ state: "retained", at, reason: "user_activity", stopped: false });
    const dto = replaceField(snapshot, ["runs", 0, "retirement"], retirement);
    for (const [path, bad] of [
      [["retirement_id"], "not-a-uuid"], [["trigger"], "unknown"], [["task_revision"], "bad"],
      [["trigger"], "resume_handover"],
      [["identity", "run_attempt"], 0x100000000], [["identity", "launch_attempt"], -1],
      [["identity", "process", "pid"], -1], [["identity", "process", "pid"], 0x100000000],
      [["identity", "process", "start_ticks"], Number.MAX_SAFE_INTEGER + 1],
      [["identity", "process", "start_ticks"], -1], [["identity", "process", "start_ticks"], 1.5],
      [["identity", "process", "start_ticks"], "1234"],
      [["created_at"], "yesterday"], [["updated_at"], ""],
      [["state", "at"], null], [["state", "reason"], "unknown"], [["state", "stopped"], "yes"],
    ] as const) {
      expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", ...path], bad)), path.join("."))
        .toThrow(CockpitClientError);
    }
    for (const state of [
      { state: "waiting", blockers: ["unknown"] },
      { state: "retired", at, terminal: "unknown" }, { state: "retained", at, reason: "unknown", stopped: true },
      { state: "unknown", at, phase: "unknown", detail: "Unconfirmed" },
    ]) expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", "state"], state))).toThrow(CockpitClientError);
    const maxProcess = { pid: 0xffffffff, start_ticks: Number.MAX_SAFE_INTEGER, kernel_boot_id: null };
    expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", "identity", "process"], maxProcess))).toBeDefined();
    for (const trigger of ["accept", "accept_recovery", "operator_conflict_resolution"]) {
      expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", "trigger"], trigger))).toBeDefined();
    }
    expect(parseOrchestrationSnapshot(replaceField(dto, ["attention", 0, "kind"], "retirement_unconfirmed")).attention[0]!.kind)
      .toBe("retirement_unconfirmed");
  });

  it("requires complete strict shell proof rather than shell PID alone", () => {
    const dto = replaceField(snapshot, ["runs", 0, "retirement"], retirementRecord({ state: "waiting", blockers: [] }));
    for (const ownerPath of [
      ["runs", 0, "attachment", "shell"],
      ["runs", 0, "retirement", "identity", "shell"],
    ] as const) {
      for (const invalid of [
        { ...shellIdentity, executable_device: "01" },
        { ...shellIdentity, executable_inode: "18446744073709551616" },
        { ...shellIdentity, executable_device: -1 },
        { ...shellIdentity, executable_inode: " 42" },
        { ...shellIdentity, executable_device: "+42" },
        { ...shellIdentity, executable_inode: "" },
        { ...shellIdentity, argv_digest: "A".repeat(64) },
        { ...shellIdentity, argv_digest: "not-a-digest" },
        { ...shellIdentity, process: { ...shellIdentity.process, start_ticks: -1 } },
        { ...shellIdentity, process: { ...shellIdentity.process, pid: -1 } },
        { ...shellIdentity, extra: true },
        { ...shellIdentity, process: { ...shellIdentity.process, extra: true } },
        { process: shellIdentity.process },
      ]) expect(() => parseOrchestrationSnapshot(replaceField(dto, [...ownerPath], invalid))).toThrow(CockpitClientError);
      expect(parseOrchestrationSnapshot(replaceField(dto, [...ownerPath], {
        ...shellIdentity, executable_device: "0", executable_inode: "9007199254740993",
      }))).toBeDefined();
    }
    expect(parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", "identity", "shell"], null))
      .runs[0]!.retirement!.identity!.shell).toBeNull();
    expect(() => parseOrchestrationSnapshot(replaceField(dto, ["runs", 0, "retirement", "identity", "shell"], undefined, true)))
      .toThrow(CockpitClientError);
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "attachment", "shell"], null))
      .runs[0]!.attachment!.shell).toBeNull();
  });


  it.each([
    ["roots", 0, "kind"], ["board", "tasks", 0, "lane"], ["runs", 0, "kind"], ["runs", 0, "stage"],
    ["runs", 0, "close_reason"], ["runs", 0, "dispatch", "step"], ["runs", 0, "dispatch", "recovery"],
    ["runs", 0, "target", "target"], ["runs", 0, "setup", "ownership"],
    ["runs", 0, "cleanup_policy"], ["runs", 0, "annotations", 0, "by", "type"],
    ["messages", 0, "from", "type"], ["messages", 0, "kind"], ["messages", 0, "stage"],
    ["messages", 0, "report", "kind"], ["messages", 0, "report", "outcome"],
    ["intents", 0, "state"], ["runtime", "status"], ["runtime", "runs", 0, "presence"], ["attention", 0, "kind"],
    ["intents", 0, "origin"], ["assignment_intents", 0, "state"],
  ])("rejects unknown nested enum at %s", (...path) => {
    expect(() => parseOrchestrationSnapshot(replaceField(snapshot, path, "unknown"))).toThrow(CockpitClientError);
  });

  it("decodes truthful current TaskIntent decision provenance", () => {
    for (const origin of ["browser", "native", "supervisor"] as const) {
      const provenance = {
        origin, supervisor_run_id: origin === "supervisor" ? "root" : null,
        conversation: origin === "supervisor" ? attachment.conversation : null,
      };
      const intent = { ...snapshot.intents[0]!, ...provenance, result_message_id: "exact-result" };
      const dto = { ...snapshot, intents: [intent] };
      expect(parseOrchestrationSnapshot(dto)).toEqual(dto);
    }
    expect(parseOrchestrationSnapshot(snapshot).intents[0]!.origin).toBeNull();
  });

  it("rejects incomplete, foreign-root or fabricated supervisor provenance", () => {
    const badProvenances = [
      { origin: "supervisor", supervisor_run_id: null, conversation: null },
      { origin: "supervisor", supervisor_run_id: "root", conversation: null },
      { origin: "supervisor", supervisor_run_id: null, conversation: attachment.conversation },
      { origin: "supervisor", supervisor_run_id: "other-root", conversation: attachment.conversation },
      { origin: "supervisor", supervisor_run_id: "root", conversation: { harness: "omp", session_id: "" } },
      { origin: "browser", supervisor_run_id: "root", conversation: attachment.conversation },
      { origin: "native", supervisor_run_id: null, conversation: attachment.conversation },
      { origin: "native", supervisor_run_id: "root", conversation: null },
      { origin: "unknown", supervisor_run_id: null, conversation: null },
    ];
    for (const provenance of badProvenances) {
      const intent = { ...snapshot.intents[0]!, ...provenance };
      expect(() => parseOrchestrationSnapshot({ ...snapshot, intents: [intent] })).toThrow(CockpitClientError);
    }
    for (const provenance of [
      { origin: null, supervisor_run_id: "root", conversation: null },
      { origin: null, supervisor_run_id: null, conversation: attachment.conversation },
    ]) expect(() => parseOrchestrationSnapshot({ ...snapshot, intents: [{ ...snapshot.intents[0]!, ...provenance }] })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationSnapshot(replaceField(snapshot, ["intents", 0, "result_message_id"], ""))).toThrow(CockpitClientError);
  });

  it("decodes pending launches and refuses live agent proof on absent panes", () => {
    for (const step of ["launch_pending", "move_intent", "move_unknown", "placed"]) {
      expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runs", 0, "dispatch", "step"], step)).runs[0]!.dispatch!.step).toBe(step);
    }
    for (const presence of ["missing", "endpoint_changed", "unobserved"]) {
      const dto = replaceField(snapshot, ["runtime", "runs", 0, "presence"], presence);
      expect(() => parseOrchestrationSnapshot(dto)).toThrow(CockpitClientError);
      expect(parseOrchestrationSnapshot(replaceField(dto, ["runtime", "runs", 0, "actual_agent"], false))).toBeDefined();
    }
  });

  it("requires stable assignment UUIDs, exact resolve revisions and pointer-only intents", () => {
    const assign = { action: "task_assign", root_id: "root", task_id: assignmentTaskId, title: "Title", description: "Body" };
    const resolve = { action: "task_assignment_resolve", root_id: "root", task_id: assignmentTaskId, expected_task_revision: null, assign: false };
    expect(parseOrchestrationAction(resolve)).toEqual(resolve);
    expect(() => parseOrchestrationAction({ ...resolve, assign: true })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationAction({ ...resolve, assign: true, expected_task_revision: "bad" })).toThrow(CockpitClientError);
    for (const task_id of ["task", "", "7b613f19-4a52-41fa-8864-a880cd69ef5z"]) {
      expect(() => parseOrchestrationAction({ ...assign, task_id })).toThrow(CockpitClientError);
      expect(() => parseOrchestrationAction({ ...resolve, task_id })).toThrow(CockpitClientError);
      expect(() => parseOrchestrationSnapshot({ ...snapshot, assignment_intents: [{ ...snapshot.assignment_intents[0]!, task_id }] })).toThrow(CockpitClientError);
    }
    for (const extra of [{ body: "Not canonical" }, { title: "Not canonical" }]) {
      expect(() => parseOrchestrationSnapshot({ ...snapshot, assignment_intents: [{ ...snapshot.assignment_intents[0]!, ...extra }] })).toThrow(CockpitClientError);
    }
    expect(() => parseOrchestrationAction({ ...assign, origin: "supervisor" })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationAction({ ...assign, title: "é".repeat(129) })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationAction({ ...assign, description: "é".repeat(8193) })).toThrow(CockpitClientError);
  });

  it("rejects unsafe counters, malformed digests, timestamps, booleans and foreign sessions", () => {
    for (const revision of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, Infinity, "2", null]) {
      expect(() => parseOrchestrationSnapshot({ ...snapshot, revision })).toThrow(CockpitClientError);
      expect(() => parseOrchestrationMutationResponse({ revision, result: { result: "done" } })).toThrow(CockpitClientError);
      expect(() => parseOrchestrationWaitResponse({ revision, tasks_token: hash, changed: false })).toThrow(CockpitClientError);
    }
    for (const [path, bad] of [
      [["messages", 0, "seq"], Number.MAX_SAFE_INTEGER + 1], [["runs", 0, "attempt"], 0x100000000],
      [["tasks_token"], "bad"], [["board", "doc_revision"], "A".repeat(64)], [["runs", 0, "task_revision_at_start"], "bad"],
      [["runs", 0, "created_at"], "yesterday"], [["messages", 0, "stale"], 1], [["runs", 0, "session_id"], "other"],
      [["runs", 0, "attachment", "location", "session_id"], "other"], [["unmanaged_agents", 0, "agent_status"], {}],
    ] as const) expect(() => parseOrchestrationSnapshot(replaceField(snapshot, [...path], bad))).toThrow(CockpitClientError);
    expect(() => matchOrchestrationSnapshot(snapshot, { session_id: "other", root_id: null })).toThrow(CockpitClientError);
    expect(() => matchOrchestrationSnapshot(snapshot, { session_id: "session", root_id: "other" })).toThrow(CockpitClientError);
    expect(parseOrchestrationSnapshot({ ...snapshot, revision: Number.MAX_SAFE_INTEGER }).revision).toBe(Number.MAX_SAFE_INTEGER);
  });

  it("enforces UTF-8 text bounds, not just JavaScript character counts", () => {
    expect(parseOrchestrationAction({ action: "annotate", run_id: "run", text: "é".repeat(8192) })).toBeDefined();
    expect(() => parseOrchestrationAction({ action: "annotate", run_id: "run", text: "é".repeat(8193) })).toThrow(CockpitClientError);
    for (const path of [["messages", 0, "text"], ["runs", 0, "brief"]]) {
      expect(() => parseOrchestrationSnapshot(replaceField(snapshot, path, "é".repeat(8193)))).toThrow(CockpitClientError);
    }
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["board", "tasks", 0, "task", "body"], "é".repeat(8193)))).toBeDefined();
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runtime", "runs", 0, "workspace_label"], "é".repeat(129)))).toBeDefined();
    expect(() => parseOrchestrationAction({ action: "run_adopt", label: "é".repeat(129) })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationAction({ action: "annotate", run_id: "run", text: "a\0b" })).toThrow(CockpitClientError);
  });

  it("validates every action and rejects missing fields, actor spoofing and unknown fields", () => {
    for (const action of actions) {
      expect(parseOrchestrationAction(action)).toEqual(action);
      expect(parseOrchestrationMutationRequest({ session_id: "session", expected_revision: 2, action }).action).toEqual(action);
      for (const path of fieldPaths(action)) {
        if (action.action === "cleanup_finish" && path.join(".") === "request.run_id") continue;
        if (path.join(".") === "location.peer_process" || path[path.length - 1] === "herdr_peer") {
          expect(parseOrchestrationAction(replaceField(action, path, undefined, true))).toEqual(action);
          continue;
        }
        expect(() => parseOrchestrationAction(replaceField(action, path, undefined, true)), `${action.action}:${path.join(".")}`).toThrow(CockpitClientError);
      }
      expect(() => parseOrchestrationAction({ ...action, actor: "operator" })).toThrow(CockpitClientError);
    }
    for (const bad of [null, [], { action: "unknown" }, { action: "__proto__" }]) expect(() => parseOrchestrationAction(bad)).toThrow(CockpitClientError);
    expect(() => parseOrchestrationAction({ action: "inbox_pull", after_seq: 0, limit: 101 })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationMutationRequest({ session_id: "session", expected_revision: 2, action: actions[0], origin: "native" })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationSnapshotRequest({ session_id: "../session", root_id: null })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationSnapshotRequest({ session_id: "session" })).toThrow(CockpitClientError);
    expect(parseOrchestrationAction({ ...startAction, target: { target: "existing_space", workspace_id: "space" } })).toBeDefined();
  });

  it("validates all mutation result variants deeply", () => {
    const results = [{ result: "task", task }, { result: "task_ids", assigned: 1, doc_revision: hash },
      { result: "task_assigned", task, to_run_id: "root", seq: 1, duplicate: false },
      { result: "run", run_id: "run", attempt: 1 }, { result: "message", to_run_id: "run", seq: 1, duplicate: false, stale: true },
      { result: "inbox", messages: [message], read_through_seq: 1 }, { result: "done" },
      { result: "supervisor_started", start: { root_id: "root", operation_id: assignmentTaskId, outcome: "resuming", resume_operation_id: prerequisiteId, duplicate: false } },
      { result: "run_resumed", resume: { run_id: "run", resume, duplicate: false } },
      { result: "run_attached", attach: { run_id: "run", attachment, duplicate: false } },
      workerStarted, scopeUpdated,
      { result: "task_scheduled", operation_id: assignmentTaskId, task_id: parentStepId, duplicate: false },
      { result: "worker_replaced", start: workerStarted.start, previous_run_id: "run", assignment_generation: 2,
        placement: { placement: "same_checkout" } },
      { result: "integration_recorded", receipt: { operation_id: prerequisiteId, run_id: "run", integration: integrated, checks: integrated.checks, duplicate: false } },
      { result: "resource", request: resourceRecord, duplicate: false },
      { result: "resource_registered", resource: ownedResource, duplicate: false },
      { result: "cleanup_finished", cleanup: cleanupFinished }];
    for (const result of results) {
      const response = { revision: 2, result };
      expect(parseOrchestrationMutationResponse(response)).toEqual(response);
      for (const path of fieldPaths(response)) {
        if (["result.attach.attachment.process_scope", "result.attach.attachment.location.peer_process"].includes(path.join("."))) {
          expect(parseOrchestrationMutationResponse(replaceField(response, path, undefined, true))).toEqual(response);
        } else {
          expect(() => parseOrchestrationMutationResponse(replaceField(response, path, undefined, true))).toThrow(CockpitClientError);
        }
      }
    }
    expect(() => parseOrchestrationMutationResponse({ revision: 2, result: { result: "unknown" } })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationMutationResponse({ revision: 2, result: { result: "inbox", messages: [{ ...message, seq: -1 }], read_through_seq: 1 } })).toThrow(CockpitClientError);
    for (const bad of [
      { result: "task_assigned", task: { ...task, task_revision: "bad" }, to_run_id: "root", seq: 1, duplicate: false },
      { result: "task_assigned", task, to_run_id: "", seq: 1, duplicate: false },
      { result: "task_assigned", task, to_run_id: "root", seq: Number.MAX_SAFE_INTEGER + 1, duplicate: false },
      { result: "task_assigned", task, to_run_id: "root", seq: 1, duplicate: "false" },
    ]) expect(() => parseOrchestrationMutationResponse({ revision: 2, result: bad })).toThrow(CockpitClientError);
  });

  it("preserves scope-update state, optional revision facts and worker-self no-instruction results", () => {
    for (const state of ["applied", "conflict", "pending"]) {
      for (const task_revision of [hash, null]) {
        for (const doc_revision of [hash, null]) {
          for (const instruction_message_id of ["scope-instruction", null]) {
            for (const duplicate of [false, true]) {
              const response = { revision: 2, result: { ...scopeUpdated,
                update: { ...scopeUpdated.update, state, task_revision, doc_revision, instruction_message_id, duplicate } } };
              expect(parseOrchestrationMutationResponse(response)).toEqual(response);
            }
          }
        }
      }
    }
    for (const field of ["task_revision", "doc_revision", "instruction_message_id"]) {
      const response = { revision: 2, result: { ...scopeUpdated, update: { ...scopeUpdated.update, [field]: null } } };
      expect(parseOrchestrationMutationResponse(response)).toEqual(response);
      expect(() => parseOrchestrationMutationResponse(replaceField(response, ["result", "update", field], undefined, true)))
        .toThrow(CockpitClientError);
    }
  });

  it("rejects malformed and private one-start and scope-update result evidence", () => {
    for (const result of [workerStarted, scopeUpdated]) {
      const response = { revision: 2, result };
      for (const path of fieldPaths(response)) {
        expect(() => parseOrchestrationMutationResponse(replaceField(response, path, {})), path.join(".")).toThrow(CockpitClientError);
      }
      for (const ownerPath of [[], ...fieldPaths(response)]) {
        const owner = ownerPath.reduce<unknown>((value, key) => (value as Record<string | number, unknown>)[key], response);
        if (owner === null || typeof owner !== "object" || Array.isArray(owner)) continue;
        for (const field of ["extra", "credential_id", "secret", "secret_digest", "auth", "job_auth", "credential_file"]) {
          expect(() => parseOrchestrationMutationResponse(replaceField(response, ownerPath, { ...owner, [field]: "private" })))
            .toThrow(CockpitClientError);
        }
      }
    }
    for (const [result, path, value] of [
      [workerStarted, ["start", "operation_id"], "bad"], [workerStarted, ["start", "operation_id"], null],
      [workerStarted, ["start", "run_id"], "run"], [workerStarted, ["start", "run_id"], null],
      [workerStarted, ["start", "attempt"], 0], [workerStarted, ["start", "attempt"], -1],
      [workerStarted, ["start", "attempt"], 1.5], [workerStarted, ["start", "attempt"], 0x100000000],
      [workerStarted, ["start", "attempt"], "1"], [workerStarted, ["start", "duplicate"], "false"],
      [scopeUpdated, ["update", "operation_id"], "bad"], [scopeUpdated, ["update", "state"], "done"],
      [scopeUpdated, ["update", "task_revision"], "bad"], [scopeUpdated, ["update", "doc_revision"], "A".repeat(64)],
      [scopeUpdated, ["update", "instruction_message_id"], ""],
      [scopeUpdated, ["update", "instruction_message_id"], "bad\nid"], [scopeUpdated, ["update", "duplicate"], 0],
    ] as const) {
      expect(() => parseOrchestrationMutationResponse({ revision: 2, result: replaceField(result, [...path], value) }))
        .toThrow(CockpitClientError);
    }
    expect(parseOrchestrationMutationResponse({ revision: 2, result: {
      ...workerStarted, start: { ...workerStarted.start, attempt: 0xffffffff, duplicate: true },
    } })).toBeDefined();
  });

  it("rejects unknown or private mutation-envelope fields", () => {
    const response = { revision: 2, result: { result: "supervisor_started", start: { root_id: "root", operation_id: assignmentTaskId, outcome: "created", resume_operation_id: null, duplicate: false } } };
    expect(parseOrchestrationMutationResponse(response)).toEqual(response);
    for (const field of ["extra", "credential_id", "secret", "secret_digest", "auth", "job_auth"]) {
      expect(() => parseOrchestrationMutationResponse({ ...response, [field]: "private" })).toThrow(CockpitClientError);
    }
  });

  it("rejects malformed or private wave-one result records at every nesting level", () => {
    const started = { result: "supervisor_started", start: { root_id: "root", operation_id: assignmentTaskId,
      outcome: "created", resume_operation_id: null, duplicate: false } };
    const resumed = { result: "run_resumed", resume: { run_id: "run", resume, duplicate: false } };
    const attached = { result: "run_attached", attach: { run_id: "run", attachment, duplicate: false } };
    for (const result of [started, resumed, attached]) {
      const response = { revision: 2, result };
      expect(parseOrchestrationMutationResponse(response)).toEqual(response);
      for (const path of fieldPaths(response)) {
        expect(() => parseOrchestrationMutationResponse(replaceField(response, path, {}))).toThrow(CockpitClientError);
      }
      for (const ownerPath of [[], ...fieldPaths(response)]) {
        const owner = ownerPath.reduce<unknown>((value, key) => (value as Record<string | number, unknown>)[key], response);
        if (owner === null || typeof owner !== "object" || Array.isArray(owner)) continue;
        for (const field of ["extra", "credential_id", "secret", "secret_digest", "auth", "job_auth"]) {
          expect(() => parseOrchestrationMutationResponse(replaceField(response, ownerPath, { ...owner, [field]: "private" })))
            .toThrow(CockpitClientError);
        }
      }
    }
    for (const [result, path, value] of [
      [started, ["start", "root_id"], ""], [started, ["start", "operation_id"], "bad"],
      [started, ["start", "resume_operation_id"], "bad"], [started, ["start", "outcome"], "started"],
      [started, ["start", "duplicate"], "false"],
      [resumed, ["resume", "run_id"], ""], [resumed, ["resume", "run_id"], "foreign-run"],
      [resumed, ["resume", "duplicate"], 0], [resumed, ["resume", "resume", "operation_id"], "bad"],
      [resumed, ["resume", "resume", "previous_attachment_epoch"], 0],
      [resumed, ["resume", "resume", "replacement_attachment_epoch"], Number.MAX_SAFE_INTEGER + 1],
      [resumed, ["resume", "resume", "cause", "class"], "unknown"],
      [resumed, ["resume", "resume", "cause", "message"], "é".repeat(8193)],
      [resumed, ["resume", "resume", "updated_at"], "yesterday"],
      [attached, ["attach", "run_id"], ""], [attached, ["attach", "duplicate"], "false"],
      [attached, ["attach", "attachment", "attachment_id"], "bad"],
      [attached, ["attach", "attachment", "epoch"], Number.MAX_SAFE_INTEGER + 1],
      [attached, ["attach", "attachment", "state"], "live"],
      [attached, ["attach", "attachment", "launch_attempt"], 0x100000000],
      [attached, ["attach", "attachment", "conversation", "session_id"], ""],
      [attached, ["attach", "attachment", "machine_id"], "é".repeat(257)],
      [attached, ["attach", "attachment", "updated_at"], "yesterday"],
      [attached, ["attach", "attachment", "location", "session_id"], "../session"],
    ] as const) expect(() => parseOrchestrationMutationResponse({ revision: 2, result: replaceField(result, [...path], value) }))
      .toThrow(CockpitClientError);
    for (const outcome of ["reused", "resuming", "created"]) {
      expect(parseOrchestrationMutationResponse({ revision: 2, result: { ...started,
        start: { ...started.start, outcome, duplicate: true, resume_operation_id: prerequisiteId } } })).toBeDefined();
    }
    expect(parseOrchestrationMutationResponse({ revision: 2, result: { ...resumed,
      resume: { ...resumed.resume, resume: { ...resume, previous_attachment_epoch: null, cause: null, decision_id: null } } } })).toBeDefined();
    expect(parseOrchestrationMutationResponse({ revision: 2, result: { ...attached, attach: { ...attached.attach,
      attachment: { ...attachment, location: null, conversation: null, process: null, shell: null, machine_id: null } } } })).toBeDefined();
  });

  it("validates long-poll timeouts and invalidation tokens", () => {
    const request = { root_id: null, after_revision: 2, after_tasks_token: hash, timeout_ms: 30_000 };
    expect(parseOrchestrationWaitRequest(request)).toEqual(request);
    expect(parseOrchestrationWaitRequest({ ...request, root_id: "selected-root" })).toEqual({ ...request, root_id: "selected-root" });
    for (const root_id of ["", " ", 7, false]) expect(() => parseOrchestrationWaitRequest({ ...request, root_id })).toThrow(CockpitClientError);
    for (const timeout_ms of [-1, 30_001, 1.5, NaN, "30000"]) expect(() => parseOrchestrationWaitRequest({ ...request, timeout_ms })).toThrow(CockpitClientError);
    for (const path of fieldPaths(request)) expect(() => parseOrchestrationWaitRequest(replaceField(request, path, undefined, true))).toThrow(CockpitClientError);
    expect(() => parseOrchestrationWaitRequest({ ...request, after_revision: Number.MAX_SAFE_INTEGER + 1 })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationWaitResponse({ revision: 2, tasks_token: hash, changed: "yes" })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationWaitResponse({ revision: 2, tasks_token: "bad", changed: false })).toThrow(CockpitClientError);
  });
});

describe("browser/native orchestration parity", () => {
  it.each(["browser", "native"] as const)("preserves W3 integration receipts and resource proof on %s", async transport => {
    const cases: { action: OrchestrationAction; response: unknown }[] = [
      { action: { action: "integration_record", record: integrationRecord }, response: { revision: 3,
        result: { result: "integration_recorded", receipt: { operation_id: prerequisiteId, run_id: "run",
          integration: integrated, checks: integrated.checks, duplicate: true } } } },
      { action: { action: "resource_request", request: resourceRequest }, response: { revision: 3,
        result: { result: "resource", request: resourceRecord, duplicate: false } } },
      { action: { action: "cleanup_finish", request: { operation_id: parentStepId, root_id: "root", run_id: null } },
        response: { revision: 3, result: { result: "cleanup_finished", cleanup: cleanupFinished } } },
    ];
    for (const { action, response } of cases) {
      const browserCall = vi.fn(async () => new Response(JSON.stringify(response)));
      const nativeCall = vi.fn(async () => response);
      const client = transport === "browser" ? createBrowserClient(browserCall) : createNativeClient(nativeCall);
      await expect(client.orchestrationMutate({ session_id: "session", expected_revision: null, action })).resolves.toEqual(response);
      expect(transport === "browser" ? browserCall : nativeCall).toHaveBeenCalledOnce();
    }
    const bad = { ...snapshot, runs: [{ ...codeRun, integration: { ...integrated, accepted_result_message_id: "wrong-result" } }] };
    const client = transport === "browser" ? createBrowserClient(vi.fn(async () => new Response(JSON.stringify(bad))))
      : createNativeClient(vi.fn(async () => bad));
    await expect(client.orchestrationSnapshot({ session_id: "session", root_id: "root" })).rejects.toMatchObject({ code: "malformed_response" });
  });

  it.each(["browser", "native"] as const)("preserves question_not_current refusal identity and never retries an Answer on %s", async transport => {
    const envelope = { code: "question_not_current", message: "The displayed question is no longer current" };
    const call = vi.fn(async () => {
      if (transport === "browser") return new Response(JSON.stringify(envelope), { status: 409 });
      throw envelope;
    });
    const client = transport === "browser" ? createBrowserClient(call) : createNativeClient(call);
    const request = { session_id: "session", expected_revision: null, action: { action: "message_send" as const, message_id: "answer-q1", to_run_id: "run", kind: "answer" as const, text: "Use the existing checkout", in_reply_to: "q1" } };
    await expect(client.orchestrationMutate(request)).rejects.toMatchObject({
      code: transport === "browser" ? "http_error" : "native_error",
      operationCode: "question_not_current",
    });
    expect(call).toHaveBeenCalledOnce();
  });

  it.each(["browser", "native"] as const)("preserves one-start refusal identity without automatically retrying on %s", async transport => {
    const envelope = { code: "task_revision_conflict", message: "Task changed before worker start" };
    const call = vi.fn(async () => {
      if (transport === "browser") return new Response(JSON.stringify(envelope), { status: 409 });
      throw envelope;
    });
    const client = transport === "browser" ? createBrowserClient(call) : createNativeClient(call);
    await expect(client.orchestrationMutate({ session_id: "session", expected_revision: null, action: workerStartAction }))
      .rejects.toMatchObject({
        code: transport === "browser" ? "http_error" : "native_error", operationCode: envelope.code,
      });
    expect(call).toHaveBeenCalledOnce();
  });

  const snapshotRequest = { session_id: "session", root_id: "root" };
  const mutationRequest = { session_id: "session", expected_revision: 2, action: { action: "cancel_run" as const, run_id: "run" } };
  const waitRequest = { root_id: "root", after_revision: 2, after_tasks_token: hash, timeout_ms: 30_000 };

  it.each(["browser", "native"] as const)("rejects malformed responses and foreign sessions on %s", async transport => {
    for (const badSnapshot of [{ ...snapshot, session_id: "foreign" },
      replaceField(snapshot, ["runs", 0, "stage"], "ready")]) {
      const client = transport === "browser" ? createBrowserClient(vi.fn(async () => new Response(JSON.stringify(badSnapshot)))) : createNativeClient(vi.fn(async () => badSnapshot));
      await expect(client.orchestrationSnapshot(snapshotRequest)).rejects.toMatchObject({ code: "malformed_response" });
    }
    const malformed = transport === "browser" ? createBrowserClient(vi.fn(async () => new Response('{"revision":9007199254740992,"result":{"result":"done"}}'))) : createNativeClient(vi.fn(async () => ({ revision: Number.MAX_SAFE_INTEGER + 1, result: { result: "done" } })));
    await expect(malformed.orchestrationMutate(mutationRequest)).rejects.toMatchObject({ code: "malformed_response" });
    await expect(malformed.orchestrationWait(waitRequest)).rejects.toMatchObject({ code: "malformed_response" });
  });

  it.each(["browser", "native"] as const)("does not send invalid requests on %s", async transport => {
    const call = vi.fn();
    const client = transport === "browser" ? createBrowserClient(call) : createNativeClient(call);
    await expect(client.orchestrationSnapshot({ ...snapshotRequest, session_id: "../session" })).rejects.toMatchObject({ code: "malformed_response" });
    await expect(client.orchestrationMutate({ ...mutationRequest, expected_revision: Number.MAX_SAFE_INTEGER + 1 })).rejects.toMatchObject({ code: "malformed_response" });
    await expect(client.orchestrationMutate({ ...mutationRequest, action: { ...workerStartAction, operation_id: "bad" } }))
      .rejects.toMatchObject({ code: "malformed_response" });
    await expect(client.orchestrationMutate({ ...mutationRequest, action: { ...scopeUpdateAction, title: null, description: null } }))
      .rejects.toMatchObject({ code: "malformed_response" });
    await expect(client.orchestrationWait({ ...waitRequest, timeout_ms: 30_001 })).rejects.toMatchObject({ code: "malformed_response" });
    expect(call).not.toHaveBeenCalled();
  });
});
