import type {
  QuotaAccount, QuotaErrorCode, QuotaLevel, QuotaLimit, QuotaProvider, QuotaProviderState,
  QuotaProviderStatus, QuotaStatusResponse,
} from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";

const PROVIDERS: readonly QuotaProvider[] = ["codex", "claude", "copilot"];
const STATES: readonly QuotaProviderState[] = ["pending", "available", "not_signed_in", "unsupported", "unavailable"];
const ERRORS: readonly QuotaErrorCode[] = ["source_missing", "not_signed_in", "usage_unavailable", "unsupported", "failed", "timeout", "malformed", "cache_unavailable"];
const LEVELS: readonly QuotaLevel[] = ["ok", "warning", "exhausted", "unknown"];
const STALE_AFTER_MS = 15 * 60 * 1000;
const RETAIN_FOR_MS = 24 * 60 * 60 * 1000;

// Fixed text only: no source fields, identities or command output enter an error.
function malformed(): never {
  throw new CockpitClientError("malformed_response", "Invalid subscription quota response");
}

const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const timestamp = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const member = <T extends string>(value: unknown, choices: readonly T[]): value is T => typeof value === "string" && choices.includes(value as T);

function nullableNumber(value: unknown, maximum: number): number | null {
  if (value === null) return null;
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0 || value > maximum) malformed();
  return value;
}

function nullableToken(value: unknown, maximumLength: number): string | null {
  if (value === null) return null;
  if (typeof value !== "string" || value.length < 1 || value.length > maximumLength || !/^[A-Za-z0-9._-]+$/.test(value)) malformed();
  return value;
}

function parseLimit(value: unknown, provider: QuotaProvider): QuotaLimit {
  const unit = provider === "copilot" ? "credits" : "percent";
  if (!record(value) || typeof value.id !== "string" || !/^[A-Za-z0-9:._-]{1,64}$/.test(value.id)
    || value.unit !== unit || typeof value.unlimited !== "boolean"
    || !member(value.level, LEVELS) || (value.resets_at_ms !== null && !timestamp(value.resets_at_ms))) malformed();
  const used_fraction = nullableNumber(value.used_fraction, 10);
  const used = nullableNumber(value.used, 1e9);
  const limit = nullableNumber(value.limit, 1e9);
  const remaining = nullableNumber(value.remaining, 1e9);
  if (value.unit === "percent" && (used !== null || limit !== null || remaining !== null || value.unlimited)) malformed();
  if (value.unlimited && (used_fraction !== null || used !== null || limit !== null || remaining !== null)) malformed();
  const window = nullableToken(value.window, 16);
  const tier = nullableToken(value.tier, 32);
  if (window !== null && !/^(?:\d{1,4}[smhdw]|monthly|weekly)$/.test(window)) malformed();
  if (tier !== null && !["opus", "sonnet", "haiku", "fable"].includes(tier)) malformed();
  return {
    id: value.id,
    window,
    tier,
    unit,
    used_fraction, used, limit, remaining,
    unlimited: value.unlimited,
    level: value.level,
    resets_at_ms: value.resets_at_ms as number | null,
  };
}

function parseAccount(value: unknown, provider: QuotaProvider, generatedAt: number): QuotaAccount {
  if (!record(value) || !timestamp(value.fetched_at_ms) || value.fetched_at_ms > generatedAt + 60_000
    || generatedAt - value.fetched_at_ms > RETAIN_FOR_MS || !Array.isArray(value.limits)
    || value.limits.length < 1 || value.limits.length > 24) malformed();
  return { fetched_at_ms: value.fetched_at_ms, limits: value.limits.map((limit) => parseLimit(limit, provider)) };
}

function parseProvider(value: unknown, provider: QuotaProvider, generatedAt: number): QuotaProviderStatus {
  if (!record(value) || value.provider !== provider || !member(value.state, STATES)
    || (value.error !== null && !member(value.error, ERRORS)) || typeof value.stale !== "boolean"
    || (value.fetched_at_ms !== null && !timestamp(value.fetched_at_ms))
    || !Array.isArray(value.accounts) || value.accounts.length > 8) malformed();
  const accounts = value.accounts.map((account) => parseAccount(account, provider, generatedAt));
  const fetched_at_ms = accounts.length ? Math.min(...accounts.map((account) => account.fetched_at_ms)) : null;
  const error = value.error as QuotaErrorCode | null;
  if ((value.state === "available") !== (accounts.length > 0) || value.fetched_at_ms !== fetched_at_ms
    || (value.state === "pending" && error !== null) || (value.state === "unavailable" && error === null)
    || value.stale !== (fetched_at_ms !== null && (error !== null || generatedAt - fetched_at_ms > STALE_AFTER_MS))) malformed();
  return { provider, state: value.state, error, fetched_at_ms, stale: value.stale, accounts };
}

/** Validates and projects the identity-free quota contract shared by HTTP and native hosts. */
export function parseQuotaStatusResponse(value: unknown): QuotaStatusResponse {
  if (!record(value) || !timestamp(value.generated_at_ms) || typeof value.collecting !== "boolean"
    || !Array.isArray(value.providers) || value.providers.length !== PROVIDERS.length) malformed();
  const generated_at_ms = value.generated_at_ms;
  return {
    generated_at_ms,
    collecting: value.collecting,
    providers: value.providers.map((provider, index) => parseProvider(provider, PROVIDERS[index], generated_at_ms)),
  };
}
