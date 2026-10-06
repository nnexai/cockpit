# Native user-local install

`scripts/install-native.py` builds and installs both Cockpit surfaces: the graphical Tauri application and the `cockpit-cli` terminal tool, plus the bundled per-process OMP orchestration extension. It installs only files owned by Cockpit. It never uses `sudo`, starts Cockpit, stops a running Cockpit process, or changes Cockpit or global OMP configuration/authentication.

Run this from the repository root:

```sh
python3 scripts/install-native.py
```

On Linux, the default build invokes `bunx tauri build --no-bundle`, then builds the CLI with Cargo. It installs `target/release/cockpit-tauri` as `$XDG_DATA_HOME/cockpit/bin/cockpit` (default `~/.local/share/cockpit/bin/cockpit`) and `target/release/cockpit` beside it as `cockpit-cli`.

On macOS, the build uses `bunx tauri build --bundles app` and installs the complete `target/release/bundle/macos/Cockpit.app` bundle at `~/Applications/Cockpit.app`. The CLI is installed under `$XDG_DATA_HOME/cockpit/bin`, defaulting to `~/Library/Application Support/cockpit/bin`. The graphical launcher points into the installed bundle; no second raw graphical binary is installed.

Both platforms install `cockpit` and `cockpit-cli` launchers under `$XDG_BIN_HOME`, or `~/.local/bin` by default. Updates stage artifacts on their destination filesystems and journal publication and rollback. Individual replacements are atomic; a multi-artifact update is not one atomic filesystem operation. Existing processes are not stopped or restarted.

The installer also receipt-owns `$XDG_DATA_HOME/cockpit/omp/cockpit-orchestration.ts` (under `PREFIX/share/cockpit/omp/` for a prefix install). This is a Cockpit artifact, not a globally registered OMP extension. Supervisor/worker launches pass an explicit per-process `-e` path through Herdr `agent.start`; manually started OMP processes are not reconfigured. Hosts embed the same integration and, by default, materialize it in a private `<state_root>/orchestration/omp/` directory. `COCKPIT_OMP_EXTENSION` or `[orchestration] omp_extension` can select another explicit path. Existing OMP authentication is reused; install does not copy credentials, sign in, or edit OMP settings.

After changing browser paths or installing an update, fully restart any
existing `cockpit` or `cockpit serve` process before testing. Browser
configuration is loaded at startup; if another process already owns the
browser state root, the new process observes it and forwards browser
operations to that owner.

Linux desktop entries and icons are installed under the selected data directory at `applications/dev.cockpit.app.desktop` and `icons/hicolor/256x256/apps/dev.cockpit.app.png`. On macOS, launch the installed app bundle or use the `cockpit` shell launcher. Add the selected `bin` directory to `PATH` if needed.

## Native window settings

The native Tauri window reads optional presentation settings from the same
shared Cockpit TOML configuration file used by the core services:
`$XDG_CONFIG_HOME/cockpit/config.toml`, or `~/.config/cockpit/config.toml` when
`XDG_CONFIG_HOME` is unset.

```toml
[window]
scale_factor = 1.0
decorations = true
```

`scale_factor` controls the WebView page scale and must be finite and between
`0.2` and `10.0` (inclusive). `decorations` controls the native title bar and
borders. Without a `[window]` section, every platform defaults to scale `1.0`
with decorations enabled. These settings affect the native Tauri client only;
the browser client ignores them.

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

## Inline Space browser

Open the selected Space's browser from Commands or its context menu. The browser appears beside the Herdr layout. Browser tabs stay inside that split. **Expand browser** switches to a browser-only view and **Restore split** returns to the Herdr layout. **Close browser pane** hides the local browser presentation; **Close browser** closes the owned browser session and keeps its profile and saved feedback.

Cockpit attaches to the same named Playwright CLI browser that agents use. The packaged Node helper streams binary JPEG frames from Chromium. Install Node and Playwright CLI before opening a browser. The helper is embedded in the host binary. Configuration can override `[browser]` keys `playwright_cli`, `chromium_executable`, `node_executable`, `browser_helper`, and `playwright_core`. The last value identifies the Playwright-core package paired with the CLI. No second browser is launched for the inline view.
Browser prerequisites are checked lazily when the browser view is attached, not during
ordinary terminal startup. Missing or invalid settings name the corresponding
`COCKPIT_*` variable and `[browser]` key. The diagnostics distinguish a missing
paired package from an import failure, packaged-helper materialization from a
helper runtime failure, and unsupported facilities from transient browser state;
repair the named prerequisite and retry the browser view without restarting Herdr.

On initial attachment, an empty `about:blank` is replaced with a loopback-only
start page before screencast capture begins, guaranteeing a paintable first
frame. Non-empty pages and later user navigation are left unchanged.
The owned Playwright CLI daemon initializes a narrow native user-agent policy before page navigation. It reads the running Chromium's `Browser.getVersion.userAgent` and replaces only the `HeadlessChrome/` token with `Chrome/`; it does not pin a Chrome version. The policy applies to the initial page, CLI-created pages and popups, and embedded child frames including nested cross-site frames, before navigation; it remains with the CLI browser rather than the capture helper, including while the inline view is hidden. Chromium's executable, headless mode, launch/security/viewport settings, and JPEG capture are unchanged. This change does not claim general anti-bot acceptance, video/audio/DRM correctness, or performance parity.

Close and reopen Browser associations created before this policy so their saved launch configuration contains the daemon hook. Existing associations remain eligible for close and cleanup; they are not silently restarted.

Interact with the browser surface to acquire control; clicking a terminal returns keyboard control through Herdr. Address and annotation editors keep their own keyboard input. Other clients observe until they explicitly take control. Agents can still change the page through Playwright.

Use Browse, Select, Freehand, Region, or Element in the browser toolbar. Marks have a color and optional inline text; drawing or marking an element opens the inline note editor immediately. **Send annotations** composes the displayed page, marks, and comments as a PNG and delivers it to the active tab's eligible agent without pressing Enter. Failed saves retain the composed image for retry. Saved feedback remains available after closing the browser.

JPEG streaming has no audio. Browser-local notices report dialogs and unsupported browser facilities. Focused browser verification and an isolated Linux Tauri startup smoke passed on 2026-09-13; full browser/native acceptance, security, performance, and WebKit input/decode parity remain unclaimed.

The inline browser is the sole production browser path. Legacy extension draft migration is outside this replacement. Existing saved feedback remains readable.
