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
