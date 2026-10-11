// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Task, TaskView } from "../../protocol/generated/v1";
import type { SourceSubmission, MutationRequestScope } from "./useSupervisor";
import type { StepSubmission } from "./stepInteractions";
import type { SupervisorDraftStorage } from "./useSupervisorDrafts";
import {
  deserializeSupervisorDrafts, messageDraft, newScopeDrafts, newSupervisorDraftStore,
  persistSupervisorDraftStore, readSupervisorDraftStore, retainedSupervisorSubmissions,
  serializeSupervisorDrafts, stepDraft, supervisorDraftStorageKey, useSupervisorDrafts,
} from "./useSupervisorDrafts";

const requestScope: MutationRequestScope = { stateStoreId: "store-a", sessionId: "original-session", rootId: "root-a", expectedRevision: 17 };
const source: SourceSubmission = {
  submissionId: "original-source-uuid", scope: { sessionId: "original-session", rootId: "root-a", taskId: "task-a" },
  action: { action: "task_update", root_id: "root-a", task_id: "task-a", expected_task_revision: "original-task-cas", title: "Original submitted title", description: "Original submitted text" },
};
const step: StepSubmission = {
  submissionId: "original-step-uuid", scope: source.scope, expectedTaskRevision: "original-step-cas",
  intent: { kind: "add", stepId: "original-added-step-uuid", parentStepId: null, beforeStepId: "step-before", title: "Original submitted step" },
};
function task(): Task {
  return { task_id: "task-a", title: "Saved title", body: "Saved source", description: "Saved description", description_editable: true,
    description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null,
    depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1, task_revision: "original-task-cas", diagnostic: null };
}
function baseView(): TaskView { return { task: task(), lane: "queued", current_run_id: null, dependencies: { state: "none", unmet: [], problems: [] } }; }
let root: Root | null = null;
let host: HTMLDivElement | null = null;
beforeEach(() => {
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
    removeItem: (key: string) => { values.delete(key); },
    clear: () => values.clear(),
    key: (index: number) => [...values.keys()][index] ?? null,
    get length() { return values.size; },
  } satisfies Storage);
});
afterEach(async () => {
  try {
    if (root) await act(async () => root!.unmount());
  } finally {
    root = null;
    try {
      host?.remove(); host = null;
      localStorage.clear();
    } finally {
      vi.restoreAllMocks();
      vi.unstubAllGlobals();
    }
  }
});

describe("supervisor owned durable draft storage", () => {
  it("reloads all source forms with text and exact original submission identity together", () => {
    const store = newSupervisorDraftStore(), scope = newScopeDrafts(); store.scopes.set("root-a", scope);
    scope.edits.set("task-a", { title: "Later unsent title", description: "Later unsent text", revision: "original-task-cas", baseTask: task(), baseView: baseView(), submitted: source, reviewed: null });
    const relation: SourceSubmission = { ...source, submissionId: "relation-uuid", action: { action: "task_dependencies_set", root_id: "root-a", task_id: "task-a", expected_task_revision: "original-task-cas", expected_doc_revision: "document-cas", depends_on: ["dependency"] } };
    scope.relations.set("task-a", { dependsOn: ["later-dependency"], baseSet: [], baseTask: task(), baseView: baseView(), revision: "original-task-cas", docRevision: "document-cas", query: "dep", submitted: relation, reviewed: null });
    const followUp: SourceSubmission = { ...source, submissionId: "create-uuid", scope: { ...source.scope, taskId: "original-new-task-uuid" }, action: { action: "task_create", root_id: "root-a", task_id: "original-new-task-uuid", title: "Submitted follow-up", description: "Submitted description", depends_on: ["task-a"], follow_up_of: "task-a", expected_doc_revision: "document-cas", source_revision: "original-task-cas" } };
    scope.followUps.set("task-a", { taskId: "original-new-task-uuid", title: "Later follow-up title", description: "Later description", waitForSource: true, baseSource: task(), baseView: baseView(), docRevision: "document-cas", submitted: followUp, reviewed: null });
    const loaded = deserializeSupervisorDrafts(serializeSupervisorDrafts(store));
    expect(loaded.scopes.get("root-a")!.edits.get("task-a")).toEqual(scope.edits.get("task-a"));
    expect(loaded.scopes.get("root-a")!.relations.get("task-a")).toEqual(scope.relations.get("task-a"));
    expect(loaded.scopes.get("root-a")!.followUps.get("task-a")).toEqual(scope.followUps.get("task-a"));
    expect(retainedSupervisorSubmissions(loaded)).toEqual([{ kind: "source", submitted: source }, { kind: "source", submitted: relation }, { kind: "source", submitted: followUp }]);
  });

  it("treats an interrupted step as unknown without rebasing its text, UUID or task CAS", () => {
    const store = newSupervisorDraftStore(), scope = newScopeDrafts(); store.scopes.set("root-a", scope);
    const draft = stepDraft(scope, "task-a");
    draft.add.set(null, { text: "Later unsent step", baseTaskRevision: "original-step-cas", baseTitle: null, baseParentStepId: null, baseBeforeStepId: "step-before", retainedSubmission: step, open: true });
    draft.operation = { kind: "sending", submitted: step };
    draft.focusRequest = { serial: 9, target: { kind: "toolbar_add" } };
    scope.view.offsets.tasks = { left: 12, top: 25 };
    scope.disclosures.completed = true;
    const loaded = deserializeSupervisorDrafts(serializeSupervisorDrafts(store)), saved = loaded.scopes.get("root-a")!;
    expect(saved.steps.get("task-a")!.add.get(null)).toEqual(draft.add.get(null));
    expect(saved.steps.get("task-a")!.operation).toMatchObject({ kind: "unknown", submitted: step });
    expect(saved.steps.get("task-a")!.focusRequest).toBeNull();
    expect(saved.view.offsets.tasks).toBeNull();
    expect(saved.disclosures.completed).toBe(false);
    expect(retainedSupervisorSubmissions(loaded)).toEqual([{ kind: "steps", submitted: step }]);
  });

  it("retains conflict CAS without pretending it was an unknown write", () => {
    const store = newSupervisorDraftStore(), scope = newScopeDrafts(); store.scopes.set("root-a", scope);
    stepDraft(scope, "task-a").operation = { kind: "review", submitted: step, currentTask: { ...task(), task_revision: "newer-task-cas" }, reason: "conflict" };
    const loaded = deserializeSupervisorDrafts(serializeSupervisorDrafts(store));
    expect(loaded.scopes.get("root-a")!.steps.get("task-a")!.operation).toEqual(scope.steps.get("task-a")!.operation);
    expect(retainedSupervisorSubmissions(loaded)).toEqual([]);
  });

  it("reloads answer, follow-up, note and send-back forms independently with request scope", () => {
    const store = newSupervisorDraftStore(), scope = newScopeDrafts(); store.scopes.set("root-a", scope);
    for (const form of ["answer", "follow-up", "note", "send-back"]) {
      const draft = messageDraft(scope, `${form}:task-a`);
      draft.text = `${form} later unsent text`;
      draft.operation = { id: `${form}-uuid`, text: `${form} original submitted text`, requestScope,
        action: form === "note" ? { action: "annotate", run_id: "worker-a", text: "Original note" } : form === "send-back" ? { action: "send_back", run_id: "worker-a", text: "Original feedback" } : { action: "message_send", message_id: `${form}-uuid`, to_run_id: "worker-a", kind: "instruction", text: "Original message", in_reply_to: null } };
    }
    const loaded = deserializeSupervisorDrafts(serializeSupervisorDrafts(store));
    expect(loaded.scopes.get("root-a")!.messages).toEqual(scope.messages);
  });

  it("keeps empty cleanup intent and working scope-update payloads with their original CAS", () => {
    const store = newSupervisorDraftStore(), scope = newScopeDrafts(); store.scopes.set("root-a", scope);
    messageDraft(scope, "cleanup:root-a:worker-a").operation = { id: "cleanup-uuid", text: "", requestScope,
      action: { action: "cleanup_finish", request: { operation_id: "cleanup-uuid", root_id: "root-a", run_id: "worker-a" } } };
    const update = messageDraft(scope, "followup:worker-a");
    update.text = "Later unsent update";
    update.operation = { id: "scope-update-uuid", text: "Original update", requestScope,
      action: { action: "task_scope_update", operation_id: "scope-update-uuid", root_id: "root-a", task_id: "task-a", run_id: "worker-a",
        expected_task_revision: "original-task-cas", expected_doc_revision: "original-document-cas", title: null, description: "Saved description\n\nOriginal update" } };
    const loaded = deserializeSupervisorDrafts(serializeSupervisorDrafts(store));
    expect(loaded.scopes.get("root-a")!.messages).toEqual(scope.messages);
  });

  it("retains Start and Resume at store level when no root is selected", () => {
    const store = newSupervisorDraftStore();
    store.lifecycle.start = { label: "Later name", location: "directory", spaceId: "", directory: "/later/path", intent: "another", requestScope,
      operation: { action: "supervisor_start", operation_id: "start-uuid", mode: "another", target: { target: "setup", request: { operation: "open", path: "/original/path", label: "Original", task_name: null, focus: false } }, label: "Original", selected_root_id: null, expected_project_key: null } };
    store.lifecycle.resumes.set("worker-a", { action: { action: "run_resume", run_id: "worker-a", operation_id: "resume-uuid", expected_attachment_epoch: 7, mode: "reattach_or_restart", decision_id: null }, requestScope });
    const loaded = deserializeSupervisorDrafts(serializeSupervisorDrafts(store));
    expect(loaded.lifecycle.start).toEqual(store.lifecycle.start);
    expect(loaded.lifecycle.startUnknown).toBe(true);
    expect(loaded.lifecycle.resumes).toEqual(store.lifecycle.resumes);
  });

  it("writes only owned fields, never credentials, clients, run records or whole snapshots", () => {
    const store = newSupervisorDraftStore(), scope = newScopeDrafts(); store.scopes.set("root-a", scope);
    scope.edits.set("task-a", { title: "Draft", description: "Body", revision: "original-task-cas", baseTask: task(), baseView: baseView(), submitted: source, reviewed: null });
    const draft = messageDraft(scope, "answer:task-a");
    draft.operation = { id: "answer-uuid", text: "Submitted answer", requestScope: { ...requestScope }, action: { action: "message_send", message_id: "answer-uuid", to_run_id: "worker-a", kind: "answer", text: "Answer", in_reply_to: "question-a" } };
    Object.assign(store, { snapshot: { credentials: "never-store-snapshot" }, client: { token: "never-store-client" } });
    Object.assign(scope.edits.get("task-a")!.baseView, { run: { token: "never-store-run" } });
    Object.assign(draft.operation, { credentials: "never-store-operation" });
    Object.assign(draft.operation.requestScope!, { authorization: "never-store-auth" });
    const raw = serializeSupervisorDrafts(store);
    expect(raw).not.toContain("never-store-");
    expect(raw).not.toContain("credentials");
    const contaminated = JSON.parse(raw);
    contaminated.snapshot = { token: "untrusted-restored-token" };
    contaminated.scopes[0][1].messages[0][1].operation.token = "untrusted-restored-token";
    expect(serializeSupervisorDrafts(deserializeSupervisorDrafts(JSON.stringify(contaminated)))).not.toContain("untrusted-restored-token");
  });

  it("refuses malformed persisted original payloads instead of inventing replacement identities", () => {
    const store = newSupervisorDraftStore(), scope = newScopeDrafts(); store.scopes.set("root-a", scope);
    stepDraft(scope, "task-a").operation = { kind: "unknown", submitted: step, message: "Lost connection" };
    const value = JSON.parse(serializeSupervisorDrafts(store));
    value.scopes[0][1].steps[0][1].operation.submitted.expectedTaskRevision = null;
    expect(() => deserializeSupervisorDrafts(JSON.stringify(value))).toThrow();
    expect(() => deserializeSupervisorDrafts('{"version":999}')).toThrow();
  });

  it("uses only the stable state-store namespace and keeps memory when persistence fails", () => {
    const id = "failure-test-store", store = newSupervisorDraftStore(), scope = newScopeDrafts();
    store.scopes.set("root-a", scope); messageDraft(scope, "answer:task-a").text = "Retained offline";
    const storage = { getItem: vi.fn(() => null), setItem: vi.fn((_key: string, _value: string) => { throw new Error("Quota exceeded"); }) };
    persistSupervisorDraftStore(id, store, storage);
    expect(storage.setItem.mock.calls[0][0]).toBe(supervisorDraftStorageKey(id));
    expect(store.storageWarning).toContain("app session");
    expect(readSupervisorDraftStore(id, storage)).toBe(store);
    expect(readSupervisorDraftStore("other-failure-test-store", storage).scopes.size).toBe(0);
  });

  it("keeps separate roots across hide/remount and separate stores across identity switches", async () => {
    let storeId: string | null = "hook-test-store-a";
    let api!: SupervisorDraftStorage;
    function Harness() { api = useSupervisorDrafts(storeId); return null; }
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
    await act(async () => { root!.render(createElement(Harness)); });
    await act(async () => {
      messageDraft(api.scope("root-a"), "answer:task").text = "Store A root A";
      messageDraft(api.scope("root-b"), "answer:task").text = "Store A root B";
      api.changed();
    });
    await act(async () => { storeId = "hook-test-store-b"; root!.render(createElement(Harness)); });
    expect(api.scope("root-a").messages.size).toBe(0);
    await act(async () => { storeId = "hook-test-store-a"; root!.render(createElement(Harness)); });
    expect(api.scope("root-a").messages.get("answer:task")!.text).toBe("Store A root A");
    expect(api.scope("root-b").messages.get("answer:task")!.text).toBe("Store A root B");
    await act(async () => { root!.unmount(); }); root = createRoot(host);
    await act(async () => { root!.render(createElement(Harness)); });
    expect(api.scope("root-a").messages.get("answer:task")!.text).toBe("Store A root A");
    expect(localStorage.getItem(supervisorDraftStorageKey("hook-test-store-a"))).toContain("Store A root B");
  });
});
