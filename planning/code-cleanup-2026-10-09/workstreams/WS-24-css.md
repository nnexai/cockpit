# WS-24 CSS consolidation

Wave 3 · Size M · Depends on: WS-08, WS-09, WS-10, WS-11, WS-18 (and WS-05's dead-selector list) · Blocks: –

## Goal
Each selector is declared once. Overrides use specificity instead of `!important`. Colours come from tokens. `styles.css` is organized by area. The rendering is pixel-identical.

## Owns
All `*.css` under `src/app/`.

## Evidence
- `styles.css` is 1,652 lines and re-declares selectors:
  - `.main-workarea` (~422/998), `.tab-strip` (~490/1002), `.tab-label` (~182/503/1054), `.terminal-surface` (~595/1099), `.focus-label` (~401/579/591);
  - `.context-menu` (~1136/1602), `.command-overlay` (~1231/1589/1610), `.command-row` (~1316/1592/1613), `.tab-strip-action` (~1069/1582/1615), `.tab-strip-actions` (~1063/1576/1643).
- `!important` appears in `styles.css` (~213, 847-848, 1571-1572), `notes.css` (~14 lines), `supervisor.css` (~11), `setup.css` (75, 185-187) and `library.css` (385, 415-416).
- `setup.css:185-187` and `taskSetup.css:142-143` hard-code the same hex colours.

## Change
1. Merge the duplicate rule sets.
2. Remove `!important` through selector specificity or layer order.
3. Replace hex values with the existing tokens.
4. Delete selectors with no remaining users, including WS-05's list. Confirm each with a source search.
5. Split `styles.css` into `tokens`, `shell`, `tabs`, `terminal` and `overlays` files imported in a fixed order.

## Keep
- Class names.
- Computed styles.
- Container queries and breakpoints.

## Acceptance
- No duplicate top-level selector declarations.
- No `!important` except where documented as unavoidable (third-party override).
- No hard-coded hex outside the tokens.

## Verify
- Screenshot every main surface before and after at 1440/900/420 widths: shell, tabs, terminal, palette, context menu, Library, Notes, Supervisor, setup, widgets, browser pane. Use `skill://cockpit-component-harness-screenshot` and a disposable fixture. Report any pixel difference.
- One native screenshot pass (WebKitGTK; `skill://cockpit-webkitgtk-layout-mismatch`).
