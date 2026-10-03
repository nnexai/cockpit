import { useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryItemSummary, RepositoryCandidate } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { rankFuzzyMatches } from "../input/fileNavigation";
import { errorText, type LibrarySpace } from "../library/libraryState";
import { SpaceContextList } from "../library/SpaceContextList";
import { trapDialogKeys, useRestoreFocus } from "../library/LibraryConfirmDialog";
import { announceLibraryChanged, type SpaceListingState } from "../library/useLibraryOperation";
import "../projects/setup.css";
import "../projects/taskSetup.css";

function RepositoryDialog({ client, selectedPaths, checkoutPath, onSave, onClose }: {
  client: CockpitClient;
  selectedPaths: readonly string[];
  checkoutPath: string | null;
  onSave: (paths: string[]) => Promise<void>;
  onClose: () => void;
}) {
  const titleId = useId();
  const closeRef = useRef<HTMLButtonElement>(null);
  const [repositories, setRepositories] = useState<RepositoryCandidate[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<Set<string>>(() => new Set());
  const [saving, setSaving] = useState(false);
  useRestoreFocus();
  useEffect(() => { closeRef.current?.focus(); }, []);
  useEffect(() => {
    let current = true;
    setLoading(true); setError(null);
    client.repositories().then((listing) => { if (current) { setRepositories(listing.repositories); setLoading(false); } }, (cause) => { if (current) { setError(errorText(cause, "Repositories could not be read.")); setLoading(false); } });
    return () => { current = false; };
  }, [client, retry]);
  const available = useMemo(() => repositories.filter((repository, index) => repository.checkout_path !== checkoutPath && !selectedPaths.includes(repository.checkout_path) && repositories.findIndex((candidate) => candidate.checkout_path === repository.checkout_path) === index), [repositories, selectedPaths, checkoutPath]);
  const matches = useMemo(() => rankFuzzyMatches(query, available, (repository) => `${repository.name} ${repository.checkout_path}`), [query, available]);
  const save = async () => {
    if (saving || picked.size === 0) return;
    setSaving(true); setError(null);
    try { await onSave([...selectedPaths, ...picked]); onClose(); }
    catch (cause) { setError(errorText(cause, "Repositories could not be selected.")); }
    finally { setSaving(false); }
  };
  return createPortal(<div className="setup-overlay library-dialog-overlay" onKeyDown={(event) => trapDialogKeys(event, onClose)}><section className="setup-dialog task-setup context-repository-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId}>
    <header className="task-setup-header"><h2 id={titleId}>Add repositories</h2><button ref={closeRef} type="button" className="task-setup-close" aria-label="Close Add repositories" onClick={onClose}><UiIcon name="close" /></button></header>
    <div className="task-setup-body"><p className="task-setup-note">Agents read these existing repository paths directly. Nothing is captured or deleted.</p>
      <div className="task-setup-picker"><input type="search" aria-label="Find repositories" placeholder="Type to find a repository" value={query} onChange={(event) => setQuery(event.target.value)} /></div>
      {loading ? <p role="status">Loading repositories…</p> : null}
      {error ? <div className="context-resource-error" role="alert"><span>{error}</span>{!saving ? <button type="button" onClick={() => setRetry((value) => value + 1)}>Retry</button> : null}</div> : null}
      {!loading && matches.length === 0 ? <p className="context-resource-empty">No matching repositories.</p> : null}
      <div className="context-repository-options">{matches.map((repository) => <label key={repository.checkout_path} className="task-setup-check"><input type="checkbox" checked={picked.has(repository.checkout_path)} disabled={saving} onChange={(event) => { const checked = event.target.checked; setPicked((current) => { const next = new Set(current); if (checked) next.add(repository.checkout_path); else next.delete(repository.checkout_path); return next; }); }} /><span><strong>{repository.name}</strong><code title={repository.checkout_path}>{repository.checkout_path}</code></span></label>)}</div>
    </div>
    <footer className="task-setup-footer"><button type="button" onClick={onClose}>Cancel</button><button type="button" className="setup-primary" disabled={saving || loading || picked.size === 0} onClick={() => void save()}>{saving ? "Adding…" : `Add ${picked.size} repositories`}</button></footer>
  </section></div>, document.body);
}

/** The Space's live Library relevance and existing repository paths. */
export function ContextResources({ client, space, spaceListing, onAdd, onClose, onOpenItem, onOpenRepository }: {
  client: CockpitClient;
  space: LibrarySpace | null;
  spaceListing: SpaceListingState;
  onAdd: () => void;
  onClose: () => void;
  onOpenItem?: (item: LibraryItemSummary) => void;
  onOpenRepository?: (path: string) => Promise<void>;
}) {
  const closeRef = useRef<HTMLButtonElement>(null);
  const addRepositoriesRef = useRef<HTMLButtonElement>(null);
  const repositoriesRef = useRef<HTMLUListElement>(null);
  const [repositoriesOpen, setRepositoriesOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useRestoreFocus();
  useEffect(() => { closeRef.current?.focus(); }, []);
  const listing = spaceListing.listing;
  const label = listing?.space_label ?? space?.label ?? "Space";
  const paths = listing?.repository_paths ?? [];
  const replace = async (repositoryPaths: string[]) => {
    if (!space?.live) throw new Error("Herdr isn't live.");
    await client.librarySpaceRepositories({ target: space.target, repository_paths: repositoryPaths });
    announceLibraryChanged(); spaceListing.reload();
  };
  const remove = async (path: string) => {
    if (busy) return;
    const row = [...(repositoriesRef.current?.querySelectorAll<HTMLElement>("li") ?? [])].find((element) => element.title === path);
    const next = row?.nextElementSibling as HTMLElement | null;
    const restore = row?.contains(document.activeElement) ?? false;
    setBusy(path); setError(null);
    try {
      await replace(paths.filter((candidate) => candidate !== path));
      if (restore) requestAnimationFrame(() => { (next?.querySelector<HTMLButtonElement>("button") ?? addRepositoriesRef.current)?.focus({ preventScroll: true }); });
    }
    catch (cause) { setError(`Couldn't remove ${path} from ${label}. ${errorText(cause, "The selection could not be changed.")}`); }
    finally { setBusy(null); }
  };
  const open = async (path: string) => {
    if (!onOpenRepository || busy) return;
    setBusy(path); setError(null);
    try { await onOpenRepository(path); onClose(); }
    catch (cause) { setError(errorText(cause, "This repository could not be opened.")); }
    finally { setBusy(null); }
  };
  return <section className="context-resources" role="dialog" aria-modal="true" aria-label="Context resources" onKeyDown={(event) => trapDialogKeys(event, onClose)}>
    <header><strong>Context resources</strong><button ref={closeRef} type="button" className="context-resources-close" aria-label="Close Context resources" title="Close Context resources" onClick={onClose}><UiIcon name="close" /></button></header>
    <div className="context-resources-body">
      <p className="task-setup-note">Context is read directly from the Library, so Library refreshes show up here immediately. The Library is managed by Cockpit and read-only by convention. Notes live in your checkout.</p>
      {space?.live ? <SpaceContextList client={client} space={space} state={spaceListing} onAdd={onAdd} onOpenItem={onOpenItem} /> : <p className="context-resource-empty space-context-offline">Herdr isn't live, so {label}'s context is Library-only.</p>}
      <section className="context-repositories"><div className="space-context-bar"><strong>Repositories</strong><span className="context-toolbar-spacer" />{space?.live ? <button ref={addRepositoriesRef} type="button" disabled={!listing || busy !== null} onClick={() => setRepositoriesOpen(true)}>Add repositories…</button> : null}</div>
        {error ? <div className="context-resource-error" role="alert">{error}</div> : null}
        {paths.length === 0 ? <p className="context-resource-empty">No extra repositories. The Space's own checkout is always included.</p> : null}
        <ul ref={repositoriesRef} className="context-repos">{paths.map((path) => <li key={path} title={path}>{onOpenRepository ? <button className="context-repo-path" type="button" disabled={busy !== null} onClick={() => void open(path)}>{path}</button> : <code className="context-repo-path">{path}</code>}{listing?.diagnostics.some((diagnostic) => diagnostic.path === path) ? <span className="chip" role="status">Not found</span> : null}{space?.live ? <button type="button" disabled={busy !== null} aria-label={`Remove ${path} from ${label}`} onClick={() => void remove(path)}>{busy === path ? "Working…" : "Remove"}</button> : null}</li>)}</ul>
      </section>
    </div>
    {repositoriesOpen ? <RepositoryDialog client={client} selectedPaths={paths} checkoutPath={listing?.checkout_path ?? null} onSave={replace} onClose={() => setRepositoriesOpen(false)} /> : null}
  </section>;
}
