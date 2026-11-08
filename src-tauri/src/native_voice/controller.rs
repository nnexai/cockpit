use std::sync::Arc;

use cockpit_host::{transport::{error::OperationError, operations::backend::BackendContext, Transport}, voice::ops};
use cockpit_protocol::{orchestration::HandoffBinding, voice::{VoiceConnectRequest, VoiceControl, VoiceControlRequest, VoiceEventsRequest}};
use parking_lot::Mutex;
use tokio::sync::{watch, Mutex as AsyncMutex};

use super::{media::NativeMedia, presentation::{NativeVoiceContext, NativeVoicePresentation}};

// Only fixed host-generated categories cross the presentation boundary. Provider
// errors can contain arbitrary signaling or credentials and are never echoed.
fn connect_error(error: &OperationError) -> &'static str {
    let OperationError::Service(error) = error else {
        return "Voice connection failed or its outcome is unknown";
    };
    let message = error.message.as_str();
    if message == "no eligible supervisor" {
        "No eligible supervisor is available for this session"
    } else if message == "supervisor is no longer eligible" {
        "The selected supervisor is no longer eligible"
    } else if message.starts_with("handoff_binding_unavailable: repository_not_found:") {
        "The selected repository is no longer available"
    } else if message.starts_with("handoff_binding_unavailable: repository_identity_conflict:") {
        "The selected repository identity is ambiguous"
    } else if message.starts_with("handoff_binding_unavailable: workspace_missing:") {
        "The selected Space is not present in the current session"
    } else if message.starts_with("handoff_binding_unavailable: workspace_cwd_missing:") {
        "The selected Space has no unambiguous working directory"
    } else if message.starts_with("handoff_binding_unavailable:") || message == "canonical handoff binding is unavailable" {
        "The selected project or Space could not be resolved for handoff"
    } else if message == "session, supervisor or selected project changed while connecting" {
        "The session, supervisor or selected project changed while connecting"
    } else if message == "a voice call is already active" {
        "A voice call is already active"
    } else {
        "Voice connection failed or its outcome is unknown"
    }
}

fn media_start_error(error: &str) -> &str {
    const PREFIX: &str = "Native Voice requires the GStreamer '";
    const SUFFIX: &str = "' plugin, which is unavailable";
    let name = error.strip_prefix(PREFIX).and_then(|value| value.strip_suffix(SUFFIX));
    match name {
        Some("pulsesrc" | "pulsesink" | "osxaudiosrc" | "osxaudiosink" |
             "webrtcbin" | "nicesrc" | "opusenc" | "opusdec" | "rtpopuspay" |
             "rtpopusdepay" | "webrtcdsp" | "webrtcechoprobe" | "volume") => error,
        _ => "Native voice media could not start",
    }
}

struct Attempt {
    generation: u64,
    cancel: watch::Sender<bool>,
    media: Option<Arc<NativeMedia>>,
    teardown: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    call: Option<(String, u64)>,
    mute_version: u64,
}

struct State {
    presentation: NativeVoicePresentation,
    attempt: Option<Attempt>,
}

pub struct NativeVoiceController {
    backend: Arc<BackendContext>,
    state: Mutex<State>,
    updates: watch::Sender<NativeVoicePresentation>,
    mute_serial: AsyncMutex<()>,
}

impl NativeVoiceController {
    pub fn new(backend: Arc<BackendContext>) -> Arc<Self> {
        let presentation = NativeVoicePresentation::default();
        let (updates, _) = watch::channel(presentation.clone());
        Arc::new(Self { backend, state: Mutex::new(State { presentation, attempt: None }), updates, mute_serial: AsyncMutex::new(()) })
    }

    fn publish(&self, state: &mut State) {
        state.presentation.revision = state.presentation.revision.saturating_add(1);
        self.updates.send_replace(state.presentation.clone());
    }

    fn schedule_teardown(attempt: &mut Attempt) {
        if let Some(media) = attempt.media.take() {
            media.gate_and_cancel();
            let (sender, receiver) = tokio::sync::oneshot::channel();
            attempt.teardown = Some(receiver);
            std::thread::spawn(move || { let _ = sender.send(media.stop()); });
        }
    }

    pub fn snapshot(&self) -> NativeVoicePresentation { self.state.lock().presentation.clone() }
    pub fn subscribe(&self) -> watch::Receiver<NativeVoicePresentation> { self.updates.subscribe() }

    pub fn start(self: &Arc<Self>, session_id: String, selected_root_id: Option<String>, project_scope: Option<HandoffBinding>) -> Result<(), String> {
        let mut state = self.state.lock();
        if state.attempt.is_some() { return Err("A voice call is still active or stopping".into()); }
        if session_id.is_empty() { return Err("A session is required".into()); }
        let generation = state.presentation.view_generation.saturating_add(1);
        let revision = state.presentation.revision;
        state.presentation = NativeVoicePresentation { view_generation: generation, revision, status: "starting".into(), context: Some(NativeVoiceContext { session_id: session_id.clone(), selected_root_id: selected_root_id.clone(), project_scope: project_scope.clone() }), ..Default::default() };
        let (cancel, receiver) = watch::channel(false);
        state.attempt = Some(Attempt { generation, cancel, media: None, teardown: None, call: None, mute_version: 0 });
        self.publish(&mut state);
        drop(state);
        let controller = Arc::clone(self);
        tauri::async_runtime::spawn(async move { controller.run(generation, receiver, session_id, selected_root_id, project_scope).await; });
        Ok(())
    }

    /// Gate locally immediately; device teardown runs on a dedicated thread.
    pub fn stop(self: &Arc<Self>) {
        let mut state = self.state.lock();
        if let Some(attempt) = state.attempt.as_mut() {
            attempt.cancel.send_replace(true);
            Self::schedule_teardown(attempt);
            state.presentation.status = "stopping".into();
            self.publish(&mut state);
        }
    }

    pub fn set_muted(self: &Arc<Self>, muted: bool) -> Result<(), String> {
        let (generation, version, media) = {
            let mut state = self.state.lock();
            if state.presentation.status != "active" { return Err("Voice is not active".into()); }
            let attempt = state.attempt.as_mut().ok_or("Voice is not active")?;
            let media = attempt.media.clone().ok_or("Voice media is unavailable")?;
            if muted {
                if media.set_muted(true).is_err() {
                    let generation = attempt.generation;
                    drop(state);
                    self.fail(generation, "Native microphone mute failed".into());
                    return Err("Native microphone mute failed".into());
                }
            }
            attempt.mute_version = attempt.mute_version.saturating_add(1);
            let result = (attempt.generation, attempt.mute_version, media);
            if muted { state.presentation.muted = true; self.publish(&mut state); }
            result
        };
        let controller = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let _serial = controller.mute_serial.lock().await;
            let call = {
                let state = controller.state.lock();
                match state.attempt.as_ref() {
                    Some(a) if a.generation == generation && a.mute_version == version && !*a.cancel.borrow() => a.call.clone(),
                    _ => None,
                }
            };
            let Some((call_id, owner_generation)) = call else { return; };
            let result = ops::control(&controller.backend, Transport::Native, VoiceControlRequest { call_id, generation: owner_generation, control: VoiceControl::Mute { muted } }).await;
            if result.is_err() {
                controller.fail(generation, "Voice mute failed; the call was stopped".into());
                return;
            }
            let mut state = controller.state.lock();
            if state.attempt.as_ref().is_some_and(|a| a.generation == generation && a.mute_version == version && !*a.cancel.borrow()) {
                if !muted {
                    if media.set_muted(false).is_err() {
                        drop(state);
                        controller.fail(generation, "Native microphone unmute failed".into());
                        return;
                    }
                }
                state.presentation.muted = muted;
                controller.publish(&mut state);
            }
        });
        Ok(())
    }

    fn fail(self: &Arc<Self>, generation: u64, error: String) {
        let mut state = self.state.lock();
        if let Some(attempt) = state.attempt.as_mut().filter(|a| a.generation == generation) {
            attempt.cancel.send_replace(true);
            Self::schedule_teardown(attempt);
            state.presentation.status = "error".into();
            state.presentation.error = Some(error);
            self.publish(&mut state);
        }
    }

    async fn run(self: Arc<Self>, generation: u64, mut cancelled: watch::Receiver<bool>, session_id: String, selected_root_id: Option<String>, project_scope: Option<HandoffBinding>) {
        let prepared = NativeMedia::start_cancellable(cancelled.clone()).await;
        let (media, offer) = match prepared {
            Ok(value) => value,
            Err(error) => {
                if !*cancelled.borrow() { self.fail(generation, media_start_error(&error).into()); }
                self.finish(generation).await;
                return;
            }
        };
        let media = Arc::new(media);
        let admitted = {
            let mut state = self.state.lock();
            match state.attempt.as_mut().filter(|a| a.generation == generation) {
                Some(attempt) => {
                    attempt.media = Some(media.clone());
                    if *attempt.cancel.borrow() { Self::schedule_teardown(attempt); false } else { true }
                }
                None => false,
            }
        };
        if !admitted {
            self.finish(generation).await;
            return;
        }
        let already_cancelled = *cancelled.borrow();
        if already_cancelled { self.finish(generation).await; return; }
        // Do not cancel this future: a late successful connect must be stopped exactly once.
        let connected = ops::connect(&self.backend, Transport::Native, VoiceConnectRequest { session_id, sdp_offer: offer, selected_root_id, project_scope }).await;
        let response = match connected {
            Ok(response) => response,
            Err(error) => { self.fail(generation, connect_error(&error).into()); self.finish(generation).await; return; }
        };
        let call_id = response.call_id;
        let owner_generation = response.generation;
        let stopped = {
            let mut state = self.state.lock();
            match state.attempt.as_mut().filter(|a| a.generation == generation) {
                Some(attempt) => { attempt.call = Some((call_id.clone(), owner_generation)); *attempt.cancel.borrow() },
                None => true,
            }
        };
        if !stopped {
            if media.apply_answer(&response.sdp_answer).await.is_err() { self.fail(generation, "Native voice negotiation failed".into()); }
        }
        let owner_ended = if !*cancelled.borrow() {
            self.poll(generation, &mut cancelled, &media, &call_id, owner_generation).await
        } else { false };
        {
            let mut state = self.state.lock();
            if let Some(attempt) = state.attempt.as_mut().filter(|a| a.generation == generation) { Self::schedule_teardown(attempt); }
        }
        if !owner_ended {
            if ops::control(&self.backend, Transport::Native, VoiceControlRequest { call_id, generation: owner_generation, control: VoiceControl::Stop }).await.is_err() {
                self.fail(generation, "Voice stopped locally; remote stop outcome is unknown".into());
            }
        }
        self.finish(generation).await;
    }

    async fn poll(self: &Arc<Self>, generation: u64, cancelled: &mut watch::Receiver<bool>, media: &NativeMedia, call_id: &str, owner_generation: u64) -> bool {
        let mut cursor = 0;
        loop {
            let request = ops::events(&self.backend, Transport::Native, VoiceEventsRequest { call_id: call_id.to_owned(), generation: owner_generation, after_seq: cursor, wait_ms: 25_000 });
            tokio::pin!(request);
            let response = loop {
                tokio::select! {
                    _ = cancelled.changed() => return false,
                    result = &mut request => break result,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {
                        if media.failure().is_some() { self.fail(generation, "Native voice media failed".into()); return false; }
                        if media.connected() {
                            let mut state = self.state.lock();
                            if state.attempt.as_ref().is_some_and(|a| a.generation == generation && !*a.cancel.borrow()) && state.presentation.status == "starting" {
                                state.presentation.status = "active".into();
                                self.publish(&mut state);
                            }
                        }
                    }
                }
            };
            let response = match response { Ok(value) => value, Err(_) => { self.fail(generation, "Voice events became unavailable".into()); return false; } };
            if response.events.len() > 256 { self.fail(generation, "Voice event batch exceeds presentation limit".into()); return false; }
            let mut ended = response.ended;
            {
                let mut state = self.state.lock();
                if !state.attempt.as_ref().is_some_and(|a| a.generation == generation && !*a.cancel.borrow()) { return false; }
                for event in response.events {
                    if event.call_id != call_id || event.generation != owner_generation || event.seq != cursor.saturating_add(1) {
                        drop(state); self.fail(generation, "Voice event sequence or identity mismatch".into()); return false;
                    }
                    cursor = event.seq;
                    match state.presentation.apply(event.event) {
                        Ok(value) => ended |= value,
                        Err(error) => { drop(state); self.fail(generation, error); return false; }
                    }
                }
                if response.next_seq != cursor { drop(state); self.fail(generation, "Voice event cursor mismatch".into()); return false; }
                self.publish(&mut state);
            }
            if ended { return true; }
        }
    }

    async fn finish(&self, generation: u64) {
        let receiver = {
            let mut state = self.state.lock();
            state.attempt.as_mut().filter(|a| a.generation == generation).and_then(|a| a.teardown.take())
        };
        let teardown_failed = match receiver { Some(receiver) => !matches!(receiver.await, Ok(Ok(()))), None => false };
        let mut state = self.state.lock();
        if state.attempt.as_ref().is_some_and(|a| a.generation == generation) {
            if teardown_failed { state.presentation.status = "error".into(); state.presentation.error = Some("Native audio device teardown failed".into()); }
            state.attempt = None;
            if state.presentation.status != "error" {
                state.presentation.status = if state.presentation.error.is_some() { "error" } else { "stopped" }.into();
            }
            self.publish(&mut state);
        }
    }
}
