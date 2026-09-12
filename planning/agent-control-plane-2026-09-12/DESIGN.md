# Proposed interaction decisions

These are proposals for discussion and implementation packets. They do not change the repository's authoritative `DECISIONS.md`.

## Preserve the Herdr model

Keep the current Session → Spaces → tabs → panes structure and existing agent order. Render exact supported agent status text alongside the existing glyph. Improve its readability without inventing categories or priorities. Use supplied names, pane titles, Space labels, and tab labels. Do not present a pane title as a reported task or invent an owner field.

The main pane canvas retains Herdr's rectangles and pane count at every viewport. The mock keeps the two equal terminal panes in the source capture. A closed sidebar gives those panes more space; it cannot make two 240 px panes equivalent to a desktop terminal. Existing Herdr zoom remains an explicit operator action. No automatic stacking, pane hiding, zoom, or pane reordering is part of the responsive proposal.

Terminal text and rendering remain Herdr-backed. Clipboard, pointer input, focus, ownership transitions, and browser/native integration are Cockpit responsibilities that can be fixed without changing terminal content.

## Refine the shell in small steps

The initial visual choice is a 224 px desktop rail, retaining the current width. Use a consistent 10 px inset, 32 px Space rows, and roughly 52 px two-line agent rows. These are mock starting measurements, not hard requirements. Agent location occupies the first line; the existing name and exact status share the second. Avoid per-row cards, extra task summaries, and duplicate badges.

Place Agents immediately after the visible Spaces content with a small separator gap. Let the two lists scroll independently under bounded headers when they grow. Do not allocate most of the rail to two Space rows and push Agents to the bottom. Long names retain their complete accessible label and an inspectable full value.

Add a reversible collapse control and a keyboard-operable resize separator. Persist only the user's rail width and collapse preference. Fit the existing canvas through its current sizing path. Sidebar state must never become pane layout state.

Compare two responsive choices:

| Choice | 1440 px | 800 px | 600 and 480 px | Tradeoff |
| --- | --- | --- | --- | --- |
| A · Refined rail | Persistent, resizable rail | Keep rail open unless manually collapsed | Overlay drawer | Fewer navigation clicks at 800, but less terminal width |
| B · Responsive drawer | Same rail | Closed-by-default overlay drawer | Same drawer | One extra click for navigation; all canvas width is available while closed |

Recommend B as the initial portrait direction. Preserve any deliberate user collapse choice. Opening or closing the overlay does not resize the canvas behind it. Escape restores the prior input target; a confirmed resource selection closes the drawer and follows the existing focus transaction. A rejected selection keeps its target and failure available. Resizing across the breakpoint must not orphan DOM focus in hidden content.

Pane titles need a readable line box and contrast. The current 16 px strip is a specific candidate for a 24 px strip. Reserve border and icon space so hover and pending states do not move content. Keep terminal font metrics unchanged. Use compact actions with full accessible names and larger touch hit areas where appropriate.

## Make focus automatic and feedback local

Clicking a pane, selecting its agent, or navigating with the existing Herdr-compatible keys requests the corresponding focus and writable control. Successful control requires no second click and no toast. Ordinary typing goes through the established input path after ownership confirmation.

Keep semantic focus, DOM focus, and writable ownership separate internally. Passive live updates, hover, and a reconnect must not cause two clients to take control repeatedly. Reproduce the normal selection behavior against the Herdr TUI before changing arbitration. Preserve the first intended input exactly once when acquisition is delayed, or retain a visible retry path if it fails. Never deliver it to the previous pane.

Bind queued input to the existing session epoch, pane ID, and focus request token. A late confirmation for A must not flush A's input after the user has selected B. Invalidate or retain the bound action safely on ownership loss, reconnect, and retry; never relabel it as input for a new target.

A small pending indicator in the pane border is sufficient for routine acquisition. Expose an accessible state label. If acquisition fails, show an icon with an inspectable reason and a nearby retry action. Retain visible text for consequential failures, unknown delivery outcomes, and destructive setup effects. This is not a blanket conversion of all errors to tooltips.

Replace the global delayed-focus text toast with local feedback. Restore focus after menus and dialogs without recentering scroll or selecting a different terminal. Keep the user's selection and draft intact when a background update arrives.

## Put actions on their resources

Right-click or the keyboard context-menu action on a Space, tab, or pane acts on that resource, even when another pane is focused. Use the same action definition for a small local overflow button, keyboard access, and the right-click menu. The generic top-bar Pane button can be removed after those paths are usable at touch and keyboard sizes.

Dragging shows a source and insertion target, then a pending result until Herdr confirms the reorder. Bind a drag to stable source and neighbor IDs and the hierarchy view that started it. If the view changes before drop, revalidate or cancel the gesture. Use existing API capabilities; do not invent a server-side conditional mutation. The final order comes from Herdr. A rejected drop restores the confirmed arrangement and shows the failure at the attempted target. Do not implement a competing local sorting policy.

Commands becomes a bounded, searchable action list with existing shortcuts aligned beside their actions. Keep Files, Context, Review, and browser feedback easy to find. Direction choices belong to the same action row or a clear second step, not duplicated long labels. Separate optional keyboard reference from actionable entries. Keep capability reasons accessible on unavailable commands. Do not add invented shortcuts that conflict with Herdr prefix handling, terminal input, browser keys, or IME input.

## Redesign Cockpit-owned interfaces

Files, Context, and Review follow [one viewer design](VIEWERS.md). Use shared source typography, line-number metrics, file rows, controls, insets, and comment editors. Distinct document roles can use distinct styles; the same role cannot change size between panes. Narrow widths collapse navigation and reduce insets instead of shrinking text.

Files and Context default to reading. A single toolbar carries the root, file search, and relevant document actions. Tree, document path, and source metadata each appear once. Sources, imports, snapshot management, and draft overview are disclosures or focused overlays, not permanent competing panels. Use the same reader where the existing binding permits it, without erasing the difference between a repository and companion context.

Use pane width, not only window width, to collapse the file tree. Preserve document scroll, query, root, selected line, and draft on return. Large files and directories need incremental navigation and explicit partial results. A limit must not masquerade as an empty file or the end of a directory. Loading more content must retain exact source identity and line anchors. Continuation needs a revision-bound cursor or equivalent validated read contract. A file or directory change between pages marks that continuation stale; it cannot silently concatenate different versions. Do not remove safety bounds by reading an entire repository into memory.

Review opens useful changes promptly and loads expensive content on demand. Keep one scope selector, a changed-file list, the diff, and compact draft actions. Preserve file selection, hunk, side, scroll, and comments on refresh when the revision still matches. A stale anchor requires revalidation; it must not silently move to a different line. Benchmark the same checkout and actions against herdr-reviewr before choosing the data-loading change.

Space setup prepares a task, its real worktree, and its companion context. Show the primary local repository and task source together, followed by inputs required for create or open. Additional sources can use a disclosure. Validate supported issue identity early and preview the actual parent association, checkout, branch, companion, approved downloads, ownership, and effects. Keep worktree creation distinct from context readiness. Open prepared Context through its binding without terminal navigation. Preserve plan expiry, consent, operation identity, partial progress, and cleanup recovery. [Packet 12](tasks/12-task-context-readiness.md) defines the full issue-to-review acceptance flow.

Browser feedback shows its Space, page, annotations, and exact delivery target. Use one meaningful empty message and one browser state. Preserve pending work on errors. Accepted delivery, acknowledgement, and agent completion remain different facts. If delivery is uncertain, retain the item and require the existing duplicate-risk step before a retry.

## Make annotation mode follow toolbar visibility

Opening the annotation toolbar enters annotation mode. Closing it returns the page to ordinary browsing. Closing does not delete drafts. The extension action or a documented shortcut reopens it. Page scroll and ordinary interaction work when the toolbar is closed.

Add a Select tool for existing annotations. Selection, editing, deletion, and moving supported annotations must not create accidental new marks. In the Element tool, outline the hovered eligible page element before a click commits the anchor. Exclude the extension's own UI. Recompute the preview on scroll and layout changes; stale anchors remain visible as stale. Keep document and frame identity with the anchor. A selector match alone cannot retarget an old annotation after DOM replacement.

Tool shortcuts work only while the annotation toolbar is active and focus is outside editable controls. Show the keys on tooltips and in an on-demand reference. Escape first cancels an in-progress gesture or editor operation according to an explicit draft-preserving rule; a subsequent Escape closes the toolbar. Do not consume typing in the page's form fields or annotation editor.

## States that need explicit acceptance

| State | Visible behavior | Recovery and context |
| --- | --- | --- |
| Initial loading | Compact loader at the requested resource; no fabricated empty list | Cancel or retry if the request fails |
| Confirmed empty | One specific message, such as “No agents detected” or “No changes in this scope” | Show a relevant existing action only |
| Partial content | Show what is loaded and what remains available | Load more or narrow scope without dropping position |
| Stale | Retain the last view and mark the affected resource | Resync; preserve local drafts and user selection |
| Disconnected | Preserve the last frame; disable writes that require a live connection | Reconnect without declaring an agent failed |
| Local error | Explain the failed operation at its resource | Retry that operation; other resources remain usable |
| Recovering | Keep the last view with a compact pending indication | Clear it only after authoritative refresh, not on click |
| Control pending | Small pane-local indicator | No extra Take focus action on normal success |
| Ownership lost | Continue observation; use a discreet state indication | Natural user selection requests control; no background takeover loop |
| Delivery unknown | Keep the item, target, and uncertainty visible | Inspect before an explicit duplicate-risk retry |

No modal or toast announces normal successful focus, navigation, copying, or refreshing. Announce meaningful state changes accessibly without narrating every terminal frame.

## Reconsider the terminal-edge line

Confirm the origin of the reported right-most line before changing it. Compare a real scroll thumb with a quiet scroll affordance that appears only when useful. Remove a decorative full-height track if it conveys no position. Preserve Herdr scrollback ownership, actual wheel and key behavior, application mouse mode, selection, and terminal cell metrics. A static mock cannot establish scrollbar correctness. Packet 11 owns this comparison and its temporal evidence.

## Design review

The sequential design critique retained the refined rail/drawer candidates and rejected adding a new agent table, task model, or status mapping. It added explicit late-focus/input invalidation, stale-drag revalidation, revision-bound file continuation, user-gesture clipboard boundaries, and document/frame-bound annotation anchors. Implementation still begins with real reproduction; none of these proposals establishes the reported root cause.
