// @vitest-environment jsdom
import { act, createElement, useState } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { FilePicker } from "./FilePicker";
import { FILE_NAVIGATION_EVENT, rankFileMatches, rankFuzzyMatches } from "./fileNavigation";

describe("file navigation", () => {
  it("uses case-insensitive subsequence matches and favors contiguous basename matches", () => {
    expect(rankFileMatches("ctx", [
      { id: "spread", path: "src/context/ContextViewer.tsx" },
      { id: "basename", path: "src/ctx.ts" },
      { id: "later", path: "src/components/contextual.ts" },
    ]).map((match) => match.id)).toEqual(["basename", "spread", "later"]);
  });
  it("ranks generic command candidates with the same subsequence positions", () => {
    const matches = rankFuzzyMatches("nsp", [
      { id: "new-space", label: "New Space" },
      { id: "next-pane", label: "Next pane" },
    ], (candidate) => candidate.label);
    expect(matches[0]?.matchedIndices.map((index) => "New Space"[index]).join("")).toBe("NSp");
    expect(matches[1]?.id).toBeUndefined();
  });


  it("keeps the selected result and DOM focus when polling replaces the candidate array", async () => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    const host = document.createElement("div");
    document.body.append(host);
    const mounted = createRoot(host);
    const candidates = [{ id: "one", path: "one.ts" }, { id: "two", path: "two.ts" }];
    const choose = vi.fn();
    const render = (items = candidates) => mounted.render(createElement(FilePicker, { candidates: items, onChoose: choose, onDismiss: vi.fn() }));
    try {
      await act(async () => render());
      const input = host.querySelector<HTMLInputElement>("input")!;
      await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowDown" })));
      const second = host.querySelector<HTMLButtonElement>("[data-file-picker-result-index='1']")!;
      await act(async () => second.focus());
      await act(async () => render(candidates.map((candidate) => ({ ...candidate }))));
      expect(second.getAttribute("aria-selected")).toBe("true");
      expect(document.activeElement).toBe(second);
      // Indexing may insert earlier results; selection follows file identity.
      await act(async () => render([{ id: "zero", path: "zero.ts" }, ...candidates]));
      expect(second.getAttribute("aria-selected")).toBe("true");
      expect(document.activeElement).toBe(second);
      await act(async () => second.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" })));
      expect(choose).toHaveBeenLastCalledWith(expect.objectContaining({ id: "two" }));
    } finally {
      await act(async () => mounted.unmount());
      host.remove();
    }
  });

  it("reports the actual fuzzy match positions, including case and Unicode", () => {
    for (const [query, path, expected] of [["FF2", "focus-file-02.md", "ff2"], ["😀m", "😀-memo.md", "😀m"], ["i", "İtem.txt", "İ"]]) {
      const match = rankFileMatches(query, [{ id: "file", path }])[0];
      expect(match.matchedIndices.map((index) => Array.from(path)[index]).join("")).toBe(expected);
    }
    expect(rankFileMatches("", [{ id: "file", path: "file.md" }])[0].matchedIndices).toEqual([]);
    expect(rankFileMatches("zz", [{ id: "file", path: "file.md" }])).toEqual([]);
  });

  it("highlights matching path characters and resets selection only for a changed query", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const mounted = createRoot(host);
    try {
      await act(async () => mounted.render(createElement(FilePicker, { candidates: [{ id: "one", path: "focus-file-01.md" }, { id: "two", path: "focus-file-02.md" }], onChoose: vi.fn(), onDismiss: vi.fn() })));
      const input = host.querySelector<HTMLInputElement>("input")!;
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "ff2");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      expect([...host.querySelectorAll("mark")].map((mark) => mark.textContent).join("")).toBe("ff2");
      expect(host.querySelector("[aria-selected='true'] code")?.textContent).toBe("focus-file-02.md");
      expect(document.activeElement).toBe(input);
    } finally {
      await act(async () => mounted.unmount());
      host.remove();
    }
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
