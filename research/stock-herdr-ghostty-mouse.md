# Stock Herdr + Ghostty mouse experiment

**Verdict: FAIL for button delivery through `herdr terminal attach`; PASS for ordinary input.**

A real X11 click in an independent Ghostty window running stock Herdr terminal attach did not reach the fixture's PTY. The same fixture in a standalone Ghostty received the same kind of click (press and release), proving the GUI/X path and fixture mode were working.

## Owned scenario and versions

- Scenario ledger: `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/ghostty-attach-mouse/resources.json`
- Run-owned session: `run-20260904T214621Z-gm2` (ledger final status `stopped`)
- Private root: `/tmp/cp-gm-7f3c9a` (0700); private display `:192`
- Herdr: `/home/linuxbrew/.linuxbrew/Cellar/herdr/0.8.2/bin/herdr`, version `0.8.2`, SHA-256 `450cb7b1c67fa8c8312653917bbe1cd89a5223c2248e6c87f680907ade93f621`
- Ghostty: `/home/nnex/.local/bin/ghostty`, `Ghostty 1.1.3`
- Isolated X tools: `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/native-tools/usr/bin/Xvfb` and `xdotool`

The guard was exercised before effects. The ledger loaded successfully; missing-session, `default`, and foreign-run target attempts were rejected. Server startup and workspace creation were planned through `scripts/verify/resource_guard.py` before dispatch.

## Fixture and controls

The retained `mouse_feedback.py` fixture under the evidence directory is a bounded Python app (SHA-256 `de836d03c34a0bc75cbaa3965e29289b3c4814c8ccc16080e17024feebba0121`). It enables SGR mouse mode (`ESC[?1002h` + `ESC[?1006h`), visibly prints `MOUSE-FEEDBACK READY`, decodes SGR button/coordinates, records timestamped raw hex and decoded JSONL events, and treats `k` as a harmless ordinary-input control. Captured input is never executed.

## Reproduction

### Standalone baseline

Ghostty was launched with `-e python3 .../mouse_feedback.py` on `DISPLAY=:192`. The verified window was ID `2097157`, geometry `1000x600` at `(0,0)`. Actual GUI input used:

```text
xdotool windowfocus 2097157 key k
xdotool mousemove --window 2097157 300 250 click 1
```

`standalone-events.jsonl` records `ready`, `control` (`k`, count 1), and both:

```text
mouse press button=0 x=30 y=9
mouse release button=0 x=30 y=9
```

**Standalone: PASS** for ordinary key and click.

### Fresh Herdr session and attach

The stock server created exactly one workspace/tab/pane:

- workspace `w1`, tab `w1:t1`, pane `w1:p1`
- terminal identity `term_65ab876782be61`
- `pane get` title: `MOUSE-FEEDBACK-READY`
- `pane process-info` showed the fixture process and exact argv
- fixture startup log recorded `1002+1006`

A second independent Ghostty ran this exact command:

```text
herdr --session run-20260904T214621Z-gm2 terminal attach term_65ab876782be61
```

`herdr terminal attach --help` exposes only `--takeover`; there is no mouse-forwarding flag. The direct-attach implementation hardcodes host mouse capture on, so `[ui] mouse_capture = false` is not a button-forwarding switch for this path. `--takeover` only changes attach ownership.

After its verified window was ready, actual X input was sent only to that window:

```text
xdotool windowfocus <window> key k
xdotool mousemove --window <window> 300 250 click 1
xdotool windowfocus <window> mousemove --window <window> 500 300 mousedown 1 sleep 0.1 mouseup 1
```

`attached-events.jsonl` contains `ready` and `control` (`k`, count 1), but no mouse event. `attached-raw.jsonl` contains only raw `6b` (the `k` control), with no mouse bytes.

**Attached: PASS** ordinary-input positive control; **FAIL** click delivery.

## Why this is an attach filtering result

Primary source is the pinned stock Herdr 0.8.2 source archive at `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/herdr-0.8.2`:

- `src/client/mod.rs:349-356` documents direct attach as forwarding stdin to the attached PTY and enabling mouse capture.
- `src/client/mod.rs:129-169` filters attach input; `src/client/mod.rs:174-228` converts wheel reports to `AttachScroll`, but returns `None` for non-wheel mouse events.
- `src/client/mod.rs:1523-1557` forwards only `AttachInputAction::Forward`; button events therefore do not become PTY bytes.
- `src/app/mod.rs:1816-1822` routes captured mouse through Herdr when `state.mouse_capture` is true; `src/app/state.rs:1637-1655` shows that this is an independent capture condition.

Thus app mode, GUI injection, attach readiness/ownership, and key input are all positively exercised; the missing button receipt is consistent with stock attach's non-wheel mouse filtering, not a Ghostty or fixture failure. No pre-encoded SGR bytes were injected and no source or production code was changed.

## Evidence and cleanup

Raw logs, process/launcher logs, ledger, fixture, and manifest are retained outside Git:

- `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/ghostty-attach-mouse/resources.json` (SHA-256 `3656417ec8a0e95e83d42b487157f7061a41e8d01f0bc43fd75a8601f2e58afd`); manifest `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/ghostty-attach-mouse/experiment-manifest.txt` (SHA-256 `88dd261c5199af8a8df56d537d3b0254d2960d7dcdea3ab4a1228671fc70bfe8`)
- `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/ghostty-attach-mouse/standalone-events.jsonl` (SHA-256 `6b95a1f2f1cb16b55a8ee57e22fc05613f48c46f7da7a11fdccb483c17605b0b`)
- `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/ghostty-attach-mouse/standalone-raw.jsonl` (SHA-256 `b0ff0d6395839bea8cbdf20f2ec3949d32b9fcd8f418a07576761006a57da24c`)
- `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/ghostty-attach-mouse/attached-events.jsonl` (SHA-256 `572f171c308d33d720bb3b6aab7077e590f29f20ceddc2bf767a0146cfb9e65d`)
- `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/ghostty-attach-mouse/attached-raw.jsonl` (SHA-256 `49e7c6c7211a58189ac1c0790924187523b6e82d88d523ec791897966178ce12`)

The owned session was stopped with the guarded exact `session stop` command; both Ghostty clients had exited; the owned Herdr server was stopped; Xvfb `:192` was stopped. Final checks found no owned processes, no `/tmp/.X11-unix/X192`, and no owned session socket. No project validation commands were run.
