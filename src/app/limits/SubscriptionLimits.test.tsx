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
      id: "copilot:monthly:0", window: "monthly", tier: null, unit: "credits", used_fraction: 0,
      used: 0, limit: 1500, remaining: 1500, unlimited: false, level: "ok", resets_at_ms: 900000,
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
    expect(trigger.getAttribute("aria-label")).toContain("1,500 of 1,500 AI credits left");
    expect(trigger.getAttribute("aria-label")).not.toContain("Codex");
    const outside = host.querySelector<HTMLButtonElement>("[data-outside]")!;
    outside.focus();
    await act(async () => movePointer(outside, trigger));
    expect(host.querySelector(".limits-popup")).not.toBeNull();
    expect(document.activeElement).toBe(outside);
    await act(async () => trigger.click());
    const popup = host.querySelector<HTMLElement>(".limits-popup")!;
    expect(document.activeElement).toBe(popup);
    expect(popup.textContent).toContain("1,500 of 1,500 AI credits left");
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
  { usedFraction: 0.8, remaining: null, unlimited: false, tone: "ok" },
  { usedFraction: 0.8000001, remaining: null, unlimited: false, tone: "warning" },
  { usedFraction: 0.95, remaining: null, unlimited: false, tone: "warning" },
  { usedFraction: 0.9500001, remaining: null, unlimited: false, tone: "alert" },
  { usedFraction: null, remaining: 285, unlimited: false, tone: "warning" },
  { usedFraction: null, remaining: null, unlimited: false, tone: null },
  { usedFraction: 1, remaining: 0, unlimited: true, tone: null },
])("colors actual usage independently of backend level: $usedFraction used, $remaining credits left, unlimited=$unlimited", async ({ usedFraction, remaining, unlimited, tone }) => {
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
    }
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
