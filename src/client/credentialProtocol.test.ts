import { describe, expect, it, vi } from "vitest";
import { createBrowserClient } from "./browser";
import { CockpitClientError } from "./CockpitClient";
import {
  matchProviderCredential, parseProviderCredentialClearRequest, parseProviderCredentialSetRequest, parseProviderCredentialStatus,
  parseProviderCredentialStatusList,
} from "./credentialProtocol";
import { createNativeClient } from "./native";

const TOKEN = "s3cr3t-token-VALUE-0123456789";
const stored = { provider_id: "jira", state: "stored", kind: "basic", supported_kinds: ["bearer", "basic"] };
const notStored = { provider_id: "jira", state: "not_stored", kind: null, supported_kinds: ["bearer", "basic"] };

async function rejection(promise: Promise<unknown>): Promise<CockpitClientError> {
  try { await promise; } catch (error) { return error as CockpitClientError; }
  throw new Error("expected a rejection");
}

/** Every failure must be a fixed message that never quotes what was sent. */
function failure(run: () => unknown): CockpitClientError {
  try { run(); } catch (error) { expect(error).toBeInstanceOf(CockpitClientError); return error as CockpitClientError; }
  throw new Error("expected a rejection");
}

describe("parseProviderCredentialStatus", () => {
  it("accepts each state with its kind", () => {
    expect(parseProviderCredentialStatus(stored).kind).toBe("basic");
    expect(parseProviderCredentialStatus(notStored).kind).toBeNull();
    expect(parseProviderCredentialStatus({ provider_id: "gitlab", state: "unsupported", kind: null, supported_kinds: [] }).state).toBe("unsupported");
    expect(parseProviderCredentialStatus({ ...notStored, state: "vault_unavailable" }).state).toBe("vault_unavailable");
  });

  it.each([
    ["a kind on a token that isn't stored", { ...notStored, kind: "bearer" }],
    ["a stored token with no kind", { ...stored, kind: null }],
    ["a stored kind the provider doesn't support", { ...stored, supported_kinds: ["bearer"] }],
    ["unsupported with supported kinds", { ...notStored, state: "unsupported" }],
    ["no supported kinds without unsupported", { ...notStored, supported_kinds: [] }],
    ["an unknown state", { ...notStored, state: "locked" }],
    ["a repeated kind", { ...notStored, supported_kinds: ["bearer", "bearer"] }],
    ["an empty provider id", { ...notStored, provider_id: "" }],
    ["a control character in the provider id", { ...notStored, provider_id: "ji\nra" }],
  ])("rejects %s", (_name, value) => {
    expect(failure(() => parseProviderCredentialStatus(value)).code).toBe("malformed_response");
  });

  it("rejects a status list that names one provider twice", () => {
    expect(failure(() => parseProviderCredentialStatusList({ providers: [notStored, notStored] })).code).toBe("malformed_response");
    expect(parseProviderCredentialStatusList({ providers: [notStored] }).providers).toHaveLength(1);
  });

  it("refuses an answer about another provider", () => {
    expect(failure(() => matchProviderCredential(parseProviderCredentialStatus(stored), { provider_id: "wiki" })).code).toBe("malformed_response");
  });
});

describe("parseProviderCredentialSetRequest", () => {
  it("returns a bearer request without a username and a basic request with one", () => {
    expect(parseProviderCredentialSetRequest({ provider_id: "jira", kind: "bearer", token: TOKEN })).toEqual({ provider_id: "jira", kind: "bearer", token: TOKEN });
    expect(parseProviderCredentialSetRequest({ provider_id: "jira", kind: "basic", username: "me@example.com", token: TOKEN }))
      .toEqual({ provider_id: "jira", kind: "basic", username: "me@example.com", token: TOKEN });
  });

  it.each([
    ["a basic request without a username", { provider_id: "jira", kind: "basic", token: TOKEN }],
    ["a bearer request with a username", { provider_id: "jira", kind: "bearer", username: "me", token: TOKEN }],
    ["a username with a colon", { provider_id: "jira", kind: "basic", username: "a:b", token: TOKEN }],
    ["a token with a newline", { provider_id: "jira", kind: "bearer", token: `${TOKEN}\n` }],
    ["a token with a NUL", { provider_id: "jira", kind: "bearer", token: `${TOKEN}\u0000` }],
    ["a blank token", { provider_id: "jira", kind: "bearer", token: "   " }],
    ["an empty token", { provider_id: "jira", kind: "bearer", token: "" }],
    ["a token over 8192 bytes", { provider_id: "jira", kind: "bearer", token: "é".repeat(4097) }],
    ["an unknown field", { provider_id: "jira", kind: "bearer", token: TOKEN, extra: 1 }],
    ["an unknown kind", { provider_id: "jira", kind: "oauth", token: TOKEN }],
  ])("rejects %s without quoting the token", (_name, value) => {
    const error = failure(() => parseProviderCredentialSetRequest(value));
    expect(error.code).toBe("malformed_response");
    expect(`${error.message} ${JSON.stringify(error)} ${String(error.cause)}`).not.toContain(TOKEN);
    expect(error.message).not.toContain("oauth");
  });

  it("accepts a token of exactly 8192 bytes", () => {
    expect(parseProviderCredentialSetRequest({ provider_id: "jira", kind: "bearer", token: "a".repeat(8192) }).token).toHaveLength(8192);
  });

  it("validates a clear request", () => {
    expect(parseProviderCredentialClearRequest({ provider_id: "jira" })).toEqual({ provider_id: "jira" });
    expect(failure(() => parseProviderCredentialClearRequest({ provider_id: "jira", token: TOKEN })).code).toBe("malformed_response");
  });
});

describe("adapters", () => {
  it("native: calls the fixed commands, sends the token once, and never puts it in an error", async () => {
    const invoke = vi.fn(async (command: string) => command === "cockpit_provider_credentials" ? { providers: [notStored] } : stored);
    const client = createNativeClient(invoke);
    expect((await client.providerCredentials()).providers).toEqual([notStored]);
    await client.setProviderCredential({ provider_id: "jira", kind: "basic", username: "me", token: TOKEN });
    await client.clearProviderCredential({ provider_id: "jira" });
    expect(invoke.mock.calls.map(([command]) => command)).toEqual(["cockpit_provider_credentials", "cockpit_provider_credential_set", "cockpit_provider_credential_clear"]);
    expect(invoke).toHaveBeenNthCalledWith(2, "cockpit_provider_credential_set", { request: { provider_id: "jira", kind: "basic", username: "me", token: TOKEN } });

    const rejected = createNativeClient(async () => { throw { code: "credential_vault_unavailable", message: "The credential vault is unavailable" }; });
    const error = await rejection(rejected.setProviderCredential({ provider_id: "jira", kind: "bearer", token: TOKEN }));
    expect(error.operationCode).toBe("credential_vault_unavailable");
    expect(error.message).toBe("The credential vault is unavailable");
    expect(`${error.message} ${error.code}`).not.toContain(TOKEN);
  });

  it("native: refuses an invalid request before invoking, and an answer about another provider", async () => {
    const invoke = vi.fn(async () => ({ ...stored, provider_id: "wiki" }));
    const client = createNativeClient(invoke);
    await expect(client.setProviderCredential({ provider_id: "jira", kind: "basic", token: TOKEN })).rejects.toBeInstanceOf(CockpitClientError);
    expect(invoke).not.toHaveBeenCalled();
    await expect(client.setProviderCredential({ provider_id: "jira", kind: "bearer", token: TOKEN })).rejects.toMatchObject({ code: "malformed_response" });
  });

  it("browser: calls the fixed routes with a JSON body and reports a rejection without the token", async () => {
    const request = vi.fn(async (path: string, _init?: RequestInit) => new Response(JSON.stringify(path.endsWith("provider-credentials") ? { providers: [notStored] } : stored)));
    const client = createBrowserClient(request);
    expect((await client.providerCredentials()).providers).toEqual([notStored]);
    await client.setProviderCredential({ provider_id: "jira", kind: "bearer", token: TOKEN });
    await client.clearProviderCredential({ provider_id: "jira" });
    expect(request.mock.calls.map(([path]) => path)).toEqual(["/api/v1/provider-credentials", "/api/v1/provider-credentials/set", "/api/v1/provider-credentials/clear"]);
    expect(request.mock.calls[1]![1]).toMatchObject({ method: "POST", body: JSON.stringify({ provider_id: "jira", kind: "bearer", token: TOKEN }) });

    const rejecting = createBrowserClient(async () => new Response(JSON.stringify({ code: "invalid_credential_request", message: "The token must be between 1 and 8192 bytes" }), { status: 400 }));
    const error = await rejection(rejecting.setProviderCredential({ provider_id: "jira", kind: "bearer", token: TOKEN }));
    expect(error.operationCode).toBe("invalid_credential_request");
    expect(`${error.message} ${JSON.stringify(error)}`).not.toContain(TOKEN);
  });
});
