# Campaign inventory

Inventory date: 2026-09-20. Source baseline: `6f6222b74e4f552ce697e61364cf653f4b6be29f`; local main was clean and matched GitHub main. GitHub remote: `nnexai/cockpit`; the separate origin remote is not the GitLab fixture. Re-inspect the worktree at execution start; these are dated facts, not immutable runtime assumptions.

## Scope selected by the user

Fix known issues, implement missing `glab` integration, and make existing daily-use functionality stable: only genuine errors, good performance, clean scrolling, reliable interaction and preserved work. The user then requested an orchestrator/subagent-ready, individually trackable task package. This package does not implement product changes.

GitLab issues and standalone merge requests are the selected provider addition. GitHub PR and Jira expansion from #6 remain explicit deferred tasks, not silently implemented or prerequisites for GitLab. Existing GitHub issue/Tea support must not regress. All other open issues are accounted for, including platform-specific ones. Legacy inline-browser migration, parked terminal graphics, remote-access/auth products and new generic frameworks are excluded.

## GitHub issues

Fetched all 10 open issues and all 3 closed issues, including open-issue comments. No tracker state was changed.

| Issue | Evidence at baseline | Owning tasks / disposition |
| --- | --- | --- |
| [#13 Herdr 0.9.1 compatibility](https://github.com/nnexai/cockpit/issues/13) | Issue reports compatible protocol 22/schema 1 rejected by exact version. `crates/cockpit-herdr/src/cli.rs::inspect_status` still rejects versions other than 0.9.0. Described local patch is not present here. | TERM-01 |
| [#12 cached terminal failures](https://github.com/nnexai/cockpit/issues/12) | Recorded disposable Linux reproduction on this baseline: 10 tabs, 60 rapid switches, hidden cached hosts attach and paint errors. `App.tsx` retains projections; `TerminalPane.tsx` attaches mounted panes. Server visibility gate is correct. | TERM-02, SYNC-01, TERM-03, PERF-01 |
| [#11 browser page/annotation loss](https://github.com/nnexai/cockpit/issues/11) | Recorded Close/Open resets page and Notes 1 to 0. BrowserPane lifecycle/draft selection resets require reconciliation; saved capture recovery is not all unsent-draft recovery. | WEB-01, WEB-05 |
| [#10 pointer clicks not delivered](https://github.com/nnexai/cockpit/issues/10) | Recorded failed first/repeated clicks; exact dispatch cause remains to trace. BrowserPane silently returns on stale pointer/wheel outcomes. | WEB-02, WEB-04 |
| [#9 exploratory findings](https://github.com/nnexai/cockpit/issues/9) | Findings 1/2 overlap #12, 3 overlaps #11, 4 overlaps #10. Finding 5 is initial Element-pick readiness; 6 is missing paired Playwright-core configuration. Broader input/resource evidence remains unclaimed. | TERM-02, WEB-01/02/04/05/06/07/08, PERF-01; retain umbrella until every finding has a disposition |
| [#8 blurry resized browser](https://github.com/nnexai/cockpit/issues/8) | Issue describes commit 0c36914 and a gist patch that could not be pushed. Current helper resize branch updates viewport but does not restart the screencast capture bounds. Do not equate issue wording “fixed” with integration. | WEB-03 |
| [#7 issue opening mutates setup](https://github.com/nnexai/cockpit/issues/7) | Reported unexpected Space/worktree creation and macOS companion failure. SetupDialog submit plans then immediately starts; no separate exact-plan approval. `project_store.rs::publish_companion_no_replace` explicitly rejects non-GNU/Linux. The precise original click path still needs an operation trace. | SETUP-01, SETUP-02, FLOW-01, NATIVE-02 |
| [#6 GitHub/GitLab/Jira providers](https://github.com/nnexai/cockpit/issues/6) | Provider factory has gh/tea, no glab. Current GitHub adapter is issue-only. Source origin identity only accepts two repository path segments. MR must be valid without issue/Jira. | GLAB-01/02/03/04; LATER-GHPR and LATER-JIRA deferred; never close whole issue for GitLab alone |
| [#5 stale macOS installed bundle](https://github.com/nnexai/cockpit/issues/5) | Proposed patch not integrated. `install-native.py` builds with --no-bundle and uses binary receipt schema 2; no app-bundle installation. | NATIVE-01, NATIVE-02 |
| [#3 annotation toolbar/feedback polish](https://github.com/nnexai/cockpit/issues/3) | Legacy extension patch partly superseded by inline cutover. Compact controls, freehand simplification, point stripping from feedback and Playwright bugfix-version compatibility have current code. Exact requirement reconciliation is still necessary, including bounded iterative simplification and pointer cleanup. | WEB-05, WEB-07; no blind old-extension patch application |

Closed regression anchors:

- [#1](https://github.com/nnexai/cockpit/issues/1): request storms/terminal lag. Keep snapshot-only decoration, bounded resync, resize coalescing and no idle mouse flood.
- [#2](https://github.com/nnexai/cockpit/issues/2): macOS plugin/root verification and native windows. Keep current process-generation/root authority and latest window defaults, not the old patch's defaults.
- [#4](https://github.com/nnexai/cockpit/issues/4): terminal glyph/cell pitch. Retain one-cell semantics and Nerd Font rendering; do not repin tests to arbitrary font constants instead of visible behavior.

These are regression requirements, not claims that the closed bugs have reappeared.

## GitLab fixture and authorization

Fresh read-only CLI probe: `glab 1.118.0 (570955d42)`. Authenticated GETs succeeded for the user-designated [nnex.ai/integration](https://gitlab.com/nnex.ai/integration), ID `86672117`. It is initialized, issues/MRs enabled, current credentials have project access level 50 (Owner), and only protected branch `main` was present. This is access evidence, not blanket authorization to mutate any resource.

The user explicitly permitted creating test issues. One was created and read back:

- Issue IID `1`, project ID `86672117`, state opened, `issue_type: issue`.
- Title: `Cockpit glab integration fixture — 2026-09-20-a17b`.
- Marker: `cockpit-glab-2026-09-20-a17b`.
- Returned canonical URL: <https://gitlab.com/nnex.ai/integration/-/work_items/1>.
- API read: `GET /projects/86672117/issues/1`.
- Description has Markdown, inline code, Unicode and explicit import/refresh/ownership intent.

This proves CLI access and fixture creation, not Cockpit integration. The work-item URL is an important real parser fixture: accept verified issue work items without coercing other work-item types. No MR exists at inventory. Branch/MR write permission has not been recorded; GLAB-04 must obtain a designated existing MR or explicit authorization before creating one. Do not mutate protected main or branch protections. Record fixture mutations and close only the owned issue after all tests finish.

A local checkout whose primary origin matches the test project is required by the source-authority contract. RUN-01/GLAB tasks must discover or explicitly prepare an authorized disposable fixture checkout; the Cockpit repository's GitHub/internal remotes do not satisfy that authority. Test setup must not weaken origin checks or turn product code into an automatic clone workflow.

## Current integration seams

- Provider construction: `crates/cockpit-providers/src/lib.rs`; gh/tea implementations live beside it.
- Artifact URL parsing: `crates/cockpit-core/src/repositories.rs`.
- Origin/instance authority, identity, fetch/cache/freshness/materialization: `crates/cockpit-core/src/sources.rs` and `context_assets.rs`.
- Repository matching and review source-branch defaults: `crates/cockpit-core/src/projects/defaults.rs`.
- Plan/start/recovery/ownership: `projects.rs`, `project_store.rs`, project teardown modules.
- Generic source capabilities already include Issue, IssueComments, Review and Wiki; ProjectArtifact separates provider identity from generic issue/review kind. Preserve equivalent typed distinction rather than require a parallel task-source model.
- Setup UI: `src/app/projects/SetupDialog.tsx`; Context resources/source UI: `src/app/context/SourceImport.tsx`, `ContextResources.tsx`.
- Shared client contract with browser HTTP and native commands: existing project/source endpoints need no glab-specific transport merely to add an adapter.

Use full project namespace identity and configured-host authority. An MR SHA alone is not a sufficient freshness token for comments/title/description updates. Authentication stays in CLI-owned configuration; no credentials in snapshots, generated environment or evidence.

## Additional source-review candidates

These are investigation targets, **not fresh runtime reproductions**. Repair only after confirming the affected contract, or retain evidence that it already holds.

| Candidate | Source starting point | Task |
| --- | --- | --- |
| Obsolete session handshakes and command errors can outlive selection | `src/client/browser.ts`, `native.ts`, `App.tsx`, `BrowserPane.tsx` | SYNC-01 |
| Terminal close/send timing must not throw or route stale input | `TerminalPane.tsx` input/resize/cleanup | SYNC-01, TERM-03 |
| Review source pagination lacks uniformly retained cancellation/identity; same-path refresh restoration deserves proof | `ReviewViewer.tsx`, `ReviewPane.tsx` | VIEW-01 |
| Long Context/Review rendering and resource-list overflow need measurement | `ContextViewer.tsx`, `ContextResources.tsx`, `SourceImport.tsx` | VIEW-01, GLAB-03, PERF-01 |
| Initial Element inspection returns early without cursor metadata | `BrowserPane.tsx::inspect` | WEB-02 |
| Decoded image and fallback object-URL cleanup on failing decode/present paths | `framePresenter.ts::decodeActive` | WEB-06 |
| Browser boundary input jobs can queue without a clear cap | `BrowserPane.tsx::enqueueInput` | WEB-06 |

## Evidence limitations and authorities

The initial inventory ran GitHub queries, source/planning inspection, bounded GitLab CLI/API reads, and the authorized fixture issue create/read. It did not run a new Cockpit browser/native smoke, test suite, or Herdr reproduction. Existing issue reproductions are accepted evidence; future safe reproductions establish fix acceptance.

`NEXT_PHASE_PLAN.md` is complete history. The latest inline handoff records focused browser open/frame/region/hide/show/navigation and Linux-native startup, but explicitly does not claim broad A01–A25, native input/decode, security or sustained performance. Older Context/next-level summaries can say “planned” for now-existing code; do not restart those whole implementation plans. Preserve latest domain decisions and verify the current implementation.

Use [the campaign acceptance map](ACCEPTANCE.md) to cover missing proof. No prior screenshot, startup lifetime or green build establishes clean scrolling, retained drafts, correct first input or stable long-running resource use.
