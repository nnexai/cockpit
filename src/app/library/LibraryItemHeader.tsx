import { Fragment, useLayoutEffect, useRef, useState, type FocusEvent, type ReactNode } from "react";
import type { LibraryAttachment, LibraryAttachmentRequest, LibraryItemSummary, LibraryOperation, ProjectProvider, SpaceAddAttempt, SpaceCopyRow } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { instanceHost, isConfluencePage, itemDisplayId, itemKindLabel, libraryFreshness, libraryStateChip, relativeTime } from "./libraryState";
import { ATTACHMENT_STATE, LibraryMenu, attachmentPath, attachmentProgress, byteSize, downloadableAttachments, itemMenuEntries, menuAnchor, type LibraryAttachmentActions, type LibraryItemActions } from "./LibraryTree";
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

/** A Confluence page's last edit, from the page document's frontmatter; `by` is a display name, never an email. */
export type PageUpdate = { at: string | null; by: string | null };

/**
 * Library item header (design §4.4): kind chip and container path, title,
 * state chip with its freshness phrase and actions, then `Metadata`. The one
 * Space action comes from `headerSpaceAction`; S2 offers only `Add to <Space>`.
 * A Confluence page adds its page metadata and an attachments table: metadata
 * only until the user explicitly downloads (`Download all`, `Download selected`
 * or a row's `Download`); `Remove downloaded` drops the bytes and keeps the rows.
 * At pane width ≤ 520 px `Refresh`, the Space action and the attachment bulk actions move into `⋯`.
 */
export function LibraryItemHeader({ item, providers, narrow, rootCrumb, pending, actions, onReplace, details, space = null, pageUpdate = null }: {
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
  pageUpdate?: PageUpdate | null;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const now = Date.now();
  const chip = libraryStateChip(item.state);
  const freshness = libraryFreshness(item, now);
  const container = item.container?.label ?? null;
  const folder = item.folder;
  const copiedAgo = relativeTime(item.fetched_at, now);
  const page = isConfluencePage(item);
  // `SD / Release process`: the space key, then the page's ancestors.
  const pagePath = page ? [item.container?.container_id ?? instanceHost(item.provider_instance), ...item.ancestors.map((ancestor) => ancestor.title)].join(" / ") : null;
  const version = page && item.source_revision ? `v${item.source_revision}` : null;
  // Display names only: a value that looks like an email is never shown.
  const updatedBy = page && pageUpdate?.by && !/\S+@\S+/.test(pageUpdate.by) ? pageUpdate.by : null;
  const downloaded = item.attachments.filter((attachment) => attachment.state === "downloaded");
  const attachments = actions.attachments;
  const downloadable = downloadableAttachments(item);
  const progress = attachmentProgress(attachments?.active, item.item_id);
  // Chosen rows, for this item only; a row that stops being downloadable leaves the selection.
  const [chosen, setChosen] = useState<{ itemId: string; ids: ReadonlySet<string> }>({ itemId: item.item_id, ids: new Set() });
  const selectedIds = chosen.itemId === item.item_id ? downloadable.map((attachment) => attachment.attachment_id).filter((id) => chosen.ids.has(id)) : [];
  const choose = (id: string, on: boolean) => setChosen((current) => {
    const ids = new Set(current.itemId === item.item_id ? current.ids : []);
    if (on) ids.add(id); else ids.delete(id);
    return { itemId: item.item_id, ids };
  });
  const download = (ids: string[]) => {
    if (!attachments || ids.length === 0) return;
    attachments.start(item, "download", ids);
    setChosen({ itemId: item.item_id, ids: new Set() });
  };
  const rowAction = (attachment: LibraryAttachment) => {
    if (!attachments) return null;
    if (attachment.state === "downloaded") {
      return attachmentPath(item, attachment) ? <button type="button" onClick={() => attachments.open(item, attachment)} aria-label={`Open ${attachment.stored_name}`}>Open</button> : null;
    }
    if (attachment.state === "over_limit") return null;
    return <button type="button" onClick={() => download([attachment.attachment_id])} disabled={attachments.busy} aria-label={`${attachment.state === "failed" ? "Retry download of" : "Download"} ${attachment.stored_name}`}>{attachment.state === "failed" ? "Retry" : "Download"}</button>;
  };
  const shownAttachments = item.attachments.map((attachment) => ({ attachment, state: attachmentProgress(attachments?.active, item.item_id, attachment.attachment_id) ?? ATTACHMENT_STATE[attachment.state] }));
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
      <span className="library-item-path" title={page ? [item.container?.label, ...item.ancestors.map((ancestor) => ancestor.title)].filter(Boolean).join(" › ") : undefined}>{rootCrumb ? <><span>Library</span><span aria-hidden="true"> › </span></> : null}{folder ? "Folders" : pagePath ?? container ?? instanceHost(item.provider_instance)}{itemDisplayId(item, providers) ? <span className="library-item-id"> {itemDisplayId(item, providers)}</span> : null}</span>
      <span className="context-toolbar-spacer" />
      {details}
    </div>
    <h2 className="library-item-title" title={item.title}>{item.title}</h2>
    <div className="library-item-line library-item-state">
      {pending
        ? <span className="context-source-chip library-state is-muted"><span className="library-spinner" aria-hidden="true" />{progress ?? (folder ? "Re-copying…" : "Refreshing…")}</span>
        : <span className={`context-source-chip library-state is-${chip.tone}`}><span aria-hidden="true">{chip.glyph}</span> {chip.word}</span>}
      <span className="library-item-phrase">{folder ? <>Copied{copiedAgo ? ` ${copiedAgo}` : ""} from <code>{folder.origin_path}</code> · {folder.files} files · {folder.bytes >= 1_000_000 ? `${(folder.bytes / 1_000_000).toFixed(1)} MB` : `${folder.bytes} bytes`}{folder.git_working_tree ? " · Git working tree" : ""}</> : `${freshness.phrase}${version ? ` · ${version}` : ""}${updatedBy ? ` by ${updatedBy}` : ""}`}</span>
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
        {page ? <>
          {item.container ? <><dt>Space</dt><dd>{item.container.label}</dd></> : null}
          {item.canonical_id ? <><dt>Page id</dt><dd><code>{item.canonical_id}</code></dd></> : null}
          {item.ancestors.length ? <><dt>Parent</dt><dd>{item.ancestors.at(-1)!.title}</dd></> : null}
          <dt>Ancestors</dt><dd>{item.ancestors.length ? item.ancestors.map((ancestor) => ancestor.title).join(" / ") : "None (top-level page)"}</dd>
          {version ? <><dt>Version</dt><dd>{version}</dd></> : null}
          {pageUpdate?.at || updatedBy ? <><dt>Last updated</dt><dd>{[pageUpdate?.at, updatedBy ? `by ${updatedBy}` : null].filter(Boolean).join(" ")}</dd></> : null}
        </> : item.canonical_id ? <><dt>Source identity</dt><dd><code>{item.canonical_id}</code></dd></> : null}
        {item.provider_instance ? <><dt>Provider</dt><dd>{item.provider_id} · {item.provider_instance}</dd></> : null}
        {item.source_url ? <><dt>Source link</dt><dd><code>{item.source_url}</code></dd></> : null}
        {item.original_url && item.original_url !== item.source_url ? <><dt>Added from</dt><dd><code>{item.original_url}</code></dd></> : null}
        {item.source_revision && !page ? <><dt>Source revision</dt><dd><code>{item.source_revision}</code></dd></> : null}
        {item.fetched_at ? <><dt>Fetched</dt><dd>{item.fetched_at}</dd></> : null}
        {item.checked_at ? <><dt>Checked</dt><dd>{item.checked_at}</dd></> : null}
        <dt>Library path</dt><dd><code>{item.item_path}</code></dd>
        <dt>Library revision</dt><dd><code>{item.revision}</code></dd>
        {item.conflict.map((file) => <Fragment key={file.path}><dt>Edited file</dt><dd><code>{file.path}</code></dd></Fragment>)}
        {item.diagnostics.map((diagnostic, index) => <Fragment key={`${diagnostic.code}:${index}`}><dt>Diagnostic</dt><dd><code>{diagnostic.code}</code> · {diagnostic.message}</dd></Fragment>)}
      </dl>
    </details>
    {page && item.attachments.length > 0 ? <div className="library-attachments">
      <div className="library-attachments-heading">
        <span className="library-attachments-title">Attachments <span>{item.attachments.length} · {downloaded.length} downloaded</span></span>
        {attachments && !narrow ? <>
          {selectedIds.length > 0 ? <button type="button" onClick={() => download(selectedIds)} disabled={attachments.busy}>{`Download selected (${selectedIds.length})`}</button> : null}
          {downloadable.length > 0 ? <button type="button" onClick={() => download(downloadable.map((attachment) => attachment.attachment_id))} disabled={attachments.busy}>Download all</button> : null}
          {downloaded.length > 0 ? <button type="button" onClick={() => attachments.start(item, "remove_downloaded", downloaded.map((attachment) => attachment.attachment_id))} disabled={attachments.busy}>Remove downloaded</button> : null}
        </> : null}
      </div>
      {narrow ? <ul aria-label="Attachments">
        {shownAttachments.map(({ attachment, state }) => <li key={attachment.attachment_id}>
          <span title={attachment.original_name !== attachment.stored_name ? attachment.original_name : undefined}>{attachment.stored_name}</span>
          <span className="library-attachment-detail">{byteSize(attachment.bytes)} · {state}</span>
          {rowAction(attachment)}
        </li>)}
      </ul> : <table aria-label="Attachments">
        <thead><tr>
          {attachments ? <th scope="col"><span className="sr-only">Select</span></th> : null}
          <th scope="col">Name</th><th scope="col">Size</th><th scope="col">Type</th><th scope="col">State</th>
          {attachments ? <th scope="col"><span className="sr-only">Action</span></th> : null}
        </tr></thead>
        <tbody>
          {shownAttachments.map(({ attachment, state }) => <tr key={attachment.attachment_id}>
            {attachments ? <td className="library-attachment-select">{attachment.state === "not_downloaded" || attachment.state === "failed"
              ? <input type="checkbox" aria-label={`Select ${attachment.stored_name}`} checked={selectedIds.includes(attachment.attachment_id)} disabled={attachments.busy} onChange={(event) => choose(attachment.attachment_id, event.target.checked)} />
              : null}</td> : null}
            <td className="library-attachment-name" title={attachment.original_name !== attachment.stored_name ? attachment.original_name : undefined}>{attachment.stored_name}</td>
            <td>{byteSize(attachment.bytes)}</td>
            <td>{attachment.media_type ?? "—"}</td>
            <td className={`library-state is-${attachment.state === "downloaded" ? "idle" : attachment.state === "failed" ? "blocked" : "muted"}`}>{state}</td>
            {attachments ? <td className="library-attachment-action">{rowAction(attachment)}</td> : null}
          </tr>)}
        </tbody>
      </table>}
    </div> : null}
    {menu ? <LibraryMenu x={menu.x} y={menu.y} label={`${item.title} actions`} entries={itemMenuEntries(item, actions, false)} onDismiss={() => setMenu(null)} /> : null}
  </div>;
}

/**
 * Document area for an attachment that can't be opened (design §4.3): what is
 * known about it and, unless it is over the limit, `Download`. Nothing is
 * fetched until that is pressed; the original name is a tooltip, never markup.
 */
export function LibraryAttachmentNotice({ item, attachment, attachments }: { item: LibraryItemSummary; attachment: LibraryAttachment; attachments: LibraryAttachmentActions }) {
  const progress = attachmentProgress(attachments.active, item.item_id, attachment.attachment_id);
  const facts = [attachment.bytes === null ? null : byteSize(attachment.bytes), attachment.media_type, attachment.version ? `version ${attachment.version}` : null].filter(Boolean).join(" · ");
  const lead = attachment.state === "over_limit" ? "Not downloaded: over limit." : attachment.state === "failed" ? "Download failed." : attachment.state === "downloaded" ? "Downloaded, but not stored as an attachment file." : "Not downloaded.";
  const downloadable = attachment.state === "not_downloaded" || attachment.state === "failed";
  return <>
    <div className="context-document-header">
      <span className="document-source-kind">Attachment</span>
      <strong title={attachment.original_name !== attachment.stored_name ? attachment.original_name : undefined}>{attachment.stored_name}</strong>
      <span className="library-item-phrase">{item.title}</span>
    </div>
    <div className={`context-notice library-attachment-notice${attachment.state === "failed" ? " context-notice-error" : ""}`} role="status">
      {progress ? <span className="library-spinner" aria-hidden="true" /> : null}
      <span>{progress ?? `${lead}${facts ? ` ${facts}.` : ""}`}</span>
      {downloadable && !progress ? <button type="button" onClick={() => attachments.start(item, "download", [attachment.attachment_id])} disabled={attachments.busy}>{attachment.state === "failed" ? "Retry download" : "Download"}</button> : null}
    </div>
  </>;
}

function attachmentCount(count: number): string {
  return `${count} ${count === 1 ? "attachment" : "attachments"}`;
}

/**
 * Progress and result of one explicit attachment request (design §4.9 row
 * `Remove downloaded attachments`). Counts come from the Library as reread
 * after the operation, so they read `Checking…` until that reread lands.
 */
export function AttachmentReport({ request, operation, starting, error, item, settled, onCancel, onDismiss, onRetry }: {
  request: LibraryAttachmentRequest;
  operation: LibraryOperation | null;
  starting: boolean;
  error: string | null;
  /** The request's item as last listed; null once it left the Library. */
  item: LibraryItemSummary | null;
  /** The Library was reread after the operation finished. */
  settled: boolean;
  onCancel: () => void;
  onDismiss: () => void;
  onRetry: (attachmentIds: string[]) => void;
}) {
  const download = request.action === "download";
  const count = request.attachment_ids.length;
  const title = item ? ` of “${item.title}”` : "";
  const dismiss = <button type="button" onClick={onDismiss}>Dismiss</button>;
  if (!operation) {
    if (error) return <div className="context-notice context-notice-error library-report" role="alert">
      <strong>{download ? "Download didn't start:" : "Removal didn't start:"}</strong><span>{error}</span>
      <button type="button" onClick={() => onRetry(request.attachment_ids)}>Retry</button>{dismiss}
    </div>;
    return starting ? <div className="context-notice library-report" role="status"><span className="library-spinner" aria-hidden="true" /><span>{`${download ? "Downloading" : "Removing"} ${attachmentCount(count)}${title}…`}</span></div> : null;
  }
  const phase = operation.phases.find((candidate) => candidate.phase === "library");
  if (!operation.finished) {
    return <div className="context-notice library-report" role="status">
      <span className="library-spinner" aria-hidden="true" />
      <span>{`${download ? "Downloading" : "Removing downloaded"} ${attachmentCount(phase?.total ?? count)}${title}… ${phase?.done ?? 0} done`}</span>
      {operation.cancel_requested ? <span>Cancelling after the attachment in flight…</span> : <button type="button" onClick={onCancel}>Cancel</button>}
    </div>;
  }
  if (phase?.state === "failed") {
    return <div className="context-notice context-notice-error library-report" role="alert">
      <strong>{download ? "Download failed:" : "Removal failed:"}</strong>
      <span>{`${phase.error?.message ?? "The attachment operation failed."} Reload to see the current Library state.`}</span>
      <button type="button" onClick={() => onRetry(request.attachment_ids)}>Retry</button>{dismiss}
    </div>;
  }
  const cancelled = phase?.state === "cancelled";
  const states = item && settled ? request.attachment_ids.map((id) => item.attachments.find((attachment) => attachment.attachment_id === id)?.state ?? null) : null;
  const failedIds = states && download ? request.attachment_ids.filter((_, index) => states[index] === "failed") : [];
  let summary: string;
  if (!settled) summary = "Checking the result…";
  else if (!states) summary = download ? "Download finished." : "Removal finished.";
  else if (download) {
    const got = states.filter((state) => state === "downloaded").length;
    const over = states.filter((state) => state === "over_limit").length;
    summary = [`Downloaded ${got} of ${attachmentCount(count)}.`, over ? `${over} over limit, not downloaded.` : null, failedIds.length ? `${failedIds.length} failed.` : null].filter(Boolean).join(" ");
  } else {
    const removed = states.filter((state) => state !== "downloaded").length;
    summary = `Removed ${removed} downloaded ${removed === 1 ? "attachment" : "attachments"}. Their details stay listed.`;
  }
  return <div className={`context-notice library-report${failedIds.length ? " context-notice-warning" : ""}`} role="status">
    <strong>{cancelled ? (download ? "Download cancelled:" : "Removal cancelled:") : (download ? "Download finished:" : "Removal finished:")}</strong>
    <span>{summary}</span>
    <span className="library-report-note">Spaces aren't changed.</span>
    <span className="context-toolbar-spacer" />
    {failedIds.length ? <button type="button" onClick={() => onRetry(failedIds)}>Retry failed</button> : null}
    {dismiss}
  </div>;
}
