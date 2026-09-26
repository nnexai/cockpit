# Cockpit delivery rules

- Do the work yourself. Delegate only read-only exploration or independent mechanical slices with a short brief. Never poll subagents; results are delivered automatically.
- Claim "fixed" only after running the affected scenario and observing the result. Otherwise report "patch builds, behavior not verified" and what you will check. Never ask the user to retest an unverified guess.
- Debug by observation: reproduce first, then trace one failing event end to end before patching. After two failed hypotheses, change how you observe the bug, not what you patch.
- When a reference exists (Herdr TUI or state, upstream schema, previous behavior), check against it with a throwaway script and iterate until it matches, instead of reasoning about what it probably does.
- Stay on the requested outcome. List unrelated findings in your report instead of fixing them. Edit docs or existing tests only when the behavior they describe changed. Permanent verification tooling and architecture changes only when asked.
- Stop when the acceptance check passes: report instead of rewriting, optimizing or widening coverage. Delete your throwaway scripts first.
- Type, test and build failures are yours to fix. Stop only for an explicit user pause, a missing external prerequisite, or a decision that changes scope.
- Commit each verified coherent change and leave unrelated work untouched. Report the outcome, commits and anything still unverified in a few lines.
- Test Herdr mutations only in uniquely named disposable sessions (`python3 scripts/verify/ui_polish_runtime.py start|stop <root>`). Stop what you started; never touch the user's session or resources.
