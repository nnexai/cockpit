# Native Linux install

`scripts/install-native.py` builds Cockpit's Tauri binary, then installs only files owned by Cockpit. It never uses `sudo`, starts Cockpit, stops a running Cockpit process, or changes configuration.

Run this from the repository root:

```sh
python3 scripts/install-native.py
```

The default build invokes the local Tauri CLI as `bunx tauri build --no-bundle`. That builds the frontend and creates `target/release/cockpit-tauri`, without generating an AppImage, deb, or rpm bundle. The installer copies that binary to `$XDG_DATA_HOME/cockpit/bin/cockpit`, or `~/.local/share/cockpit/bin/cockpit` when `XDG_DATA_HOME` is unset.

`$XDG_BIN_HOME/cockpit`, or `~/.local/bin/cockpit` by default, is a stable symlink to that binary. Existing Cockpit processes retain their old executable image. The installer writes a new temporary binary beside the installed binary, syncs it, then atomically replaces the old file. New launches use the update without killing or restarting anything.

The desktop entry is `$XDG_DATA_HOME/applications/dev.cockpit.app.desktop` and the icon is `$XDG_DATA_HOME/icons/hicolor/256x256/apps/dev.cockpit.app.png`. The entry launches the same stable `cockpit` path. Log out and back in, or refresh the desktop environment, if a newly installed launcher does not appear immediately.

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

`--reuse` expects `target/release/cockpit-tauri`, or `target/debug/cockpit-tauri` with `--debug`.

Use a disposable prefix for an isolated verification. It contains its launcher in `PREFIX/bin` and its data, desktop entry, and icon under `PREFIX/share`:

```sh
python3 scripts/install-native.py --prefix /tmp/cockpit-native-check
```

The default mode also supports disposable XDG paths:

```sh
XDG_DATA_HOME=/tmp/cockpit-data XDG_BIN_HOME=/tmp/cockpit-bin \
  python3 scripts/install-native.py --reuse
```

Run the launcher directly after installation:

```sh
cockpit
```

For a prefix install, run `PREFIX/bin/cockpit`. The installer does not add any directory to `PATH`.

To remove an installation, use the same XDG values or prefix used to install it:

```sh
python3 scripts/install-native.py --uninstall
python3 scripts/install-native.py --prefix /tmp/cockpit-native-check --uninstall
```

Uninstall reads the install receipt and removes only the launcher, desktop entry, icon, installed binary, and receipt that it owns. It leaves configuration and changed replacement paths in place. An update also stops before replacing a changed installed file, so resolve that change or uninstall it before continuing.

## Space browser annotations

Open the selected Space's browser from Commands or its context menu. Cockpit uses the installed Playwright CLI and its default Chrome with a dedicated profile. The extension loads through Chrome's CDP extension API, without deprecated browser extension flags.

Extension updates use a distinct worker script URL and retain Chrome's local storage. Cockpit publishes new pairing credentials only after the bundle loads successfully. Restart Cockpit after installing an update so the running owner uses the new embedded assets; existing processes keep their previous binary.

On the page, draw or select an element, then add optional text beside the mark. Capture saves the visible page and annotations. Unrelated page updates do not require position review. After layout movement, use **Review positions**, or choose **Capture anyway (as shown)** to save the marks at their displayed positions. The override does not bypass navigation, wrong-tab, offscreen, or viewport changes during capture.

The extension popup retains unfinished drafts and exposes older documents under **Stale draft recovery**. Those marks are not attached to a replacement page. Failed submissions retain captured pixels for retry. Cockpit's feedback view sends selected annotations to the active tab's eligible agent; acknowledgement marks them handled without sending.
