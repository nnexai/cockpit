# NATIVE-01 — owned macOS bundle installation

## Accepted plan

Main/Astra accepts this plan on 2026-09-21 against `6593cf4ecba87687c809ee9f3d2a295c757db787`. RUN-01 is complete. Current owned campaign edits remain protected. Original `tasks/NATIVE-01.md` criteria 1–6 are retained; OBS-013 explicitly assigns actual macOS execution to user verification. Lock: installer.

Inspected current InstallPaths, build arguments, schema-1/2 receipt loading, ownership checks, per-file atomic publication and uninstall. Current macOS flow incorrectly shares Linux --no-bundle/binary installation. Read issue #5's proposed patch as evidence; adapt it, do not copy its ownership assumptions.

Design: Darwin builds an app bundle (`--bundles app`) and installs `target/{debug,release}/bundle/macos/Cockpit.app`; Linux keeps --no-bundle and current binary/desktop behavior. --reuse and --debug select the same artifacts as a normal build. Add an explicit disposable bundle-destination option; default to the user's Applications location, and choose a deterministic prefix-local Applications location when --prefix is used. Print resolved targets. Never use sudo or force-stop a running process.

Validate Info.plist application identity and its declared executable, including safe relative path, containment and executable mode. Copy the whole bundle preserving permitted internal relative symlinks and modes; reject unsupported file types and escaping links. A receipt must bind the exact destination and a deterministic content/type/mode/link-target manifest, not only the main executable. Schema migration may preserve known old ownership but never adopt an already-existing foreign bundle or CLI path.

Publication: stage completely in the destination filesystem, verify the staged manifest, and journal the exact owned staging/backup/target identities before replacing anything. Directory backup/publish renames are NOT an atomic exchange. On handled failure restore a verified previous installation where possible; on interruption retain an explicit recoverable pending record, never a successful receipt. Subsequent recovery must verify journaled manifests/paths before moving or removing anything; modified/unknown states are refused. Include launcher/CLI publication in the success/rollback accounting so no successful receipt describes mixed generations. Use a bounded installer-specific transaction, not a general framework.

Update/uninstall checks all receipt-owned bundle content and modes before mutation and refuses foreign/modified artifacts. Preserve user config, borrowed paths and running processes. Keep schema-1/2 Linux receipt support and existing CLI-launcher behavior. No signing/notarization/auto-updater work.

## Ownership

Worker owns `scripts/install-native.py`, `scripts/test_install_native.py`, and installer-specific package command changes only if essential. Main owns documentation integration because WEB-07 changed `docs/native-install.md`; return precise proposed doc changes, do not edit that file concurrently. Do not edit Tauri runtime, browser helper or shared protocol. Python LSP is unavailable. Inspect existing installer test conventions and Tauri bundle identity before implementation. Skip every test/build/formatter/service/install/runtime command and commits during the writing wave.

## Verification and blockers

Per user OBS-011, long validation remains deferred. Original macOS checks are retained in [MACOS-HANDOFF.md](MACOS-HANDOFF.md): fresh install/two-build update and launched identity; wrong identity/foreign/modified refusal; publication failure with recognized previous or explicit recoverable state; safe migration/uninstall/config retention; installed startup/resource discovery. Their status is **user verification—not executed**, not passed.

OBS-013 supersedes the original missing-macOS blocker. Reachable delivery requires implementation, read-only ownership/publication review, final Python behavior checks, the real owned Linux prefix install/update/uninstall scenario, explicit macOS handoff and an owned commit. Do not acquire/emulate a runner or substitute Linux for Darwin proof. All verification destinations remain owned disposable roots; no installed user app or default Herdr session is touched.

## Implementation and bounded evidence

Darwin now uses a schema-4 bundle-only layout: the graphical launcher targets `Cockpit.app/Contents/MacOS/cockpit-tauri`; fresh installs contain no redundant raw GUI copy. Linux remains schema 3. Verified schema-1/2/3 Darwin raw binaries retire through journaled before=file/after=absent transactions with rollback, not direct deletion. This follows the bounded Astra launcher consultation; Main retains verification ownership.

The read-only installer review found and the worker repaired consistent lexical launcher identity, canonical-parent overlap checks, missing-owned-bundle refusal before deletion, and platform-neutral binary-only test fixtures. Prior ownership, staging cleanup and transaction findings were repaired as well.

Final delegated verification after those repairs: `TMPDIR=/tmp/csg-a3e9b950 python3 -m unittest discover -s scripts -p test_install_native.py` — **15 tests passed** (0.055s). Expected refusal stderr: `kept unrecognized path: binary`.

Real Linux CLI scenario, all successful: fresh `--reuse --debug --prefix /tmp/csg-a3e9b950/native-cli-smoke-final-20260921` install; installed `cockpit-cli --help`; repeat update; `--uninstall`; removal of that uniquely owned prefix. HOME/XDG paths were owned fixture paths. No service/GUI was launched. Evidence: [NATIVE-01-linux-evidence.json](NATIVE-01-linux-evidence.json). The existing debug binaries were exercised as installer artifacts, not asserted to include subsequent campaign source edits.

`docs/native-install.md` describes platform build/layout, XDG isolation and transaction limits. Its unrelated WEB-07 hunk is excluded from this increment's commit. No throwaway implementation or build artifact is added to the repository. macOS runtime results remain explicitly unexecuted; the adjusted delivery closes only with the owned installer commit recorded in `tasks.json`.
