// @vitest-environment jsdom
import { act, createElement, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentAttentionResponse, SessionSnapshotResponse } from "../../protocol/generated/v1";
import { Agents } from "./Agents";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const snapshot = { session_id: "s", server_instance: "server", panes: [{ id: "a", terminal_id: "ta" }, { id: "b", terminal_id: "tb" }], agents: [
  { pane_id: "a", name: "one", status: "idle", state_change_seq: 2, focused: false },
  { pane_id: "b", name: "two", status: "done", state_change_seq: 1, focused: false },
] } as SessionSnapshotResponse;
const response: AgentAttentionResponse = { session_id: "s", server_instance: "server", epoch: 1, refused: null, records: [] };

function mockStorage() {
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  });
}

it("dismisses only on response, hands off focus, and restores from All", async () => {
  mockStorage();
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const dismiss = vi.fn(async () => true); const restore = vi.fn(async () => true); const seen = vi.fn(async () => true);
  const attention: ComponentProps<typeof Agents>["attention"] = { availability: "ready", response, pending: new Set(), notes: new Map(), announcement: "", recoveryIncomplete: false, acknowledgeRecoveryWarning: vi.fn(), activate: vi.fn(), dismiss, restore, seen, seenHistorical: vi.fn(async () => true), restoreAll: vi.fn(async () => true), retry: vi.fn(), clearNote: vi.fn() };
  const props: ComponentProps<typeof Agents> = { snapshot, attention, live: true, spaces: [], tabs: [], selectedPaneId: null, pendingPaneId: null, loading: false, hasSession: true, onSelect: vi.fn(), focusRef: { current: null } };
  try {
    await act(async () => root.render(createElement(Agents, props)));
    const row = host.querySelector<HTMLButtonElement>('[data-row-id="a:one"]')!;
    await act(async () => { row.focus(); row.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true })); });
    expect(dismiss).toHaveBeenCalledOnce(); expect(host.contains(row)).toBe(true);
    const hidden = { ...response, records: [{ historical_id: null, pane_id: "a", terminal_id: "ta", agent: "one", last: { epoch: 1, seq: 2, status: "idle", observed_at: "now" }, completion: null, acked: { epoch: 1, seq: 2, at: "now" }, dismissed: { epoch: 1, seq: 2, at: "now" } }] };
    await act(async () => root.render(createElement(Agents, { ...props, attention: { ...attention, response: hidden } })));
    expect(host.contains(row)).toBe(false); expect((document.activeElement as HTMLElement).dataset.rowId).toBe("b:two");
    const showAll = [...host.querySelectorAll("button")].find(button => button.textContent === "Show all")!;
    await act(async () => showAll.click());
    const restoredRow = host.querySelector<HTMLButtonElement>('[data-row-id="a:one"]')!;
    await act(async () => { restoredRow.focus(); restoredRow.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true })); });
    expect(restore).toHaveBeenCalledOnce();
    const done = host.querySelector<HTMLButtonElement>('[data-row-id="b:two"]')!;
    await act(async () => done.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true })));
    expect(seen).toHaveBeenCalledOnce();
  } finally { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); }
});

it("fails visible when unavailable and fences a pointer that moved to another row", async () => {
  mockStorage(); const host = document.createElement("div"); document.body.append(host); const root = createRoot(host); const select = vi.fn();
  const attention: ComponentProps<typeof Agents>["attention"] = { availability: "unavailable", response: null, pending: new Set(), notes: new Map(), announcement: "", recoveryIncomplete: true, acknowledgeRecoveryWarning: vi.fn(), activate: vi.fn(), dismiss: vi.fn(async () => true), restore: vi.fn(async () => true), seen: vi.fn(async () => true), seenHistorical: vi.fn(async () => true), restoreAll: vi.fn(async () => true), retry: vi.fn(), clearNote: vi.fn() };
  try {
    await act(async () => root.render(createElement(Agents, { snapshot, attention, live: true, spaces: [], tabs: [], selectedPaneId: null, pendingPaneId: null, loading: false, hasSession: true, onSelect: select, focusRef: { current: null } })));
    expect(host.textContent).toContain("Results may be missing");
    await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === "Acknowledge")!.click());
    expect(attention.acknowledgeRecoveryWarning).toHaveBeenCalledOnce();
    const a = host.querySelector<HTMLButtonElement>('[data-row-id="a:one"]')!; const b = host.querySelector<HTMLButtonElement>('[data-row-id="b:two"]')!;
    expect(a).toBeTruthy(); expect(b).toBeTruthy();
    await act(async () => { a.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })); b.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 })); });
    expect(select).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); }
});

it("marks a past result seen without activating the replacement pane", async () => {
  mockStorage();
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const historical = { historical_id: "history-1", pane_id: "a", terminal_id: "old", agent: "one", last: { epoch: 1, seq: 1, status: "done", observed_at: "now" }, completion: { epoch: 1, seq: 1, at: "now", inferred: false }, acked: null, dismissed: null };
  const seenHistorical = vi.fn(async () => true); const onSelect = vi.fn();
  const attention: ComponentProps<typeof Agents>["attention"] = { availability: "ready", response: { ...response, records: [historical] }, pending: new Set(), notes: new Map(), announcement: "", recoveryIncomplete: false, acknowledgeRecoveryWarning: vi.fn(), activate: vi.fn(), dismiss: vi.fn(async () => true), restore: vi.fn(async () => true), seen: vi.fn(async () => true), seenHistorical, restoreAll: vi.fn(async () => true), retry: vi.fn(), clearNote: vi.fn() };
  try {
    await act(async () => root.render(createElement(Agents, { snapshot, attention, live: true, spaces: [], tabs: [], selectedPaneId: null, pendingPaneId: null, loading: false, hasSession: true, onSelect, focusRef: { current: null } })));
    await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === "Show all")!.click());
    const mark = host.querySelector<HTMLButtonElement>('[aria-label="Mark past result from one seen"]')!;
    expect(mark).toBeTruthy();
    await act(async () => mark.click());
    expect(seenHistorical).toHaveBeenCalledWith(historical);
    expect(onSelect).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); }
});
