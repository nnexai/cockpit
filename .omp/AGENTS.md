# Cockpit agent context

## Authorities

- `CONTEXT.md`, `DECISIONS.md`, `research/ui-design-direction.md` and `research/ui-implementation-constraints.md` are authoritative. They are long: list their headings (`grep -n '^#'`) and read only the sections relevant to the change.
- Herdr-server is the runtime authority. When Herdr behavior is ambiguous, compare with the running Herdr TUI, then upstream Herdr source or schema.
- Planning folders are history unless the user names one.

## Verification

- UI changes: exercise the changed interaction in the browser build against a disposable fixture (`scripts/verify/ui_polish_runtime.py`) and look at the result. Native-only changes (Tauri commands, channels, startup, window behavior, native rendering) also need one native run.
- Reuse a running fixture across checks; stop it when done.

Delivery rules live in `.omp/RULES.md`.
