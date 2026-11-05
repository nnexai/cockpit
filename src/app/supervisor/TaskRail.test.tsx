// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { TaskView } from "../../protocol/generated/v1";
import { TaskRail } from "./SupervisorTasks";
import { TaskHistoryDetail } from "./TaskHistoryDetail";
import type { TaskHistory } from "./taskHistory";
import { orchestrationFixture, taskFixture } from "./testFixtures";

let root: Root | null = null, host: HTMLDivElement;
afterEach(() => { if (root) act(() => root!.unmount()); root = null; host?.remove(); });
async function render(tasks: TaskView[]) {
  if (!root) { host = document.createElement("div"); document.body.append(host); root = createRoot(host); }
  const snapshot = orchestrationFixture(tasks);
  await act(async () => root!.render(<TaskRail tasks={tasks} snapshot={snapshot} rootId="root" fresh waiting={false} status={null} onOverview={vi.fn()} />));
}
it("retains full task titles and stage words in rail rows", async () => {
  const title = "Experiment with agent steering and busy delivery across a long-running task";
  const task = taskFixture("task", { lane: "working" });
  await render([{ ...task, task: { ...task.task, title } }]);
  const row = host.querySelector<HTMLButtonElement>("[data-row-key]")!;
  expect(row.querySelector(".liaison-task-title")?.textContent).toBe(title);
  expect(row.querySelector(".liaison-task-title")?.getAttribute("title")).toBe(title);
  expect(row.querySelector(".liaison-task-stage")?.textContent).toBe("working");
  expect(row.getAttribute("aria-label")).toBe(`${title}, working. Open history`);
});
it("restores the originating row after Escape and does not steal focus on snapshot updates", async () => {
  const task = taskFixture("task");
  await render([task]);
  act(() => host.querySelector<HTMLButtonElement>("[data-row-key]")!.click());
  expect(document.activeElement?.getAttribute("aria-label")).toBe("Back to tasks");
  const aside = host.querySelector("aside")!;
  await render([{ ...task, lane: "working" }]);
  expect(document.activeElement?.getAttribute("aria-label")).toBe("Back to tasks");
  act(() => aside.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(document.activeElement).toBe(host.querySelector("[data-row-key]"));
  const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
  aside.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(false);
});
it("keeps completed history reachable after the last task finishes, including earlier completed tasks", async () => {
  const tasks = Array.from({ length: 7 }, (_, i) => { const view = taskFixture(`task-${i}`); return { ...view, lane: "accepted" as const, task: { ...view.task, checked: true, line: i } }; });
  await render([{ ...tasks[0], lane: "working", task: { ...tasks[0].task, checked: false } }, ...tasks.slice(1)]);
  const rail = host.querySelector("aside")!, header = rail.querySelector("header")!, list = rail.querySelector("ul")!;
  await render(tasks);
  expect(host.querySelector("aside")).toBe(rail);
  expect(rail.querySelector("header")).toBe(header);
  expect(rail.querySelector("ul")).toBe(list);
  expect(rail.className).toBe("liaison-task-rail");
  expect(host.querySelector(".liaison-bar-toggle")).toBeNull();
  expect(header.textContent).toContain("all done · 7 completed");
  expect(list.hidden).toBe(false);
  expect(host.querySelectorAll("[data-row-key]")).toHaveLength(5);
  act(() => host.querySelector<HTMLButtonElement>(".liaison-earlier-toggle")!.click());
  const row = [...host.querySelectorAll<HTMLButtonElement>("[data-row-key]")].find(element => element.textContent?.includes("task-0"))!;
  act(() => row.click());
  expect(host.querySelector('[aria-label="History of task-0"]')).not.toBeNull();
  act(() => host.querySelector<HTMLButtonElement>('[aria-label="Back to tasks"]')!.click());
  expect(document.activeElement).toBe(host.querySelector('[data-row-key="task-0:0"]'));
});
it("keeps the normal rail when new work arrives and folds Done rows into their disclosure", async () => {
  const done = taskFixture("done", { lane: "accepted" }); done.task.checked = true;
  const work = taskFixture("work");
  await render([done, work]);
  act(() => host.querySelector<HTMLButtonElement>(".liaison-done-toggle")!.click());
  expect(host.querySelector('[data-row-key="done:0"]')).not.toBeNull();
  await render([done]);
  const rail = host.querySelector("aside")!;
  act(() => host.querySelector<HTMLButtonElement>('[data-row-key="done:0"]')!.click());
  act(() => host.querySelector<HTMLButtonElement>('[aria-label="Back to tasks"]')!.click());
  await render([done, work]);
  expect(host.querySelector("aside")).toBe(rail);
  expect(rail.className).toBe("liaison-task-rail");
  expect(host.querySelectorAll("[data-row-key]")).toHaveLength(1);
  expect(host.querySelector('[data-row-key="work:1"]')).not.toBeNull();
  expect(host.querySelector(".liaison-done-toggle")?.getAttribute("aria-expanded")).toBe("false");
  act(() => host.querySelector<HTMLButtonElement>(".liaison-done-toggle")!.click());
  expect(host.querySelector('[data-row-key="done:0"]')).not.toBeNull();
});
it("returns from vanished task history without taking focus from an external input", async () => {
  const task = taskFixture("task"), other = taskFixture("other");
  await render([task, other]);
  act(() => host.querySelector<HTMLButtonElement>("[data-row-key]")!.click());
  const input = document.createElement("input"); document.body.append(input); input.focus();
  await render([other]);
  expect(host.querySelector('[aria-label="History of task"]')).toBeNull();
  expect(document.activeElement).toBe(input);
  input.remove();
});

it("shows recorded waiting facts and accessible task progress in the shared detail", async () => {
  const task = taskFixture("dependent", {
    lane: "blocked",
    waiting: { cause: "dependency_input", since: "2026-10-01T12:00:00Z", held_by: [{ id: "predecessor", title: "Predecessor" }], code: "dependency_input_not_materialized", detail: "The frozen base lacks the output.", next_automatic: true },
  });
  if (!root) { host = document.createElement("div"); document.body.append(host); root = createRoot(host); }
  await act(async () => root!.render(<TaskHistoryDetail view={task} history={null} />));
  const rail = host.querySelector('[aria-label="Task progress"]')!;
  expect([...rail.querySelectorAll("li")].map(item => item.textContent)).toEqual(["Ready", "Working", "Review", "Accepted", "Merged"]);
  expect(rail.querySelector('[aria-current="step"]')?.textContent).toBe("Ready");
  const wait = host.querySelector('[aria-label="Why is this waiting"]')!;
  expect(wait.textContent).toContain("Predecessor");
  expect(wait.textContent).toContain("The frozen base lacks the output.");
  expect(wait.querySelector("time")?.getAttribute("datetime")).toBe("2026-10-01T12:00:00Z");
});

it("shows liaison-owned waits without invented times and omits merge for non-code work", async () => {
  const task = taskFixture("report", {
    lane: "review",
    waiting: { cause: "review", since: null, held_by: [], code: null, detail: null, next_automatic: false },
  });
  const history: TaskHistory = { taskId: "report", identity: "unique", outcome: { stage: "review", scheduled: false, accepted: false, attempt: 1, kept: 0, stalled: false }, sub: null, scheduledAt: null, attempts: [], leftBehind: [], liveUnavailable: false };
  if (!root) { host = document.createElement("div"); document.body.append(host); root = createRoot(host); }
  await act(async () => root!.render(<TaskHistoryDetail view={task} history={history} />));
  const rail = host.querySelector('[aria-label="Task progress"]')!;
  expect(rail.querySelector('[aria-current="step"]')?.textContent).toBe("Review");
  const wait = host.querySelector('[aria-label="Why is this waiting"]')!;
  expect(wait.textContent).not.toContain("Held by");
  expect(wait.textContent).not.toContain("Since");
  expect(wait.querySelector("time")).toBeNull();
  await act(async () => root!.render(<TaskHistoryDetail view={{ ...task, lane: "accepted", waiting: null }} history={{ ...history, sub: { kind: "no_merge" } }} />));
  expect([...host.querySelectorAll('[aria-label="Task progress"] li')].map(item => item.textContent)).toEqual(["Ready", "Working", "Review", "Accepted"]);
  expect(host.querySelector('[aria-current="step"]')?.textContent).toBe("Accepted");
});

it("marks recorded integration as merged and hides the waiting block when clear", async () => {
  const task = taskFixture("code", { lane: "accepted", waiting: null });
  const history: TaskHistory = { taskId: "code", identity: "unique", outcome: { stage: "done", scheduled: false, accepted: true, attempt: 1, kept: 0, stalled: false }, sub: { kind: "merged", target: "main" }, scheduledAt: null, attempts: [], leftBehind: [], liveUnavailable: false };
  if (!root) { host = document.createElement("div"); document.body.append(host); root = createRoot(host); }
  await act(async () => root!.render(<TaskHistoryDetail view={task} history={history} />));
  expect(host.querySelector('[aria-current="step"]')?.textContent).toBe("Merged");
  expect(host.querySelector('[aria-label="Why is this waiting"]')).toBeNull();
});
