import type {
  ProviderCredentialClearRequest, ProviderCredentialSetRequest, ProviderCredentialStatus, ProviderCredentialStatusList,
} from "../protocol/generated/v1";
import {
  wireProviderCredentialClearRequest, wireProviderCredentialSetRequest, wireProviderCredentialStatus,
  wireProviderCredentialStatusList, type TypedWirePolicy,
} from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { definePolicy, parseWire, rootMessage } from "./wire";

/** Fixed text only: neither request fields nor tokens are included in errors or causes. */
function malformed(what: string): never { throw new CockpitClientError("malformed_response", `Invalid provider credential ${what}`); }
export const EMAIL_COLON_MESSAGE = "Email can't contain a colon";
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;
const bounded = (value: string, maximum: number) => value.length > 0 && new TextEncoder().encode(value).length <= maximum && !CONTROL.test(value);
const providerId = (value: string) => bounded(value, 512);
const message = rootMessage("Invalid provider credential", {
  ProviderCredentialStatus: "status", ProviderCredentialStatusList: "status list",
  ProviderCredentialSetRequest: "request", ProviderCredentialClearRequest: "request",
});
const RULES = {
  exact: new Set(["ProviderCredentialSetRequest", "ProviderCredentialClearRequest"]),
  absent: new Set(["ProviderCredentialStatus.kind"]),
  raw: new Set(["ProviderCredentialStatus.supported_kinds"]),
  lengths: { "ProviderCredentialStatus.supported_kinds": { max: 2 }, "ProviderCredentialStatusList.providers": { max: 256 } },
  order: { ProviderCredentialSetRequest: ["keys", "kind", "provider_id", "token", "username", "check:email_colon", "check:username_valid"] },
  fields: {
    "ProviderCredentialStatus.provider_id": providerId, "ProviderCredentialSetRequest.provider_id": providerId,
    "ProviderCredentialClearRequest.provider_id": providerId,
    "ProviderCredentialSetRequest.token": (value) => bounded(value, 8192) && value.trim().length > 0,
  },
  checks: {
    ProviderCredentialStatus: { consistency: (status) => new Set(status.supported_kinds).size === status.supported_kinds.length
      && (status.state === "stored") === (status.kind !== null)
      && (status.kind === null || status.supported_kinds.includes(status.kind))
      && (status.state === "unsupported") === (status.supported_kinds.length === 0) },
    ProviderCredentialStatusList: { unique: (list) => new Set(list.providers.map((status) => status.provider_id)).size === list.providers.length },
    ProviderCredentialSetRequest: {
      email_colon: (request) => request.kind !== "basic" || request.username == null || !request.username.includes(":"),
      username_valid: (request) => request.kind === "bearer" ? request.username == null : request.username != null && bounded(request.username, 256),
    },
  },
} satisfies TypedWirePolicy;
const CREDENTIAL = definePolicy({ wire: RULES, message: (failure) => failure.refinement === "email_colon" ? EMAIL_COLON_MESSAGE : message(failure) });
export function parseProviderCredentialStatus(value: unknown): ProviderCredentialStatus { return parseWire(value, wireProviderCredentialStatus, CREDENTIAL); }
export function parseProviderCredentialStatusList(value: unknown): ProviderCredentialStatusList { return parseWire(value, wireProviderCredentialStatusList, CREDENTIAL); }
/** A fresh projection never retains the caller's request object. Bearer usernames are omitted, including null. */
export function parseProviderCredentialSetRequest(value: unknown): ProviderCredentialSetRequest {
  const request = parseWire(value, wireProviderCredentialSetRequest, CREDENTIAL);
  return request.kind === "bearer" ? { provider_id: request.provider_id, kind: request.kind, token: request.token }
    : { provider_id: request.provider_id, kind: request.kind, username: request.username, token: request.token };
}
export function parseProviderCredentialClearRequest(value: unknown): ProviderCredentialClearRequest { return parseWire(value, wireProviderCredentialClearRequest, CREDENTIAL); }


/** A set or clear answer is about the provider that was asked. */
export function matchProviderCredential(status: ProviderCredentialStatus, request: { provider_id: string }): ProviderCredentialStatus {
  if (status.provider_id !== request.provider_id) malformed("status identity");
  return status;
}
