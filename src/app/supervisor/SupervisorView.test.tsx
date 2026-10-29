// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { SupervisorView } from "./SupervisorView";
import type { TaskView } from "../../protocol/generated/v1";
import { orchestrationFixture, runFixture, taskFixture } from "./testFixtures";
let root: Root | null = null, host: HTMLDivElement;
afterEach(() => { if (root) act(() => root!.unmount()); root = null; host?.remove(); });
async function render(tasks: TaskView[], overrides: Partial<ComponentProps<typeof SupervisorView>> = {}) {
  if (!root) { host = document.createElement("div"); document.body.append(host); root = createRoot(host); }
  const snapshot = orchestrationFixture(tasks);
  await act(async () => root!.render(<SupervisorView tasks={tasks} runs={snapshot.runs} snapshot={snapshot} rootId="root" fresh waiting={false} status={null} onClose={vi.fn()} {...overrides} />));
}
it("opens the board with Conversation focused, changes to a task-only graph and closes on Escape", async () => {
  const task = taskFixture("Work"), close = vi.fn();
  await render([task], { onClose: close });
  expect(host.querySelectorAll(".liaison-task-lane")).toHaveLength(5);
  expect(document.activeElement).toBe(host.querySelector(".liaison-conversation"));
  const graph = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Graph")!;
  await act(async () => graph.click());
  expect(host.querySelector('[aria-label="Task dependency graph"]')).not.toBeNull();
  await act(async () => graph.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(close).toHaveBeenCalledOnce();
});

it("opens full snapshot history from a board card and restores origin focus and scroll on nested Escape", async () => {
  const task = taskFixture("Work"), close = vi.fn();
  const worker = runFixture({ kind: "worker", run_id: "worker", task_id: "Work", stage: "working" });
  const snapshot = orchestrationFixture([task], [worker]);
  snapshot.messages = [
    { message_id: "question", from: { type: "run", run_id: "worker" }, to_run_id: "root", kind: "report", text: "Which target?", report: { message_id: "question", kind: "needs_input", outcome: null, summary: "Which target?", plan: null, source_output: null, at: worker.created_at }, in_reply_to: null, stale: false, escalated_from: null, stage: "acked", seq: 1, created_at: worker.created_at, acked_at: worker.created_at },
    { message_id: "answer", from: { type: "operator" }, to_run_id: "worker", kind: "answer", text: "Use main", report: null, in_reply_to: "question", stale: false, escalated_from: null, stage: "acked", seq: 2, created_at: "2026-10-07T12:02:00Z", acked_at: "2026-10-07T12:02:00Z" },
  ];
  snapshot.cleanup = [{ run_id: "worker", removed_count: 0, kept_count: 1, waiting_count: 0, unknown_count: 0, items: [
    { resource_id: "output", kind: "attempt_directory", ownership: "owned", cleanup: { operation_id: null, state: "kept", reason: { code: "outputs_kept", class: "user_work", message: "redacted" }, effects: [], updated_at: worker.created_at } },
  ] }];
  await render([task], { snapshot, onClose: close });
  const card = host.querySelector<HTMLButtonElement>(".liaison-card-open")!;
  const board = host.querySelector<HTMLElement>(".liaison-task-board")!, lane = card.closest("ul")!;
  board.scrollLeft = 120; lane.scrollTop = 40;
  act(() => { card.focus(); card.click(); });
  const detail = host.querySelector(".liaison-overview-detail")!;
  expect(detail.textContent).toContain("answered by you 2 min later");
  expect(detail.textContent).toContain("Outputs kept");
  expect(document.activeElement).toBe(host.querySelector(".liaison-detail-back"));
  expect(host.querySelector('[aria-label="Overview view"]')).not.toBeNull();
  expect(host.querySelector(".liaison-task-overview > header h1")?.textContent).toBe("Overview");
  expect(board.closest("[hidden]")).toBeNull();
  expect(board.parentElement?.hasAttribute("inert")).toBe(false);
  expect(card.getAttribute("aria-current")).toBe("true");
  const outerEscape = vi.fn(); document.addEventListener("keydown", outerEscape);
  const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
  act(() => document.activeElement!.dispatchEvent(escape));
  document.removeEventListener("keydown", outerEscape);
  expect(escape.defaultPrevented).toBe(true);
  expect(outerEscape).not.toHaveBeenCalled();
  expect(close).not.toHaveBeenCalled();
  expect(host.querySelector(".liaison-task-board")).toBe(board);
  expect(board.scrollLeft).toBe(120); expect(lane.scrollTop).toBe(40);
  expect(document.activeElement).toBe(card);
  expect(card.hasAttribute("aria-current")).toBe(false);
  expect(board.closest("[hidden]")).toBeNull();
  act(() => card.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(close).toHaveBeenCalledOnce();
});

it("returns to the graph node with its pan position and mode preserved", async () => {
  const task = taskFixture("Work"), close = vi.fn();
  await render([task], { onClose: close });
  act(() => [...host.querySelectorAll<HTMLButtonElement>("header button")].find(button => button.textContent === "Graph")!.click());
  const graph = host.querySelector<HTMLElement>(".liaison-task-graph")!;
  const node = host.querySelector<HTMLButtonElement>(".liaison-graph-task")!;
  graph.scrollLeft = 180; graph.scrollTop = 60;
  act(() => { node.focus(); node.click(); });
  expect(host.querySelector('[aria-label="History of Work"]')).not.toBeNull();
  act(() => host.querySelector<HTMLButtonElement>(".liaison-detail-back")!.click());
  expect(host.querySelector(".liaison-task-graph")).toBe(graph);
  expect(graph.scrollLeft).toBe(180); expect(graph.scrollTop).toBe(60);
  expect(document.activeElement).toBe(node);
  expect(host.querySelector('header button[aria-pressed="true"]')?.textContent).toBe("Graph");
  expect(close).not.toHaveBeenCalled();
});

it("restores the precise narrow graph outline row instead of its hidden canvas node", async () => {
  const task = taskFixture("Work");
  await render([task]);
  act(() => [...host.querySelectorAll<HTMLButtonElement>("header button")].find(button => button.textContent === "Graph")!.click());
  host.querySelector<HTMLElement>(".liaison-graph-canvas")!.style.display = "none";
  host.querySelector<HTMLElement>(".liaison-dependency-outline")!.style.display = "block";
  const row = host.querySelector<HTMLButtonElement>(".liaison-dependency-outline button")!;
  act(() => { row.focus(); row.click(); });
  expect(host.querySelector('[aria-label="History of Work"]')).not.toBeNull();
  act(() => host.querySelector<HTMLButtonElement>(".liaison-detail-back")!.click());
  expect(document.activeElement).toBe(row);
});

it("refreshes the selected projection without stealing focus, including external focus", async () => {
  const task = taskFixture("Work");
  const worker = runFixture({ kind: "worker", run_id: "worker", task_id: "Work", stage: "working" });
  worker.result = { message_id: "result", kind: "result", outcome: "succeeded", summary: "Work reported", plan: null, source_output: null, at: worker.created_at };
  await render([task], { snapshot: orchestrationFixture([task], [worker]) });
  act(() => host.querySelector<HTMLButtonElement>(".liaison-card-open")!.click());
  const summary = host.querySelector<HTMLElement>(".liaison-detail summary")!;
  act(() => summary.focus());
  const changed = { ...task, lane: "working" as const };
  await render([changed], { snapshot: orchestrationFixture([changed], [worker]), fresh: false });
  expect(document.activeElement).toBe(summary);
  expect(host.querySelector(".liaison-detail")?.textContent).toContain("Live worker state unavailable");
  const input = document.createElement("input"); document.body.append(input);
  try {
    act(() => input.focus());
    await render([changed], { snapshot: orchestrationFixture([changed], [worker]) });
    expect(document.activeElement).toBe(input);
  } finally { input.remove(); }
});

it("returns from a vanished task without closing Overview or stealing external focus", async () => {
  const task = taskFixture("Work"), other = taskFixture("Other"), close = vi.fn();
  await render([task, other], { onClose: close });
  act(() => host.querySelector<HTMLButtonElement>(".liaison-card-open")!.click());
  await render([other], { onClose: close });
  expect(host.querySelector(".liaison-detail")).toBeNull();
  expect(document.activeElement).toBe(host.querySelector(".liaison-conversation"));
  act(() => host.querySelector<HTMLButtonElement>(".liaison-card-open")!.click());
  const input = document.createElement("input"); document.body.append(input);
  try {
    act(() => input.focus());
    await render([], { onClose: close });
    expect(host.querySelector(".liaison-detail")).toBeNull();
    expect(document.activeElement).toBe(input);
    expect(close).not.toHaveBeenCalled();
  } finally { input.remove(); }
});

it("restores overview focus when a focused history event vanishes with its task", async () => {
  const task = taskFixture("Work"), other = taskFixture("Other");
  const worker = runFixture({ kind: "worker", run_id: "worker", task_id: "Work", stage: "working" });
  worker.result = { message_id: "result", kind: "result", outcome: "succeeded", summary: "Work reported", plan: null, source_output: null, at: worker.created_at };
  await render([task, other], { snapshot: orchestrationFixture([task, other], [worker]) });
  act(() => host.querySelector<HTMLButtonElement>(".liaison-card-open")!.click());
  act(() => host.querySelector<HTMLElement>(".liaison-detail summary")!.focus());
  await render([other]);
  expect(document.activeElement).toBe(host.querySelector(".liaison-conversation"));
});

it("keeps earlier Done cards reachable and preserves expanded history on return", async () => {
  const tasks = Array.from({ length: 14 }, (_, index) => {
    const task = taskFixture(`Done ${index}`, { lane: "accepted" });
    return { ...task, task: { ...task.task, checked: true, line: index } };
  });
  await render(tasks);
  expect(host.querySelectorAll(".liaison-card-open")).toHaveLength(12);
  act(() => host.querySelector<HTMLButtonElement>(".liaison-task-earlier")!.click());
  expect(host.querySelectorAll(".liaison-card-open")).toHaveLength(14);
  const card = [...host.querySelectorAll<HTMLButtonElement>(".liaison-card-open")].find(button => button.dataset.rowKey === "Done 0:0")!;
  act(() => card.click());
  expect(host.querySelector('[aria-label="History of Done 0"]')).not.toBeNull();
  act(() => host.querySelector<HTMLButtonElement>(".liaison-detail-back")!.click());
  expect(host.querySelectorAll(".liaison-card-open")).toHaveLength(14);
  expect(document.activeElement).toBe(card);
});

it("renders unavailable and ambiguous histories through the shared detail component", async () => {
  const task = taskFixture("Work");
  await render([task], { snapshot: null });
  act(() => host.querySelector<HTMLButtonElement>(".liaison-card-open")!.click());
  expect(host.querySelector(".liaison-detail")?.textContent).toContain("History unavailable");
  act(() => host.querySelector<HTMLButtonElement>(".liaison-detail-back")!.click());
  await render([task, { ...task, task: { ...task.task } }]);
  const cards = host.querySelectorAll<HTMLButtonElement>(".liaison-card-open");
  expect(cards[0].dataset.rowKey).not.toBe(cards[1].dataset.rowKey);
  act(() => cards[1].click());
  expect(host.querySelector(".liaison-detail")?.textContent).toContain("ID appears more than once");
  act(() => host.querySelector<HTMLButtonElement>(".liaison-detail-back")!.click());
  expect(document.activeElement).toBe(cards[1]);
});

it("resolves the selected task across reordering and restores its new board location", async () => {
  const task = taskFixture("Work"), other = taskFixture("Other");
  await render([task, other]);
  act(() => host.querySelector<HTMLButtonElement>('[data-row-key="Work:0"]')!.click());
  const changed = { ...task, lane: "working" as const };
  await render([other, changed]);
  expect(host.querySelector('[aria-label="History of Work"]')).not.toBeNull();
  expect(host.querySelector('[data-row-key="Work:1"]')?.getAttribute("aria-current")).toBe("true");
  expect(document.activeElement).toBe(host.querySelector(".liaison-detail-back"));
  act(() => host.querySelector<HTMLButtonElement>(".liaison-detail-back")!.click());
  expect(document.activeElement).toBe(host.querySelector('[data-row-key="Work:1"]'));
});

it("swaps selected history beside the visible board and closes the latest detail before Overview", async () => {
  const first = taskFixture("First"), second = taskFixture("Second"), close = vi.fn();
  await render([first, second], { onClose: close });
  const header = host.querySelector(".liaison-task-overview > header")!;
  const board = host.querySelector(".liaison-task-board")!;
  const firstCard = host.querySelector<HTMLButtonElement>('[data-row-key="First:0"]')!;
  const secondCard = host.querySelector<HTMLButtonElement>('[data-row-key="Second:1"]')!;
  act(() => firstCard.click());
  expect(host.querySelector('[aria-label="History of First"]')).not.toBeNull();
  expect(board.closest("[hidden], [inert]")).toBeNull();
  act(() => secondCard.click());
  expect(host.querySelector('[aria-label="History of First"]')).toBeNull();
  expect(host.querySelector('[aria-label="History of Second"]')).not.toBeNull();
  expect(firstCard.hasAttribute("aria-current")).toBe(false);
  expect(secondCard.getAttribute("aria-current")).toBe("true");
  expect(host.querySelector(".liaison-task-board")).toBe(board);
  expect(host.querySelector(".liaison-task-overview > header")).toBe(header);
  expect(document.activeElement?.getAttribute("aria-label")).toBe("Close task history");
  act(() => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(host.querySelector(".liaison-overview-detail")).toBeNull();
  expect(document.activeElement).toBe(secondCard);
  expect(close).not.toHaveBeenCalled();
  act(() => secondCard.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(close).toHaveBeenCalledOnce();
});

it("keeps history open across Board and Graph switches and restores the visible selected row", async () => {
  const task = taskFixture("Work");
  await render([task]);
  act(() => host.querySelector<HTMLButtonElement>(".liaison-card-open")!.click());
  const detail = host.querySelector(".liaison-overview-detail")!;
  act(() => [...host.querySelectorAll<HTMLButtonElement>("header button")].find(button => button.textContent === "Graph")!.click());
  expect(host.querySelector(".liaison-overview-detail")).toBe(detail);
  const node = host.querySelector<HTMLButtonElement>(".liaison-graph-task")!;
  expect(node.getAttribute("aria-current")).toBe("true");
  expect(host.querySelector(".liaison-task-graph")?.closest("[hidden], [inert]")).toBeNull();
  act(() => host.querySelector<HTMLButtonElement>('[aria-label="Close task history"]')!.click());
  expect(document.activeElement).toBe(node);
  act(() => node.click());
  act(() => [...host.querySelectorAll<HTMLButtonElement>("header button")].find(button => button.textContent === "Board")!.click());
  expect(host.querySelector('[aria-label="History of Work"]')).not.toBeNull();
  const card = host.querySelector<HTMLButtonElement>(".liaison-card-open")!;
  expect(card.getAttribute("aria-current")).toBe("true");
  act(() => host.querySelector<HTMLButtonElement>('[aria-label="Close task history"]')!.click());
  expect(document.activeElement).toBe(card);
});

it("leaves Overview directly from Conversation while task history is open", async () => {
  const task = taskFixture("Work"), close = vi.fn();
  await render([task], { onClose: close });
  act(() => host.querySelector<HTMLButtonElement>(".liaison-card-open")!.click());
  act(() => host.querySelector<HTMLButtonElement>(".liaison-conversation")!.click());
  expect(close).toHaveBeenCalledOnce();
});

it("keeps Done history visible when Graph omits its row and closes back to Conversation", async () => {
  const task = taskFixture("Done", { lane: "accepted" }); task.task.checked = true;
  await render([task]);
  act(() => host.querySelector<HTMLButtonElement>(".liaison-card-open")!.click());
  act(() => [...host.querySelectorAll<HTMLButtonElement>("header button")].find(button => button.textContent === "Graph")!.click());
  expect(host.querySelector('[aria-label="History of Done"]')).not.toBeNull();
  expect(host.querySelector('[aria-label="Task dependency graph"]')).not.toBeNull();
  expect(host.querySelector(".liaison-graph-task")).toBeNull();
  act(() => host.querySelector<HTMLButtonElement>('[aria-label="Close task history"]')!.click());
  expect(document.activeElement).toBe(host.querySelector(".liaison-conversation"));
});

it("shows current root-scoped attention, separates reported results from acceptance, and opens history from either view", async () => {
  const question = taskFixture("Question", { current_run_id: "q" });
  const failed = taskFixture("Failed", { lane: "review", current_run_id: "f" });
  const successful = taskFixture("Success", { lane: "review", current_run_id: "s" });
  const uncertain = taskFixture("Uncertain", { current_run_id: "u" });
  const tasks = [question, failed, successful, uncertain];
  const report = (outcome: "succeeded" | "failed") => ({ message_id: `result-${outcome}`, kind: "result" as const, outcome, summary: "Reported", plan: null, source_output: null, at: "2026-10-07T12:00:00Z" });
  const workers = [
    runFixture({ kind: "worker", run_id: "q", task_id: "Question", stage: "working" }),
    runFixture({ kind: "worker", run_id: "f", task_id: "Failed", stage: "reported", result: report("failed") }),
    runFixture({ kind: "worker", run_id: "s", task_id: "Success", stage: "reported", result: report("succeeded") }),
    runFixture({ kind: "worker", run_id: "u", task_id: "Uncertain", stage: "working" }),
  ];
  const snapshot = orchestrationFixture(tasks, workers);
  const at = workers[0].created_at;
  snapshot.attention = [
    { kind: "needs_input", run_id: "q", task_id: "Question", message_seq: null, since: at },
    { kind: "to_accept", run_id: "f", task_id: "Failed", message_seq: null, since: at },
    { kind: "to_accept", run_id: "s", task_id: "Success", message_seq: null, since: at },
    { kind: "runtime_blocked", run_id: "u", task_id: "Uncertain", message_seq: null, since: at },
    { kind: "brief_unread", run_id: "u", task_id: "Uncertain", message_seq: 1, since: at },
    { kind: "dispatch_unknown", run_id: "old", task_id: "Question", message_seq: null, since: at },
  ];
  await render(tasks, { snapshot, runs: workers });
  const panel = host.querySelector<HTMLElement>('[aria-label="Task attention"]')!;
  expect(panel.textContent).toContain("Needs input");
  expect(panel.textContent).toContain("Failed result reported");
  expect(panel.textContent).toContain("Successful result reported");
  expect(panel.textContent).toContain("not task acceptance");
  expect(panel.textContent).toContain(`Observed working · as of ${at}`);
  expect(panel.textContent).not.toContain("accepted");
  expect(panel.textContent).toContain("Runtime reported blocked");
  expect(panel.textContent).toContain("Work brief stored; delivery unconfirmed");
  expect(panel.textContent).not.toContain("Dispatch outcome uncertain");
  const row = panel.querySelector<HTMLButtonElement>("button")!;
  act(() => { row.focus(); row.click(); });
  expect(document.activeElement).toBe(host.querySelector(".liaison-detail-back"));
  expect(host.querySelector('[aria-label="Task history"]')).not.toBeNull();
  act(() => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(document.activeElement).toBe(row);
  act(() => [...host.querySelectorAll("button")].find(button => button.textContent === "Graph")!.click());
  act(() => { row.focus(); row.click(); });
  act(() => host.querySelector<HTMLButtonElement>(".liaison-detail-back")!.click());
  expect(document.activeElement).toBe(row);
});

it("warns when observations are stale and excludes ambiguous, foreign and superseded identities", async () => {
  const task = taskFixture("Work", { current_run_id: "current" });
  const foreign = taskFixture("Foreign", { current_run_id: "foreign" });
  const duplicate = taskFixture("Duplicate", { current_run_id: "duplicate" });
  const tasks = [task, foreign, duplicate, duplicate];
  const current = runFixture({ kind: "worker", run_id: "current", task_id: "Work", stage: "working" });
  const old = runFixture({ kind: "worker", run_id: "old", task_id: "Work", stage: "closed" });
  const wrongRoot = runFixture({ kind: "worker", root_id: "other", run_id: "foreign", task_id: "Foreign", stage: "working" });
  const dupRun = runFixture({ kind: "worker", run_id: "duplicate", task_id: "Duplicate", stage: "working" });
  const snapshot = orchestrationFixture(tasks, [current, old, wrongRoot, dupRun]);
  snapshot.attention = [current, old, wrongRoot, dupRun].map(run => ({ kind: "idle_without_report", run_id: run.run_id, task_id: run.task_id, message_seq: null, since: run.created_at }));
  await render(tasks, { snapshot, fresh: false });
  const panel = host.querySelector<HTMLElement>('[aria-label="Task attention"]')!;
  expect(panel.textContent).toContain("Runtime observations may be stale or unavailable");
  expect(panel.textContent).toContain("Idle without a result");
  expect(panel.textContent).toContain(`Last observed working · as of ${snapshot.runtime.status === "fresh" ? snapshot.runtime.observed_at : ""}`);
  expect(panel.querySelectorAll("button")).toHaveLength(1);
  expect(panel.textContent).not.toContain("Foreign");
  expect(panel.textContent).not.toContain("Duplicate");
  await render(tasks, { snapshot: { ...snapshot, attention: [] }, fresh: false });
  expect(host.querySelector('[aria-label="Task attention"]')?.textContent).toContain("stale or unavailable");
});

it("keeps scheduled and prerequisite-blocked task facts visible on the board", async () => {
  const prerequisite = taskFixture("Prerequisite");
  const queued = taskFixture("Queued", { scheduled: true });
  const blocked = taskFixture("Blocked", { lane: "blocked", dependencies: { state: "blocked", unmet: [{ task_id: "Prerequisite", reason: "unchecked" }], problems: [] } });
  const unknown = taskFixture("Unknown blocker", { lane: "blocked" });
  await render([prerequisite, queued, blocked, unknown]);
  const cards = [...host.querySelectorAll<HTMLButtonElement>(".liaison-card-open")];
  expect(cards.find(card => card.textContent?.includes("Queued"))?.textContent).toContain("scheduled");
  expect(cards.find(card => card.textContent?.includes("Blocked"))?.textContent).toContain("after “Prerequisite”");
  const panel = host.querySelector<HTMLElement>('[aria-label="Task attention"]')!;
  expect(panel.textContent).toContain("Waiting for dispatch; reason unknown");
  expect(panel.textContent).toContain("Blocked: after “Prerequisite”");
  expect(panel.textContent).toContain("Blocked; reason unknown");
});

it("shows unknown live worker state when no current agent can be observed", async () => {
  const task = taskFixture("Work", { current_run_id: "worker", lane: "working" });
  const worker = runFixture({ kind: "worker", run_id: "worker", task_id: "Work", stage: "working" });
  const snapshot = orchestrationFixture([task], [worker]);
  if (snapshot.runtime.status !== "fresh") throw new Error("fixture runtime must be fresh");
  const runtime = snapshot.runtime;
  const observation = runtime.runs[0];
  for (const runs of [[], [{ ...observation, presence: "unobserved" as const }], [{ ...observation, presence: "missing" as const }], [{ ...observation, actual_agent: false }]]) {
    await render([task], { snapshot: { ...snapshot, runtime: { ...runtime, runs } } });
    const panel = host.querySelector<HTMLElement>('[aria-label="Task attention"]')!;
    expect(panel.textContent).toContain("Live worker state unknown");
    expect(panel.textContent).not.toContain("Observed working");
  }
  await render([task], { snapshot: { ...snapshot, runtime: { status: "unavailable", error: { code: "offline", message: "Unavailable" } } } });
  expect(host.querySelector('[aria-label="Task attention"]')?.textContent).toContain("Live worker state unknown");
  await render([task], { snapshot, fresh: false });
  expect(host.querySelector('[aria-label="Task attention"]')?.textContent).toContain("Live worker state unknown");
});
