# WS-05 Orchestration migration removal

Wave 1 · Size M (~220–350 lines plus frontend/SDK) · Depends on: WS-02 · Blocks: WS-13, WS-18, WS-19 (and through them WS-20, WS-23)

## Goal
The supervisor reads only current task/run records. The in-place "Track checklist" adoption of untracked bullets is removed end to end. **Owner decision D1:** default is remove; skip phase 2 if the owner keeps it.

Source of truth: `../migration-inventory.md`, sections "Orchestration" and "Protocol / frontend / integrations" (orchestration items), and coupling notes 6–7.

## Owns
- core `orchestration/{store,steps,retirement,projection}.rs`: migration parts only;
- the `TaskStepsAdopt` arm in `orchestration.rs`;
- `orchestration/{service_tests,retirement_tests}.rs`;
- `crates/cockpit-protocol/src/orchestration.rs` and its emitter entry in `typescript.rs`;
- the steps-adopt command, help and test in `crates/cockpit-host/src/cli_orchestration.rs`;
- `src/app/supervisor/{stepInteractions.ts,useSupervisor.ts,SupervisorSteps.tsx}` + tests (adoption branches only);
- `src/client/orchestrationProtocol.ts` + test;
- the adoption operation and its test in `integrations/omp/cockpit-orchestration.ts`;
- in `orchestration/dispatch.rs`: only the `exact_result` call sites reached by step 3 and the companion-folder wording at `~370`. WS-03 edits the env lines at `~676` and merges first.

## Change
1. **Old records:**
   - remove the missing assignment-journal default and its test;
   - remove the ACK-without-timestamp and unlinked-Answer fixtures from the mixed projection tests;
   - remove the old missing-field defaults in protocol `orchestration.rs` (Vec defaults matter; Option defaults are redundant) and the seven `legacy_*` protocol tests.
2. **Checklist adoption (D1):** remove it everywhere: `StepAdoption`, `StepIntent::Adopt`, the insertion plan, the handler, the DTO/action, the CLI `steps-adopt`, the frontend intent/validator/routing and the "Track checklist" UI, and the SDK operation. Leave no alias and no dead command.
3. **Retirement:**
   - remove the acceptance-without-reviewed-Result fallback (`retirement.rs:~99-103`) and its test;
   - trace `exact_result` through the commit/recovery callers;
   - remove `legacy_schema_one_grants_and_acceptance_intents_keep_truthful_default_provenance`.
4. Regenerate TS.

## Keep
- `RunAdopt` / `RunKind::Adopted`: a live feature, unrelated to this removal.
- The worker retirement lifecycle and nullable retirement for cancelled or closed runs.
- Parser safety for untracked/external Markdown: untracked bullets stay visible and read-only.
- Byte-preserving assignment recovery.
- Do not edit CSS. List selectors made dead by the removed adoption preview for WS-24.

## Acceptance
- No `Adopt`/adoption step symbols remain in core, protocol, CLI, frontend or SDK.
- No `legacy_*` orchestration tests remain.
- Current checklist add/check/edit works.

## Verify
- `cargo test -p cockpit-core orchestration`, `cargo test -p cockpit-host cli_orchestration`.
- `bun run test -- src/app/supervisor src/client/orchestrationProtocol integrations/omp`.
- Disposable supervisor smoke:
  - open the Supervisor;
  - on a task with a checklist, add, check and edit a step from the UI and from the agent CLI;
  - append an untracked bullet externally and confirm it renders read-only, with no upgrade button.

## Doc notes for WS-25
`DECISIONS.md` ~57/66, `CONTEXT.md` ~345, `CODE_GUIDE.md` ~23/59, `integrations/agent-skills/cockpit-cli-orchestration/SKILL.md` ~112.
