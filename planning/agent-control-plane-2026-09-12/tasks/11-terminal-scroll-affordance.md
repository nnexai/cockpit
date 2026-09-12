# Replace the unhelpful terminal-edge scrollbar line

The user reports that the right-most line beside each terminal conveys no useful scroll information. The source sets xterm scrollbar visibility and width to 8 px in `src/app/TerminalPane.tsx:49`. The exact line's runtime origin still needs confirmation.

## Inspect and reproduce

Identify whether the visible line is the xterm scrollbar, its track, a pane border, or a doubled combination. Inspect computed geometry during real output and scrolling. Compare a terminal with no history, one with extensive history, and an alternate-screen application. Check native and browser clients at normal and high-DPI scale.

Use the same commands in Herdr and Cockpit. Record wheel, PageUp/PageDown, drag if supported, return-to-bottom, application mouse mode, and text selection. Do not infer scroll position from xterm's local buffer if Herdr owns it.

## Compare two small treatments

A: keep a compact scrollbar only when it represents real scrollable history, with a visible thumb and meaningful position. Its hit target can be wider than the painted thumb.

B: remove the persistent decorative line and reveal a compact position/return-to-bottom affordance during scroll or when away from the bottom. Keep a discoverable pointer path where needed.

Choose with temporal browser/native evidence. Do not replace Herdr scrolling with a local scrollback implementation or change terminal text, font metrics, pane order, or application-mouse semantics. Coordinate `TerminalPane.tsx` ownership with packets 01 and 02.

## Acceptance

- At the live bottom or with no history, no unexplained full-height line resembles a divider or broken scrollbar.
- While scrolled, the affordance corresponds to actual Herdr state and offers a working return path.
- Hover, drag, pending frames, and new output do not shift the terminal grid or steal selection.
- Wheel, keyboard scrolling, app mouse mode, Shift text selection, and glyph continuity remain intact.
- Record a short temporal capture at desktop and all portrait sizes. A before/after still alone does not pass.

## Delivery rules

This packet describes future implementation. The current planning pass does not execute it.

When assigned, inspect the current checkout and reproduce before editing. Preserve unrelated work. Use a uniquely named disposable Herdr session and browser profile, never `default` or inherited user resources. Keep Herdr terminal content, status meanings, hierarchy, ordering, tabs, and pane layout authoritative. Compare changed Herdr-backed behavior with its TUI. Cockpit-owned graphical content may be redesigned.

Verify the complete real path at 1440×900, 800×1000, 600×900, and 480×900 where relevant. Use native Cockpit as well as the browser for clipboard, platform input, and window-dependent behavior. Use real data; mockup clicks and fixture pages are not acceptance evidence. Record PASS, FAIL, or INCONCLUSIVE, created resources, and cleanup. Remove replaced UI in the same increment, run appropriate checks, and commit only the completed scope. Report observed behavior, remaining limits, evidence paths, and commit.
