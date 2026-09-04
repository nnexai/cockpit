# UI design: project setup, context, and graphical review

The selected direction preserves the current dense graphite workbench and extends real Herdr panes. Context is the graphical replacement for a supported file-viewer pane. Review is a full Cockpit replacement for a supported Reviewr pane. Cockpit detects the extension and renders its own UI; it does not control or synchronize the extension's private state.

## Interactive designs

Start at [the mock index](mocks/index.html).

- [Context workbench](mocks/workbench.html): real-pane placement examples, hierarchical companion files, rendered Markdown/source lines, comments, payload preview, and terminal fallback.
- [Project setup and recovery](mocks/setup.html): required local repository, optional artifact, create/open worktree, explicit source selection, reviewed effects, progress/retry, and owned-resource teardown.
- [Graphical review](mocks/reviewr.html): changed files, diff scopes, original line gutters, multi-file comments, paste preview, and independent original-TUI view.

The HTML files are design artifacts with simulated data and actions. Their introductory bars and scene switches are prototype controls, not product chrome. The original [direction study](mocks/directions.html) is retained only as superseded checkpoint evidence. Do not implement its outer docks or Build/Review labels.

## Workbench hierarchy

```text
Herdr session
  Spaces tree, with current state
  Agents attention queue, separate from Spaces
  selected Space
    Herdr tabs, with user/Herdr names
      ordinary terminal pane
      file-viewer pane -> Cockpit Context/file GUI
      Reviewr pane -> Cockpit review GUI
```

Use the existing sidebar hierarchy/branch geometry and state vocabulary. Agents stays in Herdr-provided order. Do not regroup agents by Context versus Review and do not turn the sidebar into a project wizard. Local repositories appear in the setup selector; Spaces remain Herdr resources.

An issue or MR URL determines source context and perhaps a branch suggestion. It does not create a Build/Review switch. The sample tab `main` is simply a Herdr tab label.

Pane header actions remain discoverable by right-click/menu and existing shortcuts. Add `Render as`, `Show terminal view`, and `Open source folder` when available. Move, split, swap, resize, zoom, and close retain Herdr semantics. Switching renderers changes no process or layout. Closing a pane closes its backing process after the normal explicit action.

## Visual system

Retain current `src/app/styles.css` variables for colors, typography, borders, and focus. CLEAN-04 consolidates duplicates before feature styles accumulate. Baseline colors are `--app-bg #0b0e13`, `--sidebar-bg #11151c`, `--surface #151a22`, `--surface-raised #1b222d`, `--surface-selected #243144`, `--border #2a3340`, `--text-primary #e6eaf0`, `--text-secondary #a7b0be`, and `--accent #70a7ff`.

Use bundled IBM Plex Sans for chrome and IBM Plex Mono for source/terminals. Preserve the current terminal font/cell metrics. Do not copy the older research's smaller typography over today's UI. New context body text starts at 14px/22px, source at 13px/20px, metadata at 12px/17px. Test those values in real WebKit and reduce density through shared tokens if needed.

Primary regions are flat with one-pixel borders. Use the existing control radius only for buttons, menus, and dialogs. Selection, pending, error, and focus states must not change geometry. Color supplements a text/icon state. Do not add global dashboards, activity bars, landing screens, or a permanent status footer.

Reference window is 1440×900; minimum full workbench is 1024×640. Pane widths are user/Herdr controlled, so internal layouts must also work below a preferred width rather than force the outer pane to resize.

## Context pane

Default internal arrangement: header with an unsent-comment count, toolbar, and file tree beside document. Unsent range comments appear inline; whole-file comments appear below the file. The pane fills its Herdr rectangle.

- Pane header uses existing pane chrome. Label `Context` when the companion root is selected, or `Files` for a repository file root. A small `GUI` marker distinguishes the replacement without exposing implementation jargon throughout the UI.
- Toolbar has root selector, breadcrumb, Rendered/Source toggle, Search, and full-file comment action. File metadata is collapsed by default with source identity and freshness visible.
- Tree defaults to 200-232px where space permits and can resize internally. Actual folders are shown; managed issue/wiki/repository roots get type/status glyphs. `notes` is a normal user folder, not a generated scratchpad. Symlinks/unsupported entries remain visible with their reason.
- Document measure is bounded for Markdown but left-aligned within its area. Source can scroll horizontally. Large trees and source files are paginated/virtualized within finite read limits.
- At pane width below 640px, collapse the file tree into a toolbar popover. At below 420px, put secondary actions in a menu and open the requested comments overview as a local overlay. The document remains scrollable and the pane header remains usable. Offer Herdr zoom; never silently zoom or change its layout.
- No persistent comments bottom panel. Show `N comments` in the Context/Reviewr header. Click it or use the GUI comments shortcut to open an overview across files, with edit/remove, same-tab target selection, optional preview, and explicit paste. Keep reading unobstructed until the overview is requested. Inline and file-bottom comments show only unsent drafts; accepted delivery removes them from that view while retaining the receipt/history.

Opening a file requests a bounded revision-bearing read. Show the breadcrumb and loading row immediately, but do not show the previous file's content under the new name. A late response cannot replace a newer file selection. Refresh retains the selected file and scroll when its identity matches; removed files show the retained excerpt and a missing-file state.

### Markdown and Mermaid

Rendered Markdown is for reading; Source is the unmodified numbered source. Frontmatter is visually collapsed only in rendered mode. Safe links point to an explicit open action, not arbitrary in-webview navigation. Remote images do not fetch automatically.

Mermaid fences render inside the document with their original source span retained. Provide `View source` on the diagram. Invalid/oversized/timed-out diagrams show a local error and the bounded fenced code while other Markdown remains readable. Mermaid engine assets are bundled for offline operation. The prototype's diagram is a labeled static SVG; it is not a runtime proof.

A rendered paragraph or code block can offer `Comment on source lines`, selecting its actual source span. Do not fabricate source line numbers from rendered HTML. Whole-file comments are always available independently of rendering support. An image comment references the whole file; pixel-region annotation is outside this plan. PDFs are an optional bounded viewer; metadata/external opening is sufficient for the main loop.

### Reference collection

Click a source gutter line, Shift-click another for a contiguous range, then use `Add comment`. Keyboard line selection is available without requiring terminal keystrokes. The comment editor shows file path and range above a multiline field. Save adds a draft; Cancel changes nothing. Editing a comment never edits the file.

Each collected item shows the comment, path, side/range if relevant, and a stale marker when source bytes changed. Select an item to revisit its captured source; edit and remove are local item actions. Collect across any eligible files in the same pane's root/context. Full-file comments include no entire file body. Selected-line comments retain the exact excerpt and numbers.

`N comments` opens the batch overview and target selector; Preview opens the exact outgoing text. Preserve the quick paste shortcut for an already established same-tab target, and identify the target in accessible help/feedback. If that target becomes invalid or ambiguous, open the overview for selection instead of guessing. Only Herdr-detected agents in this real tab appear. Preselect the last locally used same-tab agent if still valid; otherwise select the sole candidate or require a choice. If none exists, keep the batch and show `No agent in this tab`, with copy preview available. Never silently pick an agent from another tab or spawn one.

The payload shows an actual absolute companion/worktree path that the agent can open, source revision, comment, and numbered excerpt where selected. For deleted/old-side content it explicitly says which revision/side the excerpt describes. Preview displays any removed terminal control characters and actual byte size.

Paste explicitly requests the agent's Herdr focus, revalidates target identity, and writes once without submission. Accepted text leaves the terminal ready for the user to inspect and press Enter. An uncertain result keeps the receipt and offers `Inspect agent input`, `Mark pasted`, or an explicit `Paste again`; it does not present an ordinary automatic Retry button that might duplicate text.

## Review pane

Review uses the same pane chrome, density, source selection, and comments UI. It presents a full local review feature backed by Cockpit's own Git reads.

Top row: comparison scope, base ref where relevant, revision/dirty summary, Refresh. The left tree groups staged, unstaged, and untracked changes in the default local view. This avoids presenting a partially staged file as one ambiguous diff. A file row shows status and old/new path for renames.

Unified diff is the first renderer. Use separate old/new gutters and understated addition/deletion backgrounds; source text remains readable without color. Deleted-line comments carry the old side, not a fictional current file line. Hunk headers are navigable. Binary/submodule/mode-only changes receive summaries and whole-file comments. Large-diff limits show `More omitted` with a bounded next-page action.

Comments can appear inline and in the on-demand collection overview, referencing one shared draft object. `All files` is a local pane action when the user needs full-file inspection; it does not change the global workbench. Provider review snapshots remain separate context files until an explicit local-revision match exists.

`Show terminal view` displays the original Reviewr TUI. If GUI drafts exist, show `Your Cockpit comments stay saved here; the terminal view has its own state.` Do not export/import private TUI comments or add a bridge. Switching back restores Cockpit's own selection and drafts.

## Project setup

Entry points: Spaces `New task Space…`, an existing Space context menu `Attach context…`, and a task-level CLI/setup operation. Keep ordinary Herdr `New Space` available for simple terminal use. A pasted provider URL can prefill setup but never bypass local repository selection.

1. Repository and task. Search already-discovered local repositories. Optional issue/review URL or manual task name. Show provider capability and repository match. A mismatch keeps both values visible and offers selection of the matching local repository. No remote clone.
2. Worktree. Create branch worktree or open existing checkout. Show repository/base, proposed branch, destination, and label; explicit values override suggestions. Existing checkout selection marks it borrowed. Surface branch/path collisions before mutation.
3. Context. List explicitly selected source downloads and optional local reference snapshots. Show local repo revision/dirty status and exclusions. Reflink/copy is a compact storage detail, not a prominent user decision. Optional `Open Context pane` launches the actual supported file-viewer pane in this tab. Agent launch remains manual.
4. Review. Show exact checkout and companion paths, create/open ownership, selected downloads/reference repos, context pane placement, and which new terminals receive context environment. The initial Herdr-created worktree root pane cannot be retrofitted; default to offering a clearly named new context-aware terminal rather than silently replacing it.
5. Progress. Show current step and durable result. Successful work remains usable when a secondary source fails. Retry addresses only the failed step/resource. Closing the dialog keeps the operation discoverable in its Space; it does not cancel or remove work.

Use a bounded dialog/overlay integrated with the workbench. Its steps are navigation inside setup, not permanent product modes. A future settings screen is optional; startup configuration remains a file.

## Source overview

The Context toolbar's `Sources` action opens a pane-local source list. Each row has kind, canonical title/ID, provider/local repository, checked/fetched timestamp, status, and actions valid for that source. Distinguish up to date, changed, never checked, refreshing, partial, unavailable, removed at source, and locally modified conflict.

Actions are Add source, Refresh, Review changes/conflict, Retry failed, and Remove from this companion. Removing a source never deletes its origin or the shared central cache. A user-edited managed file is retained and cannot be silently overwritten. Source refresh is explicit in the main loop; no background provider scheduler is implied.

## Close, remove, and recovery

`Close pane` affects that real Herdr pane/process. `Close Space` ends its processes and keeps the checkout/context on disk. `Remove task worktree…` is a separate destructive operation listing the exact owned checkout and companion. Shared caches, primary repositories, borrowed checkouts, and reference origins are excluded.

The removal preview performs fresh Herdr provenance and local Git dirty/untracked checks. Dirty or unknown state blocks the default removal. Force, if exposed at all, is an explicit separate choice with renewed exact-resource confirmation. A changed resource invalidates the reviewed removal plan.

Recovery has a compact list of partial setup, missing companion, orphaned context, stale source, unbound renderer, and uncertain paste receipts. Each action states its concrete effect. Reattachment validates current provenance; cleanup only touches owned paths. A missing plugin remains a terminal/capability issue and must not make context data disappear.

## State and error matrix

| Trigger | Visible result | Recovery and retained state |
|---|---|---|
| No companion | Empty Context with `Attach context` | Existing Space/terminals remain usable |
| Extension detection ambiguous | Ordinary terminal with optional Render as action | No automatic title-based replacement |
| Provider missing | Source row `Unavailable` | Read last snapshot or add local files |
| File loading/unreadable | Local loading/error under correct breadcrumb | Choose another file or retry |
| Diagram invalid | Code block plus error | Read/comment original source |
| Source changes under comment | Draft `Source changed` | Reselect, remove, or explicitly keep captured revision |
| Agent absent/moved | Target unavailable | Keep drafts; choose a valid same-tab agent |
| Paste rejected | Inline rejection | Fix target/capability and retry explicitly |
| Paste outcome unknown | Persistent receipt | Inspect before deciding to repeat |
| Pane moved to another tab | Same GUI/drafts in authoritative new location | Clear old paste target |
| Pane moved to another Space | Root association mismatch notice | Explicit reattach or retain original-context read-only view |
| Herdr disconnect | Last state marked disconnected | Restore snapshot/surface; no pane deletion |
| Backing process changes | Terminal renderer restored or detection notice | Explicitly select renderer if appropriate |
| Setup partial | Steps show exact completed effects | Resume failed step or review cleanup |
| Removal partially completes | Orphaned companion receipt | Retry only companion cleanup |

## Keyboard and verification

Preserve Herdr magic escape and existing navigation; add context/review shortcuts in the owning keymap module with GUI-focus guards. Define discoverable actions first, then bind keys after checking current Herdr/Cockpit conflicts. Source selection, comment save/cancel, tree navigation, menu actions, and payload review must all work by keyboard. Escape closes the innermost transient UI after Herdr's higher-priority escape contract is handled.

Mock validation checks layout and simulated interactions only. Implementation acceptance uses real `cockpit serve` and native WebKit with disposable sessions. It must observe extension detection, exact pane geometry, visible-only renderers, no GUI-to-TUI input leakage, shared terminal surface continuity, source line fidelity, read-only Git/files, and actual paste-without-submit behavior. Compare against the original Herdr TUI at normal and minimum window sizes.

## Second interaction checkpoint

After the initial plan commit, [the consolidated workflow lab](mocks/workflow.html) tests the entire daily loop. [Checkpoint notes](10-interaction-checkpoint.md) specify candidate shortcuts, focus boundaries, action targets, and the proposed direct explicit paste default with optional preview. These are discussion proposals; the established pane-replacement architecture remains unchanged.

### Second-checkpoint feedback

The user likes the quick interactions. Replace the always-present comments panel with the pane-header count and requested overview. Render unsent range comments near their source lines and whole-file comments below the file; rendered documents may group that file’s comments below its content. Pane/ellipsis button styling and source setup feel awkward and remain a separate visual/interaction refinement, not approved final UI. Preserve the efficient comment loop while revisiting those surfaces.
