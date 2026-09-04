# Cockpit agent context

For substantial implementation, orchestration, live feedback, verification, or commits, MUST read `skill://incremental-delivery`.

## Authorities

- Read `CONTEXT.md` and `DECISIONS.md` before changing architecture or domain behavior.
- Read the active phase plan before defining an increment. `NEXT_PHASE_PLAN.md` is historical when its status says complete.
- Read `research/ui-design-direction.md` and `research/ui-implementation-constraints.md` before changing UI structure, styling, terminal rendering, focus, or interaction.
- For Cockpit-to-Herdr UI parity, MUST read `skill://cockpit-ui-parity`.

## Project verification

- Herdr-server is runtime authority. Test mutations and terminal control only in uniquely named disposable Herdr sessions, never the user's active session.
- A UI increment needs browser proof for shared frontend behavior. Changes to Tauri commands, channels, startup, window behavior, or native-only rendering also need a real native smoke.
- Compare hierarchy, ordering, focus, ownership, and interaction semantics with the running Herdr TUI. Use upstream Herdr source or schema when observed behavior is ambiguous.
- Verify the complete user action, authoritative response or event, rendered result, and failure state. A successful request alone is not UI proof.
- Clean up only resources created by the verification scenario.

## Delivery

Each implementation increment ends with one verified commit containing only that increment's owned changes. Completion reports name the commit hash and the exact behavior checks that passed. Keep unrelated pre-existing work unstaged.
