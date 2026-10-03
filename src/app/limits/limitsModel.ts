import type { QuotaErrorCode, QuotaLimit, QuotaProvider, QuotaProviderStatus, QuotaStatusResponse } from "../../protocol/generated/v1";
import { formatAgo, formatDate } from "../library/libraryState";

export const PROVIDER_ORDER: readonly QuotaProvider[] = ["codex", "claude", "copilot"];
const number = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
const clamp = (value: number) => Math.max(0, Math.min(1, value));

export function providerName(provider: string): string {
  return provider.charAt(0).toUpperCase() + provider.slice(1);
}

export function usedFraction(limit: QuotaLimit): number | null {
  if (limit.unlimited) return null;
  if (limit.used_fraction !== null) return clamp(limit.used_fraction);
  if (limit.unit === "credits" && limit.limit !== null && limit.limit > 0) {
    if (limit.remaining !== null) return clamp((limit.limit - limit.remaining) / limit.limit);
    if (limit.used !== null) return clamp(limit.used / limit.limit);
  }
  return null;
}

export type UsageTone = "ok" | "warning" | "alert";

export function usageTone(used: number | null): UsageTone | null {
  return used === null ? null : used > 0.95 ? "alert" : used > 0.8 ? "warning" : "ok";
}

export function windowLabel(window: string | null, tier: string | null): string {
  const title = (value: string) => /^\d+[smhdw]$/.test(value) ? value : providerName(value.replace(/[-_]/g, " "));
  return [tier && title(tier), window && title(window)].filter(Boolean).join(" ") || "Limit";
}

export function formatLimitValue(limit: QuotaLimit | null, detail = false): string {
  if (!limit) return "—";
  if (limit.unlimited) return detail ? "Unlimited" : "unlimited";
  const fraction = usedFraction(limit);
  const percent = fraction === null ? null : `${number.format(fraction * 100)}%`;
  if (!detail && percent !== null) return percent;
  if (limit.unit === "percent") return percent === null ? "—" : `${percent} used`;
  const used = limit.used ?? (limit.limit !== null && limit.remaining !== null ? Math.max(0, limit.limit - limit.remaining) : null);
  if (used === null) return percent === null ? "—" : `${percent} used`;
  if (detail) return `${number.format(used)}${limit.limit === null ? "" : ` of ${number.format(limit.limit)}`} AI credits used${percent === null ? "" : ` · ${percent}`}`;
  return `${used >= 10_000 ? `${number.format(used / 1000)}k` : number.format(used)} cr used`;
}

export function errorText(error: QuotaErrorCode | null): string {
  switch (error) {
    case "source_missing": return "OMP usage source not found";
    case "not_signed_in": return "Not signed in to OMP";
    case "usage_unavailable": return "No usage report available";
    case "unsupported": return "Not reported for this account";
    case "failed": return "Usage check failed";
    case "timeout": return "Usage check timed out";
    case "malformed": return "Usage report couldn't be read";
    case "cache_unavailable": return "Shared usage cache unavailable";
    default: return "Limits unavailable";
  }
}

export function formatReset(ms: number | null, now: number): string | null {
  if (ms === null) return null;
  if (ms <= now) return `reset ${formatAgo(ms, now)}`;
  const minutes = Math.max(1, Math.ceil((ms - now) / 60_000));
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor(minutes / 60);
  const duration = days > 0 ? `${days} d` : hours > 0 ? `${hours} h${minutes % 60 ? ` ${minutes % 60} min` : ""}` : `${minutes} min`;
  const date = new Date(ms);
  const time = date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });
  const reset = minutes < 1440 ? time : minutes < 10080 ? `${date.toLocaleDateString(undefined, { weekday: "short" })} ${time}` : formatDate(ms, now);
  return `resets ${reset} · in ${duration}`;
}

export function limitIsStale(limit: QuotaLimit, fetchedAt: number, now: number): boolean {
  return limit.resets_at_ms !== null && limit.resets_at_ms <= now && limit.resets_at_ms > fetchedAt;
}

function windowDuration(window: string | null): number {
  if (window === "weekly") return 7 * 86400;
  if (window === "monthly") return 30 * 86400;
  const match = window?.match(/^(\d+)([smhdw])$/);
  if (!match) return Infinity;
  const seconds: Record<string, number> = { s: 1, m: 60, h: 3600, d: 86400, w: 604800 };
  return Number(match[1]) * seconds[match[2]];
}

export interface Pace {
  /** Fraction of the window already elapsed: where an even burn would be now. */
  expected: number;
  /** Time until the limit is hit at the current burn rate, only when that is before the reset. */
  runsOutMs: number | null;
}

// Needs a known window length and reset. The tick shows from 5% elapsed; a time-to-exhaustion is only
// projected from 20% elapsed, since the burn rate over a small slice of the window is too noisy.
const MIN_ELAPSED = 0.05, MIN_PROJECT = 0.2;

export function limitPace(limit: QuotaLimit, used: number | null, now: number): Pace | null {
  const length = windowDuration(limit.window) * 1000;
  if (used === null || limit.resets_at_ms === null || !Number.isFinite(length) || limit.resets_at_ms <= now) return null;
  const remaining = limit.resets_at_ms - now;
  const expected = clamp(1 - remaining / length);
  if (expected < MIN_ELAPSED) return null;
  const toExhaust = used > 0 && used < 1 ? (1 - used) * expected * length / used : null;
  return { expected, runsOutMs: expected >= MIN_PROJECT && toExhaust !== null && toExhaust < remaining ? toExhaust : null };
}

export function formatSpan(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / 60_000));
  if (minutes >= 1440) return `${Math.floor(minutes / 1440)}d${Math.floor(minutes % 1440 / 60) ? ` ${Math.floor(minutes % 1440 / 60)}h` : ""}`;
  if (minutes >= 60) return `${Math.floor(minutes / 60)}h${minutes % 60 ? ` ${minutes % 60}m` : ""}`;
  return `${minutes}m`;
}

export function compareWindows(a: QuotaLimit, b: QuotaLimit): number {
  const durationA = windowDuration(a.window), durationB = windowDuration(b.window);
  if (durationA !== durationB) return durationA < durationB ? -1 : 1;
  return Number(a.tier !== null) - Number(b.tier !== null);
}

export interface WindowSegment {
  key: string;
  label: string;
  limit: QuotaLimit;
  used: number | null;
  text: string;
  tone: UsageTone | null;
  stale: boolean;
  pace: Pace | null;
}

export function providerView(provider: QuotaProviderStatus, now: number, offline: boolean): ProviderView {
  const stale = offline || provider.stale || provider.error !== null;
  const groups = new Map<string, WindowSegment>();
  for (const account of provider.accounts) {
    for (const limit of account.limits) {
      const key = JSON.stringify([limit.tier, limit.window]);
      const used = usedFraction(limit);
      const best = groups.get(key);
      if (best) {
        const better = used !== null && (best.used === null || used > best.used);
        const earlierReset = used === best.used && (limit.resets_at_ms ?? Infinity) < (best.limit.resets_at_ms ?? Infinity);
        if (!better && !earlierReset) continue;
      }
      const segmentStale = stale || limitIsStale(limit, account.fetched_at_ms, now);
      groups.set(key, {
        key, label: "", limit, used, text: formatLimitValue(limit), tone: usageTone(used),
        stale: segmentStale, pace: segmentStale ? null : limitPace(limit, used, now),
      });
    }
  }
  const segments = [...groups.values()].sort((a, b) => compareWindows(a.limit, b.limit));
  for (const segment of segments) {
    const disambiguate = segments.some(other => other !== segment && other.limit.window === segment.limit.window);
    segment.label = windowLabel(segment.limit.window, disambiguate ? segment.limit.tier : null);
  }
  return { provider, segments, stale, name: providerName(provider.provider) };
}

export interface ProviderView {
  provider: QuotaProviderStatus;
  segments: WindowSegment[];
  stale: boolean;
  name: string;
}

export function providerViews(snapshot: QuotaStatusResponse | null, now: number, offline: boolean): ProviderView[] {
  return PROVIDER_ORDER.map(provider => providerView(snapshot?.providers.find(status => status.provider === provider) ?? {
    provider, state: "pending", error: null, fetched_at_ms: null, stale: false, accounts: [],
  }, now, offline));
}

export function summaryView(views: readonly ProviderView[]): ProviderView | null {
  const visible = views.filter(view => view.provider.state !== "not_signed_in");
  const maxUsed = (view: ProviderView, freshOnly: boolean): number => view.segments.reduce((max, segment) =>
    segment.used !== null && (!freshOnly || !segment.stale) ? Math.max(max, segment.used) : max, -1);
  const fresh = visible.filter(view => maxUsed(view, true) >= 0);
  const candidates = fresh.length ? fresh : visible;
  return candidates.reduce<ProviderView | null>((best, view) =>
    !best || maxUsed(view, fresh.length > 0) > maxUsed(best, fresh.length > 0) ? view : best, null);
}

export function accessibleLabel(views: readonly ProviderView[], now: number, offline: boolean): string {
  const parts = views.filter(view => view.provider.state !== "not_signed_in").map(view => {
    const value = view.segments.length
      ? view.segments.map(segment => `${segment.label} ${formatLimitValue(segment.limit, true)}${segment.pace?.runsOutMs != null ? `, runs out in ${formatSpan(segment.pace.runsOutMs)}` : ""}${segment.stale ? ", stale" : ""}`).join(", ")
      : view.provider.state === "unsupported" ? "not reported" : view.provider.state === "pending" ? "reading limits" : "not reported";
    const age = view.provider.fetched_at_ms === null ? "" : `, updated ${formatAgo(view.provider.fetched_at_ms, now)}`;
    return `${view.name}: ${value}${view.stale ? ", stale" : ""}${view.provider.error ? `, ${errorText(view.provider.error)}` : ""}${age}`;
  });
  return `Subscription limits${offline ? ", offline" : ""}: ${parts.join("; ") || "not signed in"}`;
}
