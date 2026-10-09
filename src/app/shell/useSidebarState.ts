import { useCallback, useEffect, useRef, useState, type MutableRefObject } from "react";
import type { WorkbenchProps } from "./Workbench";
export const SIDEBAR_MIN_WIDTH = 224;
export const SIDEBAR_MAX_WIDTH = 360;
export const SIDEBAR_DEFAULT_WIDTH = 240;
const SIDEBAR_WIDTH_KEY = "cockpit.sidebar.width";
const SIDEBAR_COLLAPSED_KEY = "cockpit.sidebar.collapsed";

function isNarrowViewport(): boolean {
  return typeof window !== "undefined" && window.innerWidth <= 800;
}

function readSidebarWidth(): number {
  if (typeof window === "undefined") return SIDEBAR_DEFAULT_WIDTH;
  try {
    const stored = window.localStorage.getItem(SIDEBAR_WIDTH_KEY);
    const value = stored === null ? NaN : Number(stored);
    return Number.isFinite(value) ? Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, value)) : SIDEBAR_DEFAULT_WIDTH;
  } catch {
    return SIDEBAR_DEFAULT_WIDTH;
  }
}

function readSidebarCollapsed(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "true";
  } catch {
    return false;
  }
}
export interface SidebarState {
  sidebarWidth: number; sidebarCollapsed: boolean; narrowViewport: boolean; drawerOpen: boolean;
  sidebarCloseRef: MutableRefObject<HTMLButtonElement | null>;
  drawerFocusTarget: MutableRefObject<{ spaceId: string; paneId: string | null } | null>;
  closeDrawer(restoreFocus?: boolean): void; openDrawer(): void; updateSidebarWidth(next: number): void;
  toggleSidebarCollapsed(): void; toggleSidebar(): void;
}
export function useSidebarState({ selection, state }: Pick<WorkbenchProps, "selection" | "state">): SidebarState {
  const [sidebarWidth, setSidebarWidth] = useState(readSidebarWidth);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readSidebarCollapsed);
  const [narrowViewport, setNarrowViewport] = useState(isNarrowViewport);
  const [drawerOpen, setDrawerOpen] = useState(() => !isNarrowViewport());
  const sidebarReturnFocus = useRef<HTMLElement | null>(null);
  const sidebarCloseRef = useRef<HTMLButtonElement | null>(null);
  const drawerFocusTarget = useRef<{ spaceId: string; paneId: string | null } | null>(null);
  const closeDrawer = useCallback((restoreFocus = true) => {
    setDrawerOpen(false);
    drawerFocusTarget.current = null;
    if (restoreFocus) window.setTimeout(() => {
      const target = sidebarReturnFocus.current ?? document.querySelector<HTMLElement>(".drawer-toggle");
      target?.focus({ preventScroll: true });
      sidebarReturnFocus.current = null;
    }, 0);
  }, []);
  const openDrawer = useCallback(() => {
    if (!narrowViewport) return;
    sidebarReturnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setDrawerOpen(true);
  }, [narrowViewport]);
  const updateSidebarWidth = useCallback((next: number) => {
    const width = Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, next));
    setSidebarWidth(width);
    try { window.localStorage.setItem(SIDEBAR_WIDTH_KEY, String(width)); } catch { /* local preferences are optional */ }
  }, []);
  const toggleSidebarCollapsed = useCallback(() => {
    setSidebarCollapsed((collapsed) => {
      const next = !collapsed;
      try { window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(next)); } catch { /* local preferences are optional */ }
      return next;
    });
  }, []);
  useEffect(() => {
    let previous = isNarrowViewport();
    const update = () => {
      const narrow = isNarrowViewport();
      setNarrowViewport(narrow);
      if (narrow !== previous) setDrawerOpen(!narrow);
      previous = narrow;
    };
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);
  useEffect(() => {
    if (!narrowViewport || !drawerOpen) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (document.querySelector("[data-server-modal]")) return;
      if (event.key === "Escape") {
        event.preventDefault();
        closeDrawer();
        return;
      }
      if (event.key !== "Tab") return;
      const controls = [...document.querySelectorAll<HTMLElement>(".sidebar:not([hidden]) button:not(:disabled), .sidebar:not([hidden]) input:not(:disabled), .sidebar:not([hidden]) select:not(:disabled), .sidebar:not([hidden]) [tabindex]:not([tabindex='-1'])")];
      if (controls.length === 0) return;
      event.preventDefault();
      const current = controls.indexOf(document.activeElement as HTMLElement);
      controls[(current + (event.shiftKey ? controls.length - 1 : 1)) % controls.length]?.focus();
    };
    window.addEventListener("keydown", handleKeyDown, true);
    window.setTimeout(() => sidebarCloseRef.current?.focus({ preventScroll: true }), 0);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, [closeDrawer, drawerOpen, narrowViewport]);
  useEffect(() => {
    const target = drawerFocusTarget.current;
    if (!target || !narrowViewport || !drawerOpen || state.focusPending || state.focusError) return;
    if (selection.spaceId !== target.spaceId) return;
    if (target.paneId && selection.paneId !== target.paneId) return;
    closeDrawer();
  }, [closeDrawer, drawerOpen, narrowViewport, selection.paneId, selection.spaceId, state.focusError, state.focusPending]);
  const toggleSidebar = () => {
        const visible = narrowViewport ? drawerOpen : !sidebarCollapsed;
        if (narrowViewport) { if (drawerOpen) closeDrawer(); else openDrawer(); } else toggleSidebarCollapsed();
        // A collapsed sidebar cannot keep focus; the selected tab is a safe target that sends nothing to Herdr.
        if (visible && !narrowViewport && document.activeElement?.closest("#cockpit-sidebar")) {
          window.setTimeout(() => document.querySelector<HTMLElement>('.tab-button[aria-selected="true"], .drawer-toggle')?.focus({ preventScroll: true }), 0);
        }
  };
  return { sidebarWidth, sidebarCollapsed, narrowViewport, drawerOpen, sidebarCloseRef, drawerFocusTarget, closeDrawer, openDrawer, updateSidebarWidth, toggleSidebarCollapsed, toggleSidebar };
}
