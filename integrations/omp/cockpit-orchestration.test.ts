import { describe, expect, it, vi } from "vitest";
import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import type { NativeStopReceipt, Run, RunRetirement, TaskView } from "../../src/protocol/generated/v1";
import cockpitOrchestration from "./cockpit-orchestration";
import { emptyWakeState, observeWake, recoverWake, mayAcknowledge, parseWakeSummary, parseMainWaitRead, type MainWaitRead } from "./wake";
import { prepareToolAllowed, reportIdentity, requireNativeChild, requireSupervisorManagement, requireWorkerExecution } from "./identity";
import { taskArgs, delegateArgs, managementArgs, contextSource, contextPage, type TaskToolParams } from "./tools";
import { decodeControl, lifecycleStatus } from "./controlLoop";
import { retirementReadiness, readRetirementReadiness, createRetirementHandler, type RetirementReadinessInput } from "./retirement";

describe("durable inbox wake bookkeeping", () => {
  it("coalesces arrivals without implying a read or acknowledgement", () => {
    const first = observeWake(emptyWakeState(), 7, 2);
    const queued = { ...first, queued: true, queuedThrough: 7, pendingThrough: 0, pendingCount: 0 };
    const second = observeWake(queued, 9, 2);
    expect(second).toMatchObject({ queued: true, queuedThrough: 7, pendingThrough: 9, pendingCount: 2, readThrough: 0, ackedThrough: 0 });
    expect(observeWake(second, 9, 2)).toBe(second);
    expect(mayAcknowledge(second, 9)).toBe(false);
  });

  it("uses numeric kind counts, not the CLI pending boolean, and rejects malformed numeric boundaries", () => {
    const envelope = { pending: true, through_seq: 7, counts: [{ kind: "report", count: 2 }, { kind: "instruction", count: 1 }] };
    const summary = parseWakeSummary(envelope);
    expect(observeWake(emptyWakeState(), summary.through, summary.count).pendingCount).toBe(3);
    for (const invalid of [
      { ...envelope, pending: 1 },
      { ...envelope, through_seq: Number.MAX_SAFE_INTEGER + 1 },
      { ...envelope, counts: [{ kind: "report", count: true }] },
      { ...envelope, counts: [{ kind: "report", count: -1 }] },
      { ...envelope, counts: [{ kind: "report", count: Number.MAX_SAFE_INTEGER }, { kind: "instruction", count: 1 }] },
      { ...envelope, pending: false },
    ]) expect(() => parseWakeSummary(invalid)).toThrow();
    expect(parseWakeSummary({ pending: false, through_seq: 7, counts: [] })).toEqual({ through: 7, count: 0 });
  });

  it("re-notifies durable unacked mail after every crash point, independent of queued markers", () => {
    for (const queued of [false, true]) {
      const restored = recoverWake([{ type: "custom", customType: "cockpit-orchestration-wake-v1", data: { ...emptyWakeState(), seen: 9, queued, queuedThrough: 9, readThrough: 7, ackedThrough: 4 } }]);
      expect(restored).toMatchObject({ seen: 0, queued: false, readThrough: 7, ackedThrough: 4 });
      expect(observeWake(restored, 9, 3).pendingThrough).toBe(9);
    }
  });

  it("does not treat read or wake as processed and forbids acknowledging unseen sequences", () => {
    const state = { ...emptyWakeState(), seen: 20, queuedThrough: 20, readThrough: 7 };
    expect(mayAcknowledge(state, 7)).toBe(true);
    expect(mayAcknowledge(state, 8)).toBe(false);
    expect(mayAcknowledge(state, 0)).toBe(false);
    expect(mayAcknowledge(state, 3.5)).toBe(false);
  });

  it("ignores unrelated/custom malformed entries", () => {
    expect(recoverWake([{ type: "custom", customType: "another-extension", data: { readThrough: 100, ackedThrough: 100 } }, { type: "custom", customType: "cockpit-orchestration-wake-v1", data: { readThrough: "7", ackedThrough: 4 } }])).toEqual(emptyWakeState());
  });
});

describe("native identity and bounded preparation", () => {
  it("distinguishes subagents by kind, never by depth or inherited run env", () => {
    expect(reportIdentity({ kind: "sub", id: "clone-at-depth-zero" }, "progress")).toEqual(["--agent-kind", "subagent", "--subagent-id", "clone-at-depth-zero"]);
    expect(() => reportIdentity({ kind: "sub", id: "worker" }, "ready")).toThrow("bound main");
    expect(() => reportIdentity({ kind: "sub", id: "worker" }, "result")).toThrow("bound main");
    expect(reportIdentity({ kind: "main", id: "Main" }, "result")).toEqual(["--agent-kind", "main"]);
  });

  it("fails closed for mutation, shell execution, nested dispatch and unknown tools", () => {
    for (const name of ["write", "edit", "bash", "eval", "task", "cockpit_delegate", "cockpit_task", "cockpit_manage", "mcp__provider__write", "toString"]) expect(prepareToolAllowed(name)).toBe(false);
    for (const name of ["read", "find", "grep", "glob", "cockpit_context", "cockpit_report"]) expect(prepareToolAllowed(name)).toBe(true);
    expect(prepareToolAllowed("cockpit_inbox", "list")).toBe(true);
    expect(prepareToolAllowed("cockpit_inbox", "ack")).toBe(true);
    expect(prepareToolAllowed("cockpit_inbox", "send")).toBe(false);
    expect(prepareToolAllowed("cockpit_message", "show")).toBe(true);
    expect(prepareToolAllowed("cockpit_task", "list")).toBe(true);
    expect(prepareToolAllowed("cockpit_task", "show")).toBe(true);
    for (const operation of ["prepare", "execute", "accept", "send_back", "cancel", "reconcile", "retry_launch"]) expect(prepareToolAllowed("cockpit_manage", operation)).toBe(false);
    for (const operation of ["message", "annotate", "list", undefined]) expect(prepareToolAllowed("cockpit_message", operation)).toBe(false);
    for (const operation of ["create", "update", undefined]) expect(prepareToolAllowed("cockpit_task", operation)).toBe(false);
  });
});

describe("canonical task mutation serialization", () => {
  const task_id = "11111111-1111-4111-8111-111111111111";
  const step_id = "22222222-2222-4222-8222-222222222222";
  const expected_task_revision = "full-item";

  it("retains caller UUID and exact create fences and sends description without a writable body", () => {
    const params: TaskToolParams = { operation: "create", task_id, title: "Follow-up", description: "Prose\nonly",
      depends_on: ["prerequisite"], follow_up_of: "source", expected_doc_revision: "document", source_revision: "source-item" };
    expect(taskArgs(params)).toEqual(["task", "create", "--task-id", task_id, "--title", "Follow-up",
      "--description", "Prose\nonly", "--doc-revision", "document", "--source-revision", "source-item",
      "--follow-up-of", "source", "--depends-on", "prerequisite"]);
    expect(taskArgs(params)).toEqual(taskArgs(params));
    expect(() => taskArgs({ ...params, task_id: undefined })).toThrow("task_id");
    expect(() => taskArgs({ ...params, expected_doc_revision: undefined })).toThrow("expected_doc_revision");
    expect(() => taskArgs({ ...params, source_revision: undefined })).toThrow("source_revision");
    expect(() => taskArgs({ ...params, body: "overwrite" } as TaskToolParams)).toThrow("body is read-only");
  });

  it("requires whole-item and document fences, preserving empty descriptions and dependency clearing", () => {
    expect(taskArgs({ operation: "update", task_id, expected_task_revision, description: "" }))
      .toEqual(["task", "update", task_id, "--revision", expected_task_revision, "--description", ""]);
    expect(taskArgs({ operation: "dependencies_set", task_id, expected_task_revision, expected_doc_revision: "doc", depends_on: [] }))
      .toEqual(["task", "dependencies-set", task_id, "--revision", expected_task_revision, "--doc-revision", "doc"]);
    expect(() => taskArgs({ operation: "update", task_id, description: "new" })).toThrow("expected_task_revision");
    expect(() => taskArgs({ operation: "update", task_id, expected_task_revision, revision: "old" } as TaskToolParams)).toThrow("Unsupported task fields");
    expect(() => taskArgs({ operation: "dependencies_set", task_id, expected_task_revision, expected_doc_revision: "doc" })).toThrow("full depends_on");
  });

  it("exposes all five checklist operations with stable IDs and explicit scope", () => {
    const base = { task_id, expected_task_revision, step_id };
    expect(taskArgs({ ...base, operation: "step_add", parent_step_id: "parent", before_step_id: "next", title: "Step" }))
      .toEqual(["task", "step-add", task_id, "--revision", expected_task_revision, "--step-id", step_id, "--title", "Step", "--parent-step-id", "parent", "--before-step-id", "next"]);
    expect(taskArgs({ ...base, operation: "step_rename", title: "Renamed" }))
      .toContain("step-rename");
    for (const scope of ["leaf", "subtree"] as const) expect(taskArgs({ ...base, operation: "step_set_checked", checked: false, scope }))
      .toEqual(["task", "step-set-checked", task_id, "--revision", expected_task_revision, "--step-id", step_id, "--checked", "false", "--scope", scope]);
    expect(() => taskArgs({ ...base, operation: "step_set_checked", checked: true })).toThrow("explicit");
    expect(taskArgs({ ...base, operation: "step_move", parent_step_id: null, before_step_id: "next" }))
      .toEqual(["task", "step-move", task_id, "--revision", expected_task_revision, "--step-id", step_id, "--before-step-id", "next"]);
    expect(taskArgs({ ...base, operation: "step_remove" }))
      .toEqual(["task", "step-remove", task_id, "--revision", expected_task_revision, "--step-id", step_id]);
  });
});

describe("fresh canonical worker execution eligibility", () => {
  const run = { run_id: "worker", task_id: "task", kind: "worker", stage: "working", close_reason: null,
    init_receipt: { kind: "ready" }, work_plan: { plan_revision: "exact-plan" },
    grants: [{ scope: "execute", plan_revision: "exact-plan" }] } as unknown as Run;
  const view = { current_run_id: "worker", task: { task_id: "task", checked: false, diagnostic: null },
    dependencies: { state: "none", unmet: [], problems: [] } } as unknown as TaskView;

  it("allows healthy independent work and accepted canonical prerequisites, not a Result shortcut", () => {
    expect(() => requireWorkerExecution(run, view)).not.toThrow();
    expect(() => requireWorkerExecution(run, { ...view, dependencies: { state: "satisfied", unmet: [], problems: [] } })).not.toThrow();
    for (const state of ["blocked", "invalid"] as const) expect(() => requireWorkerExecution({
      ...run, result: { kind: "result", outcome: "succeeded" } as Run["result"],
    }, { ...view, dependencies: { state, unmet: [], problems: [] } })).toThrow("prerequisites");
  });

  it("refuses revoked/superseded attempts, missing initialization, stale Execute and accepted or diagnosed tasks", () => {
    for (const change of [
      { stage: "ready" }, { stage: "reported" }, { stage: "closed" }, { task_id: null },
      { init_receipt: null }, { work_plan: null }, { grants: [] },
      { grants: [{ scope: "execute", plan_revision: "old-plan" }] },
    ]) expect(() => requireWorkerExecution({ ...run, ...change } as Run, view)).toThrow("not authorized");
    for (const change of [
      { current_run_id: "new-attempt" }, { current_run_id: null },
      { task: { ...view.task, checked: true } }, { task: { ...view.task, diagnostic: "duplicate UUID" } },
    ]) expect(() => requireWorkerExecution(run, { ...view, ...change })).toThrow("not authorized");
    expect(prepareToolAllowed("functions.cockpit_task", "show")).toBe(true);
    expect(prepareToolAllowed("functions.cockpit_task", "step_add")).toBe(false);
    for (const tool of ["functions.eval", "functions.write", "tools.eval", "multi_tool_use.parallel"]) expect(prepareToolAllowed(tool)).toBe(false);
    expect(prepareToolAllowed("functions.cockpit_report", "result")).toBe(true);
  });
});

describe("authenticated native child registry evidence", () => {
  it("uses the actual live child session and private main binding rather than supplied labels/status", () => {
    const child = { id: "Child", kind: "sub", status: "running", session: {
      isDisposed: false, sessionManager: { getSessionId: () => "child-native" },
    } };
    const main = { id: "Main", kind: "main", session: {
      isDisposed: false, sessionManager: { getSessionId: () => "main-native" },
    } };
    const registry = { get: (id: string) => id === "Child" ? child : id === "Main" ? main : undefined };
    const pi = { pi: { MAIN_AGENT_ID: "Main", AgentRegistry: { global: () => registry } } } as unknown as ExtensionAPI;
    const ctx = { agent: { id: "Child", kind: "sub" }, sessionManager: { getSessionId: () => "child-native" } } as unknown as ExtensionContext;
    expect(() => requireNativeChild(pi, ctx, "main-native")).not.toThrow();
    expect(() => requireNativeChild(pi, ctx, "parent-supplied")).toThrow("ownership changed");
    expect(() => requireNativeChild(pi, { ...ctx, agent: { ...ctx.agent, id: "Sibling" } }, "main-native")).toThrow("ownership changed");
    child.session.isDisposed = true;
    expect(() => requireNativeChild(pi, ctx, "main-native")).toThrow("ownership changed");
    child.session.isDisposed = false;
    child.session.sessionManager.getSessionId = () => "replacement-child";
    expect(() => requireNativeChild(pi, ctx, "main-native")).toThrow("ownership changed");
    child.session.sessionManager.getSessionId = () => "child-native";
    main.session.isDisposed = true;
    expect(() => requireNativeChild(pi, ctx, "main-native")).toThrow("ownership changed");
  });
});

describe("supervisor native management identity", () => {
  const root = { run_id: "root", root_id: "root", parent_run_id: null, kind: "supervisor", stage: "active", bound_omp_session: "native-main", location: { workspace_id: "supervisor-space" } };

  it("allows only the currently bound active native main root", () => {
    expect(() => requireSupervisorManagement(root, { kind: "main" }, "native-main")).not.toThrow();
    expect(() => requireSupervisorManagement({ ...root, kind: "adopted" }, { kind: "main" }, "native-main")).not.toThrow();
    for (const nativeSession of ["", "old-native-main", "clone-native"]) {
      expect(() => requireSupervisorManagement(root, { kind: "main" }, nativeSession)).toThrow("bound native main");
    }
    expect(() => requireSupervisorManagement({ ...root, bound_omp_session: null }, { kind: "main" }, "native-main")).toThrow();
    for (const stage of ["preparing", "initializing", "ready", "reported", "closed"]) {
      expect(() => requireSupervisorManagement({ ...root, stage }, { kind: "main" }, "native-main")).toThrow();
    }
  });

  it("rejects workers and native clones even with inherited matching root/session fields", () => {
    for (const kind of ["sub", "subagent", "unknown"]) {
      expect(() => requireSupervisorManagement(root, { kind }, "native-main")).toThrow("internal subagent");
    }
    for (const stage of ["active", "working"]) {
      expect(() => requireSupervisorManagement({ ...root, kind: "worker", stage }, { kind: "main" }, "native-main")).toThrow();
    }
    expect(() => requireSupervisorManagement({ ...root, parent_run_id: "parent" }, { kind: "main" }, "native-main")).toThrow();
    expect(() => requireSupervisorManagement({ ...root, run_id: "child" }, { kind: "main" }, "native-main")).toThrow();
  });
});

describe("project placement and scoped recovery arguments", () => {
  const proposal = { task_id: "task", target_id: "project-space", prepare_brief: "Read project evidence" };

  it("rejects missing explicit targets and branch/base on non-worktree placements", () => {
    expect(() => delegateArgs({ ...proposal, target_id: " " })).toThrow("explicit real project Space");
    for (const target of ["space", "path"] as const) {
      expect(() => delegateArgs({ ...proposal, target, branch: "feature" })).toThrow("Branch and base");
      expect(() => delegateArgs({ ...proposal, target, base: "main" })).toThrow("Branch and base");
    }
  });

  it("rejects recovery outside reconcile and missing management targets", () => {
    for (const operation of ["prepare", "execute", "accept", "send_back", "cancel", "retry_launch"] as const) {
      expect(() => managementArgs({ operation, run_id: "worker", recovery: "accept_existing_worktree" })).toThrow("only for reconcile");
    }
    expect(() => managementArgs({ operation: "retry_launch", run_id: "" })).toThrow("target run_id");
  });
});

describe("read-only project context boundaries", () => {
  const source = { session_id: "herdr", space_id: "project" };
  const listing = {
    target: source, space_label: "Project", pane_id: null, library_root: "/library",
    items: Array.from({ length: 5_000 }, (_, index) => ({ item_id: `item-${index}`, title: `Item ${index}`, kind: "folder_copy", path: `/library/${index}.md` })),
    checkout_path: "/repo", repository_paths: ["/repo/CODE_GUIDE.md"], diagnostics: [],
  };

  it("keeps linked workers on their source project selection and refuses unverified source identity", () => {
    const run = { session_id: "herdr", setup: { project_workspace_id: "project" },
      location: { session_id: "herdr", workspace_id: "linked" } };
    expect(contextSource(run).space_id).toBe("project");
    expect(contextSource({ ...run, setup: null }).space_id).toBe("linked");
    for (const invalid of [
      { ...run, session_id: "" }, { ...run, location: null },
      { ...run, location: { session_id: "other-session", workspace_id: "linked" } },
      { ...run, setup: { project_workspace_id: "" } },
    ]) expect(() => contextSource(invalid)).toThrow();
  });

  it("makes all 5000 existing paths reachable in bounded pages without losing source or totals", () => {
    const paths: string[] = [];
    let offset = 0;
    do {
      const page = contextPage(listing, source, offset);
      expect(page.total_items).toBe(5_000);
      expect(page.target).toEqual(source);
      expect(page.repository_paths).toEqual(listing.repository_paths);
      const items = page.items as typeof listing.items;
      expect(items.length).toBeLessThanOrEqual(100);
      paths.push(...items.map(item => item.path));
      if (page.next_offset === null) break;
      expect(page.next_offset).toBeGreaterThan(offset);
      offset = page.next_offset as number;
    } while (true);
    expect(paths).toEqual(listing.items.map(item => item.path));
    expect(contextPage(listing, source, 5_000).items).toEqual([]);
    for (const [offset, limit] of [[-1, 100], [0.5, 100], [Number.MAX_SAFE_INTEGER + 1, 100], [0, 0], [0, 101], [0, 1.5]]) {
      expect(() => contextPage(listing, source, offset, limit)).toThrow();
    }
    for (const invalid of [
      { ...listing, target: { ...source, space_id: "other" } },
      { ...listing, target: { ...source, session_id: "other" } },
      { ...listing, items: [{}] }, { ...listing, repository_paths: [false] },
    ]) expect(() => contextPage(invalid, source)).toThrow();
  });
});

describe("exact-subagent controls and terminal telemetry", () => {
  it("decodes only the addressed agent and preserves actual send content", () => {
    expect(decodeControl(JSON.stringify({ subagent_id: "nested-2", op: { op: "send", text: "Use the new brief." } }), "nested-2")).toEqual({ op: "send", text: "Use the new brief." });
    expect(decodeControl(JSON.stringify({ subagent_id: "nested-2", op: { op: "cancel" } }), "nested-2")).toEqual({ op: "cancel" });
    expect(() => decodeControl(JSON.stringify({ subagent_id: "sibling", op: { op: "cancel" } }), "nested-2")).toThrow("another subagent");
    expect(() => decodeControl(JSON.stringify({ subagent_id: "nested-2", op: { op: "send", text: " " } }), "nested-2")).toThrow("Invalid");
  });

  it("keeps real cancellation separate from successful termination", () => {
    expect(lifecycleStatus([{ role: "assistant", stopReason: "stop" }], true)).toBe("cancelled");
    expect(lifecycleStatus([{ role: "assistant", stopReason: "aborted" }], false)).toBe("cancelled");
    expect(lifecycleStatus([{ role: "assistant", stopReason: "error" }], false)).toBe("failed");
    expect(lifecycleStatus([{ role: "assistant", stopReason: "error" }, { role: "assistant", stopReason: "stop" }], false)).toBe("done");
    expect(lifecycleStatus([], false)).toBe("failed");
  });
});

const EMPTY_RETIREMENT_TOKEN = "0".repeat(64);
const OFFER_RETIREMENT_TOKEN = "1".repeat(64);
const DEFER_RETIREMENT_TOKEN = "2".repeat(64);

const retirementOffer: RunRetirement = {
  retirement_id: "retirement", trigger: "accept", result_message_id: "result", task_revision: "revision",
  created_at: "2026-10-07T03:00:00.000Z", updated_at: "2026-10-07T03:00:01.000Z",
  identity: {
    run_attempt: 1, launch_attempt: 1, launch_tag: "worker", endpoint_identity: "endpoint",
    session_id: "herdr", workspace_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal",
    herdr_boot_id: null, omp_session_id: "native-main",
    process: { pid: process.pid, start_ticks: 123, kernel_boot_id: null },
    shell: { process: { pid: process.ppid, start_ticks: 122, kernel_boot_id: null }, executable_device: "1", executable_inode: "2", argv_digest: "digest" },
  },
  state: { state: "native_stop_offered", offered_at: "2026-10-07T03:00:01.000Z" },
};

function nativeReadiness(overrides: Partial<RetirementReadinessInput> = {}): RetirementReadinessInput {
  return {
    retirement: retirementOffer, agentKind: "main", nativeSession: "native-main", boundSession: "native-main",
    pid: process.pid, mode: "tui", entries: [], idle: true, pendingMessages: false, admittedSubmission: false,
    asyncJobs: { running: [] }, mainPendingAsyncWork: false, liveSubagents: false, editorText: "", ...overrides,
  };
}

describe("accepted-worker native readiness", () => {
  it("permits old user messages but refuses all later user entries, including wakes and alternate branches", () => {
    expect(retirementReadiness(nativeReadiness({ entries: [
      { type: "message", timestamp: "2026-10-07T02:59:59.999Z", message: { role: "user", content: "original assignment" } },
    ] }))).toEqual({ kind: "ready" });
    for (const content of [
      "[Cockpit inbox notification]\n1 pending inbox message(s), through sequence 12.\nRun cockpit_inbox with operation=list using the bound SDK tool.",
      "My ordinary user prompt contains [Cockpit inbox notification] but is not a trusted system wake.",
      "Please continue with my edits.",
    ]) {
      for (const timestamp of [retirementOffer.created_at, "2026-10-07T03:00:02.000Z"]) {
        expect(retirementReadiness(nativeReadiness({ entries: [
          { type: "message", parentId: "alternate-branch", timestamp, message: { role: "user", content } },
        ] }))).toMatchObject({ kind: "refuse", reason: "user_activity" });
      }
    }
  });

  it("compares native ISO timestamps against finer Rust acceptance precision without truncating", () => {
    const retirement = { ...retirementOffer, created_at: "2026-10-07T03:00:00.000000001Z" };
    expect(retirementReadiness(nativeReadiness({ retirement, entries: [
      { type: "message", timestamp: "2026-10-07T03:00:00.000Z", message: { role: "user" } },
    ] }))).toEqual({ kind: "ready" });
    expect(retirementReadiness(nativeReadiness({ retirement, entries: [
      { type: "message", timestamp: "2026-10-07T04:00:00.000000001+01:00", message: { role: "user" } },
    ] }))).toMatchObject({ kind: "refuse", reason: "user_activity" });
  });

  it("fails closed on missing journal, invalid dates, malformed messages and unsupported modes", () => {
    for (const entries of [null, undefined, {}, [null], [{ type: "message", message: { role: "user" } }],
      [{ type: "message", timestamp: "2026-02-30T00:00:00Z", message: { role: "user" } }],
      [{ type: "message", timestamp: "2026-10-07T03:00:00Z", message: null }],
      [{ type: "message", timestamp: 123, message: { role: "user" } }],
    ]) expect(retirementReadiness(nativeReadiness({ entries }))).toMatchObject({ kind: "refuse", reason: "native_refused" });
    for (const mode of [undefined, "print", "json", "rpc"]) {
      expect(retirementReadiness(nativeReadiness({ mode }))).toMatchObject({ kind: "refuse", reason: "native_refused" });
    }
    expect(retirementReadiness(nativeReadiness({ retirement: { ...retirementOffer, created_at: "invalid" } }))).toMatchObject({ kind: "refuse", reason: "native_refused" });
  });

  it("requires the exact native main session and PID, never inherited identity", () => {
    for (const overrides of [
      { agentKind: "sub" }, { nativeSession: "child-session" }, { boundSession: "" }, { pid: process.pid + 1 },
      { retirement: { ...retirementOffer, identity: null } },
    ]) expect(retirementReadiness(nativeReadiness(overrides))).toMatchObject({ kind: "refuse", reason: "native_refused" });
  });

  it("defers every kind of pending local work and draft", () => {
    for (const [overrides, reason] of [
      [{ idle: false }, "busy"], [{ admittedSubmission: true }, "busy"],
      [{ pendingMessages: true }, "pending_messages"], [{ asyncJobs: null }, "async_jobs"],
      [{ asyncJobs: { running: ["background"] } }, "async_jobs"], [{ mainPendingAsyncWork: true }, "async_jobs"],
      [{ liveSubagents: true }, "live_subagents"], [{ editorText: " unsent draft " }, "editor_draft"],
    ] as const) expect(retirementReadiness(nativeReadiness(overrides))).toEqual({ kind: "defer", reason });
    expect(retirementReadiness(nativeReadiness({ editorText: " \n " }))).toEqual({ kind: "ready" });
    expect(retirementReadiness(nativeReadiness({ idle: false, entries: [
      { type: "message", timestamp: retirementOffer.created_at, message: { role: "user" } },
    ] }))).toMatchObject({ kind: "refuse", reason: "user_activity" });
  });
});

describe("native retirement receipt authority", () => {
  function consumer() {
    let input = nativeReadiness();
    let active = true;
    const shutdown = vi.fn();
    const receipt = vi.fn<(record: RunRetirement, outcome: NativeStopReceipt) => Promise<void>>().mockResolvedValue(undefined);
    const handler = createRetirementHandler({
      readiness: record => retirementReadiness({ ...input, retirement: record }),
      receipt, shutdown, active: () => active,
    });
    return { handler, shutdown, receipt, setInput(overrides: Partial<RetirementReadinessInput>) { input = { ...input, ...overrides }; }, stop() { active = false; } };
  }

  it("does not shut down after a rejected receipt, but accepts a later fresh receipt", async () => {
    const c = consumer();
    c.receipt.mockRejectedValueOnce(new Error("retirement_state_changed"));
    await expect(c.handler.observe(retirementOffer)).rejects.toThrow("retirement_state_changed");
    expect(c.shutdown).not.toHaveBeenCalled();
    await c.handler.observe(retirementOffer);
    expect(c.receipt).toHaveBeenLastCalledWith(retirementOffer, { outcome: "shutdown_requested" });
    expect(c.shutdown).toHaveBeenCalledTimes(1);
  });

  it("rechecks all native safety evidence after durable receipt before any shutdown", async () => {
    for (const mutation of [
      { editorText: "new draft" }, { pendingMessages: true }, { asyncJobs: { running: ["new job"] } },
      { admittedSubmission: true }, { liveSubagents: true }, { nativeSession: "replaced" }, { mode: "rpc" },
      { entries: [{ type: "message", timestamp: retirementOffer.created_at, message: { role: "user" } }] },
    ]) {
      const c = consumer();
      c.receipt.mockImplementationOnce(async () => { c.setInput(mutation); });
      await c.handler.observe(retirementOffer);
      await c.handler.agentEnd();
      expect(c.shutdown).not.toHaveBeenCalled();
    }
    const c = consumer();
    c.receipt.mockImplementationOnce(async () => { c.stop(); });
    await c.handler.observe(retirementOffer);
    expect(c.shutdown).not.toHaveBeenCalled();
  });

  it("requests native shutdown once after receipt and re-requests on agent_end only while locally safe", async () => {
    const c = consumer();
    await c.handler.observe(retirementOffer);
    await c.handler.observe(retirementOffer);
    expect(c.receipt).toHaveBeenCalledTimes(1);
    expect(c.shutdown).toHaveBeenCalledTimes(1);
    await c.handler.agentEnd();
    expect(c.shutdown).toHaveBeenCalledTimes(2);
    c.setInput({ editorText: "do not discard this" });
    await c.handler.agentEnd();
    expect(c.shutdown).toHaveBeenCalledTimes(2);
    c.setInput({ editorText: "", entries: [{ type: "message", timestamp: retirementOffer.created_at, message: { role: "user" } }] });
    await c.handler.agentEnd();
    expect(c.shutdown).toHaveBeenCalledTimes(2);
  });

  it("sends typed refusal and only changed deferral reasons; unrelated states have no authority", async () => {
    const c = consumer();
    c.setInput({ idle: false });
    await c.handler.observe(retirementOffer);
    await c.handler.observe(retirementOffer);
    expect(c.receipt).toHaveBeenCalledTimes(1);
    c.setInput({ idle: true, editorText: "draft" });
    await c.handler.observe(retirementOffer);
    expect(c.receipt).toHaveBeenLastCalledWith(retirementOffer, { outcome: "deferred", reason: "editor_draft" });
    c.setInput({ entries: [{ type: "message", timestamp: retirementOffer.created_at, message: { role: "user" } }] });
    await c.handler.observe(retirementOffer);
    expect(c.receipt).toHaveBeenLastCalledWith(retirementOffer, expect.objectContaining({ outcome: "refused", reason: "user_activity" }));
    const states: RunRetirement["state"][] = [
      { state: "waiting", blockers: [] }, { state: "native_stop_requested", at: retirementOffer.created_at },
      { state: "retained", at: retirementOffer.created_at, reason: "user_activity", native_stopped: false },
      { state: "unknown", at: retirementOffer.created_at, phase: "native_stop", detail: "unconfirmed" },
    ];
    for (const state of states) await c.handler.observe({ ...retirementOffer, state });
    expect(c.receipt).toHaveBeenCalledTimes(3);
    expect(c.shutdown).not.toHaveBeenCalled();
  });

  it("revokes a deferred cached offer after durable timeout retention", async () => {
    const c = consumer();
    c.setInput({ idle: false });
    await c.handler.observe(retirementOffer);
    expect(c.receipt).toHaveBeenLastCalledWith(retirementOffer, { outcome: "deferred", reason: "busy" });
    await c.handler.observe({ ...retirementOffer, state: {
      state: "retained", at: retirementOffer.updated_at, reason: "worker_busy_timeout", native_stopped: false,
    } });
    c.receipt.mockClear();
    c.setInput({ idle: true });
    await c.handler.agentEnd();
    expect(c.receipt).not.toHaveBeenCalled();
    expect(c.shutdown).not.toHaveBeenCalled();
  });

  it("revokes re-request authority when the owner ends the retirement attempt", async () => {
    const c = consumer();
    await c.handler.observe(retirementOffer);
    await c.handler.observe({ ...retirementOffer, state: { state: "native_stop_requested", at: retirementOffer.created_at } });
    await c.handler.agentEnd();
    expect(c.shutdown).toHaveBeenCalledTimes(2);
    await c.handler.observe({ ...retirementOffer, state: { state: "unknown", phase: "native_stop", at: retirementOffer.created_at, detail: "exit not proven" } });
    await c.handler.agentEnd();
    expect(c.shutdown).toHaveBeenCalledTimes(2);
  });
});

describe("OMP18.7 native observation boundary", () => {
  function nativeObservation() {
    const sessionManager = { getSessionId: () => "native-main", getEntries: () => [] as unknown[] };
    const main = {
      id: "Main", kind: "main", status: "idle",
      session: { sessionManager, isDisposed: false, queuedMessageCount: 0, hasAdmittedSubmission: false, hasPendingAsyncWork: () => false },
    };
    const refs = [main] as Array<{ id: string; kind: string; status: string; session: typeof main.session | null }>;
    const registry = { get: (id: string) => refs.find(ref => ref.id === id), list: () => refs };
    const pi = { pi: { AgentRegistry: { global: () => registry } } } as unknown as ExtensionAPI;
    const ctx = {
      agent: { id: "Main", kind: "main" }, mode: "tui", sessionManager, isIdle: () => true,
      hasPendingMessages: () => false, getAsyncJobSnapshot: () => ({ running: [] }), ui: { getEditorText: () => "" },
    } as unknown as ExtensionContext;
    return { pi, ctx, main, refs, sessionManager };
  }

  it("detects actual registry work while excluding passive advisors", () => {
    const n = nativeObservation();
    expect(readRetirementReadiness(n.pi, n.ctx, "native-main", retirementOffer)).toEqual({ kind: "ready" });
    n.refs.push({ id: "advisor", kind: "advisor", status: "running", session: n.main.session });
    expect(readRetirementReadiness(n.pi, n.ctx, "native-main", retirementOffer)).toEqual({ kind: "ready" });
    n.refs.push({ id: "child", kind: "sub", status: "running", session: null });
    expect(readRetirementReadiness(n.pi, n.ctx, "native-main", retirementOffer)).toEqual({ kind: "defer", reason: "live_subagents" });
    n.refs[2].status = "idle";
    n.refs[2].session = { ...n.main.session, hasPendingAsyncWork: () => true };
    expect(readRetirementReadiness(n.pi, n.ctx, "native-main", retirementOffer)).toEqual({ kind: "defer", reason: "live_subagents" });
  });

  it("fails closed on journal/registry observation failure and exact native session replacement", () => {
    const n = nativeObservation();
    n.sessionManager.getEntries = () => { throw new Error("journal unavailable"); };
    expect(readRetirementReadiness(n.pi, n.ctx, "native-main", retirementOffer)).toMatchObject({ kind: "refuse", reason: "native_refused" });
    const replaced = nativeObservation();
    replaced.main.session.sessionManager = { getSessionId: () => "replacement", getEntries: () => [] };
    expect(readRetirementReadiness(replaced.pi, replaced.ctx, "native-main", retirementOffer)).toMatchObject({ kind: "refuse", reason: "native_refused" });
    const absent = nativeObservation();
    absent.refs.length = 0;
    expect(readRetirementReadiness(absent.pi, absent.ctx, "native-main", retirementOffer)).toMatchObject({ kind: "refuse", reason: "native_refused" });
    expect(readRetirementReadiness({} as ExtensionAPI, nativeObservation().ctx, "native-main", retirementOffer)).toMatchObject({ kind: "refuse", reason: "native_refused" });
  });

  it("observes main admitted submissions, queued async delivery, and every journal branch", () => {
    const n = nativeObservation();
    n.main.session.hasAdmittedSubmission = true;
    expect(readRetirementReadiness(n.pi, n.ctx, "native-main", retirementOffer)).toEqual({ kind: "defer", reason: "busy" });
    n.main.session.hasAdmittedSubmission = false;
    n.main.session.hasPendingAsyncWork = () => true;
    expect(readRetirementReadiness(n.pi, n.ctx, "native-main", retirementOffer)).toEqual({ kind: "defer", reason: "async_jobs" });
    n.main.session.hasPendingAsyncWork = () => false;
    n.sessionManager.getEntries = () => [
      { type: "message", parentId: "abandoned-branch", timestamp: retirementOffer.created_at, message: { role: "user", content: "keep my work" } },
    ];
    expect(readRetirementReadiness(n.pi, n.ctx, "native-main", retirementOffer)).toMatchObject({ kind: "refuse", reason: "user_activity" });
  });
});

describe("native main-only lifecycle hooks", () => {
  interface CliResponse { code: number; killed: boolean; stdout: string; stderr: string }

  function extensionHost() {
    const hooks = new Map<string, (event: unknown, ctx: ExtensionContext) => unknown>();
    const schema = {
      optional() { return schema; }, nullable() { return schema; }, describe() { return schema; },
      strict() { return schema; }, int() { return schema; }, min() { return schema; }, max() { return schema; },
    };
    const holdObservation = (signal?: AbortSignal) => {
      const pending = Promise.withResolvers<MainWaitRead>();
      signal?.addEventListener("abort", () => pending.resolve({
        mode: "retirement_only", retirement_token: "3".repeat(64),
        retirement: { ...retirementOffer, state: { state: "retained", at: retirementOffer.created_at, reason: "native_refused", native_stopped: false } },
      }), { once: true });
      return pending.promise;
    };
    const observations = vi.fn<(signal?: AbortSignal) => Promise<MainWaitRead>>()
      .mockResolvedValueOnce({ mode: "retirement_only", retirement: retirementOffer, retirement_token: OFFER_RETIREMENT_TOKEN })
      .mockImplementation(holdObservation);
    const exec = vi.fn(async (_cli: string, args: string[], options: { signal?: AbortSignal }) => {
      if (args[0] === "inbox" && args[1] === "wait") {
        const envelope = await observations(options.signal);
        return { code: 0, killed: false, stderr: "", stdout: JSON.stringify(envelope) };
      }
      return { code: 0, killed: false, stdout: "{}", stderr: "" };
    });
    const sessionManager = { getSessionId: () => "native-main", getEntries: () => [] as unknown[] };
    const mainSession = { sessionManager, isDisposed: false, hasAdmittedSubmission: false, queuedMessageCount: 0, hasPendingAsyncWork: () => false };
    const main = { id: "Main", kind: "main", status: "idle", session: mainSession };
    const pi = {
      zod: { object: () => schema, enum: () => schema, number: () => schema, string: () => schema, boolean: () => schema, array: () => schema },
      registerTool: vi.fn(), appendEntry: vi.fn(), sendUserMessage: vi.fn(), exec,
      on: (event: string, handler: (event: unknown, ctx: ExtensionContext) => unknown) => { hooks.set(event, handler); },
      pi: { MAIN_AGENT_ID: "Main", AgentRegistry: { global: () => ({ get: (id: string) => id === "Main" ? main : undefined, list: () => [main] }) } },
    } as unknown as ExtensionAPI;
    const shutdown = vi.fn();
    const ctx = {
      agent: { kind: "main", id: "Main" }, sessionManager, mode: "tui", cwd: "/tmp",
      isIdle: () => true, hasPendingMessages: () => false, getAsyncJobSnapshot: () => ({ running: [] }),
      ui: { getEditorText: () => "", notify: vi.fn() }, shutdown,
      setTimeout: vi.fn(() => 0), clearTimer: vi.fn(),
    } as unknown as ExtensionContext;
    cockpitOrchestration(pi);
    return { hooks, exec, observations, holdObservation, ctx, pi, shutdown, sessionManager, main };
  }

  it("rejects missing or inapplicable answer links before calling the CLI", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "message-root");
    try {
      const h = extensionHost();
      type MessageParams = { operation: "message" | "annotate" | "show"; run_id: string; kind?: "answer" | "instruction" | "cancel-request" | "report"; text?: string; in_reply_to?: string };
      type MessageTool = { name: string; execute: (id: string, params: MessageParams, signal: undefined, update: undefined, ctx: ExtensionContext) => Promise<unknown> };
      const tool = vi.mocked(h.pi.registerTool).mock.calls.map(([tool]) => tool as unknown as MessageTool)
        .find(tool => tool.name === "cockpit_message")!;
      const invalid: MessageParams[] = [
        { operation: "message", run_id: "worker", kind: "answer", text: "Answer" },
        ...["", " \t "].map(in_reply_to => ({ operation: "message" as const, run_id: "worker", kind: "answer" as const, text: "Answer", in_reply_to })),
        ...([undefined, "instruction", "cancel-request", "report"] as const).map(kind => ({ operation: "message" as const, run_id: "worker", kind, text: "Feedback", in_reply_to: "question-1" })),
        { operation: "annotate", run_id: "worker", text: "Feedback", in_reply_to: "question-1" },
        { operation: "show", run_id: "worker", in_reply_to: "question-1" },
      ];
      for (const params of invalid) {
        await expect(tool.execute("invalid", params, undefined, undefined, h.ctx)).rejects.toBeInstanceOf(Error);
      }
      expect(h.exec).not.toHaveBeenCalled();
    } finally { vi.unstubAllEnvs(); }
  });

  it("does not send a linked answer from a stale native supervisor binding", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "message-root");
    try {
      const h = extensionHost();
      const root = { run_id: "message-root", root_id: "message-root", parent_run_id: null,
        kind: "supervisor", stage: "active", bound_omp_session: "replaced-native-main" };
      h.exec.mockResolvedValue({ code: 0, killed: false, stderr: "", stdout: JSON.stringify(root) });
      type MessageTool = { name: string; execute: (id: string, params: { operation: "message"; run_id: string; kind: "answer"; text: string; in_reply_to: string; message_id: string }, signal: undefined, update: undefined, ctx: ExtensionContext) => Promise<unknown> };
      const tool = vi.mocked(h.pi.registerTool).mock.calls.map(([tool]) => tool as unknown as MessageTool)
        .find(tool => tool.name === "cockpit_message")!;
      await expect(tool.execute("stale", {
        operation: "message", run_id: "worker", kind: "answer", text: "Answer",
        in_reply_to: "question-1", message_id: "answer-1",
      }, undefined, undefined, h.ctx)).rejects.toBeInstanceOf(Error);
      expect(h.exec.mock.calls.some(([, args]) => args[0] === "run" && args[1] === "message")).toBe(false);
    } finally { vi.unstubAllEnvs(); }
  });

  it("rechecks the canonical attempt and prerequisites for every mutation while keeping recovery reports/reads callable", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "worker-gate");
    try {
      const h = extensionHost();
      const run = { run_id: "worker-gate", task_id: "task", root_id: "root", kind: "worker", stage: "working", close_reason: null,
        init_receipt: { kind: "ready" }, work_plan: { plan_revision: "plan" }, grants: [{ scope: "execute", plan_revision: "plan" }] };
      let view = { current_run_id: "worker-gate", task: { task_id: "task", checked: false, diagnostic: null },
        dependencies: { state: "none", unmet: [], problems: [] as TaskView["dependencies"]["problems"] } };
      h.exec.mockImplementation(async (_cli, args) => ({ code: 0, killed: false, stderr: "",
        stdout: JSON.stringify(args[0] === "run" ? run : view) }));
      const gate = h.hooks.get("tool_call")!;
      expect(await gate({ toolName: "edit", input: {} }, h.ctx)).toBeUndefined();
      view = { ...view, dependencies: { state: "none", unmet: [], problems: [
        { code: "task_follow_up_source_unavailable", message: "The follow-up source is no longer available." },
      ] } };
      expect(await gate({ toolName: "edit", input: {} }, h.ctx)).toBeUndefined();
      view = { ...view, current_run_id: "replacement-attempt" };
      for (const toolName of ["edit", "functions.edit", "bash", "functions.eval", "task", "cockpit_delegate", "multi_tool_use.parallel"]) {
        expect(await gate({ toolName, input: {} }, h.ctx)).toMatchObject({ block: true });
      }
      view = { ...view, current_run_id: "worker-gate", dependencies: { state: "blocked", unmet: [], problems: [] } };
      expect(await gate({ toolName: "cockpit_task", input: { operation: "step_set_checked" } }, h.ctx)).toMatchObject({ block: true });
      run.stage = "reported";
      for (const [toolName, operation] of [
        ["read", undefined], ["functions.read", undefined], ["cockpit_report", "result"],
        ["cockpit_report", "needs-input"], ["cockpit_task", "show"], ["cockpit_message", "show"],
      ]) expect(await gate({ toolName, input: { operation } }, h.ctx)).toBeUndefined();
    } finally { vi.unstubAllEnvs(); }
  });

  it("registers task mutations and retains caller UUIDs after an unknown create outcome without automatic retry", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "task-root");
    try {
      const h = extensionHost();
      type TaskTool = { name: string; execute: (id: string, params: TaskToolParams, signal: undefined, update: undefined, ctx: ExtensionContext) => Promise<unknown> };
      const tool = vi.mocked(h.pi.registerTool).mock.calls.map(([tool]) => tool as unknown as TaskTool)
        .find(tool => tool.name === "cockpit_task")!;
      const root = { run_id: "task-root", root_id: "task-root", parent_run_id: null, kind: "supervisor", stage: "active", bound_omp_session: "native-main" };
      h.exec.mockImplementation(async (_cli, args) => {
        if (args[0] === "run") return { code: 0, killed: false, stderr: "", stdout: JSON.stringify(root) };
        throw new Error("transport disconnected after submit");
      });
      const params: TaskToolParams = { operation: "create", task_id: "11111111-1111-4111-8111-111111111111", title: "Task", description: "" };
      await expect(tool.execute("first", params, undefined, undefined, h.ctx)).rejects.toThrow(`Retain task_id=${params.task_id}`);
      expect(h.exec.mock.calls.filter(([, args]) => args[0] === "task" && args[1] === "create")).toHaveLength(1);
      await expect(tool.execute("explicit-retry", params, undefined, undefined, h.ctx)).rejects.toThrow(params.task_id!);
      for (const [, args] of h.exec.mock.calls.filter(([, args]) => args[0] === "task")) {
        expect(args).toEqual(expect.arrayContaining(["--task-id", params.task_id, "--description", ""]));
        expect(args).not.toContain("--body");
      }
      const before = h.exec.mock.calls.length;
      await expect(tool.execute("old-body", { ...params, body: "replacement" } as TaskToolParams, undefined, undefined, h.ctx))
        .rejects.toThrow("body is read-only");
      expect(h.exec.mock.calls).toHaveLength(before);
      h.exec.mockResolvedValueOnce({ code: 0, killed: false, stderr: "", stdout: JSON.stringify({ ...root, kind: "worker",
        stage: "working", task_id: "task", close_reason: null, init_receipt: { kind: "ready" },
        work_plan: { plan_revision: "plan" }, grants: [{ scope: "execute", plan_revision: "plan" }] }) })
        .mockResolvedValueOnce({ code: 0, killed: false, stderr: "", stdout: JSON.stringify({
          current_run_id: "task-root", task: { task_id: "task", checked: false, diagnostic: null },
          dependencies: { state: "none", unmet: [], problems: [] },
        }) });
      await expect(tool.execute("worker-relations", { operation: "dependencies_set", task_id: "task",
        expected_task_revision: "item", expected_doc_revision: "doc", depends_on: [] }, undefined, undefined, h.ctx))
        .rejects.toThrow("native main supervisor");
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "dependencies-set")).toHaveLength(0);
    } finally { vi.unstubAllEnvs(); }
  });

  it("permits read-only context during initialization but fails closed after a context binding change", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "context-worker");
    vi.stubEnv("COCKPIT_HERDR_SOCKET", "/tmp/context-fixture.sock");
    try {
      const h = extensionHost();
      const run = { run_id: "context-worker", session_id: "herdr", kind: "worker", stage: "initializing",
        bound_omp_session: "native-main", setup: { project_workspace_id: "project" },
        location: { session_id: "herdr", workspace_id: "linked" } };
      const response = (value: unknown) => ({ code: 0, killed: false, stdout: JSON.stringify(value), stderr: "" });
      h.exec.mockResolvedValueOnce(response(run));
      expect(await h.hooks.get("tool_call")!({ toolName: "cockpit_context", input: {} }, h.ctx)).toBeUndefined();
      type ContextTool = { name: string; execute: (id: string, params: { offset?: number; limit?: number },
        signal: undefined, update: undefined, ctx: ExtensionContext) => Promise<unknown> };
      const tool = vi.mocked(h.pi.registerTool).mock.calls.map(([tool]) => tool as unknown as ContextTool)
        .find(tool => tool.name === "cockpit_context")!;
      h.exec.mockResolvedValueOnce(response(run));
      h.exec.mockResolvedValueOnce(response({
        target: { session_id: "herdr", space_id: "project" }, space_label: "Project", pane_id: null,
        library_root: "/library", items: [], checkout_path: "/repo", repository_paths: [], diagnostics: [],
      }));
      h.exec.mockResolvedValueOnce(response({ ...run, setup: { project_workspace_id: "different-project" } }));
      await expect(tool.execute("context", {}, undefined, undefined, h.ctx)).rejects.toThrow("binding changed");
      h.exec.mockResolvedValueOnce(response({ ...run, stage: "closed" }));
      await expect(tool.execute("context", {}, undefined, undefined, h.ctx)).rejects.toThrow("closed");
    } finally { vi.unstubAllEnvs(); }
  });


  it.each(["attempt_stale", "original_agent_present", "original_identity_unverifiable"])("does not repeat a refused scoped recovery after %s", async code => {
    vi.stubEnv("COCKPIT_RUN_ID", "recovery-root");
    try {
      const h = extensionHost();
      const root = { run_id: "recovery-root", root_id: "recovery-root", parent_run_id: null,
        kind: "supervisor", stage: "active", bound_omp_session: "native-main", location: { workspace_id: "supervisor-space" } };
      h.exec.mockResolvedValueOnce({ code: 0, killed: false, stdout: JSON.stringify(root), stderr: "" });
      h.exec.mockResolvedValueOnce({ code: 1, killed: false, stdout: "", stderr: JSON.stringify({ code, message: "Fresh original evidence refuses restart" }) });
      type ManageTool = { name: string; execute: (id: string, params: Parameters<typeof managementArgs>[0],
        signal: undefined, update: undefined, ctx: ExtensionContext) => Promise<unknown> };
      const tools = vi.mocked(h.pi.registerTool).mock.calls.map(([tool]) => tool as unknown as ManageTool);
      const manage = tools.find(tool => tool.name === "cockpit_manage")!;
      await expect(manage.execute("call", { operation: "retry_launch", run_id: "worker" }, undefined, undefined, h.ctx))
        .rejects.toThrow("Fresh original evidence refuses restart");
      expect(h.exec).toHaveBeenCalledTimes(2);
      expect(h.exec.mock.calls[1][1]).not.toContain("--operator");
    } finally { vi.unstubAllEnvs(); }
  });

  it("rejects stale supervisor binding before issuing a recovery command", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "recovery-root");
    try {
      const h = extensionHost();
      const root = { run_id: "recovery-root", root_id: "recovery-root", parent_run_id: null,
        kind: "supervisor", stage: "active", bound_omp_session: "old-native-main", location: { workspace_id: "supervisor-space" } };
      h.exec.mockResolvedValueOnce({ code: 0, killed: false, stdout: JSON.stringify(root), stderr: "" });
      type ManageTool = { name: string; execute: (id: string, params: Parameters<typeof managementArgs>[0],
        signal: undefined, update: undefined, ctx: ExtensionContext) => Promise<unknown> };
      const tools = vi.mocked(h.pi.registerTool).mock.calls.map(([tool]) => tool as unknown as ManageTool);
      await expect(tools.find(tool => tool.name === "cockpit_manage")!.execute(
        "call", { operation: "reconcile", run_id: "worker" }, undefined, undefined, h.ctx,
      )).rejects.toThrow("bound native main");
      expect(h.exec).toHaveBeenCalledTimes(1);
    } finally { vi.unstubAllEnvs(); }
  });

  it("does not bind, observe, or retire an internal clone with inherited run environment", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    try {
      const h = extensionHost();
      const child = { ...h.ctx, agent: { ...h.ctx.agent, kind: "sub" as const } };
      await h.hooks.get("session_start")!({}, child);
      expect(h.exec).not.toHaveBeenCalled();
      expect(h.shutdown).not.toHaveBeenCalled();
    } finally { vi.unstubAllEnvs(); }
  });

  it("observes retirement through the existing inbox wait after PID bind, without a prompt wake", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const requested = Promise.withResolvers<void>();
      h.shutdown.mockImplementation(requested.resolve);
      await h.hooks.get("session_start")!({}, h.ctx);
      await requested.promise;
      expect(h.exec.mock.calls[0][1]).toEqual(expect.arrayContaining(["run", "bind-session", "--omp-pid", String(process.pid)]));
      const wait = h.exec.mock.calls.find(([, args]) => args[0] === "inbox" && args[1] === "wait");
      expect(wait?.[1]).toEqual(expect.arrayContaining(["--with-retirement", "--omp-pid", String(process.pid), "--timeout", "30"]));
      expect(h.exec.mock.calls.some(([, args]) => args[0] === "run" && args[1] === "retirement")).toBe(false);
      const receipt = h.exec.mock.calls.find(([, args]) => args[1] === "retirement-receipt");
      expect(receipt?.[1]).toEqual(expect.arrayContaining(["--omp-pid", String(process.pid), "--retirement", "retirement", "--shutdown-requested"]));
      expect(h.shutdown).toHaveBeenCalledTimes(1);
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it.each(["caller_mismatch", "session_mismatch", "attempt_stale"])("does not start any watcher or shutdown when native PID binding fails with %s", async code => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      h.exec.mockResolvedValueOnce({ code: 1, killed: false, stdout: "", stderr: JSON.stringify({ code, message: "This native binding is no longer authorized." }) });
      await h.hooks.get("session_start")!({}, h.ctx);
      expect(h.exec).toHaveBeenCalledTimes(1);
      expect(h.shutdown).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it.each(["caller_not_ready", "transport"])("recovers native binding after a transient %s failure with bounded silent backoff", async failure => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    vi.useFakeTimers();
    const h = extensionHost();
    try {
      Object.assign(h.ctx, { setTimeout: vi.fn(setTimeout), clearTimer: vi.fn(clearTimeout) });
      h.observations.mockReset().mockImplementation(h.holdObservation);
      h.exec.mockResolvedValueOnce({
        code: 1, killed: false, stdout: "",
        stderr: failure === "transport" ? "connection closed before a response" : JSON.stringify({ code: failure, message: "Native startup evidence is pending." }),
      });
      const started = h.hooks.get("session_start")!({}, h.ctx);
      await vi.advanceTimersByTimeAsync(1_999);
      expect(h.exec).toHaveBeenCalledTimes(1);
      expect(h.observations).not.toHaveBeenCalled();
      expect(h.ctx.ui.notify).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      await started;
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "bind-session")).toHaveLength(2);
      expect(h.observations).toHaveBeenCalledTimes(1);
      expect(h.exec.mock.calls.every(([, args]) => args.includes("--omp-session") && args[args.indexOf("--omp-session") + 1] === "native-main")).toBe(true);
      expect(h.exec.mock.calls[0][2]).toMatchObject({ timeout: 35_000, signal: expect.any(AbortSignal) });
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
      expect(h.shutdown).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
      vi.useRealTimers();
    }
  });

  it("keeps an empty startup inbox observed across pending attestation and wakes delayed mail without reading or acknowledging it", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    vi.useFakeTimers();
    const h = extensionHost();
    try {
      Object.assign(h.ctx, { setTimeout: vi.fn(setTimeout), clearTimer: vi.fn(clearTimeout) });
      const delayed = Promise.withResolvers<MainWaitRead>();
      const waitingForMail = Promise.withResolvers<void>();
      const continued = Promise.withResolvers<void>();
      h.observations.mockReset()
        .mockResolvedValueOnce({
          mode: "open", inbox: { run_id: "retirement-test-run", pending: false, through_seq: 0, counts: [] },
          retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN,
        })
        .mockImplementationOnce(() => { waitingForMail.resolve(); return delayed.promise; })
        .mockImplementation(signal => { continued.resolve(); return h.holdObservation(signal); });
      const original = h.exec.getMockImplementation()!;
      let waits = 0;
      h.exec.mockImplementation(async (cli, args, options) => {
        if (args[0] === "inbox" && args[1] === "wait" && ++waits <= 2) {
          return { code: 1, killed: false, stdout: "", stderr: JSON.stringify({ code: "caller_not_ready", message: "Herdr has not yet attested this native." }) };
        }
        return original(cli, args, options);
      });
      await h.hooks.get("session_start")!({}, h.ctx);
      await vi.advanceTimersByTimeAsync(1_999);
      expect(waits).toBe(1);
      await vi.advanceTimersByTimeAsync(1);
      expect(waits).toBe(2);
      await vi.advanceTimersByTimeAsync(2_000);
      await waitingForMail.promise;
      expect(waits).toBe(4);
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
      expect(h.ctx.ui.notify).not.toHaveBeenCalled();
      delayed.resolve({
        mode: "open", inbox: { run_id: "retirement-test-run", pending: true, through_seq: 7, counts: [{ kind: "instruction", count: 1 }] },
        retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN,
      });
      await continued.promise;
      expect(h.pi.sendUserMessage).toHaveBeenCalledTimes(1);
      expect(h.pi.sendUserMessage).toHaveBeenCalledWith(expect.stringContaining("through sequence 7"), { deliverAs: "aside" });
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "woken")).toHaveLength(1);
      expect(h.exec.mock.calls.some(([, args]) => args[0] === "inbox" && ["list", "ack"].includes(args[1]))).toBe(false);
      expect(h.pi.appendEntry).toHaveBeenLastCalledWith("cockpit-orchestration-wake-v1", expect.objectContaining({ seen: 7, readThrough: 0, ackedThrough: 0, queued: true }));
      expect(h.shutdown).not.toHaveBeenCalled();
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      await vi.advanceTimersByTimeAsync(10_000);
      expect(waits).toBe(5);
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
      vi.useRealTimers();
    }
  });

  it("cancels pending native binding on shutdown without starting an observer from its late completion", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const binding = Promise.withResolvers<CliResponse>();
      h.exec.mockReturnValueOnce(binding.promise);
      const started = h.hooks.get("session_start")!({}, h.ctx);
      const signal = h.exec.mock.calls[0][2].signal!;
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      expect(signal.aborted).toBe(true);
      binding.resolve({ code: 0, killed: false, stdout: "{}", stderr: "" });
      await started;
      expect(h.observations).not.toHaveBeenCalled();
      expect(h.ctx.ui.notify).not.toHaveBeenCalled();
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("cancels startup backoff on shutdown instead of retrying its exact native binding", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    vi.useFakeTimers();
    const h = extensionHost();
    try {
      Object.assign(h.ctx, { setTimeout: vi.fn(setTimeout), clearTimer: vi.fn(clearTimeout) });
      h.exec.mockResolvedValueOnce({ code: 1, killed: false, stdout: "", stderr: JSON.stringify({ code: "caller_not_ready", message: "Startup pending." }) });
      const started = h.hooks.get("session_start")!({}, h.ctx);
      await vi.advanceTimersByTimeAsync(0);
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      await started;
      await vi.advanceTimersByTimeAsync(10_000);
      expect(h.exec).toHaveBeenCalledTimes(1);
      expect(h.ctx.clearTimer).toHaveBeenCalledTimes(1);
      expect(h.observations).not.toHaveBeenCalled();
      expect(h.ctx.ui.notify).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
      vi.useRealTimers();
    }
  });

  it("reloads the exact native session once and rejects the superseded startup completion", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const binding = Promise.withResolvers<CliResponse>();
      h.exec.mockReturnValueOnce(binding.promise);
      h.observations.mockReset().mockImplementation(h.holdObservation);
      const oldStart = h.hooks.get("session_start")!({}, h.ctx);
      const oldSignal = h.exec.mock.calls[0][2].signal!;
      await h.hooks.get("session_start")!({}, h.ctx);
      expect(oldSignal.aborted).toBe(true);
      binding.resolve({ code: 0, killed: false, stdout: "{}", stderr: "" });
      await oldStart;
      expect(h.observations).toHaveBeenCalledTimes(1);
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "bind-session")).toHaveLength(2);
      const observedSignal = h.exec.mock.calls.find(([, args]) => args[0] === "inbox" && args[1] === "wait")![2].signal!;
      await h.hooks.get("session_start")!({}, h.ctx);
      expect(observedSignal.aborted).toBe(true);
      expect(h.observations).toHaveBeenCalledTimes(2);
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "bind-session")).toHaveLength(3);
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
      expect(h.shutdown).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it.each(["new", "resume", "fork"])("rebinds a completed %s switch and observes only the current native inbox", async reason => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    vi.useFakeTimers();
    const h = extensionHost();
    try {
      const oldMail = Promise.withResolvers<MainWaitRead>();
      const currentWait = Promise.withResolvers<void>();
      h.observations.mockReset()
        .mockReturnValueOnce(oldMail.promise)
        .mockResolvedValueOnce({
          mode: "open", inbox: { run_id: "retirement-test-run", pending: true, through_seq: 8, counts: [{ kind: "instruction", count: 1 }] },
          retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN,
        })
        .mockImplementation(signal => { currentWait.resolve(); return h.holdObservation(signal); });
      await h.hooks.get("session_start")!({}, h.ctx);
      const oldWait = h.exec.mock.results[1].value;
      const oldSignal = h.exec.mock.calls[1][2].signal!;
      h.sessionManager.getSessionId = () => "switched-main";
      await h.hooks.get("session_switch")!({ reason, previousSessionFile: "/tmp/previous.jsonl" }, h.ctx);
      await currentWait.promise;
      expect(oldSignal.aborted).toBe(true);
      const binds = h.exec.mock.calls.filter(([, args]) => args[1] === "bind-session");
      expect(binds).toHaveLength(2);
      expect(binds[1][1]).toEqual(expect.arrayContaining([
        "--omp-session", "switched-main", "--omp-main-session", "switched-main", "--agent-kind", "main",
        "--omp-pid", String(process.pid),
      ]));
      const currentSignal = binds[1][2].signal!;
      expect(currentSignal).not.toBe(oldSignal);
      expect(currentSignal.aborted).toBe(false);
      const currentCalls = h.exec.mock.calls.slice(2);
      expect(currentCalls.every(([, args]) => args[args.indexOf("--omp-session") + 1] === "switched-main")).toBe(true);
      expect(h.pi.sendUserMessage).toHaveBeenCalledTimes(1);
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "woken")).toHaveLength(1);
      oldMail.resolve({
        mode: "open", inbox: { run_id: "retirement-test-run", pending: true, through_seq: 99, counts: [{ kind: "instruction", count: 1 }] },
        retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN,
      });
      await oldWait;
      await vi.advanceTimersByTimeAsync(0);
      expect(h.observations).toHaveBeenCalledTimes(3);
      expect(h.pi.sendUserMessage).toHaveBeenCalledTimes(1);
      expect(h.pi.appendEntry).toHaveBeenLastCalledWith("cockpit-orchestration-wake-v1", expect.objectContaining({ seen: 8 }));
      expect(h.exec.mock.calls.some(([, args]) => args[0] === "inbox" && ["list", "ack"].includes(args[1]))).toBe(false);
      expect(h.shutdown).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
      vi.useRealTimers();
    }
  });

  it("supersedes an unresolved startup bind on switch without restarting its stale observer", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const binding = Promise.withResolvers<CliResponse>();
      h.exec.mockReturnValueOnce(binding.promise);
      h.observations.mockReset().mockImplementation(h.holdObservation);
      const startup = h.hooks.get("session_start")!({}, h.ctx);
      const startupSignal = h.exec.mock.calls[0][2].signal!;
      h.sessionManager.getSessionId = () => "switched-main";
      await h.hooks.get("session_switch")!({ reason: "new" }, h.ctx);
      expect(startupSignal.aborted).toBe(true);
      binding.resolve({ code: 0, killed: false, stdout: "{}", stderr: "" });
      await startup;
      expect(h.observations).toHaveBeenCalledTimes(1);
      const waits = h.exec.mock.calls.filter(([, args]) => args[0] === "inbox" && args[1] === "wait");
      expect(waits).toHaveLength(1);
      expect(waits[0][1]).toEqual(expect.arrayContaining(["--omp-session", "switched-main", "--omp-main-session", "switched-main"]));
      expect(waits[0][2].signal!.aborted).toBe(false);
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
      expect(h.ctx.ui.notify).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("keeps the original observer functional when a before-switch transition is cancelled", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const mail = Promise.withResolvers<MainWaitRead>();
      const continued = Promise.withResolvers<void>();
      h.observations.mockReset().mockReturnValueOnce(mail.promise)
        .mockImplementation(signal => { continued.resolve(); return h.holdObservation(signal); });
      await h.hooks.get("session_start")!({}, h.ctx);
      const signal = h.exec.mock.calls[1][2].signal!;
      await h.hooks.get("session_before_switch")?.({ reason: "new" }, h.ctx);
      // A cancelled native transition emits no session_switch and keeps its ID.
      mail.resolve({
        mode: "open", inbox: { run_id: "retirement-test-run", pending: true, through_seq: 7, counts: [{ kind: "instruction", count: 1 }] },
        retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN,
      });
      await continued.promise;
      expect(signal.aborted).toBe(false);
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "bind-session")).toHaveLength(1);
      expect(h.pi.sendUserMessage).toHaveBeenCalledTimes(1);
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "woken")).toHaveLength(1);
      expect(h.observations).toHaveBeenCalledTimes(2);
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("does not promote a switched native child or replace the process main-session identity", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "switch-child-run");
    const h = extensionHost();
    try {
      h.sessionManager.getSessionId = () => "actual-main";
      h.observations.mockReset().mockImplementation(h.holdObservation);
      await h.hooks.get("session_start")!({}, h.ctx);
      const mainSignal = h.exec.mock.calls[1][2].signal!;
      const childHost = extensionHost();
      childHost.sessionManager.getSessionId = () => "switched-child";
      const child = { ...childHost.ctx, agent: { kind: "sub" as const, id: "native-child", parentId: "Main" } };
      const childRef = { id: "native-child", kind: "sub", status: "running", session: { sessionManager: childHost.sessionManager, isDisposed: false } };
      Object.assign(childHost.pi.pi.AgentRegistry, { global: () => ({
        get: (id: string) => id === "Main" ? h.main : id === "native-child" ? childRef : undefined,
      }) });
      await childHost.hooks.get("session_start")!({}, child);
      await childHost.hooks.get("session_switch")!({ reason: "fork" }, child);
      expect(childHost.exec).not.toHaveBeenCalled();
      expect(childHost.observations).not.toHaveBeenCalled();
      expect(childHost.shutdown).not.toHaveBeenCalled();
      expect(mainSignal.aborted).toBe(false);
      childHost.exec.mockResolvedValueOnce({
        code: 0, killed: false, stderr: "",
        stdout: JSON.stringify({ run_id: "switch-child-run", kind: "supervisor", stage: "active" }),
      });
      expect(await childHost.hooks.get("tool_call")!({ toolName: "read", input: {} }, child)).toBeUndefined();
      expect(childHost.exec.mock.calls[0][1]).toEqual(expect.arrayContaining([
        "--omp-session", "switched-child", "--omp-main-session", "actual-main", "--agent-kind", "subagent", "--subagent-id", "native-child",
      ]));
      expect(h.observations).toHaveBeenCalledTimes(1);
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("does not transfer a delayed startup binding into a replacement native session", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const binding = Promise.withResolvers<CliResponse>();
      h.exec.mockReturnValueOnce(binding.promise);
      const started = h.hooks.get("session_start")!({}, h.ctx);
      h.sessionManager.getSessionId = () => "replacement-native";
      binding.resolve({ code: 0, killed: false, stdout: "{}", stderr: "" });
      await started;
      expect(h.observations).not.toHaveBeenCalled();
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
      expect(h.ctx.ui.notify).not.toHaveBeenCalled();
    } finally {
      h.sessionManager.getSessionId = () => "native-main";
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("discards delayed inbox mail after the actual native session changes without waking or autoACKing the replacement", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    vi.useFakeTimers();
    const h = extensionHost();
    try {
      const delayed = Promise.withResolvers<MainWaitRead>();
      h.observations.mockReset().mockReturnValueOnce(delayed.promise).mockImplementation(h.holdObservation);
      await h.hooks.get("session_start")!({}, h.ctx);
      h.sessionManager.getSessionId = () => "replacement-native";
      delayed.resolve({
        mode: "open", inbox: { run_id: "retirement-test-run", pending: true, through_seq: 7, counts: [{ kind: "instruction", count: 1 }] },
        retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN,
      });
      await vi.advanceTimersByTimeAsync(10_000);
      expect(h.observations).toHaveBeenCalledTimes(1);
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
      expect(h.pi.appendEntry).not.toHaveBeenCalled();
      expect(h.exec.mock.calls.some(([, args]) => args[0] === "inbox" && ["woken", "list", "ack"].includes(args[1]))).toBe(false);
      expect(h.shutdown).not.toHaveBeenCalled();
    } finally {
      h.sessionManager.getSessionId = () => "native-main";
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
      vi.useRealTimers();
    }
  });

  it("preserves ordinary open-inbox wakes, then consumes acceptance through that same wait", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const acceptance = Promise.withResolvers<MainWaitRead>();
      const waiting = Promise.withResolvers<void>();
      const woke = Promise.withResolvers<void>();
      const requested = Promise.withResolvers<void>();
      h.shutdown.mockImplementation(requested.resolve);
      vi.mocked(h.pi.sendUserMessage).mockImplementation(() => { woke.resolve(); });
      h.observations.mockReset()
        .mockResolvedValueOnce({ mode: "open", inbox: { run_id: "retirement-test-run", pending: true, through_seq: 7, counts: [{ kind: "report", count: 1 }] }, retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN })
        .mockImplementationOnce(() => { waiting.resolve(); return acceptance.promise; })
        .mockImplementation(h.holdObservation);
      await h.hooks.get("session_start")!({}, h.ctx);
      await woke.promise;
      await waiting.promise;
      const waits = h.exec.mock.calls.filter(([, args]) => args[0] === "inbox" && args[1] === "wait");
      expect(waits[0][1]).not.toContain("--after-retirement");
      expect(waits[1][1]).toEqual(expect.arrayContaining(["--after-retirement", EMPTY_RETIREMENT_TOKEN]));
      expect(h.pi.sendUserMessage).toHaveBeenCalledTimes(1);
      expect(h.shutdown).not.toHaveBeenCalled();
      acceptance.resolve({ mode: "retirement_only", retirement: retirementOffer, retirement_token: OFFER_RETIREMENT_TOKEN });
      await requested.promise;
      expect(h.pi.sendUserMessage).toHaveBeenCalledTimes(1);
      expect(h.shutdown).toHaveBeenCalledTimes(1);
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "retirement")).toHaveLength(0);
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("waits on the shared observation while deferred and sends only a changed reason", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      h.ctx.isIdle = () => false;
      const next = Promise.withResolvers<MainWaitRead>();
      const waiting = Promise.withResolvers<void>();
      const processed = Promise.withResolvers<void>();
      h.observations.mockImplementationOnce(() => { waiting.resolve(); return next.promise; })
        .mockImplementationOnce(signal => { processed.resolve(); return h.holdObservation(signal); });
      const journal = vi.spyOn(h.sessionManager, "getEntries");
      await h.hooks.get("session_start")!({}, h.ctx);
      await waiting.promise;
      expect(journal).toHaveBeenCalledTimes(2); // recovery plus the first offer
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "retirement-receipt")).toHaveLength(1);
      expect(h.shutdown).not.toHaveBeenCalled();
      next.resolve({ mode: "retirement_only", retirement_token: DEFER_RETIREMENT_TOKEN, retirement: { ...retirementOffer, state: {
        state: "native_stop_deferred", offered_at: retirementOffer.created_at, at: retirementOffer.updated_at, reason: "busy",
      } } });
      await processed.promise;
      expect(journal).toHaveBeenCalledTimes(3);
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "retirement-receipt")).toHaveLength(1);
      const waits = h.exec.mock.calls.filter(([, args]) => args[0] === "inbox" && args[1] === "wait");
      expect(waits[2][1]).toEqual(expect.arrayContaining(["--after-retirement", DEFER_RETIREMENT_TOKEN]));
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("never exits when the real CLI receipt rejects, including later agent_end", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const notice = Promise.withResolvers<void>();
      vi.mocked(h.ctx.ui.notify).mockImplementation(() => { notice.resolve(); });
      const original = h.exec.getMockImplementation()!;
      h.exec.mockImplementation(async (cli, args, options) => args[1] === "retirement-receipt"
        ? { code: 1, killed: false, stdout: "", stderr: "retirement_state_changed" }
        : original(cli, args, options));
      await h.hooks.get("session_start")!({}, h.ctx);
      await notice.promise;
      await h.hooks.get("agent_end")!({ willContinue: false, messages: [] }, h.ctx);
      expect(h.shutdown).not.toHaveBeenCalled();
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
      expect(h.ctx.setTimeout).toHaveBeenCalled(); // existing bounded error backoff
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("re-requests on the current main agent_end only while native input remains safe", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const requested = Promise.withResolvers<void>();
      h.shutdown.mockImplementation(requested.resolve);
      await h.hooks.get("session_start")!({}, h.ctx);
      await requested.promise;
      await h.hooks.get("agent_end")!({ willContinue: true, messages: [] }, h.ctx);
      expect(h.shutdown).toHaveBeenCalledTimes(1);
      await h.hooks.get("agent_end")!({ willContinue: false, messages: [] }, h.ctx);
      expect(h.shutdown).toHaveBeenCalledTimes(2);
      h.ctx.ui.getEditorText = () => "new unsent draft";
      await h.hooks.get("agent_end")!({ willContinue: false, messages: [] }, h.ctx);
      expect(h.shutdown).toHaveBeenCalledTimes(2);
      h.ctx.ui.getEditorText = () => "";
      h.sessionManager.getEntries = () => [{ type: "message", timestamp: retirementOffer.created_at, message: { role: "user", content: "[Cockpit inbox notification]" } }];
      await h.hooks.get("agent_end")!({ willContinue: false, messages: [] }, h.ctx);
      expect(h.shutdown).toHaveBeenCalledTimes(2);
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("discards a late accepted observation after this exact session lifecycle shuts down", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const observation = Promise.withResolvers<MainWaitRead>();
      h.observations.mockReset().mockReturnValueOnce(observation.promise).mockImplementation(h.holdObservation);
      await h.hooks.get("session_start")!({}, h.ctx);
      const pendingWait = h.exec.mock.results.find((_, index) => h.exec.mock.calls[index][1][0] === "inbox")!.value;
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      observation.resolve({ mode: "retirement_only", retirement: retirementOffer, retirement_token: OFFER_RETIREMENT_TOKEN });
      await pendingWait;
      expect(h.shutdown).not.toHaveBeenCalled();
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "retirement-receipt")).toHaveLength(0);
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("does not re-request shutdown after the shared scope observation becomes unavailable", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const requested = Promise.withResolvers<void>();
      const notice = Promise.withResolvers<void>();
      h.shutdown.mockImplementation(requested.resolve);
      vi.mocked(h.ctx.ui.notify).mockImplementation(() => { notice.resolve(); });
      h.observations.mockRejectedValueOnce(new Error("caller_mismatch"));
      await h.hooks.get("session_start")!({}, h.ctx);
      await requested.promise;
      await notice.promise;
      await h.hooks.get("agent_end")!({ willContinue: false, messages: [] }, h.ctx);
      expect(h.shutdown).toHaveBeenCalledTimes(1);
      expect(h.ctx.setTimeout).toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("sends a typed user-activity refusal for native generated wakes without creating an accepted wake", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      const processed = Promise.withResolvers<void>();
      h.sessionManager.getEntries = () => [{
        type: "message", timestamp: retirementOffer.created_at,
        message: { role: "user", content: "[Cockpit inbox notification]\n1 pending inbox message(s), through sequence 12." },
      }];
      h.observations.mockImplementationOnce(signal => { processed.resolve(); return h.holdObservation(signal); });
      await h.hooks.get("session_start")!({}, h.ctx);
      await processed.promise;
      const receipt = h.exec.mock.calls.find(([, args]) => args[1] === "retirement-receipt");
      expect(receipt?.[1]).toEqual(expect.arrayContaining(["--refuse-reason", "user_activity", "--refused"]));
      expect(h.shutdown).not.toHaveBeenCalled();
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("retries busy-to-idle readiness through agent_end without starting another observer", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      h.ctx.isIdle = () => false;
      const waiting = Promise.withResolvers<void>();
      h.observations.mockImplementationOnce(signal => { waiting.resolve(); return h.holdObservation(signal); });
      await h.hooks.get("session_start")!({}, h.ctx);
      await waiting.promise;
      expect(h.shutdown).not.toHaveBeenCalled();
      h.ctx.isIdle = () => true;
      await h.hooks.get("agent_end")!({ willContinue: false, messages: [] }, h.ctx);
      expect(h.shutdown).toHaveBeenCalledTimes(1);
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "retirement-receipt")).toHaveLength(2);
      expect(h.observations).toHaveBeenCalledTimes(2); // existing second wait stays in flight
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("rejects cached-offer authority when the fresh agent_end receipt is revoked", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      h.ctx.isIdle = () => false;
      const waiting = Promise.withResolvers<void>();
      h.observations.mockImplementationOnce(signal => { waiting.resolve(); return h.holdObservation(signal); });
      await h.hooks.get("session_start")!({}, h.ctx);
      await waiting.promise;
      const original = h.exec.getMockImplementation()!;
      h.exec.mockImplementation(async (cli, args, options) => args[1] === "retirement-receipt"
        ? { code: 1, killed: false, stdout: "", stderr: "retirement_state_changed" }
        : original(cli, args, options));
      h.ctx.isIdle = () => true;
      await h.hooks.get("agent_end")!({ willContinue: false, messages: [] }, h.ctx);
      expect(h.shutdown).not.toHaveBeenCalled();
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
      expect(h.ctx.ui.notify).toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("uses the existing bounded wait deadline to retry a cleared non-turn draft", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    const h = extensionHost();
    try {
      h.ctx.ui.getEditorText = () => "draft";
      const deadline = Promise.withResolvers<MainWaitRead>();
      const waiting = Promise.withResolvers<void>();
      const requested = Promise.withResolvers<void>();
      h.shutdown.mockImplementation(requested.resolve);
      h.observations.mockImplementationOnce(() => { waiting.resolve(); return deadline.promise; });
      await h.hooks.get("session_start")!({}, h.ctx);
      await waiting.promise;
      const waits = h.exec.mock.calls.filter(([, args]) => args[0] === "inbox" && args[1] === "wait");
      expect(waits[0][1]).not.toContain("--after-retirement");
      expect(waits[1][1]).toEqual(expect.arrayContaining(["--after-retirement", OFFER_RETIREMENT_TOKEN]));
      expect(h.shutdown).not.toHaveBeenCalled();
      h.ctx.ui.getEditorText = () => "";
      deadline.resolve({ mode: "retirement_only", retirement: retirementOffer, retirement_token: OFFER_RETIREMENT_TOKEN });
      await requested.promise;
      expect(h.shutdown).toHaveBeenCalledTimes(1);
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
    }
  });

  it("fails closed before effects for malformed envelopes, tokens, and closed inbox leakage", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    try {
      for (const malformed of [
        { mode: "retirement_only", retirement: retirementOffer, retirement_token: "bad-token" },
        { mode: "retirement_only", retirement: retirementOffer, retirement_token: OFFER_RETIREMENT_TOKEN, inbox: { messages: ["secret"] } },
        { mode: "retirement_only", retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN },
        { mode: "retirement_only", retirement: { ...retirementOffer, identity: null }, retirement_token: OFFER_RETIREMENT_TOKEN },
      ]) {
        const h = extensionHost();
        try {
          const notice = Promise.withResolvers<void>();
          vi.mocked(h.ctx.ui.notify).mockImplementation(() => { notice.resolve(); });
          h.exec.mockImplementation(async (_cli, args) => ({
            code: 0, killed: false, stderr: "", stdout: args[0] === "inbox" ? JSON.stringify(malformed) : "{}",
          }));
          await h.hooks.get("session_start")!({}, h.ctx);
          await notice.promise;
          expect(h.shutdown).not.toHaveBeenCalled();
          expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
          expect(h.exec.mock.calls.filter(([, args]) => args[1] === "retirement-receipt")).toHaveLength(0);
        } finally { await h.hooks.get("session_shutdown")!({}, h.ctx); }
      }
    } finally { vi.unstubAllEnvs(); }
  });

  it.each(["caller_mismatch", "session_mismatch", "attempt_stale"])("disposes an observer revoked with %s without shutting down the native or trusting its cached offer", async code => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    vi.useFakeTimers();
    const h = extensionHost();
    try {
      Object.assign(h.ctx, { setTimeout: vi.fn(setTimeout), clearTimer: vi.fn(clearTimeout) });
      h.ctx.ui.getEditorText = () => "unsent draft";
      const original = h.exec.getMockImplementation()!;
      const revoked = JSON.stringify({ code, message: "This native is no longer authorized to observe its run." });
      let waits = 0;
      h.exec.mockImplementation(async (cli, args, options) => {
        if (args[0] === "inbox" && args[1] === "wait" && ++waits > 1) {
          return { code: 1, killed: false, stdout: "", stderr: revoked };
        }
        return original(cli, args, options);
      });
      await h.hooks.get("session_start")!({}, h.ctx);
      await vi.advanceTimersByTimeAsync(10_000);
      expect(waits).toBe(2);
      expect(h.ctx.ui.notify).toHaveBeenCalledTimes(1);
      expect(vi.mocked(h.ctx.ui.notify).mock.calls[0][0]).toContain(revoked);
      expect(h.ctx.setTimeout).not.toHaveBeenCalled();
      h.ctx.ui.getEditorText = () => "";
      await h.hooks.get("agent_end")!({ willContinue: false, messages: [] }, h.ctx);
      expect(h.shutdown).not.toHaveBeenCalled();
      expect(h.exec.mock.calls.filter(([, args]) => args[1] === "retirement-receipt")).toHaveLength(1);
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
      vi.useRealTimers();
    }
  });

  it("recovers an ordinary inbox wake after an unstructured caller-mismatch transport error", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    vi.useFakeTimers();
    const h = extensionHost();
    try {
      Object.assign(h.ctx, { setTimeout: vi.fn(setTimeout), clearTimer: vi.fn(clearTimeout) });
      h.observations.mockReset().mockImplementation(h.holdObservation);
      const original = h.exec.getMockImplementation()!;
      let waits = 0;
      h.exec.mockImplementation(async (cli, args, options) => {
        if (args[0] === "inbox" && args[1] === "wait") {
          waits++;
          if (waits === 1) return { code: 1, killed: false, stdout: "", stderr: "transport lost before decoding caller_mismatch" };
          if (waits === 2) return { code: 0, killed: false, stderr: "", stdout: JSON.stringify({
            mode: "open", inbox: { run_id: "retirement-test-run", pending: true, through_seq: 7, counts: [{ kind: "report", count: 1 }] },
            retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN,
          }) };
        }
        return original(cli, args, options);
      });
      await h.hooks.get("session_start")!({}, h.ctx);
      await vi.advanceTimersByTimeAsync(1_999);
      expect(waits).toBe(1);
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(waits).toBe(3);
      expect(h.pi.sendUserMessage).toHaveBeenCalledTimes(1);
      expect(h.shutdown).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
      vi.useRealTimers();
    }
  });

  it("stops on durable timeout retention and never retries a now-terminal cached offer", async () => {
    vi.stubEnv("COCKPIT_RUN_ID", "retirement-test-run");
    vi.useFakeTimers();
    const h = extensionHost();
    try {
      h.ctx.isIdle = () => false;
      const timedOut = Promise.withResolvers<void>();
      h.observations.mockImplementationOnce(async () => {
        timedOut.resolve();
        return { mode: "retirement_only", retirement_token: DEFER_RETIREMENT_TOKEN, retirement: { ...retirementOffer, state: {
          state: "retained", at: retirementOffer.updated_at, reason: "worker_busy_timeout", native_stopped: false,
        } } };
      });
      await h.hooks.get("session_start")!({}, h.ctx);
      await timedOut.promise;
      const terminalWait = h.exec.mock.results[h.exec.mock.results.length - 1].value;
      const terminalResponse = await terminalWait;
      expect(parseMainWaitRead(JSON.parse(terminalResponse.stdout))).toMatchObject({
        mode: "retirement_only", retirement: { state: { state: "retained", reason: "worker_busy_timeout" } },
      });
      // CLI completion precedes wakeLoop's async envelope consumption. Drain
      // that completed turn before simulating the later native agent_end.
      await vi.runAllTimersAsync();
      h.exec.mockClear();
      h.ctx.isIdle = () => true;
      await h.hooks.get("agent_end")!({ willContinue: false, messages: [] }, h.ctx);
      expect(h.shutdown).not.toHaveBeenCalled();
      expect(h.exec.mock.calls.some(([, args]) => args[1] === "retirement-receipt")).toBe(false);
      expect(h.pi.sendUserMessage).not.toHaveBeenCalled();
    } finally {
      await h.hooks.get("session_shutdown")!({}, h.ctx);
      vi.unstubAllEnvs();
      vi.useRealTimers();
    }
  });
});

describe("shared main-wait boundary parsing", () => {
  const open: MainWaitRead = {
    mode: "open", inbox: { run_id: "run", pending: false, through_seq: 0, counts: [] },
    retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN,
  };
  const closed: MainWaitRead = { mode: "retirement_only", retirement: retirementOffer, retirement_token: OFFER_RETIREMENT_TOKEN };

  it("accepts only the approved mode/token envelope, with normal counts separated from retirement", () => {
    expect(parseMainWaitRead(open)).toEqual(open);
    expect(parseMainWaitRead(closed)).toEqual(closed);
    for (const retirement_token of ["short", "A".repeat(64), "g".repeat(64), "a".repeat(63), null, 3]) {
      expect(() => parseMainWaitRead({ ...closed, retirement_token })).toThrow();
    }
    for (const field of ["inbox", "counts", "pending", "messages", "text", "result", "body"]) {
      expect(() => parseMainWaitRead({ ...closed, [field]: "private payload" })).toThrow();
    }
    expect(() => parseMainWaitRead({ mode: "retirement_only", retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN })).toThrow();
    expect(() => parseMainWaitRead({ ...open, mode: "unknown" })).toThrow();
    expect(() => parseMainWaitRead({ kind: "open", inbox: open.inbox, retirement: null, retirement_token: EMPTY_RETIREMENT_TOKEN })).toThrow();
  });

  it("validates the whole retirement shape, native identity, timestamp and state before effects", () => {
    for (const retirement of [
      { ...retirementOffer, identity: null },
      { ...retirementOffer, created_at: "2026-02-30T00:00:00Z" },
      { ...retirementOffer, state: { state: "not-a-state" } },
      { ...retirementOffer, state: { state: "native_stop_offered" } },
      { ...retirementOffer, state: { ...retirementOffer.state, messages: ["body"] } },
      { ...retirementOffer, identity: { ...retirementOffer.identity, process: { pid: -1, start_ticks: 123, kernel_boot_id: null } } },
      { ...retirementOffer, identity: { ...retirementOffer.identity, process: { pid: process.pid, start_ticks: 1.5, kernel_boot_id: null } } },
      { ...retirementOffer, identity: { ...retirementOffer.identity, shell: null } },
      { ...retirementOffer, result: "not retirement metadata" },
    ]) expect(() => parseMainWaitRead({ ...closed, retirement })).toThrow();
    expect(parseMainWaitRead({ ...closed, retirement: { ...retirementOffer, identity: null, state: {
      state: "retained", at: retirementOffer.created_at, reason: "identity_incomplete", native_stopped: false,
    } } })).toMatchObject({ mode: "retirement_only" });
  });

  it("rejects malformed or body-bearing open counts rather than generating a wake", () => {
    for (const inbox of [
      { run_id: "run", pending: true, through_seq: 7, counts: [] },
      { run_id: "run", pending: false, through_seq: 0, counts: [], messages: ["secret"] },
      { run_id: "run", pending: true, through_seq: 7, counts: [{ kind: "report", count: 1, text: "body" }] },
      { pending: false, through_seq: 0, counts: [] },
    ]) expect(() => parseMainWaitRead({ ...open, inbox })).toThrow();
  });
});
