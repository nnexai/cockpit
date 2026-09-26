# Cockpit agent context

- `CONTEXT.md` (architecture), `DECISIONS.md` (rules in force) and `CODE_GUIDE.md` (code layout) describe the current product. `research/ui-*.md` are design direction and UI invariants, not a pixel spec.
- Herdr-server is the runtime authority. When its behavior is unclear, check the running Herdr TUI, then upstream Herdr source or schema.
- `archive/` and `planning/` are history unless the user names something there.
- UI changes are verified in the browser build against a disposable fixture; native-only changes also need one native run.

Delivery rules: `.omp/RULES.md`.
