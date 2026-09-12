# Make the existing sidebar readable and controllable

The user reports unreadable portrait navigation, no collapse or resize, and less visible information than Herdr. The atlas and current CSS corroborate the fixed-width/stacked behavior and large gap above Agents.

## Scope

Own the sidebar composition in `src/app/App.tsx` and its active rules in `styles.css`. Preserve `projectSpaceTree`, `orderAgentsByHerdrPriority`, IDs, source status text, hierarchy, and selection handlers. Do not invent a new agent table or task model. Coordinate shared-file edits with packets 04, 05, and 09.

Compare `mocks/index.html` directions A and B. Start with B: a resizable desktop rail and an overlay drawer at 800 px and below. Keep a manual collapse control on desktop. Use the current 224 px rail as a baseline; do not narrow labels merely to gain pixels.

## Work

Let Agents follow the actual Spaces content instead of a flex-grown empty band. Keep headers and rows aligned to a consistent inset. Show each agent's exact `status` once as text next to its existing name. Preserve meaningful location and selected-state contrast. Retain complete names through accessible labels and an inspectable overflow value.

Add a keyboard-operable resize separator, a reversible collapse control, and persisted local width/collapse preference. At 800×1000, 600×900, and 480×900, an open drawer overlays the canvas. It never stacks above it or changes Herdr pane rectangles. Preserve focus when crossing breakpoints.

## Acceptance

- Compare 2 Spaces/4 agents and a crowded real hierarchy with the Herdr sidebar. All resources remain reachable in the same order.
- Resize and collapse using pointer and keyboard. Opening a portrait drawer leaves the canvas bounds unchanged.
- A confirmed selection closes the drawer and follows existing focus. Rejection leaves its target and retry accessible.
- Escape restores the prior focus. Tab stays in an open modal drawer; hidden controls are not focusable.
- Long names, an empty agent list, unknown status, live updates, and 200% text zoom remain readable without page overflow.
- Delete the superseded fixed/stacked sidebar rules rather than adding another overriding CSS layer.

## Delivery rules

This packet describes future implementation. The current planning pass does not execute it.

When assigned, inspect the current checkout and reproduce before editing. Preserve unrelated work. Use a uniquely named disposable Herdr session and browser profile, never `default` or inherited user resources. Keep Herdr terminal content, status meanings, hierarchy, ordering, tabs, and pane layout authoritative. Compare changed Herdr-backed behavior with its TUI. Cockpit-owned graphical content may be redesigned.

Verify the complete real path at 1440×900, 800×1000, 600×900, and 480×900 where relevant. Use native Cockpit as well as the browser for clipboard, platform input, and window-dependent behavior. Use real data; mockup clicks and fixture pages are not acceptance evidence. Record PASS, FAIL, or INCONCLUSIVE, created resources, and cleanup. Remove replaced UI in the same increment, run appropriate checks, and commit only the completed scope. Report observed behavior, remaining limits, evidence paths, and commit.
