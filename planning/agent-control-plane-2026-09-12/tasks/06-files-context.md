# Make Files and Context usable without an overloaded reader

The user reports that file limits make viewing impractical and that Context file viewing is overloaded. Redesign Cockpit-owned content while keeping real Herdr pane bindings and file provenance.

## Inspect and reproduce

Start with `src/app/context/ContextViewer.tsx`, `ContextSearch.tsx`, `context.css`, `crates/cockpit-core/src/context.rs`, `context_search.rs`, and production defaults in `config.rs:264-289`. Use the limit table in `planning/agent-control-plane-2026-09-12/EVIDENCE.md`. Do not confuse smaller test fixtures with defaults.

Record the user's effective configuration. In a disposable repository, include files above 64 KiB, 1 MiB, and 5,000 lines; a directory over 1,000 entries; nested paths; Markdown; images; binary and generated files. Reproduce browse, search, read, and comment separately. Search currently has its own 256-entry/64 KiB bounds.

## Work

Use [the shared viewer design](../VIEWERS.md) with packet 07. Files, Context source, diff text, and expanded source must share typography, gutters, controls, and spacing. Do not reduce type size in narrow panes.

The [real issue-4 trial](../workflow/issue-4/README.md) reproduced an unusably narrow document beside the persistent tree. It also found that Open Context remains disabled after setup until the operator changes a terminal into the companion directory. Coordinate that launch repair with packet 12; a user must not need a shell command to open prepared context.

Default to a lean reader: root, file picker/search, tree, document, and relevant local actions. Move source import, snapshot management, and draft overview out of the persistent reading path. Collapse or resize the tree based on pane width. Preserve roots, source identity, search, selected file, scroll, and drafts across navigation and updates.

Bind continuation to a file/directory revision or validated cursor. Mutate the directory or file between pages: the UI must preserve the previous view and mark it stale, without duplicate/omitted entries or anchors on another version.

Replace all-or-nothing limits with bounded incremental access where useful: continued directory enumeration, windowed large text, cancellable search, explicit partial results, and on-demand preview. Do not load an entire repository or fake completion after truncation. Keep symlink/root boundaries, finite resource use, exact original line numbers, and comment provenance. Unsupported media gets a useful local fallback.

Deliver the content-access repair and the reading-interface cutover as separate complete commits. Each must retain a working real viewer.

## Acceptance

- The test corpus remains navigable beyond the initial batch. Every partial result identifies continuation or the precise limitation.
- A file visible in the tree does not silently disappear from search because a different hidden threshold applied.
- Cancel and retry do not discard the selected file, loaded text, or draft. Navigation is responsive while large work is pending.
- At 480 px and inside a 360 px split pane, the document has a usable width and tree access remains one action away.
- Multi-file comments retain original line anchors and explicit same-tab paste target; stale sources require revalidation. Paste does not submit Enter.
- Record first-useful-content and subsequent navigation timings before/after. Delete obsolete import/read chrome during cutover.

## Delivery rules

This packet describes future implementation. The current planning pass does not execute it.

When assigned, inspect the current checkout and reproduce before editing. Preserve unrelated work. Use a uniquely named disposable Herdr session and browser profile, never `default` or inherited user resources. Keep Herdr terminal content, status meanings, hierarchy, ordering, tabs, and pane layout authoritative. Compare changed Herdr-backed behavior with its TUI. Cockpit-owned graphical content may be redesigned.

Verify the complete real path at 1440×900, 800×1000, 600×900, and 480×900 where relevant. Use native Cockpit as well as the browser for clipboard, platform input, and window-dependent behavior. Use real data; mockup clicks and fixture pages are not acceptance evidence. Record PASS, FAIL, or INCONCLUSIVE, created resources, and cleanup. Remove replaced UI in the same increment, run appropriate checks, and commit only the completed scope. Report observed behavior, remaining limits, evidence paths, and commit.
