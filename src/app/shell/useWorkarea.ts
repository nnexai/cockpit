import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { flushSync } from "react-dom";
import type { LibraryCommand } from "../context/ContextViewer";

export type Workarea = { kind: "terminal" } | { kind: "library"; command: LibraryCommand | null } | { kind: "notes" } | { kind: "supervisor" };
export type SupervisorSurface = { modal: boolean; startSessionId: string | null; startToken: number };
export type WorkareaState = { view: Workarea; supervisor: SupervisorSurface | null };
export interface WorkareaController extends WorkareaState {
  attachFocusSuppressed: boolean;
  setAttachFocusSuppressed(value: boolean): void;
  openLibrary(command?: { kind: "refresh" } | { kind: "tokens" } | { kind: "open"; itemId: string }): void;
  closeLibrary(): void; openSupervisor(start?: boolean): void; closeSupervisor(): void;
  openNotes(): void; closeNotes(): void; leave(mode: "focus" | "keep"): void;
  exitForPaneCommand(execute: () => void): void; captureLibraryInvoker(invoker: HTMLElement | null): void;
  setSupervisorModal(modal: boolean): void;
}
export type WorkareaAction =
  | { type: "show"; view: Exclude<Workarea, { kind: "supervisor" }> }
  | { type: "supervisor/open"; start: boolean; sessionId: string | null }
  | { type: "supervisor/modal"; modal: boolean }
  | { type: "session/reset" };
export const initialWorkareaState: WorkareaState = { view: { kind: "terminal" }, supervisor: null };
export function workareaReducer(state: WorkareaState, action: WorkareaAction): WorkareaState {
  switch (action.type) {
    case "show": return { ...state, view: action.view };
    case "supervisor/open": {
      const surface = state.supervisor ?? { modal: false, startSessionId: null, startToken: 0 };
      return { view: { kind: "supervisor" }, supervisor: action.start ? { ...surface, startSessionId: action.sessionId, startToken: surface.startToken + 1 } : surface };
    }
    case "supervisor/modal": return state.supervisor ? { ...state, supervisor: { ...state.supervisor, modal: action.modal } } : state;
    case "session/reset": return state.supervisor ? { ...state, supervisor: { ...state.supervisor, startSessionId: null, startToken: 0 } } : state;
  }
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
  const openSupervisor = useCallback((start = false) => {
    setAttachFocusSuppressed(true);
    dispatch({ type: "supervisor/open", start, sessionId });
  }, [sessionId]);
  const closeSupervisor = useCallback(() => { dispatch({ type: "show", view: { kind: "terminal" } }); setAttachFocusSuppressed(false); }, []);
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
  const setSupervisorModal = useCallback((modal: boolean) => dispatch({ type: "supervisor/modal", modal }), []);
  return { ...workarea, attachFocusSuppressed, setAttachFocusSuppressed, openLibrary, closeLibrary, openSupervisor, closeSupervisor, openNotes, closeNotes, leave, exitForPaneCommand, captureLibraryInvoker, setSupervisorModal };
}
