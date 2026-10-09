use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};

const MAX_PROTOCOL_LINE: usize = 8 * 1024 * 1024;

struct AppState {
    helper: Mutex<Option<HelperProcess>>,
    self_test: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            helper: Mutex::new(None),
            self_test: matches!(env::var("POC_SELF_TEST").as_deref(), Ok("1")),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    jpeg_data_url: String,
    width: u32,
    height: u32,
    title: String,
    url: String,
    cursor: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InspectResult {
    status: String,
    fixture_status: String,
    title: String,
    url: String,
    active_element: String,
    selected_text: String,
    scroll_y: u32,
    cursor: String,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InputAck {
    cursor: String,
}

#[derive(Serialize)]
struct StopResult {
    status: &'static str,
}

struct HelperProcess {
    child: Child,
    stdin: ChildStdin,
    pending: Arc<Mutex<HashMap<u64, mpsc::SyncSender<Result<Value, String>>>>>,
    next_id: u64,
}

impl HelperProcess {
    fn spawn(app: &AppHandle) -> Result<Self, String> {
        let node = env::var("NODE_BINARY").unwrap_or_else(|_| "node".to_string());
        let helper_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("helper")
            .join("browser-helper.mjs");
        let mut child = Command::new(node)
            .arg(helper_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("could not start browser helper: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "browser helper stdin was unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "browser helper stdout was unavailable".to_string())?;
        let pending = Arc::new(Mutex::new(HashMap::<u64, mpsc::SyncSender<Result<Value, String>>>::new()));
        let pending_reader = Arc::clone(&pending);
        let app = app.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            while let Ok(read) = reader.read_line(&mut line) {
                if read == 0 {
                    break;
                }
                if line.len() > MAX_PROTOCOL_LINE {
                    line.clear();
                    continue;
                }
                let response: Value = match serde_json::from_str(&line) {
                    Ok(value) => value,
                    Err(_) => {
                        line.clear();
                        continue;
                    }
                };
                if response.get("event").and_then(Value::as_str) == Some("screencastFrame") {
                    if let Some(frame) = response.get("frame").cloned() {
                        let _ = app.emit("interactive-browser-panel-screencast-frame", frame);
                    }
                    line.clear();
                    continue;
                }
                let Some(id) = response.get("id").and_then(Value::as_u64) else {
                    line.clear();
                    continue;
                };
                let waiter = pending_reader
                    .lock()
                    .ok()
                    .and_then(|mut pending| pending.remove(&id));
                if let Some(waiter) = waiter {
                    let result = if response.get("ok").and_then(Value::as_bool) == Some(true) {
                        response
                            .get("result")
                            .cloned()
                            .ok_or_else(|| "browser helper response omitted result".to_string())
                    } else {
                        Err(response
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("browser helper rejected the request")
                            .to_string())
                    };
                    let _ = waiter.send(result);
                }
                line.clear();
            }
            if let Ok(mut pending) = pending_reader.lock() {
                for (_, waiter) in pending.drain() {
                    let _ = waiter.send(Err("browser helper exited unexpectedly".to_string()));
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            pending,
            next_id: 1,
        })
    }

    fn request(&mut self, method: &str, arguments: Option<Value>) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let mut request = Map::new();
        request.insert("id".into(), json!(id));
        request.insert("method".into(), json!(method));
        if let Some(Value::Object(fields)) = arguments {
            request.extend(fields);
        }
        let encoded = serde_json::to_string(&Value::Object(request))
            .map_err(|error| format!("could not encode helper request: {error}"))?;
        if encoded.len() > MAX_PROTOCOL_LINE {
            return Err("helper request exceeded protocol limit".to_string());
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending
            .lock()
            .map_err(|_| "browser response state lock is poisoned".to_string())?
            .insert(id, sender);
        if let Err(error) = writeln!(self.stdin, "{encoded}").and_then(|_| self.stdin.flush()) {
            self.pending.lock().ok().and_then(|mut pending| pending.remove(&id));
            return Err(format!("could not write to browser helper: {error}"));
        }
        receiver
            .recv()
            .map_err(|_| "browser helper response channel closed".to_string())?
    }
}

impl HelperProcess {
    fn terminate(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// The POC deliberately uses a synchronous stdio boundary; termination is bounded
// only by the helper/browser process exit and is not a production channel design.

impl AppState {
    fn stop(&self) {
        let helper = match self.helper.lock() {
            Ok(mut slot) => slot.take(),
            Err(_) => return,
        };
        if let Some(helper) = helper {
            let mut helper = helper;
            let _ = helper.request("stop", None);
            helper.terminate();
        }
    }

    fn request(&self, app: &AppHandle, method: &str, arguments: Option<Value>) -> Result<Value, String> {
        let mut slot = self
            .helper
            .lock()
            .map_err(|_| "browser state lock is poisoned".to_string())?;
        if slot.is_none() {
            *slot = Some(HelperProcess::spawn(app)?);
        }
        let result = slot
            .as_mut()
            .expect("helper was inserted above")
            .request(method, arguments);
        let failed_helper = if result.is_err() { slot.take() } else { None };
        drop(slot);
        if let Some(helper) = failed_helper {
            helper.terminate();
        }
        result
    }
}

fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("invalid browser helper result: {error}"))
}

#[tauri::command]
fn browser_start(app: AppHandle, state: State<'_, AppState>) -> Result<Snapshot, String> {
    match state.request(&app, "start", None).and_then(decode) {
        Ok(snapshot) => Ok(snapshot),
        Err(error) => {
            state.stop();
            Err(error)
        }
    }
}

#[tauri::command]
fn browser_snapshot(app: AppHandle, state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.request(&app, "snapshot", None).and_then(decode)
}

#[tauri::command]
fn browser_input(app: AppHandle, state: State<'_, AppState>, event: Value) -> Result<InputAck, String> {
    if !event.is_object() || serde_json::to_vec(&event).map_or(true, |bytes| bytes.len() > 8 * 1024) {
        return Err("input event must be a bounded JSON object".to_string());
    }
    state
        .request(&app, "input", Some(json!({ "event": event })))
        .and_then(decode)
}

#[tauri::command]
fn browser_reload(app: AppHandle, state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.request(&app, "reload", None).and_then(decode)
}

#[tauri::command]
fn browser_navigate(app: AppHandle, state: State<'_, AppState>, url: String) -> Result<Snapshot, String> {
    if url.len() > 2048 {
        return Err("URL is too long".to_string());
    }
    state
        .request(&app, "navigate", Some(json!({ "url": url })))
        .and_then(decode)
}

#[tauri::command]
fn browser_inspect(app: AppHandle, state: State<'_, AppState>) -> Result<InspectResult, String> {
    state.request(&app, "inspect", None).and_then(decode)
}

#[tauri::command]
fn browser_stop(state: State<'_, AppState>) -> Result<StopResult, String> {
    state.stop();
    Ok(StopResult { status: "stopped" })
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
    if success {
        eprintln!("interactive-browser-panel-poc self-test passed: {detail}");
    } else {
        eprintln!("interactive-browser-panel-poc self-test failed: {detail}");
    }
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            eprintln!("interactive-browser-panel-poc ready");
            if app.state::<AppState>().self_test {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(750));
                    let _ = handle.emit("interactive-browser-panel-self-test", ());
                });
            }
            Ok(())
        })
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            browser_start,
            browser_snapshot,
            browser_input,
            browser_reload,
            browser_navigate,
            browser_self_test_enabled,
            browser_self_test_report,
            browser_inspect,
            browser_stop,
        ])
        .build(tauri::generate_context!())
        .expect("error while building interactive browser panel")
        .run(|app, event| {
            if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) {
                app.state::<AppState>().stop();
            }
        });
}
