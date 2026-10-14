import type { OrchestrationSnapshot, Run, TaskView } from "../../protocol/generated/v1";

export function taskFixture(id: string, overrides: Partial<TaskView> = {}): TaskView {
  return {
    task: { task_id: id, title: id, body: "Description", description: "Description", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1, task_revision: `revision-${id}`, diagnostic: null },
    lane: "ready", scheduled: false, attempt: null, current_run_id: null, dependencies: { state: "none", unmet: [], problems: [] }, ...overrides,
  };
}
export function runFixture(overrides: Partial<Run> = {}): Run {
  const at = "2026-10-07T12:00:00Z";
  return {
    session_id: "session-1", brief: "", run_id: "root", root_id: "root", kind: "supervisor", label: "Liaison", parent_run_id: null, task_id: null, attempt: 1, assignment_generation: null,
    launch_config: { harness_kind: "omp", command: "omp", extra_args: [], resume_args: null, inbox_prompt_hints: false, routes: [], hook_args: null, initial_prompt_args: null, max_parallel: 4, process_scope: "auto" },
    task_revision_at_start: null, stage: "active", close_reason: null, dispatch: null, target: { target: "existing_space", workspace_id: "space-1" }, setup: null, cleanup_policy: null, last_report: null, result: null, source_output: null, integration: { state: "pending" }, annotations: [], project: null, checkout: null, checkout_claim_id: null, resume: null, retirement: null, created_at: at, updated_at: at,
    attachment: { attachment_id: "attachment", epoch: 1, state: "connected", launch_mode: "shell_agent", conversation: { harness: "omp", session_id: "conversation" }, process: null, shell: null, machine_id: null, process_scope: null, launch_attempt: 1, updated_at: at,
      location: { boot_id: null, terminal_id: "terminal-2", native_session_id: null, peer_process: null, endpoint_identity: "fixture-endpoint", session_id: "session-1", workspace_id: "space-1", tab_id: "tab-2", pane_id: "pane-2", launch_tag: "ck-root-1-1" } },
    ...overrides,
  };
}
export function orchestrationFixture(tasks: TaskView[] = [], runs: Run[] = [runFixture()]): OrchestrationSnapshot {
  return {
    session_id: "session-1", state_store_id: "store", revision: 1, tasks_token: "tasks", roots: [],
    board: runs.length ? { root_id: "root", path: "/state/tasks.md", doc_revision: "doc", unidentified_items: 0, diagnostics: [], tasks } : null,
    runs, messages: [], questions: [], task_ownership: [], intents: [], assignment_intents: [], attention: [], unmanaged_agents: [], cleanup: [], restoration: { reattached: 0, total_attachments: 0, cleanup_pending: 0 },
    runtime: { status: "fresh", endpoint_identity: "fixture-endpoint", observed_at: "2026-10-07T12:00:00Z", runs: runs.map(run => ({ run_id: run.run_id, presence: "present", actual_agent: true, workspace_id: "space-1", workspace_label: "Project", tab_id: "tab-2", tab_label: "Liaison", pane_id: "pane-2", agent_status: "working", state_changed_at: null })) },
  };
}
