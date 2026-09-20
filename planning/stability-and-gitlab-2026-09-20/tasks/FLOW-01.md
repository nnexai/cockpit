# FLOW-01 — Verify durable local development workflows

## Outcome
The existing local workflow is complete and durable from project setup through Context/Review use, comments, exact same-tab paste, and safe teardown. It must work for a plain directory, nested borrowed directory, and owned worktree without confusing ownership or deleting dirty/borrowed data.

## Evidence and starting points
- Baseline, dependencies, locks, and issue anchors are authoritative in [../tasks.json](../tasks.json); this task follows VIEW-01, SETUP-02, and SYNC-01.
- Regression anchors: [GitHub #2](https://github.com/nnexai/cockpit/issues/2) and [GitHub #7](https://github.com/nnexai/cockpit/issues/7). Closed anchors do not assert a current failure.
- `src/app/App.tsx` projects parent/child worktree Space hierarchy, opens Setup/Teardown, and wires Context/Review/browser/terminal actions.
- `src/app/projects/SetupDialog.tsx`, `src/app/projects/TeardownDialog.tsx`, and `src/app/projects/TeardownRecoveryPanel.tsx` own setup, refusal, and recovery presentation.
- `crates/cockpit-core/src/projects.rs`, `project_store.rs`, `project_teardown.rs`, and `context_assets.rs` own directory/worktree association, manifest, dirty checks, pending source intent, and cleanup safety.
- `src/app/context/ContextViewer.tsx`, `ContextResources.tsx`, `SnapshotImport.tsx`, `SourceImport.tsx`, `CommentDrafts.tsx`, and `CommentPasteControls.tsx` own source views, snapshots, imports, durable comments, and paste receipts.
- `CommentPasteControls.tsx` explicitly says paste fills the selected agent input without submitting; preserve its receipt/unknown-outcome semantics.
- Use [../ORCHESTRATOR.md](../ORCHESTRATOR.md) for fixture ownership, resource guards, and evidence/commit rules.

## Changes
- Complete the existing local workflow end to end; repair only behavior reproduced in the scenarios below, without introducing a new project abstraction.
- Cover setup and association for a plain directory, a nested borrowed directory, and an owned worktree; show parent/child Space hierarchy and correct branch/path identity.
- In Context, exercise source, Markdown, Mermaid, media, search, source import, and snapshot import; verify source identity and revision remain clear after refresh/reopen.
- In Review, exercise staged, unstaged, branch, and untracked changes with file selection, source/diff navigation, and durable line/file comments.
- Exercise comment retention through refresh/revision changes, explicit save/discard, and same-tab paste to the selected agent with exact payload and no Enter/submit.
- Exercise real failure retention: failed comment save/paste/source operation preserves user text and reports the actual error or unknown outcome for reconciliation.
- Teardown must refuse dirty or borrowed resources safely, explain the refusal, retain recovery information, and allow owned clean worktree teardown with recorded cleanup.
- Add a workflow/reliability review because this crosses setup, source identity, comments, paste, and teardown boundaries.

## Non-goals
- No remote GitLab/GitHub/Jira mutation; this task is local workflow proof.
- No new credential store, generic provider framework, or legacy extension pane.
- No forced deletion, main-branch mutation, borrowed-directory takeover, or automatic paste retry.
- No claims based solely on existing unit fixtures or closed issue status.

## Acceptance
1. A plain local directory opens as the intended Context source without being treated as an owned worktree; cleanup leaves it intact.
2. A nested borrowed directory resolves its parent/child identity, displays the correct Space hierarchy, and teardown refuses borrowed/dirty removal with actionable recovery.
3. An owned worktree can be created/associated, reviewed, and safely torn down only when clean and owned; manifests and pending intents remain valid after reopen.
4. Context source, Markdown, Mermaid, media, search, source import, and snapshots all work with revision/path identity visible and no stale result after cancellation.
5. Review staged, unstaged, branch, and untracked changes support file/hunk navigation and durable line/file comments across refresh and pane revisit.
6. Same-tab paste uses the prepared exact payload, targets the selected eligible agent, inserts text without pressing Enter, and records accepted, refused, or outcome-unknown receipts.
7. Failed saves, paste responses, and source operations retain user text and the real failure state; retry or reconciliation does not duplicate a dispatched paste.
8. Browser/native surfaces show the same workflow result, and Herdr TUI hierarchy/agent ordering remains authoritative throughout.

## Verification
Create uniquely named disposable local fixtures for each directory/worktree case and a controlled agent/session for paste. Run the actual browser and native workflows, capture paths, ownership/dirty state, manifests, source/review/comment payload hashes, paste receipts, and teardown results. Deliberately make a borrowed and an owned fixture dirty, and disconnect a dispatched paste response to prove refusal and unknown-outcome retention. Compare Space/tab/pane/agent order with the TUI oracle. Clean every recorded fixture only after evidence is captured.

## Handoff
The evidence record must include fixture ledger, before/after filesystem state, browser/native captures, review/comment/paste receipts, refusal and recovery messages, TUI comparison, and cleanup. Link the durable evidence plus real implementation commit in the task ledger. A docs-only or component-test pass is insufficient for this runnable workflow.
- Create each fixture under an owned temporary root and record repository, branch, checkout, companion, session, and worktree identities before opening Cockpit.
- For the nested borrowed case, keep the parent repository outside the task's cleanup ownership and prove no setup or teardown command writes into it unexpectedly.
- For the owned worktree case, record the ownership manifest and pending source intent before and after a source write or recovery interruption.
- Exercise reopen after a refresh and after a process/session restart so source identity is not only an in-memory property.
- In Context, select a nested search result, open Markdown and Mermaid content, load media, import source, and import a snapshot with working changes.
- In Review, select staged, unstaged, branch, and untracked fixtures and verify line/file comments remain associated with the right revision and path.
- Edit a comment, switch source/review, refresh, and return; retained prose must remain visible and must require explicit current-source confirmation when revisions differ.
- Prepare a same-tab paste, change the selected agent before send, and ensure the prepared target/payload is refreshed rather than sent to the old target.
- Simulate a lost paste response and verify the receipt says outcome unknown, retains comments, and requires explicit duplicate-risk acknowledgement.
- Make both borrowed and owned worktrees dirty; refusal must leave all files and manifests intact.
- Complete a clean owned teardown, then inspect the recorded cleanup list and confirm no borrowed path or active session was removed.
- Repeat one representative path in browser and native, including a failure that is visible in both surfaces.

The evidence should distinguish:
- code already enforcing ownership, revision, and receipt invariants;
- repaired workflow behavior;
- runtime behavior that could not be exercised because a fixture or platform was unavailable.

Do not claim the workflow is durable from a single successful first-run setup; reopen and failure retention are required.
- Include exact same-tab target identity, payload hash, operation ID, and receipt state in paste evidence.
- Record every path that was intentionally refused and verify its bytes remain unchanged afterward.
- Preserve durable comment text in evidence while omitting credentials and unrelated repository data.
- The handoff must identify which fixtures were deleted and which borrowed paths were intentionally retained.
- A missing native or worktree fixture blocks its criterion rather than being silently omitted.
