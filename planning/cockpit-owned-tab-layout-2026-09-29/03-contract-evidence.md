# Planning contract evidence

Scope: implementation planning only. Product code/configuration unchanged. No product acceptance is claimed.

## Disposable Herdr split probe

Used `scripts/verify/ui_polish_runtime.py start`, which created owned fixture `/tmp/cpol-23ecgv3j`; all RPCs below targeted that fixture only. Its snapshot reported Herdr 0.9.2, protocol 22. Stopped the fixture with the script after the probe; only recorded matching fixture processes were stopped.

Commands:

```sh
python3 scripts/verify/ui_polish_runtime.py rpc /tmp/cpol-23ecgv3j pane.split '{"target_pane_id":"w1:p1","direction":"right","focus":true}'
python3 scripts/verify/ui_polish_runtime.py rpc /tmp/cpol-23ecgv3j pane.list '{"tab_id":"w1:t1"}'
python3 scripts/verify/ui_polish_runtime.py rpc /tmp/cpol-23ecgv3j session.snapshot
python3 scripts/verify/ui_polish_runtime.py stop /tmp/cpol-23ecgv3j
```

Observed split result (identity-bearing fields shown; cwd/status/scroll/revision also returned):

```json
{
  "id": "polish-proof",
  "result": {
    "type": "pane_info",
    "pane": {
      "pane_id": "w1:p2",
      "terminal_id": "term_65ca641775fa62",
      "workspace_id": "w1",
      "tab_id": "w1:t1",
      "focused": true
    }
  }
}
```

`pane.list` returned `type: "pane_list"` with two members, `w1:p1` and `w1:p2`. The subsequent snapshot independently confirmed `w1:p2`, terminal `term_65ca641775fa62`, workspace `w1`, tab `w1:t1`. Thus the current raw split result provides a creation identity which the Cockpit adapter can validate and retain instead of discarding. Implementers must validate identity/membership against current ordered state and retain unknown-outcome handling; focus is not creation proof.

The observed array order does not establish a documented semantic member-order guarantee. The initial Cockpit grid will use deterministic ordering of confirmed stable pane IDs, not Herdr geometry or layout-order hints. No concurrent creation race was exercised in this probe.

## Approved interactive reference

[`mocks/tab-layout/demo.html`](mocks/tab-layout/demo.html), commit `6aa0331`, is the approved interaction/appearance reference, not a backend implementation.

Before user approval, browser smoke exercised actual pointer-driven centre swap, edge reposition, divider resizing; right/down splitting, viewer reuse, zoom/restore, external equal-share column insertion, last-terminal closure of viewers, and tab isolation. Browser errors were empty at the end of that smoke. User then explicitly approved drag/drop, resizing, appearance and control icons; approval is recorded in [`00-contract.md`](00-contract.md).

## Limits

- This planning probe verifies raw split-result identity and membership, not the new Cockpit layout or browser integration.
- No observer/control arbitration probe was repeated: the user's stated control-only sizing behavior is an accepted requirement.
- Files/Review authorization, tab-scoped browser isolation, durable draft migration, focus reconciliation and native parity remain implementation acceptance scenarios in the plan.
- No product build, tests, formatter or new product runtime scenario was run during this planning phase.
