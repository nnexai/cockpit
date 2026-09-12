# Create a task Space with its context ready

Cockpit's central workflow is selecting a task and receiving a real worktree with prepared context. The [issue-4 runtime trial](../workflow/issue-4/README.md) reached that state only after a manual GitHub download and terminal navigation. Implement this workflow through the real application. Do not work on the referenced issue itself.

## Own and coordinate

Inspect `crates/cockpit-core/src/projects.rs`, `repositories.rs`, `sources.rs`, `sources/hydration.rs`, `context.rs`, `crates/cockpit-providers/src/lib.rs`, `tea.rs`, the project/source protocols, and `src/app/projects/SetupDialog.tsx`. Coordinate the form with packet 10, viewer opening with packet 06, and shared source/comment identity with packet 07. Preserve the transport-neutral client and authoritative Herdr topology.

## Required behavior

The local repository remains primary. Starting from its selected Space should preselect that repository and show the resulting parent association. Accept its GitHub issue URL in the primary task flow. Validate supported host, provider availability, authentication, and issue identity immediately. Offer a useful default task label, branch, and destination while retaining edits. Do not silently switch repositories or claim that an arbitrary URL can be imported.

Implement a real GitHub provider alongside the existing Tea provider. Use the owner's configured/authenticated local tooling through the provider boundary. Download the issue body and comments as static companion assets with canonical URL, timestamps, provider identity, and refresh provenance. GitHub issue #4 currently fails before creation; configuring the `gh` executable alone is insufficient because the provider factory only constructs Tea adapters.

Review actual effects once: linked worktree, branch/base, resulting parent Space, companion, requested context assets, and the intended initial panes. Keep configured repository-action consent. After confirmation, create resources through Herdr and the existing Cockpit operation ledger. Fetch the approved context as part of the operation. Opening Context must use the reviewed companion binding directly; it cannot require manually changing the foreground cwd or adding an unrelated terminal.

Distinguish `worktree created`, `context preparing`, and `ready` using the existing operation/state contracts or an explicit reviewed extension. A failed download keeps the existing worktree and companion visible and offers retry for that source. Retry cannot create a duplicate worktree or overwrite local context edits. Keep useful terminal access while recovery is pending. Do not start an agent or submit a prompt unless separately requested.

Check installed Files/Review capabilities early. A missing plugin needs a local explanation and a supported recovery path. A ready claim should describe the capabilities actually available. Avoid permanent status cards or a toast for every completed substep.

## End-to-end acceptance

1. Use the real Cockpit checkout or an isolated clone, a named disposable Herdr session, and a parent `cockpit` Space. Record prerequisites separately from user-flow actions.
2. Select Cockpit issue #4. Validate it before the final effects step; create its child task Space with a real linked Git worktree and companion containing the downloaded body and comments.
3. Verify Git common-directory/worktree evidence and Herdr's actual grouping. Open the issue in Context directly from the resulting worktree Space without shell commands, pasted paths, or manual filesystem repair.
4. Make two harmless tracked changes and one untracked file. Open graphical Review, annotate a new-side line and a whole file, generate a preview, and switch files/tabs. Refresh retains the selected file, anchors, and saved drafts. No agent delivery is required.
5. Close the task Space. Keep the parent intact. Show that closing and deleting resources remain distinct; retained worktree/context/drafts must be discoverable through the existing recovery/open flow. Reopen and verify draft recovery.
6. Repeat with a provider error after worktree creation and a missing viewer prerequisite. Preserve resources, inputs, and retry identity. Record timings for issue resolution, creation, context readiness, first useful diff, file switching, and comment save separately.

Use desktop and all three portrait sizes. Save screenshots and authoritative receipts at each transition. Do not call manual downloads or direct filesystem repairs a Cockpit pass. Do not remove source bounds without bounded continuation and exact revision handling.

## Delivery

Land the provider capability and setup integration as separate complete, verified increments if ownership requires it. The final acceptance is the full workflow above. Delete replaced UI during cutover, preserve unrelated work, and commit each completed scope. Keep failed or unavailable coverage explicit. This document is a future implementation packet; the planning pass does not implement it.
