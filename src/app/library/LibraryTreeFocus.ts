import { useEffect, useLayoutEffect, useRef, useState, type FocusEvent, type KeyboardEvent } from "react";
import type { LibraryItemActions } from "./LibraryTree";
import type { LibraryTreeRow } from "./LibraryTreeRows";

/** Roving tab stop, tree navigation, and recovery when a focused row disappears. */
export function useLibraryTreeFocus({ rows, selectedItemId, selectedAttachmentId, actions, toggle, openMenu }: {
  rows: readonly LibraryTreeRow[];
  selectedItemId: string | null;
  selectedAttachmentId: string | null;
  actions: LibraryItemActions;
  toggle: (key: string) => void;
  openMenu: (row: LibraryTreeRow, target: HTMLElement) => void;
}) {
  const listRef = useRef<HTMLDivElement>(null);
  const focusedRow = useRef<{ key: string; parent: string | null; index: number } | null>(null);
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const rowElement = (key: string) => [...(listRef.current?.querySelectorAll<HTMLElement>("[data-library-row]") ?? [])].find((element) => element.dataset.libraryRow === key);
  const focusRow = (key: string) => rowElement(key)?.focus();
  const tabStopKey = (activeKey !== null && rows.some((row) => row.key === activeKey) ? activeKey : null)
    ?? rows.find((row) => (row.kind === "item" || row.kind === "page") && row.item.item_id === selectedItemId || row.kind === "attachment" && row.key === selectedAttachmentId)?.key
    ?? rows[0]?.key ?? null;
  const tabStopRef = useRef<string | null>(null);
  tabStopRef.current = tabStopKey;
  // Body focus after a disabled button, menu or dialog would otherwise strand navigation.
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
  useLayoutEffect(() => {
    const last = focusedRow.current;
    if (!last || (document.activeElement !== null && document.activeElement !== document.body) || rows.some((row) => row.key === last.key)) return;
    const next = rows.find((row) => row.key === last.parent) ?? rows[Math.min(last.index, rows.length - 1)];
    if (!next) return;
    rowElement(next.key)?.focus({ preventScroll: true });
  }, [rows]);
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = event.target instanceof HTMLElement ? event.target.closest<HTMLElement>("[data-library-row]") : null;
    const index = target ? rows.findIndex((row) => row.key === target.dataset.libraryRow) : -1;
    const row = rows[index];
    if (!target || !row || event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
      event.preventDefault();
      openMenu(row, target);
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
      if (row.kind === "item" || row.kind === "page") actions.open(row.item);
      else if (row.kind === "attachment") actions.attachments?.open(row.item, row.attachment);
      else toggle(row.key);
    }
  };
  const onFocus = (event: FocusEvent<HTMLDivElement>) => {
    const key = event.target instanceof HTMLElement ? event.target.closest<HTMLElement>("[data-library-row]")?.dataset.libraryRow : undefined;
    const index = key === undefined ? -1 : rows.findIndex((row) => row.key === key);
    focusedRow.current = index < 0 ? null : { key: key!, parent: rows[index]!.parent, index };
    if (index >= 0) setActiveKey(key!);
  };
  // A menu, dialog or another pane owns focus once it moves outside the tree.
  const onBlur = (event: FocusEvent<HTMLDivElement>) => { if (event.relatedTarget) focusedRow.current = null; };
  return { listRef, tabStopKey, focusRow, onKeyDown, onFocus, onBlur };
}
