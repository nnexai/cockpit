---
name: cockpit-ui-parity
description: Use when changing Cockpit UI that represents Herdr behavior, such as Space/tab/pane hierarchy, ordering, selection, terminal focus, keyboard ownership, or Herdr-backed actions. Excludes browser-extension annotation tools, page geometry, capture controls, pairing, and other Cockpit-only UI without a Herdr TUI counterpart.
---

# Cockpit UI parity

Apply this skill only to behavior with a Herdr TUI counterpart. For that behavior, Herdr is the regression oracle; Cockpit may deliberately improve desktop interaction.

Browser-extension annotation tools, page geometry checks, capture controls, and pairing use their own product requirements and browser evidence. A change is not a parity task merely because it affects layout, styling, focus, or interaction. For mixed changes, apply this skill only to the Herdr-backed portion.

## Establish the oracle

1. Read the sections of `research/ui-design-direction.md` and `research/ui-implementation-constraints.md` that cover the affected surface (list headings first; don't read the whole files).
2. Observe the current behavior in the running Herdr TUI using a disposable named session. Capture hierarchy, ordering, selection, semantic focus, keyboard ownership, action placement, and failure behavior relevant to the increment.
3. Use the installed schema or upstream Herdr source when the UI observation does not expose the contract.
4. State whether the increment preserves, clarifies, or replaces the Herdr behavior. A replacement requires an explicit `DECISIONS.md` entry.

The oracle is established when the intended difference can be described as observable before-and-after behavior rather than aesthetic adjectives.

## Implement as a system

- Reconcile from Herdr responses and events; local clicks are intent, not authority.
- Use existing semantic typography, spacing, color, geometry, and state tokens. Add one shared token when a new semantic role is required; avoid component-local size or color patches.
- Preserve stable geometry across hover, focus, pending, error, and selected states. Borders and controls must not reflow the layout.
- Keep server ordering and stable IDs. Display user-facing names rather than internal IDs unless the ID is the only honest label.
- Make the full interaction available at its natural target: row click, context menu, drag target, pane header, or terminal surface. Do not substitute an unrelated button or local-only state.

## Prove the increment

Use a uniquely named disposable Herdr session and record every resource created by the scenario.

Browser proof covers the complete changed path:

1. load real authoritative state;
2. perform the user gesture;
3. observe pending state when applicable;
4. observe the Herdr-confirmed result or inline failure;
5. verify ordering, selection, focus, ownership, and geometry remain stable;
6. repeat the interaction needed to expose flicker, stale state, or delayed-event regressions.

Also run the real Tauri app when the increment changes native commands, channels, startup, window behavior, platform input, or native-only rendering. Pin native and browser runs to disposable sessions separate from the user's active session.

Compare the final Cockpit surface with the Herdr oracle at the normal desktop size and the documented minimum size. Inspect terminal glyph continuity at zoom when fonts, xterm, pane sizing, or rendering changed.

Clean up only resources created by the proof. Report the observed behavior and screenshots or runtime evidence, not merely successful requests.
