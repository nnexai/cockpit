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

it.each([false, true])("refreshes every 60 seconds with working=%s, including rerenders", async working => {
  const { client, quotaStatus } = fixture();
  const display = view(client);
  await display.render(working);
  await display.render(working);
  await advance(59_999);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  await advance(1);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  await advance(60_000);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
  expect(quotaStatus.mock.calls.map(([request]) => request.agents_working)).toEqual([working, working, working]);
});

it("refreshes on both activity transitions and focus, but not unchanged renders", async () => {
  const { client, quotaStatus } = fixture();
  const display = view(client);
  await display.render(false);
  await display.render(true);
  await display.render(true);
  await display.render(false);
  await act(async () => window.dispatchEvent(new Event("focus")));
  expect(quotaStatus.mock.calls.map(([request]) => request.agents_working)).toEqual([false, true, false, false]);
});

it("reschedules the periodic refresh after focus and cancels it on unmount", async () => {
  const { client, quotaStatus } = fixture();
  const display = view(client);
  await display.render(false);
  await advance(30_000);
  await act(async () => window.dispatchEvent(new Event("focus")));
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  await advance(59_999);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  await advance(1);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
  await display.unmount();
  await advance(120_000);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
});

it("coalesces triggers during a request without overlapping and uses the latest activity", async () => {
  const { client, quotaStatus } = fixture();
  let complete!: (response: QuotaStatusResponse) => void;
  quotaStatus.mockReturnValueOnce(new Promise(resolve => { complete = resolve; }));
  const display = view(client);
  await display.render(false);
  await display.render(true);
  await act(async () => window.dispatchEvent(new Event("focus")));
  await advance(90_000);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  expect(quotaStatus.mock.calls[0][1]?.aborted).toBe(false);
  await act(async () => complete(snapshot()));
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  expect(quotaStatus.mock.calls[1][0].agents_working).toBe(true);
  await advance(60_000);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
});

it("uses faster polling while collection is in progress, then returns to the normal cadence", async () => {
  const { client, quotaStatus } = fixture();
  quotaStatus.mockImplementationOnce(async () => snapshot(true));
  quotaStatus.mockImplementationOnce(async () => snapshot(true));
  const display = view(client);
  await display.render(false);
  await advance(2999);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  await advance(3001);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
  await advance(59_999);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
  await advance(1);
  expect(quotaStatus).toHaveBeenCalledTimes(4);
});

it("returns to the normal cadence after a failed collecting request", async () => {
  const { client, quotaStatus } = fixture();
  quotaStatus.mockImplementationOnce(async () => snapshot(true));
  quotaStatus.mockRejectedValueOnce(new CockpitClientError("transport_error", "Disconnected"));
  const display = view(client);
  await display.render(false);
  await advance(3000);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  await advance(59_999);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  await advance(1);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
});

it("refreshes once at a known future quota reset, not repeatedly for a past deadline", async () => {
  const { client, quotaStatus } = fixture();
  const response: QuotaStatusResponse = {
    ...snapshot(),
    providers: [{ provider: "codex", state: "available", error: null, fetched_at_ms: 1000, stale: false,
      accounts: [{ fetched_at_ms: 1000, limits: [{ id: "primary", window: null, tier: null, unit: "percent",
        used_fraction: 0.5, used: null, limit: null, remaining: null, unlimited: false, level: "ok", resets_at_ms: 6000 }] }] }],
  };
  quotaStatus.mockResolvedValue(response);
  const display = view(client);
  await display.render(false);
  await advance(4999);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  await advance(1);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  await advance(59_999);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  await advance(1);
  expect(quotaStatus).toHaveBeenCalledTimes(3);
});

it("suppresses requests while hidden and refreshes when visible", async () => {
  visibility("hidden");
  const { client, quotaStatus } = fixture();
  const display = view(client);
  await display.render(false);
  await display.render(true);
  await act(async () => window.dispatchEvent(new Event("focus")));
  await advance(600_000);
  expect(quotaStatus).not.toHaveBeenCalled();
  await changeVisibility("visible");
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  await changeVisibility("hidden");
  await advance(600_000);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  await changeVisibility("visible");
  expect(quotaStatus).toHaveBeenCalledTimes(2);
});

it("pauses collection polling while hidden and refreshes on return", async () => {
  const { client, quotaStatus } = fixture();
  quotaStatus.mockImplementationOnce(async () => snapshot(true));
  const display = view(client);
  await display.render(false);
  await changeVisibility("hidden");
  await advance(30_000);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  await changeVisibility("visible");
  expect(quotaStatus).toHaveBeenCalledTimes(2);
  await advance(30_000);
  expect(quotaStatus).toHaveBeenCalledTimes(2);
});

it("ignores a late response after switching client", async () => {
  const first = fixture();
  const second = fixture();
  let complete!: (response: QuotaStatusResponse) => void;
  first.quotaStatus.mockReturnValueOnce(new Promise(resolve => { complete = resolve; }));
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push({ root, host });
  function View({ client }: { client: CockpitClient }) {
    const state = useSubscriptionLimits(client, false);
    return <output>{state.snapshot?.generated_at_ms ?? "loading"}</output>;
  }
  await act(async () => root.render(<View client={first.client} />));
  await act(async () => root.render(<View client={second.client} />));
  expect(first.quotaStatus.mock.calls[0][1]?.aborted).toBe(true);
  await act(async () => complete({ ...snapshot(), generated_at_ms: 999 }));
  expect(host.textContent).toBe("1000");
});

it("stops unavailable-host requests despite activity, focus and visibility changes", async () => {
  const { client, quotaStatus } = fixture();
  quotaStatus.mockRejectedValue(new CockpitClientError("http_error", "Quota unavailable", { operationCode: "quota_unavailable" }));
  const display = view(client);
  await display.render(false);
  await display.render(true);
  await act(async () => window.dispatchEvent(new Event("focus")));
  await changeVisibility("hidden");
  await changeVisibility("visible");
  await advance(360_000);
  expect(quotaStatus).toHaveBeenCalledTimes(1);
  expect(display.host.querySelector("output")?.dataset.absent).toBe("true");
});

it("retains last-good values through failures and recovers on an event", async () => {
  const { client, quotaStatus } = fixture();
  const display = view(client);
  await display.render(false);
  quotaStatus.mockRejectedValueOnce(new CockpitClientError("transport_error", "Disconnected"));
  quotaStatus.mockRejectedValueOnce(new CockpitClientError("transport_error", "Disconnected"));
  await display.render(true);
  await display.render(false);
  expect(display.host.querySelector("output")?.dataset.link).toBe("offline");
  expect(display.host.textContent).toBe("1000");
  await advance(1000);
  await act(async () => window.dispatchEvent(new Event("focus")));
  expect(display.host.querySelector("output")?.dataset.link).toBe("live");
  expect(display.host.textContent).toBe("2000");
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
