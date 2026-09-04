fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "cockpit_status",
            "cockpit_sessions",
            "cockpit_session_snapshot",
            "cockpit_focus",
            "cockpit_mutate",
            "cockpit_session_subscribe",
            "cockpit_terminal_open",
            "cockpit_terminal_command",
            "cockpit_stream_cancel",
        ]),
    ))
    .expect("failed to build Tauri application manifest");
}
