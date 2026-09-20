# WEB-01 — Preserve browser page drafts and saved feedback

## Outcome

Repair the inline browser lifecycle so presentation hide/show is non-destructive and true close preserves recoverable work. A user can annotate a real page with Region and Freehand marks plus a note, hide the browser, show it again, and continue on the same page with the same draft revision. Closing the browser never makes already-saved feedback unreadable. Persist unsent draft/editor state before close or retain the open view with an actionable save failure; discarding work requires a separate explicit user action, not an ordinary toolbar toggle.

This task resolves the bounded #11/#9 state-loss path; it does not implement legacy profile migration. Preserve Herdr Space/tab/layout authority and keep browser-local state separate from Herdr pane IDs.

## Evidence and starting points

- Baseline is `6f6222b74e4f552ce697e61364cf653f4b6be29f`; historical source review and issue reports are not fresh runtime proof.
- Issue #11: https://github.com/nnexai/cockpit/issues/11. Related umbrella findings: https://github.com/nnexai/cockpit/issues/9.
- Latest implementation handoff: `planning/inline-space-browser-2026-09-13/HANDOFF.md`; its hide/show smoke did not prove URL, draft, or unsent-work preservation.
- Architecture authority: `planning/inline-space-browser-2026-09-13/01-architecture-and-transport.md`, especially visibility/recovery and identity sections; acceptance rows A02, A18, A19, A21, A22, with A23 excluded.
- Read `src/app/App.tsx` browser hide/close actions and browser presentation ownership before changing lifecycle semantics.
- Read `src/app/browser/BrowserPane.tsx` stream rebind cleanup, `navigation`, `targets_changed`, `document_changed`, `openDraft`, and pending-capture recovery paths.
- Read `crates/cockpit-core/src/browser/drafts.rs` draft identity/revision/tombstone and pending-capture records, `crates/cockpit-core/src/browser_feedback.rs` saved capture lookup/ack, and `crates/cockpit-core/src/browser/delivery.rs` operation identity.
- Use `planning/stability-and-gitlab-2026-09-20/tasks.json` for authoritative dependencies and locks; do not duplicate mutable status here.

## Changes

1. Define the lifecycle distinction in the existing browser view contract: Hide releases frame/input presentation resources while retaining association, page/target/document identity, structured draft, and pending delivery state; Close is a separate owner action.
2. Prevent stream cleanup or navigation invalidation from silently dropping structured annotations, note text, pending save IDs, or the last known page identity before durable recovery has acknowledged them.
3. Keep geometry-bound marks stale when target/document/viewport identity changes. Cancel only the unfinished gesture; retain earlier marks for explicit review/repositioning rather than painting them on a new document.
4. On reopen, load the matching current draft and surface an explicit stale-draft recovery choice when the target/document changed. Do not guess an anchor from URL or selector alone.
5. Preserve saved feedback/image lookup and exact pending operation identity across close, owner/helper restart, and lost response. A retry must reuse the same draft/capture ID and bytes.
6. Keep close scoped to the owned browser association; do not close an unrelated CLI browser, create a second profile, or revive any legacy import/migration behavior.
7. Include a required lifecycle/durability review covering owner versus observer close, save/navigation races, and recovery UI wording.

## Non-goals

- No legacy draft, pending PNG, consumed-ID, extension-profile, or migration-journal import (A23 remains excluded).
- No redesign of Herdr layout, Space membership, feedback provider behavior, or generic persistence framework.
- No automatic replay of an uncertain navigation, save, annotation, or paste operation.
- No claim that source review or the old handoff proves this task complete.

## Acceptance

1. Browser gateway: on a disposable fixture, open a page, create Region and Freehand marks and a note, Hide, then Show; URL/title, page target, mark count, note text, and draft revision are unchanged.
2. Browser gateway: repeat after a pending or delayed save response; the draft remains visible, retryable, and tied to the original ID/bytes rather than duplicated.
3. Linux-native Tauri: the same Hide/Show scenario releases frame/input resources while hidden, restores a live frame without launching a second browser/profile, and preserves the shared durable draft.
4. True Close: saved feedback remains readable with the browser closed; unsent work is durably recoverable with a visible stale/retry state. If persistence fails, close is refused without losing the draft. Explicit discard is separate from ordinary close.
5. Navigate, reload, target switch, resize, and helper reconnect invalidate only geometry-dependent live actions. Earlier marks remain recoverable, and no stale mark is painted onto the replacement document.
6. Observer close cannot stop the owner browser; owner shutdown closes only its verified browser/helper resources and leaves an unrelated sentinel browser intact.
7. Herdr Space/tab order, pane layout IDs, and terminal focus behavior remain unchanged through hide/show/close.

8. Keep a durable association-level record for a pending capture even when the live target is gone; recovery must identify the original Space association and browser incarnation before offering retry.
9. Treat a save acknowledgement for an older revision as non-authoritative when a newer local revision exists. The newer draft remains dirty and visible until its own acknowledgement.
10. Preserve note editor text through hide, stream reconnect, and a failed save. Escape/cancel may intentionally discard only the actively edited note, not unrelated marks.
11. Make browser close and hide labels match their actual scope. Do not call a full owner close from a presentation toggle or imply that saved feedback is still live pixels.
12. Closing an observer client releases that client's view without stopping the shared browser. An explicit shared-browser close follows the existing authorized owner-forwarding and association/endpoint validation path; do not confuse the runtime owner with the current input controller or prohibit otherwise authorized forwarded operations.

The implementation must use existing draft/feedback identities and existing state-root ownership. If a particular unsent draft cannot be persisted safely, retain it and explain why close cannot complete. A warning followed by ordinary-close data loss does not satisfy this task.

## Verification

The integration owner must use uniquely named disposable gateway and Linux Tauri sessions, isolated profile/state roots, and the resource guard. Record URL/title, target/document generations, draft/capture IDs, revisions, bytes hashes, feedback lookup/ack results, frame/helper counts, and cleanup. Exercise A02, A18, A19, A21, and A22; record A23 as excluded rather than attempted. Verify both close roles and a lost-save response through the authoritative rendered state. Static checks and tests may support the result but cannot replace browser/native runtime proof.

The evidence record must include:

- the exact hide/show and true-close action sequence, including owner or observer role;
- before/after URL, title, target/document identity, draft revision, annotation count, note text, and pending IDs;
- a saved-feedback lookup after close and a lost-save/retry result with byte identity;
- resource ownership and cleanup for browser, helper, frame stream, profile, and disposable session.

Do not mark a destructive close as preservation merely because the next open reaches `Live browser view`; page identity and recoverable work are separate acceptance facts.

## Handoff

Deliver changed paths and the lifecycle decision to the orchestrator; do not edit `tasks.json` from this brief. The integration owner must attach a compact evidence record under `runs/<run-id>/WEB-01.md` with durable artifact locations and exact scenarios, then land a real commit containing only this task’s implementation/docs/test changes. Include unresolved platform or recovery risks; a green build without hide/show and close recovery evidence is not completion. Coordinate shared `app-shell`, `browser-ui`, `browser-helper`, and `browser-store` writes serially per `../tasks.json` and `../ORCHESTRATOR.md`.
