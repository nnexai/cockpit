// @vitest-environment jsdom
import { act, createElement, useState, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentAttentionResponse, SessionSnapshotResponse } from "../../protocol/generated/v1";
import { focusSidebarList, registerSidebarFocus } from "../input/shortcuts";
import { Agents } from "./Agents";
import { AGENTS_VIEW_KEY, readAgentsView, writeAgentsView, type AgentsView } from "./agentAttentionModel";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const snapshot = { session_id: "s", server_instance: "server", panes: [{ id: "a", terminal_id: "ta" }, { id: "b", terminal_id: "tb" }], agents: [
  { pane_id: "a", name: "one", status: "idle", state_change_seq: 2, focused: false },
  { pane_id: "b", name: "two", status: "done", state_change_seq: 1, focused: false },
] } as SessionSnapshotResponse;
const response: AgentAttentionResponse = { session_id: "s", server_instance: "server", epoch: 1, refused: null, records: [] };


it("filters idle agents by default and acknowledges an unseen result", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const seen = vi.fn(async () => true);
  const attention: ComponentProps<typeof Agents>["attention"] = { availability: "ready", response, pending: new Set(), notes: new Map(), announcement: "", activate: vi.fn(), seen, retry: vi.fn(), clearNote: vi.fn() };
  const props: ComponentProps<typeof Agents> = { snapshot, attention, live: true, spaces: [], tabs: [], selectedPaneId: null, pendingPaneId: null, loading: false, hasSession: true, onSelect: vi.fn(), focusRef: { current: null }, view: "attention", onViewChange: vi.fn() };
  try {
    await act(async () => root.render(createElement(Agents, props)));
    expect(host.querySelector('[data-row-id="a:one"]')).toBeNull();
    expect(host.textContent).not.toMatch(/Dismiss|Restore/);
    const done = host.querySelector<HTMLButtonElement>('[data-row-id="b:two"]')!;
    expect(done).toBeTruthy();
    expect(done.querySelector(".agent-flag")?.textContent).toBe("Done");
    await act(async () => done.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true })));
    expect(seen).toHaveBeenCalledOnce();
    const retained = { ...response, records: [{ pane_id: "a", terminal_id: "ta", agent: "one", last: { epoch: 1, seq: 2, status: "idle", observed_at: "now" }, completion: { epoch: 1, seq: 1, at: "now", inferred: false }, acked: null }] };
    await act(async () => root.render(createElement(Agents, { ...props, attention: { ...attention, response: retained } })));
    expect(host.querySelector('[data-row-id="a:one"]')?.getAttribute("aria-label")).toContain("not seen");
    expect(host.querySelector('[data-row-id="a:one"] .agent-flag')?.textContent).toBe("Done");
    const working = { ...snapshot, agents: [{ ...snapshot.agents[0]!, status: "working" }, snapshot.agents[1]!] };
    await act(async () => root.render(createElement(Agents, { ...props, snapshot: working, attention: { ...attention, response: retained } })));
    expect(host.querySelector('[data-row-id="a:one"] .agent-flag')?.textContent).toBe("Unseen");
    const acknowledged = { ...retained, records: [{ ...retained.records[0]!, acked: { epoch: 1, seq: 1, at: "now" } }] };
    await act(async () => root.render(createElement(Agents, { ...props, attention: { ...attention, response: acknowledged } })));
    expect(host.querySelector('[data-row-id="a:one"]')).toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("keeps active statuses visible when unavailable and fences a pointer that moved to another row", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host); const select = vi.fn();
  const active = { ...snapshot, agents: [{ ...snapshot.agents[0]!, status: "working" }, snapshot.agents[1]!] };
  const attention: ComponentProps<typeof Agents>["attention"] = { availability: "unavailable", response: null, pending: new Set(), notes: new Map(), announcement: "", activate: vi.fn(), seen: vi.fn(async () => true), retry: vi.fn(), clearNote: vi.fn() };
  try {
    await act(async () => root.render(createElement(Agents, { snapshot: active, attention, live: true, spaces: [], tabs: [], selectedPaneId: null, pendingPaneId: null, loading: false, hasSession: true, onSelect: select, focusRef: { current: null }, view: "attention", onViewChange: vi.fn() })));
    expect(host.textContent).toContain("Attention unavailable.");
    await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === "Retry")!.click());
    expect(attention.retry).toHaveBeenCalledOnce();
    const a = host.querySelector<HTMLButtonElement>('[data-row-id="a:one"]')!; const b = host.querySelector<HTMLButtonElement>('[data-row-id="b:two"]')!;
    expect(a).toBeTruthy(); expect(b).toBeTruthy();
    await act(async () => { a.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })); b.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 })); });
    expect(select).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); host.remove(); }
});


it("focuses the row that takes the removed row's position", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const attention: ComponentProps<typeof Agents>["attention"] = { availability: "unavailable", response: null, pending: new Set(), notes: new Map(), announcement: "", activate: vi.fn(), seen: vi.fn(async () => true), retry: vi.fn(), clearNote: vi.fn() };
  const active = { ...snapshot, agents: snapshot.agents.map(agent => ({ ...agent, status: "working" })) };
  const props: ComponentProps<typeof Agents> = { snapshot: active, attention, live: true, spaces: [], tabs: [], selectedPaneId: null, pendingPaneId: null, loading: false, hasSession: true, onSelect: vi.fn(), focusRef: { current: null }, view: "attention", onViewChange: vi.fn() };
  try {
    await act(async () => root.render(createElement(Agents, props)));
    const rows = [...host.querySelectorAll<HTMLButtonElement>("[data-row-id]")];
    await act(async () => rows[0]!.focus());
    const remaining = active.agents.filter(agent => `${agent.pane_id}:${agent.name}` !== rows[0]!.dataset.rowId);
    await act(async () => root.render(createElement(Agents, { ...props, snapshot: { ...active, agents: remaining } })));
    expect(document.activeElement).toBe(rows[1]);
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("hands focus to the adjacent remaining row, then back to the sidebar origin", async () => {
  const host = document.createElement("div"); host.id = "cockpit-sidebar"; document.body.append(host);
  const origin = document.createElement("button"); document.body.append(origin);
  const root = createRoot(host);
  const attention: ComponentProps<typeof Agents>["attention"] = { availability: "unavailable", response: null, pending: new Set(), notes: new Map(), announcement: "", activate: vi.fn(), seen: vi.fn(async () => true), retry: vi.fn(), clearNote: vi.fn() };
  const active = { ...snapshot, agents: snapshot.agents.map(agent => ({ ...agent, status: "working" })) };
  const focusRef: ComponentProps<typeof Agents>["focusRef"] = { current: null };
  const props: ComponentProps<typeof Agents> = { snapshot: active, attention, live: true, spaces: [], tabs: [], selectedPaneId: null, pendingPaneId: null, loading: false, hasSession: true, onSelect: vi.fn(), focusRef, view: "attention", onViewChange: vi.fn() };
  const unregister = registerSidebarFocus({ spaces: () => {}, agents: () => focusRef.current?.() });
  try {
    await act(async () => root.render(createElement(Agents, props)));
    origin.focus();
    await act(async () => { focusSidebarList("agents"); });
    const rows = [...host.querySelectorAll<HTMLButtonElement>("[data-row-id]")];
    expect(rows).toHaveLength(2);
    await act(async () => rows[1]!.focus());
    const remaining = active.agents.filter(agent => `${agent.pane_id}:${agent.name}` !== rows[1]!.dataset.rowId);
    await act(async () => root.render(createElement(Agents, { ...props, snapshot: { ...active, agents: remaining } })));
    expect(document.activeElement).toBe(host.querySelector("[data-row-id]"));
    await act(async () => root.render(createElement(Agents, { ...props, snapshot: { ...active, agents: [] } })));
    expect(document.activeElement).toBe(origin);
  } finally { unregister(); await act(async () => root.unmount()); host.remove(); origin.remove(); }
});

it("does not steal focus when a previously focused row disappears after focus leaves", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const outside = document.createElement("button"); document.body.append(outside);
  const root = createRoot(host);
  const attention: ComponentProps<typeof Agents>["attention"] = { availability: "unavailable", response: null, pending: new Set(), notes: new Map(), announcement: "", activate: vi.fn(), seen: vi.fn(async () => true), retry: vi.fn(), clearNote: vi.fn() };
  const active = { ...snapshot, agents: snapshot.agents.map(agent => ({ ...agent, status: "working" })) };
  const props: ComponentProps<typeof Agents> = { snapshot: active, attention, live: true, spaces: [], tabs: [], selectedPaneId: null, pendingPaneId: null, loading: false, hasSession: true, onSelect: vi.fn(), focusRef: { current: null }, view: "attention", onViewChange: vi.fn() };
  try {
    await act(async () => root.render(createElement(Agents, props)));
    const row = host.querySelector<HTMLButtonElement>("[data-row-id]")!;
    await act(async () => { row.focus(); outside.focus(); });
    await act(async () => root.render(createElement(Agents, { ...props, snapshot: { ...active, agents: active.agents.filter(agent => `${agent.pane_id}:${agent.name}` !== row.dataset.rowId) } })));
    expect(document.activeElement).toBe(outside);
    await act(async () => { host.querySelector<HTMLButtonElement>("[data-row-id]")!.focus(); (document.activeElement as HTMLElement).blur(); });
    await act(async () => root.render(createElement(Agents, { ...props, snapshot: { ...active, agents: [] } })));
    expect(document.activeElement).toBe(document.body);
  } finally { await act(async () => root.unmount()); host.remove(); outside.remove(); }
});

it("toggles all agents, retains focus and counts, and keeps idle activation separate from acknowledgement", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const select = vi.fn();
  const attention: ComponentProps<typeof Agents>["attention"] = { availability: "unavailable", response: null, pending: new Set(), notes: new Map(), announcement: "", activate: vi.fn(), seen: vi.fn(async () => true), retry: vi.fn(), clearNote: vi.fn() };
  function Fixture() {
    const [view, setView] = useState<AgentsView>("attention");
    return createElement(Agents, { snapshot, attention, live: true, spaces: [], tabs: [], selectedPaneId: null, pendingPaneId: null, loading: false, hasSession: true, onSelect: select, focusRef: { current: null }, view, onViewChange: setView });
  }
  try {
    await act(async () => root.render(createElement(Fixture)));
    const toggle = host.querySelector<HTMLButtonElement>(".agents-view-toggle")!;
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    expect(toggle.getAttribute("aria-label")).toBe("Show all agents, 1 idle hidden");
    expect(host.querySelector(".section-count")?.textContent).toBe("1");
    await act(async () => toggle.click());
    expect(document.activeElement).toBe(toggle);
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    expect(host.querySelector(".section-count")?.textContent).toBe("2");
    expect([...host.querySelectorAll("[data-row-id]")].map(row => row.getAttribute("data-row-id"))).toEqual(["b:two", "a:one"]);
    expect(host.textContent).toContain("Showing all 2 agents");
    const idle = host.querySelector<HTMLButtonElement>('[data-row-id="a:one"]')!;
    await act(async () => idle.click());
    expect(select).toHaveBeenCalledOnce();
    await act(async () => idle.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true })));
    expect(attention.seen).not.toHaveBeenCalled();
    await act(async () => toggle.click());
    expect(host.querySelector('[data-row-id="a:one"]')).toBeNull();
    expect(host.textContent).toContain("Showing 1 agents that need attention");
    expect(host.textContent).toContain("Attention unavailable.");
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("persists the agent view and tolerates unavailable preference storage", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: vi.fn((key: string) => values.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => { values.set(key, value); }),
  };
  vi.stubGlobal("localStorage", storage);
  try {
    expect(readAgentsView()).toBe("attention");
    writeAgentsView("all");
    expect(values.get(AGENTS_VIEW_KEY)).toBe("all");
    expect(readAgentsView()).toBe("all");
    writeAgentsView("attention");
    expect(readAgentsView()).toBe("attention");
    values.set(AGENTS_VIEW_KEY, "invalid");
    expect(readAgentsView()).toBe("attention");
    storage.getItem.mockImplementation(() => { throw new Error("denied"); });
    storage.setItem.mockImplementation(() => { throw new Error("denied"); });
    expect(readAgentsView()).toBe("attention");
    expect(() => writeAgentsView("all")).not.toThrow();
  } finally { vi.unstubAllGlobals(); }
});

it("distinguishes empty all view from no session", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const attention: ComponentProps<typeof Agents>["attention"] = { availability: "unavailable", response: null, pending: new Set(), notes: new Map(), announcement: "", activate: vi.fn(), seen: vi.fn(async () => true), retry: vi.fn(), clearNote: vi.fn() };
  const props: ComponentProps<typeof Agents> = { snapshot: { ...snapshot, agents: [] }, attention, live: true, spaces: [], tabs: [], selectedPaneId: null, pendingPaneId: null, loading: false, hasSession: true, onSelect: vi.fn(), focusRef: { current: null }, view: "all", onViewChange: vi.fn() };
  try {
    await act(async () => root.render(createElement(Agents, props)));
    expect(host.textContent).toContain("No agents in this session");
    await act(async () => root.render(createElement(Agents, { ...props, snapshot: null, hasSession: false })));
    expect(host.textContent).toContain("No session selected");
    expect(host.querySelector<HTMLButtonElement>(".agents-view-toggle")?.disabled).toBe(true);
  } finally { await act(async () => root.unmount()); host.remove(); }
});
