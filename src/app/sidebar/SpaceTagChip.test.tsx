// @vitest-environment jsdom
import { act, createElement, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { SpaceTagChip } from "./SpaceTagChip";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => { for (const cleanup of cleanups.splice(0)) await cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function mount(overrides: Partial<ComponentProps<typeof SpaceTagChip>> = {}) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const props: ComponentProps<typeof SpaceTagChip> = { spaceLabel: "login-flow", tags: ["My check", "Needs testing", "Demo Friday"], showAdd: false, showAll: false, expanded: false, disabled: null, busy: false, onToggle: vi.fn(), ...overrides };
  await act(async () => root.render(createElement(SpaceTagChip, props)));
  cleanups.push(async () => { await act(async () => root.unmount()); host.remove(); });
  return { host, props, button: host.querySelector<HTMLButtonElement>("button"), render: async (next: Partial<typeof props>) => { Object.assign(props, next); await act(async () => root.render(createElement(SpaceTagChip, props))); } };
}

it("shows every tag with its colour, with all tag names available and no row selection", async () => {
  const { host, button, props } = await mount();
  expect(button!.className).toBe("space-tag-dots");
  expect([...button!.querySelectorAll(":scope > .space-tag")].map(tag => tag.textContent)).toEqual(["My check", "Needs testing", "Demo Friday"]);
  expect(button!.title).toBe("My check, Needs testing, Demo Friday");
  expect(button!.getAttribute("aria-label")).toBe("Edit tags for login-flow: My check, Needs testing, Demo Friday");
  expect(button!.getAttribute("aria-haspopup")).toBe("dialog");
  expect(button!.getAttribute("aria-expanded")).toBe("false");
  expect(button!.tabIndex).toBe(-1);
  const rowClick = vi.fn();
  document.addEventListener("click", rowClick);
  cleanups.push(async () => document.removeEventListener("click", rowClick));
  await act(async () => button!.click());
  expect(props.onToggle).toHaveBeenCalledWith(button);
  expect(rowClick).not.toHaveBeenCalled();
});

it("excludes the group tag, and omits the trigger for a Space with only that tag", async () => {
  const other = await mount({ exclude: "My check" });
  expect([...other.button!.querySelectorAll(":scope > .space-tag")].map(tag => tag.textContent)).toEqual(["Needs testing", "Demo Friday"]);
  const only = await mount({ tags: ["My check"], exclude: "My check" });
  expect(only.button).toBeNull();
  const empty = await mount({ tags: [] });
  expect(empty.button).toBeNull();
});

it("offers + tag for untagged Spaces and keeps blocked triggers focusable and inert", async () => {
  const { button, props } = await mount({ tags: [], showAdd: true, expanded: true, busy: true, disabled: "Tags are loading" });
  expect(button!.className).toBe("space-tag-add");
  expect(button!.textContent).toBe("+ tag");
  expect(button!.getAttribute("aria-label")).toBe("Add tag to login-flow");
  expect(button!.getAttribute("aria-expanded")).toBe("true");
  expect(button!.getAttribute("aria-disabled")).toBe("true");
  expect(button!.getAttribute("aria-busy")).toBe("true");
  expect(button!.disabled).toBe(false);
  await act(async () => { button!.focus(); button!.click(); });
  expect(document.activeElement).toBe(button);
  expect(props.onToggle).not.toHaveBeenCalled();
});

it("refits on container resize and tag changes, keeping hidden copies inaccessible", async () => {
  let width = 150;
  let resize!: ResizeObserverCallback;
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: ResizeObserverCallback) { resize = callback; }
    observe() {}
    disconnect() {}
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return { width: this.classList.contains("space-tag") ? 60 : this.classList.contains("space-tag-more") ? 20 : width } as DOMRect;
  });
  const { button, render } = await mount();
  const visible = () => button!.querySelectorAll(":scope > .space-tag");
  expect(visible()).toHaveLength(2);
  expect(button!.querySelector(":scope > .space-tag-more")?.textContent).toBe("+1");
  expect(button!.querySelector(".space-tag-measure")?.getAttribute("aria-hidden")).toBe("true");
  expect(button!.getAttribute("aria-label")).toContain("My check, Needs testing, Demo Friday");
  width = 210;
  await act(async () => resize([], {} as ResizeObserver));
  expect(visible()).toHaveLength(3);
  expect(button!.querySelector(":scope > .space-tag-more")).toBeNull();
  width = 80;
  await act(async () => resize([], {} as ResizeObserver));
  expect(visible()).toHaveLength(1);
  expect(button!.querySelector(":scope > .space-tag-more")?.textContent).toBe("+2");
  await render({ tags: ["Only"] });
  expect(visible()).toHaveLength(1);
  expect(button!.querySelector(".space-tag-measure")).toBeNull();
});

it("wraps the full selected set without overflow badges or measuring copies", async () => {
  const { button, render } = await mount({ showAll: true });
  expect(button!.classList.contains("is-all")).toBe(true);
  expect(button!.querySelectorAll(":scope > .space-tag")).toHaveLength(3);
  expect(button!.querySelector(".space-tag-more")).toBeNull();
  expect(button!.querySelector(".space-tag-measure")).toBeNull();
  await render({ showAll: false });
  expect(button!.classList.contains("is-all")).toBe(false);
  expect(button!.querySelector(".space-tag-measure")).not.toBeNull();
});
