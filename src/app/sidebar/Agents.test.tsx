// @vitest-environment jsdom
import { act, createElement, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { Agents } from "./Agents";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

it("keeps urgent words and tab context compact until selection is confirmed", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const current: ComponentProps<typeof Agents> = {
    agents: [
      { pane_id: "blocked", space_id: "space", tab_id: "tab", name: "omp", status: "blocked", title: null, focused: false, state_change_seq: 2 },
      { pane_id: "working", space_id: "space", tab_id: "tab", name: "omp", status: "working", title: null, focused: true, state_change_seq: 1 },
      { pane_id: "done", space_id: "space", tab_id: "tab", name: "helper", status: "done", title: null, focused: false, state_change_seq: 3 },
    ],
    spaces: [{ id: "space", label: "A very long Space name", number: 1, tab_count: 1, pane_count: 3, focused: true, agent_status: "blocked", git: null }],
    tabs: [{ id: "tab", space_id: "space", label: "2", number: 2, pane_count: 3, focused: true, focused_pane_id: "working" }],
    selectedPaneId: "working", pendingPaneId: null, loading: false, hasSession: true, onSelect: vi.fn(), focusRef: { current: null },
  };
  try {
    await act(async () => root.render(createElement(Agents, current)));
    const blocked = host.querySelector<HTMLButtonElement>('[data-row-id="blocked:omp"]')!;
    const working = host.querySelector<HTMLButtonElement>('[data-row-id="working:omp"]')!;
    const done = host.querySelector<HTMLButtonElement>('[data-row-id="done:helper"]')!;
    expect(blocked.querySelector(".agent-flag")?.textContent).toBe("Blocked");
    expect(done.querySelector(".agent-flag")?.textContent).toBe("Done");
    expect(blocked.querySelector(".agent-sub")).toBeNull();
    expect(blocked.querySelector(".agent-tab")?.textContent).toBe(" · 2");
    expect(working.querySelector(".agent-sub")?.textContent).toBe("Working · omp");
    expect(working.querySelector(".agent-flag")).toBeNull();
    expect(working.getAttribute("aria-label")).toBe("A very long Space name · 2, omp, working");
    await act(async () => blocked.focus());
    expect(blocked.querySelector(".agent-sub")).toBeNull();
    await act(async () => blocked.click());
    expect(current.onSelect).toHaveBeenCalledWith(current.agents[0]);
    expect(blocked.getAttribute("aria-current")).toBeNull();
    await act(async () => root.render(createElement(Agents, { ...current, selectedPaneId: "blocked", pendingPaneId: "blocked" })));
    expect(host.querySelector(".agent-sub")).toBeNull();
    expect(blocked.querySelector(".agent-flag")?.textContent).toBe("Blocked");
    await act(async () => root.render(createElement(Agents, { ...current, selectedPaneId: "blocked" })));
    expect(blocked.querySelector(".agent-sub")?.textContent).toBe("Blocked · omp");
    expect(blocked.querySelector(".agent-flag")).toBeNull();
    await act(async () => root.render(createElement(Agents, { ...current, spaces: [], tabs: [] })));
    expect(working.querySelector(".agent-line .agent-sub")?.textContent).toBe("Working · omp");
    expect(working.querySelectorAll(".agent-sub")).toHaveLength(1);
  } finally { await act(async () => root.unmount()); host.remove(); }
});
