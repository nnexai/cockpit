import { useEffect, useLayoutEffect, useRef, useState, type FocusEvent, type ReactNode } from "react";
import type { LibraryAttachment, LibraryAttachmentRequest, LibraryItemSummary, LibraryOperation, ProjectProvider } from "../../protocol/generated/v1";
import type { ProviderFacts } from "../context/providerDocument";
import { UiIcon } from "../UiIcon";
import { ErrorSlot } from "../ErrorSlot";
import { attachmentSummary, instanceHost, isConfluencePage, itemDisplayId, itemKindLabel, libraryFreshness, libraryStateChip, providerFamily, relativeTime, sourceEditPhrase, timeDetail } from "./libraryState";
import { ATTACHMENT_STATE, LibraryMenu, attachmentMark, attachmentPath, attachmentProgress, byteSize, downloadableAttachments, itemMenuEntries, menuAnchor, type LibraryAttachmentActions, type LibraryItemActions, type LibraryMenuEntry } from "./LibraryTree";
import { ProviderMark } from "./ProviderMark";
import { PendingPill, StatePill } from "./StatePill";

/** The item's standing in the target Space (design §4.4); absent without a live target Space. */
export type ItemSpaceState = {
  label: string;
  selected: boolean;
  adding: boolean;
  busy: boolean;
  error: string | null;
  onAdd: () => void;
  onRemove: () => void;
};

/** A Confluence page's last edit, from the page document's frontmatter; `by` is a display name, never an email. */
export type PageUpdate = { at: string | null; by: string | null };

/**
 * An issue's or review's own facts from its generated document's frontmatter as
 * one quiet line: kind, status, priority, assignee, author, last update.
 * Nothing renders when the document reports none of them.
 */
export function ProviderFactsLine({ facts, now, className }: { facts: ProviderFacts; now: number; className: string }) {
  const updated = relativeTime(facts.updated, now);
  const entries = [
    facts.itemType || null,
    facts.status || null,
    facts.priority ? `Priority ${facts.priority}` : null,
    facts.assignee === undefined ? null : facts.assignee ? `Assignee ${facts.assignee}` : "Unassigned",
    facts.author ? `Author ${facts.author}` : null,
  ].filter((entry): entry is string => entry !== null);
  if (entries.length === 0 && !updated) return null;
  return <div className={className}>
    <span className="library-facts-text">{entries.join(" · ")}{updated ? <>{entries.length > 0 ? " · " : ""}<span title={facts.updated ?? undefined}>{`Updated ${updated}`}</span></> : null}</span>
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
 * Library item header (design §4.5). The title row: a provider tile, the kind
 * and place as a quiet eyebrow, the title, and the actions at the right edge
 * (`Refresh`, the Space action, `⋯`, Details `ⓘ`). Under it the item's own
 * facts (an issue's or review's kind, status, priority, people, update) as one
 * quiet line, then one state line: the state pill, the freshness phrase, the
 * item's standing in the Space, a token cue and the attachments toggle, which
 * opens the attachments panel (a Jira issue's list is read-only: names, sizes,
 * types). Item facts beyond that live in the Details popover (`details`).
 * The Space action selects or unselects the live Library item: `Add to Space`
 * or `Remove from Space`. Attachments are metadata only until the user
 * explicitly downloads (`Download all`, `Download selected` or a row's
 * `Download`); `Remove downloaded` drops the bytes and keeps the rows. A Jira
 * issue whose files need a token says so on the state line, and `⋯` offers
 * `Provider tokens…` for every provider that stores one.
 * `⋯` takes `Refresh`, the Space actions and the attachment bulk actions, and the attachments list stacks, when the pane is ≤ 520 px wide or the header itself is ≤ 640 px (the tree beside an item can leave it that narrow in a wider pane; the attachments table needs about 590 px).
 */
export function LibraryItemHeader({ item, providers, narrow: paneNarrow, rootCrumb, pending, actions, onReplace, details, space = null, pageUpdate = null, facts = null }: {
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
  const headerRef = useRef<HTMLDivElement>(null);
  const [cramped, setCramped] = useState(false);
  useLayoutEffect(() => {
    const header = headerRef.current;
    if (!header || typeof ResizeObserver === "undefined") return;
    const measure = () => { const width = header.getBoundingClientRect().width; if (width > 0) setCramped(width <= 640); };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(header);
    return () => observer.disconnect();
  }, []);
  const narrow = paneNarrow || cramped;
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
  // Both providers download over Cockpit HTTP with a stored token; the Jira issue cue gates its download controls.
  const credentials = actions.credentials;
  const jira = credentials?.attachmentAccess(item) ?? null;
  const attachments = page || jira === "stored" ? actions.attachments : undefined;
  // The cue outside the folded attachments panel: this issue's files can't be downloaded until a token is stored.
  const needsToken = jira === "needs_token" && credentials ? credentials : null;
  // `Provider tokens…` in `⋯` for every provider that stores one; it stays before the destructive entry.
  const tokenProvider = credentials && item.provider_id && (family.key === "jira" || family.key === "confluence") ? item.provider_id : null;
  const spacePending = Boolean(space && (space.adding || space.busy));
  const spaceActionLabel = space
    ? spacePending ? space.adding || !space.selected ? "Adding…" : "Removing…"
      : space.selected ? "Remove from Space" : "Add to Space"
    : null;
  const changeSpaceSelection = () => {
    if (!space || spacePending) return;
    if (space.selected) space.onRemove(); else space.onAdd();
  };
  const menuEntries = (): LibraryMenuEntry[] => {
    const entries = itemMenuEntries(item, space ? {
      ...actions,
      spaceEntries: () => [{ label: spaceActionLabel!, onSelect: changeSpaceSelection, disabled: spacePending }],
    } : actions, false);
    if (!credentials || !tokenProvider) return entries;
    const destructive = entries.lastIndexOf("separator");
    const tokens = { label: "Provider tokens…", onSelect: () => credentials.open(tokenProvider) };
    return destructive < 0 ? [...entries, tokens] : [...entries.slice(0, destructive), tokens, ...entries.slice(destructive)];
  };
  const ensureCredentials = credentials?.ensure;
  useEffect(() => { if (jira === "loading" && item.attachments.length > 0) ensureCredentials?.(); }, [jira, item.attachments.length, ensureCredentials]);
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
  // Keep focus in the Space slot as a selection request settles.
  const spaceSlotRef = useRef<HTMLSpanElement>(null);
  const spaceFocused = useRef(false);
  const trackSpaceFocus = {
    onFocus: () => { spaceFocused.current = true; },
    onBlur: (event: FocusEvent) => { if (event.relatedTarget) spaceFocused.current = false; },
  };
  useLayoutEffect(() => {
    if (spaceFocused.current && (document.activeElement === null || document.activeElement === document.body)) spaceSlotRef.current?.focus({ preventScroll: true });
  });
  const idText = itemDisplayId(item, providers);
  // An issue's project key opens its id (`SCRUM` · `SCRUM-6`): say it once.
  const showPath = !(idText && !chain && !folder && container && idText.startsWith(`${container}-`));
  const refreshLabel = folder ? "Re-copy" : "Refresh";
  // aria-disabled keeps focus on the pressed button while the refresh runs.
  const refreshButton = narrow ? null : <button type="button" className="library-button" aria-disabled={actions.refreshBusy} onClick={() => { if (!actions.refreshBusy) refresh(); }} title={folder ? `Re-copy from ${folder.origin_path}` : undefined}><UiIcon name="refresh" />{refreshLabel}</button>;
  return <div className={`library-item-header${narrow ? " is-narrow" : ""}`} ref={headerRef}>
    <div className="library-item-column">
      <div className="library-item-top">
        <ProviderMark family={family} size="header" folder={folder !== null} />
        <div className="library-item-idtext">
          <div className="library-item-line library-item-eyebrow">
            <span className="library-kind-chip">{itemKindLabel(item, providers)}</span>
            <span className="library-item-dot" aria-hidden="true">·</span>
            <span className="library-item-path" title={chain ? [item.container?.label, ...item.ancestors.map((ancestor) => ancestor.title)].filter(Boolean).join(" › ") : undefined}>{rootCrumb ? <><span>Library</span><span aria-hidden="true"> › </span></> : null}{showPath ? (folder ? "Folders" : chain ? breadcrumb(chain) : container ?? instanceHost(item.provider_instance)) : null}{idText ? <span className="library-item-id">{showPath ? " " : ""}{idText}</span> : null}</span>
          </div>
          <h2 className="library-item-title" title={item.title}>{item.title}</h2>
        </div>
        <div className="library-item-actions">
          {refreshButton}
          {space ? <span ref={spaceSlotRef} className="library-space-slot" tabIndex={-1} {...trackSpaceFocus}>
            {spacePending && narrow ? <span role="status"><PendingPill word={spaceActionLabel!} /></span> : null}
            {!narrow ? <button type="button" className={`library-button${space.selected ? "" : " is-primary"}`} aria-disabled={spacePending} aria-label={space.selected ? `Remove ${item.title} from ${space.label}` : `Add ${item.title} to ${space.label}`} title={space.selected ? `Remove this item from Space "${space.label}"; keep it in the Library` : `Select this item for Space "${space.label}"; read it directly from the Library`} onClick={changeSpaceSelection}>
              {!space.selected ? <UiIcon name="plus" /> : null}
              <span role={spacePending ? "status" : undefined}>{spaceActionLabel}</span>
            </button> : null}
          </span> : null}
          <button type="button" className="library-icon-button library-overflow library-more" aria-label={`More actions for ${item.title}`} title="More actions" aria-haspopup="menu" aria-expanded={menu !== null} onClick={(event) => setMenu(menuAnchor(event.currentTarget))}><UiIcon name="more" /></button>
          {details}
        </div>
      </div>
      {facts ? <ProviderFactsLine facts={facts} now={now} className="library-item-meta" /> : null}
      <div className="library-item-line library-item-state">
        {pending
          ? <PendingPill size="header" word={progress ?? (folder ? "Re-copying…" : "Refreshing…")} />
          : <StatePill size="header" shape={chip.shape} word={chip.word} tone={chip.tone} />}
        <span className="library-item-phrase" title={phraseTitle || undefined}>{folder ? <>Copied{copiedAgo ? ` ${copiedAgo}` : ""} from <code>{folder.origin_path}</code> · {folder.files} files · {folder.bytes >= 1_000_000 ? `${(folder.bytes / 1_000_000).toFixed(1)} MB` : `${folder.bytes} bytes`}{folder.git_working_tree ? " · Git working tree" : ""}</> : phrase}</span>
        {space ? <span className="library-space-state" role="status">
          <span className="library-space-context">{space.label}</span>
          <span className="library-state">{space.selected ? "Selected" : "Not selected"}</span>
        </span> : null}
        {needsToken ? <span className="library-token-cue" role="status">
          <UiIcon name="info" />Attachments need a token
          <button type="button" className="library-link" onClick={() => needsToken.open(item.provider_id ?? "")}>Provider tokens…</button>
        </span> : null}
        <span className="context-toolbar-spacer" />
        {item.attachments.length > 0 ? <button type="button" className="library-attachments-toggle" aria-expanded={attachmentsOpen} aria-controls={attachmentsOpen ? attachmentListId : undefined} onClick={() => setAttachmentsOpenFor(attachmentsOpen ? null : item.item_id)}>
          <UiIcon name={attachmentsOpen ? "down" : "right"} />{attachmentSummary(item)}
        </button> : null}
      </div>
      {freshness.notice && !pending ? <div className={`context-notice library-item-notice${item.state === "failed" ? " context-notice-error" : item.state === "conflict" || item.state === "partial" ? " context-notice-warning" : ""}`} role={item.state === "failed" ? "alert" : "status"}>
        <span>{freshness.notice}</span>
        {item.state === "conflict" ? <button type="button" className="library-button" onClick={() => onReplace(item)} disabled={actions.refreshBusy}>Replace with source version…</button> : null}
        {item.state === "failed" ? <button type="button" className="library-button" onClick={refresh} disabled={actions.refreshBusy}>Retry</button> : null}
      </div> : null}
      {space?.error ? <div className="context-notice context-notice-error library-item-notice" role="alert" {...trackSpaceFocus}>
        <span>{space.error}</span>
        <button type="button" className="library-button" aria-label={`Retry ${space.selected ? `removing ${item.title} from` : `adding ${item.title} to`} ${space.label}`} disabled={spacePending} onClick={changeSpaceSelection}>Retry</button>
      </div> : null}
      {item.attachments.length > 0 && attachmentsOpen ? <div className="library-attachments" id={attachmentListId}>
        <div className="library-attachments-head">
          {jira === "needs_token" && credentials
            ? <><span>Downloading Jira attachments needs a token stored in Cockpit</span><button type="button" className="library-button is-panel" onClick={() => credentials.open(item.provider_id ?? "")}>Provider token…</button></>
            : jira === "loading" ? null : <span>{page || jira === "stored" ? `${downloaded.length} of ${item.attachments.length} downloaded` : "Listed from Jira; attachment files are not downloaded"}</span>}
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
                <td className="library-attachment-state" title={running ?? ATTACHMENT_STATE[attachment.state]}>{attachmentState(attachment, running)}</td>
                {attachments ? <td className="library-attachment-action">{rowAction(attachment)}</td> : null}
              </tr>)}
            </tbody>
          </table>}
        </div>
      </div> : null}
    </div>
    {menu ? <LibraryMenu x={menu.x} y={menu.y} label={`${item.title} actions`} entries={menuEntries()} onDismiss={() => setMenu(null)} /> : null}
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
    if (error) return <ErrorSlot placement="pane" className="library-report" message={`${download ? "Download didn't start:" : "Removal didn't start:"} ${error}`}
      actions={<><button type="button" onClick={() => onRetry(request.attachment_ids)}>Retry</button>{dismiss}</>} />;
    return <ErrorSlot placement="pane" className="library-report" error={false} message={starting ? `${download ? "Downloading" : "Removing"} ${attachmentCount(count)}${title}…` : null} />;
  }
  const phase = operation.phases.find((candidate) => candidate.phase === "library");
  if (!operation.finished) {
    return <ErrorSlot placement="pane" className="library-report" error={Boolean(error)}
      message={error ?? `${download ? "Downloading" : "Removing downloaded"} ${attachmentCount(phase?.total ?? count)}${title}… ${phase?.done ?? 0} done`}
      actions={operation.cancel_requested ? <span>Cancelling after the attachment in flight…</span> : <button type="button" onClick={onCancel}>Cancel</button>} />;
  }
  if (phase?.state === "failed") {
    return <ErrorSlot placement="pane" className="library-report" message={`${download ? "Download failed:" : "Removal failed:"} ${phase.error?.message ?? "The attachment operation failed."} Reload to see the current Library state.`}
      actions={<><button type="button" onClick={() => onRetry(request.attachment_ids)}>Retry</button>{dismiss}</>} />;
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
  return <ErrorSlot placement="pane" className="library-report" error={failedIds.length > 0}
    message={<><strong>{cancelled ? (download ? "Download cancelled:" : "Removal cancelled:") : (download ? "Download finished:" : "Removal finished:")}</strong> {summary} <span className="library-report-note">Spaces selecting this item read its current Library files.</span></>}
    actions={<>{failedIds.length ? <button type="button" onClick={() => onRetry(failedIds)}>Retry failed</button> : null}{dismiss}</>} />;
}
