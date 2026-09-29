import { useCallback, useMemo, useRef, useState, type ReactNode } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryItemSummary, ProjectProvider, ProviderCredentialStatus } from "../../protocol/generated/v1";
import { jiraAttachmentAccess } from "./libraryState";
import { ProviderCredentialsDialog } from "./ProviderCredentialsDialog";

/** What a Library surface needs to know about provider tokens, and how to open the dialog for one. */
export type ProviderCredentialActions = {
  /** Token state per provider, or null until first read. Presence only, never the token. */
  statuses: readonly ProviderCredentialStatus[] | null;
  /** Reads the states once (the host may prompt to unlock the keyring, so nothing reads them before a surface needs them). */
  ensure: () => void;
  /** Opens the dialog, entering a token for `providerId` first when it has none. */
  open: (providerId: string) => void;
  /** Whether a Jira issue's attachment bytes can be downloaded; null for anything that isn't a Jira issue. */
  attachmentAccess: (item: LibraryItemSummary) => "stored" | "needs_token" | "loading" | null;
};

/**
 * Token state plus the dialog that edits it, for a surface that shows a provider-token entry point. The surface
 * renders `dialog` once and passes `actions` to the components that offer the entry points.
 */
export function useProviderCredentialActions(client: CockpitClient, providers: readonly ProjectProvider[]): { actions: ProviderCredentialActions; dialog: ReactNode } {
  const [statuses, setStatuses] = useState<ProviderCredentialStatus[] | null>(null);
  const [dialogFor, setDialogFor] = useState<string | null>(null);
  const requested = useRef(false);
  const ensure = useCallback(() => {
    if (requested.current) return;
    requested.current = true;
    client.providerCredentials().then((list) => setStatuses(list.providers), () => {
      // A failed read counts as "no token" so the entry point stays reachable; the dialog shows the reason and a later read retries.
      requested.current = false;
      setStatuses((current) => current ?? []);
    });
  }, [client]);
  const open = useCallback((providerId: string) => setDialogFor(providerId), []);
  const actions = useMemo<ProviderCredentialActions>(() => ({
    statuses,
    ensure,
    open,
    attachmentAccess: (item) => jiraAttachmentAccess(item, providers, statuses),
  }), [ensure, open, providers, statuses]);
  const dialog = dialogFor === null ? null : <ProviderCredentialsDialog client={client} focusProviderId={dialogFor}
    onChanged={(status) => setStatuses((current) => current === null ? null : [...current.filter((existing) => existing.provider_id !== status.provider_id), status])}
    onClose={() => setDialogFor(null)} />;
  return { actions, dialog };
}
