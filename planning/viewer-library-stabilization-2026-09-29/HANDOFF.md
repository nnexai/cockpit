# Viewer and Library stabilization — 2026-09-29

## Scope and starting state

Resume the paused splitter and Jira subtask edits, make tree focus visible without browser heuristics, and commit in small steps. Native scaling verification is assigned to the user. No real Library index, provider configuration, plugins.json, or Herdr session is modified by this work.

The starting working tree contained edits in ViewerLayout.tsx and seven Library Rust/UI/test files from the previous workers. Those edits are included explicitly in the commits described below.

Baseline checks from the read-only investigation:

- `bun run typecheck`: failed with the single reported error, missing `splitterLeft` at ViewerLayout.tsx:117.
- `bunx vitest run src/app/library/libraryState.test.ts src/app/library/LibraryTree.test.tsx src/app/viewer/TreeSplitter.test.tsx --reporter=dot`: 35 Library tests passed; four splitter tests failed on the same ReferenceError.
- `git diff --check`: passed.

## Increment 1 — splitter

Commit subject: `fix(viewer): anchor splitter to the actual tree column`

- Complete the existing ViewerLayout.tsx conversion to unitless widths; remove the stale `splitterLeft` call that broke both typecheck and rendering.
- Register `--viewer-tree-width` as `<number>` and convert it to pixels in the grid definition. This implements the proposed workaround for the earlier agent's reported WebKitGTK registered-length scaling issue; this run does not independently establish that engine root cause.
- Anchor the absolute splitter to grid column 1's right edge. It does not occupy an extra grid cell, and its center follows the capped track width.
- Preserve the existing preference keys, bounds, drag batching, keyboard steps, and reset behavior.
- Add a faint hover/focus tint and show the splitter's focus line whenever it holds focus.
- Update tests to mock the preceding tree's bounding rectangle and expect unitless custom-property writes.

Checks after this increment:

- `bun run typecheck`: passed.
- `bunx vitest run src/app/viewer/TreeSplitter.test.tsx --reporter=dot`: four tests passed (drag batching, final-frame flush, capped-width resizing, keyboard bounds/reset).
- `git diff --check`: passed.

Native layout and scaling are not proven by these jsdom tests.

## Increment 2 — Jira refresh and subtask ordering

Commit subject: `fix(library): recheck legacy Jira relations and scope subtask order`

- Include the previous workers' `relations_captured` index field and refresh logic: old copies default to false, trigger a full query listing instead of a watermark probe, and are re-fetched to capture relations. Successful Jira capture marks them settled. Folder/non-Jira entry constructors supply false.
- Include the previous worker's ascending numeric subtask order and tree tests showing subtasks before Attachments.
- Narrow ascending child sorting to Jira issue parents with issue children from the same provider and instance. Identify Jira by configured executable, including custom provider ids; other providers retain their supplied child order.
- Pass providers into the nesting function and include providers in the tree-row memo dependencies.
- Add regression coverage for unchanged GitLab/Confluence child ordering and custom Jira provider ids. Ascending numeric key order is a UI policy; it does not claim to reproduce a source-defined subtask order.

Checks after this increment:

- `bun run typecheck`: passed.
- `bunx vitest run src/app/library --reporter=dot`: 95 tests passed across ten files.
- After refining the new non-Jira test fixture, `bunx vitest run src/app/library/libraryState.test.ts --reporter=dot`: 26 tests passed; typecheck passed again.
- `cargo test -p cockpit-core --lib -- library::`: 109 tests passed, including the one-time legacy refetch and existing cross-page Jira parent projection tests.
- `git diff --check`: passed.
- Non-fatal warnings: React `act(...)` warnings in two LibraryTree tests, Node localStorage experimental warning, and ts-rs warnings for `deny_unknown_fields`.

These are fixture/unit checks. No refresh of the user's real Library was performed, and SCRUM-4's real parent remains unconfirmed.

## Remaining during execution

Deterministic tree focus styles and final handoff will be recorded here before completion.
