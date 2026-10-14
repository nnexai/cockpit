// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it } from "vitest";
import { SupervisorGraph } from "./SupervisorGraph";
import { taskFixture } from "./testFixtures";
let root: Root | null = null, host: HTMLDivElement;
afterEach(() => { if (root) act(() => root!.unmount()); root = null; host?.remove(); });
it("renders task prerequisites only, with no completed/provenance nodes or operational controls", async () => {
  const first = taskFixture("first"), dependent = taskFixture("dependent");
  dependent.task.depends_on = ["first"];
  dependent.dependencies = { state: "blocked", unmet: [{ task_id: "first", reason: "unchecked" }], problems: [] };
  dependent.lane = "blocked";
  const done = taskFixture("done", { lane: "accepted" }); done.task.checked = true;
  const followUp = taskFixture("follow-up"); followUp.task.follow_up_of = "first";
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root!.render(<SupervisorGraph tasks={[first, dependent, done, followUp]} />));
  expect(host.querySelectorAll(".liaison-graph-task")).toHaveLength(3);
  expect(host.querySelectorAll(".liaison-graph-edge")).toHaveLength(1);
  expect(host.querySelectorAll("button, input, textarea")).toHaveLength(0);
  expect(host.querySelector('[aria-label="Task dependency outline"]')).not.toBeNull();
  expect(host.querySelector(".liaison-graph-footer")?.textContent).toContain("1 done");
  const node = host.querySelector<HTMLElement>('.liaison-graph-task[aria-label^="dependent"]')!;
  await act(async () => node.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
  expect(host.querySelectorAll(".liaison-graph-edge.is-highlighted")).toHaveLength(1);
});
