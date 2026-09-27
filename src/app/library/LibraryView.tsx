import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import { ContextViewer, createContextViewState, type LibraryCommand } from "../context/ContextViewer";
import { useLibraryListing } from "./useLibraryOperation";
import "./library.css";

/** Safe workbench targets that never send a Herdr request when focused (D13). */
const SAFE_FOCUS_TARGETS = ['.tab-button[aria-selected="true"]:not(:disabled)', ".tab-sidebar-toggle:not(:disabled)", ".drawer-toggle"];

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
export function LibraryView({ client, onClose, command = null, fullScreen = false }: {
  client: CockpitClient;
  onClose: () => void;
  command?: LibraryCommand | null;
  /** No session: the view fills the window and is Library-only. */
  fullScreen?: boolean;
}) {
  const rootRef = useRef<HTMLElement>(null);
  const invokerRef = useRef<HTMLElement | null>(null);
  const treeFocusedRef = useRef(false);
  const [view, setView] = useState(createContextViewState);
  const library = useLibraryListing(client, true);
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
    if (treeFocusedRef.current || library.status !== "ready") return;
    treeFocusedRef.current = true;
    if (document.activeElement !== rootRef.current) return;
    const rows = rootRef.current?.querySelectorAll<HTMLElement>("[data-library-row]");
    (rootRef.current?.querySelector<HTMLElement>(".library-tree-row.is-selected") ?? rows?.[0])?.focus({ preventScroll: true });
  }, [library.status]);
  return <section ref={rootRef} className={`library-view${fullScreen ? " is-full-screen" : ""}`} aria-label="Library" tabIndex={-1} onKeyDown={(event) => {
    if (event.key !== "Escape" || event.defaultPrevented || event.nativeEvent.isComposing) return;
    event.preventDefault();
    onClose();
  }}>
    <header className="library-view-header">
      <h2>Library</h2>
      {library.listing ? <code className="library-view-path" title={library.listing.root.path}>{library.listing.root.path}</code> : null}
      <span className="context-toolbar-spacer" />
      <button type="button" className="library-view-close" onClick={onClose} aria-label="Close Library">Close</button>
    </header>
    <ContextViewer client={client} presentation={null} value={view} onChange={setView} controlAllowed onRequestControl={() => undefined} library={library} libraryCommand={command} />
  </section>;
}
