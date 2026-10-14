// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { SupervisorView } from "./SupervisorView";
import { taskFixture } from "./testFixtures";
let root: Root | null = null, host: HTMLDivElement;
afterEach(() => { if (root) act(() => root!.unmount()); root = null; host?.remove(); });
it("opens the board with Back focused, changes to a task-only graph and closes on Escape", async () => {
  const task = taskFixture("Work"), close = vi.fn();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root!.render(<SupervisorView tasks={[task]} runs={[]} waiting={false} status={null} onClose={close} />));
  expect(host.querySelectorAll(".liaison-task-lane")).toHaveLength(5);
  expect(document.activeElement).toBe(host.querySelector(".liaison-conversation"));
  expect(host.querySelector(".liaison-task-card button")).toBeNull();
  const graph = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Graph")!;
  await act(async () => graph.click());
  expect(host.querySelector('[aria-label="Task dependency graph"]')).not.toBeNull();
  await act(async () => graph.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(close).toHaveBeenCalledOnce();
});
