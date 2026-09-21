# macOS verification handoff

## Status and scope

**User verification — not executed on macOS.** OBS-013 explicitly assigns these checks to the user. Linux checks and source review do not establish Darwin filesystem, AppKit/WebKit, input, scaling or bundle runtime behavior. Use the final delivered source revision, not an earlier campaign checkpoint. Record `git rev-parse HEAD`, macOS version/architecture, Herdr version, window size and display scale with results.

This checklist preserves the original macOS portions of NATIVE-01, SETUP-02 and NATIVE-02. It is not a claim that those gestures passed. Browser/Linux-native evidence remains separate in the task records.

## Safe build and isolated launch

From the delivered checkout, using your normal development toolchain:

```sh
CHECK_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/cockpit-macos-check.XXXXXX")"
export CHECK_ROOT
python3 scripts/install-native.py --debug --prefix "$CHECK_ROOT/install"
"$CHECK_ROOT/install/bin/cockpit-cli" --help
```

The expected bundle is `$CHECK_ROOT/install/Applications/Cockpit.app`. The graphical launcher must resolve into `Contents/MacOS/cockpit-tauri`, not a second raw executable under Application Support. `--reuse --debug` requires the complete debug bundle and CLI build. An alternate owned bundle destination can be supplied with `--application-path`.

Before starting **either** the fixture Herdr server or Cockpit, isolate their state in the same shell environment:

```sh
mkdir -p "$CHECK_ROOT/home" "$CHECK_ROOT/config" "$CHECK_ROOT/data" "$CHECK_ROOT/cache" "$CHECK_ROOT/state" "$CHECK_ROOT/repositories"
export HOME="$CHECK_ROOT/home"
export XDG_CONFIG_HOME="$CHECK_ROOT/config"
export XDG_DATA_HOME="$CHECK_ROOT/data"
export XDG_CACHE_HOME="$CHECK_ROOT/cache"
export XDG_STATE_HOME="$CHECK_ROOT/state"
export COCKPIT_HERDR_SESSION="cockpit-mac-$(basename "$CHECK_ROOT")"
export COCKPIT_REPOSITORY_ROOTS="$CHECK_ROOT/repositories"
export COCKPIT_WORKTREE_ROOT="$CHECK_ROOT/worktrees"
export COCKPIT_COMPANION_ROOT="$CHECK_ROOT/companions"
export COCKPIT_STATE_ROOT="$CHECK_ROOT/operations"
unset COCKPIT_HERDR_SOCKET COCKPIT_CONFIG HERDR_CONFIG_PATH
```

Use the same values in both terminals. Start `herdr --session "$COCKPIT_HERDR_SESSION" server` in one; launch `"$CHECK_ROOT/install/bin/cockpit"` in the other. Do not use a working session, default user profile, installed production app, or `/Applications/Cockpit.app`. Start fixture processes from a disposable repository. An unavailable dependency should be reported rather than worked around by using a protected session.

Put credential-free test configuration at `$XDG_CONFIG_HOME/cockpit/config.toml`. Provider configuration is a top-level TOML array, for example:

```toml
[[providers]]
id = "glab"
base_url = "https://gitlab.com"
executable = "glab"
```

Authentication remains owned by the installed CLI/keyring. Configure the CLI for the isolated environment through its supported mechanism; never put a token in Cockpit config, evidence, argv or an environment dump. Terminal-only checks require no provider authentication.

## Installer and bundle — NATIVE-01

- [ ] Fresh prefix install launches the intended complete bundle; compare the built and installed executable with `shasum -a 256`, inspect Info.plist identity and verify resources resolve from the installed bundle.
- [ ] Build a distinguishable second version and update the same prefix. Launcher, CLI, bundle and receipt describe one installed generation. The installer does not stop the existing process; quit that owned old process deliberately before verifying a fresh launch of the update.
- [ ] In an additional disposable prefix, an unrelated bundle, wrong application identity, escaping executable/symlink, modified bundle file or missing receipt-owned bundle causes refusal before other owned artifacts are changed. Restore the exact owned fixture before cleanup.
- [ ] Exercise a handled publication failure/interruption using a disposable installation. Outcome is the verified previous installation or explicit recoverable journal state, never an installed receipt for mixed generations. Do not improvise destructive fault injection on a real installation.
- [ ] A verified legacy raw-binary receipt migrates to the bundle-only layout without claiming a foreign bundle; unknown/modified legacy artifacts remain untouched.
- [ ] Update/uninstall preserve config and unrelated sentinels. Run `python3 scripts/install-native.py --prefix "$CHECK_ROOT/install" --uninstall` with the same bundle override, if used. This must remove only receipt-owned artifacts.

## Companion setup and recovery — SETUP-02

- [ ] Inspect a source URL and review its exact plan without creating a worktree, companion, Space or terminal. Creation starts only after explicit approval.
- [ ] Create an owned worktree/companion: the final directory appears complete through Darwin no-replace publication; manifest and operation receipt match the authoritative workspace.
- [ ] Existing and concurrently created foreign destinations, including empty directories and symlinks, remain unchanged. Unsupported/known parent prerequisites fail before Herdr creation.
- [ ] Interrupt before publication, after publication and before receipt acknowledgement in an owned scenario. Resume validates exact ownership, retries publication durability, does not duplicate a successful workspace/terminal and does not overwrite local generated-file edits.
- [ ] Permission/path replacement/cross-device failures leave actionable partial state and exact recovery actions. Borrowed directories and unrelated sentinels survive retry and teardown.

Darwin durability calls remain a platform-specific risk requiring this run. If the filesystem rejects the primitive or durability operation, record the exact diagnostic and retained resources; do not delete a destination or bypass the check to force success.

## Daily-use native surface — NATIVE-02

Run at **1440×900 and 1024×640**, where supported; record actual CSS/device scale.

| Area | Expected observable result | Result |
| --- | --- | --- |
| Compatibility and request storm (#1) | Compatible Herdr patch release works; rapid Space/tab changes do not create hidden attach/request loops. | User verification—not executed |
| Terminal continuity/alignment (#4) | Split/zoom/restore, resize and output preserve visible rows and scrollback; glyph/Nerd Font fallback aligns; input goes only to the confirmed pane. | User verification—not executed |
| Plugin roots and rendering (#2) | Real Files/Context and Review plugin panes use approved cwd/root and process generation; navigation/scroll survive pending and failed loads. | User verification—not executed |
| Bundle startup (#5) | Fresh launch uses the installed updated bundle and its resources. | User verification—not executed |
| Setup/partial state (#7) | Owned and borrowed workflows publish/recover/teardown with exact ownership and no replacement. | User verification—not executed |
| Browser first use | First measured click fires once; first Element pick works without a Browse gesture; stale/refused outcomes are visible and not replayed. | User verification—not executed |
| Native input | Pointer/drag-out/release, X/Y wheel, modifiers, repeat, AltGr/dead keys, Unicode/IME and clipboard arrive once; local editors remain local. | User verification—not executed |
| Focus/takeover | Terminal→browser→terminal requires authoritative ownership; two clients have one controller and no duplicate terminal bytes or forced focus reclaim. | User verification—not executed |
| Image/geometry | Resize, display scale, zoom, scrolling and letterbox preserve frame/descriptor/pointer alignment; stale geometry cannot enable input/capture. | User verification—not executed |
| Draft/capture/delivery | Hide/show, navigation, reconnect and close preserve or explicitly recover unsent work; delayed old receipts do not erase newer text; retry uses frozen bytes/IDs and paste does not submit. | User verification—not executed |
| Provider flow | Issue and standalone MR resolve/setup/import/list/refresh work; same-SHA metadata/discussion change is detected and local edits remain conflicts. | User verification—not executed |
| Local failures/cleanup | Missing browser prerequisites do not break terminal-only use; failures remain resource-local; detached views release channels/decoders/input and do not stop another owner. | User verification—not executed |

## GitLab fixtures

The owned fixture ledger is [gitlab-fixtures.json](gitlab-fixtures.json): project `nnex.ai/integration` (86672117), [issue #2](https://gitlab.com/nnex.ai/integration/-/work_items/2), [standalone draft MR !1](https://gitlab.com/nnex.ai/integration/-/merge_requests/1). Source branch: `cockpit-fixture/csg-a3e9b950-20260921`; baseline SHA: `8493e6e72e56df62006b084eb211f2f5156390e0`. Prepare an authorized disposable local checkout/ref before setup; Cockpit must not silently clone/fetch it. Keep MR refresh changes in title/body/discussions so SHA stays fixed. Do not merge, approve, alter protected main, or delete remote resources as part of this handoff.

## Evidence and cleanup

Record per scenario PASS/FAIL with build identity, action, authoritative response/event, visible result and exact error. Keep issue and MR, browser and native, and successful versus refused cases separate. A startup screenshot alone is insufficient for input/geometry/recovery.

Stop only the named fixture session (`herdr --session "$COCKPIT_HERDR_SESSION" server stop`) and processes you started. Uninstall the owned prefix, inspect retained journals/partial resources before deleting anything, and preserve evidence outside the removed prefix. Report failures against the owning campaign task; do not conceal them with retries, ordinary rename fallbacks or forced cleanup.
