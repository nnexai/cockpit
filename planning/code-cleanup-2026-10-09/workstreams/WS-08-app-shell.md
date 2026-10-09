# WS-08 App shell split (`App.tsx`)

Wave 1 · Size L · Depends on: – · Blocks: WS-24

## Goal
`App.tsx` becomes a short composition root. Workbench state is grouped into named hooks, and the command palette and shortcuts come from one data source. `App.tsx` is the most-changed file in the repo (86 commits in 60 days).

## Owns
- `src/app/App.tsx`
- a new `src/app/shell/` (components and hooks extracted from App)
- `src/app/input/shortcuts.ts` (metadata flags only)
- the App tests (`App.test.ts`, `App.integration.test.tsx`)

## Coordinate
`src/app/layout/*` and `src/app/session/*`: read and import only. Message the owner before changing them.

## Evidence
- `Workbench` (`~457-1218`, ~45 hooks, 22 props) and `App` (`~1219-1521`, ~20 hooks).
- In-file components: `TabStrip` (~180, ~27 props), `CommandOverlay`, `PaneDialogOverlay`, `SessionDialogOverlay`, `ContextMenu`, `RecoveryPanel`, `CompatibilityNotice`, `OpenLibraryButton`.
- `COMMANDS_ALLOWED_WHILE_BUSY` (~282) and `primaryIds` (~300) are hand-maintained lists, and the palette filter hard-codes `pull-space`, `push-space` and `subscription-limits` (~1074).
- `rendererReasonFor` (~266) parses a reason string by prefix.

## Change
1. Move each in-file component into `src/app/shell/`.
2. Replace `localWorkarea` and `supervisorOpen`/`supervisorModal`/`supervisorMounted` with one discriminated `Workarea` union and a `useWorkarea` hook.
3. Extract hooks:
   - `useWidgetWindowState` (MutationObserver/blocker block, ~505-555);
   - `useSidebarState` (~420-448, ~688-691);
   - in `App`, one hook that owns both reducers plus `drainLayoutEffects`/`dispatchOrdered`/`stateRef`.
4. Make `buildCommands(...)` a pure function, unit-testable without React.
5. Add `allowWhileBusy`, `primary` and `palette` flags to the `ShortcutEntry` metadata. Derive the three lists from them.
6. `rendererReasonFor` returns a typed result instead of parsing text.

## Keep
- Every palette entry, its label and order.
- Every shortcut and binding.
- Focus/roving behaviour, DOM structure, class names and ARIA.
- No CSS edits.

## Acceptance
- `App.tsx` is under 400 lines.
- `Workbench` is under 250 lines.
- No hand-maintained ID allowlists remain.
- `renderShortcutDocs` output is unchanged (`docs/keyboard-shortcuts.md` has no diff).

## Verify
- `bun run typecheck`; `bun run test -- src/app/App src/app/input`.
- Browser smoke on a disposable fixture (`skill://cockpit-disposable-herdr-fixture`):
  - open the palette and run commands while busy and idle;
  - open and close Library, Notes, Supervisor, the widget dock and the sidebar;
  - create, split and close tabs;
  - confirm keyboard focus returns after each overlay.
