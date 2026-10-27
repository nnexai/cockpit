// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { QuickAskPhase, QuickAskStatus } from "../../protocol/generated/v1";
import { QuickAskDialog } from "./QuickAskDialog";
import { useQuickAsk, type QuickAskState } from "./useQuickAsk";

const location = { space_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", tab_label: "Agent", agent_name: "omp" };
const question = "-Investigate the actual failure";
let root: Root | null = null;
let host: HTMLDivElement;
let state: QuickAskState;

afterEach(() => {
  if (root) act(() => root!.unmount());
  root = null;
  host?.remove();
  vi.useRealTimers();
});

async function setup() {
  vi.useFakeTimers();
  let current: QuickAskStatus;
  const status = (phase: QuickAskPhase, updated: number): QuickAskStatus => ({
    ...current, phase, updated_at_ms: updated,
    delivery: phase === "submitted" ? "submitted" : "not_sent",
  });
  const action = vi.fn(async (_session: string, request: { action: string }) => {
    if (request.action === "send") current = status("submitted", 4);
    return current;
  });
  const client = {
    quickAskAvailability: vi.fn(async () => ({ agents: [{ harness: "omp", executable: "omp", found: true }] })),
    quickAskStart: vi.fn(async (_session: string, request: { request_id: string }) => {
      current = { request_id: request.request_id, session_id: "session", server_instance: "server", harness: "omp", phase: "waiting_for_agent", delivery: "not_sent", cancellable: true, location, error: null, updated_at_ms: 1 };
      return current;
    }),
    quickAskStatus: vi.fn(async () => current),
    quickAskAction: action,
  } as unknown as CockpitClient;
  const onOpenTab = vi.fn();
  function Composer() {
    const [open, setOpen] = useState(true);
    state = useQuickAsk({ client, open, setOpen, sessionId: "session", serverInstance: "server", spaceId: "space", initialCwd: "/work", live: true, spaceIds: ["space"], mutationBusy: false, onOpenTab });
    return <><button onClick={() => setOpen(true)}>Reopen</button>{open && <QuickAskDialog {...state} cwdSource="Space" onClose={() => setOpen(false)} />}</>;
  }
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => root!.render(<Composer />));
  const textarea = host.querySelector("textarea")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, question);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  // Keep the receipt visible rather than automatically switching to the tab.
  const switchCheckbox = host.querySelector<HTMLInputElement>('.quick-ask-more input[type="checkbox"]')!;
  await act(async () => switchCheckbox.click());
  await click("Ask OMP");
  return {
    action, onOpenTab,
    setPhase(phase: QuickAskPhase, updated: number) { current = status(phase, updated); },
    async poll() { await act(async () => { await vi.advanceTimersByTimeAsync(400); }); },
  };
}

function button(label: string): HTMLButtonElement {
  const result = [...host.querySelectorAll("button")].find(element => element.textContent?.trim() === label);
  if (!result) throw new Error(`Missing button: ${label}`);
  return result;
}
async function click(label: string) { await act(async () => button(label).click()); }
function confirmation(): HTMLInputElement {
  const checkbox = [...host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].find(input => !input.closest(".quick-ask-more"));
  if (!checkbox) throw new Error("Missing prompt confirmation");
  return checkbox;
}

it("requires explicit prompt confirmation after navigating while waiting and again after readiness changes", async () => {
  const fixture = await setup();
  await click("Open tab");
  expect(fixture.onOpenTab).toHaveBeenCalledWith(location);
  fixture.setPhase("ready_to_send", 2);
  await fixture.poll();
  await click("Reopen");
  expect(button("Send question").disabled).toBe(true);
  await act(async () => state.send());
  expect(fixture.action).not.toHaveBeenCalled();
  await act(async () => confirmation().click());
  expect(button("Send question").disabled).toBe(false);
  await fixture.poll();
  expect(button("Send question").disabled).toBe(false);
  await click("Open tab");
  await click("Reopen");
  expect(button("Send question").disabled).toBe(true);
  await act(async () => confirmation().click());
  // A changed status invalidates confirmation even if it remains in the ready phase.
  fixture.setPhase("ready_to_send", 3);
  await fixture.poll();
  expect(button("Send question").disabled).toBe(true);
  await act(async () => state.send());
  expect(fixture.action).not.toHaveBeenCalled();
  await act(async () => confirmation().click());
  await click("Send question");
  expect(fixture.action).toHaveBeenCalledTimes(1);
  expect(fixture.action.mock.calls[0][1]).toMatchObject({ action: "send" });
});

it("retains the submitted question in the mounted composer until New question", async () => {
  const fixture = await setup();
  fixture.setPhase("ready_to_send", 2);
  await fixture.poll();
  await act(async () => confirmation().click());
  await click("Send question");
  expect(fixture.action).toHaveBeenCalledTimes(1);
  expect(host.querySelector(".error-slot-dialog")).not.toBeNull();
  expect(button("Open tab").classList.contains("setup-primary")).toBe(true);
  expect(button("Open tab").disabled).toBe(false);
  expect(host.textContent).toContain("Kept here. New question clears it.");
  expect([...host.querySelectorAll("button")].some(element => ["Sent", "Started"].includes(element.textContent?.trim() ?? ""))).toBe(false);
  expect(host.querySelector("textarea")!.value).toBe(question);
  expect(host.querySelector("textarea")!.readOnly).toBe(true);
  await click("New question");
  expect(host.querySelector("textarea")!.value).toBe("");
  expect(host.querySelector("textarea")!.readOnly).toBe(false);
});

it("shows delivery guidance and keeps long errors in the reserved status slot", async () => {
  await setup();
  const render = async (overrides: Partial<Parameters<typeof QuickAskDialog>[0]>) => {
    await act(async () => root!.render(<QuickAskDialog {...state} cwdSource={'Checkout of Space "Work"'} onClose={() => {}} {...overrides} />));
  };
  const longError = "Failure detail ".repeat(30);
  await render({ status: null, directLaunch: true, manualPaste: false, error: longError });
  expect(host.textContent).toContain("Sent at launch in the command line.");
  expect(host.querySelector(".quick-ask-path")?.textContent).toContain("Space checkout");
  expect(host.querySelector(".error-slot-dialog .error-slot-message > span")?.getAttribute("title")).toBe(longError);
  const textarea = host.querySelector("textarea")!;
  expect(textarea.getAttribute("aria-describedby")).toContain(host.querySelector(".quick-ask-delivery")!.id);
  await render({ status: null, directLaunch: false, manualPaste: true, error: null });
  expect(host.textContent).toContain("You paste it into the agent yourself.");
  await render({ status: null, directLaunch: false, manualPaste: false, error: null });
  expect(host.textContent).toContain("You confirm the agent’s prompt before it is sent.");
});

it("keeps unavailable agents visible and omits an impossible multiline send", async () => {
  await setup();
  const status = { ...state.status!, phase: "ready_to_send" as const };
  await act(async () => root!.render(<QuickAskDialog {...state} cwdSource="Terminal folder" onClose={() => {}} status={status} manualPaste
    availability={{ agents: [{ harness: "omp", executable: "omp", found: true }, { harness: "pi", executable: "pi", found: false }] }} />));
  expect(host.querySelector<HTMLInputElement>('input[value="pi"]')?.disabled).toBe(true);
  expect(host.textContent).toContain("not on PATH");
  expect(button("Copy question")).toBeDefined();
  expect([...host.querySelectorAll("button")].some(element => element.textContent?.trim() === "Send question")).toBe(false);
});
