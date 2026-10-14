import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { flushSync } from "react-dom";
import type { LibraryCommand } from "../context/ContextViewer";

export type Workarea = { kind: "terminal" } | { kind: "library"; command: LibraryCommand | null } | { kind: "notes" } | { kind: "overview" };
export type WorkareaState = { view: Workarea };
export interface WorkareaController extends WorkareaState {
  attachFocusSuppressed: boolean;
  setAttachFocusSuppressed(value: boolean): void;
  openLibrary(command?: { kind: "refresh" } | { kind: "tokens" } | { kind: "open"; itemId: string }): void;
  closeLibrary(): void; openOverview(): void; closeOverview(): void;
  openNotes(): void; closeNotes(): void; leave(mode: "focus" | "keep"): void;
  exitForPaneCommand(execute: () => void): void; captureLibraryInvoker(invoker: HTMLElement | null): void;
}
export type WorkareaAction = { type: "show"; view: Workarea } | { type: "session/reset" };
export const initialWorkareaState: WorkareaState = { view: { kind: "terminal" } };
export function workareaReducer(_state: WorkareaState, action: WorkareaAction): WorkareaState {
  return action.type === "show" ? { view: action.view } : initialWorkareaState;
}
export function useWorkarea(sessionId: string | null, selectedPaneId: string | null): WorkareaController {
  const [workarea, dispatch] = useReducer(workareaReducer, initialWorkareaState);
  useEffect(() => { dispatch({ type: "session/reset" }); }, [sessionId]);
  const libraryCommandToken = useRef(0);
  // Preserve an explicit sidebar invoker; other Library closes let the selected terminal attach with focus.
  const [attachFocusSuppressed, setAttachFocusSuppressed] = useState(false);
  const libraryOrigin = useRef<{ paneId: string; graphical: boolean } | null>(null);
  const librarySidebarInvoker = useRef<HTMLElement | null>(null);
  const selectedPaneIdRef = useRef(selectedPaneId);
  selectedPaneIdRef.current = selectedPaneId;
  const openLibrary = useCallback((command?: { kind: "refresh" } | { kind: "tokens" } | { kind: "open"; itemId: string }) => {
    const active = document.activeElement;
    librarySidebarInvoker.current = null;
    libraryOrigin.current = active instanceof HTMLElement && active.closest(".pane-view") && selectedPaneIdRef.current
      ? { paneId: selectedPaneIdRef.current, graphical: active.closest<HTMLElement>("[data-kind]")?.dataset.kind !== "terminal" }
      : null;
    // A command belongs to this opening only; reopening must not replay it.
    dispatch({ type: "show", view: { kind: "library", command: command ? { ...command, token: ++libraryCommandToken.current } : null } });
  }, []);
  const closeLibrary = useCallback(() => {
    const origin = libraryOrigin.current;
    libraryOrigin.current = null;
    const sidebarInvoker = librarySidebarInvoker.current;
    librarySidebarInvoker.current = null;
    const returnToSidebar = Boolean(sidebarInvoker?.isConnected && !sidebarInvoker.closest("[inert]"));
    const returnToPane = origin !== null && origin.paneId === selectedPaneIdRef.current;
    dispatch({ type: "show", view: { kind: "terminal" } });
    setAttachFocusSuppressed(returnToSidebar);
    if (returnToPane && origin.graphical) {
      const focusDocument = (attempts: number) => {
        const document_ = document.querySelector<HTMLElement>(".pane-view.is-selected .context-document, .pane-view.is-selected .review-diff, .pane-view.is-selected .browser-surface");
        if (document_) document_.focus({ preventScroll: true });
        else if (attempts > 0) requestAnimationFrame(() => focusDocument(attempts - 1));
      };
      requestAnimationFrame(() => focusDocument(60));
    }
  }, []);
  const openOverview = useCallback(() => {
    setAttachFocusSuppressed(true);
    dispatch({ type: "show", view: { kind: "overview" } });
  }, []);
  const closeOverview = useCallback(() => { dispatch({ type: "show", view: { kind: "terminal" } }); setAttachFocusSuppressed(false); }, []);
  const closeNotes = useCallback(() => { dispatch({ type: "show", view: { kind: "terminal" } }); setAttachFocusSuppressed(true); }, []);
  const openNotes = useCallback(() => { setAttachFocusSuppressed(true); dispatch({ type: "show", view: { kind: "notes" } }); }, []);
  const leave = (mode: "focus" | "keep") => { dispatch({ type: "show", view: { kind: "terminal" } }); if (mode === "focus") setAttachFocusSuppressed(false); };
  const exitForPaneCommand = (execute: () => void) => {
    libraryOrigin.current = null;
    setAttachFocusSuppressed(false);
    flushSync(() => dispatch({ type: "show", view: { kind: "terminal" } }));
    requestAnimationFrame(() => window.setTimeout(execute, 0));
  };
  const captureLibraryInvoker = (invoker: HTMLElement | null) => { librarySidebarInvoker.current = invoker?.closest(".sidebar") ? invoker : null; };
  return { ...workarea, attachFocusSuppressed, setAttachFocusSuppressed, openLibrary, closeLibrary, openOverview, closeOverview, openNotes, closeNotes, leave, exitForPaneCommand, captureLibraryInvoker };
}
