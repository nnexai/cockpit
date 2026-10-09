# WS-23 Orchestration CLI split

Wave 3 · Size M · Depends on: WS-13 · Blocks: –
Security-relevant (caller identity): one round of independent `security-reviewer` review (high-severity only).

## Goal
The agent CLI is a set of small modules. Caller-identity rules live once, in core.

## Owns
- `crates/cockpit-host/src/cli_orchestration.rs` → `cli_orchestration/{mod,args,context,caller,wait,retirement,output}.rs`
- the caller-identity predicates, moved into `crates/cockpit-core/src/orchestration/retirement.rs` (beside the existing `caller_location_matches`) or a new `orchestration/caller.rs`. WS-13 has merged by then, so this file is free.

## Evidence
- The file is 5,270 lines with ~100 functions. `run` (`~1910`) is 184 lines.
- The CLI defines its own predicates, `startup_launch_matches` (`~879`), `retirement_read_scope` (`~938`) and `caller_location_matches` (`~1011`). The last shares its name with core's `retirement.rs:~141`.
- `wait_next` is at `~2417`.

## Change
1. Move the code by concern.
2. Make `run` a thin dispatcher.
3. Unify the identity predicates in core: one function per rule, used by both the CLI and core.

## Keep
- CLI commands, flags, help text (diff the full `--help` tree before and after: it must be identical), exit codes and JSON output.
- Caller-evidence freshness.

## Acceptance
- No file over 1,200 lines.
- No function over 150 lines.
- One definition per caller-identity rule.
- Reviewer sign-off.

## Verify
- `cargo test -p cockpit-host`, `cargo test -p cockpit-core orchestration`.
- The help-tree diff.
- Disposable negative caller probes (`skill://cockpit-native-caller-negative-claim-matrix`): stale, forged and wrong-PID callers are still refused, and a genuine worker is still admitted.
