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
- Avoid duplicate unchanged gates. Classify verification failures as product defect, automation issue, stale runtime, or missing prerequisite before editing product code. Focused checks are permitted under exclusive or isolated ownership; do not run shared validation gates while concurrent writers mutate the integration surface.
- Clean up only resources created by the verification scenario.

## Delivery

Use one implementation owner per coherent repair and delegate independent slices only when they provide useful concurrency. Do not infer deadlines from example durations or impose default time limits; honor explicit user limits when present. Record hypothesis, result, what it rules out, and next check. After two failed attempts on the same criterion, stop editing pending new evidence; allow one review/repair wave. Valid stops are completion, user pause/handoff, external blocker, or an inconclusive attempt with no justified next step. Pause is sticky until explicit resume.

Commit each coherent verified repair or integrated batch with only owned changes, and report its commit hash and exact behavior checks. Do not create separate plan, evidence, or status commits solely for workflow ceremony. An unsuccessful checkpoint may preserve unverified changes without a product commit, but must label them unverified and must not claim completion. Preserve unrelated pre-existing work.
