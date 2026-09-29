// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ProviderCredentialSetRequest, ProviderCredentialStatus } from "../../protocol/generated/v1";
import { ProviderCredentialsDialog } from "./ProviderCredentialsDialog";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const TOKEN = "s3cr3t-token-VALUE-0123456789";
const providers = [
  { id: "jira", base_url: "https://team.atlassian.net", executable: "/usr/bin/jira" },
  { id: "wiki", base_url: "https://team.atlassian.net/wiki", executable: "confluence" },
  { id: "gitlab", base_url: "https://gitlab.test", executable: "glab" },
];
const status = (id: string, state: ProviderCredentialStatus["state"], kind: ProviderCredentialStatus["kind"] = null): ProviderCredentialStatus => ({
  provider_id: id, state, kind, supported_kinds: id === "gitlab" ? [] : ["bearer", "basic"],
});

const mounted: Array<() => Promise<void>> = [];
afterEach(async () => { for (const unmount of mounted.splice(0)) await unmount(); });

/** A client whose host state changes only through the calls the dialog makes. */
function fakeClient(initial: ProviderCredentialStatus[], options: { rejectSet?: Error } = {}) {
  let statuses = initial;
  const set = vi.fn(async (request: ProviderCredentialSetRequest) => {
    if (options.rejectSet) throw options.rejectSet;
    const next = status(request.provider_id, "stored", request.kind);
    statuses = statuses.map((existing) => existing.provider_id === next.provider_id ? next : existing);
    return next;
  });
  const clear = vi.fn(async (request: { provider_id: string }) => {
    const next = status(request.provider_id, "not_stored");
    statuses = statuses.map((existing) => existing.provider_id === next.provider_id ? next : existing);
    return next;
  });
  const read = vi.fn(async () => ({ providers: statuses }));
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers })),
    providerCredentials: read,
    setProviderCredential: set,
    clearProviderCredential: clear,
  } as unknown as CockpitClient;
  return { client, set, clear, read };
}

async function render(client: CockpitClient, focusProviderId: string | null = null) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(async () => { await act(async () => root.unmount()); host.remove(); });
  const onChanged = vi.fn();
  await act(async () => root.render(<ProviderCredentialsDialog client={client} focusProviderId={focusProviderId} onChanged={onChanged} onClose={vi.fn()} />));
  await flush();
  return { onChanged };
}

async function flush() { for (let i = 0; i < 6; i++) await act(async () => { await Promise.resolve(); }); }
const button = (name: string) => [...document.body.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.textContent === name || node.getAttribute("aria-label") === name);
const tokenInput = () => document.body.querySelector<HTMLInputElement>("input[type='password']")!;
const usernameInput = () => document.body.querySelector<HTMLInputElement>("input[type='text']")!;
async function type(input: HTMLInputElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

it("shows each provider's state and offers a token only where the provider supports one", async () => {
  const { client } = fakeClient([status("jira", "stored", "basic"), status("wiki", "vault_unavailable"), status("gitlab", "unsupported")]);
  await render(client);
  const rows = [...document.body.querySelectorAll("li")].map((row) => row.textContent ?? "");
  expect(rows[0]).toContain("Jira · team.atlassian.net");
  expect(rows[0]).toContain("Token stored · email + token");
  expect(rows[0]).toContain("Replace token…");
  expect(rows[1]).toContain("Confluence · team.atlassian.net");
  expect(rows[1]).toContain("Keyring unavailable – using the confluence login");
  expect(rows[2]).toContain("Not supported");
  expect(rows[2]).not.toContain("token…");
  expect(button("Remove token for Jira · team.atlassian.net")).toBeDefined();
  expect(button("Remove token for Confluence · team.atlassian.net")).toBeUndefined();
});

it("stores a bearer token, then leaves the field empty and the token out of the DOM", async () => {
  const { client, set } = fakeClient([status("jira", "not_stored"), status("wiki", "not_stored"), status("gitlab", "unsupported")]);
  const { onChanged } = await render(client, "jira");
  // The failing provider's form is already open.
  expect(tokenInput()).not.toBeNull();
  expect(button("Save token")!.disabled).toBe(true);
  await type(tokenInput(), TOKEN);
  expect(button("Save token")!.disabled).toBe(false);
  await act(async () => button("Save token")!.click());
  await flush();
  expect(set).toHaveBeenCalledTimes(1);
  expect(set).toHaveBeenCalledWith({ provider_id: "jira", kind: "bearer", token: TOKEN });
  expect(document.body.querySelector("input[type='password']")).toBeNull();
  expect(document.body.innerHTML).not.toContain(TOKEN);
  expect(document.body.textContent).toContain("Token stored · PAT");
  expect(document.body.textContent).toContain("Stored a token for Jira · team.atlassian.net.");
  expect(onChanged).toHaveBeenCalledWith(expect.objectContaining({ provider_id: "jira", state: "stored", kind: "bearer" }));
  // Replacing starts from an empty field too.
  await act(async () => button("Replace token for Jira · team.atlassian.net")!.click());
  expect(tokenInput().value).toBe("");
});

it("empties the field when the host rejects the token, keeps the form open and shows the host's fixed message", async () => {
  const { client, set } = fakeClient([status("jira", "not_stored"), status("wiki", "not_stored"), status("gitlab", "unsupported")],
    { rejectSet: Object.assign(new Error("The credential vault is unavailable"), { code: "credential_vault_unavailable" }) });
  await render(client, "jira");
  await type(tokenInput(), TOKEN);
  await act(async () => button("Save token")!.click());
  await flush();
  expect(set).toHaveBeenCalledTimes(1);
  expect(tokenInput().value).toBe("");
  expect(button("Save token")!.disabled).toBe(true);
  expect(document.body.querySelector("[role='alert']")?.textContent).toBe("The credential vault is unavailable");
  expect(document.body.innerHTML).not.toContain(TOKEN);
  expect(document.body.textContent).toContain("Using the jira login");
});

it("needs a username for the Basic kind before it can be submitted, and sends it with the token", async () => {
  const { client, set } = fakeClient([status("jira", "not_stored"), status("wiki", "not_stored"), status("gitlab", "unsupported")]);
  await render(client, "jira");
  expect(usernameInput()).toBeNull();
  await act(async () => document.body.querySelector<HTMLInputElement>("input[type='radio'][name$='-kind']:not(:checked)")!.click());
  expect(usernameInput()).not.toBeNull();
  await type(tokenInput(), TOKEN);
  expect(button("Save token")!.disabled).toBe(true);
  await type(usernameInput(), "  me@example.com ");
  expect(button("Save token")!.disabled).toBe(false);
  await act(async () => button("Save token")!.click());
  await flush();
  expect(set).toHaveBeenCalledWith({ provider_id: "jira", kind: "basic", username: "me@example.com", token: TOKEN });
  expect(document.body.textContent).toContain("Token stored · email + token");
});

it("asks before removing a token, then shows the provider as not stored", async () => {
  const { client, clear } = fakeClient([status("jira", "stored", "bearer"), status("wiki", "not_stored"), status("gitlab", "unsupported")]);
  await render(client);
  await act(async () => button("Remove token for Jira · team.atlassian.net")!.click());
  expect(clear).not.toHaveBeenCalled();
  expect(document.body.textContent).toContain("Remove this token?");
  await act(async () => button("Keep token")!.click());
  expect(clear).not.toHaveBeenCalled();
  await act(async () => button("Remove token for Jira · team.atlassian.net")!.click());
  await act(async () => button("Remove token")!.click());
  await flush();
  expect(clear).toHaveBeenCalledWith({ provider_id: "jira" });
  expect(document.body.textContent).toContain("Using the jira login");
  expect(document.body.textContent).toContain("Removed the token for Jira · team.atlassian.net. Cockpit uses the jira login again.");
  expect(button("Remove token for Jira · team.atlassian.net")).toBeUndefined();
});

it("reports a failed status read with a retry instead of an empty list", async () => {
  const { client, read } = fakeClient([status("jira", "not_stored")]);
  read.mockRejectedValueOnce(Object.assign(new Error("Provider tokens are not available in this build"), { code: "credentials_unavailable" }));
  await render(client);
  expect(document.body.querySelector("[role='alert']")?.textContent).toContain("Provider tokens are not available in this build");
  // Focus stays inside the dialog, or Esc and Tab would act on the page behind it.
  expect(document.activeElement).toBe(button("Retry"));
  await act(async () => button("Retry")!.click());
  await flush();
  expect(document.body.textContent).toContain("Using the jira login");
});

it("opens a replacement in the kind that is stored", async () => {
  const { client } = fakeClient([status("jira", "stored", "basic"), status("wiki", "stored", "bearer"), status("gitlab", "unsupported")]);
  await render(client);
  await act(async () => button("Replace token for Jira · team.atlassian.net")!.click());
  expect(usernameInput()).not.toBeNull();
  expect(document.activeElement).toBe(usernameInput());
  await act(async () => button("Cancel")!.click());
  await act(async () => button("Replace token for Confluence · team.atlassian.net")!.click());
  expect(usernameInput()).toBeNull();
  expect(document.activeElement).toBe(tokenInput());
});
