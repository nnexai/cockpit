use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};

const MAX_PROTOCOL_LINE: usize = 8 * 1024;

struct AppState {
    helper: Mutex<Option<Arc<HelperProcess>>>,
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
struct Viewport {
    width: u32,
    height: u32,
    scale: f32,
    offset_x: f32,
    offset_y: f32,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    stream_url: String,
    sequence: u32,
    pixel_width: u32,
    pixel_height: u32,
    viewport: Viewport,
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
enum HelperRequestError {
    Rejected(String),
    Transport(String),
}

impl HelperRequestError {
    fn message(self) -> String {
        match self {
            Self::Rejected(message) | Self::Transport(message) => message,
        }
    }
}

struct HelperProcess {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<u64, mpsc::SyncSender<Result<Value, HelperRequestError>>>>>,
    next_id: AtomicU64,
}

impl HelperProcess {
    fn spawn() -> Result<Self, String> {
        let node = env::var("NODE_BINARY").unwrap_or_else(|_| "node".to_string());
        let helper_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("helper")
            .join("browser-helper.mjs");
        let mut command = Command::new(node);
        command
            .arg(helper_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
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
        let pending = Arc::new(Mutex::new(HashMap::<
            u64,
            mpsc::SyncSender<Result<Value, HelperRequestError>>,
        >::new()));
        let pending_reader = Arc::clone(&pending);
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
                // Stdout carries bounded JSON responses only. JPEG bytes travel
                // directly from the helper to the webview's loopback MJPEG image
                // stream, never through Tauri events or this reader.
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
                        response.get("result").cloned().ok_or_else(|| {
                            HelperRequestError::Transport(
                                "browser helper response omitted result".to_string(),
                            )
                        })
                    } else {
                        Err(HelperRequestError::Rejected(
                            response
                                .get("error")
                                .and_then(Value::as_str)
                                .unwrap_or("browser helper rejected the request")
                                .to_string(),
                        ))
                    };
                    let _ = waiter.send(result);
                }
                line.clear();
            }
            if let Ok(mut pending) = pending_reader.lock() {
                for (_, waiter) in pending.drain() {
                    let _ = waiter.send(Err(HelperRequestError::Transport(
                        "browser helper exited unexpectedly".to_string(),
                    )));
                }
            }
        });
        Ok(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending,
            next_id: AtomicU64::new(1),
        })
    }

    fn request(
        &self,
        method: &str,
        arguments: Option<Value>,
        timeout: Option<Duration>,
    ) -> Result<Value, HelperRequestError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut request = Map::new();
        request.insert("id".into(), json!(id));
        request.insert("method".into(), json!(method));
        if let Some(Value::Object(fields)) = arguments {
            request.extend(fields);
        }
        let encoded = serde_json::to_string(&Value::Object(request)).map_err(|error| {
            HelperRequestError::Rejected(format!("could not encode helper request: {error}"))
        })?;
        if encoded.len() > MAX_PROTOCOL_LINE {
            return Err(HelperRequestError::Rejected(
                "helper request exceeded protocol limit".to_string(),
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending
            .lock()
            .map_err(|_| {
                HelperRequestError::Transport("browser response state lock is poisoned".to_string())
            })?
            .insert(id, sender);
        let write_result = self
            .stdin
            .lock()
            .map_err(|_| {
                HelperRequestError::Transport("browser helper stdin lock is poisoned".to_string())
            })
            .and_then(|mut stdin| {
                writeln!(stdin, "{encoded}")
                    .and_then(|_| stdin.flush())
                    .map_err(|error| {
                        HelperRequestError::Transport(format!(
                            "could not write to browser helper: {error}"
                        ))
                    })
            });
        if let Err(error) = write_result {
            self.pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.remove(&id));
            return Err(error);
        }
        let response = match timeout {
            Some(limit) => receiver
                .recv_timeout(limit)
                .map_err(|_| {
                    HelperRequestError::Transport("browser helper response timed out".to_string())
                })
                .and_then(|result| result),
            None => receiver
                .recv()
                .map_err(|_| {
                    HelperRequestError::Transport(
                        "browser helper response channel closed".to_string(),
                    )
                })
                .and_then(|result| result),
        };
        if response.is_err() {
            self.pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.remove(&id));
        }
        response
    }
}

impl HelperProcess {
    fn terminate(&self) {
        let mut child = match self.child.lock() {
            Ok(child) => child,
            Err(_) => return,
        };
        #[cfg(unix)]
        unsafe {
            // spawn() made the helper the leader of a dedicated process group,
            // so an unresponsive helper cannot leave Chromium behind.
            let _ = libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

// The POC deliberately uses a synchronous stdio control boundary. Shutdown
// waits at most 500 ms for its polite stop response, then always kills/reaps it.

impl AppState {
    fn stop(&self) {
        let helper = match self.helper.lock() {
            Ok(mut slot) => slot.take(),
            Err(_) => return,
        };
        if let Some(helper) = helper {
            let _ = helper.request("stop", None, Some(Duration::from_millis(500)));
            helper.terminate();
        }
    }

    fn request(
        &self,
        _app: &AppHandle,
        method: &str,
        arguments: Option<Value>,
    ) -> Result<Value, String> {
        let helper = {
            let mut slot = self
                .helper
                .lock()
                .map_err(|_| "browser state lock is poisoned".to_string())?;
            match slot.as_ref() {
                Some(helper) => Arc::clone(helper),
                None => {
                    let helper = Arc::new(HelperProcess::spawn()?);
                    *slot = Some(Arc::clone(&helper));
                    helper
                }
            }
        };
        let result = helper.request(method, arguments, None);
        if matches!(&result, Err(HelperRequestError::Transport(_))) {
            let removed = self.helper.lock().ok().and_then(|mut slot| {
                if slot
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &helper))
                {
                    slot.take()
                } else {
                    None
                }
            });
            if let Some(removed) = removed {
                removed.terminate();
            }
        }
        result.map_err(HelperRequestError::message)
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
fn browser_input(
    app: AppHandle,
    state: State<'_, AppState>,
    event: Value,
) -> Result<InputAck, String> {
    if !event.is_object() || serde_json::to_vec(&event).map_or(true, |bytes| bytes.len() > 8 * 1024)
    {
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
fn browser_navigate(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
) -> Result<Snapshot, String> {
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
    eprintln!(
        "interactive-browser-panel-mjpeg-poc self-test {}: {detail}",
        if success { "passed" } else { "failed" }
    );
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            eprintln!("interactive-browser-panel-mjpeg-poc ready");
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
