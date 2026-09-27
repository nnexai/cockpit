import { Fragment, useState, type ReactNode } from "react";
import type { LibraryItemSummary, ProjectProvider } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { instanceHost, itemDisplayId, itemKindLabel, libraryFreshness, libraryStateChip } from "./libraryState";
import { LibraryMenu, itemMenuEntries, menuAnchor, type LibraryItemActions } from "./LibraryTree";

/**
 * Library item header (design §4.4): kind chip and container path, title,
 * state chip with its freshness phrase and actions, then `Metadata`. S1 has no
 * Space action. At pane width ≤ 520 px `Refresh` moves into `⋯`.
 */
export function LibraryItemHeader({ item, providers, narrow, rootCrumb, pending, actions, onReplace, details }: {
  item: LibraryItemSummary;
  providers: readonly ProjectProvider[];
  narrow: boolean;
  /** Shows the `Library` breadcrumb segment when the Library is one root among several. */
  rootCrumb: boolean;
  pending: boolean;
  actions: LibraryItemActions;
  onReplace: (item: LibraryItemSummary) => void;
  /** The viewer's document details disclosure, kept at the end of line 1. */
  details: ReactNode;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const now = Date.now();
  const chip = libraryStateChip(item.state);
  const freshness = libraryFreshness(item, now);
  const container = item.container?.label ?? null;
  const refresh = () => actions.refresh({ scope: "items", item_ids: [item.item_id] }, [item.item_id]);
  return <div className="library-item-header">
    <div className="library-item-line">
      <span className="document-source-kind library-kind-chip">{itemKindLabel(item, providers)}</span>
      <span className="library-item-path">{rootCrumb ? <><span>Library</span><span aria-hidden="true"> › </span></> : null}{container ?? instanceHost(item.provider_instance)}{itemDisplayId(item, providers) ? <span className="library-item-id"> {itemDisplayId(item, providers)}</span> : null}</span>
      <span className="context-toolbar-spacer" />
      {details}
    </div>
    <h2 className="library-item-title" title={item.title}>{item.title}</h2>
    <div className="library-item-line library-item-state">
      {pending
        ? <span className="context-source-chip library-state is-muted"><span className="library-spinner" aria-hidden="true" />Refreshing…</span>
        : <span className={`context-source-chip library-state is-${chip.tone}`}><span aria-hidden="true">{chip.glyph}</span> {chip.word}</span>}
      <span className="library-item-phrase">{freshness.phrase}</span>
      <span className="context-toolbar-spacer" />
      {narrow ? null : <button type="button" onClick={refresh} disabled={actions.refreshBusy}>Refresh</button>}
      <button type="button" className="library-more" aria-label={`More actions for ${item.title}`} aria-haspopup="menu" aria-expanded={menu !== null} onClick={(event) => setMenu(menuAnchor(event.currentTarget))}><UiIcon name="more" /></button>
    </div>
    {freshness.notice && !pending ? <div className={`context-notice library-item-notice${item.state === "failed" ? " context-notice-error" : item.state === "conflict" || item.state === "partial" ? " context-notice-warning" : ""}`} role={item.state === "failed" ? "alert" : "status"}>
      <span>{freshness.notice}</span>
      {item.state === "conflict" ? <button type="button" onClick={() => onReplace(item)} disabled={actions.refreshBusy}>Replace with source version…</button> : null}
      {item.state === "failed" ? <button type="button" onClick={refresh} disabled={actions.refreshBusy}>Retry</button> : null}
    </div> : null}
    <details className="library-metadata">
      <summary>Metadata</summary>
      <dl>
        {item.canonical_id ? <><dt>Source identity</dt><dd><code>{item.canonical_id}</code></dd></> : null}
        {item.provider_instance ? <><dt>Provider</dt><dd>{item.provider_id} · {item.provider_instance}</dd></> : null}
        {item.source_url ? <><dt>Source link</dt><dd><code>{item.source_url}</code></dd></> : null}
        {item.original_url && item.original_url !== item.source_url ? <><dt>Added from</dt><dd><code>{item.original_url}</code></dd></> : null}
        {item.source_revision ? <><dt>Source revision</dt><dd><code>{item.source_revision}</code></dd></> : null}
        {item.fetched_at ? <><dt>Fetched</dt><dd>{item.fetched_at}</dd></> : null}
        {item.checked_at ? <><dt>Checked</dt><dd>{item.checked_at}</dd></> : null}
        <dt>Library path</dt><dd><code>{item.item_path}</code></dd>
        <dt>Library revision</dt><dd><code>{item.revision}</code></dd>
        {item.conflict.map((file) => <Fragment key={file.path}><dt>Edited file</dt><dd><code>{file.path}</code></dd></Fragment>)}
        {item.diagnostics.map((diagnostic, index) => <Fragment key={`${diagnostic.code}:${index}`}><dt>Diagnostic</dt><dd><code>{diagnostic.code}</code> · {diagnostic.message}</dd></Fragment>)}
      </dl>
    </details>
    {menu ? <LibraryMenu x={menu.x} y={menu.y} label={`${item.title} actions`} entries={itemMenuEntries(item, actions, false)} onDismiss={() => setMenu(null)} /> : null}
  </div>;
}
