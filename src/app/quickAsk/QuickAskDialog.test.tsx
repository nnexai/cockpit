// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { QuickAskHarness } from "../../protocol/generated/v1";
import { QuickAskDialog } from "./QuickAskDialog";
import { useQuickAsk } from "./useQuickAsk";

const location = { space_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", tab_label: "why" };
let root: Root | null = null;
let host: HTMLDivElement;

afterEach(() => {
  if (root) act(() => root!.unmount());
  root = null;
  host?.remove();
});

async function setup(installed: QuickAskHarness[], quickAsk: CockpitClient["quickAsk"]) {
  const client = {
    quickAskAvailability: async () => ({ agents: (["omp", "pi", "claude", "copilot", "codex", "opencode"] as const).map(harness => ({ harness, executable: harness, found: installed.includes(harness) })) }),
    quickAsk: vi.fn(quickAsk),
  } as unknown as CockpitClient;
  const onOpenTab = vi.fn();
  let setOpen!: (open: boolean) => void;
  function Composer() {
    const [open, setOpenState] = useState(true);
    setOpen = setOpenState;
    const state = useQuickAsk({ client, open, setOpen: setOpenState, sessionId: "session", serverInstance: "instance", spaceId: "space", initialCwd: "/work", onOpenTab });
    return open ? <QuickAskDialog state={state} onClose={() => setOpenState(false)} /> : null;
  }
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => root!.render(<Composer />));
  const textarea = () => host.querySelector("textarea")!;
  const type = async (value: string) => act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea(), value);
    textarea().dispatchEvent(new Event("input", { bubbles: true }));
  });
  const key = async (init: KeyboardEventInit) => act(async () => { textarea().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init })); });
  return { client, onOpenTab, textarea, type, key, reopen: () => act(async () => setOpen(true)) };
}

it("asks OMP by default on Enter, then closes and opens the new tab", async () => {
  const { client, onOpenTab, type, key, reopen, textarea } = await setup(["omp", "claude"], async () => location);
  await type("why\tdoes\r\nthis fail");
  await key({ key: "Enter", shiftKey: true });
  expect(client.quickAsk).not.toHaveBeenCalled();
  await key({ key: "Enter" });
  expect(client.quickAsk).toHaveBeenCalledWith("session", { server_instance: "instance", space_id: "space", cwd: "/work", harness: "omp", prompt: "why    does\nthis fail" });
  expect(host.querySelector(".quick-ask")).toBeNull();
  expect(onOpenTab).toHaveBeenCalledWith(location);
  await reopen();
  expect(textarea().value).toBe("");
});

it("offers only installed agents and keeps the draft when the launch fails", async () => {
  const { client, onOpenTab, type, key, textarea } = await setup(["claude", "codex"], async () => { throw new Error("Herdr did not confirm the new tab"); });
  expect([...host.querySelectorAll('[role="radio"]')].map(radio => radio.textContent)).toEqual(["Claude", "Codex"]);
  expect(host.querySelector('[role="radio"][aria-checked="true"]')?.textContent).toBe("Claude");
  await act(async () => (host.querySelector('[data-agent="codex"]') as HTMLButtonElement).click());
  await type("explain");
  await key({ key: "Enter" });
  expect(client.quickAsk).toHaveBeenCalledWith("session", expect.objectContaining({ harness: "codex", prompt: "explain" }));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe("Herdr did not confirm the new tab");
  expect(textarea().value).toBe("explain");
  expect(onOpenTab).not.toHaveBeenCalled();
});
