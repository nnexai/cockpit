import { useCallback, useEffect, useLayoutEffect, useRef, useState, type Dispatch, type SetStateAction, type MutableRefObject } from "react";
import type { TerminalStream } from "../../client/CockpitClient";
import type { WorkbenchProps } from "./Workbench";
import type { TabLayoutState } from "../layout/tabLayoutStore";
import type { ContextMenuState, ContextTarget } from "./ContextMenu";
import type { PaneDialog } from "./PaneDialogOverlay";
import { useSubscriptionLimits, type SubscriptionLimitsState } from "../limits/useSubscriptionLimits";
import { stateClass } from "../sidebar/spaceTree";
import { subscribeBrowserLifecycle } from "../layout/browserLifecycle";
import { herdrBindings, herdrPrefixes, setEffectiveHerdrBindings, type HerdrBinding, type HerdrChord } from "../input/herdrBindings";
import type { WorkareaController } from "./useWorkarea";
export function acknowledgeWidgetPrefix(active: boolean, target: EventTarget | null = document.activeElement): void {
  if (!(target instanceof HTMLIFrameElement) || !target.isConnected || document.activeElement !== target
    || !target.matches(".widget-frame:not([data-pending])") || !target.closest("[data-widget-tab]")
    || target.closest('[inert], [aria-hidden="true"]')) return;
  window.dispatchEvent(new CustomEvent("cockpit-widget-prefix", { detail: { target, active } }));
}
export interface WorkbenchUI {
  customBindings: readonly HerdrBinding[]; customPrefixes: readonly HerdrChord[];
  popupPending: string | null; setPopupPending: Dispatch<SetStateAction<string | null>>;
  commandNotice: string | null; setCommandNotice: Dispatch<SetStateAction<string | null>>;
  lifecycleError: string | null; setLifecycleError: Dispatch<SetStateAction<string | null>>;
  paintedTab: TabLayoutState | null; setPaintedTab: Dispatch<SetStateAction<TabLayoutState | null>>;
  switching: boolean; canvasTabs: TabLayoutState[];
  attachedPaneIds: MutableRefObject<Set<string>>; registerStream(stream: TerminalStream, active: boolean): void;
  menu: ContextMenuState | null; setMenu: Dispatch<SetStateAction<ContextMenuState | null>>;
  editing: ContextTarget | null; setEditing: Dispatch<SetStateAction<ContextTarget | null>>;
  dialog: PaneDialog | null; setDialog: Dispatch<SetStateAction<PaneDialog | null>>;
  commandsOpen: boolean; setCommandsOpen: Dispatch<SetStateAction<boolean>>;
  limitsOpen: boolean; setLimitsOpen: Dispatch<SetStateAction<boolean>>; limits: SubscriptionLimitsState;
  commandsOpener: MutableRefObject<HTMLElement | null>;
  sessionChooserOpen: boolean; setSessionChooserOpen: Dispatch<SetStateAction<boolean>>;
  setupOpen: boolean; setSetupOpen: Dispatch<SetStateAction<boolean>>;
  recoveryOpen: boolean; setRecoveryOpen: Dispatch<SetStateAction<boolean>>;
  teardownSpaceId: string | null; setTeardownSpaceId: Dispatch<SetStateAction<string | null>>;
  libraryAddOpen: boolean; setLibraryAddOpen: Dispatch<SetStateAction<boolean>>;
  prefixActive: boolean; setPrefixActive: Dispatch<SetStateAction<boolean>>;
  armedPrefix: { origin: "cockpit" | "herdr"; label: string }; setArmedPrefix: Dispatch<SetStateAction<{ origin: "cockpit" | "herdr"; label: string }>>;
  prefixHint: string | null; setPrefixHint: Dispatch<SetStateAction<string | null>>;
  openSessionChooser(): void;
}
export function useWorkbenchUI({ state, client, tabLayout, onOpenSession, mutations }: WorkbenchProps, { setAttachFocusSuppressed }: WorkareaController): WorkbenchUI {
  const snapshot = state.snapshot;
  const shell = snapshot?.herdr_shell ?? null;
  const popup = shell?.popup ?? null;
  const customBindings = herdrBindings(shell?.commands ?? []);
  const customPrefixes = herdrPrefixes(shell?.prefix_bindings ?? []);
  setEffectiveHerdrBindings(customBindings, customPrefixes);
  useEffect(() => () => setEffectiveHerdrBindings([]), []);
  const [popupPending, setPopupPending] = useState<string | null>(null);
  const [commandNotice, setCommandNotice] = useState<string | null>(null);
  const streamRegistry = useRef(new Set<TerminalStream>());
  const attachedPaneIds = useRef(new Set<string>());
  const registerStream = useCallback((stream: TerminalStream, active: boolean) => { if (active) streamRegistry.current.add(stream); else streamRegistry.current.delete(stream); }, []);
  useEffect(() => () => { streamRegistry.current.forEach(stream => stream.close()); streamRegistry.current.clear(); }, []);
  const [lifecycleError, setLifecycleError] = useState<string | null>(null);
  const [, setBrowserLifecycleRevision] = useState(0);
  useEffect(() => subscribeBrowserLifecycle(() => setBrowserLifecycleRevision(value => value + 1)), []);
  const [paintedTab, setPaintedTab] = useState<TabLayoutState | null>(tabLayout);
  const switching = Boolean(paintedTab && tabLayout && paintedTab.tabId !== tabLayout.tabId);
  useLayoutEffect(() => {
    if (!tabLayout || !switching || state.focusPending?.kind !== "tab") { setPaintedTab(tabLayout); return; }
    const focused = tabLayout.focusedPaneId;
    if (!focused || (tabLayout.zoomLeafId && tabLayout.zoomLeafId !== focused)) { setPaintedTab(tabLayout); return; }
    const timeout = window.setTimeout(() => setPaintedTab(tabLayout), 300);
    return () => window.clearTimeout(timeout);
  }, [tabLayout, switching, state.focusPending?.kind]);
  const canvasTabs = switching && paintedTab ? [paintedTab, tabLayout!] : tabLayout ? [tabLayout] : [];
  const [menu, setMenu] = useState<ContextMenuState | null>(null);
  const [editing, setEditing] = useState<ContextTarget | null>(null);
  const [dialog, setDialog] = useState<PaneDialog | null>(null);
  const [commandsOpen, setCommandsOpen] = useState(false);
  const [limitsOpen, setLimitsOpen] = useState(false);
  const agentsWorking = state.sync === "live" && (snapshot?.agents ?? []).some(agent => stateClass(agent.status) === "working");
  const limits = useSubscriptionLimits(client, agentsWorking);
  const commandsOpener = useRef<HTMLElement | null>(null);
  // Capture before CommandOverlay's passive effect focuses its search field.
  useLayoutEffect(() => {
    if (commandsOpen) commandsOpener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  }, [commandsOpen]);
  const [sessionChooserOpen, setSessionChooserOpen] = useState(false);
  const [setupOpen, setSetupOpen] = useState(false);
  const [recoveryOpen, setRecoveryOpen] = useState(false);
  const [teardownSpaceId, setTeardownSpaceId] = useState<string | null>(null);
  const [libraryAddOpen, setLibraryAddOpen] = useState(false);
  const [prefixActive, setPrefixActive] = useState(false);
  const [armedPrefix, setArmedPrefix] = useState<{ origin: "cockpit" | "herdr"; label: string }>({ origin: "cockpit", label: "Ctrl+B" });
  const [prefixHint, setPrefixHint] = useState<string | null>(null);
  useEffect(() => acknowledgeWidgetPrefix(prefixActive), [prefixActive]);
  useEffect(() => {
    if (prefixHint === null) return;
    const timer = window.setTimeout(() => setPrefixHint(null), 2000);
    return () => window.clearTimeout(timer);
  }, [prefixHint]);
  useEffect(() => {
    if (popup || state.sync !== "live" || shell?.status !== "live") setPopupPending(null);
    if (popup) { setPrefixActive(false); setMenu(null); setAttachFocusSuppressed(false); }
  }, [popup, state.sync, shell?.status]);
  useEffect(() => {
    if (!popupPending || mutations.pending) return;
    if (mutations.errors[`command:${popupPending}`]) { setPopupPending(null); return; }
    const timer = window.setTimeout(() => setPopupPending(null), 1000);
    return () => window.clearTimeout(timer);
  }, [popupPending, mutations.pending, mutations.errors]);
  const openSessionChooser = useCallback(() => {
    setSessionChooserOpen(true);
    onOpenSession();
  }, [onOpenSession]);
  return { customBindings, customPrefixes, popupPending, setPopupPending, commandNotice, setCommandNotice, lifecycleError, setLifecycleError, paintedTab, setPaintedTab, switching, canvasTabs, attachedPaneIds, registerStream, menu, setMenu, editing, setEditing, dialog, setDialog, commandsOpen, setCommandsOpen, limitsOpen, setLimitsOpen, limits, commandsOpener, sessionChooserOpen, setSessionChooserOpen, setupOpen, setSetupOpen, recoveryOpen, setRecoveryOpen, teardownSpaceId, setTeardownSpaceId, libraryAddOpen, setLibraryAddOpen, prefixActive, setPrefixActive, armedPrefix, setArmedPrefix, prefixHint, setPrefixHint, openSessionChooser };
}
