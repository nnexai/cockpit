import type { QuickAskAvailability, QuickAskLocation, QuickAskRequest } from "../protocol/generated/v1";
import { wireQuickAskAvailability, wireQuickAskLocation, wireQuickAskRequest, type TypedWirePolicy } from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { constantMessage, definePolicy, parseWire } from "./wire";

const HARNESS_ORDER = ["omp", "pi", "claude", "copilot", "codex", "opencode"] as const;
const RULES = {
  exact: new Set(["QuickAskRequest"]),
  lengths: { "QuickAskAvailability.agents": { min: 6, max: 6 } },
} satisfies TypedWirePolicy;
const POLICY = definePolicy({ wire: RULES, message: constantMessage("Invalid quick ask data") });

export function parseQuickAskRequest(value: unknown): QuickAskRequest {
  return parseWire(value, wireQuickAskRequest, POLICY);
}
export function parseQuickAskAvailability(value: unknown): QuickAskAvailability {
  const availability = parseWire(value, wireQuickAskAvailability, POLICY);
  if (availability.agents.some((agent, index) => agent.harness !== HARNESS_ORDER[index])) {
    throw new CockpitClientError("malformed_response", "Invalid quick ask availability");
  }
  return availability;
}
export function parseQuickAskLocation(value: unknown): QuickAskLocation {
  return parseWire(value, wireQuickAskLocation, POLICY);
}
