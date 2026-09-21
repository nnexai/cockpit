# SETUP-01 — explicit reviewed setup

## Accepted plan

Main/Astra accepts this plan on 2026-09-21 against `6593cf4ecba87687c809ee9f3d2a295c757db787`, with current owned SYNC-01/VIEW-01/WEB-07 changes preserved. TERM-01 dependency is complete. Original `tasks/SETUP-01.md` criteria 1–8 remain unchanged.

Inspected SetupDialog defaults resolution, retained receipt, PlanDetails and submit; core ProjectService plan/start. The concrete defect is submit obtaining a plan then immediately setting dispatchRef and calling startWorkspace. Existing DTOs already separate plan/start and support generations, recovery and receipts. Keep that architecture.

Decision: split read-only plan generation from an explicit approval action. Display the authoritative plan's source/provider, repository, branch/base, label, checkout/companion paths, hydration/configured actions and warnings before Start. Editing any relevant input invalidates the plan/token. Start uses only the reviewed operation ID/generation. Retain dispatch/receipt across close, failure and reopen; duplicate click or uncertain response cannot cause another create. Preserve explicit-field precedence in delayed defaults and path-only Open without Git/provider requirements. Never reintroduce trust_repository overrides or action-consent checkboxes.

Inspect the existing pre-mutation provider/origin/path/action validation and make it reject stale authority/effects rather than silently rewrite the reviewed plan. If fresh validation rejects a changed plan, return to review and obtain/display a new authoritative plan for approval; do not automatically execute it. Allowed metadata caches/plan journals are not resource creation. Preserve existing partial-source recovery and owned-resource accounting. No new generic journal/protocol framework.

## Ownership and dispatch

Worker owns `src/app/projects/SetupDialog.tsx`, its existing tests and `setup.css` only if needed; `crates/cockpit-core/src/projects.rs` plan/start pre-mutation validation; relevant `projects/defaults.rs`, `repositories.rs`, `project_store.rs` and directly affected tests only where required. No GitLab implementation, transport cancellation changes or generated DTO edits. Run LSP references before exported TypeScript changes.

Locks are refined to `setup-ui` and `core-projects`: no App.tsx edit is assigned. Main exclusively owns any necessary App integration after SYNC-01 releases that file. This removes only an unnecessarily broad app-shell lock, not acceptance scope or dependencies. NATIVE-01 owns installer files, independently. WEB-07 owns browser launch/config; do not edit its files or docs. No validation, formatters, services, fixture mutations or commits during the writing wave.

## Verification and preserved acceptance

Per user OBS-011, delegate one minimal focused check after integration and defer long matrices to final acceptance. Required final coverage remains: (1) inspect/change/cancel source creates no user resources, allowing owned caches/journals; (2) exact effects visible before explicit approval; (3) delayed defaults and changed plans cannot override edits/approval; (4) provider/auth/identity failure preserves form and creates no resources; (5) repository Create and plain/nested path-only Open preserve configured actions and unset trust override; (6) duplicate click/close/reconnect/lost response retain one operation ID and distinguish unknown outcome; (7) partial failure recovery retries only owned incomplete effects; (8) matching browser and Linux-native behavior. Lifecycle/ownership/UI review is required before final runtime acceptance.

Use only Main-authorized uniquely named disposable Herdr/config resources under `/tmp/csg-a3e9b950`, with guarded HOME. No remote writes, borrowed-directory deletion, installed-app or user-session mutation. Runtime is not assigned to the implementation worker. Missing shared DTO capacity or a necessary App change is reported to Main for serialized integration.

Implementation authorization only; status remains exclusively in tasks.json. No acceptance/commit claim.

## Implementation handoff — 2026-09-21

ExplicitSetup returned SetupDialog, stale-operation regressions, and projects.rs preflight changes; no runtime/tests/builds ran. Implementation is not acceptance. The core-projects writing reservation passes to GLAB-01/Main against this settled interface, while setup-ui remains reserved. A focused read-only review covers approval invalidation, stale/unknown operation retention, and preflight-before-mutation; Main integrates any projects.rs repair before source-metadata changes. Original criteria remain for the final round.

## Lightweight delivery evidence — OBS-014

The focused `TerminalPane.test.tsx` and `SetupDialog.stale.test.ts` run passed (35 tests total across the two files). Typecheck, frontend build and the updated gateway build passed.

One actual owned gateway smoke passed: review displayed the operation identity, branch, checkout, companion and effects while the sidebar remained at its original one Space; explicit **Start setup** completed operation `74b12ce7-c098-4138-b969-c7465f35f196`. The persisted authoritative receipt is completed, generation 11, with workspace `w3`, tab `w3:t2`, pane `w3:p3`, and the exact reviewed owned worktree/companion paths. Main inspected the before/after screenshots and receipt. See [SETUP-browser-evidence.json](SETUP-browser-evidence.json), [review](SETUP-review.png) and [result](SETUP-result.png).

The created fixture is retained for the immediate source-resource UI check, then Main-owned cleanup. This was a real shared-frontend setup path, not a native or exhaustive fault/reconnect matrix. Under OBS-014 those deeper scenarios are explicit follow-up checks rather than a reason to delay this working increment.
