// @vitest-environment jsdom
import { act, createElement, useState } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { FilePicker } from "./FilePicker";
import { FILE_NAVIGATION_EVENT, rankFileMatches } from "./fileNavigation";

describe("file navigation", () => {
  it("uses case-insensitive subsequence matches and favors contiguous basename matches", () => {
    expect(rankFileMatches("ctx", [
      { id: "spread", path: "src/context/ContextViewer.tsx" },
      { id: "basename", path: "src/ctx.ts" },
      { id: "later", path: "src/components/contextual.ts" },
    ]).map((match) => match.id)).toEqual(["basename", "spread", "later"]);
  });

  it("publishes one stable event name for workbench prefix routing", () => {
    expect(FILE_NAVIGATION_EVENT).toBe("cockpit:file-navigation");
  });

  it("traps keyboard focus, closes from a result, and restores the opener", async () => {
    const trigger = document.createElement("button");
    const host = document.createElement("div");
    document.body.append(trigger, host);
    trigger.focus();
    const mounted = createRoot(host);
    function Harness() {
      const [open, setOpen] = useState(true);
      return open ? createElement(FilePicker, { candidates: [{ id: "one", path: "one.ts" }, { id: "two", path: "two.ts" }], onChoose: () => setOpen(false), onDismiss: () => setOpen(false) }) : null;
    }
    try {
      await act(async () => mounted.render(createElement(Harness)));
      const input = host.querySelector<HTMLInputElement>(".file-picker input")!;
      const results = host.querySelectorAll<HTMLButtonElement>(".file-picker-results button");
      expect(document.activeElement).toBe(input);
      await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Tab", shiftKey: true })));
      expect(document.activeElement).toBe(results[1]);
      await act(async () => results[1]!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Escape" })));
      expect(host.querySelector(".file-picker")).toBeNull();
      expect(document.activeElement).toBe(trigger);
    } finally {
      await act(async () => mounted.unmount());
      trigger.remove();
      host.remove();
    }
  });

  it("keeps the active result visible during keyboard navigation", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const mounted = createRoot(host);
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: scrollIntoView });
    try {
      await act(async () => mounted.render(createElement(FilePicker, { candidates: [{ id: "one", path: "one.ts" }, { id: "two", path: "two.ts" }], onChoose: vi.fn(), onDismiss: vi.fn() })));
      scrollIntoView.mockClear();
      const input = host.querySelector<HTMLInputElement>(".file-picker input")!;
      await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowDown" })));
      expect(scrollIntoView).toHaveBeenCalledWith({ block: "nearest" });
      expect(host.querySelector<HTMLButtonElement>("[data-file-picker-result-index='1']")?.getAttribute("aria-selected")).toBe("true");
    } finally {
      await act(async () => mounted.unmount());
      host.remove();
    }
  });
});
