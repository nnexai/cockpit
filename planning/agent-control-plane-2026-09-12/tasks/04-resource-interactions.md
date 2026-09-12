# Repair drag and drop and resource-local menus

The user reports that Space reordering fails and that right-click menus are poorly used. Restore natural interactions on Spaces, tabs, and panes. The handlers already exist; their runtime failure is not yet diagnosed.

## Inspect and reproduce

Read `src/app/App.tsx:304-323`, `354-366`, `407-420`, `911-950`, and `styles.css:1506-1543`. Test dragging from the actual label/button center and row edge, dropping before/after a row, moving to the end, grouped worktree Spaces, and tab insertion halves. Test browser and native separately. Record the drag payload, requested Herdr mutation, response, and visible order.

## Work

Bind drag intent to stable source/neighbor IDs and its starting hierarchy view. If a remote reorder occurs during drag, revalidate or cancel; do not apply a stale insertion index. Test duplicate labels, delayed success after a newer snapshot, and the source being closed during drag. Use supported Herdr operations without inventing an atomic version precondition.

Fix the failing path at its boundary. Add a stable source/target insertion indication and pending/rejected feedback at the affected resource. Final order comes from Herdr. A no-op drop stays a no-op; stale responses must not overwrite a later confirmed reorder.

Right-click and the keyboard context-menu gesture act on the resource under the pointer or keyboard target, not a different selected resource. Expose the supported actions at Spaces, tabs, and panes with compact grouping and capability reasons. Preserve clipboard context behavior in terminal content. Share definitions between right-click, keyboard, and local overflow access.

Remove the global Pane button once a small pane-local overflow control and keyboard/touch alternatives are proven. Do not remove access to pane actions merely to reduce chrome. Coordinate with packet 05's Commands definitions and packet 01's clipboard menu.

## Acceptance

- Successful reorder matches a fresh Herdr snapshot; failed reorder retains the last confirmed order.
- Opening menus on an unfocused resource neither silently retargets commands nor steals terminal input.
- Menus fit every viewport and a small split pane, and remain scrollable with all actions reachable.
- Escape and click-away dismiss once and restore prior focus; keyboard navigation reaches every enabled action.
- Closing a resource retains existing confirmation/provenance rules. No new destructive shortcut bypasses them.

## Delivery rules

This packet describes future implementation. The current planning pass does not execute it.

When assigned, inspect the current checkout and reproduce before editing. Preserve unrelated work. Use a uniquely named disposable Herdr session and browser profile, never `default` or inherited user resources. Keep Herdr terminal content, status meanings, hierarchy, ordering, tabs, and pane layout authoritative. Compare changed Herdr-backed behavior with its TUI. Cockpit-owned graphical content may be redesigned.

Verify the complete real path at 1440×900, 800×1000, 600×900, and 480×900 where relevant. Use native Cockpit as well as the browser for clipboard, platform input, and window-dependent behavior. Use real data; mockup clicks and fixture pages are not acceptance evidence. Record PASS, FAIL, or INCONCLUSIVE, created resources, and cleanup. Remove replaced UI in the same increment, run appropriate checks, and commit only the completed scope. Report observed behavior, remaining limits, evidence paths, and commit.
