# VIEW-01 — Preserve Context and Review navigation continuity

## Outcome
Context and Review remain responsive and identity-safe while directories, documents, pickers, snapshots, source pages, and searches are cancelled or replaced. Scroll restoration is per root/path/revision (and Review comparison/file/side), and long-file behavior is measured before any rendering strategy change.

## Evidence and starting points
- Baseline and dependency/lock ownership are in [../tasks.json](../tasks.json); this task depends on TERM-01 and owns `context-review`.
- No issue is assigned in the ledger. These are source-review candidates until reproduced on browser and native surfaces.
- `src/app/context/ContextViewer.tsx` owns directory/document/picker loading, Markdown/source/media rendering, selection, and scroll state; `SourceLines` currently maps loaded lines.
- `src/app/context/CommentDrafts.tsx` owns revisioned editor retention across source/review changes.
- `src/app/review/ReviewPane.tsx` owns snapshot/file cancellation, selection, diff/source scroll, and source-page callbacks.
- `src/app/review/ReviewViewer.tsx:48-63,75-80,122-147` accumulates source pages and currently validates offset/revision but not every active review identity.
- `src/app/ContextViewer.test.tsx` and `src/app/context/ContextViewer.test.tsx` provide behavior fixtures for nested picker loading and AbortSignal use; tests are not runtime proof.
- `planning/inline-space-browser-2026-09-13/03-delivery-and-verification.md` is the browser acceptance matrix authority; Context/Review are current inline panes, not legacy extension docks.

## Changes
- Give Review source pagination one controller/request identity tied to session, pane, binding, review generation, file, side, and revision; discard late pages without mutating current text, scroll, selection, or error state.
- Include review generation/revision in scroll restoration identity. Same revision revisit may restore its recorded position; a new revision starts at the documented position.
- Preserve existing Context root/path/revision identity guards and cancellation for directory, document, picker, search, and comment batches; fix only observed gaps.
- Keep source, Markdown, Mermaid, media, and safe HTML behavior intact while navigation is replaced or cancelled.
- Measure long files and large diffs on the real browser and native surfaces: DOM/line count, first presentation latency, scroll/input latency, memory, and page replacement time.
- Choose virtualization or another rendering change only if the measured budget requires it; do not add speculative virtualization or change scroll authority first.
- Require context/review cancellation and accessibility/scroll review before integration.

## Non-goals
- No new generic document store or provider framework.
- No speculative virtualization, pagination redesign, or truncation policy without measurements.
- No loss of durable comment drafts during source/review identity changes.
- No browser extension pane restoration or legacy browser migration.

## Acceptance
1. Browser and native: request a truncated Review source page, switch file/comparison/pane, resolve the old page, and observe no old text, scroll jump, selection, or error in the new view.
2. Same path with a new review generation/revision does not reuse the old scroll restoration; same revision revisit restores the documented position.
3. Cancel recursive picker indexing and document paging while switching roots/files; late results cannot replace the current directory, focus, source, or comments.
4. Context source, Markdown, Mermaid, media, and safe HTML continue to render with selection and scrolling; search/picker cancellation leaves no stale loading state.
5. At narrow and desktop widths, keyboard and wheel navigation keep the active line visible and do not reset user scroll during comments or page append.
6. Long-file evidence records measured DOM count, presented latency, scroll/input latency, and memory for representative Context and Review files; any optimization is justified by those observations.
7. TUI/Herdr hierarchy and focus remain unchanged while Context/Review panes are selected, replaced, or cancelled.

## Verification
Use disposable browser and native runs with deferred directory/document/review-page responses and a fixture containing nested directories, Markdown/Mermaid/media, a long source file, and a truncated Review. Capture identity, abort, scroll, focus, timing, DOM, and memory observations. Exercise comment draft retention across revision changes. Compare pane selection/focus with the Herdr TUI oracle. Existing component tests can guard retained contracts but cannot establish responsiveness.

## Handoff
Provide durable measurements and cancellation traces, screenshots/captures of source and Review continuity, fixture identities, and cleanup. Link evidence and the real commit in the ledger. If performance remains within budget without a rendering rewrite, record that result explicitly rather than creating a speculative follow-up.
- Build a fixture with enough nested directories to force picker indexing beyond the initially opened tree.
- Hold directory, document, search, and Review page responses open while changing root, file, comparison, or pane identity.
- Resolve each old response after the new view has painted and inspect both visible text and persisted view state.
- Exercise a same-path refresh with unchanged revision, then with a changed revision, and compare restoration behavior.
- Test source and diff scroll with keyboard navigation, wheel input, inline comment editors, and page append at the top and bottom.
- Include a Markdown document with Mermaid code, an image/media entry, and safe HTML so the measurement does not cover only plain text.
- Measure a representative long source and long diff in both browser and native, including first paint, append, scroll latency, DOM nodes, and memory.
- Record viewport width and device pixel ratio with each performance observation; do not compare unrelated fixtures.
- If a limit is exceeded, identify the smallest measured rendering change and re-run the same fixture before proposing virtualization.
- Verify aborted work does not leave an infinite spinner, stale focus target, or old error message.
- Check that comment drafts survive an identity change and are explicitly reattached to the current revision before saving.
- Compare Context/Review pane focus and selected hierarchy with the Herdr TUI oracle after each replacement.

The evidence should distinguish:
- current identity/cancellation repairs;
- existing behavior retained without change;
- measured performance observations and the decision they support;
- source-review risks still awaiting real reproduction.

Do not count a component test that resolves promises in the same tick as cancellation proof.

- Include revision, comparison, file, side, and scroll values in each before/after capture.
- Record whether long-file measurements include cold load and revisit, not only a warm render.
- Preserve a negative control in which a late page would have visibly changed the old implementation.
- The handoff must state whether virtualization was rejected, adopted, or left pending based on measurements.
- A missing native performance fixture blocks that criterion and must not be inferred from browser timings.
