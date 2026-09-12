# Replace the command menu's broken presentation

The user says Commands looks broken. The current overlay mixes a long action list, runtime paragraphs, and a two-column shortcut reference. Some reference rows are inert.

## Scope and direction

Read `src/app/App.tsx:520-529`, `967-983`, and active command CSS near `styles.css:1580-1660`. Inspect the supplemental atlas `runtime/workbench-terminal/commands-menu-1440x900.png`. Own the CommandOverlay presenter and coordinate action definitions with packet 04.

Use a bounded searchable list of actual actions. Place an existing shortcut beside its action. Keep Files, Context, Review, browser feedback, and session selection findable by name. Group Right/Below choices with their operation, preserving both existing actions. Put long shortcut help behind a disclosure instead of mixing inert rows among commands.

Keep resource context apparent where it prevents mis-targeting. Use one title or search input, not a title plus explanatory subtitle. Use consistent row heights, aligned text, a visible selected row, and one scroll region. Unavailable commands explain the actual capability restriction.

## Runtime evidence

The [issue-4 trial](../workflow/issue-4/README.md) found that disabled Files/Review commands expose an extension reason in a tooltip, but disabled Context commands expose no reason. After plugins were enabled, Context still depended on a hidden companion-directory requirement. Coordinate the direct Context launch with packet 12; repair capability explanations and recovery consistently across all actions.

## Acceptance

- Pointer and the existing Herdr-compatible Commands shortcut open the same menu. Preserve magic-escape priority and normal typing.
- Search Files, Context, Review, Feedback, and a nonmatching query. Keyboard selection and Enter perform the same real actions as pointer selection.
- Disabled actions are not executed; their reason is accessible without relying only on hover.
- At 480×900 and 200% text zoom, the list and close control fit and all actions are reachable.
- Escape returns to the previous input element with its selection and scroll intact.
- No duplicate command implementation, fake shortcut, persistent success toast, or obsolete shortcut-grid CSS remains.

## Delivery rules

This packet describes future implementation. The current planning pass does not execute it.

When assigned, inspect the current checkout and reproduce before editing. Preserve unrelated work. Use a uniquely named disposable Herdr session and browser profile, never `default` or inherited user resources. Keep Herdr terminal content, status meanings, hierarchy, ordering, tabs, and pane layout authoritative. Compare changed Herdr-backed behavior with its TUI. Cockpit-owned graphical content may be redesigned.

Verify the complete real path at 1440×900, 800×1000, 600×900, and 480×900 where relevant. Use native Cockpit as well as the browser for clipboard, platform input, and window-dependent behavior. Use real data; mockup clicks and fixture pages are not acceptance evidence. Record PASS, FAIL, or INCONCLUSIVE, created resources, and cleanup. Remove replaced UI in the same increment, run appropriate checks, and commit only the completed scope. Report observed behavior, remaining limits, evidence paths, and commit.
