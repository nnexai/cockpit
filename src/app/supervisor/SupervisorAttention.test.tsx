// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AttentionQueue, SupervisorSummary, type AttentionQueueProps, type QueueRowView } from "./SupervisorAttention";

let host: HTMLDivElement;
let root: Root | null = null;
async function mount(node: React.ReactNode) {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => { root!.render(node); });
}
const rows: QueueRowView[] = [
  { id: "decide:root", tier: "decide", title: "Supervisor needs a choice", since: null, body: <textarea aria-label="Answer" defaultValue="Retained draft" />, actions: [], showIn: null },
  { id: "recover:worker", tier: "recover", title: "Search worker terminal is gone", since: null, body: <p>Check the saved worker.</p>, actions: [], showIn: null },
  { id: "notice:task", tier: "notice", title: "Search task changed", since: null, actions: [], showIn: null },
];
function ControlledQueue({ initial = null, ...props }: Partial<AttentionQueueProps> & { initial?: string | null }) {
  const [expanded, setExpanded] = useState(initial);
  return <AttentionQueue rows={rows} expandedId={expanded} onExpand={setExpanded} capPx={160} variant="inline" focusTier={null} onFocusedTier={() => undefined} {...props} />;
}
function summaries(): HTMLButtonElement[] { return [...host.querySelectorAll<HTMLButtonElement>("[data-row-id]")]; }
function key(target: HTMLElement, value: string) { act(() => { target.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true })); }); }
function click(target: HTMLElement) { act(() => target.click()); }
afterEach(async () => { if (root) await act(async () => root!.unmount()); root = null; host?.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("Supervisor attention interactions", () => {
  it("toggles one controlled disclosure using Enter and Space, then lets collapsed Escape reach the parent", async () => {
    const outerEscape = vi.fn();
    await mount(<div onKeyDown={event => { if (event.key === "Escape") outerEscape(); }}><ControlledQueue /></div>);
    let buttons = summaries();
    act(() => buttons[0].focus());
    key(buttons[0], "Enter");
    expect(buttons[0].getAttribute("aria-expanded")).toBe("true");
    expect(host.querySelector("textarea")?.value).toBe("Retained draft");
    key(buttons[1], " ");
    buttons = summaries();
    expect(buttons.map(button => button.getAttribute("aria-expanded"))).toEqual(["false", "true", "false"]);
    expect(host.querySelector("textarea")).toBeNull();
    act(() => buttons[1].focus());
    key(buttons[1], "Escape");
    expect(outerEscape).not.toHaveBeenCalled();
    expect(buttons[1].getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(buttons[1]);
    key(buttons[1], "Escape");
    expect(outerEscape).toHaveBeenCalledTimes(1);
  });

  it("leaves overlay Close and non-row Escape to the parent while an expanded row consumes it", async () => {
    const outerEscape = vi.fn();
    await mount(<div onKeyDown={event => { if (event.key === "Escape") outerEscape(event.defaultPrevented); }}><ControlledQueue variant="overlay" initial={rows[0].id} /></div>);
    const close = host.querySelector<HTMLButtonElement>(".supervisor-queue-header button")!;
    act(() => close.focus());
    key(close, "Escape");
    expect(outerEscape).toHaveBeenLastCalledWith(false);
    expect(document.activeElement).toBe(close);
    expect(summaries()[0].getAttribute("aria-expanded")).toBe("true");
    key(host.querySelector<HTMLElement>(".supervisor-queue-scroll")!, "Escape");
    expect(outerEscape).toHaveBeenCalledTimes(2);
    expect(outerEscape).toHaveBeenLastCalledWith(false);
    act(() => summaries()[0].focus());
    key(summaries()[0], "Escape");
    expect(outerEscape).toHaveBeenCalledTimes(2);
    expect(summaries()[0].getAttribute("aria-expanded")).toBe("false");
  });

  it("roves only summary buttons and leaves editing/navigation inside the expanded body alone", async () => {
    await mount(<ControlledQueue initial={rows[0].id} />);
    const buttons = summaries();
    expect(buttons.map(button => button.tabIndex)).toEqual([0, -1, -1]);
    act(() => buttons[0].focus());
    key(buttons[0], "ArrowDown");
    expect(document.activeElement).toBe(buttons[1]);
    expect(buttons.map(button => button.tabIndex)).toEqual([-1, 0, -1]);
    key(buttons[1], "End");
    expect(document.activeElement).toBe(buttons[2]);
    key(buttons[2], "ArrowUp");
    expect(document.activeElement).toBe(buttons[1]);
    key(buttons[1], "Home");
    expect(document.activeElement).toBe(buttons[0]);
    const field = host.querySelector<HTMLTextAreaElement>("textarea")!;
    act(() => field.focus());
    key(field, "ArrowDown");
    expect(document.activeElement).toBe(field);
    expect(field.value).toBe("Retained draft");
    const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    act(() => field.dispatchEvent(escape));
    expect(escape.defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(field);
    expect(host.querySelector("textarea")).toBe(field);
    expect(field.value).toBe("Retained draft");
    expect(buttons[0].getAttribute("aria-expanded")).toBe("true");
  });

  it("does not steal field focus when attention changes, but honors an explicit summary counter focus request", async () => {
    const consumed = vi.fn();
    const props: AttentionQueueProps = { rows, expandedId: rows[0].id, onExpand: vi.fn(), capPx: 160, variant: "inline", focusTier: null, onFocusedTier: consumed };
    await mount(<AttentionQueue {...props} />);
    const field = host.querySelector<HTMLTextAreaElement>("textarea")!;
    act(() => field.focus());
    await act(async () => root!.render(<AttentionQueue {...props} rows={[...rows]} />));
    expect(document.activeElement).toBe(field);
    expect(field.value).toBe("Retained draft");
    await act(async () => root!.render(<AttentionQueue {...props} focusTier="recover" />));
    expect(document.activeElement).toBe(summaries()[1]);
    expect(consumed).toHaveBeenCalledTimes(1);
    expect(summaries()[0].getAttribute("aria-expanded")).toBe("true");
  });

  it("keeps parent-disabled authority actions inert while enabled actions receive their actual invoker", async () => {
    const blocked = vi.fn(); const show = vi.fn();
    const activeRows: QueueRowView[] = [{ ...rows[1], actions: [
      { key: "restart", label: "Restart agent", disabled: true, reason: "Cannot restart while offline", onActivate: blocked },
      { key: "check", label: "Check status", primary: true, onActivate: show },
    ], showIn: { key: "show", label: "Show in Tasks", onActivate: show } }];
    await mount(<ControlledQueue rows={activeRows} initial={rows[1].id} />);
    const actionButtons = [...host.querySelectorAll<HTMLButtonElement>(".supervisor-queue-actions button")];
    const restart = actionButtons.find(button => button.textContent === "Restart agent")!;
    expect(restart.disabled).toBe(true);
    expect(document.getElementById(restart.getAttribute("aria-describedby")!)?.textContent).toBe("Cannot restart while offline");
    click(restart);
    expect(blocked).not.toHaveBeenCalled();
    const check = actionButtons[0];
    click(check);
    expect(show).toHaveBeenLastCalledWith(check);
    const showIn = actionButtons[actionButtons.length - 1];
    click(showIn);
    expect(show).toHaveBeenLastCalledWith(showIn);
  });

  it("keeps aria-disabled recovery focusable but blocks activation until the parent enables it", async () => {
    const restart = vi.fn();
    const recoveryRow: QueueRowView = { ...rows[1], actions: [{ key: "restart", label: "Restart agent", ariaDisabled: true, disabled: true, reason: "Check the current agent first", onActivate: restart }] };
    await mount(<ControlledQueue rows={[recoveryRow]} initial={recoveryRow.id} />);
    const button = host.querySelector<HTMLButtonElement>(".supervisor-queue-actions button")!;
    expect(button.disabled).toBe(false);
    expect(button.getAttribute("aria-disabled")).toBe("true");
    act(() => button.focus());
    expect(document.activeElement).toBe(button);
    click(button);
    expect(restart).not.toHaveBeenCalled();
    await act(async () => root!.render(<ControlledQueue rows={[{ ...recoveryRow, actions: [{ ...recoveryRow.actions[0], disabled: false }] }]} initial={recoveryRow.id} />));
    expect(button.disabled).toBe(false);
    expect(button.getAttribute("aria-disabled")).toBe("false");
    click(button);
    expect(restart).toHaveBeenCalledExactlyOnceWith(button);
  });

  it("counts partly hidden rows as more and updates the full-row indication on scroll and resize", async () => {
    await mount(<ControlledQueue capPx={100} />);
    const scroller = host.querySelector<HTMLElement>(".supervisor-queue-scroll")!;
    let height = 100;
    Object.defineProperty(scroller, "clientHeight", { get: () => height, configurable: true });
    vi.spyOn(scroller, "getBoundingClientRect").mockImplementation(() => ({ top: 10, bottom: 10 + height } as DOMRect));
    const elements = [...host.querySelectorAll<HTMLElement>("[data-queue-row]")];
    elements.forEach((element, index) => vi.spyOn(element, "getBoundingClientRect").mockImplementation(() => ({ top: 10 + index * 60 - scroller.scrollTop, bottom: 10 + (index + 1) * 60 - scroller.scrollTop } as DOMRect)));
    act(() => window.dispatchEvent(new Event("resize")));
    expect(host.querySelector(".supervisor-queue-more")?.textContent).toBe("2 more");
    expect(scroller.style.maxHeight).toBe("60px");
    act(() => { scroller.scrollTop = 80; scroller.dispatchEvent(new Event("scroll")); });
    expect(host.querySelector(".supervisor-queue-more")).toBeNull();
    scroller.scrollTop = 0; height = 120;
    await act(async () => root!.render(<ControlledQueue capPx={120} />));
    expect(host.querySelector(".supervisor-queue-more")?.textContent).toBe("1 more");
    expect(scroller.style.maxHeight).toBe("120px");
    await act(async () => root!.render(<ControlledQueue variant="overlay" />));
    expect(host.querySelector(".supervisor-queue-more")).toBeNull();
    expect(host.querySelector<HTMLElement>(".supervisor-queue-scroll")?.style.maxHeight).toBe("");
  });

  it("keeps an oversized expanded row scrollable with all its fields and actions mounted", async () => {
    const expandedRow: QueueRowView = { ...rows[0], actions: [{ key: "show", label: "Show in Tasks", onActivate: vi.fn() }] };
    await mount(<ControlledQueue rows={[expandedRow, rows[1]]} initial={expandedRow.id} capPx={100} />);
    const scroller = host.querySelector<HTMLElement>(".supervisor-queue-scroll")!;
    Object.defineProperty(scroller, "clientHeight", { value: 100, configurable: true });
    vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue({ top: 10 } as DOMRect);
    const elements = [...host.querySelectorAll<HTMLElement>("[data-queue-row]")];
    vi.spyOn(elements[0], "getBoundingClientRect").mockReturnValue({ top: 10, bottom: 240 } as DOMRect);
    vi.spyOn(elements[1], "getBoundingClientRect").mockReturnValue({ top: 240, bottom: 300 } as DOMRect);
    act(() => window.dispatchEvent(new Event("resize")));
    expect(scroller.style.maxHeight).toBe("100px");
    expect(scroller.style.overflowY).toBe("auto");
    expect(host.querySelector("textarea")?.value).toBe("Retained draft");
    expect(host.querySelector(".supervisor-queue-actions button")?.textContent).toBe("Show in Tasks");
    expect(summaries()[0].getAttribute("aria-expanded")).toBe("true");
  });

  it("passes each summary counter invoker for focus restoration and replaces the shortcut when attention appears", async () => {
    const counter = vi.fn(); const shortcut = vi.fn();
    const props = { rootLabel: "Supervisor", stateLabel: "Managing tasks", stateGlyph: "working" as const, observedLine: "2 agents observed", banner: null, queueMode: "overlay" as const, onCounter: counter, shortcut: { key: "terminal", label: "Open terminal", onActivate: shortcut } };
    await mount(<SupervisorSummary {...props} counts={{ decide: 0, recover: 0, notice: 0 }} />);
    const terminal = host.querySelector<HTMLButtonElement>(".supervisor-summary-shortcut button")!;
    click(terminal);
    expect(shortcut).toHaveBeenLastCalledWith(terminal);
    await act(async () => root!.render(<SupervisorSummary {...props} counts={{ decide: 1, recover: 2, notice: 0 }} />));
    expect(host.querySelector(".supervisor-summary-shortcut")).toBeNull();
    const counters = [...host.querySelectorAll<HTMLButtonElement>(".supervisor-summary-counter")];
    click(counters[1]);
    expect(counter).toHaveBeenLastCalledWith("recover", counters[1]);
    expect(counters).toHaveLength(2);
    expect(host.querySelectorAll("[aria-live]")).toHaveLength(1);
    expect(host.querySelector("[aria-live]")?.contains(counters[0])).toBe(true);
  });
});
