import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type FocusEvent, type KeyboardEvent, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import type { LibraryAttachment, LibraryAttachmentAction, LibraryAttachmentRequest, LibraryFollowSummary, LibraryItemSummary, LibraryRefreshRequest, ProjectProvider } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { FollowRemoveDialog, type FollowRemoveMode } from "./LibraryConfirmDialog";
import { attachmentTreeMeta, errorText, followCountText, followTitle, hasFollowRef, isConfluencePage, isKeepable, itemAccessibleName, itemTreeLabel, libraryStateChip, libraryTree, nestUnderParents, pageCount, partialText, providerFamily, purgeNotice, type LibraryContainerNode, type LibraryInstanceNode, type LibraryItemNode, type StateShape, type StateTone } from "./libraryState";
import { ProviderMark } from "./ProviderMark";
import { PendingPill, StatePill } from "./StatePill";
import type { ProviderCredentialActions } from "./useProviderCredentials";
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
  if (jira === "needs_token") return [{ label: "Store a token to download attachments…", onSelect: () => credentials?.open(item.provider_id ?? "") }];
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

/**
 * A Confluence page's place under its space: a Library page, or an ancestor
 * known only by its title. In a completely enumerated followed space every
 * page is in the Library, so an ancestor that isn't, and wasn't removed from
 * it, is a non-document node such as a Cloud folder.
 */
type PageNode = { key: string; title: string; item: LibraryItemSummary | null; folder: boolean; children: PageNode[] };

/**
 * Pages beneath their space (design §4.3). Each page's ancestor chain, root
 * first, is its path, so a page sits under the Library copy of its parent, or
 * under the parent's title when only the child was added.
 */
function pageForest(container: LibraryContainerNode): PageNode[] {
  const nodes = new Map<string, PageNode>();
  const roots: PageNode[] = [];
  for (const item of container.items) {
    const chain = [...item.ancestors, { id: item.canonical_id ?? item.item_id, title: item.title }];
    let parent: PageNode | null = null;
    for (const [index, page] of chain.entries()) {
      let node = nodes.get(page.id);
      if (!node) {
        node = { key: `${container.key}\u0000page:${page.id}`, title: page.title, item: null, folder: false, children: [] };
        nodes.set(page.id, node);
        (parent?.children ?? roots).push(node);
      }
      if (index === chain.length - 1) Object.assign(node, { key: item.item_id, title: item.title, item });
      parent = node;
    }
  }
  const follow = container.follow;
  if (follow && !follow.partial) {
    const excluded = new Set(follow.excluded_ids);
    for (const [id, node] of nodes) node.folder = !node.item && !excluded.has(id);
  }
  return roots;
}

function pageItemIds(page: PageNode): string[] {
  return [...(page.item ? [page.item.item_id] : []), ...page.children.flatMap(pageItemIds)];
}

/** An issue and the subtasks nested under it, as page nodes so both share the page rows' chevron, Enter and attachments. */
function issueNode(node: LibraryItemNode): PageNode {
  return { key: node.item.item_id, title: node.item.title, item: node.item, folder: false, children: node.children.map(issueNode) };
}

type Row =
  | { kind: "instance"; key: string; depth: 0; parent: null; node: LibraryInstanceNode; open: boolean }
  | { kind: "container"; key: string; depth: 1; parent: string; node: LibraryContainerNode; open: boolean }
  /** An ancestor page that isn't in the Library itself, or a followed space's folder: a plain group. */
  | { kind: "ancestor"; key: string; depth: number; parent: string; node: PageNode; open: boolean }
  /** A Library page with child pages or attachments: the chevron expands, the label opens (design §4.3). */
  | { kind: "page"; key: string; depth: number; parent: string; node: PageNode; item: LibraryItemSummary; open: boolean }
  | { kind: "item"; key: string; depth: number; parent: string; item: LibraryItemSummary }
  /** A page's `Attachments (N)` group, after its child pages. */
  | { kind: "attachments"; key: string; depth: number; parent: string; item: LibraryItemSummary; open: boolean }
  /** One attachment: opens when downloaded, else its not-downloaded notice; read-only metadata where nothing can download it. */
  | { kind: "attachment"; key: string; depth: number; parent: string; item: LibraryItemSummary; attachment: LibraryAttachment };

type Menu = { x: number; y: number; row: Row };

function rowLabel(row: Row): string {
  switch (row.kind) {
    case "item": case "page": return row.item.title;
    case "attachments": return "Attachments";
    case "attachment": return row.attachment.stored_name;
    case "ancestor": return row.node.title;
    default: return row.node.label;
  }
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
  const listRef = useRef<HTMLDivElement>(null);
  const focusedRow = useRef<{ key: string; parent: string | null; index: number } | null>(null);
  // The row that last held focus stays the tree's one tab stop (roving tabindex); until then the selected row, else the first.
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const tree = useMemo(() => libraryTree(items, providers, follows), [follows, items, providers]);
  const rows = useMemo(() => {
    const visible: Row[] = [];
    const pushPages = (pages: readonly PageNode[], depth: number, parent: string) => {
      for (const page of pages) {
        const open = !collapsed.has(page.key);
        const attachments = page.item?.attachments ?? [];
        if (page.item && page.children.length === 0 && attachments.length === 0) visible.push({ kind: "item", key: page.key, depth, parent, item: page.item });
        else if (page.item) visible.push({ kind: "page", key: page.key, depth, parent, node: page, item: page.item, open });
        else visible.push({ kind: "ancestor", key: page.key, depth, parent, node: page, open });
        if (!open) continue;
        pushPages(page.children, depth + 1, page.key);
        if (!page.item || attachments.length === 0) continue;
        const groupKey = `${page.key}\u0000attachments`;
        const groupOpen = collapsed.has(groupKey);
        visible.push({ kind: "attachments", key: groupKey, depth: depth + 1, parent: page.key, item: page.item, open: groupOpen });
        if (groupOpen) for (const attachment of attachments) visible.push({ kind: "attachment", key: attachment.attachment_id, depth: depth + 2, parent: groupKey, item: page.item, attachment });
      }
    };
    for (const instance of tree) {
      const instanceOpen = !collapsed.has(instance.key);
      visible.push({ kind: "instance", key: instance.key, depth: 0, parent: null, node: instance, open: instanceOpen });
      if (!instanceOpen) continue;
      for (const container of instance.containers) {
        // Folders have no container level.
        if (!container.label) {
          for (const item of container.items) visible.push({ kind: "item", key: item.item_id, depth: 1, parent: instance.key, item });
          continue;
        }
        const containerOpen = !collapsed.has(container.key);
        visible.push({ kind: "container", key: container.key, depth: 1, parent: instance.key, node: container, open: containerOpen });
        if (!containerOpen) continue;
        if (container.items.every(isConfluencePage)) pushPages(pageForest(container), 2, container.key);
        // Other issues nest under their parent (a Jira subtask); one with attachments or subtasks gets a page row, the rest are leaves.
        else pushPages(nestUnderParents(container.items, providers).map(issueNode), 2, container.key);
      }
    }
    return visible;
  }, [collapsed, tree, providers]);
  const rowElement = (key: string) => [...(listRef.current?.querySelectorAll<HTMLElement>("[data-library-row]") ?? [])].find((element) => element.dataset.libraryRow === key);
  const focusRow = (key: string) => rowElement(key)?.focus();
  const tabStopKey = (activeKey !== null && rows.some((row) => row.key === activeKey) ? activeKey : null)
    ?? rows.find((row) => (row.kind === "item" || row.kind === "page") && row.item.item_id === selectedItemId || row.kind === "attachment" && row.key === selectedAttachmentId)?.key
    ?? rows[0]?.key ?? null;
  const tabStopRef = useRef<string | null>(null);
  tabStopRef.current = tabStopKey;
  // Focus lost to <body> (a button that disabled itself while its refresh ran, a closed menu or dialog) would leave the arrow keys dead until a click: the first navigation key takes the tab stop back.
  useEffect(() => {
    const recover = (event: globalThis.KeyboardEvent) => {
      if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || event.isComposing) return;
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Home" && event.key !== "End") return;
      if (document.activeElement !== null && document.activeElement !== document.body) return;
      const key = tabStopRef.current;
      const target = listRef.current && listRef.current.getClientRects().length > 0 && key !== null ? rowElement(key) : undefined;
      if (!target) return;
      event.preventDefault();
      target.focus();
    };
    document.addEventListener("keydown", recover);
    return () => document.removeEventListener("keydown", recover);
  }, []);
  // A focused row removed by a reread (a removed space or item) leaves focus on the body: hand it to the parent row.
  useLayoutEffect(() => {
    const last = focusedRow.current;
    if (!last || (document.activeElement !== null && document.activeElement !== document.body) || rows.some((row) => row.key === last.key)) return;
    const next = rows.find((row) => row.key === last.parent) ?? rows[Math.min(last.index, rows.length - 1)];
    if (!next) return;
    rowElement(next.key)?.focus({ preventScroll: true });
  }, [rows]);
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
    // A provider instance offers its token dialog where the provider can store one.
    const tokenFamily = row.kind === "instance" && row.node.providerId ? providerFamily(providers, row.node.providerId).key : null;
    return row.kind === "instance" && row.node.providerId && actions.credentials && (tokenFamily === "jira" || tokenFamily === "confluence")
      ? [refreshAll, "separator", { label: "Provider token…", onSelect: () => actions.credentials?.open(row.node.providerId!) }]
      : [refreshAll];
  };
  const openMenu = (row: Row, x: number, y: number) => {
    // The token states are read when a row that depends on them is first acted on, not when the tree renders.
    if (row.kind === "item" || row.kind === "page" || row.kind === "attachments") actions.credentials?.ensure();
    if (menuEntries(row).length === 0) return;
    setMenu({ x, y, row });
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = event.target instanceof HTMLElement ? event.target.closest<HTMLElement>("[data-library-row]") : null;
    const index = target ? rows.findIndex((row) => row.key === target.dataset.libraryRow) : -1;
    const row = rows[index];
    if (!target || !row || event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
      event.preventDefault();
      const anchor = menuAnchor(target);
      openMenu(row, anchor.x, anchor.y);
      return;
    }
    if (event.shiftKey) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const next = event.key === "Home" ? rows[0] : event.key === "End" ? rows.at(-1)
        : rows[Math.max(0, Math.min(rows.length - 1, index + (event.key === "ArrowDown" ? 1 : -1)))];
      if (next) focusRow(next.key);
      return;
    }
    if (event.key === "ArrowRight" && "open" in row) {
      event.preventDefault();
      if (!row.open) toggle(row.key);
      else if (rows[index + 1]?.depth > row.depth) focusRow(rows[index + 1]!.key);
      return;
    }
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      if ("open" in row && row.open) toggle(row.key);
      else if (row.parent) focusRow(row.parent);
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      // Only page nodes differ from a directory: Enter opens the page (design §4.3), and an attachment its file or notice.
      if (row.kind === "item" || row.kind === "page") actions.open(row.item);
      else if (row.kind === "attachment") attachmentActions?.open(row.item, row.attachment);
      else toggle(row.key);
    }
  };
  const onContextMenu = (row: Row) => (event: MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.focus();
    openMenu(row, event.clientX, event.clientY);
  };
  const onFocus = (event: FocusEvent<HTMLDivElement>) => {
    const key = event.target instanceof HTMLElement ? event.target.closest<HTMLElement>("[data-library-row]")?.dataset.libraryRow : undefined;
    const index = key === undefined ? -1 : rows.findIndex((row) => row.key === key);
    focusedRow.current = index < 0 ? null : { key: key!, parent: rows[index]!.parent, index };
    if (index >= 0) setActiveKey(key!);
  };
  // Focus moving elsewhere (a menu, a dialog, another pane) is no longer the tree's to restore.
  const onBlur = (event: FocusEvent<HTMLDivElement>) => { if (event.relatedTarget) focusedRow.current = null; };
  return <div className="library-tree" ref={listRef} onKeyDown={onKeyDown} onFocus={onFocus} onBlur={onBlur}>
    {rows.map((row) => {
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
    })}
    {menu ? <LibraryMenu x={menu.x} y={menu.y} label={`${rowLabel(menu.row)} actions`} entries={menuEntries(menu.row)} onDismiss={() => setMenu(null)} /> : null}
    {removing && removeFollow ? <FollowRemoveDialog follow={removing} onClose={() => setRemoving(null)} remove={async (mode) => {
      await removeFollow(removing, mode);
      setRemoving(null);
      onNotice({ text: mode === "stop_following" ? stoppedFollowing(removing) : `Removed ${followTitle(removing)} from the Library.`, failed: false });
    }} /> : null}
  </div>;
}
