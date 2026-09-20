# WEB-05 — Complete annotation capture and delivery reliability

## Outcome

Make the annotation surface and feedback delivery reliable across toolbar reconciliation, freehand/region/element authoring, capture geometry, drafts, concurrency, exact-PNG retries, feedback lookup/acknowledgement, and paste retention. The live image shows only the intended marks and controls; pointer release cannot strand a gesture. A failed composition/save/send preserves the same recoverable work and bytes, while fetch/ack/paste operations remain identity-safe and idempotent.

This task completes issue #3 and the A14–A21/A24 slice not already proven by WEB-01–03.

## Evidence and starting points

- Baseline: `6f6222b74e4f552ce697e61364cf653f4b6be29f`; the inline handoff proves only focused region persistence and explicitly excludes broad A14–A24 proof.
- Issue #3: https://github.com/nnexai/cockpit/issues/3. Related lifecycle/umbrella issues: https://github.com/nnexai/cockpit/issues/9 and https://github.com/nnexai/cockpit/issues/11.
- Read `planning/inline-space-browser-2026-09-13/02-annotations-and-migration.md` and delivery rows A14–A21, A24; A23 legacy migration remains excluded.
- Source starts: `src/app/browser/BrowserPane.tsx` toolbar/tool state, `simplify`, gesture lifecycle, `inspect`, note editor, draft load/save, capture preparation/composition, feedback actions, and paste response handling.
- Source starts: `src/app/browser/transform.ts` and `framePresenter.ts` for capture-bound geometry and accepted frame identity.
- Source starts: `crates/cockpit-core/src/browser/drafts.rs`, `browser_feedback.rs`, `crates/cockpit-core/src/browser/delivery.rs`, and `crates/cockpit-core/src/comments/paste.rs` for revision/tombstone, exact PNG, lookup/ack, duplicate-risk, and uncertain receipt semantics.
- Read `planning/stability-and-gitlab-2026-09-20/tasks.json` for `browser-ui`, `browser-helper`, `browser-store`, and `comment-delivery` ownership; overlapping writes are serialized.

## Changes

1. Reconcile compact toolbar state with current tool/color/draft and minimum/narrow layout; retain accessible labels/tooltips and visible Notes count without restoring text-heavy legacy UI.
2. Keep raw freehand points during drawing and use bounded iterative simplification only after release; preserve configured color/stroke semantics. Region, Element, optional text, edit/remove, notes clear/revisit must share the same draft identity.
3. Ensure pointer capture, release, cancel, mode switch, target switch, blur, and lease loss clear transient gestures without deleting earlier marks or leaking remote page input.
4. Pin the accepted frame descriptor and immutable transform for composition. Reject wrong target/document/viewport/frame identity and exclude toolbar/editing chrome from PNG. Report retryable composition failure.
5. Persist revisioned drafts and conflicts without silent eviction; preserve the newest edits when an old save response arrives. Keep pending captures associated with the original IDs and exact PNG bytes.
6. Make fetch non-consuming, acknowledgement exact/idempotent, unknown paste receipts durable, duplicate-risk retries explicit, and unsent work immune to retention pruning. Preserve optional preview and no-eligible-agent feedback states.
7. Add required review of capture integrity, concurrency/tombstones, delivery idempotence, and payload redaction (no raw point arrays in agent feedback).

## Non-goals

- No legacy extension/profile migration or A23 implementation.
- No new generic annotation/plugin framework, alternate transport, or selector-only element evidence.
- No automatic duplicate paste/send retry after an uncertain outcome.
- No claim that toolbar screenshots or prior focused tests establish native parity, geometry, capacity, or delivery completion.

## Acceptance

1. A14: browser gateway and Linux-native Tauri author freehand/region in both directions, colors, edit/remove, optional text, and notes; pointer release/cancel leaves no stuck gesture and hide/reopen retains the draft.
2. A15/A16: ordinary DOM, same/cross-origin iframe, shadow-root, transformed/nested-scroll, and canvas boundaries produce real evidence or honest limitation; stale hover/pick replies and document drift cannot create wrong annotations.
3. A17: at multiple DPR/zoom/resize sizes, saved PNG contains pinned real page pixels, marks, and comments, excludes toolbar/chrome, and exposes correct public provenance/element fields.
4. A18: failed composition, full store/disk, lost save response, concurrent edits, navigation, and restart retain recoverable drafts/pending bytes; retry uses the same IDs and exact PNG hash.
5. A19: repeated lookup never consumes; acknowledging an older exact capture twice is idempotent and a newer capture remains pending; closed-browser feedback/image lookup remains readable.
6. A20/A21: selected same-Space agent receives paste only after confirmed focus; no Enter is submitted; rejected/unknown response preserves unsent work and no automatic duplicate occurs; retention prunes handled images only by policy.
7. A24: capacity and multi-client races fail explicitly without eviction, and revision conflicts/tombstones prevent lost or resurrected marks. Agent payloads omit raw point arrays while retaining evidence/bounds/comments/image path.

8. Keep transient drawing state out of public feedback payloads. Raw point arrays may remain local or in the owner draft where needed for editing, but sent agent feedback carries bounded metadata, bounds, comments, evidence, and image path only.
9. A composition retry must verify the stored frame descriptor and draft revision before reuse. If either changed, require a new explicit capture instead of silently combining bytes and marks from different documents.
10. Preserve selected annotations and note text when a feedback destination is absent, wrong-space, or rejected. “No eligible agent” is a visible state, not a reason to clear the draft.
11. Treat an unknown paste response as unresolved indefinitely until a later authoritative receipt/ack or explicit user resolution; do not infer success from focus alone.
12. Enforce bounded annotation count, point count, note size, capture bytes, and pending IDs using existing store limits, returning an actionable capacity error rather than evicting unfinished work.
13. Keep feedback fetch/read operations available after browser close and after target navigation; they are association/capture reads, not live-page inspection.

The implementation should preserve the current compact toolbar and feedback controls while making state transitions observable. Do not satisfy this brief with screenshot-only toolbar parity or a source assertion about PNG payload fields.

## Verification

Use run-owned browser gateway, Linux Tauri, disposable page/agent fixtures, isolated state roots, full-disk/store-limit controls where safe, and recorded PNG hashes. Exercise A14–A21/A24 and negative races through authoritative UI/store responses. Record toolbar dimensions/accessibility, raw/simplified point counts, frame identities, PNG hashes, draft revisions, operation IDs, lookup/ack results, paste target/focus acknowledgement, receipts, retention, and cleanup. Native proof is required for capture/display and paste handoff; source tests alone are insufficient.

The evidence record must include:

- toolbar accessibility/geometry and the complete authoring sequence for each supported tool;
- raw versus simplified freehand points, pinned frame descriptor, PNG hash, provenance, and chrome-exclusion inspection;
- draft revisions, conflict/capacity errors, retry IDs, lookup/ack responses, paste receipt state, and retention result;
- browser/native captures and cleanup of agent fixtures, state roots, profiles, and sentinels.

Do not mark delivery complete from a successful HTTP response alone; prove the rendered receipt and subsequent fetch/ack semantics.

## Handoff

Return changed paths, capture/delivery invariants, supported limits, and unresolved risks. The integration owner must create durable `runs/<run-id>/WEB-05.md` evidence and land a real commit after serialized shared writes. Keep `tasks.json` as the sole status/owner/dependency ledger; follow `../ORCHESTRATOR.md` for locks and resource cleanup.
