// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import { ContextResources } from "./ContextResources";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

it("offers Library Add instead of a repository snapshot and keeps the resource dialog keyboard contract", async () => {
  const host = document.createElement("div");
  const opener = document.createElement("button");
  document.body.append(opener, host);
  opener.focus();
  const root = createRoot(host);
  const onAdd = vi.fn();
  const onClose = vi.fn();
  const target = { session_id: "session", space_id: "space" };
  try {
    await act(async () => root.render(<ContextResources client={{ projectConfiguration: vi.fn(async () => ({ providers: [] })) } as unknown as CockpitClient}
      root={{ root_id: "companion", kind: "companion", label: "Context", path: "/companion", repository_id: "repo", checkout_path: "/repo", companion_id: "c1" }}
      space={{ target, label: "Review", live: true }}
      spaceListing={{ status: "ready", error: null, received: 1, reload: vi.fn(), listing: { target, companion: { status: "available", companion_root_id: "companion", companion_label: "Context" }, attempts: [], rows: [], behind: 0, diagnostics: [] } }}
      onAdd={onAdd} onClose={onClose} />));
    const dialog = host.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(document.activeElement).toBe(dialog.querySelector('button[aria-label="Close Context resources"]'));
    expect(dialog.querySelector("select")).toBeNull();
    expect(dialog.textContent).not.toMatch(/snapshot/i);
    const add = [...dialog.querySelectorAll("button")].find((button) => button.textContent === "Add…")!;
    await act(async () => add.click());
    expect(onAdd).toHaveBeenCalledOnce();
    const buttons = [...dialog.querySelectorAll<HTMLButtonElement>("button:not([disabled])")];
    await act(async () => {
      buttons[buttons.length - 1]!.focus();
      buttons[buttons.length - 1]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    });
    expect(document.activeElement).toBe(buttons[0]);
    await act(async () => dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(onClose).toHaveBeenCalledOnce();
    await act(async () => root.unmount());
    expect(document.activeElement).toBe(opener);
  } finally {
    host.remove();
    opener.remove();
  }
});
