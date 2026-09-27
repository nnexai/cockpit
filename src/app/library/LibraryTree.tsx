import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import type { LibraryItemSummary, LibraryRefreshRequest, ProjectProvider } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { itemAccessibleName, itemTreeLabel, libraryStateChip, libraryTree, type LibraryContainerNode, type LibraryInstanceNode } from "./libraryState";
import "./library.css";

export type LibraryMenuEntry = { label: string; onSelect: () => void; disabled?: boolean; destructive?: boolean } | "separator";

const MENU_WIDTH = 286;
const MENU_GUTTER = 8;

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
      : <button key={entry.label} type="button" role="menuitem" className={entry.destructive ? "destructive" : undefined} disabled={entry.disabled} onClick={() => { onDismiss(); entry.onSelect(); }}>{entry.label}</button>)}
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
  refreshBusy: boolean;
};

/** The same entries appear in the row context menu and the item header `⋯` (design §5.3). */
export function itemMenuEntries(item: LibraryItemSummary, actions: LibraryItemActions, includeOpen: boolean): LibraryMenuEntry[] {
  return [
    ...(includeOpen ? [{ label: "Open", onSelect: () => actions.open(item), disabled: !item.document_path }] : []),
    { label: "Refresh from source", onSelect: () => actions.refresh({ scope: "items", item_ids: [item.item_id] }, [item.item_id]), disabled: actions.refreshBusy },
    { label: "Copy source link", onSelect: () => actions.copyLink(item), disabled: !actions.canCopyLink || !(item.source_url ?? item.original_url) },
    "separator",
    { label: "Remove from Library…", onSelect: () => actions.remove(item), destructive: true },
  ];
}

type Row =
  | { kind: "instance"; key: string; depth: 0; parent: null; node: LibraryInstanceNode; open: boolean }
  | { kind: "container"; key: string; depth: 1; parent: string; node: LibraryContainerNode; open: boolean }
  | { kind: "item"; key: string; depth: number; parent: string; item: LibraryItemSummary };

type Menu = { x: number; y: number; row: Row };

/**
 * Library tree (design §4.3): provider instance → container → item. Rows are
 * buttons with the Context tree keys; Shift+F10 or the Menu key opens the
 * row menu. Labels are display names, and the full path is the tooltip.
 */
export function LibraryTree({ items, providers, selectedItemId, pendingItemIds, actions }: {
  items: readonly LibraryItemSummary[];
  providers: readonly ProjectProvider[];
  selectedItemId: string | null;
  pendingItemIds: ReadonlySet<string>;
  actions: LibraryItemActions;
}) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const [menu, setMenu] = useState<Menu | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const tree = useMemo(() => libraryTree(items, providers), [items, providers]);
  const rows = useMemo(() => {
    const visible: Row[] = [];
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
        if (containerOpen) for (const item of container.items) visible.push({ kind: "item", key: item.item_id, depth: 2, parent: container.key, item });
      }
    }
    return visible;
  }, [collapsed, tree]);
  const focusRow = (key: string) => window.requestAnimationFrame(() => {
    [...(listRef.current?.querySelectorAll<HTMLButtonElement>("[data-library-row]") ?? [])].find((button) => button.dataset.libraryRow === key)?.focus();
  });
  const toggle = (key: string) => setCollapsed((current) => {
    const next = new Set(current);
    if (next.has(key)) next.delete(key); else next.add(key);
    return next;
  });
  const groupIds = (row: Row): string[] => row.kind === "instance"
    ? row.node.containers.flatMap((container) => container.items.map((item) => item.item_id))
    : row.kind === "container" ? row.node.items.map((item) => item.item_id) : [row.item.item_id];
  const menuEntries = (row: Row): LibraryMenuEntry[] => {
    if (row.kind === "item") return itemMenuEntries(row.item, actions, true);
    const ids = groupIds(row);
    const request: LibraryRefreshRequest = row.kind === "container" && row.node.instance && row.node.containerId
      ? { scope: "container", provider_instance: row.node.instance, container_id: row.node.containerId }
      : { scope: "items", item_ids: ids };
    return [{ label: `Refresh all in ${row.node.label}`, onSelect: () => actions.refresh(request, ids), disabled: actions.refreshBusy || ids.length === 0 }];
  };
  const openMenu = (row: Row, x: number, y: number) => setMenu({ x, y, row });
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = event.target instanceof HTMLElement ? event.target.closest<HTMLButtonElement>("[data-library-row]") : null;
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
    if (event.key === "ArrowRight" && row.kind !== "item") {
      event.preventDefault();
      if (!row.open) toggle(row.key);
      else if (rows[index + 1]?.depth > row.depth) focusRow(rows[index + 1]!.key);
      return;
    }
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      if (row.kind !== "item" && row.open) toggle(row.key);
      else if (row.parent) focusRow(row.parent);
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      if (row.kind === "item") actions.open(row.item); else toggle(row.key);
    }
  };
  const onContextMenu = (row: Row) => (event: MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.focus();
    openMenu(row, event.clientX, event.clientY);
  };
  return <div className="library-tree" ref={listRef} onKeyDown={onKeyDown}>
    {rows.map((row) => {
      const indent = { paddingLeft: `${8 + row.depth * 16}px` };
      if (row.kind === "item") {
        const item = row.item;
        const chip = libraryStateChip(item.state);
        const pending = pendingItemIds.has(item.item_id);
        const label = itemTreeLabel(item, providers);
        return <div className="context-tree-node" key={`item:${row.key}`}>
          <button type="button" data-library-row={row.key} data-context-path={item.document_path ?? undefined}
            className={`context-tree-row library-tree-row${item.item_id === selectedItemId ? " is-selected" : ""}`} style={indent}
            aria-current={item.item_id === selectedItemId ? "true" : undefined}
            aria-label={`${itemAccessibleName(item, providers)}${pending ? ", refreshing" : ""}`}
            onClick={() => actions.open(item)} onContextMenu={onContextMenu(row)}>
            <span className="context-tree-disclosure" />
            <span className="context-tree-icon" aria-hidden="true"><UiIcon name="file" /></span>
            <span className="context-tree-name" title={item.item_path}>{label}</span>
            {pending ? <span className="context-tree-meta library-state is-muted"><span className="library-spinner" aria-hidden="true" />Refreshing…</span>
              : item.state !== "fresh" ? <span className={`context-tree-meta library-state is-${chip.tone}`}><span aria-hidden="true">{chip.glyph}</span> {chip.word}</span> : null}
          </button>
        </div>;
      }
      const meta = row.kind === "instance" && row.node.unavailable ? "Unavailable" : null;
      return <div className="context-tree-node" key={`${row.kind}:${row.key}`}>
        <button type="button" data-library-row={row.key} className={`context-tree-row library-tree-group is-${row.kind}`} style={indent} aria-expanded={row.open}
          onClick={() => toggle(row.key)} onContextMenu={onContextMenu(row)}>
          <span className="context-tree-disclosure"><UiIcon name={row.open ? "down" : "right"} /></span>
          <span className="context-tree-name" title={row.kind === "instance" ? row.node.instance ?? row.node.label : row.node.label}>{row.node.label}</span>
          {meta ? <span className="context-tree-meta library-state is-blocked">{meta}</span> : null}
        </button>
      </div>;
    })}
    {menu ? <LibraryMenu x={menu.x} y={menu.y} label={menu.row.kind === "item" ? `${menu.row.item.title} actions` : `${menu.row.node.label} actions`} entries={menuEntries(menu.row)} onDismiss={() => setMenu(null)} /> : null}
  </div>;
}
