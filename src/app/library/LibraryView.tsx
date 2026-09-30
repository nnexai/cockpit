import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import { ContextViewer, createContextViewState, type LibraryCommand } from "../context/ContextViewer";
import { UiIcon } from "../UiIcon";
import type { LibrarySpace } from "./libraryState";
import { useLibraryListing } from "./useLibraryOperation";
import "./library.css";

/** Safe workbench targets that never send a Herdr request when focused (D13). */
const SAFE_FOCUS_TARGETS = ['.tab-button[aria-selected="true"]:not(:disabled)', '.tab-icon-button[aria-controls="cockpit-sidebar"]:not(:disabled)', ".drawer-toggle"];

/** At this view width the title row drops its item count. */
const COMPACT_WIDTH = 520;

function restoreWorkbenchFocus(invoker: HTMLElement | null): void {
  if (invoker?.isConnected && !invoker.closest("[inert]")) {
    invoker.focus({ preventScroll: true });
    if (document.activeElement === invoker) return;
  }
  for (const selector of SAFE_FOCUS_TARGETS) {
    const target = document.querySelector<HTMLElement>(selector);
    if (!target || target.closest(".library-view, [inert]")) continue;
    target.focus({ preventScroll: true });
    if (document.activeElement === target) return;
  }
  if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
}

/**
 * The Cockpit-owned Library view (design §4.1). It is not a Herdr pane or tab
 * and sends no Herdr request. The workbench unmounts the covered panes while
 * it is open; on close, DOM focus returns to the invoker if it is still
 * mounted, otherwise to a safe workbench target, before the panes attach again.
 */
export function LibraryView({ client, onClose, command = null, fullScreen = false, space = null }: {
  client: CockpitClient;
  onClose: () => void;
  command?: LibraryCommand | null;
  /** Herdr's selected Space: the `Add to <Space>` target. Null with no session or Space. */
  space?: LibrarySpace | null;
  /** No session: the view fills the window and is Library-only. */
  fullScreen?: boolean;
}) {
  const rootRef = useRef<HTMLElement>(null);
  const invokerRef = useRef<HTMLElement | null>(null);
  const treeFocusedRef = useRef(false);
  const [view, setView] = useState(createContextViewState);
  const library = useLibraryListing(client, true);
  const listing = library.listing;
  // `12 items · 1 follow`; `12+ items` while the listing has more pages.
  const summary = listing ? [
    `${listing.items.length}${listing.next_offset === null ? "" : "+"} ${listing.items.length === 1 && listing.next_offset === null ? "item" : "items"}`,
    listing.follows.length > 0 ? `${listing.follows.length} ${listing.follows.length === 1 ? "follow" : "follows"}` : null,
  ].filter(Boolean).join(" · ") : null;
  const [compact, setCompact] = useState(false);
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root || typeof ResizeObserver === "undefined") return;
    const measure = () => { const width = root.getBoundingClientRect().width; if (width > 0) setCompact(width <= COMPACT_WIDTH); };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(root);
    return () => observer.disconnect();
  }, []);
  // Passive, so a closing palette has already restored focus to its opener.
  useEffect(() => {
    const active = document.activeElement;
    invokerRef.current = active instanceof HTMLElement && active !== document.body && !rootRef.current?.contains(active) ? active : null;
    rootRef.current?.focus({ preventScroll: true });
  }, []);
  // Focus leaves the view before it is removed, so the reattached panes never inherit a lost focus.
  useLayoutEffect(() => {
    const root = rootRef.current;
    return () => { if (root?.contains(document.activeElement)) restoreWorkbenchFocus(invokerRef.current); };
  }, []);
  // A palette closing in the same commit leaves focus on the document body.
  useEffect(() => () => {
    if (document.activeElement === null || document.activeElement === document.body) restoreWorkbenchFocus(invokerRef.current);
  }, []);
  useEffect(() => {
    if (treeFocusedRef.current || (library.status !== "ready" && library.status !== "error")) return;
    treeFocusedRef.current = true;
    if (document.activeElement !== rootRef.current) return;
    // Focus must land inside the viewer, or its keys (Ctrl+P, Alt+1…) don't reach it: a row, else the empty state's Add…, the error's Retry, or the tree itself.
    const view = rootRef.current;
    const target = view?.querySelector<HTMLElement>(".library-tree-row.is-selected") ?? view?.querySelector<HTMLElement>("[data-library-row]")
      ?? view?.querySelector<HTMLElement>(".library-tree-empty button, .library-status-area button") ?? view?.querySelector<HTMLElement>(".context-tree");
    target?.focus({ preventScroll: true });
    if (document.activeElement === view) view?.querySelector<HTMLElement>(".context-document")?.focus({ preventScroll: true });
  }, [library.status]);
  return <section ref={rootRef} className={`library-view${fullScreen ? " is-full-screen" : ""}`} aria-label="Library" tabIndex={-1} onKeyDown={(event) => {
    if (event.key !== "Escape" || event.defaultPrevented || event.nativeEvent.isComposing) return;
    event.preventDefault();
    onClose();
  }}>
    <header className="library-view-header">
      <span className="library-view-icon" aria-hidden="true"><UiIcon name="library" /></span>
      <h2 title={library.listing?.root.path}>Library</h2>
      {summary && !compact ? <span className="library-view-summary">{summary}</span> : null}
      <span className="context-toolbar-spacer" />
      <button type="button" className="library-icon-button" onClick={onClose} aria-label="Close Library" title="Close Library (Esc)"><UiIcon name="close" /></button>
    </header>
    <ContextViewer client={client} context={null} value={view} onChange={setView} library={library} libraryCommand={command} space={space} />
  </section>;
}
