import type { QuickAskActionRequest, QuickAskAvailability, QuickAskStartRequest, QuickAskStatus } from "../protocol/generated/v1";
import { wireQuickAskActionRequest, wireQuickAskAvailability, wireQuickAskStartRequest, wireQuickAskStatus, type TypedWirePolicy } from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { constantMessage, definePolicy, parseWire } from "./wire";

const HARNESS_ORDER = ["omp", "pi", "claude", "copilot", "codex", "opencode"] as const;
const RULES = {
  exact: new Set(["QuickAskStartRequest", "QuickAskActionRequest"]),
  lengths: { "QuickAskAvailability.agents": { min: 6, max: 6 } },
} satisfies TypedWirePolicy;
const POLICY = definePolicy({ wire: RULES, message: constantMessage("Invalid quick ask data") });

export function parseQuickAskStartRequest(value: unknown): QuickAskStartRequest {
  return parseWire(value, wireQuickAskStartRequest, POLICY);
}
export function parseQuickAskActionRequest(value: unknown): QuickAskActionRequest {
  return parseWire(value, wireQuickAskActionRequest, POLICY);
}
export function parseQuickAskAvailability(value: unknown): QuickAskAvailability {
  const availability = parseWire(value, wireQuickAskAvailability, POLICY);
  if (availability.agents.some((agent, index) => agent.harness !== HARNESS_ORDER[index])) {
    throw new CockpitClientError("malformed_response", "Invalid quick ask availability");
  }
  return availability;
}
export function parseQuickAskStatus(value: unknown): QuickAskStatus {
  const status = parseWire(value, wireQuickAskStatus, POLICY);
  if ((status.phase === "submitted") !== (status.delivery === "submitted") ||
      (status.phase === "initial_question_supplied") !== (status.delivery === "initial_question_supplied") ||
      (status.phase === "outcome_unknown") !== (status.delivery === "unknown")) {
    throw new CockpitClientError("malformed_response", "Invalid quick ask status");
  }
  return status;
}
export function matchQuickAskStatus(status: QuickAskStatus, sessionId: string, requestId: string): QuickAskStatus {
  if (status.session_id !== sessionId || status.request_id !== requestId) {
    throw new CockpitClientError("malformed_response", "Quick ask status belongs to another request");
  }
  return status;
}
