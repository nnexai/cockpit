// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { TaskView } from "../../protocol/generated/v1";
import { TaskRail } from "./SupervisorTasks";
import { orchestrationFixture, taskFixture } from "./testFixtures";

let root: Root | null = null, host: HTMLDivElement;
afterEach(() => { if (root) act(() => root!.unmount()); root = null; host?.remove(); });
async function render(tasks: TaskView[]) {
  if (!root) { host = document.createElement("div"); document.body.append(host); root = createRoot(host); }
  const snapshot = orchestrationFixture(tasks);
  await act(async () => root!.render(<TaskRail tasks={tasks} snapshot={snapshot} rootId="root" fresh waiting={false} status={null} onOverview={vi.fn()} />));
}
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
