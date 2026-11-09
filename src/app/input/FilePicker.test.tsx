// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { FilePicker } from "./FilePicker";

it("keeps picker selection and keyboard recovery separate", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const choose = vi.fn(); const refresh = vi.fn(); const retry = vi.fn();
  try {
    await act(async () => root.render(<FilePicker candidates={[{ id: "a", path: "a.md" }]} watchUnavailable
      onChoose={choose} onDismiss={vi.fn()} onRefresh={refresh} onRetryWatch={retry} />));
    const input = host.querySelector<HTMLInputElement>('input[type="search"]')!;
    expect(document.activeElement).toBe(input);
    const button = host.querySelector<HTMLButtonElement>('[data-picker-recovery="refresh"]')!;
    button.focus();
    await act(async () => { button.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); button.click(); });
    expect(refresh).toHaveBeenCalledOnce();
    expect(choose).not.toHaveBeenCalled();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-picker-recovery="retry"]')!.click());
    expect(retry).toHaveBeenCalledOnce();
    expect(host.textContent).toContain("automatic updates unavailable");
  } finally { await act(async () => root.unmount()); host.remove(); }
});
