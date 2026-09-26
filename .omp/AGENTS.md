# Cockpit agent context

- `CONTEXT.md`, `DECISIONS.md`, `research/ui-design-direction.md` and `research/ui-implementation-constraints.md` are authoritative. They are long; read the sections relevant to the change.
- Herdr-server is the runtime authority. When its behavior is unclear, check the running Herdr TUI, then upstream Herdr source or schema.
- Planning folders are history unless the user names one.
- UI changes are verified in the browser build against a disposable fixture; native-only changes also need one native run.

Delivery rules: `.omp/RULES.md`.
