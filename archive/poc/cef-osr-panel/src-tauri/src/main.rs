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
    fn default() -> Self { Self { helper: Mutex::new(None), self_test: matches!(env::var("POC_SELF_TEST").as_deref(), Ok("1")) } }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Snapshot { jpeg_data_url: String, width: u32, height: u32, title: String, url: String, cursor: String, sequence: u64 }
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InspectResult { status: String, fixture_status: String, title: String, url: String, active_element: String, selected_text: String, scroll_y: u32, cursor: String }
#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InputAck { cursor: String }
#[derive(Serialize)]
struct StopResult { status: &'static str }

struct HelperProcess {
    child: Child,
    stdin: ChildStdin,
    pending: Arc<Mutex<HashMap<u64, mpsc::SyncSender<Result<Value, String>>>>>,
    next_id: u64,
}
impl HelperProcess {
    fn spawn(app: &AppHandle) -> Result<Self, String> {
        let node = env::var("NODE_BINARY").unwrap_or_else(|_| "node".to_string());
        let helper_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("helper").join("cef-helper.mjs");
        let mut child = Command::new(node).arg(helper_path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().map_err(|error| format!("could not start CEF helper: {error}"))?;
        let stdin = child.stdin.take().ok_or_else(|| "CEF helper stdin unavailable".to_string())?;
        let stdout = child.stdout.take().ok_or_else(|| "CEF helper stdout unavailable".to_string())?;
        let pending = Arc::new(Mutex::new(HashMap::<u64, mpsc::SyncSender<Result<Value, String>>>::new()));
        let pending_reader = Arc::clone(&pending);
        let app = app.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if line.len() > MAX_PROTOCOL_LINE { continue; }
                let Ok(response) = serde_json::from_str::<Value>(&line) else { continue; };
                if response.get("event").and_then(Value::as_str) == Some("cefFrame") {
                    if let Some(frame) = response.get("frame").cloned() { let _ = app.emit("cef-osr-panel-frame", frame); }
                    continue;
                }
                let Some(id) = response.get("id").and_then(Value::as_u64) else { continue; };
                let waiter = pending_reader.lock().ok().and_then(|mut pending| pending.remove(&id));
                if let Some(waiter) = waiter {
                    let result = if response.get("ok").and_then(Value::as_bool) == Some(true) {
                        response.get("result").cloned().ok_or_else(|| "CEF helper omitted result".to_string())
                    } else { Err(response.get("error").and_then(Value::as_str).unwrap_or("CEF helper rejected request").to_string()) };
                    let _ = waiter.send(result);
                }
            }
            if let Ok(mut pending) = pending_reader.lock() { for (_, waiter) in pending.drain() { let _ = waiter.send(Err("CEF helper exited unexpectedly".to_string())); } }
        });
        Ok(Self { child, stdin, pending, next_id: 1 })
    }
    fn request(&mut self, method: &str, arguments: Option<Value>) -> Result<Value, String> {
        let id = self.next_id; self.next_id = self.next_id.saturating_add(1);
        let mut request = Map::new(); request.insert("id".into(), json!(id)); request.insert("method".into(), json!(method));
        if let Some(Value::Object(fields)) = arguments { request.extend(fields); }
        let encoded = serde_json::to_string(&Value::Object(request)).map_err(|error| format!("could not encode CEF request: {error}"))?;
        if encoded.len() > MAX_PROTOCOL_LINE { return Err("CEF request exceeded protocol limit".to_string()); }
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending.lock().map_err(|_| "CEF pending state lock poisoned".to_string())?.insert(id, sender);
        if let Err(error) = writeln!(self.stdin, "{encoded}").and_then(|_| self.stdin.flush()) {
            self.pending.lock().ok().and_then(|mut pending| pending.remove(&id));
            return Err(format!("could not write to CEF helper: {error}"));
        }
        receiver.recv().map_err(|_| "CEF helper response channel closed".to_string())?
    }
    fn terminate(mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}
impl AppState {
    fn stop(&self) {
        let helper = self.helper.lock().ok().and_then(|mut slot| slot.take());
        if let Some(mut helper) = helper { let _ = helper.request("stop", None); helper.terminate(); }
    }
    fn request(&self, app: &AppHandle, method: &str, arguments: Option<Value>) -> Result<Value, String> {
        let mut slot = self.helper.lock().map_err(|_| "CEF state lock poisoned".to_string())?;
        if slot.is_none() { *slot = Some(HelperProcess::spawn(app)?); }
        let result = slot.as_mut().expect("CEF helper inserted").request(method, arguments);
        let failed = if result.is_err() { slot.take() } else { None };
        drop(slot);
        if let Some(helper) = failed { helper.terminate(); }
        result
    }
}
fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, String> { serde_json::from_value(value).map_err(|error| format!("invalid CEF helper result: {error}")) }

#[tauri::command]
fn browser_start(app: AppHandle, state: State<'_, AppState>) -> Result<Snapshot, String> { state.request(&app, "start", None).and_then(decode) }
#[tauri::command]
fn browser_snapshot(app: AppHandle, state: State<'_, AppState>) -> Result<Snapshot, String> { state.request(&app, "snapshot", None).and_then(decode) }
#[tauri::command]
fn browser_input(app: AppHandle, state: State<'_, AppState>, event: Value) -> Result<InputAck, String> {
    if !event.is_object() || serde_json::to_vec(&event).map_or(true, |bytes| bytes.len() > 8 * 1024) { return Err("input event must be a bounded object".to_string()); }
    state.request(&app, "input", Some(json!({ "event": event }))).and_then(decode)
}
#[tauri::command]
fn browser_reload(app: AppHandle, state: State<'_, AppState>) -> Result<Snapshot, String> { state.request(&app, "reload", None).and_then(decode) }
#[tauri::command]
fn browser_navigate(app: AppHandle, state: State<'_, AppState>, url: String) -> Result<Snapshot, String> {
    if url.len() > 2048 { return Err("URL is too long".to_string()); }
    state.request(&app, "navigate", Some(json!({ "url": url }))).and_then(decode)
}
#[tauri::command]
fn browser_inspect(app: AppHandle, state: State<'_, AppState>) -> Result<InspectResult, String> { state.request(&app, "inspect", None).and_then(decode) }
#[tauri::command]
fn browser_stop(state: State<'_, AppState>) -> Result<StopResult, String> { state.stop(); Ok(StopResult { status: "stopped" }) }
#[tauri::command]
fn browser_self_test_enabled(state: State<'_, AppState>) -> bool { state.self_test }
#[tauri::command]
fn browser_self_test_report(state: State<'_, AppState>, success: bool, detail: String) -> Result<(), String> {
    if !state.self_test { return Err("POC_SELF_TEST is not enabled".to_string()); }
    if detail.len() > 512 { return Err("self-test detail is too long".to_string()); }
    eprintln!("cef-osr-panel self-test {}: {detail}", if success { "passed" } else { "failed" }); Ok(())
}
fn main() {
    tauri::Builder::default().setup(|app| {
        eprintln!("cef-osr-panel ready");
        if app.state::<AppState>().self_test { let handle = app.handle().clone(); thread::spawn(move || { thread::sleep(Duration::from_millis(750)); let _ = handle.emit("cef-osr-panel-self-test", ()); }); }
        Ok(())
    }).manage(AppState::default()).invoke_handler(tauri::generate_handler![browser_start, browser_snapshot, browser_input, browser_reload, browser_navigate, browser_inspect, browser_stop, browser_self_test_enabled, browser_self_test_report]).build(tauri::generate_context!()).expect("error while building CEF OSR panel").run(|app, event| { if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) { app.state::<AppState>().stop(); } });
}
