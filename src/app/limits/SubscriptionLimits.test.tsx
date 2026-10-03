// @vitest-environment jsdom
import { act, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import type { QuotaStatusResponse } from "../../protocol/generated/v1";
import { SubscriptionLimits } from "./SubscriptionLimits";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

function pointerEvent(type: "pointerover" | "pointerout", pointerType = "mouse", relatedTarget: EventTarget | null = null) {
  const event = new MouseEvent(type, { bubbles: true, relatedTarget });
  Object.defineProperty(event, "pointerType", { value: pointerType });
  return event;
}

function movePointer(from: HTMLElement | null, to: HTMLElement | null, pointerType = "mouse") {
  // React derives enter/leave from pointerout between managed nodes, just like a real move.
  from?.dispatchEvent(pointerEvent("pointerout", pointerType, to));
  to?.dispatchEvent(pointerEvent("pointerover", pointerType, from));
}

const snapshot: QuotaStatusResponse = {
  generated_at_ms: 1000, collecting: false,
  providers: [
    { provider: "codex", state: "not_signed_in", error: null, fetched_at_ms: null, stale: false, accounts: [] },
    { provider: "claude", state: "unsupported", error: "unsupported", fetched_at_ms: null, stale: false, accounts: [] },
    { provider: "copilot", state: "available", error: null, fetched_at_ms: 1000, stale: false, accounts: [{ fetched_at_ms: 1000, limits: [{
      id: "copilot:premium", window: "monthly", tier: "business", unit: "credits", used_fraction: 0.0005,
      used: 4, limit: 8000, remaining: 7996, unlimited: false, level: "ok", resets_at_ms: 900000,
    }] }] },
  ],
};

it("opens a readable credit popup, restores the pointer trigger on Escape, and leaves outside focus alone", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  function View() {
    const [open, setOpen] = useState(false);
    return <><SubscriptionLimits snapshot={snapshot} link="live" now={1000} open={open} onOpenChange={setOpen} suspended={false} /><button data-outside>Outside</button></>;
  }
  try {
    await act(async () => root.render(<View />));
    const trigger = host.querySelector<HTMLButtonElement>(".limits-trigger")!;
    expect(trigger.getAttribute("aria-label")).toContain("4 of 8,000 AI credits used · 0.05%");
    expect(trigger.getAttribute("aria-label")).not.toContain("Codex");
    const outside = host.querySelector<HTMLButtonElement>("[data-outside]")!;
    outside.focus();
    await act(async () => movePointer(outside, trigger));
    expect(host.querySelector(".limits-popup")).not.toBeNull();
    expect(document.activeElement).toBe(outside);
    await act(async () => trigger.click());
    const popup = host.querySelector<HTMLElement>(".limits-popup")!;
    expect(document.activeElement).toBe(popup);
    expect(popup.textContent).toContain("4 of 8,000 AI credits used · 0.05%");
    expect(popup.textContent).toContain("Monthly");
    expect(popup.textContent).toContain("Not signed in to OMP");
    await act(async () => movePointer(trigger, outside));
    expect(host.querySelector(".limits-popup")).toBe(popup);
    await act(async () => movePointer(outside, trigger));
    await act(async () => popup.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(host.querySelector(".limits-popup")).toBeNull();
    expect(document.activeElement).toBe(trigger);
    await act(async () => trigger.click());
    await act(async () => trigger.click());
    expect(host.querySelector(".limits-popup")).toBeNull();
    await act(async () => trigger.click());
    await act(async () => {
      outside.dispatchEvent(new Event("pointerdown", { bubbles: true }));
      outside.focus();
    });
    expect(host.querySelector(".limits-popup")).toBeNull();
    expect(document.activeElement).toBe(outside);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("restores the actual Commands opener, closes on focus leaving, and suspends without stealing focus", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  let suspend!: (value: boolean) => void;
  function View() {
    const [open, setOpen] = useState(false);
    const [suspended, setSuspended] = useState(false);
    suspend = setSuspended;
    const opener = useRef<HTMLElement | null>(null);
    return <><button data-command onClick={() => { opener.current = document.activeElement as HTMLElement; setOpen(true); }}>Subscription limits</button>
      <SubscriptionLimits snapshot={snapshot} link="offline" now={1000} open={open} onOpenChange={setOpen} suspended={suspended} commandOpener={opener} />
      <input data-terminal aria-label="Terminal" /></>;
  }
  try {
    await act(async () => root.render(<View />));
    const terminal = host.querySelector<HTMLInputElement>("[data-terminal]")!;
    const command = host.querySelector<HTMLButtonElement>("[data-command]")!;
    terminal.focus();
    const trigger = host.querySelector<HTMLButtonElement>(".limits-trigger")!;
    await act(async () => movePointer(terminal, trigger));
    expect(host.querySelector(".limits-popup")).not.toBeNull();
    expect(document.activeElement).toBe(terminal);
    await act(async () => command.click());
    const popup = host.querySelector<HTMLElement>(".limits-popup")!;
    expect(document.activeElement).toBe(popup);
    expect(popup.textContent).toContain("Offline — showing the last values");
    await act(async () => popup.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(document.activeElement).toBe(terminal);
    await act(async () => command.click());
    await act(async () => terminal.focus());
    expect(host.querySelector(".limits-popup")).toBeNull();
    expect(document.activeElement).toBe(terminal);
    await act(async () => command.click());
    await act(async () => { terminal.focus(); suspend(true); });
    expect(host.querySelector(".limits-popup")).toBeNull();
    expect(document.activeElement).toBe(terminal);
    await act(async () => suspend(false));
    expect(host.querySelector(".limits-popup")).toBeNull();
    await act(async () => movePointer(trigger, terminal));
    await act(async () => movePointer(terminal, trigger));
    expect(host.querySelector(".limits-popup")).not.toBeNull();
    await act(async () => suspend(true));
    expect(host.querySelector(".limits-popup")).toBeNull();
    await act(async () => {
      movePointer(trigger, terminal);
      movePointer(terminal, trigger);
    });
    expect(host.querySelector(".limits-popup")).toBeNull();
    await act(async () => suspend(false));
    expect(host.querySelector(".limits-popup")).toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("previews only for a mouse, retains terminal focus, and dismisses without consuming terminal Escape", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  let openChanges = 0;
  try {
    await act(async () => root.render(<>
      <SubscriptionLimits snapshot={snapshot} link="live" now={1000} open={false} onOpenChange={() => { openChanges++; }} suspended={false} />
      <input data-terminal aria-label="Terminal" />
    </>));
    const trigger = host.querySelector<HTMLButtonElement>(".limits-trigger")!;
    const terminal = host.querySelector<HTMLInputElement>("[data-terminal]")!;
    terminal.focus();
    for (const pointerType of ["touch", "pen"]) {
      await act(async () => movePointer(terminal, trigger, pointerType));
      expect(host.querySelector(".limits-popup")).toBeNull();
      await act(async () => movePointer(trigger, terminal, pointerType));
    }
    await act(async () => movePointer(terminal, trigger));
    const popup = host.querySelector<HTMLElement>(".limits-popup")!;
    expect(popup).not.toBeNull();
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    expect(document.activeElement).toBe(terminal);
    await act(async () => movePointer(trigger, popup));
    expect(host.querySelector(".limits-popup")).toBe(popup);
    await act(async () => movePointer(popup, terminal));
    expect(host.querySelector(".limits-popup")).toBeNull();
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    await act(async () => movePointer(terminal, trigger));
    let terminalReceivedEscape = false;
    terminal.addEventListener("keydown", event => { if (event.key === "Escape") terminalReceivedEscape = true; });
    const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    await act(async () => terminal.dispatchEvent(escape));
    expect(host.querySelector(".limits-popup")).toBeNull();
    expect(document.activeElement).toBe(terminal);
    expect(terminalReceivedEscape).toBe(true);
    expect(escape.defaultPrevented).toBe(false);
    await act(async () => {
      movePointer(trigger, terminal);
      movePointer(terminal, trigger);
    });
    expect(host.querySelector(".limits-popup")).not.toBeNull();
    await act(async () => terminal.dispatchEvent(new Event("pointerdown", { bubbles: true })));
    expect(host.querySelector(".limits-popup")).toBeNull();
    expect(document.activeElement).toBe(terminal);
    expect(openChanges).toBe(0);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it.each([
  { usedFraction: 0.8, remaining: null, unlimited: false, tone: "ok", width: 80 },
  { usedFraction: 0.8000001, remaining: null, unlimited: false, tone: "warning", width: 80.00001 },
  { usedFraction: 0.95, remaining: null, unlimited: false, tone: "warning", width: 95 },
  { usedFraction: 0.9500001, remaining: null, unlimited: false, tone: "alert", width: 95.00001 },
  { usedFraction: null, remaining: 1520, unlimited: false, tone: "warning", width: 81 },
  { usedFraction: null, remaining: null, unlimited: false, tone: null, width: null },
  { usedFraction: 1, remaining: 0, unlimited: true, tone: null, width: null },
])("colors and fills actual used quota independently of backend level: $usedFraction used, $remaining remaining, unlimited=$unlimited", async ({ usedFraction, remaining, unlimited, tone, width }) => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const base = snapshot.providers[2];
  const value: QuotaStatusResponse = { ...snapshot, providers: [{
    ...base, accounts: [{ ...base.accounts[0], limits: [{
      ...base.accounts[0].limits[0], used_fraction: usedFraction, remaining, used: null, unlimited, level: "exhausted",
    }] }],
  }] };
  try {
    await act(async () => root.render(<SubscriptionLimits snapshot={value} link="live" now={1000} open={true} onOpenChange={() => {}} suspended={false} />));
    const meter = host.querySelector(".limits-row-meter .limits-meter");
    if (tone === null) {
      expect(meter).toBeNull();
    } else {
      expect(meter?.classList.contains(`limits-tone-${tone}`)).toBe(true);
      const fill = meter?.firstElementChild as HTMLElement;
      expect(parseFloat(fill.style.getPropertyValue("--used"))).toBeCloseTo(width!, 8);
      const segment = host.querySelector(".limits-chips .limits-segment");
      expect(segment?.classList.contains(`limits-tone-${tone}`)).toBe(true);
    }
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("shows all grouped provider windows in the strip and narrow summary while retaining every account in the popup", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const base = snapshot.providers[2].accounts[0].limits[0];
  const value: QuotaStatusResponse = { ...snapshot, providers: [
    { provider: "claude", state: "available", error: null, fetched_at_ms: 1000, stale: false, accounts: [
      { fetched_at_ms: 1000, limits: [
        { ...base, id: "weekly", window: "7d", tier: null, unit: "percent", used_fraction: 0.58 },
        { ...base, id: "short-low", window: "5h", tier: null, unit: "percent", used_fraction: 0.2 },
      ] },
      { fetched_at_ms: 1000, limits: [
        { ...base, id: "short-high", window: "5h", tier: null, unit: "percent", used_fraction: 0.39 },
      ] },
    ] },
    snapshot.providers[2],
  ] };
  try {
    await act(async () => root.render(<SubscriptionLimits snapshot={value} link="live" now={1000} open={true} onOpenChange={() => {}} suspended={false} />));
    const chips = host.querySelectorAll(".limits-chips .limits-chip");
    const claude = [...chips].find(chip => chip.querySelector(".limits-name")?.textContent === "Claude")!;
    expect([...claude.querySelectorAll(".limits-window")].map(label => label.textContent)).toEqual(["5h", "7d"]);
    expect([...claude.querySelectorAll(".limits-value")].map(label => label.textContent)).toEqual(["39%", "58%"]);
    const copilot = [...chips].find(chip => chip.querySelector(".limits-name")?.textContent === "Copilot")!;
    expect(copilot.querySelector(".limits-value")?.textContent).toBe("0.05%");
    const fill = copilot.querySelector<HTMLElement>(".limits-meter > span")!;
    expect(parseFloat(fill.style.getPropertyValue("--used"))).toBeCloseTo(0.05, 8);
    expect(host.querySelector(".limits-summary .limits-name")?.textContent).toBe("Claude");
    expect([...host.querySelectorAll(".limits-summary .limits-value")].map(value => value.textContent)).toEqual(["39%", "58%"]);
    const popup = host.querySelector(".limits-provider[aria-label='Claude']")!;
    expect([...popup.querySelectorAll(".limits-account")].map(account =>
      [...account.querySelectorAll(".limits-row-value")].map(value => value.textContent),
    )).toEqual([["20%", "58%"], ["39%"]]);
    const trigger = host.querySelector(".limits-trigger")!;
    expect(trigger.getAttribute("aria-label")).toContain("Claude: 5h 39% used, 7d 58% used");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
