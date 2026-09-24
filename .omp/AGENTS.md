# Cockpit agent context

## Authorities

- Read `CONTEXT.md` and `DECISIONS.md` before changing architecture or domain behavior.
- Read the active phase plan before defining an increment. `NEXT_PHASE_PLAN.md` is historical when its status says complete.
- Read `research/ui-design-direction.md` and `research/ui-implementation-constraints.md` before changing UI structure, styling, terminal rendering, focus, or interaction.

## Project verification

- Herdr-server is runtime authority. Test mutations and terminal control only in uniquely named disposable Herdr sessions, never the user's active session.
- Reuse a defined real-surface acceptance scenario to verify the complete user action, authoritative response or event, rendered result, and relevant failure state. UI changes need browser proof for shared frontend behavior. Changes to Tauri commands, channels, startup, window behavior, or native-only rendering also need a real native smoke.
- Compare hierarchy, ordering, focus, ownership, and interaction semantics with the running Herdr TUI. Use upstream Herdr source or schema when observed behavior is ambiguous.
- Reuse owned fixtures and clean up on every exit. Verify owned browser roots, helpers and services exited and ports/sockets were released; closing tabs or receiving a successful stop response is not proof. Never kill shared/user resources.

## Delivery

Execution and delivery policy lives in `.omp/RULES.md`; campaign-specific scope and acceptance live in its active plan. Do not duplicate execution rules in handoffs.
