import { herdrShadowsPrefix, herdrShadowsChord } from "./herdrBindings";

/**
 * The keyboard shortcut registry: the single source of truth for every Cockpit
 * shortcut. The prefix router (`keymap.ts`), the Commands list, tooltips
 * (`formatShortcut`) and `docs/keyboard-shortcuts.md` (`renderShortcutDocs`) all
 * derive from `SHORTCUTS`; add or change a binding here and nowhere else.
 * Design: `planning/ui-polish-2026-09-28/02-keyboard-shortcuts.md`.
 *
 * Sidebar focus API (`Ctrl+B w` / `Ctrl+B a`). The sidebar component owns its
 * rows, so it tells the router how to focus them:
 *
 *   useEffect(() => registerSidebarFocus({
 *     spaces: () => void,   // focus the selected Space row, else the first row
 *     agents: () => void,   // focus the selected Agent row, else the first row
 *   }), [...]);             // `registerSidebarFocus` returns its own unregister
 *
 * Handlers only move DOM focus and never send a Herdr request. The workbench
 * expands the sidebar (or opens the narrow drawer) before it calls
 * `focusSidebarList`, so the rows exist when a handler runs. `Esc` inside the
 * sidebar should call `returnFocusFromSidebar()`, which restores the element
 * that held focus before `focusSidebarList` moved it, else the selected tab.
 */

export type Platform = "mac" | "other";

export type ShortcutGroup = "Navigate" | "Space" | "Tab" | "Pane" | "Browser" | "Library";
export type ShortcutScope = "global" | "terminal" | "browser" | "viewer" | "line";

export type PrefixCommand =
  | "help" | "new-space" | "rename-space" | "close-space" | "setup-space"
  | "new-tab" | "rename-tab" | "previous-tab" | "next-tab" | "close-tab"
  | "rename-pane" | "split-right" | "split-down" | "close-pane" | "zoom-pane" | "resize"
  | "previous-pane" | "next-pane" | "focus-left" | "focus-right" | "focus-up" | "focus-down"
  | "swap-left" | "swap-right" | "swap-up" | "swap-down"
  | "toggle-sidebar" | "focus-spaces" | "focus-agents" | "switch-session"
  | "toggle-library" | "toggle-browser" | "open-file-picker"
  | `select-tab-${1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9}`;

/** Actions that have a local chord but no prefix key. */
export type LocalShortcutId =
  | "focus-file-tree" | "focus-file-content" | "toggle-preview" | "toggle-wrap" | "reload-listing"
  | "review-previous-file" | "review-next-file" | "review-previous-hunk" | "review-next-hunk"
  | "comment-lines" | "comment-file"
  | "terminal-copy" | "terminal-paste" | "terminal-newline"
  | "browser-delete-annotation"
  | "literal-prefix" | "cancel-prefix" | "commands-bare";

export type ShortcutId = PrefixCommand | LocalShortcutId;

/** The key after `Ctrl+B`. `shift: "any"` ignores Shift (`?` needs it on most layouts). */
export type PrefixKey = { key: string; shift: boolean | "any"; display: string };

/**
 * A local chord. `mod` is Ctrl or Cmd; either satisfies it in viewer scope.
 * `code` matches `KeyboardEvent.code` (layout- and Option-proof, used for Alt chords);
 * `key` matches `KeyboardEvent.key` case-insensitively (Ctrl/Cmd letters follow the layout). A chord with neither is documentation only.
 * `shift` undefined ignores Shift.
 */
export type Chord = {
  mod?: boolean; ctrl?: boolean; alt?: boolean; shift?: boolean;
  code?: string; key?: string;
  display: string;
  /** Full text when the chord is not `Mod+Alt+Shift+display` (`Cmd+C`). */
  text?: string;
  platform?: Platform;
};

export type ShortcutEntry = {
  id: ShortcutId;
  label: string;
  group: ShortcutGroup;
  scope: ShortcutScope;
  prefix?: PrefixKey;
  chords?: readonly Chord[];
  /** Listed as a row in Commands. False for keys that only exist for tooltips and docs. */
  palette?: boolean;
  /** Prefix command that needs a selected Space / tab / pane. */
  needs?: "space" | "tab" | "pane";
  /** Acts on the selected pane, so the Library view is closed first (design §6.3). */
  paneScoped?: boolean;
  note?: string;
};

const k = (key: string, shift: boolean | "any" = false, display = shift === true ? `Shift+${key.toUpperCase()}` : key): PrefixKey => ({ key, shift, display });

const SELECT_TABS: ShortcutEntry[] = ([1, 2, 3, 4, 5, 6, 7, 8, 9] as const).map((number) => ({
  id: `select-tab-${number}` as const, label: `Select tab ${number}`, group: "Tab" as const, scope: "global" as const,
  prefix: k(String(number)), palette: false, needs: "tab" as const,
}));

export const SHORTCUTS: readonly ShortcutEntry[] = [
  { id: "help", label: "Commands", group: "Navigate", scope: "global", prefix: k("?", "any"), palette: false, note: "A bare ? outside text fields also opens Commands." },
  ...SELECT_TABS,
  { id: "previous-tab", label: "Previous tab", group: "Tab", scope: "global", prefix: k("p"), needs: "tab" },
  { id: "next-tab", label: "Next tab", group: "Tab", scope: "global", prefix: k("n"), needs: "tab" },
  { id: "new-tab", label: "New tab", group: "Tab", scope: "global", prefix: k("c") },
  { id: "rename-tab", label: "Rename tab", group: "Tab", scope: "global", prefix: k("t", true), needs: "tab" },
  { id: "close-tab", label: "Close tab", group: "Tab", scope: "global", prefix: k("x", true), needs: "tab", note: "Asks for confirmation." },
  { id: "new-space", label: "New Space", group: "Space", scope: "global", prefix: k("n", true), note: "Creates a bare Herdr workspace." },
  { id: "rename-space", label: "Rename Space", group: "Space", scope: "global", prefix: k("w", true), needs: "space" },
  { id: "close-space", label: "Close Space", group: "Space", scope: "global", prefix: k("d", true), needs: "space", note: "Asks for confirmation." },
  { id: "setup-space", label: "Set up a Space", group: "Navigate", scope: "global", prefix: k("s", true), note: "Opens the task Space setup dialog, like the Spaces + button." },
  { id: "rename-pane", label: "Rename terminal", group: "Pane", scope: "global", prefix: k("p", true), needs: "pane", paneScoped: true, note: "Real terminals only; viewers have fixed titles." },
  { id: "split-right", label: "New terminal beside pane", group: "Pane", scope: "global", prefix: k("v"), palette: false, needs: "pane", paneScoped: true, note: "Creates a terminal beside the selected terminal or viewer." },
  { id: "split-down", label: "New terminal below pane", group: "Pane", scope: "global", prefix: k("-"), palette: false, needs: "pane", paneScoped: true, note: "Creates a terminal below the selected terminal or viewer." },
  { id: "close-pane", label: "Close pane", group: "Pane", scope: "global", prefix: k("x"), needs: "pane", paneScoped: true, note: "Terminals ask for confirmation; Files and Review close locally. Browser close stops its session and removes its managed profile." },
  { id: "zoom-pane", label: "Toggle local pane zoom", group: "Pane", scope: "global", prefix: k("z"), needs: "pane", paneScoped: true, note: "Zooms the selected terminal or viewer in Cockpit only." },
  { id: "resize", label: "Focus adjacent pane divider", group: "Navigate", scope: "global", prefix: k("r"), needs: "pane", paneScoped: true, note: "Arrow keys resize by 24 px, Shift+arrow by 96 px; Esc returns focus to the selected pane." },
  { id: "focus-left", label: "Focus pane left", group: "Navigate", scope: "global", prefix: k("h"), needs: "pane", note: "Uses local geometry across terminals and viewers; restores zoom if the target is hidden." },
  { id: "focus-down", label: "Focus pane below", group: "Navigate", scope: "global", prefix: k("j"), needs: "pane", note: "Uses local geometry across terminals and viewers; restores zoom if the target is hidden." },
  { id: "focus-up", label: "Focus pane above", group: "Navigate", scope: "global", prefix: k("k"), needs: "pane", note: "Uses local geometry across terminals and viewers; restores zoom if the target is hidden." },
  { id: "focus-right", label: "Focus pane right", group: "Navigate", scope: "global", prefix: k("l"), needs: "pane", note: "Uses local geometry across terminals and viewers; restores zoom if the target is hidden." },
  { id: "swap-left", label: "Swap pane left", group: "Pane", scope: "global", prefix: k("h", true), needs: "pane", paneScoped: true, note: "Swaps locally with a terminal or viewer; Herdr's layout is unchanged." },
  { id: "swap-down", label: "Swap pane below", group: "Pane", scope: "global", prefix: k("j", true), needs: "pane", paneScoped: true, note: "Swaps locally with a terminal or viewer; Herdr's layout is unchanged." },
  { id: "swap-up", label: "Swap pane above", group: "Pane", scope: "global", prefix: k("k", true), needs: "pane", paneScoped: true, note: "Swaps locally with a terminal or viewer; Herdr's layout is unchanged." },
  { id: "swap-right", label: "Swap pane right", group: "Pane", scope: "global", prefix: k("l", true), needs: "pane", paneScoped: true, note: "Swaps locally with a terminal or viewer; Herdr's layout is unchanged." },
  { id: "next-pane", label: "Next pane", group: "Navigate", scope: "global", prefix: { key: "Tab", shift: false, display: "Tab" }, needs: "pane", note: "Cycles all terminals and viewers in visual tree order, restoring zoom when needed. Only as the key right after Ctrl+B; a plain Tab reaches the focused surface." },
  { id: "previous-pane", label: "Previous pane", group: "Navigate", scope: "global", prefix: { key: "Tab", shift: true, display: "Shift+Tab" }, needs: "pane", note: "Cycles all terminals and viewers in reverse visual tree order, restoring zoom when needed. Ctrl+B, Shift, Tab works too." },
  { id: "toggle-sidebar", label: "Toggle sidebar", group: "Navigate", scope: "global", prefix: k("b"), note: "Collapses or expands the sidebar; opens or closes the drawer on narrow windows." },
  { id: "focus-spaces", label: "Focus Spaces list", group: "Navigate", scope: "global", prefix: k("w"), note: "Then ↑ ↓ move, Enter selects through Herdr, Esc returns." },
  { id: "focus-agents", label: "Focus Agents list", group: "Navigate", scope: "global", prefix: k("a") },
  { id: "switch-session", label: "Switch session…", group: "Navigate", scope: "global", prefix: k("g") },
  { id: "toggle-library", label: "Open Library", group: "Library", scope: "global", prefix: k("i"), note: "Closing with Ctrl+B i returns focus to the pane it was opened from." },
  { id: "toggle-browser", label: "Toggle browser for tab", group: "Browser", scope: "global", prefix: k("b", true), palette: false, note: "Opens or closes the selected tab's browser, like the tab-strip button." },
  {
    id: "open-file-picker", label: "Open file picker", group: "Navigate", scope: "viewer", prefix: k("f"),
    chords: [{ mod: true, key: "p", display: "P" }, { key: "/", display: "/" }],
    note: "Acts only when focus is inside a Files, Review, Context or Library viewer.",
  },
  { id: "focus-file-tree", label: "Focus file tree", group: "Navigate", scope: "viewer", palette: false, chords: [{ alt: true, code: "Digit1", display: "1" }] },
  { id: "focus-file-content", label: "Focus file content", group: "Navigate", scope: "viewer", palette: false, chords: [{ alt: true, code: "Digit2", display: "2" }] },
  { id: "toggle-preview", label: "Switch Preview / Source", group: "Navigate", scope: "viewer", palette: false, chords: [{ alt: true, code: "KeyM", display: "M" }], note: "Markdown and HTML documents; Diff / Full source in Review." },
  { id: "toggle-wrap", label: "Toggle line wrap", group: "Navigate", scope: "viewer", palette: false, chords: [{ alt: true, code: "KeyZ", display: "Z" }] },
  { id: "reload-listing", label: "Reload listing", group: "Navigate", scope: "viewer", palette: false, chords: [{ mod: true, shift: false, key: "r", display: "R" }], note: "Rereads local files only; Review refreshes its comparison." },
  { id: "review-previous-file", label: "Previous file", group: "Navigate", scope: "line", palette: false, chords: [{ alt: true, key: "ArrowLeft", display: "Left" }] },
  { id: "review-next-file", label: "Next file", group: "Navigate", scope: "line", palette: false, chords: [{ alt: true, key: "ArrowRight", display: "Right" }] },
  { id: "review-previous-hunk", label: "Previous hunk", group: "Navigate", scope: "line", palette: false, chords: [{ alt: true, key: "ArrowUp", display: "Up" }] },
  { id: "review-next-hunk", label: "Next hunk", group: "Navigate", scope: "line", palette: false, chords: [{ alt: true, key: "ArrowDown", display: "Down" }] },
  { id: "comment-lines", label: "Comment on selected lines", group: "Navigate", scope: "line", palette: false, chords: [{ display: "C" }], note: "With a source or diff line focused." },
  { id: "comment-file", label: "Comment on the whole file", group: "Navigate", scope: "line", palette: false, chords: [{ shift: true, display: "C" }] },
  { id: "terminal-copy", label: "Copy terminal selection", group: "Pane", scope: "terminal", palette: false, chords: [{ mod: true, shift: true, display: "C" }, { display: "C", text: "Cmd+C", platform: "mac" }], note: "Sends no byte to the program, with or without a selection." },
  { id: "terminal-paste", label: "Paste into terminal", group: "Pane", scope: "terminal", palette: false, chords: [{ mod: true, shift: true, display: "V" }, { display: "V", text: "Cmd+V", platform: "mac" }] },
  { id: "terminal-newline", label: "Newline without submitting", group: "Pane", scope: "terminal", palette: false, chords: [{ shift: true, display: "Enter" }] },
  { id: "browser-delete-annotation", label: "Delete selected annotation", group: "Browser", scope: "browser", palette: false, chords: [{ display: "Delete" }] },
  { id: "literal-prefix", label: "Send a literal Ctrl+B to the terminal or page", group: "Navigate", scope: "global", palette: false, chords: [{ ctrl: true, display: "B" }], note: "Press Ctrl+B twice." },
  { id: "cancel-prefix", label: "Cancel an armed prefix", group: "Navigate", scope: "global", palette: false, chords: [{ display: "Esc" }], note: "The prefix has no timeout." },
  { id: "commands-bare", label: "Commands", group: "Navigate", scope: "global", palette: false, chords: [{ display: "?" }], note: "Outside text fields, terminals and the browser." },
];

const BY_ID = new Map<ShortcutId, ShortcutEntry>(SHORTCUTS.map((entry) => [entry.id, entry]));

export function shortcutEntry(id: ShortcutId): ShortcutEntry {
  const entry = BY_ID.get(id);
  if (!entry) throw new Error(`Unknown shortcut ${id}`);
  return entry;
}

export function detectPlatform(): Platform {
  const nav = typeof navigator === "undefined" ? undefined : (navigator as Navigator & { userAgentData?: { platform?: string } });
  return /mac|iphone|ipad/i.test(nav?.userAgentData?.platform ?? nav?.platform ?? "") ? "mac" : "other";
}

export const SHORTCUT_SEPARATOR = " or ";

export function formatChord(chord: Chord, platform: Platform = detectPlatform()): string {
  if (chord.text) return chord.text;
  const parts: string[] = [];
  if (chord.mod) parts.push(platform === "mac" ? "Cmd" : "Ctrl");
  if (chord.ctrl) parts.push("Ctrl");
  if (chord.alt) parts.push(platform === "mac" ? "Option" : "Alt");
  if (chord.shift) parts.push("Shift");
  parts.push(chord.display);
  return parts.join("+");
}

/** Every way to trigger `id`, prefix form first, e.g. `["Ctrl+B f", "Ctrl+P", "/"]`. */
export function shortcutForms(id: ShortcutId, platform: Platform = detectPlatform()): string[] {
  const entry = shortcutEntry(id);
  const forms: string[] = [];
  if (entry.prefix && !herdrShadowsPrefix(entry.prefix.key, entry.prefix.shift)) forms.push(`Ctrl+B ${entry.prefix.display}`);
  for (const chord of entry.chords ?? []) {
    if (chord.platform && chord.platform !== platform) continue;
    const key = chord.key ?? chord.code?.replace(/^(Key|Digit)/, "") ?? chord.display;
    if (herdrShadowsChord({ key, code: chord.code, ctrlKey: Boolean(chord.ctrl || (chord.mod && platform !== "mac")), metaKey: Boolean(chord.mod && platform === "mac"), altKey: Boolean(chord.alt), shiftKey: Boolean(chord.shift) })) continue;
    forms.push(formatChord(chord, platform));
  }
  return forms;
}

/**
 * Text for tooltips and Commands rows, e.g. `Ctrl+B Shift+N`, `Cmd+P`. Returns
 * every form joined with " or " unless `only` picks the prefix or chord form.
 */
export function formatShortcut(id: ShortcutId, platform: Platform = detectPlatform(), only?: "prefix" | "chord"): string {
  const entry = shortcutEntry(id);
  const forms = shortcutForms(id, platform);
  const prefixCount = entry.prefix && !herdrShadowsPrefix(entry.prefix.key, entry.prefix.shift) ? 1 : 0;
  const picked = only === "prefix" ? forms.slice(0, prefixCount) : only === "chord" ? forms.slice(prefixCount) : forms;
  return picked.join(SHORTCUT_SEPARATOR);
}

/** `label (Ctrl+B x)`, the form every hinted control uses. */
export function withShortcut(label: string, id: ShortcutId, platform?: Platform, only?: "prefix" | "chord"): string {
  const text = formatShortcut(id, platform, only);
  return text ? `${label} (${text})` : label;
}

/** For `aria-keyshortcuts`: single chords only, a prefix sequence cannot be expressed. */
export function ariaKeyShortcuts(id: ShortcutId, platform: Platform = detectPlatform()): string | undefined {
  const chords = (shortcutEntry(id).chords ?? []).filter((chord) => {
    if ((chord.platform && chord.platform !== platform) || (!chord.code && !chord.key)) return false;
    const key = chord.key ?? chord.code?.replace(/^(Key|Digit)/, "") ?? chord.display;
    return !herdrShadowsChord({ key, code: chord.code, ctrlKey: Boolean(chord.ctrl || (chord.mod && platform !== "mac")), metaKey: Boolean(chord.mod && platform === "mac"), altKey: Boolean(chord.alt), shiftKey: Boolean(chord.shift) });
  });
  if (chords.length === 0) return undefined;
  return chords.map((chord) => {
    const parts: string[] = [];
    if (chord.mod) parts.push(platform === "mac" ? "Meta" : "Control");
    if (chord.ctrl) parts.push("Control");
    if (chord.alt) parts.push("Alt");
    if (chord.shift) parts.push("Shift");
    parts.push(chord.code?.replace(/^(Key|Digit)/, "") ?? chord.key!);
    return parts.join("+");
  }).join(" ");
}

function matchesPrefixKey(spec: PrefixKey, key: string, shiftKey: boolean): boolean {
  if (spec.shift !== "any" && spec.shift !== shiftKey) return false;
  return key.toLowerCase() === spec.key.toLowerCase();
}

const PREFIX_ENTRIES = SHORTCUTS.filter((entry): entry is ShortcutEntry & { id: PrefixCommand; prefix: PrefixKey } => entry.prefix !== undefined);

export function prefixCommandForKey(key: string, shiftKey: boolean): PrefixCommand | null {
  return PREFIX_ENTRIES.find((entry) => matchesPrefixKey(entry.prefix, key, shiftKey))?.id ?? null;
}

/** `Shift+Q`, `Tab`, `Space`: how a key typed after the prefix is shown in the "not bound" hint. */
export function describePrefixKey(key: string, shiftKey: boolean): string {
  const name = key === " " ? "Space" : key;
  return shiftKey && /^[a-z]$/i.test(name) ? `Shift+${name.toUpperCase()}` : shiftKey && name.length > 1 ? `Shift+${name}` : name;
}

export function unboundPrefixMessage(key: string, shiftKey: boolean): string {
  return `Ctrl+B ${describePrefixKey(key, shiftKey)} is not bound in Cockpit`;
}

/** The chip text while the prefix is armed, built from the same keys as the table above. */
export function armedPrefixHint(): string {
  const hints: Array<[PrefixCommand, string]> = [["help", "commands"], ["new-tab", "tab"], ["split-right", "split"], ["split-down", "split"], ["focus-spaces", "spaces"], ["toggle-library", "library"]];
  return hints.flatMap(([id, label]) => { const prefix = shortcutEntry(id).prefix!; return herdrShadowsPrefix(prefix.key, prefix.shift) ? [] : [`${prefix.display} ${label}`]; }).join(" · ");
}

export type ShortcutKeyEvent = Pick<KeyboardEvent, "key" | "code" | "shiftKey" | "ctrlKey" | "altKey" | "metaKey"> & { target?: EventTarget | null };

export function chordMatches(chord: Chord, event: ShortcutKeyEvent): boolean {
  if (!chord.code && !chord.key) return false;
  if (chord.mod ? !(event.ctrlKey || event.metaKey) : Boolean(chord.ctrl) !== event.ctrlKey || event.metaKey) return false;
  if (Boolean(chord.alt) !== event.altKey) return false;
  if (chord.shift !== undefined && chord.shift !== event.shiftKey) return false;
  return chord.code ? event.code === chord.code : event.key.toLowerCase() === chord.key!.toLowerCase();
}

const VIEWER_ENTRIES = SHORTCUTS.filter((entry) => entry.scope === "viewer" && entry.chords?.some((chord) => chord.code || chord.key));

/**
 * The viewer-scope action a keydown asks for, or null. Viewers call this from
 * their key handlers, after their own editable-target check. Keys inside a
 * modal dialog belong to that dialog.
 */
export function viewerShortcutAction(event: ShortcutKeyEvent): ShortcutId | null {
  const target = typeof HTMLElement !== "undefined" && event.target instanceof HTMLElement ? event.target : null;
  if (target?.closest('dialog[open], [role="dialog"][aria-modal="true"]')) return null;
  return VIEWER_ENTRIES.find((entry) => entry.chords!.some((chord) => chordMatches(chord, event)))?.id ?? null;
}

// ---- Sidebar focus registration ------------------------------------------------

export type SidebarFocusHandlers = { spaces: () => void; agents: () => void };

let sidebarFocusHandlers: SidebarFocusHandlers | null = null;
let sidebarOrigin: HTMLElement | null = null;

/** Register the sidebar's list-focus handlers. Returns the unregister function. */
export function registerSidebarFocus(handlers: SidebarFocusHandlers): () => void {
  sidebarFocusHandlers = handlers;
  return () => { if (sidebarFocusHandlers === handlers) sidebarFocusHandlers = null; };
}

const insideSidebar = (element: Element | null): boolean => Boolean(element?.closest("#cockpit-sidebar"));

/** Focus a sidebar list through its registered handler. False when none is registered. */
export function focusSidebarList(list: "spaces" | "agents"): boolean {
  if (!sidebarFocusHandlers) return false;
  const active = document.activeElement;
  if (active instanceof HTMLElement && active !== document.body && !insideSidebar(active)) sidebarOrigin = active;
  sidebarFocusHandlers[list]();
  return true;
}

const SAFE_RETURN_TARGETS = ['.tab-button[aria-selected="true"]:not(:disabled)', '.tab-icon-button[aria-controls="cockpit-sidebar"]:not(:disabled)', ".drawer-toggle"];

/** Esc from the sidebar: back to where focus was, else the selected tab. DOM focus only. */
export function returnFocusFromSidebar(): boolean {
  const origin = sidebarOrigin;
  sidebarOrigin = null;
  if (origin?.isConnected && !origin.closest("[inert]") && !insideSidebar(origin)) {
    origin.focus({ preventScroll: true });
    if (document.activeElement === origin) return true;
  }
  for (const selector of SAFE_RETURN_TARGETS) {
    const target = document.querySelector<HTMLElement>(selector);
    if (!target || insideSidebar(target)) continue;
    target.focus({ preventScroll: true });
    if (document.activeElement === target) return true;
  }
  return false;
}

// ---- Documentation --------------------------------------------------------------

/** Docs markers. Regenerate the block with `bun -e 'import { renderShortcutDocs } from "./src/app/input/shortcuts.ts"; console.log(renderShortcutDocs())'`; the shortcut tests fail when it drifts. */
export const SHORTCUT_DOCS_BEGIN = "<!-- shortcuts:begin (generated by renderShortcutDocs in src/app/input/shortcuts.ts; do not edit by hand) -->";
export const SHORTCUT_DOCS_END = "<!-- shortcuts:end -->";

const escapeCell = (text: string) => text.replace(/\|/g, "\\|");

function docsTable(rows: Array<[string, string]>, head: [string, string]): string {
  return [`| ${head[0]} | ${head[1]} |`, "| --- | --- |", ...rows.map(([key, action]) => `| ${escapeCell(key)} | ${escapeCell(action)} |`)].join("\n");
}

/** The generated part of `docs/keyboard-shortcuts.md`, on Linux/Windows key names. */
export function renderShortcutDocs(): string {
  const platform: Platform = "other";
  const row = (entry: ShortcutEntry, keys: string): [string, string] => [keys, entry.note ? `${entry.label}. ${entry.note}` : entry.label];
  // The nine select-tab entries share one row.
  const prefix = SHORTCUTS.filter((entry) => entry.prefix && !/^select-tab-[2-9]$/.test(entry.id)).map((entry): [string, string] =>
    entry.id === "select-tab-1" ? ["Ctrl+B 1 … Ctrl+B 9", "Select tab by displayed position"] : row(entry, `Ctrl+B ${entry.prefix!.display}`));
  const local = (scope: ShortcutScope) => SHORTCUTS.filter((entry) => entry.scope === scope && entry.chords)
    .map((entry) => row(entry, entry.chords!.filter((chord) => !chord.platform || chord.platform === platform).map((chord) => formatChord(chord, platform)).join(SHORTCUT_SEPARATOR)));
  const meta = (["literal-prefix", "cancel-prefix", "commands-bare"] as const).map((id) => row(shortcutEntry(id), shortcutForms(id, platform).join(SHORTCUT_SEPARATOR)));
  return [
    "### Prefix commands", "", "Press Ctrl+B, release it, then press the key. Shift and other modifier keys keep the prefix armed; Escape cancels it; there is no timeout.", "",
    docsTable(prefix, ["Keys", "Action"]), "",
    "### Prefix control keys", "", docsTable(meta, ["Keys", "Action"]), "",
    "### Terminal panes", "", "With a terminal or the browser surface focused, Cockpit takes only Ctrl+B and the key after it, these clipboard chords and Shift+Enter. Every other key reaches the program, including Tab, Shift+Tab, Esc, function keys, Alt chords and Ctrl+letter chords.", "",
    docsTable(local("terminal"), ["Keys", "Action"]), "",
    "### Files, Review, Context and Library viewers", "", "Local chords act when focus is inside the viewer and not in a text field or dialog. Alt chords match the physical key, so Option works on macOS.", "",
    docsTable(local("viewer"), ["Keys", "Action"]), "",
    "### Focused source or diff line", "", docsTable(local("line"), ["Keys", "Action"]), "",
    "### Inline browser", "", docsTable(local("browser"), ["Keys", "Action"]),
  ].join("\n");
}
