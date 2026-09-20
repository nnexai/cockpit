# Acceptance coverage and completion gate

This is a requirements map, not a pass report. Results live in task evidence referenced by [tasks.json](tasks.json). Each owning task must prove its numbered criteria; shared scenarios may reference one durable artifact only when it actually covers the relevant build, client, platform and behavior. A failed or unavailable required result remains open.

## Campaign behavior coverage

| Requirement | Primary task | Additional gate |
| --- | --- | --- |
| Protected user state, named disposable resources, exact runtime/config identity | RUN-01 | Every task; ACCEPT-01 cleanup |
| Protocol/schema/capability compatibility without exact patch gate | TERM-01 | NATIVE-02 |
| Only selected visible panes attach/render; revisit works without hidden errors | TERM-02 | TERM-03, PERF-01 |
| Old requests/streams cancelled; old failures do not paint on new resources | SYNC-01 | WEB-04, FLOW-01 |
| Terminal scrolling under output, deliberate tail-follow, no blanking | TERM-03 | PERF-01, NATIVE-02 |
| Terminal prefix/modifiers/clipboard/application mouse/focus and TUI parity | TERM-03 | WEB-04, NATIVE-02 |
| Herdr hierarchy/order/Agents/layout and local control intent remain authoritative | TERM-03 | FLOW-01, ACCEPT-01 |
| Context/Review file/picker cancellation and revision-aware scroll continuity | VIEW-01 | FLOW-01, PERF-01 |
| Files/Markdown/Mermaid/media/search/snapshots, full local Review scopes | FLOW-01 | NATIVE-02 |
| Durable file/line comments, same-tab exact paste, no Enter/duplicate on unknown result | FLOW-01 | ACCEPT-01 |
| Read-only source opening, visible exact effects before explicit mutation | SETUP-01 | GLAB-04, ACCEPT-01 |
| Portable no-replace companion publication and recoverable partial setup | SETUP-02 | NATIVE-02 |
| Borrowed-directory protection, dirty-worktree refusal, ownership-aware teardown | FLOW-01 | ACCEPT-01 |
| GitLab issue and verified issue-type work-item URL support | GLAB-01 | GLAB-04 |
| Nested namespaces, self-managed host/base-path authority, wrong-project rejection | GLAB-01 | GLAB-02, GLAB-04 |
| Independent GitLab MR, source branch, metadata/comments/discussions freshness | GLAB-02 | GLAB-04 |
| Refresh/conflicts/failures/capabilities and usable resource-list scrolling | GLAB-03 | GLAB-04 |
| Existing GitHub issue/Tea behavior unchanged, no credential leakage | GLAB-03 | GLAB-04, ACCEPT-01 |
| Real authenticated issue and MR setup/import/refresh/recovery | GLAB-04 | NATIVE-02 |
| Browser hide/show preserves page/drafts; close preserves saved and recoverable unsent work | WEB-01 | WEB-05 |
| First click and initial Element pick do not require a prior Browse gesture | WEB-02 | WEB-04 |
| Resize/zoom/DPR sharpness and geometry/matching-frame barrier | WEB-03 | WEB-05, NATIVE-02 |
| Full browser input/navigation/ownership/facility behavior | WEB-04 | WEB-08, NATIVE-02 |
| Annotation toolbar/gesture/capture/draft/feedback/paste reliability | WEB-05 | WEB-08, NATIVE-02 |
| Bounded frame/input/decode/process lifetimes, crash/reconnect/observer isolation | WEB-06 | PERF-01 |
| Missing browser dependencies and incompatible CLI fail locally/actionably | WEB-07 | NATIVE-01, NATIVE-02 |
| Browser authentication/host/lease/frame/page-to-loopback negative boundaries | WEB-08 | ACCEPT-01 |
| Actual fresh macOS bundle installation/update/uninstall ownership | NATIVE-01 | NATIVE-02 |
| Actual macOS native daily-use regressions and input/decode proof | NATIVE-02 | ACCEPT-01 |
| Measured sustained performance/scroll/resource bounds | PERF-01 | ACCEPT-01 |
| Integrated journey, issue reconciliation and owned cleanup | ACCEPT-01 | All required tasks |

No task may “verify” an upstream feature solely through mocks. Fixture regression tests are appropriate for edge cases and negative boundaries, while the actual user path remains the completion gate.

## Complete inline-browser matrix ownership

The existing [A01–A25 matrix](../inline-space-browser-2026-09-13/03-delivery-and-verification.md#4-acceptance-matrix) remains the detailed scenario source. This campaign assigns every non-excluded row; no old completion heading is treated as a pass.

| Row | Required behavior | Primary owner | Companion proof |
| --- | --- | --- | --- |
| A01 | GUI/agent reuse and concurrent opens share one browser/profile | WEB-06 | WEB-07 |
| A02 | Space/tab switching, hide/show, targets and no cross-Space state leakage | WEB-01 | TERM-02, WEB-06 |
| A03 | Owner shutdown versus observer close; unrelated sentinel survives | WEB-06 | WEB-08 |
| A04 | Pointer buttons/double-click/drag/release/cancel/letterboxing | WEB-04 | WEB-02 |
| A05 | Wheel X/Y accumulation, nested scroll, selection, no host scroll | WEB-04 | PERF-01 |
| A06 | Keyboard/modifiers/repeat/Unicode/IME/clipboard/local editors | WEB-04 | NATIVE-02 |
| A07 | Terminal/browser focus and magic-prefix ownership handoff | WEB-04 | TERM-03 |
| A08 | Two clients, one viewport/controller, lease loss and takeover | WEB-04 | WEB-03, WEB-06 |
| A09 | Redirect/reload/hash/SPA/history/agent navigation and URL edit draft | WEB-04 | WEB-01 |
| A10 | Stationary cursor changes and stale metadata rejection | WEB-04 | WEB-02 |
| A11 | Resize/DPR/zoom/scroll/letterbox input and mark geometry | WEB-03 | WEB-05 |
| A12 | Dialog/file chooser/download/permission/dependency actionable state | WEB-04 | WEB-07, WEB-08 |
| A13 | Invalid/late frames, metadata gaps, slow observers and helper crash | WEB-06 | WEB-08 |
| A14 | Region/freehand/color/edit/remove/notes/toolbar/narrow widths | WEB-05 | WEB-01 |
| A15 | Element DOM/iframe/shadow/transforms/canvas honest boundaries | WEB-05 | WEB-02 |
| A16 | Anchor drift and navigation/scroll/resize while drawing/capturing | WEB-05 | WEB-03 |
| A17 | Pinned PNG/evidence alignment at multiple DPR/zoom values | WEB-05 | WEB-03 |
| A18 | Save/composition/store failure, unknown response, concurrent edit/restart | WEB-05 | WEB-01 |
| A19 | Fetch/ack exact IDs, idempotence, newer capture and closed-browser access | WEB-05 | WEB-01 |
| A20 | Correct same-Space active-tab agent, no target, no Enter | WEB-05 | FLOW-01 |
| A21 | Unknown paste, exact retry/ack and unsent retention | WEB-05 | FLOW-01 |
| A22 | Agent/Space movement, rename/close, endpoint replacement/reused IDs | WEB-06 | WEB-08 |
| A23 | Legacy draft/capture migration | Excluded by prior user cutover decision | Do not implement or claim |
| A24 | Capacity/revision conflicts/tombstones with no unsent eviction | WEB-05 | WEB-08 |
| A25 | No legacy extension runtime, bounded long-running resources/security | WEB-08 | WEB-06, PERF-01 |

Security negative cases from that plan are WEB-08 requirements; performance protocol and figures are WEB-06/PERF-01 requirements. Browser-only success does not establish WebKit/native input/decode behavior. JPEG stream audio remains absent, not an unimplemented new feature silently added to this campaign. Unsupported file/dialog facilities must have an honest usable refusal, not fake success or invisible deadlock.

## Platform and evidence policy

- Shared UI: browser proof of user action -> authoritative response/event -> visible outcome and failure recovery, at desktop and minimum sizes.
- Tauri commands/channels/startup/platform input or image decode: real Linux-native proof in the owning task. A process remaining alive does not prove interaction.
- macOS bundle/publication/plugin/platform regressions: actual macOS proof in NATIVE-01/02 and SETUP-02 as specified. Missing runner is blocked, not skipped. Evidence may be shared across task records, never inferred from Linux.
- Herdr-backed semantics: compare a disposable live TUI oracle or supported schema/source where observation is ambiguous. Browser annotation geometry itself does not need a fictional TUI counterpart.
- Provider host/path/negative cases: deterministic fixtures plus real authenticated issue/MR reads on the designated project. Self-managed/nested cases not available remotely must have representative bounded fixtures and explicitly stated live-host limitations; do not claim live self-managed coverage from gitlab.com.
- Performance: identical fixture/viewport/build conditions, defined sample counts, measured clocks and resource identities. The proposed 150ms click p95 / 250ms metadata targets are not existing measurements. Freeze the protocol before optimization and retain measurements that fail.
- Error policy: old/pre-dispatch cancellation and normal ownership loss are not resource failures. Real auth, permission, disk, protocol, runtime and source-conflict failures stay visible and actionable. Unknown post-dispatch outcomes are never silently retried.

## Done evidence checklist

Each task record must point to committed evidence containing:

1. Task ID, source commit, changed files, runtime/platform/build identities and owned fixture IDs.
2. Every numbered acceptance criterion with PASS/FAIL/BLOCKED, an actual observed result and durable evidence reference.
3. For bugs: initial reproduction or red-capable negative control and post-fix proof; reported observations remain ground truth.
4. Exact checks executed and their output/results; no copied historical suite totals.
5. Resource ownership/cleanup, retained fixture reasons, and credential redaction.
6. Review findings/resolution and consequential advisory decisions when applicable.
7. Actual implementation/evidence commit hashes and remaining blockers (none for done).

The ledger may be checkpointed after the implementation commit to record its hash. Required tasks cannot become deferred, disappear, or lose criteria without explicit user scope approval.

## Final integrated journey

ACCEPT-01 uses the settled code and authorized fixtures to prove the loop: inspect GitLab issue/MR without mutation; review exact setup effects; create owned task resources; import context; browse/search Files and Review; preserve comments while navigating; paste exact unsubmitted text; open/use/annotate/hide/restore browser; switch terminals/Spaces and scroll under output; refresh changed source without overwriting user edits; survive a disconnect/lost response; and tear down only proven owned resources. A borrowed directory and an unrelated sentinel survive. No single successful request is enough.

The final issue report must leave #6 explicitly partial while LATER-GHPR/LATER-JIRA remain deferred. #9 requires disposition of the Element/config findings as well as #10–#12. #3 requires current-inline requirement reconciliation, not a claim that an obsolete extension patch was applied.
