import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import type { LibraryAttachment, LibraryAttachmentAction, LibraryAttachmentRequest, LibraryFollowSummary, LibraryItemSummary, LibraryRefreshRequest, ProjectProvider } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { FollowRemoveDialog, type FollowRemoveMode } from "./LibraryConfirmDialog";
import { attachmentTreeMeta, errorText, followCountText, followTitle, hasFollowRef, isConfluencePage, isKeepable, itemAccessibleName, itemTreeLabel, libraryStateChip, libraryTree, pageCount, partialText, providerFamily, purgeNotice, type StateShape, type StateTone } from "./libraryState";
import { ProviderMark } from "./ProviderMark";
import { PendingPill, StatePill } from "./StatePill";
import type { ProviderCredentialActions } from "./useProviderCredentials";
import { libraryTreeRows, pageItemIds, rowLabel, type LibraryTreeRow as Row } from "./LibraryTreeRows";
import { useLibraryTreeFocus } from "./LibraryTreeFocus";
import "./library.css";

export type LibraryMenuEntry = { label: string; onSelect: () => void; disabled?: boolean; destructive?: boolean; /** Shown right-aligned as a reminder; the shortcut itself is bound elsewhere. */ shortcut?: string } | "separator";

const MENU_WIDTH = 286;
const MENU_GUTTER = 8;

/** An attachment's state as words (tooltips, accessible names, the panel's State column). */
export const ATTACHMENT_STATE: Record<LibraryAttachment["state"], string> = {
  not_downloaded: "not downloaded",
  over_limit: "not downloaded: over limit",
  downloaded: "downloaded",
  failed: "download failed",
};

/** The marked states: a downloaded or failed attachment carries a shape and tone; the others stay quiet text. */
export function attachmentMark(state: LibraryAttachment["state"]): { shape: StateShape; tone: StateTone } | null {
  if (state === "downloaded") return { shape: "check", tone: "idle" };
  if (state === "failed") return { shape: "close", tone: "blocked" };
  return null;
}

export function byteSize(bytes: number | null): string {
  if (bytes === null) return "—";
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  return bytes >= 1_000 ? `${Math.round(bytes / 1_000)} KB` : `${bytes} bytes`;
}

/**
 * The Library-root path of a downloaded attachment: its item-relative
 * `_files/<stored name>` under the item's directory, read through the
 * Library reader like any other file. Anything else is never opened.
 */
export function attachmentPath(item: LibraryItemSummary, attachment: LibraryAttachment): string | null {
  if (attachment.state !== "downloaded" || !attachment.relative_path) return null;
  const parts = attachment.relative_path.split("/");
  const name = parts[1] ?? "";
  if (parts.length !== 2 || parts[0] !== "_files" || name === "" || name === "." || name === ".." || name.includes("\\") || name !== attachment.stored_name) return null;
  return `${item.item_path.replace(/\/+$/, "")}/${attachment.relative_path}`;
}

/** Attachments a download may ask for: not an over-limit or already downloaded one. */
export function downloadableAttachments(item: LibraryItemSummary): LibraryAttachment[] {
  return item.attachments.filter((attachment) => attachment.state === "not_downloaded" || attachment.state === "failed");
}

/** `Downloading…` / `Removing…` while the attachment request in flight covers this item or attachment. */
export function attachmentProgress(active: LibraryAttachmentRequest | null | undefined, itemId: string, attachmentId?: string): string | null {
  if (!active || active.item_id !== itemId || (attachmentId !== undefined && !active.attachment_ids.includes(attachmentId))) return null;
  return active.action === "download" ? "Downloading…" : "Removing…";
}

/** Explicit attachment download and removal (S7). Nothing downloads without one of these. */
export type LibraryAttachmentActions = {
  start: (item: LibraryItemSummary, action: LibraryAttachmentAction, attachmentIds: string[]) => void;
  /** Opens a downloaded attachment in the viewer, or the not-downloaded notice for one that isn't. */
  open: (item: LibraryItemSummary, attachment: LibraryAttachment) => void;
  /** A Library operation that would conflict is starting or running here. */
  busy: boolean;
  /** The attachment request in flight, for per-row progress. */
  active: LibraryAttachmentRequest | null;
};

/** `Download attachments` / `Remove downloaded attachments` for a Confluence page's menus (design §4.3). */
export function attachmentMenuEntries(item: LibraryItemSummary, attachments: LibraryAttachmentActions | undefined, credentials?: ProviderCredentialActions): LibraryMenuEntry[] {
  if (!attachments || item.attachments.length === 0) return [];
  // A Jira issue downloads only with a token stored in Cockpit; without one its menu says so and opens the token dialog.
  const jira = credentials?.attachmentAccess(item) ?? null;
  if (jira === "needs_token") return [{ label: "Provider token…", onSelect: () => credentials?.open(item.provider_id ?? "") }];
  // Token states are still being read (openMenu asks for them): show the entry disabled until they arrive.
  if (jira === "loading") return [{ label: "Download attachments", onSelect: () => {}, disabled: true }];
  if (!isConfluencePage(item) && jira !== "stored") return [];
  const downloadable = downloadableAttachments(item);
  const downloaded = item.attachments.filter((attachment) => attachment.state === "downloaded");
  return [
    { label: "Download attachments", onSelect: () => attachments.start(item, "download", downloadable.map((attachment) => attachment.attachment_id)), disabled: attachments.busy || downloadable.length === 0 },
    ...(downloaded.length > 0 ? [{ label: "Remove downloaded attachments", onSelect: () => attachments.start(item, "remove_downloaded", downloaded.map((attachment) => attachment.attachment_id)), disabled: attachments.busy }] : []),
  ];
}

/**
 * Row / header / toolbar action menu. Reuses the workbench `context-menu`
 * component styling; arrows move, Escape closes, focus returns to the opener.
 * It renders on the document body: the viewer is a size container, which would
 * otherwise become the containing block of this fixed-position menu.
 */
export function LibraryMenu({ x, y, label, entries, onDismiss }: { x: number; y: number; label: string; entries: LibraryMenuEntry[]; onDismiss: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: MENU_WIDTH, height: 160 });
  const dismissRef = useRef(onDismiss);
  dismissRef.current = onDismiss;
  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    ref.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    const outside = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) dismissRef.current(); };
    window.addEventListener("pointerdown", outside);
    return () => {
      window.removeEventListener("pointerdown", outside);
      // Runs before a dialog opened from the menu captures its own opener.
      if (opener?.isConnected && (document.activeElement === null || document.activeElement === document.body)) opener.focus({ preventScroll: true });
    };
  }, []);
  useLayoutEffect(() => {
    const bounds = ref.current?.getBoundingClientRect();
    if (bounds && (bounds.width !== size.width || bounds.height !== size.height) && bounds.width > 0) setSize({ width: bounds.width, height: bounds.height });
  }, [size.height, size.width]);
  const left = Math.max(MENU_GUTTER, Math.min(x, window.innerWidth - size.width - MENU_GUTTER));
  const top = Math.max(MENU_GUTTER, Math.min(y, window.innerHeight - size.height - MENU_GUTTER));
  return createPortal(<div ref={ref} className="context-menu library-menu" role="menu" aria-label={label} style={{ left, top }} onContextMenu={(event) => event.preventDefault()} onKeyDown={(event) => {
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onDismiss(); return; }
    if (event.key === "Tab") { event.preventDefault(); return; }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const buttons = [...(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
    if (buttons.length === 0) return;
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1 : event.key === "ArrowDown" ? (current + 1) % buttons.length : (current - 1 + buttons.length) % buttons.length;
    buttons[next]?.focus();
  }}>
    {entries.map((entry, index) => entry === "separator"
      ? <div key={`separator-${index}`} className="context-menu-separator" role="separator" />
      : <button key={entry.label} type="button" role="menuitem" className={entry.destructive ? "destructive" : undefined} disabled={entry.disabled} onClick={() => { onDismiss(); entry.onSelect(); }}>{entry.label}{entry.shortcut ? <kbd className="library-menu-shortcut">{entry.shortcut}</kbd> : null}</button>)}
  </div>, document.body);
}

/** Anchor for a menu opened from the keyboard: just below the element. */
export function menuAnchor(element: Element): { x: number; y: number } {
  const bounds = element.getBoundingClientRect();
  return { x: bounds.left + 16, y: bounds.bottom };
}

export type LibraryItemActions = {
  open: (item: LibraryItemSummary) => void;
  refresh: (request: LibraryRefreshRequest, itemIds: string[]) => void;
  remove: (item: LibraryItemSummary) => void;
  copyLink: (item: LibraryItemSummary) => void;
  canCopyLink: boolean;
  copyLibraryPath?: (item: LibraryItemSummary) => void;
  canCopyLibraryPath?: boolean;
  refreshBusy: boolean;
  /** Selects or unselects the live Library item: `Add to Space` or `Remove from Space`. */
  spaceEntries?: (item: LibraryItemSummary) => LibraryMenuEntry[];
  /** Stops following a space or query, or removes it from the Library; resolves once the Library accepted it. */
  removeFollow?: (follow: LibraryFollowSummary, mode: FollowRemoveMode) => Promise<void>;
  /** `Keep in Library`: gives the item its own reference, so no follow's drop or unfollow can purge it. */
  keep?: (item: LibraryItemSummary) => void;
  attachments?: LibraryAttachmentActions;
  /** Provider tokens: the entry points that open the token dialog, and the Jira attachment gate. */
  credentials?: ProviderCredentialActions;
};

/** The same entries appear in the row context menu and the item header `⋯` (design §5.3). */
export function itemMenuEntries(item: LibraryItemSummary, actions: LibraryItemActions, includeOpen: boolean): LibraryMenuEntry[] {
  const space = actions.spaceEntries?.(item) ?? [];
  return [
    ...(includeOpen ? [{ label: "Open", onSelect: () => actions.open(item), disabled: !item.document_path }] : []),
    { label: item.folder ? `Re-copy from ${item.folder.origin_path}` : "Refresh from source", onSelect: () => actions.refresh({ scope: "items", item_ids: [item.item_id] }, [item.item_id]), disabled: actions.refreshBusy },
    ...(actions.keep && isKeepable(item) ? [{ label: "Keep in Library", onSelect: () => actions.keep?.(item), disabled: actions.refreshBusy || !item.source_url }] : []),
    ...space,
    ...attachmentMenuEntries(item, actions.attachments, actions.credentials),
    ...(actions.copyLibraryPath ? [{ label: "Copy Library path", onSelect: () => actions.copyLibraryPath?.(item), disabled: !actions.canCopyLibraryPath || !item.document_path }] : []),
    { label: "Copy source link", onSelect: () => actions.copyLink(item), disabled: !actions.canCopyLink || !(item.source_url ?? item.original_url) },
    "separator",
    { label: "Remove from Library…", onSelect: () => actions.remove(item), destructive: true },
  ];
}

type Menu = { x: number; y: number; row: Row };

/** Row rendering shares the same model as the keyboard navigation. */
function LibraryTreeRowView({ row, providers, selectedItemId, selectedAttachmentId, pendingItemIds, actions, tabStopKey, toggle, focusRow, onContextMenu }: {
  row: Row;
  providers: readonly ProjectProvider[];
  selectedItemId: string | null;
  selectedAttachmentId: string | null;
  pendingItemIds: ReadonlySet<string>;
  actions: LibraryItemActions;
  tabStopKey: string | null;
  toggle: (key: string) => void;
  focusRow: (key: string) => void;
  onContextMenu: (row: Row) => (event: MouseEvent<HTMLButtonElement>) => void;
}) {
  const attachmentActions = actions.attachments;
  const depth = { "--depth": row.depth } as CSSProperties;
  if (row.kind === "item" || row.kind === "page") {
    const item = row.item;
    const chip = libraryStateChip(item.state);
    const pending = pendingItemIds.has(item.item_id);
    const progress = attachmentProgress(attachmentActions?.active, item.item_id);
    const label = itemTreeLabel(item, providers);
    return <div className={`context-tree-node${row.kind === "page" ? " library-page-node" : ""}`} style={depth} key={`item:${row.key}`}>
      {row.kind === "page" ? <button type="button" tabIndex={-1} className="library-page-disclosure"
        aria-label={`Expand ${item.title}`} aria-expanded={row.open}
        onMouseDown={(event) => event.preventDefault()} onClick={() => { toggle(row.key); focusRow(row.key); }}>
        <UiIcon name={row.open ? "down" : "right"} />
      </button> : null}
      <button type="button" data-library-row={row.key} tabIndex={row.key === tabStopKey ? 0 : -1} data-context-path={item.document_path ?? undefined}
        className={`context-tree-row library-tree-row is-${row.kind}${item.item_id === selectedItemId ? " is-selected" : ""}`}
        aria-current={item.item_id === selectedItemId ? "true" : undefined}
        aria-label={`${itemAccessibleName(item, providers)}${item.purge_after !== null ? ", unreferenced" : ""}${pending ? ", refreshing" : ""}`}
        onClick={() => actions.open(item)} onContextMenu={onContextMenu(row)}>
        {/* A page node's chevron is its own hit target, drawn over this slot. */}
        <span className="library-row-chevron" aria-hidden="true" />
        <span className="library-row-icon is-document" aria-hidden="true"><UiIcon name="file" /></span>
        <span className="context-tree-name" title={item.item_path}>{label}</span>
        {pending ? <PendingPill className="context-tree-meta" word={progress ?? "Refreshing…"} />
          : item.state !== "fresh" ? <StatePill className="context-tree-meta" shape={chip.shape} word={chip.word} tone={chip.tone} /> : null}
        {!pending && item.purge_after !== null ? <StatePill className="context-tree-meta" shape="slash-ring" word="Unreferenced" tone="muted" title={purgeNotice(item)} /> : null}
      </button>
    </div>;
  }
  if (row.kind === "attachment") {
    const { item, attachment } = row;
    const progress = attachmentProgress(attachmentActions?.active, item.item_id, attachment.attachment_id);
    const mark = attachmentMark(attachment.state);
    const size = attachment.bytes === null ? null : byteSize(attachment.bytes);
    const detail = [size, attachment.media_type, progress ?? ATTACHMENT_STATE[attachment.state]].filter(Boolean).join(" · ");
    const content = <>
      <span className="library-row-chevron" aria-hidden="true" />
      <span className={`library-row-icon${attachment.state === "downloaded" ? " is-downloaded" : ""}`} aria-hidden="true"><UiIcon name="file" /></span>
      {/* The safe stored name is shown; the original name only as a tooltip, never as markup (P11). */}
      <span className="context-tree-name" title={attachment.original_name !== attachment.stored_name ? attachment.original_name : undefined}>{attachment.stored_name}</span>
      <span className={`context-tree-meta library-state is-${progress ? "muted" : mark?.tone ?? "muted"}`} title={attachment.media_type ?? undefined}>
        {progress ? <span className="library-spinner" aria-hidden="true" /> : mark?.shape === "check" ? <UiIcon name="check" /> : null}
        {progress ? progress : attachment.state === "downloaded" ? size : [size, ATTACHMENT_STATE[attachment.state]].filter(Boolean).join(" · ")}
      </span>
    </>;
    const selected = attachment.attachment_id === selectedAttachmentId;
    return <div className="context-tree-node" style={depth} key={`attachment:${row.key}`}>
      {attachmentActions
        ? <button type="button" data-library-row={row.key} tabIndex={row.key === tabStopKey ? 0 : -1} data-context-path={attachmentPath(item, attachment) ?? undefined}
          className={`context-tree-row library-tree-row library-attachment-row${selected ? " is-selected" : ""}`}
          aria-current={selected ? "true" : undefined} aria-label={`${attachment.stored_name}, attachment, ${detail}`}
          onClick={() => attachmentActions.open(item, attachment)} onContextMenu={onContextMenu(row)}>{content}</button>
        // Read-only metadata where nothing can download it: arrow keys reach it, with no click, Enter or menu.
        : <div role="group" tabIndex={row.key === tabStopKey ? 0 : -1} data-library-row={row.key} className="context-tree-row library-tree-row library-attachment-row" style={{ cursor: "default" }}
          aria-label={`${attachment.stored_name}, attachment, ${detail}`}>{content}</div>}
    </div>;
  }
  const label = rowLabel(row);
  const follow = row.kind === "container" ? row.node.follow : null;
  const folder = row.kind === "ancestor" && row.node.folder;
  // Pages added one by one read `Pages`; a followed space a `Following` pill, plus `N of M` while partial.
  const meta = row.kind === "container" && !follow && row.node.items.every((item) => isConfluencePage(item) && !hasFollowRef(item)) ? "Pages"
    : row.kind === "attachments" ? attachmentTreeMeta(row.item)
    : folder ? "Folder" : null;
  const partial = follow?.partial ? partialText(follow) : null;
  const instanceNode = row.kind === "instance" ? row.node : null;
  const instanceFamily = instanceNode ? providerFamily(providers, instanceNode.providerId) : null;
  return <div className="context-tree-node" style={depth} key={`${row.kind}:${row.key}`}>
    <button type="button" data-library-row={row.key} tabIndex={row.key === tabStopKey ? 0 : -1} className={`context-tree-row library-tree-row library-tree-group is-${row.kind}${follow ? " has-follow" : ""}`} aria-expanded={row.open}
      aria-label={follow ? `${label}, following${partial ? `, partial: ${partial}` : ""}` : folder ? `${label}, folder` : undefined}
      onClick={() => toggle(row.key)} onContextMenu={onContextMenu(row)}>
      <span className="library-row-chevron" aria-hidden="true"><UiIcon name={row.open ? "down" : "right"} /></span>
      <span className={`library-row-icon${row.kind === "instance" ? " is-tile" : ""}`} aria-hidden="true">
        {instanceNode && instanceFamily ? <ProviderMark family={instanceFamily} size="tree" folder={instanceNode.providerId === null} />
          : <UiIcon name={row.kind === "attachments" ? "clip" : "folder"} />}
      </span>
      <span className="context-tree-name" title={row.kind === "instance" ? row.node.instance ?? label : label}>{label}</span>
      {/* A follow's state sits on a second line: a long query or space name keeps the whole first line, ellipsised only when it truly overflows. */}
      {follow ? <span className="library-follow-line">
        <StatePill className="context-tree-meta" shape="dot-ring" word="Following" tone="idle" />
        {follow.source.kind === "jira_query" ? <span className="context-tree-meta library-state is-muted">{`${followCountText(follow)} · ${follow.source.mode === "live" ? "Live" : "Accumulate"}`}</span> : null}
        {follow.partial ? <StatePill className="context-tree-meta" shape="half-ring" word={`${follow.partial.have} of ${follow.partial.total ?? "?"}`} tone="working" title={partial ?? undefined} /> : null}
      </span> : null}
      {instanceNode?.unavailable ? <StatePill className="context-tree-meta" shape="close" word="Unavailable" tone="blocked" /> : null}
      {meta ? <span className="context-tree-meta library-state is-muted">{meta}</span> : null}
    </button>
  </div>;
}

/** Design §4.10: stopping keeps every item and ends refreshes that add or update from the follow. */
function stoppedFollowing(follow: LibraryFollowSummary): string {
  if (follow.source.kind === "jira_query") return `Stopped following the query. Its ${followCountText(follow)} stay in the Library; refresh no longer adds or drops ${follow.reference_depth ? "items" : "issues"} for it.`;
  return `Stopped following ${follow.source.space_key}. Its ${pageCount(follow.item_count)} stay in the Library; refresh no longer adds new pages.`;
}

const NO_FOLLOWS: readonly LibraryFollowSummary[] = [];

export type LibraryTreeNotice = { text: string; failed: boolean };

/**
 * Library tree (design §4.3): provider instance → container → item, with
 * Confluence pages under their ancestors. A followed space is a container
 * reading `◉ Following` (`◐ N of M` while partial) with its own refresh,
 * stop-following and removal actions. Rows are buttons with the Context
 * tree keys; Shift+F10 or the Menu key opens the row menu. With attachment
 * actions, an attachment row opens a downloaded file under the safe-media rules,
 * or the not-downloaded notice with `Download`, and its menu downloads or removes
 * it; without them attachment rows are read-only metadata that arrow keys reach.
 * Labels are display names, and the full path is the tooltip. A focused row that
 * disappears (a removed space) hands focus to its parent row.
 */
export function LibraryTree({ items, follows = NO_FOLLOWS, providers, selectedItemId, selectedAttachmentId = null, pendingItemIds, actions, onNotice }: {
  items: readonly LibraryItemSummary[];
  /** Followed spaces from the Library listing; each is shown as its space's container. */
  follows?: readonly LibraryFollowSummary[];
  providers: readonly ProjectProvider[];
  selectedItemId: string | null;
  /** The attachment open in the viewer, or whose not-downloaded notice is shown. */
  selectedAttachmentId?: string | null;
  pendingItemIds: ReadonlySet<string>;
  actions: LibraryItemActions;
  /** Follow outcomes go to the Library's status area, never above the rows. */
  onNotice: (notice: LibraryTreeNotice | null) => void;
}) {
  // Keys the user toggled: other rows start open, `Attachments (N)` groups start folded.
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const [menu, setMenu] = useState<Menu | null>(null);
  const [removing, setRemoving] = useState<LibraryFollowSummary | null>(null);
  const tree = useMemo(() => libraryTree(items, providers, follows), [follows, items, providers]);
  const rows = useMemo(() => libraryTreeRows(tree, providers, collapsed), [collapsed, tree, providers]);
  const toggle = (key: string) => setCollapsed((current) => {
    const next = new Set(current);
    if (next.has(key)) next.delete(key); else next.add(key);
    return next;
  });
  const removeFollow = actions.removeFollow;
  const stopFollowing = (follow: LibraryFollowSummary) => {
    if (!removeFollow) return;
    onNotice(null);
    removeFollow(follow, "stop_following").then(() => onNotice({ text: stoppedFollowing(follow), failed: false }), (cause: unknown) => {
      onNotice({ text: `Still following ${follow.source.kind === "jira_query" ? "the query" : follow.source.space_key}. ${errorText(cause, "Stopping could not be completed.")}`, failed: true });
    });
  };
  const attachmentActions = actions.attachments;
  const menuEntries = (row: Row): LibraryMenuEntry[] => {
    if (row.kind === "item" || row.kind === "page") return itemMenuEntries(row.item, actions, true);
    if (row.kind === "attachments") return attachmentMenuEntries(row.item, attachmentActions, actions.credentials);
    if (row.kind === "attachment") {
      if (!attachmentActions) return [];
      const { item, attachment } = row;
      const ids = [attachment.attachment_id];
      if (attachment.state === "downloaded") return [
        { label: "Open", onSelect: () => attachmentActions.open(item, attachment), disabled: attachmentPath(item, attachment) === null },
        { label: "Remove download", onSelect: () => attachmentActions.start(item, "remove_downloaded", ids), disabled: attachmentActions.busy },
      ];
      return attachment.state === "over_limit" ? [] : [{ label: "Download", onSelect: () => attachmentActions.start(item, "download", ids), disabled: attachmentActions.busy }];
    }
    const ids = row.kind === "instance" ? row.node.containers.flatMap((container) => container.items.map((item) => item.item_id))
      : row.kind === "container" ? row.node.items.map((item) => item.item_id) : pageItemIds(row.node);
    const follow = row.kind === "container" ? row.node.follow : null;
    if (follow) {
      // Design §5.3: `Refresh space` · `Stop following` · separator · `Remove space from Library…`; a query reads the same.
      const noun = follow.source.kind === "jira_query" ? "query" : "space";
      return [
        { label: `Refresh ${noun}`, onSelect: () => actions.refresh({ scope: "follow", follow_id: follow.follow_id }, ids), disabled: actions.refreshBusy },
        ...(removeFollow ? [
          { label: "Stop following", onSelect: () => stopFollowing(follow) },
          "separator" as const,
          { label: `Remove ${noun} and its items…`, onSelect: () => { onNotice(null); setRemoving(follow); }, destructive: true },
        ] : []),
      ];
    }
    const request: LibraryRefreshRequest = row.kind === "container" && row.node.instance && row.node.containerId
      ? { scope: "container", provider_instance: row.node.instance, container_id: row.node.containerId }
      : { scope: "items", item_ids: ids };
    const refreshAll: LibraryMenuEntry = { label: `Refresh all in ${rowLabel(row)}`, onSelect: () => actions.refresh(request, ids), disabled: actions.refreshBusy || ids.length === 0 };
    return [refreshAll];
  };
  const openMenu = (row: Row, x: number, y: number) => {
    // The token states are read when a row that depends on them is first acted on, not when the tree renders.
    if (row.kind === "item" || row.kind === "page" || row.kind === "attachments") actions.credentials?.ensure();
    if (menuEntries(row).length === 0) return;
    setMenu({ x, y, row });
  };
  const { listRef, tabStopKey, focusRow, onKeyDown, onFocus, onBlur } = useLibraryTreeFocus({
    rows, selectedItemId, selectedAttachmentId, actions, toggle,
    openMenu: (row, target) => {
      const anchor = menuAnchor(target);
      openMenu(row, anchor.x, anchor.y);
    },
  });
  const onContextMenu = (row: Row) => (event: MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.focus();
    openMenu(row, event.clientX, event.clientY);
  };
  return <div className="library-tree" ref={listRef} onKeyDown={onKeyDown} onFocus={onFocus} onBlur={onBlur}>
    {rows.map((row) => <LibraryTreeRowView key={`${row.kind === "page" ? "item" : row.kind}:${row.key}`}
      row={row} providers={providers} selectedItemId={selectedItemId} selectedAttachmentId={selectedAttachmentId}
      pendingItemIds={pendingItemIds} actions={actions} tabStopKey={tabStopKey} toggle={toggle} focusRow={focusRow} onContextMenu={onContextMenu} />)}
    {menu ? <LibraryMenu x={menu.x} y={menu.y} label={`${rowLabel(menu.row)} actions`} entries={menuEntries(menu.row)} onDismiss={() => setMenu(null)} /> : null}
    {removing && removeFollow ? <FollowRemoveDialog follow={removing} onClose={() => setRemoving(null)} remove={async (mode) => {
      await removeFollow(removing, mode);
      setRemoving(null);
      onNotice({ text: mode === "stop_following" ? stoppedFollowing(removing) : `Removed ${followTitle(removing)} from the Library.`, failed: false });
    }} /> : null}
  </div>;
}
