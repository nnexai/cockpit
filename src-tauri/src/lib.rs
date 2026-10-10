mod clipboard;
mod commands;
mod startup;
mod streams;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use cockpit_core::HerdrAdapter;
use cockpit_host::transport::shutdown::{HostShutdown, ShutdownPolicy};
use tauri::Manager;

use startup::NativeServices;
use streams::StreamRegistry;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let NativeServices {
        service,
        browser_runtime,
        orchestration_runtime,
        shutdown_projects,
        library,
        library_sync_config,
        window_config,
        startup_inspector,
    } = startup::compose();
    let mut library_sync_runtime = Some(startup::start_library_sync(&library, library_sync_config));
    let shutdown_started = Arc::new(AtomicBool::new(false));
    let shutdown = HostShutdown::new(
        Some(orchestration_runtime.clone()),
        Some(shutdown_projects),
        Some(browser_runtime.clone()),
    );
    tauri::Builder::default()
        .manage(service)
        .manage(browser_runtime)
        .manage(orchestration_runtime)
        .manage(StreamRegistry::new())
        .setup(move |app| {
            let main_window = app
                .get_webview_window("main")
                .expect("main window is missing from the Tauri configuration");
            main_window
                .set_zoom(window_config.scale_factor)
                .expect("failed to apply configured window scale factor");
            main_window
                .set_decorations(window_config.decorations)
                .expect("failed to apply configured window decorations");
            #[cfg(target_os = "linux")]
            {
                use gtk::prelude::WidgetExt as _;

                // The opaque webview paints the content; skip GTK's redundant background fill.
                main_window
                    .gtk_window()
                    .expect("failed to access the native GTK window")
                    .set_app_paintable(true);
            }
            main_window
                .show()
                .expect("failed to show the configured native window");
            tauri::async_runtime::spawn(async move {
                if let Err(error) = startup_inspector.inspect().await {
                    eprintln!("failed to initialize Herdr: {error}");
                }
            });
            Ok(())
        })
        .invoke_handler(commands::invoke_handler())
        .build(tauri::generate_context!())
        .expect("error while building Cockpit Tauri application")
        .run(move |app, event| {
            if matches!(
                &event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                drop(library_sync_runtime.take());
            }
            if let tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } = event
            {
                api.prevent_exit();
                if shutdown_started.swap(true, Ordering::AcqRel) {
                    return;
                }
                let shutdown = shutdown.clone();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    shutdown.run(ShutdownPolicy::Native).await;
                    app.exit(0);
                });
            }
        });
}
