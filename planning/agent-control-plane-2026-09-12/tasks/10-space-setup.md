# Simplify Space setup without hiding its effects

This is a Cockpit-owned form redesign, not a relaxation of workspace ownership or lifecycle rules. Put the primary repository and required inputs first and make the actual result easy to inspect.

## Scope

Own `src/app/projects/SetupDialog.tsx`, `setup.css`, and related setup/recovery presenters. Read planning, consent, operation polling, and recovery contracts before editing. Coordinate shared protocol changes with the integration owner.

## Work

Use a compact create/open form. Show only fields required by the chosen operation. Keep optional task/context inputs in a disclosure where possible. Validate inline and retain user input through planning errors. Show the concrete checkout path, branch/base, companion, focus intent, and exact effects in the existing reviewed-plan step.

Reduce repeated headings, explanatory paragraphs, and empty progress cards. Keep useful ownership, Git state, provider failures, plan expiry, and partial-operation facts. Progress should name the current operation and failed resource, with its relevant recovery action. Closed Spaces must not hide outstanding cleanup.

## Acceptance

- Exercise create and open from a real local repository. Opening requires the correct exclusive branch/path input; creation preserves required consent.
- Changing the repository, branch, or checkout path updates the effect preview and invalidates a stale plan.
- Keyboard traversal, Enter behavior, validation focus, and Escape preserve the expected draft at portrait widths.
- A provider failure retains successful assets and identifies the affected source. A partial workspace failure offers the supported resume/accept/retry path without duplicating resources.
- Reopening recovery after a Space closes still exposes the owned operation.
- Teardown retains fresh provenance checks and exact destructive confirmation. No mock “Create” success substitutes for a real operation result.

## Delivery rules

This packet describes future implementation. The current planning pass does not execute it.

When assigned, inspect the current checkout and reproduce before editing. Preserve unrelated work. Use a uniquely named disposable Herdr session and browser profile, never `default` or inherited user resources. Keep Herdr terminal content, status meanings, hierarchy, ordering, tabs, and pane layout authoritative. Compare changed Herdr-backed behavior with its TUI. Cockpit-owned graphical content may be redesigned.

Verify the complete real path at 1440×900, 800×1000, 600×900, and 480×900 where relevant. Use native Cockpit as well as the browser for clipboard, platform input, and window-dependent behavior. Use real data; mockup clicks and fixture pages are not acceptance evidence. Record PASS, FAIL, or INCONCLUSIVE, created resources, and cleanup. Remove replaced UI in the same increment, run appropriate checks, and commit only the completed scope. Report observed behavior, remaining limits, evidence paths, and commit.
