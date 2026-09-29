import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { createPortal } from "react-dom";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ProjectProvider, ProviderAuthKind, ProviderCredentialStatus } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { LibraryConfirmDialog, trapDialogKeys, useRestoreFocus } from "./LibraryConfirmDialog";
import { StatePill } from "./StatePill";
import { confluenceSite, errorText, executableName, instanceHost, providerFamily, type StateShape, type StateTone } from "./libraryState";
import "../projects/setup.css";
import "../projects/taskSetup.css";
import "./library.css";

const KIND_LABEL: Record<ProviderAuthKind, string> = {
  bearer: "Personal access token (Bearer)",
  basic: "Email and API token (Basic)",
};

/** The state pill of one provider row (design D12). `cli` is the provider CLI whose own login applies without a token. */
export function credentialPill(status: ProviderCredentialStatus | undefined, cli: string): { shape: StateShape; tone: StateTone; word: string } {
  switch (status?.state) {
    case "stored": return { shape: "check", tone: "idle", word: status.kind === "basic" ? "Token stored · email + token" : "Token stored · PAT" };
    case "not_stored": return { shape: "dot-ring", tone: "muted", word: `Using the ${cli} login` };
    case "vault_unavailable": return { shape: "close", tone: "blocked", word: `Keyring unavailable – using the ${cli} login` };
    case "unsupported": return { shape: "slash-ring", tone: "muted", word: "Not supported" };
    default: return { shape: "slash-ring", tone: "muted", word: "Status unknown" };
  }
}

type Load =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; providers: ProjectProvider[]; statuses: ProviderCredentialStatus[] };

/** The open entry form. The token itself lives only in the password input's DOM value, never here. */
type Editing = { providerId: string; kind: ProviderAuthKind; username: string };

function providerLabel(provider: ProjectProvider, providers: readonly ProjectProvider[]): string {
  const family = providerFamily(providers, provider.id);
  return `${family.name} · ${family.key === "confluence" ? confluenceSite(provider.base_url) : instanceHost(provider.base_url)}`;
}

/**
 * Write-only provider tokens (design D12): each configured provider shows whether Cockpit holds a token for its
 * site in the OS keyring, and can store, replace or remove one. Nothing here can read a token back. The entry
 * field is uncontrolled: its value is read once on submit and the field is emptied right away, whether the call
 * succeeds or fails, so a token is never in component state, restored state, a URL or an error message.
 */
export function ProviderCredentialsDialog({ client, focusProviderId = null, onChanged, onClose }: {
  client: CockpitClient;
  /** The provider to enter a token for first, when it has none stored. */
  focusProviderId?: string | null;
  /** A store or remove was accepted; carries the provider's new status (never the token). */
  onChanged?: (status: ProviderCredentialStatus) => void;
  onClose: () => void;
}) {
  const titleId = useId();
  const formId = useId();
  const tokenRef = useRef<HTMLInputElement>(null);
  const usernameRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLElement>(null);
  const mounted = useRef(true);
  const [load, setLoad] = useState<Load>({ status: "loading" });
  const [reload, setReload] = useState(0);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [hasToken, setHasToken] = useState(false);
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [removing, setRemoving] = useState<ProjectProvider | null>(null);
  useRestoreFocus();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    let current = true;
    setLoad({ status: "loading" });
    Promise.all([client.projectConfiguration(), client.providerCredentials()]).then(([configuration, list]) => {
      if (!current) return;
      setLoad({ status: "ready", providers: configuration.providers, statuses: list.providers });
      const focus = focusProviderId === null ? undefined : list.providers.find((status) => status.provider_id === focusProviderId);
      if (focus && focus.supported_kinds.length > 0 && focus.state !== "stored") setEditing({ providerId: focus.provider_id, kind: focus.supported_kinds[0]!, username: "" });
    }, (cause: unknown) => {
      if (current) setLoad({ status: "error", message: errorText(cause, "Provider tokens could not be read.") });
    });
    return () => { current = false; };
  }, [client, focusProviderId, reload]);
  const editingId = editing?.providerId;
  // Focus lands where the user acts next and always inside the dialog, so Esc and Tab reach it: the open form's first field, else the first row action, else (while reading, after a read failure, or with nothing to act on) the Retry, Close or Done button.
  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    const first = (selector: string) => dialog.querySelector<HTMLElement>(selector);
    const target = load.status === "loading" ? first(".task-setup-close")
      : load.status === "error" ? first(".library-refusal button")
      : editingId ? (usernameRef.current ?? tokenRef.current) : (first(".library-credentials-actions button") ?? first(".task-setup-footer button"));
    target?.focus();
  }, [load.status, editingId]);
  const accept = (status: ProviderCredentialStatus) => {
    setLoad((current) => current.status === "ready"
      ? { ...current, statuses: [...current.statuses.filter((existing) => existing.provider_id !== status.provider_id), status] }
      : current);
    onChanged?.(status);
  };
  const open = (status: ProviderCredentialStatus) => {
    setNotice(null);
    setFormError(null);
    setHasToken(false);
    setEditing({ providerId: status.provider_id, kind: status.kind && status.supported_kinds.includes(status.kind) ? status.kind : status.supported_kinds.includes("bearer") ? "bearer" : status.supported_kinds[0]!, username: "" });
  };
  const cancel = () => {
    if (tokenRef.current) tokenRef.current.value = "";
    setHasToken(false);
    setFormError(null);
    setEditing(null);
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const input = tokenRef.current;
    if (!editing || !input || saving) return;
    const token = input.value.trim();
    // Emptied before the request is dispatched: success and failure both leave nothing behind.
    input.value = "";
    setHasToken(false);
    setFormError(null);
    setSaving(true);
    const provider = load.status === "ready" ? load.providers.find((candidate) => candidate.id === editing.providerId) : undefined;
    const request = editing.kind === "basic"
      ? { provider_id: editing.providerId, kind: editing.kind, username: editing.username.trim(), token }
      : { provider_id: editing.providerId, kind: editing.kind, token };
    client.setProviderCredential(request).then((status) => {
      if (!mounted.current) return;
      accept(status);
      setEditing(null);
      setNotice(status.state === "stored" ? `Stored a token for ${provider && load.status === "ready" ? providerLabel(provider, load.providers) : status.provider_id}.` : `The token for ${status.provider_id} isn't stored.`);
    }, (cause: unknown) => {
      if (mounted.current) setFormError(errorText(cause, "The token could not be stored."));
    }).finally(() => { if (mounted.current) setSaving(false); });
  };
  const remove = async (provider: ProjectProvider) => {
    const status = await client.clearProviderCredential({ provider_id: provider.id });
    accept(status);
    setRemoving(null);
    if (editing?.providerId === provider.id) cancel();
    setNotice(`Removed the token for ${load.status === "ready" ? providerLabel(provider, load.providers) : provider.id}. Cockpit uses the ${executableName(provider.executable)} login again.`);
  };
  const canSave = hasToken && !saving && editing !== null && (editing.kind === "bearer" || editing.username.trim().length > 0);
  // On the body: a Context viewer is a size container and would clip a fixed overlay to its pane.
  return createPortal(<div className="setup-overlay library-dialog-overlay" role="presentation">
    <section ref={dialogRef} className="setup-dialog task-setup library-credentials" role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={(event) => trapDialogKeys(event, onClose)}>
      <header className="task-setup-header"><h2 id={titleId}>Provider tokens</h2><button type="button" className="task-setup-close" onClick={onClose} aria-label="Close provider tokens"><UiIcon name="close" /></button></header>
      <div className="task-setup-body">
        <p className="task-setup-note library-credentials-intro">A stored token is kept in this computer's keyring and can't be read back, only replaced or removed. Without one, Cockpit uses each provider CLI's own login.</p>
        {load.status === "loading" ? <p className="task-setup-note" role="status">Reading provider tokens…</p> : null}
        {load.status === "error" ? <div className="library-refusal" role="alert">
          <strong>✕ Provider tokens unavailable</strong>
          <span>{load.message}</span>
          <button type="button" className="task-setup-link" onClick={() => setReload((value) => value + 1)}>Retry</button>
        </div> : null}
        {load.status === "ready" && load.providers.length === 0 ? <p className="task-setup-note">No providers are configured. Add one to the Cockpit configuration file first.</p> : null}
        {load.status === "ready" ? <ul className="library-credentials-list" aria-label="Providers">
          {load.providers.map((provider) => {
            const status = load.statuses.find((candidate) => candidate.provider_id === provider.id);
            const pill = credentialPill(status, executableName(provider.executable));
            const label = providerLabel(provider, load.providers);
            const supported = (status?.supported_kinds.length ?? 0) > 0;
            const form = editing?.providerId === provider.id ? editing : null;
            return <li key={provider.id} className="library-credentials-row">
              <div className="library-credentials-head">
                <span className="library-credentials-name" title={provider.base_url}>{label}</span>
                <StatePill shape={pill.shape} tone={pill.tone} word={pill.word} />
                <span className="context-toolbar-spacer" />
                <span className="library-credentials-actions">
                  {supported && !form ? <button type="button" className="library-button is-panel" aria-label={`${status?.state === "stored" ? "Replace" : "Store"} token for ${label}`} onClick={() => open(status!)}>{status?.state === "stored" ? "Replace token…" : "Store token…"}</button> : null}
                  {status?.state === "stored" ? <button type="button" className="library-button is-panel" aria-label={`Remove token for ${label}`} onClick={() => { setNotice(null); setRemoving(provider); }}>Remove</button> : null}
                </span>
              </div>
              {form && status ? <form id={formId} className="library-credentials-form" aria-label={`Token for ${label}`} onSubmit={submit}>
                {status.supported_kinds.length > 1 ? <div role="radiogroup" aria-label="Token type" className="library-destination-choices">
                  {status.supported_kinds.map((kind) => <label key={kind} className="task-setup-check">
                    <input type="radio" name={`${formId}-kind`} checked={form.kind === kind} disabled={saving} onChange={() => setEditing({ ...form, kind })} /> {KIND_LABEL[kind]}
                  </label>)}
                </div> : <p className="task-setup-note">{KIND_LABEL[form.kind]}</p>}
                {form.kind === "basic" ? <div className="task-setup-row">
                  <label htmlFor={`${formId}-username`}>Email</label>
                  <div><input id={`${formId}-username`} ref={usernameRef} type="text" autoComplete="off" spellCheck={false} value={form.username} disabled={saving} onChange={(event) => setEditing({ ...form, username: event.target.value })} /></div>
                </div> : null}
                <div className="task-setup-row">
                  <label htmlFor={`${formId}-token`}>{form.kind === "basic" ? "API token" : "Token"}</label>
                  <div><input id={`${formId}-token`} ref={tokenRef} type="password" autoComplete="off" spellCheck={false} disabled={saving} onInput={(event) => setHasToken(event.currentTarget.value.length > 0)} /></div>
                </div>
                {formError ? <p className="task-setup-note is-error" role="alert">{formError}</p> : null}
                <div className="library-credentials-buttons">
                  <button type="button" onClick={cancel} disabled={saving}>Cancel</button>
                  <button type="submit" className="setup-primary" disabled={!canSave}>{saving ? "Saving…" : "Save token"}</button>
                </div>
              </form> : null}
            </li>;
          })}
        </ul> : null}
        {notice ? <p className="task-setup-note is-valid" role="status">{notice}</p> : null}
      </div>
      <footer className="task-setup-footer"><button type="button" className="setup-primary" onClick={onClose}>Done</button></footer>
    </section>
    {removing && load.status === "ready" ? <LibraryConfirmDialog title="Remove this token?" safeLabel="Keep token" confirmLabel="Remove token" destructive
      body={<p>{`Deletes the token Cockpit stored for ${providerLabel(removing, load.providers)} from the keyring. ${providerFamily(load.providers, removing.id).name} isn't changed, and Cockpit uses the ${executableName(removing.executable)} login again.`}</p>}
      onConfirm={() => remove(removing)} onClose={() => setRemoving(null)} /> : null}
  </div>, document.body);
}
