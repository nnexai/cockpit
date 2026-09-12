# Evidence for the improvement packets

Inspected checkout: `fa3d483dc715505918db63830de7ab720fbd7c14`. Atlas baseline: `1d09b5c1ecd69c427be3bf6ab60933fa9eccfed5`. `git diff 1d09b5c..fa3d483 -- src crates` is empty. The intervening commits add documentation, but the recorded frames still prove only the captured scenarios.

No fresh Herdr or production Cockpit session was launched in this planning pass. A disposable browser checks the design artifacts only. User-reported failures remain reports until their task packet reproduces them. Historical memory informed which constraints to recheck; current source and the September 8 decisions supersede the older protocol-20/stable migration notes.

## Atlas evidence

The [r2 coverage file](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/coverage.json) records **44 PASS and 13 FAIL across 57 cells**. [Audit](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/audit.json) and [validation](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/validation.json) say PASS. Those checks validate the evidence workflow; they do not establish that all product flows work.

- [Desktop Cockpit](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/evidence/fixture-r2/cockpit-atlas-1440x900.png) shows a 224 px rail, a large empty band between Spaces and Agents, tiny glyph-only agent states, two tabs, and two terminal panes.
- [800×1000](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/evidence/portrait-sidebar-r2/cockpit-sidebar-800x1000.png), [600×900](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/evidence/portrait-sidebar-r2/cockpit-sidebar-600x900.png), and [480×900](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/evidence/portrait-sidebar-r2/cockpit-sidebar-480x900.png) show the portrait constraints. At 480, the stacked rail consumes roughly the first 256 px; both terminal panes remain narrow columns.
- The [portrait observation](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/evidence/portrait-sidebar-r2/observation.json) identifies the actual Cockpit gateway and viewports. Inspect the images as well as its prose.
- [Feedback capture](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/evidence/fixture-r2/cockpit-feedback-panel.png) shows “SPACE FEEDBACK”, “Browser annotations”, “Browser: open · browser is open”, and an empty message in a separate floating panel. The redundant copy is visible evidence.
- [Commands capture](../product-atlas-2026-09-12/runtime/workbench-terminal/commands-menu-1440x900.png) is supplemental atlas runtime evidence, outside the r2 run directory. It shows the tall actions list followed by a crowded two-column shortcut reference. Its [runtime index](../product-atlas-2026-09-12/runtime/workbench-terminal/index.json) supplies provenance. Source inspection confirms the same structural composition.
- [Context-search FAIL capture](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/evidence/repair-packet-7febd1590d7118ef-32/after-context-search-1440x900.png) shows Cockpit with a Codex terminal, not a successful Context search. It supports a reproduction requirement, not a specific search-engine diagnosis.
- [Review-comments PASS capture](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/evidence/repair-packet-e74adb74a1d63747-37/after-review-comments-draft-batch-1440x900.png) shows the external Atlas Orbit fixture page. It does not prove Cockpit Review comments. Do not use its PASS result to close packet 07.

The r2 [cleanup record](../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/cleanup.json) says the owned services and profile were removed. The earlier `runtime-handoff.json` is historical despite its `LIVE_HANDOFF` label. Do not reuse its old resources.

## User reports to reproduce

1. Terminal copy and paste fails in Cockpit while working in Herdr.
2. Portrait navigation is unreadable, cannot collapse or resize, and shows less useful information than Herdr.
3. Space drag-and-drop reordering fails; right-click actions on Spaces and tabs are poorly exposed.
4. The separate Pane button feels awkward; the command menu looks broken.
5. File restrictions prevent useful Review and file viewing; Review should match herdr-reviewr's responsiveness.
6. Context file viewing is overloaded.
7. Browser annotation requires awkward mode switches and lacks convenient selection, shortcuts, and element preview.
8. Focus should acquire control naturally and use less text/toast feedback.
9. The persistent right-most terminal line feels useless as a scrollbar.

## Current source findings

Paths are relative to the repository root. Line numbers are navigation hints for the inspected commit.

| Finding | Source | What it establishes |
| --- | --- | --- |
| Agent row hides semantic status in a glyph | `src/app/App.tsx:381`; `styles.css:1319` | The row has an aria-hidden glyph, location, and name. Exposing `agent.status` needs no new data model. |
| Existing status order | `src/app/App.tsx:61-97` | Keep `orderAgentsByHerdrPriority`, including its tie behavior. Do not implement a new priority sort. |
| Sidebar space allocation | `src/app/styles.css:297`, `1109`, `1319`, `2016` | Fixed rail widths, Agents height cap, and stacked portrait layout explain the visible gap and space loss. |
| Pane title line box | `src/app/styles.css:1444` | A 16 px header uses the general line-height. A targeted typography/geometry check is warranted. |
| Ordinary terminal clipboard path | `src/app/TerminalPane.tsx:234-286`; native adapter and capabilities | No explicit app-level clipboard integration was found. xterm/browser defaults may still handle it; absence alone does not identify the failure. |
| Focus and attachment | `src/app/session/focusCoordinator.ts:17`; `session/sessionStore.ts:85`; `TerminalPane.tsx:166-197`, `293-310` | Preserve the existing confirmed-focus and input-ownership path. |
| Terminal-edge scrollbar | `src/app/TerminalPane.tsx:49` | xterm is configured with an always-shown 8 px scrollbar. Confirm whether this produces the reported line. |
| Delayed-focus toast | `src/app/App.tsx:1211`; `styles.css:1090` | The app renders global “Waiting for Herdr focus confirmation...” text. |
| Space and tab drag and drop | `src/app/App.tsx:354-366`, `407-420` | Handlers exist; the reported failure needs gesture and transport reproduction. |
| Context menus and Pane button | `src/app/App.tsx:304-323`, `911-950` | Resource menus exist alongside the separate selected-pane button. |
| Commands composition | `src/app/App.tsx:520-529`, `967-983`; `styles.css:1622` | Long action lists and inert shortcut reference rows are mixed together. |
| Viewer overload | `src/app/context/ContextViewer.tsx:978-1020` | Reading shares the tree with search, sources, and snapshot-import controls. |
| Review refresh and source scroll | `src/app/review/ReviewPane.tsx:121-170`; `ReviewViewer.tsx:49-57` | Refresh resets selected diff/file state; source view is constructed with `scrollTop: 0`. |
| Setup plan and recovery | `src/app/projects/SetupDialog.tsx:159-196`, `360-381`, `429-462` | Keep real effect review, consent, operation identity, and recovery while simplifying the form. |
| Feedback delivery outcomes | `src/app/App.tsx:608-647`, `751-854` | Pending, rejected, accepted, and unknown outcomes already have distinct handling. Preserve it. |
| Annotation mode and tools | `browser-extension/content.js:6`, `71`, `245-276` | The extension has a separate browse/annotate mode and annotation editing. Redesign those interactions instead of adding a second annotation store. |

## Limits that need a measured replacement

These are code defaults or constants, not measurements of the user's current configuration.

| Path | Current bound | Consequence to investigate |
| --- | --- | --- |
| Context preview, `config.rs:264-289` | Default 1 MiB and 5,000 lines; configurable up to 8 MiB and 20,000 lines | Raising configuration alone still leaves a finite all-at-once preview. |
| Context directory, same configuration | Default 1,000 entries and depth 32 | A truncated tree needs continued navigation, not silent omission. |
| Context search, `context_search.rs:17-23` | 100 results, 256 scanned entries, 64 KiB per file, 1.5 seconds | Search and preview have different limits; a viewable file may not be searched. |
| Review, `review.rs:27-33`, `629-685`, `777` | 256 files, 2 MiB diff-command output, 512 KiB source file, 2,048 hunks, 4 MiB aggregate stored snapshot | The bounds constrain different stages. Do not describe them all as one file-size limit. |
| Review snapshot build, `review.rs:623-692` | Iterates changed files and awaits each diff before returning | Measure eager work before choosing lazy file/hunk loading. |
| Review comment source, `review.rs:354-376` | Rejects truncated frozen source | Any larger-file design must preserve exact revision and anchor correctness. |

Test-only Context fixtures use smaller limits than production defaults. Do not base the repair on the fixture values at `context.rs:2173`.

## What the mock checks establish

The local browser checked 120 assertions over the design artifacts. It exercised all five Cockpit-owned mock surfaces at the four requested viewports, sidebar pointer/keyboard resize, collapse, drawer geometry and focus, Commands filtering, narrow-pane file navigation, source selection, setup effect preview, annotation hover/selection/Escape, and illustrative empty/disconnected recovery.

The mock implements discussion interactions only. Resource selection and mutations in the shell report their intended action outside the product frame. Region/freehand annotation in the special-pane study creates schematic marks rather than reproducing the production gesture engine. Clipboard, drag and drop, terminal scrolling, real ownership, actual file continuation, large-review performance, durable storage, and extension delivery remain future runtime acceptance.
