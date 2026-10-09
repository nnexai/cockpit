# WS-13 Orchestration core decomposition

Wave 2 · Size L · Depends on: WS-03, WS-05 · Blocks: WS-20, WS-23
**Concurrency-sensitive:** one round of independent `reviewer` review (high-severity only) before handoff.

## Goal
The supervisor's rules stay exactly as they are. Their expression becomes readable: small per-family handlers, one reviewed-CAS helper, typed agent kind and one retirement-state source.

## Owns
- `crates/cockpit-core/src/orchestration.rs`
- `orchestration/{messages,retirement,escalation}.rs`
- the CAS call sites only in `orchestration/dispatch.rs`
- a new `orchestration/mutate/`
- `src/app/supervisor/retirementView.ts` (+ test)
- the related protocol DTOs, if the agent-kind type crosses the wire

## Evidence
- `mutate_with_review` is ~1,060 lines (`~213-1276`) with ~30 action arms.
- `messages.rs` `apply` is 746 lines (`~294-1043`); its Report arm alone is ~200 lines. `append` takes 11 arguments.
- The reviewed launch CAS is repeated at `orchestration.rs` ~252, 1304, 1346, 1572, 1672, 1711, 1734, 1775, 1815 and in `dispatch.rs`.
- `actual_agent_kind: Option<String>` (`~55`) is compared to `"omp"` at ~607, 2208, 2410, 2599 and `retirement.rs` ~152, 288, 291.
- The retirement states are listed three times: `RetirementStateKind` (`retirement.rs:~11-28`), `owner_transition` (`~192-201`) and the `retirementView.ts:~44-72` switch.

## Change
1. Make `mutate_with_review` a thin dispatcher. Family handlers live in `mutate/{tasks,runs,grants,retirement,intents}.rs` and share one `MutationCtx` (locked store, actor, now, reviewed fence).
2. Add `with_reviewed_run(store, reviewed, predicate, mutate)`. Each call site keeps its **exact** predicate (`same_launch_incarnation` vs `recovery_review_matches`).
3. Split `messages.rs` `apply` into report/inbox/subagent-control modules, and replace `append`'s arguments with an `AppendMessage` struct.
4. Make `AgentKind` an enum, decided once when CLI evidence is recorded. It serializes to exactly the strings stored and sent today, so neither the persisted bytes nor the wire change. Make the string-matched codes in `escalation.rs` (`~20-23`) an enum with the same serialized values.
5. Derive the retirement kind from one definition and express the transition table as data. `retirementView.ts` switches exhaustively over the generated type.

## Keep
- Every authorization rule, fence, CAS revision, error code and persisted byte.
- The test suites: `service_tests.rs`, `retirement_tests.rs` and `retire_tests.rs` pass unchanged, apart from imports.

## Acceptance
- No function in `crates/cockpit-core/src/orchestration*` is over 300 lines.
- One reviewed-CAS helper.
- No `"omp"` string comparisons.
- Reviewer sign-off with no unresolved high-severity findings.

## Verify
- `cargo test -p cockpit-core orchestration`, `cargo test -p cockpit-host`, `bun run test -- src/app/supervisor`.
- Disposable supervisor smoke (`skill://cockpit-disposable-herdr-fixture`):
  - start the supervisor;
  - a worker goes through propose → prepare → Ready → execute → report → accept → retire;
  - one owner restart mid-run recovers without a duplicate launch.
