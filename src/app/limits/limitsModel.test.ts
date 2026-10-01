import { describe, expect, it } from "vitest";
import type { QuotaLimit, QuotaProviderStatus } from "../../protocol/generated/v1";
import { formatLimitValue, limitingLimit, providerView, remainingFraction, summaryView } from "./limitsModel";

const limit = (changes: Partial<QuotaLimit> = {}): QuotaLimit => ({
  id: "test", window: "7d", tier: null, unit: "percent", used_fraction: null,
  used: null, limit: null, remaining: null, unlimited: false, level: "unknown", resets_at_ms: null, ...changes,
});
const provider = (changes: Partial<QuotaProviderStatus> = {}): QuotaProviderStatus => ({
  provider: "codex", state: "available", error: null, stale: false, fetched_at_ms: 1000,
  accounts: [{ fetched_at_ms: 1000, limits: [limit({ used_fraction: 0.2 })] }], ...changes,
});

describe("subscription limit selection", () => {
  it("chooses the binding window across all anonymous accounts, including tier limits", () => {
    const shared = limit({ id: "shared", window: "1h", used_fraction: 0.2 });
    const tier = limit({ id: "tier", window: "7d", tier: "opus", used_fraction: 0.8 });
    const view = providerView(provider({ accounts: [{ fetched_at_ms: 1000, limits: [shared] }, { fetched_at_ms: 2000, limits: [tier] }] }), 3000, false);
    expect(view.limiting).toBe(tier);
    expect(view.text).toBe("20%");
  });
  it("breaks equal fractions by earlier reset and keeps report order for exact ties", () => {
    const later = limit({ used_fraction: 0.3, resets_at_ms: 9000 });
    const earlier = limit({ used_fraction: 0.3, resets_at_ms: 5000 });
    const equal = limit({ used_fraction: 0.3, resets_at_ms: 5000 });
    expect(limitingLimit([later, earlier, equal])).toBe(earlier);
  });
  it("prioritizes the binding fraction over an earlier reset on a less constrained limit", () => {
    const binding = limit({ used_fraction: 0.8, resets_at_ms: 9000 });
    const earlier = limit({ used_fraction: 0.3, resets_at_ms: 5000 });
    expect(limitingLimit([binding, earlier])).toBe(binding);
    expect(limitingLimit([earlier, binding])).toBe(binding);
  });
  it("never turns missing values into zero or unlimited and distinguishes tiny positive balances", () => {
    expect(formatLimitValue(limit())).toBe("—");
    expect(limitingLimit([limit()])).toBeNull();
    expect(formatLimitValue(limit({ used_fraction: 0 }))).toBe("100%");
    expect(formatLimitValue(limit({ used_fraction: 1 }))).toBe("0%");
    expect(formatLimitValue(limit({ used_fraction: 0.999 }))).toBe("<1%");
    expect(formatLimitValue(limit({ used_fraction: 1.2 }))).toBe("0%");
    expect(formatLimitValue(limit({ unlimited: true }))).toBe("unlimited");
  });
  it("shows actual credit balances, preserves zero, and invents no fraction without an entitlement", () => {
    const credits = limit({ unit: "credits", remaining: 1240, limit: 3000 });
    expect(formatLimitValue(credits, true)).toBe("1,240 of 3,000 AI credits left");
    expect(remainingFraction(credits)).toBeCloseTo(1240 / 3000);
    expect(formatLimitValue(limit({ unit: "credits", remaining: 0 }))).toBe("0 cr");
    expect(remainingFraction(limit({ unit: "credits", remaining: 0 }))).toBeNull();
    expect(remainingFraction(limit({ unit: "credits", remaining: 5, limit: 0 }))).toBeNull();
    expect(formatLimitValue(limit({ unit: "credits", used: 2 }), true)).toBe("2 AI credits used");
  });
  it("derives bounded credit fractions from usage when no remaining balance is reported", () => {
    expect(remainingFraction(limit({ unit: "credits", used: 30, limit: 100 }))).toBeCloseTo(0.7);
    expect(remainingFraction(limit({ unit: "credits", used: 0, limit: 100 }))).toBe(1);
    expect(remainingFraction(limit({ unit: "credits", used: 100, limit: 100 }))).toBe(0);
    expect(remainingFraction(limit({ unit: "credits", used: 120, limit: 100 }))).toBe(0);
    expect(remainingFraction(limit({ unit: "credits", limit: 100 }))).toBeNull();
  });
  it("uses reported credit fractions before balances, and remaining balances before usage", () => {
    expect(remainingFraction(limit({ unit: "credits", used_fraction: 0.25, remaining: 20, used: 90, limit: 100 }))).toBe(0.75);
    expect(remainingFraction(limit({ unit: "credits", remaining: 20, used: 90, limit: 100 }))).toBe(0.2);
    expect(remainingFraction(limit({ unit: "credits", remaining: 0, used: 0, limit: 100 }))).toBe(0);
  });
  it("does not infer percent fractions from raw counts or bound unlimited reports", () => {
    expect(remainingFraction(limit({ remaining: 20, used: 80, limit: 100 }))).toBeNull();
    expect(remainingFraction(limit({ unlimited: true, used_fraction: 0.9 }))).toBeNull();
    expect(remainingFraction(limit({ unlimited: true, unit: "credits", remaining: 20, used: 80, limit: 100 }))).toBeNull();
  });
  it("prefers a computable fraction to unlimited or balance-only limits", () => {
    const unlimited = limit({ unlimited: true });
    const balance = limit({ unit: "credits", remaining: 0 });
    const bounded = limit({ used_fraction: 0.1 });
    expect(limitingLimit([unlimited, balance, bounded])).toBe(bounded);
    expect(limitingLimit([unlimited, balance])).toBe(balance);
  });
  it("falls back to a reported credit usage while skipping unknown entitlements and percent counts", () => {
    const unknown = limit({ unit: "credits", limit: 100 });
    const percent = limit({ remaining: 20, used: 80, limit: 100 });
    const unlimited = limit({ unlimited: true });
    const usage = limit({ unit: "credits", used: 2 });
    const laterUsage = limit({ unit: "credits", used: 3 });
    expect(limitingLimit([unknown, percent, unlimited, usage, laterUsage])).toBe(usage);
    expect(limitingLimit([unknown, percent])).toBeNull();
  });
  it("uses the first unlimited report only when no measurable limit or credit balance exists", () => {
    const unlimited = limit({ unlimited: true });
    const laterUnlimited = limit({ unlimited: true });
    expect(limitingLimit([limit(), unlimited, laterUnlimited])).toBe(unlimited);
    expect(limitingLimit([])).toBeNull();
  });
  it("retains reported values after reset, failure, and offline while marking them stale", () => {
    const status = provider({ accounts: [{ fetched_at_ms: 1000, limits: [limit({ used_fraction: 0.8, resets_at_ms: 2000 })] }] });
    const reset = providerView(status, 3000, false);
    expect(reset.stale).toBe(true);
    expect(reset.text).toBe("20%");
    expect(providerView(provider({ error: "timeout" }), 1500, false).stale).toBe(true);
    expect(providerView(provider(), 1500, true).stale).toBe(true);
    // A reset already reflected in the source report does not invalidate that report.
    expect(providerView(provider({ accounts: [{ fetched_at_ms: 2500, limits: status.accounts[0].limits }] }), 3000, false).stale).toBe(false);
  });
  it("uses the worst fresh provider for narrow summaries before considering stale data", () => {
    const stale = providerView(provider({ provider: "claude", stale: true, accounts: [{ fetched_at_ms: 1000, limits: [limit({ used_fraction: 0.99 })] }] }), 1500, false);
    const fresh = providerView(provider(), 1500, false);
    expect(summaryView([stale, fresh])).toBe(fresh);
    expect(summaryView([stale, { ...fresh, stale: true }])).toBe(stale);
    expect(summaryView([providerView(provider({ state: "not_signed_in", accounts: [], fetched_at_ms: null }), 1500, false)])).toBeNull();
  });
});
