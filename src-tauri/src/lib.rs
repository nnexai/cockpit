mod clipboard;
mod commands;
mod power;
mod startup;
mod streams;
mod update;

use std::sync::Arc;

use cockpit_core::HerdrAdapter;
use cockpit_host::transport::{Transport, operations::backend::BackendContext, shutdown::HostShutdown};
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
        library: _,
        window_config,
        startup_inspector,
        backend_state_root,
        backend_cli,
        herdr_endpoint,
    } = startup::compose();
    let backend_context = Arc::new(BackendContext::new(
        backend_state_root,
        backend_cli.clone().unwrap_or_else(|| "cockpit-cli".into()),
        herdr_endpoint,
        Transport::Native,
        None,
    ));
    let shutdown = HostShutdown::new(
        Some(orchestration_runtime.clone()),
        Some(shutdown_projects),
        Some(browser_runtime.clone()),
    );
    let shutdown = Arc::new(update::NativeShutdown::new(shutdown));
    tauri::Builder::default()
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .manage(service)
        .manage(browser_runtime)
        .manage(orchestration_runtime)
        .manage(backend_context.clone())
        .manage(shutdown.clone())
        .manage(clipboard::TerminalFilesState::default())
        .manage(power::PowerState::default())
        .manage(StreamRegistry::new())
        .on_permission_request(|_, kind| match kind {
            tauri::webview::PermissionKind::Microphone
            | tauri::webview::PermissionKind::Camera
            | tauri::webview::PermissionKind::DisplayCapture => tauri::webview::PermissionResponse::Deny,
            _ => tauri::webview::PermissionResponse::Default,
        })
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Started {
                webview.state::<StreamRegistry>().page_started(webview.label());
            }
        })
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
            let backend_context = backend_context.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(cli) = backend_cli {
                    if let Err(error) = cockpit_host::backend::ensure(
                        &backend_context.state_root, &cli, &backend_context.herdr,
                    ).await {
                        eprintln!("Cockpit background service unavailable: {error}");
                    }
                }
            });
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
            if matches!(&event, tauri::RunEvent::Exit) {
                app.state::<clipboard::TerminalFilesState>().close();
            }
            if let tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } = event
            {
                api.prevent_exit();
                let shutdown = shutdown.clone();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    shutdown.exit(app).await;
                });
            }
        });
}
