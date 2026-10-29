// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { QuickSwitcher } from "./QuickSwitcher";
import { switchRows, switchSearchText, type SwitchRow } from "./quickSwitch";
import { rankFuzzyMatches } from "../input/fileNavigation";
import type { Agent } from "../sidebar/spaceTree";
import type { WidgetSummary } from "../../protocol/generated/v1";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
afterEach(() => document.body.replaceChildren());

function row(id: string): SwitchRow {
  return { kind: "space", id, key: `space:${id}`, primary: id, meta: "Idle", title: id, status: { shape: "idle", className: "idle", word: "Idle" }, current: false, disabledReason: null };
}

function mount(rows: SwitchRow[]) {
  const opener = document.createElement("button");
  document.body.append(opener);
  opener.focus();
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onChoose = vi.fn();
  const onDismiss = vi.fn();
  const render = (next: SwitchRow[]) => act(() => root.render(<QuickSwitcher rows={next} hasSession loading={false} live onChoose={onChoose} onDismiss={onDismiss} />));
  render(rows);
  const key = (value: string, isComposing = false) => act(() => { host.querySelector("input")!.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true, isComposing })); });
  return { host, opener, render, key, onChoose, onDismiss, unmount: () => act(() => root.unmount()) };
}

describe("quick switcher live identity and input", () => {
  it("follows a highlighted identity across reordering and drops a removed identity", () => {
    const probe = mount([row("Alpha"), row("Beta"), row("Gamma")]);
    try {
      probe.key("ArrowDown");
      probe.render([row("Gamma"), row("Alpha"), row("Beta")]);
      probe.key("Enter");
      expect(probe.onChoose).toHaveBeenLastCalledWith({ kind: "space", id: "Beta" });
      probe.render([row("Gamma"), row("Alpha")]);
      probe.key("Enter");
      expect(probe.onChoose).toHaveBeenLastCalledWith({ kind: "space", id: "Gamma" });
      const input = probe.host.querySelector("input")!;
      expect(document.getElementById(input.getAttribute("aria-activedescendant")!)?.textContent).toContain("Gamma");
    } finally { probe.unmount(); }
    expect(document.activeElement).toBe(probe.opener);
  });

  it("does not choose unavailable or composing targets, or close on composing Escape", () => {
    const probe = mount([{ ...row("Alpha"), disabledReason: "Herdr is not live" }]);
    try {
      probe.key("Enter");
      expect(probe.onChoose).not.toHaveBeenCalled();
      probe.render([row("Alpha")]);
      probe.key("Enter", true);
      probe.key("Escape", true);
      expect(probe.onChoose).not.toHaveBeenCalled();
      expect(probe.onDismiss).not.toHaveBeenCalled();
      probe.key("Enter");
      expect(probe.onChoose).toHaveBeenCalledExactlyOnceWith({ kind: "space", id: "Alpha" });
    } finally { probe.unmount(); }
  });
});

it("draws a gray dot for a no-agent Space while preserving the unknown-agent ring", () => {
  const unknown = { shape: "unknown", className: "unknown", word: "Unknown" } as const;
  const probe = mount([{ ...row("Empty Space"), status: unknown }, { ...row("Unknown agent"), kind: "agent", status: unknown }]);
  try {
    const rows = probe.host.querySelectorAll(".switcher-row");
    expect(rows[0].querySelector(".sb-glyph.is-none circle")?.getAttribute("r")).toBe("2.75");
    expect(rows[1].querySelector(".sb-glyph.is-unknown circle")?.getAttribute("r")).toBe("8.25");
  } finally { probe.unmount(); }
});

it("includes live and closed widgets and searches the visible agent title", () => {
  const agent = { name: "Worker", title: "Investigating latency", pane_id: "pane", space_id: "space", tab_id: "tab", status: "working", state_change_seq: 1, focused: false } as Agent;
  const live = { key: { session_id: "session", tab_id: "tab", id: "live" }, space_id: "space", title: "Live chart" } as WidgetSummary;
  const closed = { ...live, key: { ...live.key, id: "closed" }, title: "Closed chart" };
  const rows = switchRows({ spaces: [], tabs: [], agents: [agent], widgets: [live], closedWidgets: [closed], currentSpaceId: null, currentPaneId: null, live: true, busy: false });
  expect(rows.find(item => item.kind === "agent")?.meta).toContain("Investigating latency");
  expect(rankFuzzyMatches("Investigating latency", rows, switchSearchText).map(item => item.kind)).toEqual(["agent"]);
  expect(rows.filter(item => item.kind.includes("widget")).map(item => [item.kind, item.primary, item.meta])).toEqual([
    ["widget", "Live chart", "Widget"], ["closed-widget", "Closed chart", "Closed widget"],
  ]);
  const probe = mount(rows);
  try {
    const closedRow = Array.from(probe.host.querySelectorAll("button[role=option]")).find(button => button.textContent?.includes("Closed chart")) as HTMLButtonElement;
    act(() => closedRow.click());
    expect(probe.onChoose).toHaveBeenCalledWith({ kind: "closed-widget", id: JSON.stringify(["session", "tab", "closed"]) });
  } finally { probe.unmount(); }
});
