# Resume

## Delivery boundary

S00 commit: `41101de4ef26e8e92754a173979a526e61026344`. BOOT production code passed 84 Rust tests, Tauri check/host build, 57 frontend tests/typecheck/build, independent Terra review, and actual refreshed browser/native input smokes. G00B is PASS; strict paired inventories agree. All final inventory findings are repaired. The actual paired protocol negative changed from exit0/PASS to exit1/FAIL; protected-default collector invocation exits2 before command plans. Python suite:44 passed (31 BOOT guard/inventory,13 pending temporal).

NEXT: stage final owned BOOT edits/reports, inspect staged diff, commit, and record the delivery hash. Then advance S02. Do not commit the still-unverified temporal tools with BOOT. `__pycache__` is now ignored. Current staged production paths are owned by this run; initial tree was clean.

## Temporal integration repairs

Read `agent://FinalTemporalReview`; Main owns these bounded repairs. Do not launch another broad rewrite.
- In `scripts/verify/terminal_temporal.py`, delete unconditional `viewport_hashes = None` after successful viewport hash calculation.
- Make the paired-control consumer accept the actual successful text-only replay emitted by this CLI. It currently expects a separate report kind/shape that no operation produces. No hand-authored control proof.
- Require nonempty string display resource_id and fixture_id; bind capture/replay/control fixture identity. Add a required capture fixture argument if needed.
- Run actual capture/replay CLI and focused regressions after repairs. Full-frame RGB hashes have a streaming path (`FrameSample.pixel_sha256`). X11 `-copyts` already produces epoch PTS; no speculative clock flags.

G01 remains INCONCLUSIVE: paired control, remaining workloads/three replays, and stable application mouse coordinates are not complete. Application mouse is an explicit public-API block; no Herdr-server changes or guessed global/SGR coordinates. Continue independent selected stages after terminal preparation; do not stop at this block.

## Runtime and durable evidence

Durable directory: `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/`.
- AppImage `Cockpit-bootstrap-stable-0.1.0-amd64.AppImage`, SHA `ca1ce374f6293accc64efbe92f47430ae5c0de4692f8494c64f8829cd935e064`. `NO_STRIP=1` avoids linuxdeploy's incompatible Fedora `.relr.dyn` strip operation. Actual native WebKit window rendered the fixture and captured exact `NATIVE-REFRESH-456\r`.
- Browser captured exact `BOOT-REFRESH-é-漢字-789\r`. Both refreshed smokes used wB:p1 / `term_65ab120aec94db`. Fixture SHA `982f8795707194ed4c2ae0c65cc7bd32389d0fd75b6c1353a863e468d64aaef8`. Refreshed screenshots, receipts and raw capture are retained.
- `browser-startup-host-evidence.json`, `native-startup-host-evidence.json`, and paired startup inventories contain complete identities. `bootstrap-tested-production-hashes.json` explicitly records dirty base41101de. G00B contains final source/test hashes and review resolution.
- `scroll-record-one.mkv`, raw trace, packet PTS, framehash, references A/B, measured report and driver source:3601 frames over60000ms;16/17ms gaps;100 distinct trusted wheel inputs;43ms p95; all offsets matched. Every full-frame RGB hash was exactly A or B. Away-tail intervals stayed pixel-identical while21 and20 lines arrived. This was Main's bounded driver/ffmpeg run, NOT the repaired terminal-temporal CLI.
- Full-frame ffprobe timed out at30s during concurrent decoding; packet PTS and full framehash decode succeeded. Not a product failure.
- Fixture repair: partial scroll region accumulated no history; full-screen512-line prefill does. Nonblocking stdin aliased stdout and lost writes; raw TTY now stays blocking and output handles short writes. PTY smoke received all512 prefill lines.

## Owned resources and constraints

Homebrew Herdr0.8.2/protocol20 is pinned; the conflicting local executable is archived outside PATH. Protected `default` is untouched. Disposable session `run-20260904T214621Z-boot`, root `/tmp/cockpit-214621Z-gqhguyl2`.
- `cockpit-run-herdr-boot`: running.
- `cockpit-run-web`: running at `127.0.0.1:38339`.
- `cockpit-run-xvfb`: ready on `:191`; typed receipt in resources.json.
- `cockpit-run-native` and `cockpit-run-chromium-presented`: stopped. Managed presented CDP connection released. Original managed browser tab remains about:blank.

Native launch spec is retained in `hub describe cockpit-run-native`; restart reuses the exact AppImage and isolated environment. Chromium's owned process can likewise be restarted and attached through CDP40261; observe the actual target before attaching.

Fixtures last300s. Prepare drivers before creating a fresh guarded workspace. Pane IDs are alphanumeric (`wA`, `wB`), not decimal-only. All mutations/cleanup require exact ledger ownership and the pinned executable. User override OBS-004: WebUI behavior first; native only startup/simple AppImage compatibility. Follow timeline15 through every selected story; bootstrap is not the stopping point.
