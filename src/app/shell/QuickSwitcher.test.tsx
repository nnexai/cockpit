// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { QuickSwitcher } from "./QuickSwitcher";
import type { SwitchRow } from "./quickSwitch";

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
