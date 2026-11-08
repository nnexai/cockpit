# Native user-local install

`scripts/install-native.py` builds and installs both Cockpit surfaces: the graphical Tauri application and the `cockpit-cli` terminal tool. It installs only files owned by Cockpit, with no bundled OMP orchestration extension. It never uses `sudo`, starts Cockpit, stops a running Cockpit process, or changes Cockpit or global harness configuration/authentication.

Run this from the repository root:

```sh
python3 scripts/install-native.py
```

On Linux, the installer first builds the web frontend, then invokes `bunx tauri build --no-bundle` with both host and Tauri packages. The absolute `dist` path is passed as `COCKPIT_EMBED_WEB_DIST` when compiling the host, so its web gateway serves the matching embedded assets. It installs `target/release/cockpit-tauri` as `$XDG_DATA_HOME/cockpit/bin/cockpit` (default `~/.local/share/cockpit/bin/cockpit`) and `target/release/cockpit` beside it as `cockpit-cli`.

On macOS, the installer uses one `bunx tauri build --bundles app --config <bundle-mapping> -- --package cockpit-host --package cockpit-tauri` invocation. Tauri compiles both binaries in one Cargo feature graph before bundling, so the fresh CLI is included as `Cockpit.app/Contents/MacOS/cockpit-cli`, beside `cockpit-tauri`. It installs the complete bundle at `~/Applications/Cockpit.app` and the same CLI under `$XDG_DATA_HOME/cockpit/bin`, defaulting to `~/Library/Application Support/cockpit/bin`. Finder and Spotlight launches use the bundled CLI without a shell PATH or CLI override. The graphical launcher points into the installed bundle; no second raw graphical binary is installed.

The macOS packaging and lifecycle path is pending verification on a real Mac; Linux results do not establish macOS parity.

### Native Voice prerequisites

Native Voice uses real GStreamer WebRTC, ICE, DTLS, SCTP, Opus, audio conversion, platform audio input/output and WebRTC echo processing; there is no simulated-audio fallback. It needs a working microphone and audio server/devices as well as the plugins. Browser Voice is separate from this native media path.

On Fedora 44, the distribution package path for native Voice is:

```sh
# Build metadata (not needed to install an existing binary with --reuse):
sudo dnf install pkgconf-pkg-config gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-bad-free-devel
# Runtime elements and inspection tool:
sudo dnf install gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-good gstreamer1-plugins-bad-free libnice-gstreamer1
```

`libnice-gstreamer1` is separate from the libnice library and provides the ICE `nicesrc` element. PulseAudio-compatible input and output are required (including PipeWire's PulseAudio compatibility service). Fedora's [bad-free package](https://packages.fedoraproject.org/pkgs/gstreamer1-plugins-bad-free/gstreamer1-plugins-bad-free/fedora-44-updates.html) supplies WebRTC and echo processing; its [development package](https://packages.fedoraproject.org/pkgs/gstreamer1-plugins-bad-free/gstreamer1-plugins-bad-free-devel/fedora-44.html) supplies WebRTC pkg-config metadata.

On Debian/Ubuntu, the corresponding packages are `pkg-config`, `libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev`, `libgstreamer-plugins-bad1.0-dev`, `gstreamer1.0-tools`, `gstreamer1.0-plugins-base`, `gstreamer1.0-plugins-good`, `gstreamer1.0-plugins-bad` and `gstreamer1.0-nice`; install distribution versions providing GStreamer 1.18 or later. Package availability and plugin contents vary with distribution and release.

The installer checks the core, SDP and WebRTC pkg-config modules for GStreamer 1.18 or later before a build. It reports missing metadata and stops before building. For builds and `--reuse`, it also probes the required elements with `gst-inspect-1.0`; missing or uninspectable elements or a missing inspection tool stop installation before publication with package guidance. `--reuse` does not require development metadata; `--uninstall` runs neither check. Probe success reflects the installer's environment only, not successful device access or a Voice call. In particular an existing binary might link a different GStreamer installation. Voice reports missing required elements rather than falling back.

On macOS, the intended SDK path is the matching architecture's official [GStreamer runtime and development framework packages](https://gstreamer.freedesktop.org/download/). For a shell build, the [GStreamer macOS instructions](https://gstreamer.freedesktop.org/documentation/installing/on-mac-osx.html) use the framework tools and metadata:

```sh
export GST_ROOT=/Library/Frameworks/GStreamer.framework/Versions/1.0
export PATH="$GST_ROOT/bin:$PATH"
export PKG_CONFIG_PATH="$GST_ROOT/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
```

Inspect at least `webrtcbin`, `nicesrc`, `webrtcdsp`, `webrtcechoprobe`, `osxaudiosrc` and `osxaudiosink` with that framework's `gst-inspect-1.0`; the installer probes the full set. Do not assume the framework includes every element or mix framework and Homebrew installations. **macOS native Voice remains unverified**: the installer does not bundle the GStreamer framework or plugins, and shell `PATH`/`PKG_CONFIG_PATH` do not establish library and plugin discovery in a Finder-launched app. A real Mac must establish the linked framework/library paths and plugin discovery for the installed bundle, plus device permissions and an actual call; a bundle/discovery change may be required. The installer cannot establish those facts.

## Installation behavior

Both platforms install `cockpit` and `cockpit-cli` launchers under `$XDG_BIN_HOME`, or `~/.local/bin` by default. Updates stage artifacts on their destination filesystems and journal publication and rollback. Publication, recovery and uninstall take the persistent `$XDG_DATA_HOME/cockpit/update.lock` flock, also used by the service restart helper; a prefix install uses its own `PREFIX/share/cockpit/update.lock`. Individual replacements are atomic; a multi-artifact update is not one atomic filesystem operation. The installer waits up to 120 seconds for the lock and never unlinks it. Existing processes are not stopped or restarted.

Supervisor/worker launches use the existing file-backed `[orchestration]` harness kind, literal executable and argv; the default harness/executable is `omp`. No extension asset is installed, embedded or privately materialized. OMP resumes its registered conversation by default with `--resume`; other harnesses require configured replacement argv containing exactly one whole `{session_id}` argument. There is no implicit `--continue` or fallback harness. See [configuration](configuration.md#supervisor-launch-and-conversation-identity) for values, identity sources and optional prompt hints. The orchestration cutover has no legacy configuration aliases or store migration; unsupported store schemas fail without discarding bytes.

Existing configured-harness authentication is inherited under the existing launch policy: install does not copy credentials, sign in or edit harness settings. A start acknowledgement remains pending physical evidence; fresh matching agent/attachment observations are distinct from authenticated conversation registration and task ownership. Start reuses/reattaches the session's eligible personal liaison before creating one. Same-boot unknown consumers cannot authorize forced replacement, and semantic unassignment does not stop a worker or release its checkout/resource claims.

CLI selection uses an explicit `CliProcessRole::Host` or `CliProcessRole::Native`, not the process basename alone. A host may select its own executable. Native selects the paired CLI: the installed GUI named `cockpit` uses sibling `cockpit-cli` and must never launch itself as the CLI. In development, `cockpit-tauri` may use the distinct sibling `cockpit` host when `cockpit-cli` is absent. The installed pair needs no CLI-path override. Launch passes the selected CLI/configuration and private role capability plus generic Cockpit context environment. `cockpit-cli run context` registers the actual resumable conversation; eligible workers atomically claim their authorized task generation. Baseline inbox delivery is CLI wait/pull/durable capture and explicit ACK, not an OMP SDK. Default-off regular prompt hints opt into identity-checked text-plus-Enter submission and do not promise unknown-draft safety. Credentials narrow semantic mutations, not ordinary local tools, and are not an OS sandbox. The supervisor delegates with one WorkerStart and reviews source outcomes separately from Integration and Cleanup, without routine operator approvals.

The standalone `cockpit-cli` also embeds portable Notes and orchestration skills; no source checkout is required to read or install them. Native installation does not write agent skill directories. Inspect `cockpit-cli skills list` or `skills show NAME`, then explicitly use `skills install --home` for exactly `~/.agent/skills` (singular) or `skills install --project` for exactly `./.agents/skills` (plural). Omit names to select both bundled guides. Identical files remain untouched; differing files are preserved unless `--replace` is given. Unsafe targets are refused, and each skill has its own installation result. These are user-owned copies, not global agent configuration or receipt-owned native artifacts.

Starting a Cockpit surface starts the background service if needed. Closing the GUI or a web gateway does not stop it. Use the in-app restart offer after installation to restart the background service and relaunch the installed app; installation itself never does so. Browser work, comments and Review state are ephemeral and may clear when the service ends. Notes, Library, orchestration data and Herdr sessions are preserved.

An older app or gateway that still owns the Browser runtime must be quit explicitly before the new background service can start; the installer never signals it. Independently operated frozen gateways, including Tailscale deployments, are not updated or restarted by the installer.

The repository's **Install Cockpit** Herdr action builds and installs only. Service and GUI restart remain explicit user actions.

Linux desktop entries and icons are installed under the selected data directory at `applications/dev.cockpit.app.desktop` and `icons/hicolor/256x256/apps/dev.cockpit.app.png`. On macOS, launch the installed app bundle or use the `cockpit` shell launcher. Add the selected `bin` directory to `PATH` if needed.

For subscription-limit OMP discovery and executable overrides, see [configuration](configuration.md). “OMP usage source not found” means the CLI could not be launched, not that its Copilot usage JSON failed to parse. Restart Cockpit after changing configuration.

## Native window settings

The native Tauri window reads presentation settings from the shared Cockpit TOML. The browser client ignores them. See [configuration](configuration.md) for the `[window]` example, defaults and allowed scale bounds.

Native Commands offers **Stay awake while agents work**, enabled by default and remembered in local storage. The GUI holds an inhibitor only while an observed agent is working: Linux uses the desktop ScreenSaver D-Bus inhibitor and macOS uses an IOKit power assertion; closing Cockpit releases it. **Turn screen off** uses niri on Linux and `pmset` on macOS, without stopping agents. Controls unavailable in the current native environment are hidden; the browser client does not offer them. The macOS paths are unverified.

For a quicker repeat build, use the Tauri debug profile:

```sh
python3 scripts/install-native.py --debug
```

Repeat installs with unchanged build inputs (including Git commit provenance) reuse the compiled native binary. The frontend still type-checks and builds each time, but restores timestamps for byte-identical output when the output file set is unchanged. Changed bytes keep fresh timestamps; added or removed output files invalidate the entire embedded asset set.

To install an existing build, skip the build step:

```sh
python3 scripts/install-native.py --reuse
python3 scripts/install-native.py --reuse --debug
```

On Linux, `--reuse` expects `target/release/cockpit-tauri` and `target/release/cockpit`. The host must already have been built with `COCKPIT_EMBED_WEB_DIST` set to the absolute path of the matching built frontend; `--reuse` does not rebuild or verify embedded assets. On macOS it also requires the complete `target/release/bundle/macos/Cockpit.app`, including an executable `Contents/MacOS/cockpit-cli` matching `target/release/cockpit`, as well as the matching native binary. Bundles built without the paired CLI are refused; rebuild with this installer rather than adding files to a signed bundle. `--debug` selects the corresponding `target/debug` paths. A reused binary's build commit is not inferred from the current checkout and is recorded as unknown.

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

Uninstall uses the receipt to remove only owned launchers, desktop files, application artifacts and CLI. macOS receipts include the bundle's files, modes and symlink identities. Updates and uninstall refuse modified artifacts or path substitutions; configuration, runtime orchestration/task records, harness authentication/settings and unrelated files remain untouched. Verified legacy raw-binary macOS installations migrate through the same journaled transaction rather than an untracked deletion.

An older Linux schema-3 receipt may also own `omp/cockpit-orchestration.ts`. Updates authenticate that retired asset against its recorded path and SHA-256, then remove it through the same journaled publication/rollback; new installs and receipts omit it. Modified assets or substituted ownership paths are refused, and unrecorded extension files remain untouched. This does not migrate configuration or runtime task state.

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
