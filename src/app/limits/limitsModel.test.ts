import { describe, expect, it } from "vitest";
import type { QuotaLimit, QuotaProviderStatus } from "../../protocol/generated/v1";
import { formatLimitValue, limitPace, providerView, usedFraction, summaryView } from "./limitsModel";

const limit = (changes: Partial<QuotaLimit> = {}): QuotaLimit => ({
  id: "test", window: "7d", tier: null, unit: "percent", used_fraction: null,
  used: null, limit: null, remaining: null, unlimited: false, level: "unknown", resets_at_ms: null, ...changes,
});
const provider = (changes: Partial<QuotaProviderStatus> = {}): QuotaProviderStatus => ({
  provider: "codex", state: "available", error: null, stale: false, fetched_at_ms: 1000,
  accounts: [{ fetched_at_ms: 1000, limits: [limit({ used_fraction: 0.2 })] }], ...changes,
});

describe("subscription usage windows", () => {
  it("shows every tier/window in duration order and the highest usage across anonymous accounts", () => {
    const weekly = limit({ id: "weekly", used_fraction: 0.58 });
    const opus = limit({ id: "opus", tier: "opus", used_fraction: 0.62 });
    const lower = limit({ id: "lower", window: "5h", used_fraction: 0.2 });
    const higher = limit({ id: "higher", window: "5h", used_fraction: 0.39 });
    const view = providerView(provider({ provider: "claude", accounts: [
      { fetched_at_ms: 1000, limits: [weekly, opus, lower] },
      { fetched_at_ms: 2000, limits: [higher] },
    ] }), 3000, false);
    expect(view.segments.map(segment => [segment.label, segment.text, segment.limit])).toEqual([
      ["5h", "39%", higher], ["7d", "58%", weekly], ["Opus 7d", "62%", opus],
    ]);
  });

  it("breaks equal window usage by earlier reset, then preserves report order", () => {
    const later = limit({ used_fraction: 0.3, resets_at_ms: 9000 });
    const earlier = limit({ used_fraction: 0.3, resets_at_ms: 5000 });
    const equal = limit({ used_fraction: 0.3, resets_at_ms: 5000 });
    const lessUsed = limit({ used_fraction: 0.1, resets_at_ms: 4000 });
    const view = providerView(provider({ accounts: [{ fetched_at_ms: 1000, limits: [later, earlier, equal, lessUsed] }] }), 2000, false);
    expect(view.segments[0].limit).toBe(earlier);
  });

  it("keeps unknown and unlimited windows rather than dropping them", () => {
    const view = providerView(provider({ accounts: [{ fetched_at_ms: 1000, limits: [
      limit({ window: "5h" }), limit({ unlimited: true }),
      limit({ window: "monthly", unit: "credits", used: 4 }),
    ] }] }), 2000, false);
    expect(view.segments.map(segment => [segment.label, segment.text, segment.used])).toEqual([
      ["5h", "—", null], ["7d", "unlimited", null], ["Monthly", "4 cr used", null],
    ]);
  });

  it("shows small Business usage without scaling the reported AI credits", () => {
    const business = limit({ window: "monthly", tier: "business", unit: "credits", used: 4, limit: 8000, remaining: 7996, used_fraction: 0.0005 });
    const view = providerView(provider({ provider: "copilot", accounts: [{ fetched_at_ms: 1000, limits: [business] }] }), 2000, false);
    expect(view.segments[0]).toMatchObject({ label: "Monthly", text: "0.05%", used: 0.0005, tone: "ok" });
    expect(formatLimitValue(business, true)).toBe("4 of 8,000 AI credits used · 0.05%");
    expect(formatLimitValue({ ...business, used_fraction: null, used: null }, true)).toBe("4 of 8,000 AI credits used · 0.05%");
  });

  it.each([
    [0, "0%"], [0.0005, "0.05%"], [0.00001, "0%"], [0.01234, "1.23%"],
    [0.8, "80%"], [0.8000001, "80%"], [0.95, "95%"], [0.9500001, "95%"],
    [0.994, "99.4%"], [0.9994, "99.94%"], [1, "100%"],
  ])("formats actual used fraction %s with at most two fractional digits", (fraction, expected) => {
    expect(formatLimitValue(limit({ used_fraction: fraction }))).toBe(expected);
  });

  it("preserves zero used credits and does not mistake a remaining-only balance for usage", () => {
    expect(formatLimitValue(limit({ unit: "credits", used: 0 }), true)).toBe("0 AI credits used");
    expect(formatLimitValue(limit({ unit: "credits", used: 4 }))).toBe("4 cr used");
    expect(formatLimitValue(limit({ unit: "credits", remaining: 0 }))).toBe("—");
    expect(usedFraction(limit({ unit: "credits", remaining: 5, limit: 0 }))).toBeNull();
    expect(usedFraction(limit({ unit: "credits", limit: 100 }))).toBeNull();
  });

  it("derives bounded used credit fractions without changing source precedence", () => {
    expect(usedFraction(limit({ unit: "credits", used_fraction: 0.25, remaining: 20, used: 90, limit: 100 }))).toBe(0.25);
    expect(usedFraction(limit({ unit: "credits", remaining: 20, used: 90, limit: 100 }))).toBe(0.8);
    expect(usedFraction(limit({ unit: "credits", remaining: 0, used: 0, limit: 100 }))).toBe(1);
    expect(usedFraction(limit({ unit: "credits", used: 30, limit: 100 }))).toBe(0.3);
    expect(usedFraction(limit({ unit: "credits", used: 0, limit: 100 }))).toBe(0);
    expect(usedFraction(limit({ unit: "credits", used: 120, limit: 100 }))).toBe(1);
  });

  it("does not infer percent fractions from counts or impose a fraction on unlimited reports", () => {
    expect(usedFraction(limit({ remaining: 20, used: 80, limit: 100 }))).toBeNull();
    expect(usedFraction(limit({ unlimited: true, used_fraction: 0.9 }))).toBeNull();
    expect(usedFraction(limit({ unlimited: true, unit: "credits", remaining: 20, used: 80, limit: 100 }))).toBeNull();
  });

  it("marks only windows reset since their observation stale and retains their used value", () => {
    const windows = [limit({ window: "5h", used_fraction: 0.8, resets_at_ms: 2000 }), limit({ used_fraction: 0.3, resets_at_ms: 9000 })];
    const status = provider({ accounts: [{ fetched_at_ms: 1000, limits: windows }] });
    const view = providerView(status, 3000, false);
    expect(view.segments.map(segment => [segment.text, segment.stale])).toEqual([["80%", true], ["30%", false]]);
    expect(providerView({ ...status, error: "timeout" }, 1500, false).segments.every(segment => segment.stale)).toBe(true);
    expect(providerView(status, 1500, true).segments.every(segment => segment.stale)).toBe(true);
    expect(providerView({ ...status, accounts: [{ fetched_at_ms: 2500, limits: windows }] }, 3000, false).segments[0].stale).toBe(false);
  });

  it("uses the highest fresh usage provider for summaries and retains every window of that provider", () => {
    const stale = providerView(provider({ provider: "copilot", stale: true, accounts: [{ fetched_at_ms: 1000, limits: [limit({ used_fraction: 0.99 })] }] }), 1500, false);
    const low = providerView(provider(), 1500, false);
    const higher = providerView(provider({ provider: "claude", accounts: [{ fetched_at_ms: 1000, limits: [limit({ window: "5h", used_fraction: 0.39 }), limit({ used_fraction: 0.58 })] }] }), 1500, false);
    expect(summaryView([low, higher, stale])).toBe(higher);
    expect(summaryView([low, higher, stale])?.segments.map(segment => segment.text)).toEqual(["39%", "58%"]);
    expect(summaryView([stale])).toBe(stale);
    expect(summaryView([providerView(provider({ state: "not_signed_in", accounts: [] }), 1500, false)])).toBeNull();
  });
});

describe("pace projection", () => {
  const H = 3_600_000;
  it("projects each window independently: 5h can run out while 7d is on pace", () => {
    const now = 100 * H;
    const view = providerView(provider({ provider: "claude", accounts: [{ fetched_at_ms: now, limits: [
      limit({ window: "5h", used_fraction: 0.6, resets_at_ms: now + 3 * H }),
      limit({ window: "7d", used_fraction: 0.3, resets_at_ms: now + 4 * 24 * H }),
    ] }] }), now, false);
    const [five, seven] = view.segments;
    expect(five.pace!.expected).toBeCloseTo(0.4);
    expect(five.pace!.runsOutMs).toBeCloseTo(H * 4 / 3, -3); // 0.6 in 2h → 0.4 left lasts 80 min
    expect(seven.pace!.runsOutMs).toBeNull();
  });

  it("gives no projection without a window length, early in the window, or when stale", () => {
    const now = 100 * H;
    expect(limitPace(limit({ window: null, resets_at_ms: now + H }), 0.5, now)).toBeNull();
    expect(limitPace(limit({ window: "5h", resets_at_ms: now + 4.9 * H }), 0.5, now)).toBeNull();
    expect(limitPace(limit({ window: "5h", resets_at_ms: now - 1 }), 0.5, now)).toBeNull();
    expect(limitPace(limit({ window: "5h", resets_at_ms: now + H }), 0, now)!.runsOutMs).toBeNull();
  });
});
