import { describe, expect, it, vi } from "vitest";
import type { QuotaLimit, QuotaProvider, QuotaProviderStatus, QuotaStatusResponse } from "../protocol/generated/v1";
import { createBrowserClient } from "./browser";
import { CockpitClientError } from "./CockpitClient";
import { createNativeClient } from "./native";
import { parseQuotaStatusResponse } from "./quotaProtocol";

const NOW = 1_790_856_600_000;

function snapshot(): QuotaStatusResponse {
  const providers: QuotaProvider[] = ["codex", "claude", "copilot"];
  return {
    generated_at_ms: NOW,
    collecting: false,
    providers: providers.map((provider): QuotaProviderStatus => ({
      provider, state: "available", error: null, fetched_at_ms: NOW - 1000, stale: false,
      accounts: [{ fetched_at_ms: NOW - 1000, limits: [{
        id: `${provider}:1`, window: provider === "copilot" ? "monthly" : "5h", tier: null,
        unit: provider === "copilot" ? "credits" : "percent", used_fraction: 0.2,
        used: provider === "copilot" ? 300 : null,
        limit: provider === "copilot" ? 1500 : null,
        remaining: provider === "copilot" ? 1200 : null,
        unlimited: false, level: "ok", resets_at_ms: NOW + 100_000,
      }] }],
    })),
  };
}

function withLimit(overrides: Record<string, unknown>, providerIndex = 0): unknown {
  const response = snapshot();
  const provider = response.providers[providerIndex];
  return {
    ...response,
    providers: response.providers.map((item, index) => index !== providerIndex ? item : {
      ...provider, accounts: [{ ...provider.accounts[0], limits: [{ ...provider.accounts[0].limits[0], ...overrides }] }],
    }),
  };
}

function failure(value: unknown): CockpitClientError {
  try { parseQuotaStatusResponse(value); } catch (error) {
    expect(error).toBeInstanceOf(CockpitClientError);
    return error as CockpitClientError;
  }
  throw new Error("Expected malformed quota response");
}

describe("quota response contract", () => {
  it("keeps unknown percentage usage distinct from zero and explicit unlimited credits", () => {
    const response = snapshot();
    response.providers[0].accounts[0].limits[0].used_fraction = null;
    response.providers[0].accounts[0].limits[0].level = "unknown";
    Object.assign(response.providers[2].accounts[0].limits[0], {
      unlimited: true, used_fraction: null, used: null, limit: null, remaining: null,
    });
    const parsed = parseQuotaStatusResponse(response);
    expect(parsed.providers[0].accounts[0].limits[0]).toMatchObject({ used_fraction: null, unlimited: false });
    expect(parsed.providers[2].accounts[0].limits[0]).toMatchObject({ unlimited: true, used_fraction: null, remaining: null });
    expect(parseQuotaStatusResponse(withLimit({ used_fraction: 0 })).providers[0].accounts[0].limits[0].used_fraction).toBe(0);
  });

  it("retains all anonymous account and tier-scoped limits, with oldest fetch time", () => {
    const response = snapshot();
    const provider = response.providers[1];
    const tierLimit: QuotaLimit = { ...provider.accounts[0].limits[0], id: "claude:7d:1", window: "7d", tier: "opus", used_fraction: 0.9, level: "warning" };
    provider.accounts[0].limits.push(tierLimit);
    provider.accounts.push({ fetched_at_ms: NOW - 2000, limits: [{ ...tierLimit, id: "claude:7d:2", used_fraction: 0.5 }] });
    provider.fetched_at_ms = NOW - 2000;
    const parsed = parseQuotaStatusResponse(response).providers[1];
    expect(parsed.accounts).toEqual(provider.accounts);
    expect(parsed.fetched_at_ms).toBe(NOW - 2000);
  });

  it("retains last-good values as stale after failure even before the age threshold", () => {
    const response = snapshot();
    response.providers[0].error = "timeout";
    response.providers[0].stale = true;
    expect(parseQuotaStatusResponse(response).providers[0]).toMatchObject({ state: "available", stale: true, error: "timeout", accounts: response.providers[0].accounts });
    response.providers[0].stale = false;
    expect(failure(response).code).toBe("malformed_response");
  });

  it("computes stale from the oldest retained account, including the exact age boundary", () => {
    const response = snapshot();
    const provider = response.providers[0];
    provider.fetched_at_ms = NOW - 15 * 60_000;
    provider.accounts[0].fetched_at_ms = provider.fetched_at_ms;
    expect(parseQuotaStatusResponse(response).providers[0].stale).toBe(false);
    provider.fetched_at_ms -= 1;
    provider.accounts[0].fetched_at_ms -= 1;
    provider.stale = true;
    expect(parseQuotaStatusResponse(response).providers[0].stale).toBe(true);
  });

  it("accepts absent states without inventing amounts or accounts", () => {
    const response = snapshot();
    response.providers = [
      { provider: "codex", state: "pending", error: null, stale: false, fetched_at_ms: null, accounts: [] },
      { provider: "claude", state: "not_signed_in", error: "not_signed_in", stale: false, fetched_at_ms: null, accounts: [] },
      { provider: "copilot", state: "unsupported", error: "unsupported", stale: false, fetched_at_ms: null, accounts: [] },
    ];
    expect(parseQuotaStatusResponse(response).providers).toEqual(response.providers);
    response.providers[2].state = "unavailable";
    response.providers[2].error = "cache_unavailable";
    expect(parseQuotaStatusResponse(response).providers[2].error).toBe("cache_unavailable");
  });

  it.each([
    ["a missing nullable fraction", { used_fraction: undefined }],
    ["a non-finite fraction", { used_fraction: Infinity }],
    ["NaN credits", { used: NaN }, 2],
    ["a negative fraction", { used_fraction: -0.1 }],
    ["an excessive fraction", { used_fraction: 10.001 }],
    ["excessive credits", { limit: 1e9 + 1 }, 2],
    ["credits on a percentage limit", { used: 1 }],
    ["legacy premium requests", { unit: "requests" }, 2],
    ["percentage units on Copilot", { unit: "percent" }, 2],
    ["implicit unlimited", { unlimited: null }, 2],
    ["unlimited with known amounts", { unlimited: true }, 2],
    ["unlimited percentages", { unlimited: true, used_fraction: null }],
    ["an unknown level", { level: "healthy" }],
    ["a raw label id", { id: "Account user@example.com" }],
    ["an invalid tier", { tier: "user@example.com" }],
    ["an invalid window", { window: "Every five hours" }],
    ["an unsafe reset timestamp", { resets_at_ms: Number.MAX_SAFE_INTEGER + 1 }],
    ["a missing reset timestamp", { resets_at_ms: undefined }],
  ])("rejects %s", (_name, overrides, index?: number) => {
    expect(failure(withLimit(overrides as Record<string, unknown>, index)).code).toBe("malformed_response");
  });

  it.each([
    ["reordered providers", (response: QuotaStatusResponse) => response.providers.reverse()],
    ["missing providers", (response: QuotaStatusResponse) => response.providers.pop()],
    ["duplicate providers", (response: QuotaStatusResponse) => { response.providers[1].provider = "codex"; }],
    ["available without accounts", (response: QuotaStatusResponse) => { response.providers[0].accounts = []; }],
    ["pending with retained accounts", (response: QuotaStatusResponse) => { response.providers[0].state = "pending"; }],
    ["pending with an error", (response: QuotaStatusResponse) => { Object.assign(response.providers[0], { state: "pending", accounts: [], fetched_at_ms: null, error: "failed" }); }],
    ["unavailable without an error", (response: QuotaStatusResponse) => { Object.assign(response.providers[0], { state: "unavailable", accounts: [], fetched_at_ms: null }); }],
    ["an unknown state", (response: QuotaStatusResponse) => { Object.assign(response.providers[0], { state: "healthy" }); }],
    ["a missing error field", (response: QuotaStatusResponse) => { Object.assign(response.providers[0], { error: undefined }); }],
    ["a missing nullable fetch timestamp", (response: QuotaStatusResponse) => { Object.assign(response.providers[0], { fetched_at_ms: undefined }); }],
    ["a malformed collecting flag", (response: QuotaStatusResponse) => { Object.assign(response, { collecting: "yes" }); }],
    ["a non-finite generated timestamp", (response: QuotaStatusResponse) => { response.generated_at_ms = Infinity; }],
    ["an incorrect oldest timestamp", (response: QuotaStatusResponse) => { response.providers[0].fetched_at_ms = NOW; }],
    ["a future fetch timestamp", (response: QuotaStatusResponse) => { response.providers[0].fetched_at_ms = NOW + 60_001; response.providers[0].accounts[0].fetched_at_ms = NOW + 60_001; }],
    ["expired retained data", (response: QuotaStatusResponse) => { response.providers[0].fetched_at_ms = NOW - 24 * 60 * 60_000 - 1; response.providers[0].accounts[0].fetched_at_ms = response.providers[0].fetched_at_ms; response.providers[0].stale = true; }],
    ["an empty limits account", (response: QuotaStatusResponse) => { response.providers[0].accounts[0].limits = []; }],
    ["too many accounts", (response: QuotaStatusResponse) => { response.providers[0].accounts = Array.from({ length: 9 }, () => response.providers[0].accounts[0]); }],
    ["too many limits", (response: QuotaStatusResponse) => { response.providers[0].accounts[0].limits = Array.from({ length: 25 }, () => response.providers[0].accounts[0].limits[0]); }],
  ])("rejects %s", (_name, mutate) => {
    const response = snapshot();
    (mutate as (value: QuotaStatusResponse) => unknown)(response);
    expect(failure(response).code).toBe("malformed_response");
  });

  it("projects unknown metadata away and never includes source text in parser errors", () => {
    const response = snapshot();
    const secret = "raw-private-output@example.com";
    Object.assign(response, { stderr: secret });
    Object.assign(response.providers[0], { account_id: secret });
    Object.assign(response.providers[0].accounts[0], { label: secret });
    Object.assign(response.providers[0].accounts[0].limits[0], { metadata: secret });
    expect(JSON.stringify(parseQuotaStatusResponse(response))).not.toContain(secret);
    const error = failure(withLimit({ id: secret }));
    expect(error.message).not.toContain(secret);
    expect(error.cause).toBeUndefined();
    expect(failure({ ...snapshot(), providers: [{ ...snapshot().providers[0], error: secret }, ...snapshot().providers.slice(1)] }).message).not.toContain(secret);
  });
});

describe("quota client errors and cancellation", () => {
  it("preserves intentional quota absence as the operation code in both transports", async () => {
    const envelope = { code: "quota_unavailable", message: "Subscription quota is not configured in this host" };
    const browser = createBrowserClient(async () => new Response(JSON.stringify(envelope), { status: 503 }));
    const native = createNativeClient(async () => { throw envelope; });
    await expect(browser.quotaStatus()).rejects.toMatchObject({ code: "http_error", operationCode: "quota_unavailable", status: 503 });
    await expect(native.quotaStatus()).rejects.toMatchObject({ code: "native_error", operationCode: "quota_unavailable" });
  });

  it("refuses already-aborted requests without touching either transport", async () => {
    const request = vi.fn(async () => new Response(JSON.stringify(snapshot())));
    const invoke = vi.fn(async () => snapshot());
    const controller = new AbortController();
    controller.abort();
    await expect(createBrowserClient(request).quotaStatus(controller.signal)).rejects.toMatchObject({ name: "AbortError" });
    await expect(createNativeClient(invoke).quotaStatus(controller.signal)).rejects.toMatchObject({ name: "AbortError" });
    expect(request).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("does not publish a native result after the reader was cancelled", async () => {
    let resolve: (value: unknown) => void = () => { throw new Error("Native read not started"); };
    const pending = new Promise<unknown>((complete) => { resolve = complete; });
    const controller = new AbortController();
    const result = createNativeClient(() => pending).quotaStatus(controller.signal);
    controller.abort();
    resolve(snapshot());
    await expect(result).rejects.toMatchObject({ name: "AbortError" });
  });
});
