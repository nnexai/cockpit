use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{atomic::{AtomicBool, AtomicU64, Ordering}, mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{Emitter, Manager, RunEvent, State};

const MAX_PROTOCOL_LINE: usize = 8 * 1024;

struct AppState { helper: Mutex<Option<HelperProcess>>, self_test: bool, shutting_down: AtomicBool, prewarm_generation: AtomicU64 }
impl Default for AppState { fn default() -> Self { Self { helper: Mutex::new(None), self_test: matches!(env::var("POC_SELF_TEST").as_deref(), Ok("1")), shutting_down: AtomicBool::new(false), prewarm_generation: AtomicU64::new(0) } } }

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Viewport { width: u32, height: u32, scale: f32, offset_x: f32, offset_y: f32 }
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Snapshot { stream_url: String, sequence: u32, pixel_width: u32, pixel_height: u32, viewport: Viewport, title: String, url: String, cursor: String }
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InspectResult { status: String, fixture_status: String, title: String, url: String, active_element: String, selected_text: String, scroll_y: u32, cursor: String }
#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InputAck { cursor: String }
#[derive(Serialize)] struct StopResult { status: &'static str }

struct HelperProcess { child: Child, stdin: ChildStdin, pending: Arc<Mutex<HashMap<u64, mpsc::SyncSender<Result<Value, String>>>>>, next_id: u64 }
impl HelperProcess {
    fn spawn() -> Result<Self, String> {
        let node = env::var("NODE_BINARY").unwrap_or_else(|_| "node".to_string());
        let helper_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("helper").join("browser-helper.mjs");
        let mut command = Command::new(node);
        command.arg(helper_path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        #[cfg(unix)] command.process_group(0);
        let mut child = command.spawn().map_err(|error| format!("could not start MediaStream browser helper: {error}"))?;
        let stdin = child.stdin.take().ok_or_else(|| "browser helper stdin was unavailable".to_string())?;
        let stdout = child.stdout.take().ok_or_else(|| "browser helper stdout was unavailable".to_string())?;
        let pending = Arc::new(Mutex::new(HashMap::<u64, mpsc::SyncSender<Result<Value, String>>>::new()));
        let pending_reader = Arc::clone(&pending);
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout); let mut line = String::new();
            while let Ok(read) = reader.read_line(&mut line) {
                if read == 0 { break; }
                if line.len() <= MAX_PROTOCOL_LINE {
                    if let Ok(response) = serde_json::from_str::<Value>(&line) {
                        if let Some(id) = response.get("id").and_then(Value::as_u64) {
                            if let Some(waiter) = pending_reader.lock().ok().and_then(|mut values| values.remove(&id)) {
                                let result = if response.get("ok").and_then(Value::as_bool) == Some(true) { response.get("result").cloned().ok_or_else(|| "browser helper response omitted result".to_string()) } else { Err(response.get("error").and_then(Value::as_str).unwrap_or("browser helper rejected the request").to_string()) };
                                let _ = waiter.send(result);
                            }
                        }
                    }
                }
                line.clear();
            }
            if let Ok(mut values) = pending_reader.lock() { for (_, waiter) in values.drain() { let _ = waiter.send(Err("browser helper exited unexpectedly".to_string())); } }
        });
        Ok(Self { child, stdin, pending, next_id: 1 })
    }
    fn request(&mut self, method: &str, arguments: Option<Value>, timeout: Option<Duration>) -> Result<Value, String> {
        let id = self.next_id; self.next_id = self.next_id.saturating_add(1);
        let mut request = Map::new(); request.insert("id".into(), json!(id)); request.insert("method".into(), json!(method)); if let Some(Value::Object(fields)) = arguments { request.extend(fields); }
        let encoded = serde_json::to_string(&Value::Object(request)).map_err(|error| format!("could not encode helper request: {error}"))?;
        if encoded.len() > MAX_PROTOCOL_LINE { return Err("helper request exceeded protocol limit".to_string()); }
        let (sender, receiver) = mpsc::sync_channel(1); self.pending.lock().map_err(|_| "browser response state lock is poisoned".to_string())?.insert(id, sender);
        if let Err(error) = writeln!(self.stdin, "{encoded}").and_then(|_| self.stdin.flush()) { self.pending.lock().ok().and_then(|mut values| values.remove(&id)); return Err(format!("could not write to browser helper: {error}")); }
        let result = match timeout { Some(limit) => receiver.recv_timeout(limit).map_err(|_| "browser helper response timed out".to_string()).and_then(|result| result), None => receiver.recv().map_err(|_| "browser helper response channel closed".to_string()).and_then(|result| result) };
        if result.is_err() { self.pending.lock().ok().and_then(|mut values| values.remove(&id)); }
        result
    }
    fn terminate(mut self) {
        #[cfg(unix)] unsafe { let _ = libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL); }
        let _ = self.child.kill(); let _ = self.child.wait();
    }
}
impl AppState {
    fn stop(&self) { self.prewarm_generation.fetch_add(1, Ordering::AcqRel); let helper = match self.helper.lock() { Ok(mut slot) => slot.take(), Err(_) => return }; if let Some(mut helper) = helper { let _ = helper.request("stop", None, Some(Duration::from_millis(500))); helper.terminate(); } }
    fn shutdown(&self) { self.shutting_down.store(true, Ordering::Release); self.stop(); }
    fn prewarm(&self, generation: u64) {
        let mut slot = match self.helper.lock() { Ok(slot) => slot, Err(_) => return };
        if self.shutting_down.load(Ordering::Acquire) || self.prewarm_generation.load(Ordering::Acquire) != generation { return; }
        if slot.is_none() { *slot = match HelperProcess::spawn() { Ok(helper) => Some(helper), Err(_) => return }; }
        let failed = if slot.as_mut().expect("helper was inserted").request("start", None, Some(Duration::from_secs(5))).is_err() { slot.take() } else { None }; drop(slot); if let Some(helper) = failed { helper.terminate(); }
    }
    fn request(&self, method: &str, arguments: Option<Value>) -> Result<Value, String> {
        let mut slot = self.helper.lock().map_err(|_| "browser state lock is poisoned".to_string())?;
        if self.shutting_down.load(Ordering::Acquire) { return Err("browser state is shutting down".to_string()); }
        if slot.is_none() { *slot = Some(HelperProcess::spawn()?); }
        let result = slot.as_mut().expect("helper was inserted").request(method, arguments, None);
        let failed = if result.is_err() { slot.take() } else { None }; drop(slot); if let Some(helper) = failed { helper.terminate(); } result
    }
}
fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, String> { serde_json::from_value(value).map_err(|error| format!("invalid browser helper result: {error}")) }
#[tauri::command] fn browser_start(state: State<'_, AppState>) -> Result<Snapshot, String> { match state.request("start", None).and_then(decode) { Ok(snapshot) => Ok(snapshot), Err(error) => { state.stop(); Err(error) } } }
#[tauri::command] fn browser_snapshot(state: State<'_, AppState>) -> Result<Snapshot, String> { state.request("snapshot", None).and_then(decode) }
#[tauri::command] fn browser_input(state: State<'_, AppState>, event: Value) -> Result<InputAck, String> { if !event.is_object() || serde_json::to_vec(&event).map_or(true, |bytes| bytes.len() > MAX_PROTOCOL_LINE) { return Err("input event must be a bounded JSON object".to_string()); } state.request("input", Some(json!({ "event": event }))).and_then(decode) }
#[tauri::command] fn browser_reload(state: State<'_, AppState>) -> Result<Snapshot, String> { state.request("reload", None).and_then(decode) }
#[tauri::command] fn browser_navigate(state: State<'_, AppState>, url: String) -> Result<Snapshot, String> { if url.len() > 2048 { return Err("URL is too long".to_string()); } state.request("navigate", Some(json!({ "url": url }))).and_then(decode) }
#[tauri::command] fn browser_inspect(state: State<'_, AppState>) -> Result<InspectResult, String> { state.request("inspect", None).and_then(decode) }
#[tauri::command] fn browser_stop(state: State<'_, AppState>) -> Result<StopResult, String> { state.stop(); Ok(StopResult { status: "stopped" }) }
#[tauri::command] fn browser_self_test_enabled(state: State<'_, AppState>) -> bool { state.self_test }
#[tauri::command] fn browser_self_test_report(state: State<'_, AppState>, success: bool, detail: String) -> Result<(), String> { if !state.self_test { return Err("POC_SELF_TEST is not enabled".to_string()); } if detail.len() > 512 { return Err("self-test detail is too long".to_string()); } if success { eprintln!("interactive-browser-panel-screencast-mediastream-poc self-test passed: {detail}"); } else { eprintln!("interactive-browser-panel-screencast-mediastream-poc self-test failed: {detail}"); } Ok(()) }
fn main() {
    tauri::Builder::default().setup(|app| { eprintln!("interactive-browser-panel-screencast-mediastream-poc ready"); let prewarm_generation = app.state::<AppState>().prewarm_generation.load(Ordering::Acquire); let prewarm_handle = app.handle().clone(); thread::spawn(move || { prewarm_handle.state::<AppState>().prewarm(prewarm_generation); }); if app.state::<AppState>().self_test { let handle = app.handle().clone(); thread::spawn(move || { thread::sleep(Duration::from_millis(750)); let _ = handle.emit("interactive-browser-panel-self-test", ()); }); } Ok(()) }).manage(AppState::default()).invoke_handler(tauri::generate_handler![browser_start, browser_snapshot, browser_input, browser_reload, browser_navigate, browser_inspect, browser_stop, browser_self_test_enabled, browser_self_test_report]).build(tauri::generate_context!()).expect("error while building MediaStream browser panel").run(|app, event| { if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) { app.state::<AppState>().shutdown(); } });
}
