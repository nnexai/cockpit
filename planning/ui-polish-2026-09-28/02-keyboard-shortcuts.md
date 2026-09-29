# 02 — App-wide keyboard shortcut scheme

Design only. No product code was changed. Evidence is `path:line` from the working tree on 2026-09-28. Anything I could not run is marked `[INFERENCE]`. I could not run the browser build or a native build in this pass, so every "current behavior" claim below comes from reading code, and the acceptance checks in §11 are the runtime proof still owed.

## 1. Goal & users

One keyboard model for the whole workbench (sidebar, tab strip, terminal panes, Files/Review/Context viewers, Library view, inline browser, dialogs) that:

1. never takes a key the terminal or a TUI in it should receive, beyond the ones Herdr's own prefix model already takes;
2. matches Herdr TUI bindings wherever Herdr has one (cockpit-ui-parity skill), and never gives a Herdr-bound key a different meaning;
3. is discoverable: every bound action shows its key in the Commands list and on its button/tooltip;
4. has one source of truth, so the palette, tooltips, docs and key router cannot drift again.

Users: a keyboard-first operator moving between Spaces/tabs/panes/agents, opening the Library or browser from a terminal, and reading/reviewing files. Secondary: a Herdr TUI user expecting `Ctrl+B` + Herdr keys, on Linux and macOS, in the native app and the browser build.

Out of scope: visual design of the sidebar, Library or top bar (specs 01 and 03/04). I name the shortcut slots they asked for and the tooltip strings they must show.

## 2. Evidence

### 2.1 Current inventory (every binding found in code)

Legend: **G** window-level (capture listener), **P** after `Ctrl+B`, **V** viewer-local (focus inside), **M** modal/overlay-local, **T** terminal handler, **B** browser surface. Unless noted the binding is documented nowhere but the code.

#### Global router — `src/app/input/keymap.ts`, installed at `App.tsx:1482-1488` (window, capture)

| # | Key | Behavior | Evidence |
| --- | --- | --- | --- |
| G1 | `Ctrl+B` (no other modifier) | Arms the prefix and swallows the key (`preventDefault` + `stopPropagation`). Only when `prefixSafe`: not in a modal, not in an editable element (INPUT/TEXTAREA/SELECT/contenteditable) unless inside `.terminal-host`, and not anywhere inside `.browser-pane` | `keymap.ts:57-62,72-78` |
| G2 | `Escape` while armed | Disarms; swallowed only if `prefixSafe` and no modal | `keymap.ts:63-70` |
| G3 | bare `?` | Opens Commands when focus is not editable and not in the browser pane. Docs only list `Ctrl+B ?` | `keymap.ts:79-82`; `docs/keyboard-shortcuts.md:7` |
| G4 | modifier-only keydown while armed | Ignored, prefix stays armed, no timeout | `keymap.ts:52-54,86` |
| G5 | Ctrl/Alt/Meta + any key while armed | Disarms **without** `preventDefault`, so the key continues to the focused element. `Ctrl+B Ctrl+B` therefore delivers `^B` to a focused xterm (de-facto tmux "send-prefix"); not documented, not tested `[INFERENCE: not run]` | `keymap.ts:87-90` |
| G6 | unbound key while armed | Swallowed (`preventDefault`), prefix disarmed, no feedback | `keymap.ts:99-100` |
| G7 | anything while a modal is open | Router returns; the modal owns keys. `dialog[open]` is detected by DOM; App `modalOpen` covers Commands/chooser/session/setup/recovery/teardown/Library-add only | `keymap.ts:59,71`; `App.tsx:1002` |
| G8 | `data-browser-input` exemption | Read by the router but **never set anywhere in `src/`**: dead branch, so the browser surface can never arm the prefix | `keymap.ts:61-62`; grep of `src` finds only this reader |

#### Prefix table (after `Ctrl+B`) — `keymap.ts:9-36`, run at `App.tsx:1449-1481`, labels at `App.tsx:674-698`

| Key | Action | Herdr default for the same key |
| --- | --- | --- |
| `?` | Commands | `help` — same meaning |
| `Shift+N` | New Space (bare `space_create`, no dialog) | `new_workspace` — same |
| `Shift+W` | Rename Space | `rename_workspace` — same |
| `Shift+D` | Close Space (`window.confirm`, `App.tsx:1341`) | `close_workspace` — same |
| `c` | New tab | `new_tab` — same |
| `Shift+T` | Rename tab | `rename_tab` — same |
| `p` / `n` | Previous / next tab | same |
| `Shift+X` | Close tab (`confirm`, `App.tsx:1342`) | `close_tab` — same |
| `1`..`9` | Select tab by displayed position | `switch_tab` — same |
| `Shift+P` | Rename pane (dialog) | `rename_pane` — same |
| `v` / `-` | Split right / below | `split_vertical` / `split_horizontal` — same |
| `x` | Close pane (`confirm`, `App.tsx:1343-1346`) | `close_pane` — same |
| `z` | Toggle pane zoom | `zoom` — same |
| `r` | Focus first `.resize-handle`, then arrows | `resize_mode` — similar (Cockpit has no resize mode) |
| `h j k l` | Focus pane left/down/up/right | same |
| **`o` / `Shift+O`** | Next / previous pane | **Herdr `o` = `open_notification_target`**; Herdr cycles panes with `Tab` / `Shift+Tab` — **conflict** |
| `f` | File picker in focused Files/Review | unbound in Herdr defaults; **the user's own `config.toml:26-30` binds `prefix+f` to a file-viewer plugin** |
| **`[` / `]`** | Focus file tree / content | **Herdr `[` = `copy_mode`** — **conflict** |

Undocumented in `docs/keyboard-shortcuts.md`: `Shift+N/W/D/X`, `x`, bare `?`. `docs:11` documents `h/j/k/l` and `o`; the palette labels (`App.tsx:674-698`) are a third hand-typed copy of the same table.

Herdr defaults with **no Cockpit binding** (`herdr.dev/docs/config-reference`, 0.9.1): `s` settings, `Shift+G` new worktree, `w` workspace navigation, `g` session navigator, `q` detach, `Shift+R` reload config, `e` edit scrollback, `Shift+H/J/K/L` swap pane, `Tab`/`Shift+Tab` cycle pane, `b` toggle sidebar. The installed `~/.config/herdr/config.toml` sets no `keys.prefix`, so the default `ctrl+b` applies, plus custom `prefix+alt+a`, `prefix+alt+r`, `prefix+f`, `prefix+shift+f` (plugin actions). Cockpit swallows all of these silently (G6); it cannot forward them because Herdr's key API rejects `prefix+` strings (`research/ui-implementation-constraints.md:91`).

#### Terminal handler — `src/app/TerminalPane.tsx:572-591`

| Key | Behavior | Evidence |
| --- | --- | --- |
| `Ctrl/Cmd+Shift+C` | Copy selection; with no selection returns `true` and xterm handles it. The installed xterm emits **no bytes** for Ctrl+Shift+letter (`node_modules/@xterm/xterm/src/common/input/Keyboard.ts:312-375`), so no stray `^C` | `TerminalPane.tsx:575-580` |
| `Ctrl/Cmd+Shift+V` | Paste via async clipboard read | `TerminalPane.tsx:581-585` |
| `Shift+Enter` | Sends bare LF | `TerminalPane.tsx:142-151,586-590`; `research/ui-implementation-constraints.md:59` |
| context menu `Escape` | Closes the Copy/Paste menu (window listener, non-capture) | `TerminalPane.tsx:473-477`; menu items `:1075-1076` show no key hints |
| everything else | Straight to xterm/Herdr | — |

On macOS the terminal claims `Cmd+Shift+C/V`, not the native `Cmd+C/V`; xterm treats plain `Cmd+A` as select-all only (`Keyboard.ts:361-364`) `[INFERENCE for Cmd+C/V: not run on macOS]`.

#### Viewer-local (Files, Review, Context, Library — one `ContextViewer` serves all but Review)

| Key | Where | Behavior | Evidence |
| --- | --- | --- | --- |
| `Ctrl/Cmd+P` | not from editable target | Open file picker (also in Library) | `ContextViewer.tsx:1638`; `ReviewPane.tsx:501` |
| `Alt+1` / `Alt+2` | same | Focus tree / content. Matches `event.key === "1"`; on macOS Option+1 produces `¡`, so these **never fire on macOS** `[INFERENCE from macOS key values; Herdr documents the same Alt composing problem, `herdr.dev/docs/keyboard`]` | `ContextViewer.tsx:1639-1640`; `ReviewPane.tsx:502-503` |
| `Alt+Z` | same | Toggle wrap (uses `event.code`, works on macOS) | `ContextViewer.tsx:1641`; `ReviewPane.tsx:504` |
| `Alt+←/→`, `Alt+↑/↓` | Review diff focus | Previous/next file, previous/next hunk. Tooltips carry the keys | `ReviewPane.tsx:436-439,515-518` |
| `c` / `Shift+C` | source line / diff focus, no Ctrl/Cmd/Alt | Comment on lines / whole file | `ContextViewer.tsx:412-419`; `ReviewPane.tsx:441`; labels `ContextViewer.tsx:1598`, `ReviewPane.tsx:539` |
| `↑ ↓ Home End` | source line / diff | Move line cursor; Shift extends (Review) | `ContextViewer.tsx:420-426`; `ReviewPane.tsx:442-443` |
| tree `↑ ↓ Home End ← → Enter` | Context tree | Roving focus, expand/collapse, open | `ContextViewer.tsx:1090-1130` |
| Library tree `↑ ↓ Home End ← → Enter`, `ContextMenu`, `Shift+F10` | Library tree | Same, plus row menu | `LibraryTree.tsx:357-395` |
| Review file list `↑ ↓ Home End` | Review | Select file | `ReviewPane.tsx:445-453` |
| `Escape` | narrow tree overlay | Closes overlay, focus to document | `ContextViewer.tsx:1676`; `ReviewPane.tsx:529` |
| `↑ ↓ Ctrl+N Ctrl+P Enter Esc Tab` | File picker | Choose/open/close; Tab trapped | `FilePicker.tsx:36-57`; hint text `:65` |
| `Ctrl/Cmd+Enter` | comment textarea | Save comment; `Esc` (native `<dialog>` cancel) dismisses | `CommentDrafts.tsx:308-314`; `CommentEditor.tsx:28`; `CommentOverview.tsx:11` |
| tree splitter `←/→` (`Shift` = 48 px), `Home` | splitter | Resize/reset | `ViewerLayout.tsx:91-99` |

Library-specific: the Library view adds only `Escape` → close (`LibraryView.tsx:67-71`, skipped when `defaultPrevented`); it inherits the viewer keys above. The Library toolbar has **Files** (`Alt+1`), search (`Ctrl+P`), **Add…**, **Refresh all**, **Preview/Source**, **Wrap** (`Alt+Z`), and a refresh icon (`ContextViewer.tsx:1646-1659`); only Wrap shows its key. No Library shortcut opens the view: the only route is Commands → "Open Library" (`App.tsx:1556`, no `shortcut` field) or a pointer.

#### Workbench overlays, dialogs, menus

| Surface | Keys | Evidence |
| --- | --- | --- |
| Commands | `↑ ↓` (wrap), `Enter` runs the highlighted row, `Tab` trapped, `Esc` (window bubble listener via `useModalFocus`), footer text "↑↓ navigate · Enter choose · Esc close". No `Ctrl+N/P`. Default view shows only 4 quick actions (`primaryIds`); the rest sit behind the "All commands" button | `App.tsx:294-316,318-325,708,725-736` |
| Pane rename/move/swap chooser | `Enter` submits form, `Tab` trapped, `Esc` | `App.tsx:759,747` |
| Session chooser | `↑ ↓` in the search field, `Enter`, `Tab` trapped, `Esc` | `App.tsx:787-789` |
| App context menu | `↑ ↓ Home End`, `Esc`; `Tab` not handled | `App.tsx:344,364-370` |
| Library menu | `↑ ↓ Home End`, `Esc`, `Tab` blocked | `LibraryTree.tsx:101-104` |
| Inline rename (Space/tab) | `Enter` commits, `Esc` cancels, blur cancels | `App.tsx:328-330` |
| Setup dialog | `Enter` in any text input = create, `Esc` closes (also from outside the dialog), `Tab` trapped; repository combobox `↑ ↓ Ctrl+N Ctrl+P Enter Tab Esc` | `SetupDialog.tsx:252-265,589-596,738-759` |
| Library Add / confirm | `Esc` closes, `Tab` trapped, `Enter` submits in text field | `AddContextDialog.tsx:468-475`; `LibraryConfirmDialog.tsx:19-30` |
| Context resources | `Esc`, `Tab` trapped | `ContextResources.tsx:41-60` |
| **Teardown review, Pending cleanup** | `role="dialog" aria-modal` with **no key handler**: no `Esc`, no `Tab` trap | `TeardownDialog.tsx:96`; `TeardownRecoveryPanel.tsx:67` |
| Narrow sidebar drawer | `Esc` closes, `Tab` trapped | `App.tsx:1050-1068` |
| Separators | pane resize `←→↑↓` (5 px, `Shift` 20 px) `App.tsx:645`; sidebar resizer `←/→` 8 px, `Home` reset `App.tsx:1621`; browser splitter `←→↑↓` 2 %, `Home/End` `App.tsx:1256-1269` | as listed |
| Browser color picker | `Esc` (document capture) | `AnnotationControls.tsx:40-47` |
| Sidebar rows, tab strip | Native buttons only: no arrow navigation, no shortcut to reach them except `Tab` | `App.tsx:404-446,452-458,480-510` |

#### Inline browser — `src/app/browser/BrowserPane.tsx`

- Surface `div.browser-surface tabIndex=0` forwards **every** keydown/keyup to the remote page when input is active and the target is the surface itself, calling `preventDefault` first, including `Tab` and `Escape` (`BrowserPane.tsx:1457-1468,2217`). Exception: `Delete` removes the selected annotation (`:1470-1485`).
- Because the router refuses to arm inside `.browser-pane` (G1, G8), once the surface has focus the only keyboard route out is none: `Tab` is forwarded, `Ctrl+B` is not seen. **Keyboard trap.** The pointer is the only exit `[INFERENCE: not exercised at runtime]`.
- Annotation note editor: `Ctrl/Cmd+Enter` saves, `Esc` dismisses (`BrowserPane.tsx:2223`).
- Copy/paste events are forwarded as remote clipboard commands (`:1505-1513`).
- Toolbar buttons have `title`s but no keys (`:2185-2186`). The top-bar browser button title is only "Open/Close browser" (`App.tsx:510`).

#### Native shell

`src-tauri/tauri.conf.json:12-26` declares one window; no `Menu`, `accelerator` or `GlobalShortcut` appears anywhere in `src-tauri/` (grep: no matches). Cockpit therefore adds no native shortcuts. Tauri's default macOS application menu (Quit, Hide, Close Window, Edit Copy/Paste) would still exist `[INFERENCE: Tauri v2 default; not run]`, which is why `Cmd+W`, `Cmd+Q`, `Cmd+C/V`, `Cmd+H`, `Cmd+M` must be treated as OS-owned on macOS.

#### Discoverability today

- Hints exist on exactly: New tab button (`App.tsx:510` "New tab (Ctrl+B c)"), Wrap (`Alt+Z`), Review file/hunk arrows, comment `C`, the Commands list rows (`App.tsx:734`, `kbd` styled `styles.css:2395`), and the armed-prefix chip that only says "Ctrl+B" (`App.tsx:1647`).
- No hint on: sidebar toggle (`App.tsx:480`), Commands button (`:510`), browser button, Space `+` (`:402`), session selector (`:886`), tab buttons, pane expand (`:608`), Files/search/refresh in viewers, Library controls.
- Commands quick view hides all but 4 shortcuts.

### 2.2 Reuse

- `kbd` chip: `.command-row kbd` = `1px solid var(--border)`, `--radius-small` (3 px), padding `1px 4px`, `--font-size-2xs` (`styles.css:2395`). Reused for every hint.
- Armed-prefix chip `.prefix-indicator` fixed bottom-right (`styles.css:2169`, `App.tsx:1647`) — kept in place; only its content grows.
- Escape stack idioms already present: `useModalFocus` restores the opener (`App.tsx:294-316`), `LibraryView` restores invoker or a safe target that never sends a Herdr request (`LibraryView.tsx:8-23`), FilePicker restores focus (`FilePicker.tsx:21-25`).
- Design constraints: `research/ui-design-direction.md:190` (Cockpit must not intercept common terminal chords merely because a GUI action exists; magic escape priority not implemented), `ui-implementation-constraints.md:87-91`, `DECISIONS.md:17` (`focusOnAttach` suppression on Library close and browser input).

## 3. Scheme (rules)

**R1 — Prefix-first.** Every Cockpit-wide action is `Ctrl+B <key>`, on Linux and macOS alike (literal Control on macOS, as in Herdr). No new prefix-free global chords in this pass (§14 OQ2).

**R2 — Herdr wins.** A key that Herdr binds by default keeps Herdr's meaning in Cockpit or is left unbound. Cockpit-only actions use keys Herdr leaves free (`a i t u y`, `,` `.` `;`, and Shifted forms of unbound letters) and avoid the user's custom `f`, `Shift+F`, `Alt+A`, `Alt+R` slots.

**R3 — Terminal-focused claim set.** With a terminal pane or the browser surface focused, Cockpit consumes exactly: `Ctrl+B` (arming) and the one key after it; `Ctrl/Cmd+Shift+C`, `Ctrl/Cmd+Shift+V` (clipboard); `Shift+Enter` (LF). Never `Esc`, function keys, `Alt+*`, plain `Ctrl+letter` or any other chord. **Plain `Tab` and `Shift+Tab` are never consumed while a terminal is focused** (shell completion, TUI focus cycling); they mean "next pane" only as the key immediately after an armed `Ctrl+B`. `Ctrl+B Ctrl+B` sends a literal `Ctrl+B` (formalizing G5).

**R4 — Local chords by scope and family.**
- Viewer scope (focus inside Files/Review/Context/Library, not in an editable field): `Mod+<letter>` for OS-standard actions (`Mod+P` open, `Mod+R` reload), `Alt/Option+<key>` matched by `event.code` for panel navigation (`Alt+1/2/M/Z`, `Alt+arrows`). `Mod` = `Ctrl` on Linux/Windows, `Cmd` on macOS; code accepts both in viewer scope (no terminal there).
- Plain letters only where a line/row is focused and the action is about that row (`c`, `Shift+C`). Plus `/` as picker alias (punctuation, viewer scope only).
- Widget-local navigation (`↑ ↓ ← → Home End Enter Space ContextMenu Shift+F10`) is per-widget, never global.

**R5 — Shift marks the less frequent or heavier variant** (Herdr convention: `n`→`Shift+N`, `t`→`Shift+T`, `x`→`Shift+X`). Destructive actions keep their `window.confirm`.

**R6 — Escape is innermost-first and never leaks.** Order: armed prefix → open menu/popover → dialog/picker/overlay → narrow tree overlay or drawer → Library view. In a terminal or browser surface, `Esc` belongs to the program; Cockpit only uses it to cancel an armed prefix.

**R7 — Focus returns to where it came from, or to a safe target, and never causes a Herdr request.** Keyboard-initiated close of a transient surface returns focus to its invoker if mounted; otherwise to the selected tab button (existing `SAFE_FOCUS_TARGETS`, `LibraryView.tsx:9`). Exception (resolved, §14 R2): closing the Library with its toggle key returns focus to the originating pane.

**R8 — One registry.** `src/app/input/shortcuts.ts` (new) owns id, label, group, scope, prefix key, local chords and platform formatter. `keymap.ts`, the Commands list, tooltips and `docs/keyboard-shortcuts.md` all derive from it.

## 4. Proposed binding table

Change types: **Keep**, **Modify**, **New**, **Remove**, **Fix** (bug/consistency, key unchanged).

### 4.1 Global (prefix) — scope G, all focus locations except inside modals and editable fields (terminal and browser surface count as *not* editable for the prefix)

| Action | Proposed | Current | Scope | Change |
| --- | --- | --- | --- | --- |
| Commands | `Ctrl+B ?`, bare `?` | same | G (bare `?`: non-editable, non-terminal) | Keep; document bare `?` |
| Select tab 1–9 | `Ctrl+B 1`…`9` | same | G | Keep |
| Previous / next tab | `Ctrl+B p` / `n` | same | G | Keep |
| New tab | `Ctrl+B c` | same | G | Keep |
| Rename tab | `Ctrl+B Shift+T` | same | G | Keep |
| Close tab | `Ctrl+B Shift+X` | same | G | Keep (confirm) |
| New Space (bare Herdr workspace) | `Ctrl+B Shift+N` | same | G | Keep; Commands only |
| Rename Space | `Ctrl+B Shift+W` | same | G | Keep |
| Close Space | `Ctrl+B Shift+D` | same | G | Keep (confirm) |
| Set up a task Space (dialog; the Spaces `+`) | `Ctrl+B Shift+S` | none | G | New |
| Rename pane | `Ctrl+B Shift+P` | same | G | Keep |
| Split right / below | `Ctrl+B v` / `-` | same | G | Keep |
| Close pane | `Ctrl+B x` | same | G | Keep (confirm) |
| Toggle pane zoom | `Ctrl+B z` | same | G | Keep |
| Resize border | `Ctrl+B r` | same | G | Keep |
| Focus pane left/down/up/right | `Ctrl+B h j k l` | same | G | Keep |
| Swap pane left/down/up/right | `Ctrl+B Shift+H J K L` | none (dialog only) | G | New (Herdr `swap_pane_*`) |
| Next / previous pane | `Ctrl+B Tab` / `Ctrl+B Shift+Tab` (only as the key right after an armed prefix; plain `Tab` / `Shift+Tab` always reach the focused terminal) | `Ctrl+B o` / `Shift+O` | G | Modify (Herdr `cycle_pane_*`); `o` / `Shift+O` unbound (resolved decision R1) |
| Toggle sidebar (collapse / drawer) | `Ctrl+B b` | none | G | New (Herdr `toggle_sidebar`) |
| Focus Spaces list | `Ctrl+B w` | none | G | New (Herdr `workspace_picker`: cursor in the list, `↑ ↓` move, `Enter` selects, `Esc` back) |
| Focus Agents list | `Ctrl+B a` | none | G | New |
| Switch session | `Ctrl+B g` | none (Commands / header click) | G | New (Herdr `goto`) |
| Toggle Library view | `Ctrl+B i` | none (Commands only) | G | New. Not `Shift+L`: Herdr `swap_pane_right` |
| Open / close browser for Space | `Ctrl+B Shift+B` | none | G | New. Mirrors the top-bar button (association open ↔ close); hide/show stay in Commands |
| Add to Library… | none (Commands, Library toolbar) | none | G | Keep unbound |
| Focus file tree / content via prefix | removed | `Ctrl+B [` / `]` | G | Remove: redundant once `Alt+1/2` work on macOS; `[` is Herdr `copy_mode` |
| File picker via prefix | `Ctrl+B f` (only acts when focus is inside a Files/Review/Context/Library viewer; otherwise no-op) | same | G→V | Keep. Note: collides with the user's personal Herdr `prefix+f` plugin binding (`~/.config/herdr/config.toml:26-30`); Cockpit cannot run that plugin action |
| Literal `Ctrl+B` to the focused terminal/browser | `Ctrl+B Ctrl+B` | de-facto (G5) | G | Fix: define, document, test |
| Cancel armed prefix | `Esc` | same | G | Keep |
| Unbound key after prefix | swallowed, chip shows "Ctrl+B <key> is not bound" for 2 s | swallowed silently | G | Modify |

Herdr defaults intentionally **not** mapped, so the doc says so: `s` (Cockpit has no settings surface; "No settings cog", `research/ui-design-direction.md:296`), `Shift+G` (task setup covers worktrees), `q` detach, `Shift+R` reload config, `e` edit scrollback, `[` copy mode, `o` open_notification_target, custom `[[keys.command]]` plugin actions.

### 4.2 Terminal scope — T (terminal pane focused)

| Action | Proposed | Current | Change |
| --- | --- | --- | --- |
| Copy selection | Linux/Win `Ctrl+Shift+C`; macOS `Cmd+C` when a selection exists (else falls through) and `Cmd+Shift+C` | `Ctrl/Cmd+Shift+C` (`TerminalPane.tsx:575`) | Modify on macOS `[INFERENCE: verify native]` |
| Paste | Linux/Win `Ctrl+Shift+V`; macOS `Cmd+V` and `Cmd+Shift+V` | `Ctrl/Cmd+Shift+V` (`:581`) | Modify on macOS |
| Newline without submit | `Shift+Enter` | same (`:586`) | Keep |
| Plain `Tab` / `Shift+Tab` (prefix not armed) | pass through to the TUI/shell | same | Keep; guaranteed, tested |
| Everything else incl. `Esc`, `Ctrl+C/D/Z/R/L/W/P/N`, function keys, `Alt+*` | pass through | same | Keep |
| Copy/Paste context menu labels | show `Ctrl+Shift+C` / `Ctrl+Shift+V` (`Cmd+C/V` on macOS) right-aligned | none (`TerminalPane.tsx:1075-1076`) | Modify |

### 4.3 Browser surface — B

| Action | Proposed | Current | Change |
| --- | --- | --- | --- |
| Arm prefix from the remote-page surface | `Ctrl+B` (surface only; the URL input, note textarea and other editable chrome stay excluded) | never armed (G8) | Fix: removes the keyboard trap; §14 OQ1 |
| Literal `Ctrl+B` to the page | `Ctrl+B Ctrl+B` | n/a (page gets `Ctrl+B` today) | New (mirrors terminal) |
| Everything else | forwarded to the page, including `Esc` and `Tab` | same | Keep |
| Delete selected annotation | `Delete` | same (`:1476`) | Keep |
| Save / dismiss annotation note | `Ctrl/Cmd+Enter` / `Esc` | same | Keep |
| Browser toolbar buttons | tooltips with key where one exists (`Ctrl+B Shift+B` on the top-bar button only) | none | Modify |

### 4.4 Viewer scope — V (Files, Review, Context, and the Library viewer)

| Action | Proposed | Current | Change |
| --- | --- | --- | --- |
| Open file picker | `Mod+P`, `/` | `Ctrl/Cmd+P` | Keep `Mod+P`; New `/` (non-editable target only) |
| Focus file tree | `Alt+1` matched by `event.code === "Digit1"` | `event.key === "1"` (`ContextViewer.tsx:1639`, `ReviewPane.tsx:502`) | Fix (macOS) |
| Focus content | `Alt+2` by `event.code` | `event.key === "2"` | Fix (macOS) |
| Toggle wrap | `Alt+Z` | same | Keep |
| Preview ↔ Source (Markdown/HTML) | `Alt+M` (by `event.code`) | none | New. Chosen because `Alt+V/F/E/H/B/T` open menus in some browsers |
| Reload listing / files (local re-read only) | `Mod+R`, viewer scope, not in editable fields; Review: refresh comparison | none | New. `Refresh all` (provider network refresh) gets **no** chord: `Ctrl+Shift+R` is the browser hard-reload muscle memory |
| Row context menu | `ContextMenu` / `Shift+F10` | Library tree only (`LibraryTree.tsx:362`) | Keep; extend to sidebar rows (spec 01) |
| Previous/next file, hunk (Review) | `Alt+←/→`, `Alt+↑/↓` | same | Keep |
| Comment on lines / whole file | `c` / `Shift+C` on a source/diff line | same | Keep |
| Save comment | `Mod+Enter` | same | Keep |
| Tree navigation | `↑ ↓ Home End ← → Enter` | same | Keep |
| Close narrow tree overlay | `Esc` | same | Keep |

### 4.5 Library view — L (focus anywhere inside the Library view)

| Action | Proposed | Current | Change |
| --- | --- | --- | --- |
| Open / close the view | `Ctrl+B i` | Commands only | New |
| Close (innermost-first) | `Esc` | same (`LibraryView.tsx:67-71`) | Keep |
| Tree move / expand / collapse / open | `↑ ↓ Home End ← → Enter` | same | Keep |
| Row actions menu | `ContextMenu`, `Shift+F10` | same | Keep |
| Quick open a Library file | `Mod+P`, `/` | `Ctrl/Cmd+P` | Keep + `/` |
| Focus tree / document | `Alt+1` / `Alt+2` | `Alt+1/2` (broken on macOS) | Fix |
| Preview ↔ Source | `Alt+M` | none | New |
| Wrap | `Alt+Z` | same | Keep |
| Reload listing | `Mod+R` | toolbar icon only | New |
| Add… | button + Commands | same | Keep unbound (dialog opens with `Enter` on the button) |
| Refresh all | button + Commands | same | Keep unbound |
| Find in tree by typing | none (`/` opens the picker) | none | Not added |

Note: `LibraryView` mounts no terminal, so `Alt+*` and `Mod+*` cost nothing here. The prefix still works (R1), see §6.3 for pane-scoped commands.

### 4.6 Sidebar — S (focus inside `#cockpit-sidebar`)

Slots requested by the sidebar spec are in §4.1 (toggle `b`, Spaces `w`, Agents `a`, session `g`, setup `Shift+S`). Sidebar-local keys agreed with that spec, active only while DOM focus is inside the sidebar and never sent to Herdr: `↑ ↓ Home End` roving focus; `←` / `→` collapse/expand a repository group; `Enter` or `Space` request Herdr focus (selection still waits for Herdr confirmation); `ContextMenu` / `Shift+F10` row menu; `Esc` returns focus to the region that had it (§6.2), never a Herdr request. `Space` must `preventDefault` so the sidebar does not scroll.

### 4.7 Overlays and dialogs — M

| Action | Proposed | Current | Change |
| --- | --- | --- | --- |
| Commands / session chooser / pane chooser: move | `↑ ↓` and `Ctrl+N` / `Ctrl+P` | `↑ ↓` only (`App.tsx:726,789`) | Modify (parity with File picker and Setup combobox); `Ctrl+N` is browser-reserved in the browser build `[INFERENCE]`, so arrows stay primary |
| Run / submit | `Enter` | same | Keep |
| Close | `Esc` | same, except the two Teardown dialogs | Fix: add `Esc` to `TeardownDialog`, `TeardownRecoveryPanel` |
| Keep `Tab` inside | trap | missing in Teardown dialogs and the App context menu | Fix |
| Setup dialog: create | `Enter` in a text input | same | Keep |
| Comment editor | `Mod+Enter` save, `Esc` dismiss | same | Keep |
| Commands footer | "↑↓ navigate · Enter choose · Esc close" + "type a name or a key" | first part only | Modify |

### 4.8 Separators (kept as-is)

Pane resize, sidebar resizer, browser splitter, tree splitter: arrow keys, `Shift` = larger step where it exists, `Home` reset. Keep; add a visible key hint in the separator's `title` only where none exists.

## 5. Conflict analysis against terminal ownership

Legend: **Owner** = who normally wants the key in a terminal pane. **Verdict**: *Keep*, *Change*, or *Guard*.

| Key | Terminal / TUI owner | Cockpit today | Verdict |
| --- | --- | --- | --- |
| `Ctrl+B` | readline back-char, tmux prefix, vim/less page-back, emacs | Consumed always in a terminal (`keymap.ts:73`); Herdr's own prefix is the same key | Keep — parity. Guarantee escape hatch `Ctrl+B Ctrl+B` (R3). Document that a nested tmux needs a different prefix or the double press |
| `Ctrl+B` in the browser surface | rich-text bold on remote pages (Confluence/Jira editors) | Not consumed (trap) | Change (§14 OQ1): consume it, same rule as terminals; `Ctrl+B Ctrl+B` forwards it |
| `?` bare | shell/TUI text | Consumed only outside editable elements; xterm's helper textarea is editable so terminals are safe (`keymap.ts:79`, `editableTarget` `:39-41`) | Keep. Guard: test that a focused xterm textarea receives `?` |
| `Esc` | vim, readline, every TUI | Consumed only while the prefix is armed | Keep. Cockpit must not add any other `Esc` handler that can see a keydown whose target is a terminal or the browser surface (`TerminalPane.tsx:473-477` and `AnnotationControls.tsx:40-47` fire on window/document; they are gated by their own open state, fine) |
| `Ctrl+Shift+C/V` | emulator convention; **xterm emits no bytes** (`Keyboard.ts:314-375`) | Consumed | Keep |
| `Cmd+C/V` (macOS) | OS copy/paste | Not consumed today | Change to consume when selection / for paste `[INFERENCE]` |
| `Shift+Enter` | Claude-style "newline"; legacy terminals send CR | Mapped to LF (deliberate, `ui-implementation-constraints.md:59`) | Keep |
| `Alt+letter`, `Ctrl+Alt+letter` | readline `M-b/f/d`, emacs `M-x`; xterm sends `ESC+letter` (`Keyboard.ts:333-345`) | Not consumed in terminals; consumed only inside viewers (`ContextViewer.tsx:1639-1641`) | Keep. Rule: `Alt/Option` chords may only be added in viewer scope |
| `Ctrl+P` / `Ctrl+N` | shell history up/down, vim | `Ctrl/Cmd+P` consumed inside viewers only; `Ctrl+N/P` inside FilePicker/Setup/Commands inputs only | Keep. Guard: never register these on `window` |
| `Ctrl+R` / `Cmd+R` | reverse history search, browser reload | proposed consumed in viewers only, not in editable targets | Guard: viewer scope only; in the browser build `Ctrl+R` reload prevention is `[INFERENCE]` — fall back to the toolbar button if a probe fails |
| Plain `Tab` / `Shift+Tab` | shell completion, TUI focus cycling, vim/emacs | Not consumed by the router (`keymap.ts:72-84` acts only on `Ctrl+B`, `Esc`, `?` when unarmed); xterm handles them | Keep — guaranteed. Cockpit reads `Tab` only as the key immediately after an armed prefix (`Ctrl+B Tab` / `Ctrl+B Shift+Tab` = pane cycle, matching Herdr `cycle_pane_*`). While armed, a modifier-only `Shift` keydown is ignored (`keymap.ts:86`), so `Ctrl+B`, `Shift`, `Tab` also works |
| `F1`–`F12` | mc, htop (`F6` sort), vim | not used | Guard: reserve no function key |
| `Ctrl+Shift+letter` as a future direct family | xterm sends nothing, so TUIs cannot bind it — but kitty-protocol apps may `[INFERENCE]`; browser build reserves `Ctrl+Shift+N/T/W` and others | none | Deferred (§14 OQ2) |
| Herdr `prefix+[` copy mode | Herdr TUI only | Cockpit swallows `Ctrl+B [` (or ran file-tree focus in viewers) | Remove Cockpit's `[`; leave unbound (swallow with "not bound" chip) |
| Herdr `prefix+o` notification target | Herdr TUI | Cockpit ran next-pane | Change: `o` / `Shift+O` unbound (swallowed with "not bound" chip); pane cycling moves to `Ctrl+B Tab` |
| Herdr `prefix+shift+l` swap right | Herdr TUI | free in Cockpit, but the earlier Library design proposed `Ctrl+B Shift+L` (`planning/shared-context-library-2026-09-26/CONTEXT_LIBRARY_DESIGN.md:431,509-511`, conditional on this exact check) | Rejected for Library; Library uses `i` |
| User's Herdr `prefix+f` / `+shift+f` / `+alt+a` / `+alt+r` (plugin actions) | user config, `~/.config/herdr/config.toml:14-36` | `Ctrl+B f` = file picker; `Alt+*` after prefix disarms and leaks (G5) | Informational; Cockpit cannot run Herdr plugin actions; documented as "not available in Cockpit" |
| Magic escape (Herdr priority key) | Herdr | Not implemented (`CONTEXT.md:191`, `ui-implementation-constraints.md:87-91`) | Dependency, unchanged: when implemented it runs before step 1 of the router; this spec adds no undocumented Herdr strings |

Hazards found while tracing (not covered by a binding change alone):

1. **Keyboard trap in the browser surface** (G8, `BrowserPane.tsx:1457-1468`): fixed by arming the prefix from the surface (§14 OQ1).
2. **Pane-scoped prefix commands act on hidden panes while the Library is open.** `libraryOpen` unmounts pane canvases (`App.tsx:1610,1627`), yet `runCommand` (`App.tsx:1449-1481`) keeps acting on `selection.paneId`: `Ctrl+B v/-`, `z`, `Shift+P`, `x` (confirm text names the pane) execute on a pane the user cannot see; `r` silently no-ops (no `.resize-handle`). Focus commands are fine: `focusTab`/`focusPane`/`focusAgent` close the Library (`App.tsx:1310-1334`).
3. **Library's own dialogs are not in `modalOpen`** (Add/confirm/Resources are `role="dialog"` sections in `ContextViewer`, not `<dialog>`; `modalOpen` covers only App-level `libraryAddOpen`, `App.tsx:1002`). With a button focused inside one, `Ctrl+B x` still reaches `runCommand`.
4. **Silent swallow** of any unbound key after the prefix (G6), including all Herdr-only keys.
5. **macOS**: `Alt+1/2` never match; `Alt+Z` and `Mod+P` work.
6. **Drift**: three hand-typed tables (`keymap.ts:9-36`, `App.tsx:674-698`, `docs/keyboard-shortcuts.md`) already disagree (§2.1).

## 6. Interaction rules

### 6.1 Scope resolution order (window capture listener, unchanged structure)

1. Composing (IME) → return (`keymap.ts:57`).
2. *(reserved)* Herdr magic escape hook — not implemented; slot stays first.
3. Armed prefix and `Esc` → disarm; consume only if the target is prefix-safe.
4. Modal open (App `modalOpen`, `dialog[open]`, **plus** Library-internal dialogs: treat `[role="dialog"][aria-modal="true"]` as modal) → return.
5. Not armed: `Ctrl+B` alone on a prefix-safe target → arm. Prefix-safe = terminal host, browser **surface** (new), or any non-editable element. Bare `?` as today.
6. Armed: modifier-only ignored; `Ctrl/Alt/Meta` + key → disarm and let the key through, except `Ctrl+B` itself which is deliberately let through (literal send, R3); registry lookup; unbound → hint, disarm, swallow.

### 6.2 Focus ownership and return

| Trigger | Focus goes to | Returns to on close |
| --- | --- | --- |
| `Ctrl+B ?` / Commands button | Commands search input (`App.tsx:719`) | opener (`useModalFocus`, `:312`); running an action may move focus elsewhere first |
| `Ctrl+B i` (open Library) | tree's selected row, else first row (`LibraryView.tsx:60-66`) | Closed with `Ctrl+B i`: the originating pane if it is still selected and mounted (resolved decision R2; terminal focus follows `DECISIONS.md:17` once painted and Herdr-confirmed). `Esc` / pointer close: invoker if mounted, else selected tab button (`LibraryView.tsx:11-23`) |
| `Ctrl+B w` | Spaces list, selected row; sidebar expanded first if collapsed or (narrow) drawer opened | `Esc` → the element focused before, else selected tab; no Herdr request |
| `Ctrl+B a` | Agents list, selected row else first | same |
| `Ctrl+B b` | if the sidebar is being hidden while focus is inside it → selected tab button; otherwise unchanged | — |
| `Ctrl+B Shift+B` open browser | browser surface only after the user clicks or after `Enter` on the surface (control intent stays a local explicit action) | on close, existing `focusPane` path (`App.tsx:1398-1400`) |
| `Ctrl+B g` | session chooser search | opener |
| `Alt+1` / `Alt+2` | tree selected row / document | — |
| File picker | input | restores previous focus (`FilePicker.tsx:21-25`) |
| Tab focus change via prefix | Herdr confirms, then xterm focus (`DECISIONS.md:17`) | — |

Focus regions for orientation (used only for hints, no separate key): sidebar → tab strip → work area. Pressing `Tab` moves within the DOM order; the prefix keys above jump to a region.

### 6.3 Library-open behavior of prefix commands

- Tab, pane, agent focus commands keep closing the Library (existing).
- Pane-scoped commands (`v`, `-`, `z`, `Shift+P`, `x`, `r`, `Shift+H/J/K/L`) **close the Library first, then run**, so the user sees the target before a destructive confirm. Commands rows for them stay enabled. (Alternative not chosen, §9 Opt-5: disable with reason "Close Library first".)
- Space-level and tab-level commands (`c`, `Shift+T`, `Shift+N`, …) run in place; their visible result (tab strip, sidebar) is not covered by the Library.

### 6.4 Escape stack (innermost first)

1. Armed prefix (capture, `keymap.ts:63`).
2. Open menu (App context menu, Library menu, terminal clipboard menu, browser colour picker).
3. Open dialog/picker/overlay (Commands, chooser, session chooser, Setup, Library Add/confirm/Resources, comment dialogs, file picker, Teardown dialogs — after fix).
4. Narrow tree overlay (`ContextViewer.tsx:1676`, `ReviewPane.tsx:529`) and narrow sidebar drawer (`App.tsx:1053`).
5. Library view (`LibraryView.tsx:67-71`; each inner layer must `preventDefault`, as `trapDialogKeys` and `FilePicker` already do).

Never handled by Cockpit: `Esc` with a terminal or browser surface focused and no armed prefix; `Esc` in an inline rename (that field's own handler cancels).

### 6.5 Prefix indicator states (fixed bottom-right chip, no layout shift)

| State | Text |
| --- | --- |
| armed | `Ctrl+B` `· ? commands · c tab · v - split · w spaces · i library` (truncates with ellipsis on narrow widths, `Esc` cancels) |
| unbound key | `Ctrl+B q is not bound in Cockpit` for 2000 ms, then hides |
| literal send | none (chip hides immediately) |

Same `role="status"`, same position (`styles.css:2169`, `App.tsx:1647`).

## 7. How shortcuts are shown

Format helper `formatShortcut(id)` from the registry:

- Linux/Windows: `Ctrl+P`, `Alt+M`, `Ctrl+B Shift+N`. macOS: `Cmd+P`, `Option+M`; prefix stays `Ctrl+B …`. Words, not glyphs, matching the existing style (`App.tsx:675`).
- Sequences render as text `Ctrl+B i` in `title`, and as two `kbd` chips (`Ctrl+B` `i`) in lists.
- `aria-keyshortcuts` only for single chords (`Control+P`, `Alt+M`); sequences go in the accessible name or a visually hidden description, since the attribute cannot express a sequence.

Where hints appear (label → string; owners of the control, not this spec, apply them):

| Control | Tooltip / hint |
| --- | --- |
| Tab-bar sidebar toggle (`App.tsx:480`) | `Hide sidebar (Ctrl+B b)` / `Show sidebar (Ctrl+B b)` |
| Tab buttons 1–9 (`App.tsx:507`) | `<label> (Ctrl+B 3)` |
| New tab (`App.tsx:510`) | `New tab (Ctrl+B c)` (existing) |
| Top-bar Library button (spec 04) | `Open Library (Ctrl+B i)` / `Close Library (Ctrl+B i)` |
| Top-bar browser button (`App.tsx:510`) | `Open browser (Ctrl+B Shift+B)` / `Close browser (Ctrl+B Shift+B)` |
| Commands button (`App.tsx:510`) | `Commands (Ctrl+B ?)` |
| Session selector (`App.tsx:886`) | `Switch session (Ctrl+B g)` plus session name |
| Spaces `+` (`App.tsx:402`) | `Set up a task Space (Ctrl+B Shift+S)` |
| Pane expand (`App.tsx:608`) | `Expand / restore pane (Ctrl+B z)` |
| Viewer Files trigger | `Files (Alt+1)` |
| Viewer search/picker | `Choose file (Ctrl+P)` |
| Viewer Preview / Source | `Preview (Alt+M)` / `Source (Alt+M)` |
| Wrap | existing `Alt+Z` |
| Viewer reload icon / Library "Reload listing" | `Reload listing (Ctrl+R)` |
| Library **Refresh all** | no shortcut text |
| Terminal clipboard menu | right-aligned `Ctrl+Shift+C` / `Ctrl+Shift+V` |

Commands list: every row shows its `kbd` group in both quick and all views. The quick view (`primaryIds`, `App.tsx:708`) grows from 4 to 8: Toggle pane zoom, Open Library, Open browser, New tab, Split pane right, Switch session, Open Review right, Set up a Space. Search matches label, group and shortcut text (already so, `App.tsx:710`), so typing `Ctrl+B i` or `swap` finds it. Rows without a key show nothing, not an empty chip. `docs/keyboard-shortcuts.md` is rewritten as the full §4 table, grouped by scope, with a "Not bound in Cockpit" section for Herdr-only keys.

Layout (`command-row`, 34 px min height, existing tokens): `[icon 14px] label …………… [kbd][kbd]`; kbd chips `1px solid var(--border)`, `var(--radius-small)`, `padding 1px 4px`, `font-size: var(--font-size-2xs)`, colour `var(--text-primary)`; disabled rows dim the label and keep the chips at `var(--text-muted)`. Hover/active state never changes chip size.

## 8. Accessibility

- No key handler steals focus on failure; every jump (`w`, `a`, `i`) moves DOM focus to a named element and is announced by that element's label (`Spaces`, `Agents`, `Library`).
- Sidebar rows: roving `tabindex`, `aria-current`/`aria-selected` follow the acknowledged Herdr selection, not the roving cursor (parity with `research/ui-implementation-constraints.md:32-34`).
- Prefix chip has `role="status"` and does not announce every key; unbound feedback is polite.
- Every icon-only control keeps an `aria-label`; the key text is added to `title` and, for single chords, `aria-keyshortcuts`.
- Focus ring stays the 2 px inset `--focus-strong` (`research/ui-design-direction.md:187`); no new focus style.
- Reduced motion: the chip appears/disappears without transition.
- The browser trap is a WCAG 2.1.2 (No Keyboard Trap) failure today; §4.3 removes it.

## 9. Options considered

**Opt-1 — Global direct chords vs prefix-only.**

| | Prefix-only (recommended) | Direct `Ctrl+Shift+<letter>` (Linux) / `Cmd+Shift` (macOS) | Direct `Ctrl+Alt+<letter>` |
| --- | --- | --- | --- |
| Terminal safety | Highest, parity with Herdr ("prefix-first so Herdr does not steal input", `herdr.dev/docs/configuration#keybindings`) | xterm emits no bytes, so TUIs can't bind them (`Keyboard.ts:314-375`) `[kitty protocol INFERENCE]` | xterm sends `ESC`+ctrl byte (`:339-345`), emacs-style TUIs receive it |
| Browser build | No conflict | `Ctrl+Shift+N/T/W` reserved, others uncertain | Desktop-owned on GNOME/KDE (`Ctrl+Alt+T/L/arrows`) per `herdr.dev/docs/keyboard` |
| Speed | Two strokes | One stroke | One stroke |
| Recommendation | **Chosen now** | Follow-up after a browser/native probe | Rejected |

**Opt-2 — Library key.** `Ctrl+B Shift+L` (earlier proposal): Herdr `swap_pane_right` → rejected. `Ctrl+B l` is focus-right. Palette only: slow. **`Ctrl+B i`** free in Herdr and user config. Trade-off: weak mnemonic; the tooltip carries it.

**Opt-3 — Pane cycling.** Keep `o` (existing muscle memory, but shadows Herdr `open_notification_target`) vs Herdr `Tab`/`Shift+Tab` (parity). **Resolved: `Ctrl+B Tab` / `Ctrl+B Shift+Tab`, `o` unbound.** Constraint from the user: with a terminal focused plain `Tab` must reach the terminal, so `Tab` is bound only as the key after the armed prefix.

**Opt-4 — Browser escape.** (A) Arm the prefix from the surface — consistent with terminals, costs `Ctrl+B` on remote pages (mitigated by `Ctrl+B Ctrl+B`). (B) A separate escape chord (e.g. `F6`) — keeps `Ctrl+B` for pages but adds a special key, and `F6` conflicts with htop/mc in terminals. Recommended: A.

**Opt-5 — Pane commands while the Library covers panes.** Close Library then run (chosen) vs disable with reason. Chosen: matches how focus commands already behave and shows the target before a destructive confirm.

## 10. Implementation steps (ordered)

1. **Registry.** Add `src/app/input/shortcuts.ts`: entries `{ id, label, group, scope, prefix?: key spec, chords?: [...], appliesWhen }`, `formatShortcut(id, platform)`, `prefixCommandForKey` built from it. Extend `PrefixCommand` (`keymap.ts:1-7`) with `toggle-sidebar`, `focus-spaces`, `focus-agents`, `switch-session`, `setup-space`, `toggle-library`, `toggle-browser`, `swap-left/right/up/down`; rename `previous/next-pane` keys to `Tab`/`Shift+Tab`; delete `focus-file-tree`, `focus-file-content` prefix entries.
2. **Router** (`keymap.ts`): treat `.browser-surface` like `.terminal-host` in `prefixSafe`; delete the dead `data-browser-input` branch; explicit `Ctrl+B Ctrl+B` pass-through; unbound-key hint callback; recognise Library-internal `[role=dialog][aria-modal]` as modal; keep the reserved magic-escape slot first. Update `keymap.dialog.test.ts` scope and add router cases (terminal `?`, browser surface, `Ctrl+B Ctrl+B`, Library dialog).
3. **App wiring** (`App.tsx`): replace `prefixCommandActions` (`:674-698`) with registry-derived rows; add `runCommand` branches (`:1449-1481`) — sidebar toggle reuses `toggleSidebarCollapsed`/`openDrawer`/`closeDrawer` (`:1032`, `:1017`), `focus-spaces`/`focus-agents` call refs supplied by the sidebar spec, `switch-session` → `openSessionChooser` (`:1004`), `setup-space` → `setSetupOpen`, library/browser toggles reuse `openLibrary`/`closeLibrary` and `browserAction`. Pane-scoped commands call `setLibraryOpen(false)` first. Give `library:open`, browser rows a `shortcut`; expand `primaryIds` (`:708`); footer text (`:736`); `Ctrl+N/P` in Commands (`:726`) and session chooser (`:789`); chip states (`:1647`, `styles.css:2169`).
4. **Hints:** apply §7 tooltips at `App.tsx:402,480,507,510,608,886`, `ContextViewer.tsx:1646-1659`, `ReviewPane.tsx:512-522`, `TerminalPane.tsx:1075-1076`. Library/sidebar/top-bar owners take the strings from `formatShortcut`.
5. **Viewers:** `ContextViewer.tsx:1636-1642` and `ReviewPane.tsx:499-505` — `Alt+1/2` by `event.code`, add `/`, `Alt+M`, `Mod+R`; wire `Alt+M` to the existing Preview/Source state (`ContextViewer.tsx:1655`).
6. **Terminal** (`TerminalPane.tsx:572-591`): macOS `Cmd+C` (when selection) and `Cmd+V`; keep `Ctrl/Cmd+Shift+C/V`.
7. **Dialogs:** move `useModalFocus`/`trapModalTab` (`App.tsx:294-325`) to `src/app/input/modal.ts`; use them in `TeardownDialog.tsx:96` and `TeardownRecoveryPanel.tsx:67`; add `Tab` handling to the App `ContextMenu` (`App.tsx:364`).
8. **Library close focus** (`LibraryView.tsx`, `App.tsx:986-989`): closing with the toggle key (`Ctrl+B i`) returns DOM focus to the originating pane if still selected and mounted (resolved decision R2); `Esc` and pointer close keep the existing invoker/safe-target path and the `focusOnAttach` suppression.
9. **Docs/tests:** rewrite `docs/keyboard-shortcuts.md` from the registry; add a test that every registry entry with a prefix key resolves through `prefixCommandForKey`, that palette strings equal `formatShortcut`, and that no registry entry uses a Herdr-default key with a different action.

## 11. Acceptance (verifiable in the browser build against a disposable fixture; macOS items need one native/macOS run)

1. With a terminal pane focused, typing `?`, `Esc`, `Ctrl+P`, `Ctrl+R`, `Alt+m`, `F6` reaches the shell (echoed/handled); none opens Commands or a Cockpit surface. `Ctrl+B` then `Esc` cancels with no terminal byte; `Ctrl+B Ctrl+B` delivers `^B` (`cat -v` shows `^B`).
1a. **Plain Tab reaches the terminal.** With a terminal focused and the prefix unarmed, type a partial command and press `Tab` in a shell with completion (e.g. `ls /us` + `Tab` completes to `/usr/`), and `Shift+Tab` in a TUI that binds it; the pane selection does not change and the armed-prefix chip does not appear. Repeat after `Ctrl+B` `Esc` (cancelled prefix): `Tab` again reaches the shell.
2. `Ctrl+B ?` opens Commands from terminal, sidebar, viewer and browser surface; every row shows its key; typing `swap` and `Ctrl+B i` each find a row.
3. `Ctrl+B i` opens the Library from a terminal, focus lands on the tree; `Esc` closes (focus per §6.2 invoker rule). Re-open, then `Ctrl+B i` closes it and DOM focus is back in the originating terminal, so typing goes to it once Herdr confirms control; `Esc`/pointer close leaves focus on the tab button as today.
4. `Ctrl+B Shift+B` toggles the browser association exactly like the top-bar button; with the surface focused, `Ctrl+B 2` leaves it and selects tab 2 (no trap); `Ctrl+B Ctrl+B` sends `Ctrl+B` to the page (probe with a text page).
5. `Ctrl+B b` collapses/expands the sidebar (drawer at ≤800 px); `Ctrl+B w` then `↓` `Enter` selects the next Space through Herdr and the highlight moves only after confirmation; `Ctrl+B a` lands on the first agent; `Ctrl+B g` opens the session chooser.
6. `Ctrl+B Tab` cycles panes forward, `Ctrl+B Shift+Tab` back (also `Ctrl+B`, `Shift`, `Tab`); `Ctrl+B o` and `Ctrl+B Shift+O` show "not bound" and change nothing. `Ctrl+B [` shows "not bound".
7. With the Library open, `Ctrl+B v` closes the Library and splits the selected pane; `Ctrl+B x` closes the Library first, then the confirm names the pane.
8. `Alt+1` / `Alt+2` / `Alt+M` / `Alt+Z`, `Ctrl+P`, `/`, `Ctrl+R` work in Files, Review, Context and Library and do nothing while the comment textarea or a search input has focus. On macOS (native): `Option+1/2` work.
9. `Esc` in each of: Teardown review, Pending cleanup, Library Add, file picker over the Library, Library menu closes only that layer; Library stays open until the last layer is gone.
10. Tooltips on every control in §7 show the exact strings; `aria-keyshortcuts` present on single-chord controls.
11. Terminal clipboard: `Ctrl+Shift+C` with and without selection sends no byte; macOS `Cmd+C/V` copy/paste.
12. `docs/keyboard-shortcuts.md` matches the registry; the registry test passes.

## 12. Dependencies on other slices

- **Sidebar (01):** owns rows and roving focus; consumes slots `b w a g Shift+S` and the sidebar-local keys in §4.6; must expose focus refs for `focus-spaces`/`focus-agents` and the tooltip strings in §7. Agreed by message.
- **Library (03/04):** Library launcher button tooltip `Open Library (Ctrl+B i)`; the launcher always exists because there is always an active Space (user-confirmed), so no no-Space fallback is specified; toolbar strings `Preview (Alt+M)`, `Reload listing (Ctrl+R)`, `Refresh all` (no key); focus-return per §6.2 and resolved decision R2. Agreed by message.
- **Top bar:** browser and Library buttons sit beside the existing browser button (`App.tsx:510`); their `title` uses `formatShortcut`.
- **Magic escape (unowned):** slot 2 in §6.1; no key is defined here.

## 13. Proposed `DECISIONS.md` entries (text only; not written)

- **Keyboard scheme.** "Cockpit-wide shortcuts are `Ctrl+B` prefix sequences on every platform. A key Herdr binds by default keeps Herdr's meaning in Cockpit or stays unbound; Cockpit-only actions use keys Herdr leaves free. Deliberate difference: Cockpit does not implement Herdr-only prefix actions (`s`, `q`, `e`, `[`, `o`, `Shift+G`, `Shift+R`, custom plugin commands). Pane cycling is `Ctrl+B Tab` / `Ctrl+B Shift+Tab` (Herdr `cycle_pane_*`); `o` / `Shift+O` are unbound. Bindings live in one registry that also drives the Commands list, tooltips and `docs/keyboard-shortcuts.md`."
- **Terminal claim set.** "With a terminal pane or the browser surface focused, Cockpit consumes only `Ctrl+B` plus the next key, the clipboard chords (`Ctrl+Shift+C/V`; `Cmd+C/V` on macOS) and `Shift+Enter`. Plain `Tab` and `Shift+Tab` are never consumed there; `Tab` is read only as the key immediately after an armed prefix. `Ctrl+B Ctrl+B` sends a literal `Ctrl+B`. Alt-, function- and plain-Ctrl chords are viewer-scoped only."
- **Browser surface.** "The prefix is recognized from the inline browser surface (not from its URL field or note editor), removing the keyboard trap; the page receives `Ctrl+B` via `Ctrl+B Ctrl+B`."
- **Pane commands over the Library.** "A pane-scoped prefix command closes the Library first, then acts on the selected pane."
- **Library close focus.** "Closing the Library with its toggle key returns DOM focus to the terminal that was focused when it opened, if that pane is still selected; `Esc` and pointer closes keep the `focusOnAttach` suppression. This amends the Library-close exception in the terminal-attachment entry."

## 14. Resolved decisions and open questions

### Resolved decisions (user, 2026-09-29)

- **R1 — Pane cycling.** `Ctrl+B Tab` / `Ctrl+B Shift+Tab`; `o` / `Shift+O` unbound. User requirement: "if a terminal is focused I need Tab", so plain `Tab` / `Shift+Tab` are never consumed with a terminal focused (§3 R3, §4.2, §5, acceptance 1a, §13).
- **R2 — Library close focus.** Option (b): closing with the toggle key (`Ctrl+B i`) returns focus to the originating pane; `Esc` and pointer close keep the current behavior. `DECISIONS.md:17` amendment text kept (§13).
- **R3 — Top-bar launcher availability.** There is always an active Space, so the Library launcher always exists; no no-Space fallback is designed.

### Open questions (only where the user must choose; recommended defaults apply until answered)

- **OQ1 — `Ctrl+B` on the browser surface.** (a, recommended) Consume it like a terminal; `Ctrl+B Ctrl+B` forwards it to the page. (b) Keep it with the page; add a separate escape chord, which cannot be `F6` (htop/mc) and needs its own decision.
- **OQ2 — Direct (prefix-free) chords for Commands / Library / sidebar.** (a, recommended) None now. (b) `Ctrl+Shift+P`-style family after a browser/native probe of reserved keys. This pass does not depend on the answer.
- **OQ3 — macOS terminal clipboard.** (a, recommended) Also bind `Cmd+C` (with selection) and `Cmd+V`, keeping `Cmd+Shift+C/V`. (b) Keep only `Cmd+Shift+C/V`. Needs one native macOS run to confirm; unverified here.

## 15. Examples

- `planning/ui-polish-2026-09-28/mocks/keyboard/shortcut-hints.html` — standalone prototype (real tokens) of the Commands rows with key chips, armed-prefix chip states, and top-bar tooltip strings.
