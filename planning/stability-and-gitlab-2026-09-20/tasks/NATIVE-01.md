# NATIVE-01 — Install and update the macOS application bundle

Status, dependencies, owner, locks and completion proof: [task ledger](../tasks.json). Follow the [orchestrator contract](../ORCHESTRATOR.md).

## Outcome

A normal macOS development installation launches the freshly built Cockpit app bundle, while updates/uninstall refuse foreign or modified artifacts and Linux installation behavior stays intact.

## Evidence and starting points

[GitHub #5](https://github.com/nnexai/cockpit/issues/5) contains an unintegrated proposed patch. Current `scripts/install-native.py` always builds with `--no-bundle`, installs binaries, and uses receipt schema 2. Read `scripts/test_install_native.py`, `docs/native-install.md`, `src-tauri/tauri.conf.json`, and current CLI/config conventions. The attached patch is evidence, not a safe wholesale replacement.

## Changes

1. Review/adapt the patch against current ownership and receipt semantics. On macOS, build/use the `.app` bundle; preserve Linux binary-only behavior. Make the chosen installation target visible and support a disposable bundle destination for proof.
2. Stage and publish the complete bundle safely. Validate bundle identity/executable and destination ownership; define interruption/rollback behavior explicitly. A pair of renames is not automatically an atomic directory exchange.
3. Extend receipt/version handling for bundle identity and content integrity, including relevant executable modes/symlinks. Keep existing receipts usable without granting ownership of foreign files.
4. Refuse changed/foreign installed bundles; update/uninstall only receipt-owned unchanged artifacts. Preserve running processes and user configuration. Never use sudo or overwrite the live `/Applications/Cockpit.app` during verification.
5. Make `--reuse`, debug/release paths and a fresh normal install consistent; document actual macOS and Linux commands once verified. Preserve helper/runtime dependency discovery outside the source tree.

## Non-goals

No signing/notarization/distribution pipeline, auto-updater, forced process restart, or modification of the user's installed app. Do not claim that Linux mocked platform branches prove macOS bundle behavior.

## Acceptance

1. Fresh macOS install into a disposable app location launches the intended build; a second distinct build updates the bundle and launcher without leaving a stale executable/resources mix.
2. Wrong bundle identifier, foreign destination and post-install user modification are refused without damaging either source or destination.
3. Injected publication failure retains a usable recognized prior installation or an explicit recoverable state, with no successful receipt for a failed install.
4. Existing receipt migration does not acquire deletion rights over unowned paths; uninstall removes only proven owned artifacts and preserves config.
5. Linux prefix install/update/uninstall remains correct.
6. Native startup from the installed disposable bundle finds required resources; a real macOS launch, not only compilation, is recorded.

## Verification

After integration run `python3 scripts/test_install_native.py` with behavior-focused cases for ownership, update failure and receipt compatibility. Perform actual install/update/launch/uninstall on macOS using a disposable destination and fixture Herdr session, plus a Linux disposable-prefix smoke. Inspect bundle build identity from the launched process. Missing macOS execution leaves this task blocked at verification; do not touch the real installed application to work around it. Read-only filesystem/installer review is required before committing.

## Handoff

Record exact OS/build/source and bundle identities, tested destinations, failure-injection outcomes and owned cleanup in `runs/<run-id>/NATIVE-01.md`. Commit installer, affected tests/docs and evidence only; update task ledger with real SHA. #5 closure evidence must state the macOS path was actually exercised.
