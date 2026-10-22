// @vitest-environment jsdom
import { act, createElement, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { SpaceTagPopover } from "./SpaceTagPopover";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
type Props = ComponentProps<typeof SpaceTagPopover>;
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => { for (const cleanup of cleanups.splice(0)) await cleanup(); vi.restoreAllMocks(); });

async function mount(overrides: Partial<Props> = {}) {
  const host = document.createElement("div");
  const trigger = document.createElement("button");
  document.body.append(trigger, host);
  const root = createRoot(host);
  let props: Props = {
    spaceLabel: "login-flow", catalog: ["My check", "Needs testing", "Demo Friday"], tags: [],
    busy: false, blocked: null, onChange: vi.fn(async () => true), onClose: vi.fn(), triggerRef: { current: trigger }, ...overrides,
  };
  const render = async (next: Partial<Props> = {}) => { props = { ...props, ...next }; await act(async () => root.render(createElement(SpaceTagPopover, props))); };
  cleanups.push(async () => { await act(async () => root.unmount()); host.remove(); trigger.remove(); });
  await render();
  return { host, trigger, props, render, input: host.querySelector<HTMLInputElement>('input:not([type="checkbox"])')! };
}

async function type(input: HTMLInputElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function key(target: HTMLElement, value: string, options: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true, ...options });
  await act(async () => { target.dispatchEvent(event); });
  return event;
}
function deferred() {
  let resolve!: (acknowledged: boolean) => void;
  const promise = new Promise<boolean>(finish => { resolve = finish; });
  return { promise, resolve };
}

it("opens non-modally with input focus, live catalog-order filtering and no empty Create row", async () => {
  const { host, input, props } = await mount();
  const dialog = host.querySelector('[role="dialog"]')!;
  expect(dialog.getAttribute("aria-label")).toBe("Tags for login-flow");
  expect(dialog.hasAttribute("aria-modal")).toBe(false);
  expect(document.activeElement).toBe(input);
  expect(input.placeholder).toBe("Find or create tag…");
  expect(host.querySelector(".space-tag-create")).toBeNull();
  expect(host.querySelector(".space-tag-announcement")?.textContent).toBe("3 tags");
  await key(input, "Enter");
  expect(props.onChange).not.toHaveBeenCalled();
  await type(input, "need");
  expect([...host.querySelectorAll(".space-tag-option-name")].map(option => option.textContent)).toEqual(["Needs testing"]);
  expect(host.querySelector(".space-tag-create")?.textContent).toBe('Create "need"');
  expect(host.querySelector(".space-tag-announcement")?.textContent).toBe("1 tag");
  await type(input, "  DEMO  friday  ");
  expect(host.querySelector(".space-tag-create")).toBeNull();
  expect(host.querySelector(".space-tag-option-name")?.textContent).toBe("Demo Friday");
});

it("Enter ensures the exact catalog spelling, never toggles off, and clears the filter", async () => {
  const change = vi.fn(async () => true);
  const { host, input, render } = await mount({ onChange: change });
  await type(input, "needs TESTING");
  await key(input, "Enter");
  expect(change.mock.calls).toEqual([["Needs testing", true]]);
  expect(input.value).toBe("");
  expect(host.querySelectorAll('input[type="checkbox"]')).toHaveLength(3);
  expect(host.querySelector(".space-tag-announcement")?.textContent).toBe("Added Needs testing");
  await render({ tags: ["Needs testing"] });
  await type(input, "NEEDS testing");
  await key(input, "Enter");
  expect(change).toHaveBeenCalledTimes(1);
  expect(input.value).toBe("");
  expect(document.activeElement).toBe(input);
});

it("creates the normalized text, not the first filtered option, once and scrolls the acknowledged option", async () => {
  const change = vi.fn(async () => true);
  const { host, input, render } = await mount({ onChange: change });
  await type(input, "need");
  await key(input, "Enter");
  expect(change.mock.calls).toEqual([["need", true]]);
  expect(input.value).toBe("");
  expect(document.activeElement).toBe(input);
  const scroll = vi.fn();
  const originalScroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollIntoView");
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: scroll });
  cleanups.push(async () => {
    if (originalScroll) Object.defineProperty(HTMLElement.prototype, "scrollIntoView", originalScroll);
    else Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
  });
  await render({ catalog: ["My check", "Needs testing", "Demo Friday", "need"], tags: ["need"] });
  expect(scroll).toHaveBeenCalledWith({ block: "nearest" });
  const options = host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]');
  expect(options[3].checked).toBe(true);
  await type(input, "  Cafe\u0301   review ");
  await act(async () => host.querySelector<HTMLButtonElement>(".space-tag-create")!.click());
  expect(change).toHaveBeenLastCalledWith("Café review", true);
});

it("ignores IME Enter, empty text, and invalid or over-length names", async () => {
  const { host, input, props } = await mount();
  await type(input, "新しいタグ");
  await key(input, "Enter", { isComposing: true });
  expect(props.onChange).not.toHaveBeenCalled();
  await act(async () => { input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })); });
  await key(input, "Enter");
  expect(props.onChange).not.toHaveBeenCalled();
  await act(async () => { input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })); });
  await key(input, "Enter");
  expect(props.onChange).toHaveBeenCalledWith("新しいタグ", true);
  for (const invalid of ["   ", "a".repeat(33), "bad\u0000tag"]) {
    await type(input, invalid);
    expect(host.querySelector(".space-tag-create")).toBeNull();
    await key(input, "Enter");
  }
  expect(props.onChange).toHaveBeenCalledTimes(1);
});

it("uses native checkbox activation without an optimistic flip and preserves pending/failure focus", async () => {
  const pending = deferred();
  const change = vi.fn(() => pending.promise);
  const { host, input, render } = await mount({ tags: ["My check"], onChange: change });
  const checkbox = host.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
  await act(async () => checkbox.focus());
  await key(checkbox, "Enter");
  expect(change).not.toHaveBeenCalled();
  // The browser activates a checkbox with click after its Space key sequence.
  await key(checkbox, " ");
  await act(async () => checkbox.click());
  expect(change.mock.calls).toEqual([["My check", false]]);
  expect(checkbox.checked).toBe(true);
  expect(checkbox.disabled).toBe(false);
  expect(checkbox.getAttribute("aria-disabled")).toBe("true");
  expect(host.querySelector('[role="dialog"]')?.getAttribute("aria-busy")).toBe("true");
  expect(document.activeElement).toBe(checkbox);
  await act(async () => { checkbox.click(); checkbox.click(); });
  expect(change).toHaveBeenCalledTimes(1);
  expect(checkbox.checked).toBe(true);
  await render({ busy: true, catalog: ["My check", "Needs testing", "Demo Friday", "Other"] });
  expect(document.activeElement).toBe(checkbox);
  await act(async () => pending.resolve(false));
  expect(checkbox.checked).toBe(true);
  expect(document.activeElement).toBe(checkbox);
  expect(host.querySelector(".space-tag-status")?.getAttribute("role")).toBe("status");
  await type(input, "next");
  expect(host.querySelector(".space-tag-status")).toBeNull();
});

it("shows checked state only from acknowledged props, without refocusing on prop updates", async () => {
  const pending = deferred();
  const { host, render } = await mount({ onChange: () => pending.promise });
  const checkbox = host.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
  await act(async () => { checkbox.focus(); checkbox.click(); });
  expect(checkbox.checked).toBe(false);
  await act(async () => pending.resolve(true));
  expect(checkbox.checked).toBe(false);
  await render({ tags: ["My check"] });
  expect(checkbox.checked).toBe(true);
  expect(document.activeElement).toBe(checkbox);
});

it("retains failed create text for Enter retry and prevents duplicate immediate submissions", async () => {
  const pending = deferred();
  const change = vi.fn().mockReturnValueOnce(pending.promise).mockResolvedValue(true);
  const { host, input } = await mount({ onChange: change });
  await type(input, "Blocked on QA");
  await act(async () => {
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    host.querySelector<HTMLButtonElement>(".space-tag-create")!.click();
  });
  expect(change).toHaveBeenCalledTimes(1);
  const create = host.querySelector<HTMLButtonElement>(".space-tag-create")!;
  expect(create.getAttribute("aria-disabled")).toBe("true");
  expect(create.disabled).toBe(false);
  expect(input.disabled).toBe(false);
  await act(async () => pending.resolve(false));
  expect(input.value).toBe("Blocked on QA");
  expect(host.querySelector(".space-tag-status")?.getAttribute("role")).toBe("status");
  await key(input, "Enter");
  expect(change.mock.calls).toEqual([["Blocked on QA", true], ["Blocked on QA", true]]);
  expect(input.value).toBe("");
  expect(host.querySelector(".space-tag-status")).toBeNull();
});

it("keeps input editable while busy or blocked, ignores writes and preserves newly typed text", async () => {
  const pending = deferred();
  const change = vi.fn(() => pending.promise);
  const { host, input, render } = await mount({ busy: true, catalog: ["First aid"], onChange: change });
  await type(input, "First");
  await key(input, "Enter");
  await act(async () => host.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
  expect(change).not.toHaveBeenCalled();
  await render({ busy: false, blocked: "Herdr is not live" });
  await act(async () => host.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
  await act(async () => host.querySelector<HTMLButtonElement>(".space-tag-create")!.click());
  expect(change).not.toHaveBeenCalled();
  await render({ blocked: null });
  await key(input, "Enter");
  await type(input, "Second");
  await act(async () => pending.resolve(true));
  expect(input.value).toBe("Second");
  expect(change.mock.calls).toEqual([["First", true]]);
});

it("moves between input/options and wraps Tab including the Create row without trapping outside clicks", async () => {
  const { host, input } = await mount();
  await type(input, "check");
  const checkbox = host.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
  const create = host.querySelector<HTMLButtonElement>(".space-tag-create")!;
  await key(input, "ArrowDown");
  expect(document.activeElement).toBe(checkbox);
  await key(checkbox, "ArrowUp");
  expect(document.activeElement).toBe(input);
  await key(input, "Tab", { shiftKey: true });
  expect(document.activeElement).toBe(create);
  await key(create, "Tab");
  expect(document.activeElement).toBe(input);
  await key(input, "Tab");
  expect(document.activeElement).toBe(checkbox);
  await key(checkbox, "Tab");
  expect(document.activeElement).toBe(create);
});

it("consumes Escape but passes the original outside target without consuming pointerdown", async () => {
  const { host, trigger, input, props } = await mount();
  const bubble = vi.fn();
  window.addEventListener("keydown", bubble);
  cleanups.push(async () => window.removeEventListener("keydown", bubble));
  await type(input, "discard me");
  const escape = await key(input, "Escape");
  expect(escape.defaultPrevented).toBe(true);
  expect(bubble).not.toHaveBeenCalled();
  expect(props.onClose).toHaveBeenCalledWith("escape");
  const pointer = (target: Element) => {
    const event = new Event("pointerdown", { bubbles: true, cancelable: true });
    target.dispatchEvent(event);
    return event;
  };
  await act(async () => { pointer(input); pointer(trigger); });
  expect(props.onClose).toHaveBeenCalledTimes(1);
  const outside = document.createElement("button");
  document.body.append(outside);
  try {
    let event!: Event;
    await act(async () => { event = pointer(outside); });
    expect(event.defaultPrevented).toBe(false);
    expect(props.onClose).toHaveBeenLastCalledWith("outside", outside);
    await act(async () => { window.dispatchEvent(new Event("blur")); });
    expect(props.onClose).toHaveBeenCalledTimes(2);
  } finally { outside.remove(); }
});

it("renders an empty catalog as input-only and wraps Tab back to that input", async () => {
  const { host, input } = await mount({ catalog: [] });
  expect(host.querySelectorAll("button, input")).toHaveLength(1);
  await key(input, "Tab");
  expect(document.activeElement).toBe(input);
});

it("accepts 32 Unicode scalars, including supplementary characters, but not 33", async () => {
  const { host, input, props } = await mount({ catalog: [] });
  const tag = "😀".repeat(32);
  expect(input.maxLength).toBeGreaterThanOrEqual(tag.length);
  await type(input, tag);
  expect(host.querySelector(".space-tag-create")).not.toBeNull();
  await key(input, "Enter");
  expect(props.onChange).toHaveBeenCalledWith(tag, true);
  await type(input, `${tag}a`);
  await key(input, "Enter");
  expect(props.onChange).toHaveBeenCalledTimes(1);
});

it("treats rejected write promises as failures and returns focus to input after Create activation", async () => {
  const change = vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValue(true);
  const { host, input } = await mount({ catalog: [], onChange: change });
  await type(input, "Needs review");
  const create = host.querySelector<HTMLButtonElement>(".space-tag-create")!;
  await act(async () => { create.focus(); create.click(); });
  expect(input.value).toBe("Needs review");
  expect(document.activeElement).toBe(create);
  expect(host.querySelector(".space-tag-status")?.getAttribute("role")).toBe("status");
  await act(async () => create.click());
  expect(input.value).toBe("");
  expect(document.activeElement).toBe(input);
  expect(change).toHaveBeenCalledTimes(2);
});
