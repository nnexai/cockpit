import { UiIcon, type UiIconName } from "./UiIcon";
import { ErrorSlot } from "./ErrorSlot";
import { useCallback, useEffect, useLayoutEffect, useReducer, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent, type ReactNode } from "react";
import {
  parseResourceMutationResponse,
  type CockpitClient,
  type CockpitStatus,
  type TerminalStream,
} from "../client/CockpitClient";
import type {
  FocusRequest,
  HerdrCommand,
  ResourceMutationRequest,
  SessionSnapshotResponse,
  SessionStreamMessage,
  SessionSummary,
  ViewerSourceOptions,
} from "../protocol/generated/v1";
import { initialSessionState, sessionReducer, type SessionAction, type SessionState } from "./session/sessionStore";
import { useFocusCoordinator } from "./session/focusCoordinator";
import { spaceCheckoutKey, useSpaceGitStatus } from "./session/spaceGitStatus";
import { type MutationCoordinatorState, type MutationOperation, useMutationCoordinator } from "./session/mutationCoordinator";
import { tabDropInsertionIndex } from "./layout/layoutProjection";
import { useTabLayouts, type LayoutAction, type LeafCtx, type PendingCreation, type TabLayoutState } from "./layout/tabLayoutStore";
import { leaves, type Leaf } from "./layout/splitTree";
import { solveLayout, type Rect } from "./layout/solveLayout";
import { runtimeSource, type FocusEcho } from "./layout/reconcile";
import { TabCanvas, focusSelectedDivider } from "./layout/TabCanvas";
import { LeafHost } from "./layout/LeafHost";
import { openViewerLeaf, closeViewerLeaf, releaseViewers, getViewerClientId } from "./layout/viewerLifecycle";
import { openBrowserLeaf, closeBrowserLeaf, retireTabBrowser, browserOpenDisabledReason, retryBrowserCleanup, subscribeBrowserLifecycle } from "./layout/browserLifecycle";
import { BrowserCleanupNotices } from "./layout/BrowserCleanupNotices";
import { routeWorkbenchKeydown } from "./input/keymap";
import { herdrBindings, herdrPrefixes, herdrCommandShortcut, setEffectiveHerdrBindings } from "./input/herdrBindings";
import { ServerPopup } from "./ServerPopup";
import { SHORTCUTS, SHORTCUT_SEPARATOR, armedPrefixHint, focusSidebarList, formatShortcut, shortcutEntry, withShortcut, type PrefixCommand } from "./input/shortcuts";
import { trapModalTab, useModalFocus } from "./input/modal";
import { flushSync } from "react-dom";
import { dispatchFileNavigation, rankFuzzyMatches } from "./input/fileNavigation";
import { SetupDialog, setupParentFor } from "./projects/SetupDialog";
import { TeardownDialog } from "./projects/TeardownDialog";
import { TeardownRecoveryPanel } from "./projects/TeardownRecoveryPanel";
import type { LibraryCommand } from "./context/ContextViewer";
import { AddContextDialog } from "./library/AddContextDialog";
import { LibraryView } from "./library/LibraryView";
import { LibraryProblems } from "./library/LibraryProblems";
import type { LibrarySpace } from "./library/libraryState";
import { InlineRename } from "./InlineRename";
import { Sidebar } from "./sidebar/Sidebar";
import { spaceNotesFromFailures, type ContextAnchor } from "./sidebar/Spaces";
import type { Agent, Space } from "./sidebar/spaceTree";

type StatusError = { message: string; code?: string };
type SessionSnapshot = SessionSnapshotResponse;
type Tab = SessionSnapshot["tabs"][number];
type Pane = SessionSnapshot["panes"][number];
type Selection = { spaceId: string | null; tabId: string | null; paneId: string | null };

function describeError(error: unknown, fallback: string): StatusError {
  if (error instanceof Error) {
    const typed = error as Error & { code?: unknown; operationCode?: unknown };
    return { message: typed.message || fallback, code: typeof typed.operationCode === "string" ? typed.operationCode : typeof typed.code === "string" ? typed.code : undefined };
  }
  return { message: fallback };
}
function byId<T extends { id: string }>(items: T[], id: string | null): T | undefined { return id ? items.find((item) => item.id === id) : undefined; }
function tabsForSpace(tabs: Tab[], spaceId: string | null): Tab[] { return spaceId ? tabs.filter((tab) => tab.space_id === spaceId) : []; }
function panesForTab(panes: Pane[], tabId: string | null): Pane[] { return tabId ? panes.filter((pane) => pane.tab_id === tabId) : []; }
export function authoritativeMutationSnapshot(expectedSessionId: string, response: unknown): SessionSnapshot {
  const parsed = parseResourceMutationResponse(response);
  if (parsed.session_id !== expectedSessionId) {
    throw new Error("Mutation response belongs to another session");
  }
  return parsed.snapshot;
}
function CompatibilityNotice({ status, error, retry, onOpenLibrary }: { status: CockpitStatus | null; error: StatusError | null; retry: () => void; onOpenLibrary: () => void }) {
  const herdr = status?.herdr;
  const title = error ? "Cockpit unavailable" : herdr?.status === "incompatible" ? "Herdr is incompatible" : "Herdr is unavailable";
  const message = error?.message ?? (herdr && herdr.status !== "compatible" ? herdr.message : undefined);
  const code = error?.code ?? (herdr && herdr.status !== "compatible" ? herdr.code : undefined);
  return <main className="compatibility-main" aria-live="polite"><section className="notice notice-error" role="alert"><p className="eyebrow">Cockpit</p><h1>{title}</h1><p>{message ?? "Could not read the Herdr compatibility status."}</p>{code ? <code>{code}</code> : null}<div className="notice-actions"><button type="button" className="action-button" onClick={retry}>Retry status</button><OpenLibraryButton onOpen={onOpenLibrary} /></div></section></main>;
}

/** The Library needs no Herdr session, so every no-session screen can open it. */
function OpenLibraryButton({ onOpen }: { onOpen: () => void }) {
  return <button type="button" className="action-button" data-library-opener onClick={onOpen}>Open Library</button>;
}

type Mutate = (key: string, request: ResourceMutationRequest, focusFromSnapshot?: boolean) => boolean;
type ContextTarget =
  | { kind: "space"; id: string }
  | { kind: "tab"; id: string }
  | { kind: "pane"; id: string };
type ContextMenuState = { target: ContextTarget; x: number; y: number };
type DragIntent = { kind: "space" | "tab"; sourceId: string; order: string[] };
type DropMark = { targetId: string; side: "before" | "after" } | null;
type PaneDialog =
  | { kind: "rename"; paneId: string }
  | { kind: "swap"; paneId: string }
  | { kind: "move"; paneId: string };

export function canSwitchSessions(sessionCount: number): boolean {
  return sessionCount > 1;
}

export function tabLabelIsRedundant(label: string, displayedNumber: number): boolean {
  const trimmed = label.trim();
  return trimmed === String(displayedNumber) || /^\d+$/.test(trimmed);
}

export type PaneFocusDirection = "left" | "right" | "up" | "down";


export function contextMenuPosition(
  x: number,
  y: number,
  viewportWidth: number,
  viewportHeight: number,
  menuWidth = 286,
  menuHeight = 320,
  gutter = 8,
): { x: number; y: number } {
  return {
    x: Math.max(gutter, Math.min(x, viewportWidth - menuWidth - gutter)),
    y: Math.max(gutter, Math.min(y, viewportHeight - menuHeight - gutter)),
  };
}

function ContextMenu({ menu, children, onDismiss }: { menu: ContextMenuState; children: ReactNode; onDismiss: () => void }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const dismissAndRestore = useCallback(() => {
    onDismiss();
    window.setTimeout(() => opener.current?.focus({ preventScroll: true }), 0);
  }, [onDismiss]);
  useEffect(() => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dismiss = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) dismissAndRestore(); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") dismissAndRestore(); };
    window.addEventListener("pointerdown", dismiss);
    window.addEventListener("keydown", escape);
    ref.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    return () => { window.removeEventListener("pointerdown", dismiss); window.removeEventListener("keydown", escape); };
  }, [dismissAndRestore, onDismiss, menu]);
  const [menuSize, setMenuSize] = useState({ width: 286, height: 320 });
  useLayoutEffect(() => {
    const root = ref.current;
    if (!root) return;
    const measure = () => {
      const { width, height } = root.getBoundingClientRect();
      setMenuSize((current) => current.width === width && current.height === height ? current : { width, height });
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(root);
    return () => observer?.disconnect();
  }, [menu]);
  const position = contextMenuPosition(menu.x, menu.y, window.innerWidth, window.innerHeight, menuSize.width, menuSize.height);
  return <div ref={ref} className="context-menu" role="menu" aria-label={`${menu.target.kind} actions`} style={{ left: position.x, top: position.y }} onContextMenu={(event) => event.preventDefault()} onKeyDown={(event) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const items = [...(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
    if (items.length === 0) return;
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : event.key === "ArrowDown" ? (current + 1) % items.length : (current - 1 + items.length) % items.length;
    items[next].focus();
  }}>{children}</div>;
}

function TabStrip({ tabs, selectedTabId, editingId, busy, browserOpen, browserDisabledReason, libraryOpen, onEdit, onSelect, onContext, onCreate, onBrowserToggle, onLibraryToggle, onCommands, sidebarOpen, onToggleSidebar, mutate }: {
  tabs: Tab[];
  selectedTabId: string | null;
  editingId: string | null;
  busy: boolean;
  browserOpen: boolean;
  browserDisabledReason: string | null;
  libraryOpen: boolean;
  onEdit: (id: string | null) => void;
  onSelect: (tab: Tab) => void;
  onContext: (event: MouseEvent, target: ContextTarget) => void;
  onCreate: () => void;
  onBrowserToggle: () => void;
  onLibraryToggle: () => void;
  onCommands: () => void;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  mutate: Mutate;
}) {
  const [dragIntent, setDragIntent] = useState<DragIntent | null>(null);
  const [dropMark, setDropMark] = useState<DropMark>(null);
  const [dragMessage, setDragMessage] = useState<string | null>(null);
  return <nav className="tab-toolbar" aria-label="Tabs"><button type="button" className="tab-icon-button" aria-label={sidebarOpen ? "Hide sidebar" : "Show sidebar"} title={withShortcut(sidebarOpen ? "Hide sidebar" : "Show sidebar", "toggle-sidebar")} aria-expanded={sidebarOpen} aria-controls="cockpit-sidebar" onClick={onToggleSidebar}><UiIcon name={sidebarOpen ? "sidebar-open" : "sidebar"} /></button><div className="tab-strip" role="tablist">{tabs.map((tab, index) => {
    const displayedNumber = index + 1;
    const redundantLabel = tabLabelIsRedundant(tab.label, displayedNumber);
    const accessibleLabel = redundantLabel ? `Tab ${displayedNumber}` : `Tab ${displayedNumber}: ${tab.label}`;
    const side = dropMark?.targetId === tab.id ? dropMark.side : null;
    return <div className={`tab-item${tab.id === selectedTabId ? " is-selected" : ""}${side ? ` drop-${side}` : ""}`} key={tab.id} draggable={!busy && editingId !== tab.id}
      onDragStart={(event) => { if (!busy) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("application/x-cockpit-tab", tab.id); event.dataTransfer.setData("text/plain", `tab:${tab.id}`); setDragIntent({ kind: "tab", sourceId: tab.id, order: tabs.map((candidate) => candidate.id) }); setDragMessage(null); } }}
      onDragEnd={() => { setDragIntent(null); setDropMark(null); }}
      onDragEnter={(event) => { if (!busy) event.preventDefault(); }}
      onDragOver={(event) => { if (!busy) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; if (dragIntent && dragIntent.sourceId !== tab.id) setDropMark({ targetId: tab.id, side: event.clientX >= event.currentTarget.getBoundingClientRect().left + event.currentTarget.getBoundingClientRect().width / 2 ? "after" : "before" }); } }}
      onDrop={(event) => {
        if (busy) return;
        event.preventDefault();
        const fallback = event.dataTransfer.getData("text/plain");
        const id = event.dataTransfer.getData("application/x-cockpit-tab") || (fallback.startsWith("tab:") ? fallback.slice(4) : "");
        if (!id || id === tab.id) return;
        const afterTarget = event.clientX >= event.currentTarget.getBoundingClientRect().left + event.currentTarget.getBoundingClientRect().width / 2;
        const intent = dragIntent?.sourceId === id ? dragIntent : { kind: "tab" as const, sourceId: id, order: tabs.map((candidate) => candidate.id) };
        const unchanged = intent.order.length === tabs.length && intent.order.every((candidate, position) => candidate === tabs[position]?.id);
        const insertion = unchanged ? tabDropInsertionIndex(tabs.findIndex((candidate) => candidate.id === id), index, afterTarget) : null;
        setDropMark(null);
        if (!unchanged) setDragMessage("Tab order changed while dragging. Start again.");
        else if (insertion !== null) mutate(`tab:${id}`, { type: "tab_move", tab_id: id, insert_index: insertion });
      }}
      onContextMenu={(event) => onContext(event, { kind: "tab", id: tab.id })}>
      {editingId === tab.id
        ? <InlineRename label={tab.label} ariaLabel={`Rename tab ${tab.label}`} onCancel={() => onEdit(null)} onCommit={(label) => { const accepted = mutate(`tab:${tab.id}`, { type: "tab_rename", tab_id: tab.id, label }); if (accepted) onEdit(null); return accepted; }} />
        : <button type="button" disabled={busy} draggable={!busy} role="tab" aria-selected={tab.id === selectedTabId} aria-label={accessibleLabel} className="tab-button" title={displayedNumber <= 9 ? withShortcut(redundantLabel ? `Tab ${displayedNumber}` : tab.label, `select-tab-${displayedNumber as 1}`) : redundantLabel ? `Tab ${displayedNumber}` : tab.label} onDragStart={(event) => { if (!busy) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("application/x-cockpit-tab", tab.id); event.dataTransfer.setData("text/plain", `tab:${tab.id}`); setDragIntent({ kind: "tab", sourceId: tab.id, order: tabs.map((candidate) => candidate.id) }); setDragMessage(null); } }} onClick={() => onSelect(tab)} onDoubleClick={() => onEdit(tab.id)}><span className="n">{displayedNumber}</span>{redundantLabel ? null : <span className="tab-label">{tab.label}</span>}</button>}
    </div>;
  })}
    <button type="button" disabled={busy} className="tab-add" aria-label="Create tab" title={withShortcut("New tab", "new-tab")} onClick={onCreate}><UiIcon name="plus" /></button></div>{dragMessage ? <span className="resource-inline-status tab-drag-status" role="status">{dragMessage}</span> : null}<div className="tab-strip-actions"><span className="tab-strip-separator" aria-hidden="true" /><button type="button" className="tab-icon-button" disabled={busy || Boolean(browserDisabledReason)} aria-label="Browser" aria-pressed={browserOpen} title={browserDisabledReason ?? withShortcut(browserOpen ? "Close Browser (stops it and deletes its profile: cookies, logins, site data)" : "Open browser for tab", "toggle-browser")} onClick={onBrowserToggle}><UiIcon name="browser" /></button><button type="button" className="tab-icon-button" aria-label="Library" aria-pressed={libraryOpen} title={withShortcut(libraryOpen ? "Close Library" : "Open Library", "toggle-library")} onClick={onLibraryToggle}><UiIcon name="library" /></button><LibraryProblems /><button type="button" className="tab-strip-action" title={withShortcut("Commands", "help")} onClick={onCommands}>Commands</button></div>
  </nav>;
}


type CommandAction = { id: string; label: string; icon?: UiIconName; shortcut?: string; group: "Navigate" | "Space" | "Tab" | "Pane" | "Browser" | "Library" | "Herdr"; disabled?: boolean; reason?: string; reasonDetail?: string; run: () => void };

const commandGroupIcons: Record<CommandAction["group"], UiIconName> = { Herdr: "terminal", Navigate: "forward", Space: "grid", Tab: "browser", Pane: "terminal", Browser: "browser", Library: "library" };

function commandIcon(action: CommandAction): UiIconName {
  if (action.icon) return action.icon;
  if (action.id.endsWith("-left") || action.id.endsWith(":previous-tab")) return "back";
  if (action.id.endsWith("-right") || action.id.endsWith(":next-tab")) return "forward";
  if (action.id.endsWith("-up")) return "up";
  if (action.id.endsWith("-down")) return "down";
  if (action.id.includes("rename")) return "edit";
  if (action.id.includes("close")) return "close";
  if (action.id.includes("new-") || action.id.endsWith(":add")) return "plus";
  if (action.id.includes("cleanup") || action.id.includes("refresh")) return "refresh";
  if (action.id.endsWith(":zoom-pane")) return "expand";
  if (action.id.endsWith(":open-file-picker")) return "search";
  if (action.id.endsWith(":toggle-sidebar")) return "sidebar";
  return commandGroupIcons[action.group];
}

/** Picks the clause of a renderer diagnostic chain that explains one action, e.g. "Requires a safe source pane directory". */
export function rendererReasonFor(kind: RendererActionDefinition["kind"], reason: string): string | undefined {
  const prefix = kind === "review" ? "Open Review " : kind === "files" ? "Open files " : "Open Context ";
  const clause = reason.split(";").map((part) => part.trim()).find((part) => part.startsWith(prefix));
  if (!clause) return undefined;
  const rest = clause.slice(prefix.length);
  return rest.charAt(0).toLocaleUpperCase() + rest.slice(1);
}
type RendererActionDefinition = { id: string; label: string; icon: UiIconName; kind: "review" | "files" | "context" };

const rendererActionDefinitions: RendererActionDefinition[] = [
  { id: "review", label: "Open Review", icon: "file", kind: "review" },
  { id: "files", label: "Open Files", icon: "folder", kind: "files" },
  { id: "context", label: "Open Context", icon: "library", kind: "context" },
];

/** Prefix commands that only move focus or open a surface, so they stay available while a Herdr mutation is pending. */
const COMMANDS_ALLOWED_WHILE_BUSY: readonly PrefixCommand[] = ["previous-tab", "next-tab", "previous-pane", "next-pane", "focus-left", "focus-right", "focus-up", "focus-down", "resize", "toggle-sidebar", "focus-spaces", "focus-agents", "switch-session", "toggle-library", "open-file-picker"];

/** A shortcut as `kbd` chips: `Ctrl+B` `i` for a sequence, one chip for a chord, alternatives separated by "or". */
function ShortcutKeys({ text }: { text: string }) {
  return <span className="command-keys">{text.split(SHORTCUT_SEPARATOR).map((form, index) => <span className="command-key-form" key={form}>
    {index > 0 ? <span className="command-key-or">or</span> : null}
    {(form.startsWith("Ctrl+B ") ? ["Ctrl+B", form.slice("Ctrl+B ".length)] : [form]).map((chip) => <kbd key={chip}>{chip}</kbd>)}
  </span>)}</span>;
}

function CommandOverlay({ actions, statusContent, onSwitchSession, onDismiss }: { actions: CommandAction[]; statusContent?: ReactNode; onSwitchSession: () => void; onDismiss: () => void }) {
  const ref = useModalFocus<HTMLElement>(onDismiss);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const activeRowRef = useRef<HTMLButtonElement | null>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [showAll, setShowAll] = useState(false);
  const normalized = query.trim().toLocaleLowerCase();
  const primaryIds = ["prefix:zoom-pane", "prefix:toggle-library", "browser:open", "prefix:new-tab", "prefix:switch-session", "renderer:review", "prefix:setup-space"];
  const ranked = normalized
    ? rankFuzzyMatches(query, actions, (action) => `${action.label} ${action.shortcut ?? ""} ${action.group}`)
    : actions.map((action, index) => ({ ...action, score: index, matchedIndices: [] as number[] }));
  const groups = ["Herdr", "Navigate", "Space", "Tab", "Pane", "Browser", "Library"] as const;
  // Rows render grouped, so keep `filtered` in that order for the highlight and arrow keys. With a query, groups follow their best match.
  const groupOrder: readonly CommandAction["group"][] = normalized ? [...new Set(ranked.map((action) => action.group))] : groups;
  const filtered = groupOrder.flatMap((group) => ranked.filter((action) => action.group === group && (normalized || showAll || group === "Herdr" || primaryIds.includes(action.id))));
  useEffect(() => setActive((current) => Math.min(current, Math.max(0, filtered.length - 1))), [filtered.length]);
  useEffect(() => { searchRef.current?.focus(); }, []);
  useEffect(() => { activeRowRef.current?.scrollIntoView?.({ block: "nearest" }); }, [active, normalized]);
  const runActive = () => {
    const action = filtered[active];
    if (action && !action.disabled) action.run();
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><section ref={ref} className="command-overlay" role="dialog" aria-modal="true" aria-labelledby="commands-title" onKeyDown={(event) => {
    const listStep = event.key === "ArrowDown" || (event.ctrlKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "n") ? 1 : event.key === "ArrowUp" || (event.ctrlKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "p") ? -1 : 0;
    if (listStep !== 0) { event.preventDefault(); setActive((current) => filtered.length === 0 ? 0 : (current + (listStep === 1 ? 1 : filtered.length - 1)) % filtered.length); return; }
    if (event.key === "Enter" && document.activeElement instanceof HTMLInputElement) { event.preventDefault(); runActive(); return; }
    trapModalTab(event, ref.current);
  }}><header><h2 id="commands-title">Commands</h2><button type="button" onClick={onDismiss} aria-label="Close commands"><UiIcon name="close" /></button></header><div className="command-search-box"><UiIcon name="search" /><input ref={searchRef} className="command-search" aria-label="Find a command" placeholder="Find a command…" autoComplete="off" value={query} onChange={(event) => { setQuery(event.target.value); setActive(0); }} /></div>{statusContent ? <div className="command-status">{statusContent}</div> : null}<div className="command-list" role="listbox" aria-label="Available commands">{filtered.length === 0 ? <p className="command-empty">No matching commands.</p> : groupOrder.map((group) => {
    const groupActions = filtered.filter((action) => action.group === group);
    if (groupActions.length === 0) return null;
    return <section className="command-group" key={group}><h3>{group}</h3>{groupActions.map((action) => {
      const index = filtered.indexOf(action);
      return <button ref={index === active ? activeRowRef : null} type="button" role="option" aria-selected={index === active} className={`command-row${index === active ? " is-active" : ""}`} key={action.id} disabled={action.disabled} onMouseMove={() => { if (index !== active) setActive(index); }} onClick={() => action.run()}><UiIcon name={commandIcon(action)} /><span className="command-row-label"><span>{Array.from(action.label, (character, characterIndex) => action.matchedIndices.includes(characterIndex) ? <mark key={characterIndex}>{character}</mark> : character)}</span>{action.disabled && action.reason ? <small title={action.reasonDetail ?? action.reason}>{action.reason}</small> : null}</span>{action.shortcut ? <ShortcutKeys text={action.shortcut} /> : null}</button>;
    })}</section>;
  })}</div><footer className="command-footer"><span>↑↓ or Ctrl+N/P navigate · Enter choose · Esc close · type a name or a key</span><button type="button" onClick={() => { setShowAll((value) => !value); setActive(0); }}>{showAll ? "Quick commands" : "All commands"}</button></footer></section></div>;
}

export function moveDestinationLabel(tab: Tab, spaces: Space[]): string {
  const space = spaces.find((candidate) => candidate.id === tab.space_id);
  return `${space?.label ?? `Space ${space?.number ?? "?"}`} / ${tab.label || `Tab ${tab.number}`}`;
}

function PaneDialogOverlay({ dialog, panes, tabs, spaces, busy, onDismiss, mutate, leafChoices, onSwap, confirmMove }: { dialog: PaneDialog; panes: Pane[]; tabs: Tab[]; spaces: Space[]; busy: boolean; onDismiss: () => void; mutate: Mutate; leafChoices: { id: string; title: string }[]; onSwap(a: string, b: string): void; confirmMove(pane: Pane): boolean }) {
  const pane = panes.find((candidate) => candidate.id === dialog.paneId);
  const [value, setValue] = useState(dialog.kind === "rename" ? pane?.title ?? "" : "");
  const ref = useModalFocus<HTMLFormElement>(onDismiss);
  if (!pane && dialog.kind !== "swap") return null;
  const submit = () => {
    const key = `pane:${dialog.paneId}`;
    let accepted = false;
    if (dialog.kind === "rename" && pane) accepted = mutate(key, { type: "pane_rename", pane_id: pane.id, label: value.trim() || null });
    if (dialog.kind === "swap" && value) { onSwap(dialog.paneId, value); accepted = true; }
    if (dialog.kind === "move" && pane && value && confirmMove(pane)) {
      if (value === "new-tab") accepted = mutate(key, { type: "pane_move", pane_id: pane.id, destination: { type: "new_tab", space_id: pane.space_id, label: null } }, true);
      if (value === "new-space") accepted = mutate(key, { type: "pane_move", pane_id: pane.id, destination: { type: "new_space", label: null, tab_label: null } }, true);
      if (value.startsWith("tab:")) accepted = mutate(key, { type: "pane_move", pane_id: pane.id, destination: { type: "existing_tab", tab_id: value.slice(4), direction: "right", target_pane_id: null, ratio: null } }, true);
    }
    if (accepted) onDismiss();
  };
  const onChooserKeyDown = (event: ReactKeyboardEvent<HTMLFormElement>) => {
    if (event.target instanceof HTMLSelectElement && event.ctrlKey && !event.altKey && !event.metaKey
      && (event.key.toLowerCase() === "n" || event.key.toLowerCase() === "p")) {
      const choices = [...event.target.options].filter((option) => option.value !== "");
      if (choices.length > 0) {
        event.preventDefault();
        const current = choices.findIndex((option) => option.value === value);
        const direction = event.key.toLowerCase() === "n" ? 1 : -1;
        const next = current < 0 ? (direction > 0 ? 0 : choices.length - 1) : (current + direction + choices.length) % choices.length;
        setValue(choices[next].value);
      }
      return;
    }
    trapModalTab(event, ref.current);
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><form ref={ref} className="chooser-overlay" role="dialog" aria-modal="true" aria-labelledby="chooser-title" onSubmit={(event) => { event.preventDefault(); submit(); }} onKeyDown={onChooserKeyDown}>
    <h2 id="chooser-title">{dialog.kind} pane</h2>
    {dialog.kind === "rename" ? <input aria-label="Pane name" value={value} onChange={(event) => setValue(event.target.value)} /> : <select aria-label={dialog.kind === "swap" ? "Swap target" : "Move destination"} value={value} onChange={(event) => setValue(event.target.value)}><option value="">Choose...</option>{dialog.kind === "swap" ? leafChoices.filter(candidate => candidate.id !== dialog.paneId).map(candidate => <option key={candidate.id} value={candidate.id}>{candidate.title}</option>) : <><option value="new-tab">New tab in this space</option><option value="new-space">New space</option>{tabs.filter(tab => tab.id !== pane?.tab_id).map(tab => <option key={tab.id} value={`tab:${tab.id}`}>{moveDestinationLabel(tab, spaces)}</option>)}</>}</select>}
    <footer><button type="button" onClick={onDismiss}>Cancel</button><button type="submit" disabled={busy || (dialog.kind !== "rename" && !value)}>{dialog.kind}</button></footer>
  </form></div>;
}

export function reconcileSessionChoice(sessions: SessionSummary[], selected: string, currentSessionId: string | null): string {
  if (sessions.some((session) => session.id === selected)) return selected;
  if (currentSessionId && sessions.some((session) => session.id === currentSessionId)) return currentSessionId;
  return sessions[0]?.id ?? "";
}

function SessionDialogOverlay({ sessions, currentSessionId, onRefresh, onDismiss, onSession }: { sessions: SessionSummary[]; currentSessionId: string | null; onRefresh: () => Promise<void>; onDismiss: () => void; onSession: (sessionId: string) => void }) {
  const [sessionId, setSessionId] = useState(() => reconcileSessionChoice(sessions, "", currentSessionId));
  const [query, setQuery] = useState("");
  const selectedSessionRef = useRef<HTMLButtonElement | null>(null);
  const ref = useModalFocus<HTMLFormElement>(onDismiss);
  useEffect(() => setSessionId((selected) => reconcileSessionChoice(sessions, selected, currentSessionId)), [sessions, currentSessionId]);
  const normalized = query.trim().toLocaleLowerCase();
  const filteredSessions = sessions.filter((session) => !normalized || `${session.label} ${session.id} ${session.running ? "running" : "stopped"}`.toLocaleLowerCase().includes(normalized));
  useEffect(() => { selectedSessionRef.current?.scrollIntoView?.({ block: "nearest" }); }, [sessionId, normalized]);
  const selectRelativeSession = (direction: 1 | -1) => {
    if (filteredSessions.length === 0) return;
    const current = filteredSessions.findIndex((session) => session.id === sessionId);
    const next = current < 0 ? (direction === 1 ? 0 : filteredSessions.length - 1) : (current + direction + filteredSessions.length) % filteredSessions.length;
    setSessionId(filteredSessions[next].id);
  };
  return <div className="overlay-scrim" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onDismiss(); }}><form ref={ref} className="chooser-overlay session-chooser" role="dialog" aria-modal="true" aria-labelledby="session-chooser-title" onSubmit={(event) => { event.preventDefault(); if (sessionId && sessionId !== currentSessionId) onSession(sessionId); onDismiss(); }} onKeyDown={(event) => {
    if (event.ctrlKey && !event.altKey && !event.metaKey && (event.key.toLowerCase() === "n" || event.key.toLowerCase() === "p")) { event.preventDefault(); selectRelativeSession(event.key.toLowerCase() === "n" ? 1 : -1); return; }
    trapModalTab(event, ref.current);
  }}>
    <h2 id="session-chooser-title">Switch session</h2>
    {sessions.length === 0 ? <div className="empty-choice" role="status">No sessions are available.</div> : <><input className="session-search" aria-label="Find a session" placeholder="Find a session…" autoComplete="off" value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={(event) => { if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); selectRelativeSession(event.key === "ArrowDown" ? 1 : -1); } }} /><div className="session-list" role="listbox" aria-label="Session">{filteredSessions.length === 0 ? <p className="empty-choice" role="status">No sessions match.</p> : filteredSessions.map((session) => <button ref={session.id === sessionId ? selectedSessionRef : null} key={session.id} type="button" role="option" aria-selected={session.id === sessionId} data-session-id={session.id} className={`session-choice${session.id === sessionId ? " is-selected" : ""}`} onClick={() => setSessionId(session.id)}><span>{session.label}</span><small>{session.running ? "running" : "stopped"}</small></button>)}</div></>}
    <footer>{sessions.length === 0 ? <button type="button" onClick={() => { void onRefresh().catch(() => undefined); }}>Refresh</button> : null}<button type="button" onClick={onDismiss}>Cancel</button><button type="submit" disabled={!sessionId || sessionId === currentSessionId}>Switch</button></footer>
  </form></div>;
}

export function mutationFailureCanRetry(request: ResourceMutationRequest, code: string | undefined): boolean {
  if (code === "mutation_applied_snapshot_failed" || code === "request_outcome_unknown") return false;
  return request.type === "space_rename" || request.type === "space_move_block" || request.type === "tab_rename" || request.type === "tab_move" || request.type === "pane_rename";
}

function RecoveryPanel({ state, mutations, onReconnect, onRetryMutation }: { state: SessionState; mutations: MutationCoordinatorState; onReconnect: () => void; onRetryMutation: (operation: MutationOperation) => void }) {
  const failures = Object.values(mutations.errors);
  if (!state.syncError && failures.length === 0) return null;
  return <aside className="recovery-panel" aria-label="Recovery" role="alert">
    {state.syncError ? <div><span>{state.syncError.message}</span><button type="button" onClick={onReconnect}>Resync</button></div> : null}
    {failures.map((failure) => <div key={`${failure.operation.key}:${failure.operation.token}`}><span>{failure.message}</span>{failure.code ? <code>{failure.code}</code> : null}{mutationFailureCanRetry(failure.operation.request, failure.code) ? <button type="button" onClick={() => onRetryMutation(failure.operation)}>Retry</button> : null}<button type="button" onClick={onReconnect}>Resync</button></div>)}

  </aside>;
}

const SIDEBAR_MIN_WIDTH = 224;
const SIDEBAR_MAX_WIDTH = 360;
const SIDEBAR_DEFAULT_WIDTH = 240;
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

function Workbench({ client, state, sessions, selection, terminalMouseInput, mutations, ctx, tabLayout, registerTransient, onSession, onFocus, onSelectLeaf, onSplit, onPanePrepared, onReconnect, onRetry, onRefreshSessions, onOpenSession, onMutate, onRetryMutation, layoutError, onDismissLayoutError }: {
  client: CockpitClient; state: SessionState; sessions: SessionSummary[]; selection: Selection; terminalMouseInput: boolean; mutations: MutationCoordinatorState;
  ctx: LeafCtx; tabLayout: TabLayoutState | null; registerTransient(cancel: () => void): () => void;
  layoutError: string | null; onDismissLayoutError(): void;
  onSession(id: string): void; onFocus(request: FocusRequest, location: Selection, prepare?: { paneId: string }): void;
  onSelectLeaf(tabId: string, leafId: string): void; onSplit(tabId: string, leafId: string, direction: "right" | "down"): void;
  onPanePrepared(paneId: string): void; onReconnect(): void; onRetry(): void; onRefreshSessions(): Promise<void>; onOpenSession(): void; onMutate: Mutate; onRetryMutation(operation: MutationOperation): void;
}) {
  const snapshot = state.snapshot;
  const shell = snapshot?.herdr_shell ?? null;
  const popup = shell?.popup ?? null;
  const customBindings = herdrBindings(shell?.commands ?? []);
  const customPrefixes = herdrPrefixes(shell?.prefix_bindings ?? []);
  setEffectiveHerdrBindings(customBindings, customPrefixes);
  useEffect(() => () => setEffectiveHerdrBindings([]), []);
  const [popupPending, setPopupPending] = useState<string | null>(null);
  const [commandNotice, setCommandNotice] = useState<string | null>(null);
  const spaces = snapshot?.spaces ?? [];
  const allTabs = snapshot?.tabs ?? [];
  const tabs = tabsForSpace(allTabs, selection.spaceId);
  const selectedSpace = byId(spaces, selection.spaceId);
  const selectedTab = byId(tabs, selection.tabId);
  const panes = panesForTab(snapshot?.panes ?? [], selectedTab?.id ?? null);
  const localLeaves = leaves(tabLayout?.root ?? null);
  const selectedLeaf = localLeaves.find(leaf => leaf.id === selection.paneId);
  const selectedPane = byId(panes, selection.paneId);
  const sourcePaneId = tabLayout ? runtimeSource(tabLayout, selection.paneId, snapshot?.focused_pane_id) : null;
  const librarySpace: LibrarySpace | null = state.sessionId && selectedSpace
    ? { target: { session_id: state.sessionId, space_id: selectedSpace.id }, label: selectedSpace.label, live: state.sync === "live" } : null;
  const setupParent = setupParentFor(selectedSpace, snapshot?.panes ?? [], snapshot?.focused_pane_id ?? null);
  const spaceGit = useSpaceGitStatus(client, state.sync === "live" ? state.sessionId : null, spaceCheckoutKey(spaces, snapshot?.panes ?? []));
  const [libraryOpen, setLibraryOpen] = useState(false);
  const canvasRef = useRef<HTMLDivElement>(null);
  const [area, setArea] = useState<Rect>({ x: 0, y: 0, width: 800, height: 600 });
  useLayoutEffect(() => {
    const element = Array.from(canvasRef.current?.querySelectorAll<HTMLElement>(".tab-canvas") ?? []).find(node => node.closest<HTMLElement>("[data-tab-id]")?.dataset.tabId === selection.tabId);
    if (!element || libraryOpen) return;
    const measure = () => {
      const rect = element.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) setArea(current => current.width === rect.width && current.height === rect.height ? current : { x: 0, y: 0, width: rect.width, height: rect.height });
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(element);
    window.addEventListener("resize", measure);
    return () => { observer?.disconnect(); window.removeEventListener("resize", measure); };
  }, [libraryOpen, selection.tabId, tabLayout?.zoomLeafId]);
  const streamRegistry = useRef(new Set<TerminalStream>());
  const attachedPaneIds = useRef(new Set<string>());
  const registerStream = useCallback((stream: TerminalStream, active: boolean) => { if (active) streamRegistry.current.add(stream); else streamRegistry.current.delete(stream); }, []);
  useEffect(() => () => { streamRegistry.current.forEach(stream => stream.close()); streamRegistry.current.clear(); }, []);
  const [lifecycleError, setLifecycleError] = useState<string | null>(null);
  const [, setBrowserLifecycleRevision] = useState(0);
  useEffect(() => subscribeBrowserLifecycle(() => setBrowserLifecycleRevision(value => value + 1)), []);
  const browserOpen = Boolean(tabLayout?.viewers.browser);
  const browserInputActive = selectedLeaf?.kind === "browser" && !libraryOpen;
  const browserReason = !selectedTab ? "Select a tab first" : state.sync !== "live" ? "Herdr is not live" : browserOpenDisabledReason(ctx, selectedTab.id);
  const perform = (operation: Promise<void>) => { setLifecycleError(null); void operation.catch(error => setLifecycleError(describeError(error, "Could not change this pane").message)); };
  const openBrowser = () => { if (selectedTab && !browserReason) { setLibraryOpen(false); setAttachFocusSuppressed(false); perform(openBrowserLeaf(ctx, selectedTab.id, "row")); } };
  const toggleBrowser = () => { if (!selectedTab) return; setLibraryOpen(false); setAttachFocusSuppressed(false); if (browserOpen) perform(closeBrowserLeaf(ctx, selectedTab.id)); else openBrowser(); };
  const [viewerSources, setViewerSources] = useState<ViewerSourceOptions | null>(null);
  const [viewerSourcesError, setViewerSourcesError] = useState<string | null>(null);
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
  const [sessionChooserOpen, setSessionChooserOpen] = useState(false);
  const [setupOpen, setSetupOpen] = useState(false);
  const [recoveryOpen, setRecoveryOpen] = useState(false);
  const [teardownSpaceId, setTeardownSpaceId] = useState<string | null>(null);
  const [libraryAddOpen, setLibraryAddOpen] = useState(false);
  const [libraryCommand, setLibraryCommand] = useState<LibraryCommand | null>(null);
  const libraryCommandToken = useRef(0);
  // Preserve an explicit sidebar invoker; other Library closes let the selected terminal attach with focus.
  const [attachFocusSuppressed, setAttachFocusSuppressed] = useState(false);
  const libraryOrigin = useRef<{ paneId: string; graphical: boolean } | null>(null);
  const librarySidebarInvoker = useRef<HTMLElement | null>(null);
  const selectedPaneIdRef = useRef(selection.paneId);
  selectedPaneIdRef.current = selection.paneId;
  const openLibrary = useCallback((command?: { kind: "refresh" } | { kind: "tokens" } | { kind: "open"; itemId: string }) => {
    const active = document.activeElement;
    librarySidebarInvoker.current = null;
    libraryOrigin.current = active instanceof HTMLElement && active.closest(".pane-view") && selectedPaneIdRef.current
      ? { paneId: selectedPaneIdRef.current, graphical: active.closest<HTMLElement>("[data-kind]")?.dataset.kind !== "terminal" }
      : null;
    setLibraryOpen(true);
    // A command belongs to this opening only; reopening must not replay it.
    setLibraryCommand(command ? { ...command, token: ++libraryCommandToken.current } : null);
  }, []);
  const closeLibrary = useCallback(() => {
    const origin = libraryOrigin.current;
    libraryOrigin.current = null;
    const sidebarInvoker = librarySidebarInvoker.current;
    librarySidebarInvoker.current = null;
    const returnToSidebar = Boolean(sidebarInvoker?.isConnected && !sidebarInvoker.closest("[inert]"));
    const returnToPane = origin !== null && origin.paneId === selectedPaneIdRef.current;
    setLibraryOpen(false);
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
  const [prefixActive, setPrefixActive] = useState(false);
  const [armedPrefix, setArmedPrefix] = useState<{ origin: "cockpit" | "herdr"; label: string }>({ origin: "cockpit", label: "Ctrl+B" });
  const [prefixHint, setPrefixHint] = useState<string | null>(null);
  useEffect(() => {
    if (prefixHint === null) return;
    const timer = window.setTimeout(() => setPrefixHint(null), 2000);
    return () => window.clearTimeout(timer);
  }, [prefixHint]);
  const [sidebarWidth, setSidebarWidth] = useState(readSidebarWidth);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(readSidebarCollapsed);
  const [narrowViewport, setNarrowViewport] = useState(isNarrowViewport);
  const [drawerOpen, setDrawerOpen] = useState(() => !isNarrowViewport());
  const sidebarReturnFocus = useRef<HTMLElement | null>(null);
  const sidebarCloseRef = useRef<HTMLButtonElement | null>(null);
  const drawerFocusTarget = useRef<{ spaceId: string; paneId: string | null } | null>(null);
  const mutationBusy = mutations.pending !== null;
  const modalOpen = Boolean(popup || popupPending) || dialog !== null || commandsOpen || sessionChooserOpen || setupOpen || recoveryOpen || teardownSpaceId !== null || libraryAddOpen;
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
  const sidebarSession = sessions.find((session) => session.id === state.sessionId);
  const openSessionChooser = useCallback(() => {
    setSessionChooserOpen(true);
    onOpenSession();
  }, [onOpenSession]);
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
  useEffect(() => {
    if ((!menu && !commandsOpen) || !sourcePaneId || !state.sessionId || state.sync !== "live") { setViewerSources(null); return; }
    let active = true;
    setViewerSources(null); setViewerSourcesError(null);
    void client.viewerSources(state.sessionId, sourcePaneId).then(value => { if (active) setViewerSources(value); }, error => { if (active) setViewerSourcesError(describeError(error, "Could not inspect viewer sources").message); });
    return () => { active = false; };
  }, [client, menu, commandsOpen, sourcePaneId, state.sessionId, state.sync]);
  // Selecting a Space keeps the Library open; selecting a tab, pane or agent closes it to show that pane.
  const focusSpace = (space: Space) => {
    if (modalOpen) return;
    setAttachFocusSuppressed(false);
    const tabId = allTabs.find((tab) => tab.space_id === space.id && tab.focused)?.id ?? null;
    const paneId = snapshot?.panes.find((pane) => pane.space_id === space.id && pane.focused)?.id ?? null;
    if (narrowViewport) drawerFocusTarget.current = { spaceId: space.id, paneId };
    onFocus({ kind: "space", target_id: space.id }, { spaceId: space.id, tabId, paneId });
  };
  const focusTab = (tab: Tab) => {
    if (modalOpen) return;
    setLibraryOpen(false); setAttachFocusSuppressed(false);
    const remembered = ctx.getState().tabs[tab.id];
    const paneId = remembered?.selectedLeafId ?? tab.focused_pane_id ?? null;
    const focusedTerminal = tab.focused_pane_id;
    const painted = remembered?.zoomLeafId ? remembered.zoomLeafId === focusedTerminal : true;
    const prepare = focusedTerminal && painted && state.sync === "live" && snapshot?.focused_tab_id !== tab.id && !attachedPaneIds.current.has(focusedTerminal) ? { paneId: focusedTerminal } : undefined;
    onFocus({ kind: "tab", target_id: tab.id }, { spaceId: tab.space_id, tabId: tab.id, paneId }, prepare);
  };
  const selectLeaf = (leafId: string) => {
    if (modalOpen || !tabLayout) return;
    setLibraryOpen(false); setAttachFocusSuppressed(false);
    onSelectLeaf(tabLayout.tabId, leafId);
  };
  const focusAgent = (agent: Agent) => {
    if (modalOpen) return;
    setLibraryOpen(false); setAttachFocusSuppressed(false);
    if (narrowViewport) drawerFocusTarget.current = { spaceId: agent.space_id, paneId: agent.pane_id };
    onFocus({ kind: "agent", target_id: agent.pane_id }, { spaceId: agent.space_id, tabId: agent.tab_id, paneId: agent.pane_id });
  };
  const beginRename = (target: ContextTarget | null): boolean => {
    if (!target || mutationBusy) return false;
    if (target.kind === "pane") setDialog({ kind: "rename", paneId: target.id });
    else setEditing(target);
    return true;
  };
  const closeSpace = (space: Space | undefined): boolean => Boolean(space && window.confirm(`Close Space "${space.label}" and all of its tabs and panes?`) && onMutate(`space:${space.id}`, { type: "space_close", space_id: space.id }));
  const closeTab = (tab: Tab | undefined): boolean => Boolean(tab && window.confirm(`Close tab "${tab.label}" and all of its panes?`) && onMutate(`tab:${tab.id}`, { type: "tab_close", tab_id: tab.id }));
  const lastTerminalMessage = (pane: Pane, operation: "Closing" | "Moving") => {
    const tab = ctx.getState().tabs[pane.tab_id];
    if (!tab || Object.keys(tab.terminals).length !== 1 || !Object.values(tab.viewers).some(Boolean)) return null;
    const browser = tab.viewers.browser ? ", and deletes the browser's profile (cookies, logins, site data)" : "";
    const label = allTabs.find(candidate => candidate.id === pane.tab_id)?.label || pane.tab_id;
    return `This is the last terminal in ${label}. ${operation} it also closes Files, Review and Browser in this tab${browser}. Drafts and comments you saved stay.`;
  };
  const closeLeaf = (leafId: string): boolean => {
    if (!tabLayout) return false;
    const leaf = localLeaves.find(candidate => candidate.id === leafId);
    if (!leaf) return false;
    if (leaf.kind === "browser") { perform(closeBrowserLeaf(ctx, tabLayout.tabId)); return true; }
    if (leaf.kind !== "terminal") { perform(closeViewerLeaf(ctx, tabLayout.tabId, leaf.kind)); return true; }
    const pane = byId(panes, leafId);
    if (!pane) return false;
    const message = lastTerminalMessage(pane, "Closing") ?? `Close ${pane.title || "Terminal"}?`;
    return window.confirm(message) && onMutate(`pane:${pane.id}`, { type: "pane_close", pane_id: pane.id });
  };
  const swap = (source: string, target: string) => {
    if (!tabLayout?.root) return;
    const rect = solveLayout(tabLayout.root, area).leaves.get(target);
    if (rect) ctx.dispatch({ type: "drop", tabId: tabLayout.tabId, src: source, target: { kind: "swap", target, rect, label: "Swap" }, revision: tabLayout.revision });
  };
  const zoom = (leafId: string) => { if (tabLayout) ctx.dispatch({ type: "zoom-toggle", tabId: tabLayout.tabId, leafId }); };
  const neighbour = (direction: PaneFocusDirection) => {
    if (!tabLayout?.root || !selection.paneId) return null;
    const solved = solveLayout(tabLayout.root, area).leaves;
    const current = solved.get(selection.paneId);
    if (!current) return null;
    const cx = current.x + current.width / 2, cy = current.y + current.height / 2;
    const candidates = localLeaves.flatMap(leaf => {
      const rect = solved.get(leaf.id);
      if (!rect || leaf.id === selection.paneId) return [];
      const dx = rect.x + rect.width / 2 - cx, dy = rect.y + rect.height / 2 - cy;
      const primary = direction === "left" ? -dx : direction === "right" ? dx : direction === "up" ? -dy : dy;
      const secondary = direction === "left" || direction === "right" ? Math.abs(dy) : Math.abs(dx);
      return primary > 0 ? [{ id: leaf.id, score: primary + secondary * 2 }] : [];
    });
    candidates.sort((a, b) => a.score - b.score);
    return candidates[0]?.id ?? null;
  };
  const viewerCapability = (kind: RendererActionDefinition["kind"]) => kind === "review" ? Boolean(viewerSources?.review_repository_ids.length) : kind === "files" ? Boolean(viewerSources?.files_folder_root_id) : Boolean(viewerSources?.files_context_root_id);
  const openViewer = (kind: RendererActionDefinition["kind"]) => {
    if (!tabLayout || !sourcePaneId || !viewerCapability(kind) || mutationBusy || state.sync !== "live") return;
    const selector = kind === "review" ? { kind: "review" as const, repositoryId: viewerSources!.review_repository_ids[0] } : { kind: kind === "files" ? "files_folder" as const : "files_context" as const };
    setLibraryOpen(false); setAttachFocusSuppressed(false);
    perform(openViewerLeaf(ctx, tabLayout.tabId, kind === "review" ? "review" : "files", selector, "row", sourcePaneId));
  };
  const runCommand = useCallback((command: PrefixCommand) => {
    setPrefixHint(null);
    const space = byId(spaces, selection.spaceId);
    const tab = byId(tabs, selection.tabId);
    const pane = byId(panes, selection.paneId);
    const execute = () => {
      if (command === "new-space") onMutate("space:new", { type: "space_create", label: null, cwd: null }, true);
      if (command === "setup-space" && state.sync === "live") setSetupOpen(true);
      if (command === "rename-space" && space) beginRename({ kind: "space", id: space.id });
      if (command === "close-space") closeSpace(space);
      if (command === "new-tab" && selection.spaceId) onMutate("tab:new", { type: "tab_create", space_id: selection.spaceId, label: null }, true);
      if (command === "rename-tab" && tab) beginRename({ kind: "tab", id: tab.id });
      if (command === "close-tab") closeTab(tab);
      if (command === "split-right" && tabLayout && selectedLeaf) onSplit(tabLayout.tabId, selectedLeaf.id, "right");
      if (command === "split-down" && tabLayout && selectedLeaf) onSplit(tabLayout.tabId, selectedLeaf.id, "down");
      if (command === "close-pane" && selectedLeaf) closeLeaf(selectedLeaf.id);
      if (command === "zoom-pane" && selectedLeaf) zoom(selectedLeaf.id);
      if (command === "rename-pane" && pane) beginRename({ kind: "pane", id: pane.id });
      if (command === "previous-tab" && tab) { const index = tabs.indexOf(tab); if (index > 0) focusTab(tabs[index - 1]); }
      if (command === "next-tab" && tab) { const index = tabs.indexOf(tab); if (index >= 0 && index < tabs.length - 1) focusTab(tabs[index + 1]); }
      if (command.startsWith("select-tab-")) { const target = tabs[Number(command.slice("select-tab-".length)) - 1]; if (target) focusTab(target); }
      if ((command === "previous-pane" || command === "next-pane") && selectedLeaf) {
        const index = localLeaves.findIndex(leaf => leaf.id === selectedLeaf.id);
        const offset = command === "next-pane" ? 1 : -1;
        selectLeaf(localLeaves[(index + offset + localLeaves.length) % localLeaves.length].id);
      }
      if (["focus-left", "focus-right", "focus-up", "focus-down"].includes(command)) {
        const target = neighbour(command.slice("focus-".length) as PaneFocusDirection);
        if (target) selectLeaf(target);
      }
      if (["swap-left", "swap-right", "swap-up", "swap-down"].includes(command) && selectedLeaf) {
        const target = neighbour(command.slice("swap-".length) as PaneFocusDirection);
        if (target) swap(selectedLeaf.id, target);
      }
      if (command === "resize" && !mutationBusy && canvasRef.current && selection.paneId) focusSelectedDivider(canvasRef.current, selection.paneId);
      if (command === "open-file-picker") dispatchFileNavigation("open-picker");
      if (command === "switch-session") openSessionChooser();
      if (command === "toggle-browser") toggleBrowser();
      if (command === "toggle-library") { if (libraryOpen) closeLibrary(); else openLibrary(); }
      if (command === "toggle-sidebar") {
        const visible = narrowViewport ? drawerOpen : !sidebarCollapsed;
        if (narrowViewport) { if (drawerOpen) closeDrawer(); else openDrawer(); } else toggleSidebarCollapsed();
        // A collapsed sidebar cannot keep focus; the selected tab is a safe target that sends nothing to Herdr.
        if (visible && !narrowViewport && document.activeElement?.closest("#cockpit-sidebar")) {
          window.setTimeout(() => document.querySelector<HTMLElement>('.tab-button[aria-selected="true"], .drawer-toggle')?.focus({ preventScroll: true }), 0);
        }
      }
      if (command === "focus-spaces" || command === "focus-agents") {
        const list = command === "focus-spaces" ? "spaces" : "agents";
        const visible = narrowViewport ? drawerOpen : !sidebarCollapsed;
        if (!visible) { if (narrowViewport) openDrawer(); else toggleSidebarCollapsed(); }
        // The sidebar may still be mounting (and the drawer focuses its close button first): retry until a row holds focus.
        const attempt = (remaining: number) => {
          focusSidebarList(list);
          const focused = document.activeElement;
          if (remaining > 0 && !(focused?.closest("#cockpit-sidebar") && !focused.matches(".sidebar-close"))) window.setTimeout(() => attempt(remaining - 1), 30);
        };
        window.setTimeout(() => attempt(20), visible ? 0 : 60);
      }
    };
    if (command === "help") { setCommandsOpen(true); return; }
    if (mutationBusy && !COMMANDS_ALLOWED_WHILE_BUSY.includes(command)) return;
    if (shortcutEntry(command).paneScoped && libraryOpen) {
      // The command acts on a pane the Library covers: show the pane first, then run, so a confirmation names a visible target.
      libraryOrigin.current = null;
      setAttachFocusSuppressed(false);
      flushSync(() => setLibraryOpen(false));
      requestAnimationFrame(() => window.setTimeout(execute, 0));
      return;
    }
    execute();
  }, [spaces, tabs, panes, tabLayout, area, localLeaves, selectedLeaf, snapshot, selection.spaceId, selection.tabId, selection.paneId, mutationBusy, modalOpen, libraryOpen, narrowViewport, drawerOpen, sidebarCollapsed, state.sync, browserOpen, browserReason, closeLibrary, openLibrary, openSessionChooser, closeDrawer, openDrawer, toggleSidebarCollapsed]);
  const customCommandReason = state.sync !== "live" || shell?.status !== "live" ? shell?.error ?? "Herdr commands are not live"
    : state.focusPending || state.focusError || !snapshot?.focused_space_id || !snapshot.focused_tab_id ? "Waiting for Herdr focus"
    : mutationBusy ? "Another Herdr action is pending" : undefined;
  const runHerdrCommand = useCallback((command: HerdrCommand) => {
    setPrefixHint(null);
    if (customCommandReason || popup) { setCommandNotice(customCommandReason ?? "The popup owns keyboard input"); return; }
    if (!shell?.commands.some(candidate => candidate.command_id === command.command_id)) { setCommandNotice("Custom command is not available on this endpoint; reload configuration"); return; }
    const accepted = onMutate(`command:${command.command_id}`, { type: "command_invoke", command_id: command.command_id, space_id: snapshot!.focused_space_id!, tab_id: snapshot!.focused_tab_id!, pane_id: snapshot!.focused_pane_id }, false);
    if (accepted) {
      setCommandNotice(null);
      if (command.action === "popup") setPopupPending(command.command_id);
    }
  }, [customCommandReason, popup, shell, snapshot, onMutate]);
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      routeWorkbenchKeydown(event, { modalOpen, serverModalOpen: Boolean(popup), popupPending: Boolean(popupPending), prefixActive, prefixOrigin: armedPrefix.origin, herdrPrefixes: customPrefixes, onPrefixArm: (origin, label) => setArmedPrefix({ origin, label }), runCommand, herdrBindings: customBindings, runHerdrCommand, setPrefixActive, setCommandsOpen, onUnboundPrefixKey: setPrefixHint });
    };
    window.addEventListener("keydown", keydown, true);
    return () => window.removeEventListener("keydown", keydown, true);
  }, [prefixActive, armedPrefix, runCommand, modalOpen, popup, popupPending, shell, runHerdrCommand]);
  const openContext = (event: ContextAnchor, target: ContextTarget) => { event.preventDefault(); event.stopPropagation(); if (!mutationBusy && !modalOpen) setMenu({ target, x: event.clientX, y: event.clientY }); };
  const dismissMenu = useCallback(() => setMenu(null), []);
  const menuAction = (action: () => boolean | void) => { if (action() !== false) dismissMenu(); };
  const renderMenu = () => {
    if (!menu) return null;
    const disabled = mutationBusy;
    if (menu.target.kind === "space") {
      const space = spaces.find((candidate) => candidate.id === menu.target.id);
      if (!space) return null;
      return <ContextMenu menu={menu} onDismiss={dismissMenu}>
        <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}><UiIcon name="edit" />Rename</button>
        <button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeSpace(space))}><UiIcon name="close" />Close</button>
        <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => setTeardownSpaceId(space.id))}><UiIcon name="trash" />Review task cleanup…</button>
      </ContextMenu>;
    }
    if (menu.target.kind === "tab") {
      const tab = allTabs.find((candidate) => candidate.id === menu.target.id);
      if (!tab) return null;
      return <ContextMenu menu={menu} onDismiss={dismissMenu}><button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}><UiIcon name="edit" />Rename</button><button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeTab(tab))}><UiIcon name="close" />Close</button></ContextMenu>;
    }
    const leaf = localLeaves.find(candidate => candidate.id === menu.target.id);
    if (!leaf || !tabLayout) return null;
    const pane = byId(panes, leaf.id);
    return <ContextMenu menu={menu} onDismiss={dismissMenu}>
      <p className="context-menu-heading" role="presentation">Selected pane · {leaf.kind.charAt(0).toUpperCase() + leaf.kind.slice(1)}</p>
      <button role="menuitem" type="button" onClick={() => menuAction(() => zoom(leaf.id))}><UiIcon name="expand" />Expand / restore pane</button>
      {(leaf.kind === "files" || leaf.kind === "review") ? <button role="menuitem" type="button" onClick={() => menuAction(() => dispatchFileNavigation("open-picker"))}><UiIcon name="search" />Go to file…</button> : null}
      <div className="context-menu-separator" role="presentation" />
      <p className="context-menu-heading" role="presentation">Open view</p>
      {rendererActionDefinitions.map(({ id, icon, kind }) => <button key={id} role="menuitem" type="button" disabled={disabled || !viewerCapability(kind) || state.sync !== "live"} title={viewerSources?.reason ?? viewerSourcesError ?? "Loading viewer sources"} onClick={() => menuAction(() => openViewer(kind))}><UiIcon name={icon} /><span>{kind === "review" ? "Review" : kind === "files" ? "Files" : "Context"}</span>{kind === "context" && !viewerCapability(kind) ? <small className="context-menu-reason">no Space context</small> : null}</button>)}
      <button role="menuitem" type="button" disabled={Boolean(browserReason)} title={browserReason ?? undefined} onClick={() => menuAction(openBrowser)}><UiIcon name="browser" />Browser</button>
      <div className="context-menu-separator" role="presentation" />
      {pane ? <button role="menuitem" type="button" disabled={disabled} onClick={() => menuAction(() => beginRename(menu.target))}><UiIcon name="edit" />Rename pane…</button> : null}
      <button role="menuitem" type="button" aria-haspopup="dialog" disabled={localLeaves.length < 2} onClick={() => menuAction(() => setDialog({ kind: "swap", paneId: leaf.id }))}><UiIcon name="refresh" /><span>Swap with…</span><span className="context-menu-chevron"><UiIcon name="right" /></span></button>
      {pane ? <button role="menuitem" type="button" aria-haspopup="dialog" disabled={disabled} onClick={() => menuAction(() => setDialog({ kind: "move", paneId: pane.id }))}><UiIcon name="forward" /><span>Move to…</span><span className="context-menu-chevron"><UiIcon name="right" /></span></button> : null}
      <div className="context-menu-separator" role="presentation" />
      <button role="menuitem" type="button" disabled={disabled} className="destructive" onClick={() => menuAction(() => closeLeaf(leaf.id))}><UiIcon name="close" />Close pane</button>
    </ContextMenu>;
  };
  const commandActionRows: CommandAction[] = [
    ...(shell?.commands ?? []).filter(command => command.action !== "unknown").map((command): CommandAction => ({
      id: `herdr:${command.command_id}`, label: command.description ? command.description.charAt(0).toUpperCase() + command.description.slice(1) : "Custom command", icon: command.action === "popup" ? "more" : command.action === "plugin_action" ? "file" : "terminal", shortcut: herdrCommandShortcut(command, customPrefixes), group: "Herdr",
      disabled: Boolean(customCommandReason || popup), reason: customCommandReason, run: () => runHerdrCommand(command),
    })),
    ...SHORTCUTS.filter((entry) => entry.prefix && entry.palette !== false).map((entry): CommandAction => {
      const command = entry.id as PrefixCommand;
      const reason = entry.needs === "space" && !selectedSpace ? "Select a Space first"
        : entry.needs === "tab" && !selectedTab ? "Select a tab first"
        : entry.needs === "pane" && !selectedLeaf ? "Select a pane first"
        : command === "rename-pane" && !selectedPane ? "Terminals only"
        : command === "setup-space" && state.sync !== "live" ? "Herdr is not live"
        : command === "toggle-browser" ? browserOpen ? undefined : browserReason ?? undefined : undefined;
      return {
        id: `prefix:${command}`, label: command === "toggle-library" && libraryOpen ? "Close Library" : entry.label, shortcut: formatShortcut(command), group: entry.group,
        disabled: reason !== undefined, reason, run: () => runCommand(command),
      };
    }),
    { id: "recovery:cleanup", label: "Recover task cleanup…", group: "Navigate", run: () => setRecoveryOpen(true) },
    { id: "browser:open", label: "Open Browser", group: "Browser", disabled: Boolean(browserReason), reason: browserReason ?? undefined, run: openBrowser },
    { id: "browser:close", label: "Close browser", group: "Browser", disabled: !browserOpen, reason: !browserOpen ? "No browser in this tab" : undefined, run: () => { if (selectedTab) perform(closeBrowserLeaf(ctx, selectedTab.id)); } },
    { id: "browser:cleanup", label: "Retry browser cleanup", group: "Browser", run: () => perform(retryBrowserCleanup(ctx)) },
    { id: "library:add", label: "Add to Library…", group: "Library", run: () => setLibraryAddOpen(true) },
    { id: "library:refresh", label: "Refresh Library", group: "Library", run: () => openLibrary({ kind: "refresh" }) },
    { id: "library:tokens", label: "Provider tokens…", group: "Library", run: () => openLibrary({ kind: "tokens" }) },
    ...rendererActionDefinitions.map(({ id, label, icon, kind }) => {
      const capability = viewerCapability(kind);
      const fallback = kind === "context" ? "Context requires a configured companion directory" : "Select a terminal with a configured repository";
      const detail = viewerSources?.reason ?? viewerSourcesError ?? (sourcePaneId ? "Loading viewer sources" : "No terminal in this tab");
      const reason = state.sync !== "live" ? "Herdr is not live" : rendererReasonFor(kind, detail) ?? detail ?? fallback;
      return { id: `renderer:${id}`, label, icon, group: "Pane" as const, disabled: mutationBusy || !capability || state.sync !== "live", reason: kind === "context" && !capability && state.sync === "live" ? "no Space context" : reason, reasonDetail: detail, run: () => openViewer(kind) };
    }),
  ];
  const browserToggleShortcut = formatShortcut("toggle-browser");
  const commandActions: CommandAction[] = commandActionRows.map((action) => action.id === "browser:open" || action.id === "browser:close" ? { ...action, shortcut: browserToggleShortcut } : action);
  const commandFailures = Object.values(mutations.errors).filter(failure => failure.operation.request.type === "command_invoke");
  const commandFailureMessage = commandFailures.map(failure => `${failure.code === "request_outcome_unknown" ? "Herdr did not confirm this command. Check the session before running it again." : "Could not run Herdr command:"} ${failure.message}`).join(" ");
  const commandStatus = commandNotice || commandFailureMessage || lifecycleError ? <p role="alert">{commandNotice || commandFailureMessage || lifecycleError}</p> : null;
  const dispatchCanvas = (action: LayoutAction) => {
    if (action.type === "select-leaf") { onSelectLeaf(action.tabId, action.leafId); return; }
    if (action.type === "zoom-toggle") {
      const id = action.leafId ?? ctx.getState().tabs[action.tabId]?.selectedLeafId;
      if (id) onSelectLeaf(action.tabId, id);
    }
    ctx.dispatch(action);
  };
  const renderLeaf = (hostTab: TabLayoutState, leaf: Leaf, rect: Rect) => {
    const active = hostTab.tabId === tabLayout?.tabId;
    const pane = snapshot?.panes.find(candidate => candidate.id === leaf.id && candidate.tab_id === hostTab.tabId);
    const selected = active && hostTab.selectedLeafId === leaf.id;
    const pending = selected && state.focusPending !== null;
    const focusError = selected ? state.focusError : null;
    return <LeafHost ctx={ctx} tab={hostTab} leaf={leaf} rect={rect} pane={pane} selected={selected}
      browserInputActive={browserInputActive && active && !modalOpen && state.sync === "live"} browserLiveInputEnabled={active && !modalOpen && state.sync === "live"} focusStatus={pending ? "pending" : focusError ? "error" : null} focusError={focusError?.message}
      closeDisabled={mutationBusy}
      onSelect={() => selectLeaf(leaf.id)}
      onZoom={() => { selectLeaf(leaf.id); zoom(leaf.id); }} onClose={() => { selectLeaf(leaf.id); closeLeaf(leaf.id); }} onRetryFocus={onRetry}
      onMenu={event => { selectLeaf(leaf.id); openContext(event, { kind: "pane", id: leaf.id }); }}
      terminal={pane ? {
        client, request: { session_id: state.sessionId!, pane_id: pane.id }, selected, presented: selected,
        controlAllowed: selected && !browserInputActive && !modalOpen && state.sync === "live" && snapshot?.focused_pane_id === pane.id && !state.focusPending && !state.focusError,
        controlPending: pending, focusEpoch: state.epoch, focusToken: state.focusToken, terminalMouseInput,
        deferAttachment: state.sync !== "live" && !(state.sync === "loading" && attachedPaneIds.current.has(pane.id)), focusOnAttach: active && !attachFocusSuppressed,
        onRequestControl: () => selectLeaf(pane.id), onSelect: () => selectLeaf(pane.id), onPrepared: () => {
          onPanePrepared(pane.id);
          if (active && switching && pane.id === hostTab.focusedPaneId) setPaintedTab(hostTab);
        },
        onResync: onReconnect, onClosed: onReconnect, onClosePane: () => closeLeaf(pane.id), registerStream: (stream, attached) => {
          registerStream(stream, attached);
          if (attached) attachedPaneIds.current.add(pane.id); else attachedPaneIds.current.delete(pane.id);
        },
      } : null} />;
  };
  const workbenchStyle: CSSProperties & { "--sidebar-width": string } = { "--sidebar-width": `${sidebarWidth}px` };
  const sidebarClass = "sidebar";
  // Selection chrome follows Herdr's acknowledgement: while a focus request is in flight the sidebar keeps the confirmed row and marks the target as pending.
  const pendingSpaceId = state.focusPending?.kind === "space" ? state.focusPending.target_id : null;
  const pendingPaneId = state.focusPending?.kind === "pane" || state.focusPending?.kind === "agent" ? state.focusPending.target_id : null;
  const sidebarSelectedSpaceId = state.focusPending ? snapshot?.focused_space_id ?? null : selection.spaceId;
  const sidebarSelectedPaneId = state.focusPending ? snapshot?.focused_pane_id ?? null : selection.paneId;
  return <div className={`workbench${sidebarCollapsed ? " sidebar-collapsed" : ""}${narrowViewport && drawerOpen ? " drawer-open" : ""}`} style={workbenchStyle}>
    <div className="workbench-underlay" inert={Boolean(popup)}>
    {narrowViewport && drawerOpen ? <button type="button" className="drawer-scrim" aria-label="Close sidebar" onClick={() => closeDrawer()} /> : null}
    <aside id="cockpit-sidebar" className={sidebarClass} aria-label="Spaces and agents" role={narrowViewport && drawerOpen ? "dialog" : undefined} aria-modal={narrowViewport && drawerOpen ? "true" : undefined} aria-hidden={narrowViewport && !drawerOpen ? "true" : undefined} hidden={narrowViewport ? !drawerOpen : sidebarCollapsed}>
      <Sidebar session={sidebarSession} sync={state.sync} narrow={narrowViewport} onSession={openSessionChooser} onClose={() => closeDrawer()} closeRef={sidebarCloseRef} hasSession={state.sessionId !== null} hasSnapshot={snapshot !== null}
        spaces={{ spaces, gitStatus: spaceGit, selectedSpaceId: sidebarSelectedSpaceId, pendingSpaceId, editingId: editing?.kind === "space" ? editing.id : null, busy: mutationBusy, notes: spaceNotesFromFailures(Object.values(mutations.errors)), onEdit: (id) => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "space", id } : null); }, onSelect: focusSpace, onContext: openContext, onSetup: () => setSetupOpen(true), setupEnabled: state.sync === "live" && !modalOpen, mutate: onMutate }}
        agents={{ agents: snapshot?.agents ?? [], spaces, tabs: allTabs, selectedPaneId: sidebarSelectedPaneId, pendingPaneId, onSelect: focusAgent }} />
    </aside>
    {!narrowViewport && !sidebarCollapsed ? <div className="sidebar-resizer" role="separator" tabIndex={sidebarCollapsed ? -1 : 0} aria-label="Resize sidebar" aria-orientation="vertical" aria-valuemin={SIDEBAR_MIN_WIDTH} aria-valuemax={SIDEBAR_MAX_WIDTH} aria-valuenow={sidebarWidth}
      onKeyDown={(event) => { if (sidebarCollapsed) return; if (event.key === "Home") { event.preventDefault(); updateSidebarWidth(SIDEBAR_DEFAULT_WIDTH); } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") { event.preventDefault(); updateSidebarWidth(sidebarWidth + (event.key === "ArrowLeft" ? -8 : 8)); } }}
      onPointerDown={(event) => { if (sidebarCollapsed || event.button !== 0) return; event.preventDefault(); const start = event.clientX; const width = sidebarWidth; const move = (next: PointerEvent) => updateSidebarWidth(width + next.clientX - start); const stop = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", stop); }; window.addEventListener("pointermove", move); window.addEventListener("pointerup", stop); }} /> : null}
    <main className="main-workarea">
      {!selection.spaceId ? <button type="button" className="drawer-toggle" aria-expanded={drawerOpen} aria-controls="cockpit-sidebar" aria-label="Open sidebar" onClick={narrowViewport ? openDrawer : toggleSidebarCollapsed}><UiIcon name="sidebar" /> <span>Sidebar</span></button> : null}
      {selection.spaceId ? <TabStrip sidebarOpen={narrowViewport ? drawerOpen : !sidebarCollapsed} onToggleSidebar={narrowViewport ? (drawerOpen ? () => closeDrawer() : openDrawer) : toggleSidebarCollapsed} tabs={tabs} selectedTabId={selection.tabId} editingId={editing?.kind === "tab" ? editing.id : null} busy={mutationBusy} browserOpen={browserOpen} browserDisabledReason={browserOpen ? null : browserReason} libraryOpen={libraryOpen} onEdit={id => { if (!mutationBusy && !modalOpen) setEditing(id ? { kind: "tab", id } : null); }} onSelect={focusTab} onContext={openContext} onCreate={() => { if (selection.spaceId) onMutate("tab:new", { type: "tab_create", space_id: selection.spaceId, label: null }, true); }} onBrowserToggle={toggleBrowser} onLibraryToggle={() => { if (libraryOpen) closeLibrary(); else openLibrary(); }} onCommands={() => setCommandsOpen(true)} mutate={onMutate} /> : null}
      <div className="workarea-content">
        {libraryOpen ? <LibraryView client={client} onClose={closeLibrary} onCaptureInvoker={invoker => { librarySidebarInvoker.current = invoker?.closest(".sidebar") ? invoker : null; }} command={libraryCommand} space={librarySpace} /> : <div ref={canvasRef} data-suppress-attach-focus={attachFocusSuppressed || undefined} style={{ position: "relative", flex: "1 1 0", minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column" }} onPointerDownCapture={() => setAttachFocusSuppressed(false)}
          onContextMenu={event => { const pane = (event.target as HTMLElement).closest<HTMLElement>("[data-leaf-id]"); if (pane?.dataset.leafId) { selectLeaf(pane.dataset.leafId); openContext(event, { kind: "pane", id: pane.dataset.leafId }); } }}>
          {canvasTabs.length ? canvasTabs.map(hostTab => <div key={hostTab.tabId} style={{ position: switching ? "absolute" : "relative", inset: switching ? 0 : undefined, flex: "1 1 0", minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column", visibility: switching && hostTab.tabId === tabLayout?.tabId ? "hidden" : "visible", pointerEvents: hostTab.tabId !== tabLayout?.tabId ? "none" : undefined }} inert={hostTab.tabId !== tabLayout?.tabId}><TabCanvas tab={hostTab} area={area} inputBlocked={Boolean(popup || popupPending)} renderLeaf={(leaf, rect) => renderLeaf(hostTab, leaf, rect)} dispatch={dispatchCanvas} registerTransient={registerTransient} announce={setPrefixHint} /></div>) : <div className="empty-main"><strong>No panes</strong><span>Create a tab or select another space.</span></div>}
        </div>}
      </div>
      <BrowserCleanupNotices ctx={ctx} activeTabId={selection.tabId} fallback={<ErrorSlot placement="pane" message={[lifecycleError, layoutError].filter(Boolean).join(" · ")} actions={lifecycleError || layoutError ? <>
        <button type="button" onClick={onReconnect}>Resync</button>
        <button type="button" onClick={() => { setLifecycleError(null); onDismissLayoutError(); }}>Dismiss</button>
      </> : null} />} />
    </main>
    {renderMenu()}
    {dialog ? <PaneDialogOverlay dialog={dialog} panes={panes} tabs={allTabs} spaces={spaces} busy={mutationBusy} onDismiss={() => setDialog(null)} mutate={onMutate} leafChoices={localLeaves.map(leaf => ({ id: leaf.id, title: byId(panes, leaf.id)?.title || leaf.kind }))} onSwap={swap} confirmMove={pane => { const message = lastTerminalMessage(pane, "Moving"); return !message || window.confirm(message); }} /> : null}
    {commandsOpen ? <CommandOverlay actions={commandActions.map((action) => ({ ...action, run: () => { setCommandsOpen(false); action.run(); } }))} statusContent={commandStatus} onSwitchSession={() => { setCommandsOpen(false); void onRefreshSessions().catch(() => undefined).finally(() => setSessionChooserOpen(true)); }} onDismiss={() => setCommandsOpen(false)} /> : null}
    {libraryAddOpen ? <AddContextDialog client={client} onClose={() => setLibraryAddOpen(false)} onOpenItem={(itemId) => openLibrary({ kind: "open", itemId })} space={librarySpace} /> : null}
    {sessionChooserOpen ? <SessionDialogOverlay sessions={sessions} currentSessionId={state.sessionId} onRefresh={onRefreshSessions} onSession={onSession} onDismiss={() => setSessionChooserOpen(false)} /> : null}
    {state.sessionId ? <SetupDialog client={client} sessionId={state.sessionId} open={setupOpen} selectedParent={setupParent} parentSpaceId={selection.spaceId} onClose={() => setSetupOpen(false)} onCompleted={onReconnect} /> : null}
    {state.sessionId ? <TeardownRecoveryPanel client={client} sessionId={state.sessionId} open={recoveryOpen} onClose={() => setRecoveryOpen(false)} /> : null}
    {state.sessionId && teardownSpaceId ? <TeardownDialog client={client} sessionId={state.sessionId} workspaceId={teardownSpaceId} open onClose={() => setTeardownSpaceId(null)} onCompleted={onReconnect} /> : null}
    {prefixActive ? <div className="prefix-indicator" role="status"><span>{armedPrefix.label} · {armedPrefix.origin === "herdr" ? "Herdr commands · Esc cancels" : armedPrefixHint()}</span></div> : commandNotice || commandFailureMessage ? <div className="prefix-indicator is-notice" role="alert">{commandNotice || commandFailureMessage}</div> : popupPending ? <div className="prefix-indicator" role="status">Opening Herdr popup…</div> : prefixHint ? <div className="prefix-indicator is-notice" role="status">{prefixHint}</div> : null}
    <RecoveryPanel state={state} mutations={mutations} onReconnect={onReconnect} onRetryMutation={onRetryMutation} />
    </div>
    {popup && state.sessionId ? <ServerPopup key={`${state.sessionId}:${popup.terminal_id}`} client={client} sessionId={state.sessionId} popup={popup} live={state.sync === "live" && shell?.status === "live"} error={shell?.error ?? state.syncError?.message ?? null} focusEpoch={state.epoch} terminalMouseInput={terminalMouseInput} onReconnect={onReconnect} /> : null}
  </div>;
}
export function App({ client }: { client: CockpitClient }) {
  const [status, setStatus] = useState<CockpitStatus | null>(null);
  const [statusError, setStatusError] = useState<StatusError | null>(null);
  const [statusAttempt, setStatusAttempt] = useState(0);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [sessionsAttempt, setSessionsAttempt] = useState(0);
  const [sessionsLoaded, setSessionsLoaded] = useState(false);
  const [sessionsError, setSessionsError] = useState<StatusError | null>(null);
  const [state, dispatch] = useReducer(sessionReducer, initialSessionState);
  const layouts = useTabLayouts(state.sessionId ?? "");
  const tabLayout = layouts.state.activeTabId ? layouts.state.tabs[layouts.state.activeTabId] ?? null : null;
  const selection: Selection = { spaceId: layouts.state.activeSpaceId, tabId: layouts.state.activeTabId, paneId: tabLayout?.selectedLeafId ?? null };
  const ctx: LeafCtx = { client, sessionId: state.sessionId ?? "", serverInstance: layouts.state.serverInstance, clientId: getViewerClientId(), getState: layouts.getState, dispatch: layouts.dispatch };
  const ctxRef = useRef(ctx);
  ctxRef.current = ctx;
  const transientCallbacks = useRef(new Set<() => void>());
  const registerTransient = useCallback((cancel: () => void) => { transientCallbacks.current.add(cancel); return () => { transientCallbacks.current.delete(cancel); }; }, []);
  const [announcement, setAnnouncement] = useState("");
  const [lifecycleError, setLifecycleError] = useState<string | null>(null);
  const echoAccess = useRef<{ get(): FocusEcho[]; consume(token: number): void; supersede(): void }>({ get: () => [], consume: () => undefined, supersede: () => undefined });
  const splitPlacement = useRef<Omit<PendingCreation, "token"> | null>(null);
  const sessionStream = useRef<{ close(): void } | null>(null);
  const [resyncAttempt, setResyncAttempt] = useState(0);
  const recoveryResyncRef = useRef(false);
  const sessionObservation = useRef(0);
  const sessionListRequest = useRef(0);
  const stateRef = useRef(state);
  stateRef.current = state;
  const drainLayoutEffects = useCallback(() => {
    const viewerTabs = new Set<string>();
    for (const effect of layouts.takeEffects()) {
      if (effect.type === "cancel-transient") { transientCallbacks.current.forEach(cancel => cancel()); if (effect.reason === "external-focus") echoAccess.current.supersede(); }
      if (effect.type === "consume-echo") echoAccess.current.consume(effect.token);
      if (effect.type === "announce") setAnnouncement(effect.text);
      if (effect.type === "viewer-release") viewerTabs.add(effect.tabId);
      if (effect.type === "browser-retire") void retireTabBrowser(ctxRef.current, effect.tabId, effect.associationKey, effect.serverInstance).catch(error => setLifecycleError(describeError(error, "Could not retire browser").message));
    }
    if (viewerTabs.size) void releaseViewers(ctxRef.current, [...viewerTabs]).catch(error => setLifecycleError(describeError(error, "Could not release viewers").message));
  }, [layouts.takeEffects]);
  const dispatchOrdered = useCallback((action: SessionAction) => {
    const previous = stateRef.current;
    const next = sessionReducer(previous, action);
    stateRef.current = next;
    dispatch(action);
    if (next.snapshot && next.snapshot !== previous.snapshot
      && (next.sync === "live" || action.type === "snapshot/authoritative")) {
      layouts.dispatch({ type: "snapshot", snapshot: next.snapshot, echoes: echoAccess.current.get(), sync: "live" });
      drainLayoutEffects();
    }
  }, [layouts.dispatch, drainLayoutEffects]);
  const autoResyncTimer = useRef<number | null>(null);
  const autoResyncAttempts = useRef(0);
  const healthyLiveTimer = useRef<number | null>(null);
  const mountedRef = useRef(true);
  const clearRecoveryTimers = useCallback(() => {
    if (autoResyncTimer.current !== null) window.clearTimeout(autoResyncTimer.current);
    if (healthyLiveTimer.current !== null) window.clearTimeout(healthyLiveTimer.current);
    autoResyncTimer.current = null;
    healthyLiveTimer.current = null;
  }, []);
  const requestResync = useCallback(() => {
    recoveryResyncRef.current = true;
    setResyncAttempt((value) => value + 1);
  }, []);
  const { focus, panePrepared, reconcile: reconcileFocus, reset: resetFocus, retryFocus, tokenRef: focusTokenRef, getEchoes, consumeEcho, supersedeSelection } = useFocusCoordinator({
    client, stateRef, mountedRef, dispatch: dispatchOrdered, describeError, onTimeout: requestResync,
    onIntent: echo => layouts.dispatch({ type: "focus/register", ...echo }),
  });
  echoAccess.current = { get: getEchoes, consume: consumeEcho, supersede: supersedeSelection };
  const focusAndSelect = useCallback((request: FocusRequest, location: Selection, prepare?: { paneId: string }) => {
    if (location.tabId) {
      layouts.dispatch({ type: "activate-tab", tabId: location.tabId });
      if ((request.kind === "pane" || request.kind === "agent") && location.paneId) layouts.dispatch({ type: "select-leaf", tabId: location.tabId, leafId: location.paneId });
    }
    focus(request, location, prepare);
  }, [focus, layouts.dispatch]);
  const selectLeaf = useCallback((tabId: string, leafId: string) => {
    const tab = layouts.getState().tabs[tabId];
    if (!tab) return;
    if (tab.selectedLeafId !== leafId) layouts.dispatch({ type: "select-leaf", tabId, leafId });
    if (tab.terminals[leafId]) {
      const pending = stateRef.current.focusPending;
      if (!pending || ((pending.kind !== "pane" && pending.kind !== "agent") || pending.target_id !== leafId)) focus({ kind: "pane", target_id: leafId }, { spaceId: tab.spaceId, tabId, paneId: leafId });
    } else {
      const pending = stateRef.current.focusPending;
      // A remembered viewer can take DOM focus while its tab is preparing.
      // Reasserting that existing selection must not cancel the tab request.
      if (tab.selectedLeafId !== leafId || pending?.kind !== "tab" || pending.target_id !== tabId) supersedeSelection();
      requestAnimationFrame(() => {
        const host = Array.from(document.querySelectorAll<HTMLElement>("[data-leaf-id]")).find(element => element.dataset.leafId === leafId);
        if (!host || host.contains(document.activeElement) || layouts.getState().tabs[tabId]?.selectedLeafId !== leafId) return;
        (host.querySelector<HTMLElement>('.context-document, .review-diff, .browser-surface, input, [tabindex="0"]') ?? host).focus({ preventScroll: true });
      });
    }
  }, [focus, supersedeSelection, layouts.dispatch, layouts.getState]);
  const priorSelection = useRef<{ tabId: string | null; leafId: string | null; terminal: boolean }>({ tabId: null, leafId: null, terminal: false });
  useLayoutEffect(() => {
    const terminal = Boolean(selection.paneId && tabLayout?.terminals[selection.paneId]);
    const prior = priorSelection.current;
    priorSelection.current = { tabId: selection.tabId, leafId: selection.paneId, terminal };
    if (selection.tabId === prior.tabId && selection.paneId !== prior.leafId) {
      if (!terminal) supersedeSelection();
      else if (!prior.terminal && selection.paneId && tabLayout) focus({ kind: "pane", target_id: selection.paneId }, { spaceId: tabLayout.spaceId, tabId: tabLayout.tabId, paneId: selection.paneId });
    }
  }, [selection.tabId, selection.paneId, tabLayout, focus, supersedeSelection]);
  const { mutate, reset: resetMutations, retry: retryMutation, state: mutations, tokenRef: mutationTokenRef } = useMutationCoordinator({
    client, stateRef, mountedRef, sessionObservationRef: sessionObservation, dispatchSession: dispatchOrdered,
    describeError, mutationSnapshot: authoritativeMutationSnapshot, onResync: requestResync,
    onBegin: operation => {
      if (operation.request.type !== "pane_split") return;
      const placement = splitPlacement.current;
      splitPlacement.current = null;
      if (placement) layouts.dispatch({ type: "creation/begin", creation: { ...placement, token: operation.token } });
    },
    onSettled: (operation, created, snapshot, responseIsCurrent) => {
      if (operation.epoch !== stateRef.current.epoch || operation.request.type !== "pane_split") return;
      if (snapshot && responseIsCurrent) layouts.dispatch({ type: "snapshot", snapshot, echoes: getEchoes(), sync: "live" });
      layouts.dispatch({ type: "creation/settled", token: operation.token, created });
      drainLayoutEffects();
    },
  });
  const split = useCallback((tabId: string, leafId: string, direction: "right" | "down") => {
    const tab = layouts.getState().tabs[tabId];
    const sourcePaneId = tab ? runtimeSource(tab, leafId, stateRef.current.snapshot?.focused_pane_id) : null;
    if (!tab || !sourcePaneId || stateRef.current.sync !== "live") return;
    splitPlacement.current = { tabId, placeBeside: leafId, dir: direction === "right" ? "row" : "col", sourcePaneId };
    if (!mutate(`pane:${sourcePaneId}`, { type: "pane_split", pane_id: sourcePaneId, direction, ratio: null }, true)) splitPlacement.current = null;
  }, [layouts.getState, mutate]);
  useLayoutEffect(drainLayoutEffects, [layouts.state, drainLayoutEffects]);
  const resetSessionRuntime = useCallback(() => {
    sessionObservation.current += 1;
    sessionStream.current?.close();
    sessionStream.current = null;
    resetFocus();
    clearRecoveryTimers();
    autoResyncAttempts.current = 0;
    recoveryResyncRef.current = false;
    resetMutations();
  }, [clearRecoveryTimers, resetFocus, resetMutations]);
  const switchSession = useCallback((id: string) => {
    resetSessionRuntime();
    const outgoing = ctxRef.current;
    void releaseViewers(outgoing, "all").catch(error => setLifecycleError(describeError(error, "Could not release viewers").message));
    dispatchOrdered({ type: "switch", sessionId: id });
  }, [resetSessionRuntime, dispatchOrdered]);
  const refreshSessions = useCallback(async () => {
    const request = ++sessionListRequest.current;
    setSessionsError(null);
    try {
      const response = await client.sessions();
      if (!mountedRef.current || sessionListRequest.current !== request) return;
      setSessions(response.sessions);
      setSessionsLoaded(true);
    } catch (error: unknown) {
      if (!mountedRef.current || sessionListRequest.current !== request) return;
      setSessionsError(describeError(error, "Could not list Herdr sessions"));
      setSessionsLoaded(true);
    }
  }, [client]);
  useEffect(() => { mountedRef.current = true; return () => { mountedRef.current = false; clearRecoveryTimers(); resetFocus(); }; }, [clearRecoveryTimers, resetFocus]);
  useEffect(() => { let active = true; setStatus(null); setStatusError(null); void client.status().then((next) => { if (active) setStatus(next); }, (error: unknown) => { if (active) setStatusError(describeError(error, "Could not read Cockpit status")); }); return () => { active = false; }; }, [client, statusAttempt]);
  const compatible = status?.herdr.status === "compatible";
  const sessionAvailable = sessionsLoaded && sessions.some((session) => session.id === state.sessionId);
  useEffect(() => {
    if (!compatible) return;
    void refreshSessions();
    return () => { sessionListRequest.current += 1; };
  }, [refreshSessions, compatible, statusAttempt, sessionsAttempt]);
  useEffect(() => {
    if (!compatible || !sessionsLoaded) return;
    if (sessions.length === 0) {
      resetSessionRuntime();
      return;
    }
    if (!state.sessionId || !sessions.some((session) => session.id === state.sessionId)) {
      const preferred = sessions.find((session) => session.is_default) ?? sessions[0];
      switchSession(preferred.id);
    }
  }, [compatible, sessionsLoaded, sessions, state.sessionId, resetSessionRuntime, switchSession]);
  useEffect(() => {
    const sessionId = state.sessionId;
    if (!compatible || !sessionId || !sessionAvailable) return;
    const epoch = state.epoch;
    const observation = ++sessionObservation.current;
    const recovering = recoveryResyncRef.current;
    const recoveryFocusToken = focusTokenRef.current;
    const recoveryMutationToken = mutationTokenRef.current;
    const controller = new AbortController();
    let active = true;
    sessionStream.current?.close();
    sessionStream.current = null;
    dispatchOrdered({ type: "snapshot/request", epoch, sessionId });
    void (async () => {
      try {
        const snapshot = await client.sessionSnapshot(sessionId, controller.signal);
        if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
        dispatchOrdered({ type: "snapshot/received", epoch, sessionId, snapshot });
        const stream = await client.subscribeSession(sessionId, (message: SessionStreamMessage) => {
          if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
          if (recovering && message.type === "snapshot" && message.sequence === 1 && recoveryFocusToken === focusTokenRef.current && stateRef.current.focusError) {
            // Reissue the coordinator's retained intent. The bootstrap snapshot
            // is a stale observation and must not become a new user request.
            retryFocus();
          }
          dispatchOrdered({ type: "stream/message", epoch, sessionId, message });
          if (message.type !== "snapshot" || message.sequence !== 1) return;
          if (recovering && recoveryMutationToken === mutationTokenRef.current) {
            recoveryResyncRef.current = false;
          }
        }, (error: unknown) => {
          if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
          const described = describeError(error, "Session stream disconnected");
          dispatchOrdered({ type: "stream/error", epoch, sessionId, code: described.code ?? "stream_disconnected", message: described.message });
        }, controller.signal);
        if (active && !controller.signal.aborted && sessionObservation.current === observation) sessionStream.current = stream; else stream.close();
      } catch (error: unknown) {
        if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
        const described = describeError(error, "Could not read the session snapshot");
        dispatchOrdered({ type: "stream/error", epoch, sessionId, code: described.code ?? "snapshot_error", message: described.message });
      }
    })();
    return () => {
      active = false;
      controller.abort();
      sessionStream.current?.close();
      sessionStream.current = null;
    };
  }, [client, compatible, sessionAvailable, state.sessionId, state.epoch, resyncAttempt]);
  useEffect(() => {
    if (state.sync !== "live") {
      if (healthyLiveTimer.current !== null) {
        window.clearTimeout(healthyLiveTimer.current);
        healthyLiveTimer.current = null;
      }
      return;
    }
    if (healthyLiveTimer.current !== null) return;
    const epoch = state.epoch;
    const generation = state.generation;
    // A stream snapshot is only a bootstrap boundary; require one second of
    // ordered live traffic so an immediate snapshot/disconnect cannot renew
    // the outage budget indefinitely.
    healthyLiveTimer.current = window.setTimeout(() => {
      healthyLiveTimer.current = null;
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.generation !== generation || stateRef.current.sync !== "live") return;
      autoResyncAttempts.current = 0;
    }, 1000);
    return () => {
      if (healthyLiveTimer.current !== null) {
        window.clearTimeout(healthyLiveTimer.current);
        healthyLiveTimer.current = null;
      }
    };
  }, [state.sync, state.epoch, state.generation]);
  useEffect(() => {
    if (state.sync !== "stale" && state.sync !== "disconnected") return;
    if (autoResyncAttempts.current >= 3 || autoResyncTimer.current !== null) return;
    const epoch = state.epoch;
    const sessionId = state.sessionId;
    const attempt = autoResyncAttempts.current;
    autoResyncTimer.current = window.setTimeout(() => {
      autoResyncTimer.current = null;
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId
        || (stateRef.current.sync !== "stale" && stateRef.current.sync !== "disconnected")) return;
      autoResyncAttempts.current += 1;
      setResyncAttempt((value) => value + 1);
    }, [250, 500, 1000][attempt] ?? 1000);
    return () => {
      if (autoResyncTimer.current !== null) {
        window.clearTimeout(autoResyncTimer.current);
        autoResyncTimer.current = null;
      }
    };
  }, [state.sync, state.epoch]);
  useEffect(() => {
    reconcileFocus(state, selection, selection.paneId && tabLayout?.terminals[selection.paneId] ? selection.paneId : null);
  }, [state.snapshot, state.sync, state.epoch, state.focusPending, state.focusToken, state.focusError, selection.paneId, tabLayout, reconcileFocus]);

  // With no session the Library opens full-screen in place of the notice screens.
  const [noSessionLibraryOpen, setNoSessionLibraryOpen] = useState(false);
  const returnToLibraryOpener = useRef(false);
  useEffect(() => {
    if (noSessionLibraryOpen || !returnToLibraryOpener.current) return;
    returnToLibraryOpener.current = false;
    document.querySelector<HTMLElement>("[data-library-opener]")?.focus({ preventScroll: true });
  }, [noSessionLibraryOpen]);
  const openNoSessionLibrary = () => setNoSessionLibraryOpen(true);
  const noSessionLibrary = noSessionLibraryOpen
    ? <div className="app-shell"><LibraryView client={client} fullScreen onClose={() => { returnToLibraryOpener.current = true; setNoSessionLibraryOpen(false); }} /></div>
    : null;
  const explicitResync = () => {
    clearRecoveryTimers();
    autoResyncAttempts.current = 0;
    void refreshSessions();
    requestResync();
  };
  if ((!status || !compatible) && (statusError || status) && noSessionLibrary) return noSessionLibrary;
  if (!status || !compatible) return <div className="app-shell">{statusError || (status && !compatible) ? <CompatibilityNotice status={status} error={statusError} retry={() => setStatusAttempt((value) => value + 1)} onOpenLibrary={openNoSessionLibrary} /> : <main className="compatibility-main" aria-live="polite"><section className="notice notice-loading" role="status"><p className="eyebrow">Cockpit</p><h1>Connecting to Herdr</h1><p>Reading compatibility status...</p></section></main>}</div>;
  if (((sessionsError && sessions.length === 0) || (sessionsLoaded && sessions.length === 0)) && noSessionLibrary) return noSessionLibrary;
  if (sessionsError && sessions.length === 0) return <div className="app-shell"><CompatibilityNotice status={status} error={sessionsError} retry={() => setSessionsAttempt((value) => value + 1)} onOpenLibrary={openNoSessionLibrary} /></div>;
  if (sessionsLoaded && sessions.length === 0) return <div className="app-shell"><main className="compatibility-main"><section className="notice"><h1>No Herdr sessions</h1><p>Create or start a session, then refresh the list.</p><div className="notice-actions"><button type="button" className="action-button" onClick={() => setSessionsAttempt((value) => value + 1)}>Refresh sessions</button><OpenLibraryButton onOpen={openNoSessionLibrary} /></div></section></main></div>;
  return <div className="app-shell"><div className="sr-only" role="status" aria-live="polite">{announcement}</div><Workbench key={state.epoch} client={client} state={state} sessions={sessions} selection={selection} terminalMouseInput={status.capabilities.terminal_mouse_input} mutations={mutations} ctx={ctx} tabLayout={tabLayout} registerTransient={registerTransient} onSession={switchSession} onFocus={focusAndSelect} onSelectLeaf={selectLeaf} onSplit={split} onPanePrepared={panePrepared} onReconnect={explicitResync} onRetry={retryFocus} onRefreshSessions={refreshSessions} onOpenSession={() => { void refreshSessions(); }} onMutate={mutate} onRetryMutation={retryMutation} layoutError={lifecycleError} onDismissLayoutError={() => setLifecycleError(null)} /></div>;
}
