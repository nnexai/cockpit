# Native user-local install

`scripts/install-native.py` builds and installs both Cockpit surfaces: the graphical Tauri application and the `cockpit-cli` terminal tool. It installs only files owned by Cockpit. It never uses `sudo`, starts Cockpit, stops a running Cockpit process, or changes configuration.

Run this from the repository root:

```sh
python3 scripts/install-native.py
```

The default build invokes the local Tauri CLI as `bunx tauri build --no-bundle`, then builds the CLI with Cargo. It creates `target/release/cockpit-tauri` and `target/release/cockpit`. The installer copies the graphical binary to `$XDG_DATA_HOME/cockpit/bin/cockpit`, or `~/.local/share/cockpit/bin/cockpit` on Linux and `~/Library/Application Support/cockpit/bin/cockpit` on macOS. The CLI is installed beside it as `cockpit-cli`.

`$XDG_BIN_HOME/cockpit` and `$XDG_BIN_HOME/cockpit-cli`, or `~/.local/bin/cockpit` and `~/.local/bin/cockpit-cli` by default, are stable symlinks to the installed binaries. Existing Cockpit processes retain their old executable image. The installer writes new temporary binaries beside the installed binaries, syncs them, then atomically replaces the old files. New launches use the update without killing or restarting anything.

After changing browser paths or installing an update, fully restart any
existing `cockpit` or `cockpit serve` process before testing. Browser
configuration is loaded at startup; if another process already owns the
browser state root, the new process observes it and forwards browser
operations to that owner.

The desktop entry is written under the selected data directory at `applications/dev.cockpit.app.desktop`, and the icon at `icons/hicolor/256x256/apps/dev.cockpit.app.png`. Linux desktop environments consume these files. On macOS, the user-local binaries work from a shell; add the selected `bin` directory to `PATH` if it is not already present.

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

To install an already built binary, skip the build step:

```sh
python3 scripts/install-native.py --reuse
python3 scripts/install-native.py --reuse --debug
```

`--reuse` expects `target/release/cockpit-tauri` and `target/release/cockpit`, or their `target/debug` counterparts with `--debug`.

Use a disposable prefix for an isolated verification. It contains both launchers in `PREFIX/bin` and its data, desktop entry, and icon under `PREFIX/share`:

```sh
python3 scripts/install-native.py --prefix /tmp/cockpit-native-check
```

The default mode also supports disposable XDG paths:

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

Uninstall reads the install receipt and removes only the launchers, desktop entry, icon, installed application and CLI binaries, and receipt that it owns. It leaves configuration and changed replacement paths in place. An update also stops before replacing a changed installed file, so resolve that change or uninstall it before continuing.

## Inline Space browser

Open the selected Space's browser from Commands or its context menu. The browser appears beside the Herdr layout. Browser tabs stay inside that split. **Expand browser** switches to a browser-only view and **Restore split** returns to the Herdr layout. **Close browser pane** hides the local browser presentation; **Close browser** closes the owned browser session and keeps its profile and saved feedback.

Cockpit attaches to the same named Playwright CLI browser that agents use. The packaged Node helper streams binary JPEG frames from Chromium. Install Node and Playwright CLI before opening a browser. The helper is embedded in the host binary. Configuration can override `[browser]` keys `playwright_cli`, `chromium_executable`, `node_executable`, `browser_helper`, and `playwright_core`. The last value identifies the Playwright-core package paired with the CLI. No second browser is launched for the inline view.

On initial attachment, an empty `about:blank` is replaced with a loopback-only
start page before screencast capture begins, guaranteeing a paintable first
frame. Non-empty pages and later user navigation are left unchanged.

Interact with the browser surface to acquire control; clicking a terminal returns keyboard control through Herdr. Address and annotation editors keep their own keyboard input. Other clients observe until they explicitly take control. Agents can still change the page through Playwright.

Use Browse, Select, Freehand, Region, or Element in the browser toolbar. Marks have a color and optional inline text; drawing or marking an element opens the inline note editor immediately. **Send annotations** composes the displayed page, marks, and comments as a PNG and delivers it to the active tab's eligible agent without pressing Enter. Failed saves retain the composed image for retry. Saved feedback remains available after closing the browser.

JPEG streaming has no audio. Browser-local notices report dialogs and unsupported browser facilities. Focused browser verification and an isolated Linux Tauri startup smoke passed on 2026-09-13; full browser/native acceptance, security, performance, and WebKit input/decode parity remain unclaimed.

The inline browser is the sole production browser path. Legacy extension draft migration is outside this replacement. Existing saved feedback remains readable.
