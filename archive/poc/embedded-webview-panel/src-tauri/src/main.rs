use std::env;
use std::time::Duration;
use tauri::{Emitter, Manager, State};

struct AppState {
    self_test: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            self_test: matches!(env::var("POC_SELF_TEST").as_deref(), Ok("1")),
        }
    }
}


#[tauri::command]
fn browser_self_test_enabled(state: State<'_, AppState>) -> bool {
    state.self_test
}

#[tauri::command]
fn browser_self_test_report(
    state: State<'_, AppState>,
    success: bool,
    detail: String,
) -> Result<(), String> {
    if !state.self_test {
        return Err("POC_SELF_TEST is not enabled".to_string());
    }
    if detail.len() > 512 {
        return Err("self-test detail is too long".to_string());
    }
    let outcome = if success { "passed" } else { "failed" };
    eprintln!("embedded-webview-panel-poc self-test {outcome}: {detail}");
    Ok(())
}


fn main() {
    tauri::Builder::default()
        .setup(|app| {
            eprintln!("embedded-webview-panel-poc ready (native Tauri/Wry WebView)");
            if app.state::<AppState>().self_test {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(750));
                    let _ = handle.emit("embedded-webview-panel-self-test", ());
                });
            }
            Ok(())
        })
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            browser_self_test_enabled,
            browser_self_test_report,
        ])
        .build(tauri::generate_context!())
        .expect("error while building embedded WebView panel")
        .run(|_, _| {});
}
