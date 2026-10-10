import type { QuotaStatusResponse } from "../protocol/generated/v1";
import { wireQuotaStatusResponse, type TypedWirePolicy } from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { definePolicy, parseWire, constantMessage } from "./wire";

const PROVIDERS = ["codex", "claude", "copilot"] as const;
const STALE_AFTER_MS = 15 * 60 * 1000, RETAIN_FOR_MS = 24 * 60 * 60 * 1000;
const MESSAGE = "Invalid subscription quota response";
function malformed(): never { throw new CockpitClientError("malformed_response", MESSAGE); }
const credits = (value: number | null) => value === null || (value >= 0 && value <= 1e9);
const RULES = {
  lengths: {
    "QuotaStatusResponse.providers": { min: 3, max: 3 },
    "QuotaProviderStatus.accounts": { max: 8 }, "QuotaAccount.limits": { min: 1, max: 24 },
  },
  fields: {
    "QuotaLimit.id": (value) => /^[A-Za-z0-9:._-]{1,64}$/.test(value),
    "QuotaLimit.used_fraction": (value) => value === null || (value >= 0 && value <= 10),
    "QuotaLimit.used": credits, "QuotaLimit.limit": credits, "QuotaLimit.remaining": credits,
    "QuotaLimit.window": (value) => value === null || (value.length <= 16 && /^(?:\d{1,4}[smhdw]|monthly|weekly)$/.test(value)),
    "QuotaLimit.tier": (value) => value === null || ["opus", "sonnet", "haiku", "fable"].includes(value),
  },
  checks: { QuotaLimit: { amounts: (limit) =>
    (limit.unit !== "percent" || (limit.used === null && limit.limit === null && limit.remaining === null && !limit.unlimited))
    && (!limit.unlimited || (limit.used_fraction === null && limit.used === null && limit.limit === null && limit.remaining === null)) } },
} satisfies TypedWirePolicy;
const QUOTA = definePolicy({ wire: RULES, message: constantMessage(MESSAGE) });

/** Projects unknown metadata away; root-only association rules never infer identities from source text. */
export function parseQuotaStatusResponse(value: unknown): QuotaStatusResponse {
  const response = parseWire(value, wireQuotaStatusResponse, QUOTA);
  response.providers.forEach((provider, index) => {
    if (provider.provider !== PROVIDERS[index]) malformed();
    const fetched = provider.accounts.length ? Math.min(...provider.accounts.map((account) => account.fetched_at_ms)) : null;
    if ((provider.state === "available") !== (provider.accounts.length > 0) || provider.fetched_at_ms !== fetched
      || (provider.state === "pending" && provider.error !== null) || (provider.state === "unavailable" && provider.error === null)
      || provider.stale !== (fetched !== null && (provider.error !== null || response.generated_at_ms - fetched > STALE_AFTER_MS))) malformed();
    for (const account of provider.accounts) {
      if (account.fetched_at_ms > response.generated_at_ms + 60_000 || response.generated_at_ms - account.fetched_at_ms > RETAIN_FOR_MS) malformed();
      if (account.limits.some((limit) => limit.unit !== (provider.provider === "copilot" ? "credits" : "percent"))) malformed();
    }
  });
  return response;
}
