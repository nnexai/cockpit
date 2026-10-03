import { useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryItemSummary, ProjectProvider } from "../../protocol/generated/v1";
import { errorText, providerFamily, type LibrarySpace } from "./libraryState";
import { announceLibraryChanged, useLibraryOperation, type SpaceListingState } from "./useLibraryOperation";
import "./library.css";

function kindChips(item: LibraryItemSummary, providers: readonly ProjectProvider[]): string[] {
  if (item.kind === "folder_copy") return ["Folder"];
  const family = providerFamily(providers, item.provider_id);
  return [family.name, item.resource_type === "page" ? "page" : family.key !== "jira" && item.resource_type === "review" ? family.review : "issue"];
}

/** A Space selects live Library items; removal changes relevance, never files. */
export function SpaceContextList({ client, space, state, onAdd, onOpenItem }: {
  client: CockpitClient;
  space: LibrarySpace;
  state: SpaceListingState;
  onAdd: () => void;
  onOpenItem?: (item: LibraryItemSummary) => void;
}) {
  const addRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const [providers, setProviders] = useState<ProjectProvider[]>([]);
  const operations = useLibraryOperation(client);
  const [removing, setRemoving] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [removed, setRemoved] = useState<Set<string>>(() => new Set());
  const targetKey = `${space.target.session_id}\0${space.target.space_id}`;
  const currentTarget = useRef(targetKey);
  currentTarget.current = targetKey;
  useEffect(() => { setRemoved(new Set()); setErrors({}); setRemoving(null); }, [targetKey]);
  useEffect(() => { setRemoved(new Set()); }, [state.listing]);
  useEffect(() => {
    let current = true;
    client.projectConfiguration().then((configuration) => { if (current) setProviders(configuration.providers); }, () => undefined);
    return () => { current = false; };
  }, [client]);
  const items = state.listing?.items.filter((item) => !removed.has(item.item_id)) ?? [];
  const label = state.listing?.space_label ?? space.label;
  const remove = async (item: LibraryItemSummary) => {
    if (removing) return;
    const key = targetKey;
    const row = [...(listRef.current?.querySelectorAll<HTMLElement>("[data-space-item]") ?? [])].find((element) => element.dataset.spaceItem === item.item_id);
    const next = row?.nextElementSibling as HTMLElement | null;
    const restore = row?.contains(document.activeElement) ?? false;
    setRemoving(item.item_id);
    setErrors((current) => { const result = { ...current }; delete result[item.item_id]; return result; });
    try {
      await client.librarySpaceRemove({ target: space.target, item_ids: [item.item_id] });
      if (currentTarget.current !== key) return;
      setRemoved((current) => new Set([...current, item.item_id]));
      announceLibraryChanged();
      state.reload();
      if (restore) requestAnimationFrame(() => { (next?.querySelector<HTMLButtonElement>("button") ?? addRef.current)?.focus({ preventScroll: true }); });
    } catch (cause) {
      if (currentTarget.current === key) setErrors((current) => ({ ...current, [item.item_id]: `Couldn't remove ${item.title} from ${label}. ${errorText(cause, "The selection could not be changed.")}` }));
    } finally {
      if (currentTarget.current === key) setRemoving(null);
    }
  };
  return <section className="space-context">
    <div className="space-context-bar"><strong>Library context for {label}</strong><span className="context-toolbar-spacer" /><button ref={addRef} type="button" onClick={onAdd} disabled={!space.live}>Add…</button></div>
    {state.status === "error" ? <div className="context-resource-error" role="alert"><span>{state.error ?? `${label}'s context couldn't be read.`}</span><button type="button" onClick={state.reload}>Retry</button></div> : null}
    {!state.listing && state.status !== "error" ? <p role="status" className="context-resource-loading">Loading…</p> : null}
    {state.listing && items.length === 0 ? <p className="context-resource-empty">Nothing selected for {label} yet. Add Library items to give agents context.</p> : null}
    <ul ref={listRef} className="context-resource-list" aria-label={`Library context in ${label}`} aria-busy={state.status === "loading"}>
      {items.map((item) => <li className="space-context-row" key={item.item_id} data-space-item={item.item_id}>
        {onOpenItem && item.document_path ? <button type="button" className="space-context-title" onClick={() => onOpenItem(item)} title={item.title}>{item.title}</button> : <strong className="space-context-title" title={item.title}>{item.title}</strong>}
        <span className="space-context-chips">{kindChips(item, providers).map((chip) => <span className="chip" key={chip}>{chip}</span>)}<span className="chip" role="status">{operations.pendingItemIds.has(item.item_id) ? "Updating…" : item.state === "failed" || item.state === "conflict" ? "Refresh failed" : "Up to date"}</span></span>
        <button type="button" aria-label={`Remove ${item.title} from ${label}`} disabled={removing !== null || !space.live} onClick={() => void remove(item)}>{removing === item.item_id ? "Removing…" : "Remove from Space"}</button>
        {errors[item.item_id] ? <div className="context-resource-error" role="alert"><span>{errors[item.item_id]}</span><button type="button" disabled={removing !== null} onClick={() => void remove(item)}>Retry</button></div> : null}
      </li>)}
    </ul>
  </section>;
}
