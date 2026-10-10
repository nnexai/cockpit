# Native user-local install

`scripts/install-native.py` builds and installs both Cockpit surfaces: the graphical Tauri application and the `cockpit-cli` terminal tool, plus the bundled per-process OMP orchestration extension. It installs only files owned by Cockpit. It never uses `sudo`, starts Cockpit, stops a running Cockpit process, or changes Cockpit or global OMP configuration/authentication.

Run this from the repository root:

```sh
python3 scripts/install-native.py
```

On Linux, the default build invokes `bunx tauri build --no-bundle`, then builds the CLI with Cargo. It installs `target/release/cockpit-tauri` as `$XDG_DATA_HOME/cockpit/bin/cockpit` (default `~/.local/share/cockpit/bin/cockpit`) and `target/release/cockpit` beside it as `cockpit-cli`.

On macOS, the build uses `bunx tauri build --bundles app` and installs the complete `target/release/bundle/macos/Cockpit.app` bundle at `~/Applications/Cockpit.app`. The CLI is installed under `$XDG_DATA_HOME/cockpit/bin`, defaulting to `~/Library/Application Support/cockpit/bin`. The graphical launcher points into the installed bundle; no second raw graphical binary is installed.

Both platforms install `cockpit` and `cockpit-cli` launchers under `$XDG_BIN_HOME`, or `~/.local/bin` by default. Updates stage artifacts on their destination filesystems and journal publication and rollback. Individual replacements are atomic; a multi-artifact update is not one atomic filesystem operation. Existing processes are not stopped or restarted.

The installer receipt-owns `$XDG_DATA_HOME/cockpit/omp/cockpit-orchestration.ts` (under `PREFIX/share/cockpit/omp/` for a prefix install). This is a Cockpit artifact, not a globally registered OMP extension. Supervisor/worker launches pass an explicit per-process `-e` path through Herdr `agent.start`; manually started OMP processes are not reconfigured. Hosts embed the same integration and materialize it privately by default; see [configuration](configuration.md) for extension-path overrides.

Existing OMP authentication is reused: install does not copy credentials, sign in or edit OMP settings. A start acknowledgement remains pending until the runtime confirms fresh actual OMP evidence and main SDK-session binding before Active/Launched; working is valid and interactive-ready is not required.

CLI selection uses an explicit `CliProcessRole::Host` or `CliProcessRole::Native`, not the process basename alone. A host may select its own executable. Native selects the paired CLI: the installed GUI named `cockpit` uses sibling `cockpit-cli` and must never launch itself as the CLI. In development, `cockpit-tauri` may use the distinct sibling `cockpit` host when `cockpit-cli` is absent. The installed pair needs no CLI-path override. The bound main SDK uses the launch-selected CLI/configuration for inbox pull/ACK and supervisor actions, rather than an older ambient-PATH installation. This preserves the GUI/CLI process boundary while the authorized supervisor manages descendant worker preparation, execution and successful-result acceptance without routine operator grants.

The standalone `cockpit-cli` also embeds portable Notes and orchestration skills; no source checkout is required to read or install them. Native installation does not write agent skill directories. Inspect `cockpit-cli skills list` or `skills show NAME`, then explicitly use `skills install --home` for exactly `~/.agent/skills` (singular) or `skills install --project` for exactly `./.agents/skills` (plural). Omit names to select both bundled guides. Identical files remain untouched; differing files are preserved unless `--replace` is given. Unsafe targets are refused, and each skill has its own installation result. These are user-owned copies, not global agent configuration or receipt-owned native artifacts.

After changing browser paths or installing an update, fully restart any
existing `cockpit` or `cockpit serve` process before testing. Browser
configuration is loaded at startup; if another process already owns the
browser state root, the new process observes it and forwards browser
operations to that owner.

Linux desktop entries and icons are installed under the selected data directory at `applications/dev.cockpit.app.desktop` and `icons/hicolor/256x256/apps/dev.cockpit.app.png`. On macOS, launch the installed app bundle or use the `cockpit` shell launcher. Add the selected `bin` directory to `PATH` if needed.

For subscription-limit OMP discovery and executable overrides, see [configuration](configuration.md). “OMP usage source not found” means the CLI could not be launched, not that its Copilot usage JSON failed to parse. Restart Cockpit after changing configuration.

## Native window settings

The native Tauri window reads presentation settings from the shared Cockpit TOML. The browser client ignores them. See [configuration](configuration.md) for the `[window]` example, defaults and allowed scale bounds.

For a quicker repeat build, use the Tauri debug profile:

```sh
python3 scripts/install-native.py --debug
```

To install an existing build, skip the build step:

```sh
python3 scripts/install-native.py --reuse
python3 scripts/install-native.py --reuse --debug
```

On Linux, `--reuse` expects `target/release/cockpit-tauri` and `target/release/cockpit`. On macOS it requires the complete `target/release/bundle/macos/Cockpit.app` and the CLI binary. `--debug` selects the corresponding `target/debug` paths.

A disposable prefix contains both launchers in `PREFIX/bin` and installer data under `PREFIX/share`. On macOS its bundle is installed at `PREFIX/Applications/Cockpit.app`:

```sh
python3 scripts/install-native.py --prefix /tmp/cockpit-native-check
```

On macOS, `--application-path /absolute/path/Cockpit.app` selects a different bundle destination, including with `--prefix`. Use the same selection for update and uninstall. Existing unowned or modified bundles are refused, not adopted or removed.

On Linux, the default mode also supports disposable XDG paths. On macOS, use `--prefix` or an explicit `--application-path` as well to keep the bundle out of `~/Applications`:

```sh
XDG_DATA_HOME=/tmp/cockpit-data XDG_BIN_HOME=/tmp/cockpit-bin \
  python3 scripts/install-native.py --reuse
```

Run the graphical application or CLI directly after installation:

```sh
cockpit
cockpit-cli browser status --current
```

For a prefix install, run `PREFIX/bin/cockpit` or `PREFIX/bin/cockpit-cli`. The installer does not add any directory to `PATH`.

To remove an installation, use the same XDG values or prefix used to install it:

```sh
python3 scripts/install-native.py --uninstall
python3 scripts/install-native.py --prefix /tmp/cockpit-native-check --uninstall
```

Uninstall uses the receipt to remove only owned launchers, desktop files, application artifacts, CLI and bundled OMP extension. macOS receipts include the bundle's files, modes and symlink identities. Updates and uninstall refuse modified artifacts or path substitutions; configuration, runtime orchestration/task records, OMP authentication/settings and unrelated files remain untouched. Verified legacy raw-binary macOS installations migrate through the same journaled transaction rather than an untracked deletion.

## Tab-local Browser

The tab-local inline Browser is the production browser surface.

Open Browser for a selected Herdr tab from Commands or that tab's context menu. It is a leaf in that tab's local layout, not a Space-wide sidecar or a separate Chromium instance for the inline view. A fresh Browser leaf closes and cleans the previous association before opening a replacement; incomplete cleanup blocks replacement. Reconnect can attach to the existing live association instead.

Hiding Browser or switching away detaches capture and input, but leaves the managed daemon and disposable profile alive. Explicit close stops the owned session; confirmed tab retirement and owner shutdown also clean up owned resources. Only proven association artifacts are removed, after shutdown is confirmed. Uncertain shutdown or cleanup stays visible and retryable: an absent pane is not proof of cleanup.

The profile, annotation drafts, pending captures and feedback are tab-association/current-owner-run work, not a retained archive. Confirmed Browser cleanup discards them. A new exclusive runtime owner stops proven leftover sessions before resetting ephemeral state; observer processes do not perform that reset. Other browser profiles, unrelated files and Herdr sessions are not cleanup targets.

### Prerequisites and streaming

Install Node and Playwright CLI before opening Browser. Cockpit uses the same named Playwright CLI browser that agents use; the packaged Node helper streams binary JPEG frames from Chromium and is embedded in the host binary. See [configuration](configuration.md) for CLI, Chromium, Node, helper and paired Playwright-core overrides, initial URL and feedback limits.

Prerequisites are checked lazily on view attachment, not ordinary terminal startup. Missing or invalid settings name the corresponding `COCKPIT_*` variable and `[browser]` key. Diagnostics distinguish a missing paired package from import failure, helper materialization from runtime failure, and unsupported facilities from transient browser state. Repair the named prerequisite and retry the view without restarting Herdr.

On initial attachment, an empty `about:blank` is replaced with a loopback-only start page before screencast capture, guaranteeing a paintable first frame. Non-empty pages and later navigation are unchanged.

The owned CLI daemon applies a narrow user-agent policy before navigation: it reads Chromium's `Browser.getVersion.userAgent` and replaces only `HeadlessChrome/` with `Chrome/`, without pinning a version. The hook covers initial pages, CLI-created pages, popups and nested cross-site frames. It belongs to the daemon, so it remains active while the inline view is hidden.

Associations opened without the daemon hook require explicit close and fresh open to acquire the policy. They remain eligible for guarded close/cleanup and are not silently restarted.

Chromium's executable, headless mode, launch/security/viewport settings and JPEG capture are unchanged. This policy does not claim general anti-bot acceptance, video/audio/DRM correctness or performance parity. JPEG streaming has no audio; Browser-local notices report dialogs and unsupported facilities.

### Control and annotations

Interact with the Browser surface to acquire control; clicking a terminal returns keyboard control through Herdr. Address and annotation editors keep their own input. Other clients observe until they explicitly take control; agents can still change the page through Playwright. Input requires current control and presented-frame/document/viewport evidence; stale input is rejected, not replayed.

Use Browse, Select, Freehand, Region or Element in the toolbar. Marks have a color and optional inline text; drawing or marking an element opens the inline note editor immediately. Drafts are page/document-bound. **Send annotations** composes the displayed page, marks and comments as a PNG and pastes feedback to the tab's eligible agent without pressing Enter.

Failed saves retain the composed image for explicit retry during the current run. Pending or unknown delivery receipts require inspection/reconciliation, not automatic replay; a new send after uncertainty requires explicit duplicate-risk acknowledgement. Saved feedback is subject to configured retention and confirmed association cleanup, not preserved after Browser close.

See [CODE_GUIDE](../CODE_GUIDE.md) for lifecycle, cleanup, delivery and stream owners, and the [verification log](verification-log.md) for historical smoke results and their limitations. This installation guide asserts no new browser/native acceptance result.
