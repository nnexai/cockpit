# Make Review complete and as responsive as herdr-reviewr

The user reports that Review limits make it unusable and wants herdr-reviewr behavior and responsiveness. Review's graphical content is Cockpit-owned; its pane identity and layout remain Herdr-owned.

## Measure the real comparison

Read `crates/cockpit-core/src/review.rs:27-33`, `623-692`, `src/app/review/ReviewPane.tsx`, `ReviewViewer.tsx`, and `review.css`. Inspect installed herdr-reviewr behavior/source where needed. Compare the same disposable checkout, scopes, changed files, file selection, hunk navigation, and comments in both apps. Do not treat the atlas review-comments PASS fixture image as acceptance.

Include staged/unstaged overlap, branch comparison, renames, deletions, untracked files, binary/large generated files, more than 256 changed files, and a source over 512 KiB. Measure first useful diff, subsequent file/hunk navigation, refresh, and memory. The current snapshot build awaits per-file diffs eagerly and has separate file/output/snapshot bounds.

## Proposed redesign

Use [the shared viewer design](../VIEWERS.md) with packet 06. Match source/diff typography, file rows, document insets, gutters, controls, and comment editors. Consolidate shared rules instead of adding Review-specific font overrides.

The [real issue-4 trial](../workflow/issue-4/README.md) found three files, saved two annotations, and generated their preview. Refresh and tab return reset the selected file. A refresh briefly displayed zero comments before restoring the saved batch. First file-list display took 5,828 ms, a subsequent file display 798 ms, and one comment save 1,832 ms. These are single-run observations, not a benchmark or diagnosed backend cost. Measure first useful diff separately from the file list and from renderer detection.

Keep one scope toolbar, changed-file navigation, a readable diff, inline comments, and compact draft actions. Show base ref only when the selected comparison needs it. Use file and hunk loading on demand with cancellable work and bounded caching if measurements support it. Preserve immutable revision identity. A partial large diff must remain inspectable without falsely claiming the whole review is loaded.

Preserve selected file, hunk, old/new side, scroll, and drafts on refresh when identity matches. Avoid reconstructing source view with scroll zero on every switch. Retain exact side-aware anchors and stale revalidation. Coordinate shared comment/source changes with packet 06.

Separate the measured loading/performance repair from any larger visual cutover. Verify and commit each complete increment.

## Acceptance

- All corpus changes are reachable; a limit never silently omits files or makes remaining review navigation unusable.
- Report comparable cold/warm timings against herdr-reviewr. Provisional local target: visible acknowledgement within 100 ms and cached file navigation within 150 ms at p95; record hardware/corpus and revise with measured evidence.
- Keyboard file/hunk navigation stays responsive during loading. Cancellation retains the last view and selected file.
- Narrow split panes can collapse file navigation without changing Herdr topology.
- Multi-file old/new comments preview correctly, survive refresh/reopen, and paste once to an explicit eligible same-tab target without Enter.
- Binary, truncated, stale, and failed reads have resource-local outcomes and useful next actions. Delete the replaced eager/legacy path when the new contract is complete.

## Delivery rules

This packet describes future implementation. The current planning pass does not execute it.

When assigned, inspect the current checkout and reproduce before editing. Preserve unrelated work. Use a uniquely named disposable Herdr session and browser profile, never `default` or inherited user resources. Keep Herdr terminal content, status meanings, hierarchy, ordering, tabs, and pane layout authoritative. Compare changed Herdr-backed behavior with its TUI. Cockpit-owned graphical content may be redesigned.

Verify the complete real path at 1440×900, 800×1000, 600×900, and 480×900 where relevant. Use native Cockpit as well as the browser for clipboard, platform input, and window-dependent behavior. Use real data; mockup clicks and fixture pages are not acceptance evidence. Record PASS, FAIL, or INCONCLUSIVE, created resources, and cleanup. Remove replaced UI in the same increment, run appropriate checks, and commit only the completed scope. Report observed behavior, remaining limits, evidence paths, and commit.
