import { useCallback, useRef, useState, type FocusEvent, type KeyboardEvent } from "react";
import { returnFocusFromSidebar } from "../input/shortcuts";

/**
 * One tab stop for a list of row buttons (`[data-row-id]`). Arrow/Home/End keys move DOM focus
 * only; they never send a Herdr request. Enter and Space are the buttons' own click.
 *
 * The tab stop is the row that holds focus, else the selected row, else the first row.
 */
export function useRovingList({ rowIds, selectedId, onKey, onEscape }: {
  rowIds: readonly string[];
  selectedId: string | null;
  /** Extra keys for a focused row (→ ← Menu). Return true when the key was handled. */
  onKey?: (event: KeyboardEvent<HTMLElement>, rowId: string) => boolean;
  /** Clears a row note. Return true when there was one; otherwise Esc returns focus out of the sidebar. */
  onEscape?: () => boolean;
}) {
  const listRef = useRef<HTMLDivElement | null>(null);
  const [activeId, setActiveId] = useState<string | null>(null);
  const tabStopId = activeId !== null && rowIds.includes(activeId)
    ? activeId
    : selectedId !== null && rowIds.includes(selectedId) ? selectedId : rowIds[0] ?? null;

  const rowElements = useCallback(() => [...listRef.current?.querySelectorAll<HTMLElement>("[data-row-id]") ?? []], []);
  const focusRow = useCallback((id: string | undefined) => {
    rowElements().find((element) => element.dataset.rowId === id)?.focus();
  }, [rowElements]);

  return {
    listRef,
    tabIndexFor: (id: string) => id === tabStopId ? 0 : -1,
    /** Focus the tab-stop row (the selected row, else the first). */
    focusTarget: () => focusRow(tabStopId ?? undefined),
    focusRow,
    listProps: {
      onFocus: (event: FocusEvent<HTMLElement>) => {
        const id = (event.target as HTMLElement).dataset.rowId;
        if (id) setActiveId(id);
      },
      onBlur: (event: FocusEvent<HTMLElement>) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setActiveId(null);
      },
      onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
        const row = event.target as HTMLElement;
        const id = row.dataset.rowId;
        if (id === undefined || event.ctrlKey || event.altKey || event.metaKey) return;
        if (event.key === "Escape") {
          event.preventDefault();
          if (!onEscape?.()) returnFocusFromSidebar();
          return;
        }
        if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Home" || event.key === "End") {
          const rows = rowElements();
          const index = rows.indexOf(row);
          const next = event.key === "Home" ? 0
            : event.key === "End" ? rows.length - 1
            : Math.max(0, Math.min(rows.length - 1, index + (event.key === "ArrowDown" ? 1 : -1)));
          event.preventDefault();
          rows[next]?.focus();
          return;
        }
        if (onKey?.(event, id)) event.preventDefault();
      },
    },
  };
}
