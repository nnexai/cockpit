import type {
  ProviderAuthKind, ProviderCredentialClearRequest, ProviderCredentialSetRequest, ProviderCredentialState, ProviderCredentialStatus,
  ProviderCredentialStatusList,
} from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";

// Mirrors the host validation in `cockpit_core::credentials` so an invalid request never leaves the client.
const MAX_PROVIDER_ID_BYTES = 512;
const MAX_TOKEN_BYTES = 8192;
const MAX_USERNAME_BYTES = 256;
const MAX_PROVIDERS = 256;

/** Fixed text only: no message here may quote a request field, above all the token. */
function invalid(message: string): never {
  throw new CockpitClientError("malformed_response", message);
}
function malformed(what: string): never {
  invalid(`Invalid provider credential ${what}`);
}

/** The one request rule a person can break by typing: the host joins the email and token with a colon. */
export const EMAIL_COLON_MESSAGE = "Email can't contain a colon";

const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const utf8Length = (value: string) => new TextEncoder().encode(value).length;
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;

const KINDS: readonly ProviderAuthKind[] = ["bearer", "basic"];
const STATES: readonly ProviderCredentialState[] = ["stored", "not_stored", "vault_unavailable", "unsupported"];
const isKind = (value: unknown): value is ProviderAuthKind => typeof value === "string" && KINDS.includes(value as ProviderAuthKind);
const isState = (value: unknown): value is ProviderCredentialState => typeof value === "string" && STATES.includes(value as ProviderCredentialState);

function providerId(value: unknown, what: string): string {
  if (typeof value !== "string" || value.length === 0 || utf8Length(value) > MAX_PROVIDER_ID_BYTES || CONTROL.test(value)) malformed(what);
  return value as string;
}

export function parseProviderCredentialStatus(value: unknown): ProviderCredentialStatus {
  if (!record(value) || !isState(value.state) || !Array.isArray(value.supported_kinds) || value.supported_kinds.length > KINDS.length
    || !value.supported_kinds.every(isKind) || new Set(value.supported_kinds).size !== value.supported_kinds.length) malformed("status");
  const id = providerId(value.provider_id, "status");
  const state = value.state as ProviderCredentialState;
  const supported = value.supported_kinds as ProviderAuthKind[];
  const kind = value.kind === undefined ? null : value.kind;
  // A kind is known only for a stored token, and only one the provider supports; `unsupported` is exactly the empty support list.
  if ((state === "stored") !== (kind !== null) || (kind !== null && (!isKind(kind) || !supported.includes(kind))) || (state === "unsupported") !== (supported.length === 0)) malformed("status");
  return { provider_id: id, state, kind: kind as ProviderAuthKind | null, supported_kinds: supported };
}

export function parseProviderCredentialStatusList(value: unknown): ProviderCredentialStatusList {
  if (!record(value) || !Array.isArray(value.providers) || value.providers.length > MAX_PROVIDERS) malformed("status list");
  const providers = (value.providers as unknown[]).map(parseProviderCredentialStatus);
  if (new Set(providers.map((status) => status.provider_id)).size !== providers.length) malformed("status list");
  return { providers };
}

/**
 * Validates a set request before it is sent, with the host's rules: a bearer token takes no username; a basic
 * credential needs one (no `:`, no control characters); the token has no control characters and isn't blank.
 * Returns a fresh object so the caller's own copy of the token is not retained by the client.
 */
export function parseProviderCredentialSetRequest(value: unknown): ProviderCredentialSetRequest {
  if (!record(value) || Object.keys(value).some((key) => !["provider_id", "kind", "username", "token"].includes(key)) || !isKind(value.kind)) malformed("request");
  const id = providerId(value.provider_id, "request");
  const token = value.token;
  if (typeof token !== "string" || token.length === 0 || utf8Length(token) > MAX_TOKEN_BYTES || CONTROL.test(token) || token.trim().length === 0) malformed("request");
  const kind = value.kind as ProviderAuthKind;
  const username = value.username;
  if (kind === "bearer") {
    if (username !== undefined && username !== null) malformed("request");
    return { provider_id: id, kind, token: token as string };
  }
  if (typeof username === "string" && username.includes(":")) invalid(EMAIL_COLON_MESSAGE);
  if (typeof username !== "string" || username.length === 0 || utf8Length(username) > MAX_USERNAME_BYTES || CONTROL.test(username)) malformed("request");
  return { provider_id: id, kind, username: username as string, token: token as string };
}

export function parseProviderCredentialClearRequest(value: unknown): ProviderCredentialClearRequest {
  if (!record(value) || Object.keys(value).some((key) => key !== "provider_id")) malformed("request");
  return { provider_id: providerId((value as Record<string, unknown>).provider_id, "request") };
}

/** A set or clear answer is about the provider that was asked. */
export function matchProviderCredential(status: ProviderCredentialStatus, request: { provider_id: string }): ProviderCredentialStatus {
  if (status.provider_id !== request.provider_id) malformed("status identity");
  return status;
}
