import { Fragment, useLayoutEffect, useRef, useState, type FocusEvent, type ReactNode } from "react";
import type { LibraryItemSummary, ProjectProvider, SpaceAddAttempt, SpaceCopyRow } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { instanceHost, itemDisplayId, itemKindLabel, libraryFreshness, libraryStateChip, relativeTime } from "./libraryState";
import { LibraryMenu, itemMenuEntries, menuAnchor, type LibraryItemActions } from "./LibraryTree";
import { headerSpaceAction } from "./spaceCopyPresentation";
import { spaceAddFailure } from "./SpaceContextList";

/** The item's standing in the target Space (design §4.4); absent without a live target Space. */
export type ItemSpaceState = {
  label: string;
  /** The Space row whose `item_id` is this item; undefined when the Space holds no copy. */
  row: SpaceCopyRow | undefined;
  /** A durable add attempt for this item in that Space (D10). */
  attempt: SpaceAddAttempt | undefined;
  adding: boolean;
  /**
   * Why this surface's last `Add to <Space>` failed when no durable attempt
   * records it: the request didn't start, or the copy stopped first.
   */
  error: string | null;
  onAdd: () => void;
};

/**
 * Library item header (design §4.4): kind chip and container path, title,
 * state chip with its freshness phrase and actions, then `Metadata`. The one
 * Space action comes from `headerSpaceAction`; S2 offers only `Add to <Space>`.
 * At pane width ≤ 520 px `Refresh` and the Space action move into `⋯`.
 */
export function LibraryItemHeader({ item, providers, narrow, rootCrumb, pending, actions, onReplace, details, space = null }: {
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
  space?: ItemSpaceState | null;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const now = Date.now();
  const chip = libraryStateChip(item.state);
  const freshness = libraryFreshness(item, now);
  const container = item.container?.label ?? null;
  const folder = item.folder;
  const copiedAgo = relativeTime(item.fetched_at, now);
  const refresh = () => actions.refresh({ scope: "items", item_ids: [item.item_id] }, [item.item_id]);
  const spaceAction = space ? headerSpaceAction(space.row, space.label) : null;
  const spaceAdding = Boolean(space && (space.adding || space.attempt?.state === "pending"));
  const addLabel = spaceAction?.actions.find((action) => action.kind === "add")?.label;
  const spaceFailure = !space || spaceAdding ? null : space.attempt?.state === "failed" ? spaceAddFailure(space.attempt.error, space.label) : space.error;
  // `Add to <Space>` and its retry give way to progress, then to the result; focus stays in the Space slot.
  const spaceSlotRef = useRef<HTMLSpanElement>(null);
  const spaceFocused = useRef(false);
  const trackSpaceFocus = {
    onFocus: () => { spaceFocused.current = true; },
    onBlur: (event: FocusEvent) => { if (event.relatedTarget) spaceFocused.current = false; },
  };
  useLayoutEffect(() => {
    if (spaceFocused.current && (document.activeElement === null || document.activeElement === document.body)) spaceSlotRef.current?.focus({ preventScroll: true });
  });
  return <div className="library-item-header">
    <div className="library-item-line">
      <span className="document-source-kind library-kind-chip">{itemKindLabel(item, providers)}</span>
      <span className="library-item-path">{rootCrumb ? <><span>Library</span><span aria-hidden="true"> › </span></> : null}{folder ? "Folders" : container ?? instanceHost(item.provider_instance)}{itemDisplayId(item, providers) ? <span className="library-item-id"> {itemDisplayId(item, providers)}</span> : null}</span>
      <span className="context-toolbar-spacer" />
      {details}
    </div>
    <h2 className="library-item-title" title={item.title}>{item.title}</h2>
    <div className="library-item-line library-item-state">
      {pending
        ? <span className="context-source-chip library-state is-muted"><span className="library-spinner" aria-hidden="true" />{folder ? "Re-copying…" : "Refreshing…"}</span>
        : <span className={`context-source-chip library-state is-${chip.tone}`}><span aria-hidden="true">{chip.glyph}</span> {chip.word}</span>}
      <span className="library-item-phrase">{folder ? <>Copied{copiedAgo ? ` ${copiedAgo}` : ""} from <code>{folder.origin_path}</code> · {folder.files} files · {folder.bytes >= 1_000_000 ? `${(folder.bytes / 1_000_000).toFixed(1)} MB` : `${folder.bytes} bytes`}{folder.git_working_tree ? " · Git working tree" : ""}</> : freshness.phrase}</span>
      <span className="context-toolbar-spacer" />
      {narrow ? null : <button type="button" onClick={refresh} disabled={actions.refreshBusy} title={folder ? `Re-copy from ${folder.origin_path}` : undefined}>{folder ? "Re-copy" : "Refresh"}</button>}
      {space ? <span ref={spaceSlotRef} className="library-space-slot" tabIndex={-1} {...trackSpaceFocus}>
        {spaceAdding ? <span className="context-source-chip library-state is-muted" role="status"><span className="library-spinner" aria-hidden="true" />{`Adding to ${space.label}…`}</span> : null}
        {!spaceAdding && !narrow && spaceAction?.text ? <span className={`library-state library-space-state is-${spaceAction.tone}`}>{spaceAction.text}</span> : null}
        {!spaceAdding && !narrow && addLabel && !spaceFailure ? <button type="button" onClick={space.onAdd}>{addLabel}</button> : null}
      </span> : null}
      <button type="button" className="library-more" aria-label={`More actions for ${item.title}`} aria-haspopup="menu" aria-expanded={menu !== null} onClick={(event) => setMenu(menuAnchor(event.currentTarget))}><UiIcon name="more" /></button>
    </div>
    {freshness.notice && !pending ? <div className={`context-notice library-item-notice${item.state === "failed" ? " context-notice-error" : item.state === "conflict" || item.state === "partial" ? " context-notice-warning" : ""}`} role={item.state === "failed" ? "alert" : "status"}>
      <span>{freshness.notice}</span>
      {item.state === "conflict" ? <button type="button" onClick={() => onReplace(item)} disabled={actions.refreshBusy}>Replace with source version…</button> : null}
      {item.state === "failed" ? <button type="button" onClick={refresh} disabled={actions.refreshBusy}>Retry</button> : null}
    </div> : null}
    {space && spaceFailure ? <div className="context-notice context-notice-error library-item-notice" role="alert" {...trackSpaceFocus}>
      <span>{spaceFailure}</span>
      <button type="button" onClick={space.onAdd}>{`Retry adding to ${space.label}`}</button>
    </div> : null}
    <details className="library-metadata">
      <summary>Metadata</summary>
      <dl>
        {folder ? <>
          <dt>Copied from</dt><dd><code>{folder.origin_path}</code></dd>
          <dt>Inventory</dt><dd>{folder.git_working_tree ? "Git tracked and untracked, non-ignored files" : "Regular files with default exclusions"}</dd>
          <dt>Copied</dt><dd>{folder.files} files · {folder.bytes} bytes</dd>
          <dt>Skipped symlinks</dt><dd>{folder.skipped_symlinks}</dd>
          <dt>Skipped special files</dt><dd>{folder.skipped_special}</dd>
          <dt>Skipped ignored files</dt><dd>{folder.skipped_ignored}</dd>
          <dt>Skipped other files</dt><dd>{folder.skipped_other}</dd>
          <dt>Updates</dt><dd>Source edits do not change this copy until an explicit re-copy. Space copies update separately.</dd>
        </> : null}
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
