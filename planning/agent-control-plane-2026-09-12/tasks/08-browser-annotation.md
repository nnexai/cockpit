# Make annotation toolbar visibility define the mode

The user requests: toolbar visible means annotate; closing it means browse; shortcuts; a Select tool for existing annotations; and element hover preview before committing an element anchor.

## Scope

Own `browser-extension/content.js`, related popup/background state only where required, and focused extension checks. Reuse the existing annotation/draft schema and persistence. The current code has separate `mode` and `tool` state plus edit/select behavior. Do not introduce a second annotation store. Refer to the Browser capture discussion mock.

## Work

Open the toolbar into annotation mode. Close it into ordinary browsing with every draft retained. Provide a reachable extension action and documented shortcut to reopen it. Remove the redundant browse/annotate selector after this lifecycle works.

Add Select beside drawing tools. Select, edit, and delete existing annotations without accidentally drawing. Support movement only where the underlying annotation type can preserve its anchor correctly. For the Element tool, outline the hovered eligible page element before click. Exclude the extension UI and recompute on scroll or layout change.

Keep document/frame identity with every anchor and hover preview. Revalidate after DOM replacement; a matching selector alone must not retarget an annotation. Test same-page DOM mutation, supported iframe boundaries, and extension-UI exclusion.

Use conflict-checked tool keys when the toolbar is active, such as V/Select, E/Element, R/Region, and F/Freehand. These are proposed bindings to validate, not existing Herdr or browser shortcuts. Ignore them in inputs, textareas, selects, contenteditable elements, and IME composition. Escape cancels the current gesture/editor without losing text; the next Escape closes the toolbar. Keep tooltips and focus indication compact.

## Acceptance

- On an actual page, close the toolbar and click links, type in inputs, select text, and scroll normally. Reopen with drafts intact.
- Preview an element on hover before any annotation exists. Click anchors the previewed element; moving the page updates or marks stale accurately.
- Select/edit/delete a saved mark with pointer and keyboard. Tool switches do not duplicate it.
- Tool shortcuts do not type into the page or interfere with editable content.
- Capture, reload, navigation, stale review, and delivery preserve the existing draft lifecycle.
- Inspect real extension controls at desktop and portrait sizes. No fixture-only proxy replaces the production extension path.

## Delivery rules

This packet describes future implementation. The current planning pass does not execute it.

When assigned, inspect the current checkout and reproduce before editing. Preserve unrelated work. Use a uniquely named disposable Herdr session and browser profile, never `default` or inherited user resources. Keep Herdr terminal content, status meanings, hierarchy, ordering, tabs, and pane layout authoritative. Compare changed Herdr-backed behavior with its TUI. Cockpit-owned graphical content may be redesigned.

Verify the complete real path at 1440×900, 800×1000, 600×900, and 480×900 where relevant. Use native Cockpit as well as the browser for clipboard, platform input, and window-dependent behavior. Use real data; mockup clicks and fixture pages are not acceptance evidence. Record PASS, FAIL, or INCONCLUSIVE, created resources, and cleanup. Remove replaced UI in the same increment, run appropriate checks, and commit only the completed scope. Report observed behavior, remaining limits, evidence paths, and commit.
