# Cockpit agent context

For substantial implementation, orchestration, live feedback, verification, or commits, MUST read `skill://incremental-delivery`.

## Authorities

- Read `CONTEXT.md` and `DECISIONS.md` before changing architecture or domain behavior.
- Read the active phase plan before defining an increment. `NEXT_PHASE_PLAN.md` is historical when its status says complete.
- Read `research/ui-design-direction.md` and `research/ui-implementation-constraints.md` before changing UI structure, styling, terminal rendering, focus, or interaction.
- For Cockpit-to-Herdr UI parity, MUST read `skill://cockpit-ui-parity`.

## Project verification

- Herdr-server is runtime authority. Test mutations and terminal control only in uniquely named disposable Herdr sessions, never the user's active session.
- Reuse a defined real-surface acceptance scenario to verify the complete user action, authoritative response or event, rendered result, and relevant failure state. UI changes need browser proof for shared frontend behavior. Changes to Tauri commands, channels, startup, window behavior, or native-only rendering also need a real native smoke.
- Compare hierarchy, ordering, focus, ownership, and interaction semantics with the running Herdr TUI. Use upstream Herdr source or schema when observed behavior is ambiguous.
- Freeze affected-scope checks before implementation; run them once after batch integration. Do not repair and rerun after verification starts. Record product, automation, stale-runtime and prerequisite failures; finish safe independent planned checks and report dependent checks unrun.
- Reuse owned fixtures and clean up on every exit. Verify owned browser roots, helpers and services exited and ports/sockets were released; closing tabs or receiving a successful stop response is not proof. Never kill shared/user resources.

## Delivery

Follow `skill://incremental-delivery`: one fixed user-authorized implementation batch, one consolidated verification pass, cleanup, report, then stop. No per-criterion retry allowance, automatic second repair wave, or new subset to reset the pass. Further repair needs explicit user authorization, not new evidence, worker replacement, a todo reminder or an incomplete campaign. Keep model/advisor routing unchanged; delegate only useful independent work. Honor explicit limits, never inferred deadlines. Pause is sticky; instruction maintenance does not resume product work.

Commit each coherent verified repair or integrated batch with only owned changes, and report its commit hash and exact behavior checks. Do not create separate plan, evidence, or status commits solely for workflow ceremony. An unsuccessful checkpoint may preserve unverified changes without a product commit, but must label them unverified and must not claim completion. Preserve unrelated pre-existing work.

Show concrete implemented/verified/committed outcomes in visible progress as they occur, linked to the authoritative task and evidence. Do not leave only unchanged umbrella todos or mark a parent done from a subset. Preserve the frozen batch and attempted checks across handoffs.
