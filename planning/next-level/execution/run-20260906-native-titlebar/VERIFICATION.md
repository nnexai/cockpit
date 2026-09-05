# Native title bar removal

`src-tauri/tauri.conf.json` sets `decorations: false`. No frontend controls, transport behavior, or session ownership changed.

`bun run tauri:build --bundles appimage` passed, including frontend typecheck/build and optimized native build. The updated executable AppImage is `target/release/bundle/appimage/Cockpit_0.1.0_amd64.AppImage`.

Native smoke used a run-owned Xvfb display `:199` and nested niri 26.04 with configuration/runtime under `/tmp/ck-title-20260906`. The real AppImage displayed a 1280×800 window with zero window offset inside its tile. `borderless.png` and `windows.json` retain the rendered window and compositor evidence: app content reaches the top edge with no GTK title bar. The smoke inspected window decoration, not connected terminal behavior; its initial isolated Herdr server was unavailable.

The bundled linuxdeploy GTK hook forces `GDK_BACKEND=x11`, so the successful packaged smoke ran through the nested compositor's XWayland display `:1`. A forced pure-Wayland launch could not initialize GTK because that hook overrides the backend. No claim of native Wayland transport is made.

Only the named disposable Herdr session `ck-title-20260906` and run-owned display/application processes were used. They were stopped after verification. User niri configuration, default Herdr, and the user's running Cockpit window were untouched.
