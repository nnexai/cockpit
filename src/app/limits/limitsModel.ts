import type { QuotaErrorCode, QuotaLevel, QuotaLimit, QuotaProvider, QuotaProviderStatus, QuotaStatusResponse } from "../../protocol/generated/v1";
import { formatAgo, formatDate } from "../library/libraryState";

export const PROVIDER_ORDER: readonly QuotaProvider[] = ["codex", "claude", "copilot"];
const number = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
const clamp = (value: number) => Math.max(0, Math.min(1, value));

export function providerName(provider: string): string {
  return provider.charAt(0).toUpperCase() + provider.slice(1);
}

export function remainingFraction(limit: QuotaLimit): number | null {
  if (limit.unlimited) return null;
  if (limit.used_fraction !== null) return clamp(1 - limit.used_fraction);
  if (limit.unit === "credits" && limit.limit !== null && limit.limit > 0) {
    if (limit.remaining !== null) return clamp(limit.remaining / limit.limit);
    if (limit.used !== null) return clamp(1 - limit.used / limit.limit);
  }
  return null;
}

export function limitingLimit(limits: readonly QuotaLimit[]): QuotaLimit | null {
  let best: QuotaLimit | null = null;
  let fraction = Infinity;
  for (const limit of limits) {
    const value = remainingFraction(limit);
    if (value === null) continue;
    if (value < fraction || (value === fraction && (limit.resets_at_ms ?? Infinity) < (best?.resets_at_ms ?? Infinity))) {
      best = limit;
      fraction = value;
    }
  }
  return best ?? limits.find(limit => limit.unit === "credits" && (limit.remaining !== null || limit.used !== null))
    ?? limits.find(limit => limit.unlimited) ?? null;
}

export function windowLabel(window: string | null, tier: string | null): string {
  const title = (value: string) => /^\d+[smhdw]$/.test(value) ? value : providerName(value.replace(/[-_]/g, " "));
  return [tier && title(tier), window && title(window)].filter(Boolean).join(" ") || "Limit";
}

function creditBalance(limit: QuotaLimit): number | null {
  return limit.remaining ?? (limit.limit !== null && limit.used !== null ? Math.max(0, limit.limit - limit.used) : null);
}

export function formatLimitValue(limit: QuotaLimit | null, detail = false): string {
  if (!limit) return "—";
  if (limit.unlimited) return detail ? "Unlimited" : "unlimited";
  if (limit.unit === "percent") {
    const fraction = remainingFraction(limit);
    if (fraction === null) return "—";
    // A tiny positive balance is not exhausted, and floating point noise must not turn 62% into 61%.
    const percent = fraction === 0 ? "0%" : fraction < 0.01 ? "<1%" : `${Math.floor(fraction * 100 + 1e-9)}%`;
    return detail ? `${percent} left` : percent;
  }
  const balance = creditBalance(limit);
  if (balance === null) return limit.used === null ? "—" : `${number.format(limit.used)} ${detail ? "AI credits used" : "cr used"}`;
  if (detail) return `${number.format(balance)}${limit.limit === null ? "" : ` of ${number.format(limit.limit)}`} AI credits left`;
  return `${balance >= 10_000 ? `${number.format(balance / 1000)}k` : number.format(balance)} cr`;
}

export function errorText(error: QuotaErrorCode | null, provider?: QuotaProvider): string {
  switch (error) {
    case "source_missing": return provider === "copilot" ? "GitHub CLI not found" : "OMP usage source not found";
    case "not_signed_in": return "Not signed in";
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

export function providerView(provider: QuotaProviderStatus, now: number, offline: boolean): ProviderView {
  const limiting = limitingLimit(provider.accounts.flatMap(account => account.limits));
  const account = provider.accounts.find(account => limiting !== null && account.limits.includes(limiting));
  const stale = offline || provider.stale || provider.error !== null || Boolean(limiting?.resets_at_ms !== null && limiting?.resets_at_ms !== undefined && account && limiting.resets_at_ms <= now && limiting.resets_at_ms > account.fetched_at_ms);
  return {
    provider, limiting, stale, fraction: limiting ? remainingFraction(limiting) : null,
    level: limiting?.level ?? "unknown", name: providerName(provider.provider),
    text: provider.state === "pending" ? "…" : provider.state === "unsupported" ? "n/a" : formatLimitValue(limiting),
  };
}

export interface ProviderView {
  provider: QuotaProviderStatus;
  limiting: QuotaLimit | null;
  stale: boolean;
  fraction: number | null;
  level: QuotaLevel;
  name: string;
  text: string;
}

export function providerViews(snapshot: QuotaStatusResponse | null, now: number, offline: boolean): ProviderView[] {
  return PROVIDER_ORDER.map(provider => providerView(snapshot?.providers.find(status => status.provider === provider) ?? {
    provider, state: "pending", error: null, fetched_at_ms: null, stale: false, accounts: [],
  }, now, offline));
}

export function summaryView(views: readonly ProviderView[]): ProviderView | null {
  const visible = views.filter(view => view.provider.state !== "not_signed_in");
  const fresh = visible.filter(view => !view.stale && view.limiting !== null);
  const candidates = fresh.length ? fresh : visible;
  return candidates.reduce<ProviderView | null>((best, view) => view.fraction !== null && (!best || view.fraction < best.fraction!) ? view : best, null) ?? candidates[0] ?? null;
}

export function accessibleLabel(views: readonly ProviderView[], now: number, offline: boolean): string {
  const parts = views.filter(view => view.provider.state !== "not_signed_in").map(view => {
    const value = view.provider.state === "unsupported" ? "not reported" : view.provider.state === "pending" ? "reading limits" : formatLimitValue(view.limiting, true);
    const window = view.limiting ? ` (${windowLabel(view.limiting.window, view.limiting.tier)})` : "";
    const age = view.provider.fetched_at_ms === null ? "" : `, updated ${formatAgo(view.provider.fetched_at_ms, now)}`;
    return `${view.name}: ${value}${window}${view.stale ? ", stale" : ""}${view.provider.error ? `, ${errorText(view.provider.error, view.provider.provider)}` : ""}${age}`;
  });
  return `Subscription limits${offline ? ", offline" : ""}: ${parts.join("; ") || "not signed in"}`;
}
