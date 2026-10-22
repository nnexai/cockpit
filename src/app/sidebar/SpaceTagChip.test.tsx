// @vitest-environment jsdom
import { act, createElement, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { SpaceTagChip } from "./SpaceTagChip";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => { for (const cleanup of cleanups.splice(0)) await cleanup(); });
async function mount(overrides: Partial<ComponentProps<typeof SpaceTagChip>> = {}) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const props: ComponentProps<typeof SpaceTagChip> = { spaceLabel: "login-flow", tags: ["My check", "Needs testing", "Demo Friday"], showAdd: false, expanded: false, disabled: null, busy: false, onToggle: vi.fn(), ...overrides };
  await act(async () => root.render(createElement(SpaceTagChip, props)));
  cleanups.push(async () => { await act(async () => root.unmount()); host.remove(); });
  return { host, props, button: host.querySelector<HTMLButtonElement>("button") };
}

it("shows catalog-first tag and +N, with all tag names available and no row selection", async () => {
  const { host, button, props } = await mount();
  expect(button!.className).toBe("space-tag-chip");
  expect(button!.querySelector(".space-tag-name")?.textContent).toBe("My check");
  expect(button!.querySelector(".space-tag-more")?.textContent).toBe("+2");
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

it("excludes the group tag, and omits a chip for a Space with only that tag", async () => {
  const other = await mount({ exclude: "My check" });
  expect(other.button!.querySelector(".space-tag-name")?.textContent).toBe("Needs testing");
  expect(other.button!.querySelector(".space-tag-more")?.textContent).toBe("+1");
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
