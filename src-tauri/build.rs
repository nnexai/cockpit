fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "cockpit_status",
            "cockpit_project_configuration",
            "cockpit_repositories",
            "cockpit_workspace_plan",
            "cockpit_workspace_start",
            "cockpit_workspace_operation",
            "cockpit_workspace_resume",
            "cockpit_workspace_cancel",
            "cockpit_workspace_reconcile",
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
