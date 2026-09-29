import { useLayoutEffect, useRef, useState, type FocusEvent, type ReactNode } from "react";
import type { LibraryAttachment, LibraryAttachmentRequest, LibraryItemSummary, LibraryOperation, ProjectProvider, SpaceAddAttempt, SpaceCopyRow } from "../../protocol/generated/v1";
import type { ProviderFacts } from "../context/providerDocument";
import { UiIcon } from "../UiIcon";
import { attachmentSummary, instanceHost, isConfluencePage, itemDisplayId, itemKindLabel, libraryFreshness, libraryStateChip, providerFamily, relativeTime, sourceEditPhrase, timeDetail } from "./libraryState";
import { ATTACHMENT_STATE, LibraryMenu, attachmentMark, attachmentPath, attachmentProgress, byteSize, downloadableAttachments, itemMenuEntries, menuAnchor, type LibraryAttachmentActions, type LibraryItemActions } from "./LibraryTree";
import { ProviderMark } from "./ProviderMark";
import { PendingPill, StatePill } from "./StatePill";
import { headerSpaceAction, type SpaceCopyAction } from "./spaceCopyPresentation";
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
  /** The existing copy's actions this surface performs: `Update`, a confirmed replace or removal, the Library version. */
  actions: SpaceCopyAction[];
  /** This item's copy update or replace is running, or awaits the Space's reread. */
  updating: boolean;
  /** Holds the copy actions until the Space confirms the last one, including while its reread failed. */
  busy: boolean;
  /** Why this item's last update, replace or removal in the Space changed nothing. */
  copyError: string | null;
  onAction: (action: SpaceCopyAction) => void;
};

/** A Confluence page's last edit, from the page document's frontmatter; `by` is a display name, never an email. */
export type PageUpdate = { at: string | null; by: string | null };

/**
 * An issue's or review's own facts from its generated document's frontmatter:
 * kind and status as chips, then priority, assignee, author and last update.
 * Nothing renders when the document reports none of them.
 */
export function ProviderFactsLine({ facts, now, className }: { facts: ProviderFacts; now: number; className: string }) {
  const updated = relativeTime(facts.updated, now);
  const meta = [
    facts.priority ? `Priority ${facts.priority}` : null,
    facts.assignee === undefined ? null : facts.assignee ? `Assignee ${facts.assignee}` : "Unassigned",
    facts.author ? `Author ${facts.author}` : null,
  ].filter((entry): entry is string => entry !== null);
  if (!facts.itemType && !facts.status && meta.length === 0 && !updated) return null;
  return <div className={className}>
    {facts.itemType ? <span className="context-source-chip library-fact-kind">{facts.itemType}</span> : null}
    {facts.status ? <span className="context-source-chip library-fact-status">{facts.status}</span> : null}
    {meta.length > 0 || updated ? <span className="library-item-phrase">{meta.join(" · ")}{updated ? <>{meta.length > 0 ? " · " : ""}<span title={facts.updated ?? undefined}>{`Updated ${updated}`}</span></> : null}</span> : null}
  </div>;
}

/**
 * `SD › Engineering home › Release process`. Beyond four segments the middle
 * gives way: the space key and the last two stay, the full chain is the tooltip.
 */
function breadcrumb(segments: readonly string[]): string {
  return (segments.length > 4 ? [segments[0], "…", ...segments.slice(-2)] : segments).join(" › ");
}

/**
 * Library item header (design §4.5): a provider tile beside the kind label,
 * breadcrumb and title, then one state line (pill, freshness phrase, actions)
 * and, for a Confluence page with attachments, one summary control that opens
 * the attachments panel. Item facts live in the Details popover (`details`).
 * The Space actions come from `headerSpaceAction`: `Add to <Space>` (the one
 * primary action), or the copy's `Update in <Space>`, a confirmed replace or
 * removal, and the Library version. Attachments are metadata only until the user
 * explicitly downloads (`Download all`, `Download selected` or a row's
 * `Download`); `Remove downloaded` drops the bytes and keeps the rows. An issue
 * or review adds its provider facts under the title.
 * At pane width ≤ 520 px `Refresh`, the Space actions and the attachment bulk actions move into `⋯`.
 */
export function LibraryItemHeader({ item, providers, narrow, rootCrumb, pending, actions, onReplace, details, space = null, pageUpdate = null, facts = null }: {
  item: LibraryItemSummary;
  providers: readonly ProjectProvider[];
  narrow: boolean;
  /** Shows the `Library` breadcrumb segment when the Library is one root among several. */
  rootCrumb: boolean;
  pending: boolean;
  actions: LibraryItemActions;
  onReplace: (item: LibraryItemSummary) => void;
  /** The Details popover, kept at the end of the identity block. */
  details: ReactNode;
  space?: ItemSpaceState | null;
  pageUpdate?: PageUpdate | null;
  facts?: ProviderFacts | null;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const now = Date.now();
  const chip = libraryStateChip(item.state);
  const freshness = libraryFreshness(item, now);
  const container = item.container?.label ?? null;
  const folder = item.folder;
  const copiedAgo = relativeTime(item.fetched_at, now);
  const page = isConfluencePage(item);
  const family = providerFamily(providers, item.provider_id);
  const chain = page ? [item.container?.container_id ?? instanceHost(item.provider_instance), ...item.ancestors.map((ancestor) => ancestor.title)] : null;
  const version = page && item.source_revision ? `v${item.source_revision}` : null;
  // Display names only: a value that looks like an email is never shown.
  const updatedBy = page && pageUpdate?.by && !/\S+@\S+/.test(pageUpdate.by) ? pageUpdate.by : null;
  const sourcePhrase = page ? sourceEditPhrase({ version, editedAt: pageUpdate?.at, by: updatedBy }, now) : null;
  const phrase = folder ? null : [freshness.phrase, sourcePhrase].filter(Boolean).join(" · ");
  // Absolute times for the phrase's tooltip; the Cockpit and source clocks keep their own labels.
  const checkedDetail = timeDetail(item.checked_at ?? item.fetched_at, now);
  const editedDetail = page ? timeDetail(pageUpdate?.at, now) : null;
  const phraseTitle = [
    checkedDetail ? `Checked ${checkedDetail.text}` : null,
    page && (version || editedDetail || updatedBy) ? [item.source_revision ? `Version ${item.source_revision}` : null, editedDetail ? `edited ${editedDetail.text}` : null, updatedBy ? `by ${updatedBy}` : null].filter(Boolean).join(" ") : null,
  ].filter(Boolean).join(" · ");
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
  // The attachment panel stays folded until asked for, per item, so a page's body starts near the top.
  const [attachmentsOpenFor, setAttachmentsOpenFor] = useState<string | null>(null);
  const attachmentsOpen = attachmentsOpenFor === item.item_id;
  const attachmentListId = `library-attachments-${item.item_id.replace(/[^A-Za-z0-9_-]/g, "_")}`;
  const download = (ids: string[]) => {
    if (!attachments || ids.length === 0) return;
    attachments.start(item, "download", ids);
    setChosen({ itemId: item.item_id, ids: new Set() });
  };
  const rowAction = (attachment: LibraryAttachment) => {
    if (!attachments) return null;
    if (attachment.state === "downloaded") {
      return attachmentPath(item, attachment) ? <button type="button" className="library-button is-panel" onClick={() => attachments.open(item, attachment)} aria-label={`Open ${attachment.stored_name}`}>Open</button> : null;
    }
    if (attachment.state === "over_limit") return null;
    return <button type="button" className="library-button is-panel" onClick={() => download([attachment.attachment_id])} disabled={attachments.busy} aria-label={`${attachment.state === "failed" ? "Retry download of" : "Download"} ${attachment.stored_name}`}>{attachment.state === "failed" ? "Retry" : "Download"}</button>;
  };
  const shownAttachments = item.attachments.map((attachment) => ({ attachment, progress: attachmentProgress(attachments?.active, item.item_id, attachment.attachment_id) }));
  const attachmentState = (attachment: LibraryAttachment, running: string | null) => {
    if (running) return <PendingPill word={running} />;
    const mark = attachmentMark(attachment.state);
    return mark ? <StatePill shape={mark.shape} tone={mark.tone} word={ATTACHMENT_STATE[attachment.state]} /> : <span className="library-state is-muted">{ATTACHMENT_STATE[attachment.state]}</span>;
  };
  const refresh = () => actions.refresh({ scope: "items", item_ids: [item.item_id] }, [item.item_id]);
  const spaceAction = space ? headerSpaceAction(space.row, space.label) : null;
  const spaceStatus = spaceAction?.status ?? null;
  const spaceAdding = Boolean(space && (space.adding || space.attempt?.state === "pending"));
  const addLabel = spaceAction?.actions.find((action) => action.kind === "add")?.label;
  const spaceUpdating = Boolean(space?.updating);
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
    <div className="library-item-column">
      <div className="library-item-identity">
        <ProviderMark family={family} size="header" folder={folder !== null} />
        <div className="library-item-idtext">
          <div className="library-item-line library-item-eyebrow">
            <span className="library-kind-chip">{itemKindLabel(item, providers)}</span>
            <span className="library-item-path" title={chain ? [item.container?.label, ...item.ancestors.map((ancestor) => ancestor.title)].filter(Boolean).join(" › ") : undefined}>{rootCrumb ? <><span>Library</span><span aria-hidden="true"> › </span></> : null}{folder ? "Folders" : chain ? breadcrumb(chain) : container ?? instanceHost(item.provider_instance)}{itemDisplayId(item, providers) ? <span className="library-item-id"> {itemDisplayId(item, providers)}</span> : null}</span>
          </div>
          <h2 className="library-item-title" title={item.title}>{item.title}</h2>
        </div>
        {details}
      </div>
      {facts ? <ProviderFactsLine facts={facts} now={now} className="library-item-line library-item-facts" /> : null}
      <div className="library-item-line library-item-state">
        {pending
          ? <PendingPill size="header" word={progress ?? (folder ? "Re-copying…" : "Refreshing…")} />
          : <StatePill size="header" shape={chip.shape} word={chip.word} tone={chip.tone} />}
        <span className="library-item-phrase" title={phraseTitle || undefined}>{folder ? <>Copied{copiedAgo ? ` ${copiedAgo}` : ""} from <code>{folder.origin_path}</code> · {folder.files} files · {folder.bytes >= 1_000_000 ? `${(folder.bytes / 1_000_000).toFixed(1)} MB` : `${folder.bytes} bytes`}{folder.git_working_tree ? " · Git working tree" : ""}</> : phrase}</span>
        <span className="context-toolbar-spacer" />
        {narrow ? null : <button type="button" className="library-button" onClick={refresh} disabled={actions.refreshBusy} title={folder ? `Re-copy from ${folder.origin_path}` : undefined}><UiIcon name="refresh" />{folder ? "Re-copy" : "Refresh"}</button>}
        {space ? <span ref={spaceSlotRef} className="library-space-slot" tabIndex={-1} {...trackSpaceFocus}>
          {spaceAdding ? <span role="status"><PendingPill word={`Adding to ${space.label}…`} /></span> : null}
          {spaceUpdating ? <span role="status"><PendingPill word={`Updating ${space.label}…`} /></span> : null}
          {!spaceAdding && !spaceUpdating && !narrow && spaceStatus ? <span className="library-space-state">
            {spaceStatus.context ? <span className="library-space-context">{spaceStatus.context}</span> : null}
            <StatePill shape={spaceStatus.shape} word={spaceStatus.word} tone={spaceStatus.tone} />
          </span> : null}
          {!spaceAdding && !narrow && addLabel && !spaceFailure ? <button type="button" className="library-button is-primary" title={`Copy this ${page ? "page" : "item"} into Space "${space.label}"`} onClick={space.onAdd}><UiIcon name="plus" />{addLabel}</button> : null}
          {/* aria-disabled keeps focus on the pressed button, or the confirmation's opener, while the update runs. */}
          {!spaceAdding && !narrow ? space.actions.map((action) => <button key={action.kind} type="button" className="library-button" aria-disabled={space.busy} onClick={() => space.onAction(action)}>{action.label}</button>) : null}
        </span> : null}
        <button type="button" className="library-button is-icon library-more" aria-label={`More actions for ${item.title}`} title="More actions" aria-haspopup="menu" aria-expanded={menu !== null} onClick={(event) => setMenu(menuAnchor(event.currentTarget))}><UiIcon name="more" /></button>
      </div>
      {freshness.notice && !pending ? <div className={`context-notice library-item-notice${item.state === "failed" ? " context-notice-error" : item.state === "conflict" || item.state === "partial" ? " context-notice-warning" : ""}`} role={item.state === "failed" ? "alert" : "status"}>
        <span>{freshness.notice}</span>
        {item.state === "conflict" ? <button type="button" className="library-button" onClick={() => onReplace(item)} disabled={actions.refreshBusy}>Replace with source version…</button> : null}
        {item.state === "failed" ? <button type="button" className="library-button" onClick={refresh} disabled={actions.refreshBusy}>Retry</button> : null}
      </div> : null}
      {space && spaceFailure ? <div className="context-notice context-notice-error library-item-notice" role="alert" {...trackSpaceFocus}>
        <span>{spaceFailure}</span>
        <button type="button" className="library-button" onClick={space.onAdd}>{`Retry adding to ${space.label}`}</button>
      </div> : null}
      {space && space.copyError && !spaceUpdating ? <div className="context-notice context-notice-error library-item-notice" role="alert" {...trackSpaceFocus}>
        <span>{space.copyError}</span>
      </div> : null}
      {page && item.attachments.length > 0 ? <div className="library-attachments-line">
        <button type="button" className="library-attachments-toggle" aria-expanded={attachmentsOpen} aria-controls={attachmentsOpen ? attachmentListId : undefined} onClick={() => setAttachmentsOpenFor(attachmentsOpen ? null : item.item_id)}>
          <UiIcon name={attachmentsOpen ? "down" : "right"} />{attachmentSummary(item)}
        </button>
      </div> : null}
      {page && item.attachments.length > 0 && attachmentsOpen ? <div className="library-attachments" id={attachmentListId}>
        <div className="library-attachments-head">
          <span>{downloaded.length} of {item.attachments.length} downloaded</span>
          <span className="context-toolbar-spacer" />
          {attachments && !narrow ? <>
            {selectedIds.length > 0 ? <button type="button" className="library-button is-panel" onClick={() => download(selectedIds)} disabled={attachments.busy}>{`Download selected (${selectedIds.length})`}</button> : null}
            {downloadable.length > 0 ? <button type="button" className="library-button is-panel" onClick={() => download(downloadable.map((attachment) => attachment.attachment_id))} disabled={attachments.busy}>Download all</button> : null}
            {downloaded.length > 0 ? <button type="button" className="library-button is-panel" title="Deletes the downloaded files from the Library. The page and its attachment list stay." onClick={() => attachments.start(item, "remove_downloaded", downloaded.map((attachment) => attachment.attachment_id))} disabled={attachments.busy}>Remove downloaded</button> : null}
          </> : null}
        </div>
        <div className="library-attachments-scroll">
          {narrow ? <ul aria-label="Attachments">
            {shownAttachments.map(({ attachment, progress: running }) => <li key={attachment.attachment_id}>
              <span title={attachment.original_name !== attachment.stored_name ? attachment.original_name : undefined}>{attachment.stored_name}</span>
              <span className="library-attachment-detail">{byteSize(attachment.bytes)} · {running ?? ATTACHMENT_STATE[attachment.state]}</span>
              {rowAction(attachment)}
            </li>)}
          </ul> : <table aria-label="Attachments">
            <colgroup>
              {attachments ? <col className="is-select" /> : null}
              <col /><col className="is-size" /><col className="is-type" /><col className="is-state" />
              {attachments ? <col className="is-action" /> : null}
            </colgroup>
            <thead><tr>
              {attachments ? <th scope="col"><span className="sr-only">Select</span></th> : null}
              <th scope="col">Name</th><th scope="col" className="is-numeric">Size</th><th scope="col">Type</th><th scope="col">State</th>
              {attachments ? <th scope="col"><span className="sr-only">Action</span></th> : null}
            </tr></thead>
            <tbody>
              {shownAttachments.map(({ attachment, progress: running }) => <tr key={attachment.attachment_id}>
                {attachments ? <td className="library-attachment-select">{attachment.state === "not_downloaded" || attachment.state === "failed"
                  ? <input type="checkbox" aria-label={`Select ${attachment.stored_name}`} checked={selectedIds.includes(attachment.attachment_id)} disabled={attachments.busy} onChange={(event) => choose(attachment.attachment_id, event.target.checked)} />
                  : null}</td> : null}
                <td className="library-attachment-name" title={attachment.original_name !== attachment.stored_name ? attachment.original_name : attachment.stored_name}>{attachment.stored_name}</td>
                <td className="is-numeric">{byteSize(attachment.bytes)}</td>
                <td className="library-attachment-type" title={attachment.media_type ?? undefined}>{attachment.media_type ?? "—"}</td>
                <td className="library-attachment-state">{attachmentState(attachment, running)}</td>
                {attachments ? <td className="library-attachment-action">{rowAction(attachment)}</td> : null}
              </tr>)}
            </tbody>
          </table>}
        </div>
      </div> : null}
    </div>
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
