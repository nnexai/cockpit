// @vitest-environment jsdom
import { act, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import type { QuotaStatusResponse } from "../../protocol/generated/v1";
import { SubscriptionLimits } from "./SubscriptionLimits";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

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
    await act(async () => trigger.click());
    const popup = host.querySelector<HTMLElement>(".limits-popup")!;
    expect(document.activeElement).toBe(popup);
    expect(popup.textContent).toContain("1,500 of 1,500 AI credits left");
    expect(popup.textContent).toContain("Monthly");
    expect(popup.textContent).toContain("Not signed in to OMP");
    await act(async () => popup.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(host.querySelector(".limits-popup")).toBeNull();
    expect(document.activeElement).toBe(trigger);
    await act(async () => trigger.click());
    const outside = host.querySelector<HTMLButtonElement>("[data-outside]")!;
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
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
