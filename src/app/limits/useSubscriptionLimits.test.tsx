// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { QuotaStatusResponse } from "../../protocol/generated/v1";
import { useSubscriptionLimits } from "./useSubscriptionLimits";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const mounted: Array<{ root: Root; host: HTMLDivElement }> = [];
const originalVisibility = Object.getOwnPropertyDescriptor(document, "visibilityState");

function visibility(value: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", { value, configurable: true });
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(1000);
  visibility("visible");
});

afterEach(async () => {
  for (const { root, host } of mounted.splice(0)) {
    await act(async () => root.unmount());
    host.remove();
  }
  if (originalVisibility) Object.defineProperty(document, "visibilityState", originalVisibility);
  else Reflect.deleteProperty(document, "visibilityState");
  vi.useRealTimers();
});

function snapshot(collecting = false): QuotaStatusResponse {
  return { generated_at_ms: Date.now(), collecting, providers: [] };
}

function fixture() {
  const quotaStatus = vi.fn<CockpitClient["quotaStatus"]>(async () => snapshot());
  return { client: { quotaStatus } as unknown as CockpitClient, quotaStatus };
}

function view(client: CockpitClient) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push({ root, host });
  function View({ agentsWorking }: { agentsWorking: boolean }) {
    const state = useSubscriptionLimits(client, agentsWorking);
    return <output data-link={state.link} data-absent={state.absent}>
      {state.snapshot?.generated_at_ms ?? "loading"}
    </output>;
  }
  return {
    host,
    render: async (agentsWorking: boolean) => { await act(async () => root.render(<View agentsWorking={agentsWorking} />)); },
    unmount: async () => { await act(async () => root.render(null)); },
  };
}

async function advance(milliseconds: number) {
  await act(async () => { await vi.advanceTimersByTimeAsync(milliseconds); });
}

async function changeVisibility(value: DocumentVisibilityState) {
  await act(async () => {
    visibility(value);
    document.dispatchEvent(new Event("visibilitychange"));
  });
}

it.each([
  { working: false, cadence: 60_000 },
  { working: true, cadence: 15_000 },
])("refreshes displayed values every $cadence ms when working=$working, without duplicating the initial request", async ({ working, cadence }) => {
  const { client, quotaStatus } = fixture();
  const display = view(client);
  await display.render(working);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  expect(display.host.textContent).toBe("1000");
  await display.render(working);
  await advance(cadence - 1);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  expect(display.host.textContent).toBe("1000");
  await advance(1);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  expect(display.host.textContent).toBe(String(1000 + cadence));
  await advance(cadence);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
  expect(display.host.textContent).toBe(String(1000 + cadence * 2));
});

it("refreshes immediately on becoming active and promptly replaces the active timer on becoming idle", async () => {
  const { client, quotaStatus } = fixture();
  const display = view(client);
  await display.render(false);
  await advance(10_000);
  await display.render(true);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  expect(display.host.textContent).toBe("11000");
  await advance(15_000);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
  expect(display.host.textContent).toBe("26000");
  await advance(1000);
  await display.render(false);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
  await advance(59_999);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
  expect(display.host.textContent).toBe("26000");
  await advance(1);
  expect(quotaStatus).toHaveBeenCalledTimes(4);
  expect(display.host.textContent).toBe("87000");
});

it.each([
  { working: true, cadence: 15_000 },
  { working: false, cadence: 60_000 },
])("keeps an in-flight request intact during transition to working=$working and uses the new cadence after completion", async ({ working, cadence }) => {
  const { client, quotaStatus } = fixture();
  let complete!: (response: QuotaStatusResponse) => void;
  const pending = new Promise<QuotaStatusResponse>(resolve => { complete = resolve; });
  quotaStatus.mockReturnValueOnce(pending);
  const display = view(client);
  await display.render(!working);
  const signal = quotaStatus.mock.calls[0][1]!;
  await display.render(working);
  await changeVisibility("visible");
  await advance(90_000);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  expect(signal.aborted).toBe(false);
  expect(display.host.textContent).toBe("loading");
  await act(async () => complete(snapshot()));
  expect(display.host.textContent).toBe("91000");
  await advance(cadence - 1);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  await advance(1);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  expect(display.host.textContent).toBe(String(91_000 + cadence));
});

it.each([
  { working: false, cadence: 60_000 },
  { working: true, cadence: 15_000 },
])("polls a collecting snapshot after three seconds, then resumes $cadence ms polling", async ({ working, cadence }) => {
  const { client, quotaStatus } = fixture();
  quotaStatus.mockImplementationOnce(async () => snapshot(true));
  const display = view(client);
  await display.render(working);
  await advance(2999);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  await advance(1);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  expect(display.host.textContent).toBe("4000");
  await advance(cadence - 1);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  await advance(1);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
  expect(display.host.textContent).toBe(String(4000 + cadence));
});

it("sends no requests while hidden, including an active transition, and refreshes immediately when visible", async () => {
  visibility("hidden");
  const { client, quotaStatus } = fixture();
  const display = view(client);
  await display.render(false);
  await advance(60_000);
  await display.render(true);
  await advance(45_000);
  expect(quotaStatus).not.toHaveBeenCalled();
  expect(display.host.textContent).toBe("loading");
  await changeVisibility("visible");
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  expect(display.host.textContent).toBe("106000");
  await advance(15_000);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  expect(display.host.textContent).toBe("121000");
  await changeVisibility("hidden");
  await advance(60_000);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  expect(display.host.textContent).toBe("121000");
  await changeVisibility("visible");
  expect(quotaStatus).toHaveBeenCalledTimes(3);
  expect(display.host.textContent).toBe("181000");
});

it("stops unavailable-host polling despite mode and visibility changes", async () => {
  const { client, quotaStatus } = fixture();
  quotaStatus.mockRejectedValue(new CockpitClientError("http_error", "Quota unavailable", { operationCode: "quota_unavailable" }));
  const display = view(client);
  await display.render(false);
  expect(display.host.querySelector("output")?.dataset.absent).toBe("true");
  await display.render(true);
  await changeVisibility("hidden");
  await changeVisibility("visible");
  await advance(360_000);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  expect(display.host.querySelector("output")?.dataset.absent).toBe("true");
});

it("retains last-good values through offline failures and recovers on the active cadence", async () => {
  const { client, quotaStatus } = fixture();
  const display = view(client);
  await display.render(false);
  quotaStatus.mockRejectedValueOnce(new CockpitClientError("transport_error", "Disconnected"));
  quotaStatus.mockRejectedValueOnce(new CockpitClientError("transport_error", "Disconnected"));
  await display.render(true);
  expect(display.host.querySelector("output")?.dataset.link).toBe("live");
  expect(display.host.textContent).toBe("1000");
  await advance(15_000);
  expect(display.host.querySelector("output")?.dataset.link).toBe("offline");
  expect(display.host.textContent).toBe("1000");
  await advance(15_000);
  expect(display.host.querySelector("output")?.dataset.link).toBe("live");
  expect(display.host.textContent).toBe("31000");
});

it("retains the client snapshot across remounts without sharing it with another host", async () => {
  const first = fixture();
  const display = view(first.client);
  await display.render(false);
  await display.unmount();
  visibility("hidden");
  await display.render(true);
  expect(display.host.textContent).toBe("1000");
  expect(display.host.querySelector("output")?.dataset.link).toBe("live");
  expect(first.quotaStatus).toHaveBeenCalledTimes(1);
  const second = fixture();
  const otherDisplay = view(second.client);
  await otherDisplay.render(true);
  expect(otherDisplay.host.textContent).toBe("loading");
  expect(otherDisplay.host.querySelector("output")?.dataset.link).toBe("loading");
  expect(second.quotaStatus).not.toHaveBeenCalled();
});
