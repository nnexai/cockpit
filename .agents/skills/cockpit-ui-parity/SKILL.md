---
name: cockpit-ui-parity
description: Use when changing Cockpit UI that represents Herdr behavior, such as Space/tab/pane hierarchy, ordering, selection, terminal focus, keyboard ownership, or Herdr-backed actions. Excludes browser-extension annotation tools, page geometry, capture controls, pairing, and other Cockpit-only UI without a Herdr TUI counterpart.
---

# Cockpit UI parity

For behavior with a Herdr TUI counterpart, the running Herdr TUI is the reference; installed schema or upstream Herdr source fills gaps it doesn't show. Cockpit may improve on it for desktop use, but a deliberate difference needs a `DECISIONS.md` entry. The relevant design constraints are in `research/ui-design-direction.md` and `research/ui-implementation-constraints.md`.

Things that matter here:

- Herdr responses and events are the truth; a local click is only intent.
- Keep server ordering and stable IDs; show names, not internal IDs.
- Actions live on their natural target (row, context menu, pane header, terminal), not on extra buttons.
- Use the existing design tokens; states (hover, focus, pending, selected) must not shift layout.

Verify the changed interaction in the browser against a disposable Herdr session, and in the native app when native code changed. Watch for flicker, stale state and focus landing in the wrong place.
