//! Native-only WebRTC device ownership. No media data crosses the webview boundary.
use std::sync::{Arc, Weak, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_webrtc as webrtc;
use parking_lot::Mutex;

const SIGNAL_TIMEOUT: Duration = Duration::from_secs(5);
thread_local! {
    // Startup cancellation drops a partially initialized session on the
    // blocking worker, where teardown must finish before reporting completion.
    static STARTUP_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}


struct Inner {
    pipeline: gst::Pipeline,
    peer: gst::Element,
    gate: gst::Element,
    channel: webrtc::WebRTCDataChannel,
    stopped: AtomicBool,
    // Serializes the stop latch with unmute so a late unmute cannot reopen capture.
    gate_lock: Mutex<()>,
    teardown: Mutex<Option<Result<(), String>>>,
    failure: Mutex<Option<String>>,
}

impl Inner {
    fn gate_and_cancel(&self) {
        let _guard = self.gate_lock.lock();
        if !self.stopped.swap(true, Ordering::AcqRel) {
            self.gate.set_property("mute", true);
        }
    }
    fn stop(&self) -> Result<(), String> {
        self.gate_and_cancel();
        let mut teardown = self.teardown.lock();
        if let Some(result) = teardown.as_ref() { return result.clone(); }
        let result = self.pipeline.set_state(gst::State::Null)
            .map(|_| ())
            .map_err(|e| format!("Failed to release native audio devices: {e}"));
        self.channel.close();
        *teardown = Some(result.clone());
        result
    }
    fn fail(&self, message: String) {
        let mut failure = self.failure.lock();
        if failure.is_none() { *failure = Some(message); }
    }
}

/// A single native call; dropping it also closes its devices.
pub struct NativeMedia { inner: Arc<Inner> }

impl Drop for NativeMedia {
    fn drop(&mut self) {
        self.inner.gate_and_cancel();
        if STARTUP_WORKER.get() {
            let _ = self.inner.stop();
        } else {
            let inner = self.inner.clone();
            std::thread::spawn(move || { let _ = inner.stop(); });
        }
    }
}

impl NativeMedia {
    /// Acquires the default OS audio devices only in response to native Start.
    /// Cancellation is acknowledged only after the startup worker has unwound
    /// and released any acquired devices. A blocked OS plugin can delay it.
    pub async fn start_cancellable(mut cancelled: tokio::sync::watch::Receiver<bool>) -> Result<(Self, String), String> {
        let (mut sender, receiver) = tokio::sync::oneshot::channel();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            STARTUP_WORKER.set(true);
            let result = runtime.block_on(async {
                if *cancelled.borrow() { return Err("Native Voice start canceled".to_string()); }
                tokio::select! {
                    biased;
                    _ = sender.closed() => Err("Native Voice start canceled".to_string()),
                    _ = cancelled.changed() => Err("Native Voice start canceled".to_string()),
                    result = Self::start_inner() => result,
                }
            });
            if *cancelled.borrow() || sender.is_closed() {
                drop(result);
                let _ = sender.send(Err("Native Voice start canceled".into()));
            } else {
                let _ = sender.send(result);
            }
            STARTUP_WORKER.set(false);
        });
        receiver.await.map_err(|_| "Native Voice startup worker ended unexpectedly".to_string())?
    }

    async fn start_inner() -> Result<(Self, String), String> {
        gst::init().map_err(|e| format!("GStreamer initialization failed: {e}"))?;
        #[cfg(target_os = "linux")]
        let (source, sink) = ("pulsesrc", "pulsesink");
        #[cfg(target_os = "macos")]
        let (source, sink) = ("osxaudiosrc", "osxaudiosink");
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return Err("Native Voice audio is unsupported on this operating system".into());

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            for name in [source, sink, "webrtcbin", "nicesrc", "opusenc", "opusdec", "rtpopuspay", "rtpopusdepay", "webrtcdsp", "webrtcechoprobe", "volume"] {
                if gst::ElementFactory::find(name).is_none() {
                    return Err(format!("Native Voice requires the GStreamer '{name}' plugin, which is unavailable"));
                }
            }
            // The probe is in the actual render path. The DSP must be configured with
            // the same named probe and matching mono 48 kHz format; it is not optional.
            let graph = format!("webrtcbin name=peer bundle-policy=max-bundle \
                {source} name=input ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=48000,channels=1,layout=interleaved ! \
                webrtcdsp name=dsp probe=voice_echo ! volume name=gate ! opusenc ! rtpopuspay pt=96 ! \
                application/x-rtp,media=audio,encoding-name=OPUS,payload=96,clock-rate=48000 ! peer. \
                rtpopusdepay name=depay ! opusdec ! audioconvert ! audioresample ! \
                audio/x-raw,format=S16LE,rate=48000,channels=1,layout=interleaved ! \
                webrtcechoprobe name=voice_echo ! audioconvert ! audioresample ! {sink} name=output");
            let pipeline = gst::parse::launch(&graph).map_err(|e| format!("Native audio graph could not be created: {e}"))?
                .downcast::<gst::Pipeline>().map_err(|_| "Native audio graph is not a pipeline".to_string())?;
            let peer = pipeline.by_name("peer").ok_or("Native audio peer is missing")?;
            let gate = pipeline.by_name("gate").ok_or("Native microphone gate is missing")?;
            let depay = pipeline.by_name("depay").ok_or("Native playback input is missing")?;
            let sink_pad = depay.static_pad("sink").ok_or("Native playback pad is missing")?;
            let (pad_error, pad_error_rx) = std::sync::mpsc::sync_channel::<String>(1);
            peer.connect_pad_added(move |_, pad| {
                if sink_pad.is_linked() { return; }
                if let Err(error) = pad.link(&sink_pad) {
                    let _ = pad_error.try_send(format!("Native playback link failed: {error}"));
                }
            });
            if let Err(error) = pipeline.set_state(gst::State::Ready) {
                let _ = pipeline.set_state(gst::State::Null);
                return Err(format!("Native audio initialization failed: {error}"));
            }
            let channel = peer.emit_by_name::<Option<webrtc::WebRTCDataChannel>>("create-data-channel", &[&"oai-events", &None::<gst::Structure>])
                .ok_or_else(|| { let _ = pipeline.set_state(gst::State::Null); "Native Voice data channel could not be created".to_string() })?;
            let media = Self { inner: Arc::new(Inner { pipeline, peer, gate, channel, stopped: AtomicBool::new(false), gate_lock: Mutex::new(()), teardown: Mutex::new(None), failure: Mutex::new(None) }) };
            let bus = media.inner.pipeline.bus().ok_or("Native audio bus is missing")?;
            let weak: Weak<Inner> = Arc::downgrade(&media.inner);
            std::thread::Builder::new().name("native-voice-media".into()).spawn(move || loop {
                let Some(inner) = weak.upgrade() else { break };
                if inner.stopped.load(Ordering::Acquire) { break; }
                if let Ok(error) = pad_error_rx.try_recv() { inner.fail(error); }
                if let Some(message) = bus.timed_pop_filtered(gst::ClockTime::from_mseconds(100), &[gst::MessageType::Error, gst::MessageType::Eos]) {
                    match message.view() {
                        gst::MessageView::Error(error) => inner.fail(format!("Native audio failed: {}", error.error())),
                        gst::MessageView::Eos(_) => inner.fail("Native audio ended unexpectedly".into()),
                        _ => {}
                    }
                }
                let state = inner.peer.property::<webrtc::WebRTCPeerConnectionState>("connection-state");
                if matches!(state, webrtc::WebRTCPeerConnectionState::Failed | webrtc::WebRTCPeerConnectionState::Disconnected | webrtc::WebRTCPeerConnectionState::Closed) {
                    inner.fail(format!("Native Voice connection ended: {state:?}"));
                }
                if inner.failure.lock().is_some() { let _ = inner.stop(); break; }
            }).map_err(|e| format!("Native audio monitor could not start: {e}"))?;
            media.inner.pipeline.set_state(gst::State::Playing).map_err(|e| format!("Native audio devices could not be opened: {e}"))?;
            let (promise, future) = gst::Promise::new_future();
            media.inner.peer.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
            let reply = tokio::time::timeout(SIGNAL_TIMEOUT, future).await.map_err(|_| "Native Voice offer timed out".to_string())?
                .map_err(|e| format!("Native Voice offer failed: {e:?}"))?
                .ok_or("Native Voice offer returned no result")?;
            let offer = reply.get::<webrtc::WebRTCSessionDescription>("offer").map_err(|e| format!("Native Voice offer is invalid: {e}"))?;
            let (promise, future) = gst::Promise::new_future();
            media.inner.peer.emit_by_name::<()>("set-local-description", &[&offer, &promise]);
            let local_reply = tokio::time::timeout(SIGNAL_TIMEOUT, future).await.map_err(|_| "Setting the native Voice offer timed out".to_string())?
                .map_err(|e| format!("Setting the native Voice offer failed: {e:?}"))?;
            if let Some(reply) = local_reply {
                if let Ok(error) = reply.get::<gst::glib::Error>("error") {
                    return Err(format!("Setting the native Voice offer failed: {error}"));
                }
            }
            let deadline = tokio::time::Instant::now() + SIGNAL_TIMEOUT;
            loop {
                if let Some(error) = media.failure() { return Err(error); }
                if media.inner.peer.property::<webrtc::WebRTCICEGatheringState>("ice-gathering-state") == webrtc::WebRTCICEGatheringState::Complete { break; }
                if tokio::time::Instant::now() >= deadline { return Err("Native Voice ICE gathering timed out".into()); }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            let local = media.inner.peer.property::<Option<webrtc::WebRTCSessionDescription>>("local-description").ok_or("Native Voice local description is missing")?;
            let sdp = local.sdp().as_text().map_err(|e| format!("Native Voice SDP encoding failed: {e}"))?;
            Ok((media, sdp))
        }
    }

    pub async fn apply_answer(&self, answer: &str) -> Result<(), String> {
        if self.inner.stopped.load(Ordering::Acquire) { return Err("Native Voice has stopped".into()); }
        if answer.len() > 1024 * 1024 { return Err("Native Voice answer is too large".into()); }
        let sdp = gstreamer_sdp::SDPMessage::parse_buffer(answer.as_bytes()).map_err(|e| format!("Invalid native Voice answer: {e}"))?;
        let description = webrtc::WebRTCSessionDescription::new(webrtc::WebRTCSDPType::Answer, sdp);
        let (promise, future) = gst::Promise::new_future();
        {
            let _guard = self.inner.gate_lock.lock();
            if self.inner.stopped.load(Ordering::Acquire) { return Err("Native Voice has stopped".into()); }
            self.inner.peer.emit_by_name::<()>("set-remote-description", &[&description, &promise]);
        }
        let wait = tokio::time::timeout(SIGNAL_TIMEOUT, future);
        tokio::pin!(wait);
        let reply = loop {
            tokio::select! {
                result = &mut wait => break result.map_err(|_| "Applying the native Voice answer timed out".to_string())?
                    .map_err(|e| format!("Applying the native Voice answer failed: {e:?}"))?,
                _ = tokio::time::sleep(Duration::from_millis(25)) => {
                    if self.inner.stopped.load(Ordering::Acquire) { promise.interrupt(); return Err("Native Voice has stopped".into()); }
                }
            }
        };
        if let Some(reply) = reply {
            if let Ok(error) = reply.get::<gst::glib::Error>("error") {
                return Err(format!("Applying the native Voice answer failed: {error}"));
            }
        }
        if self.inner.stopped.load(Ordering::Acquire) { return Err("Native Voice has stopped".into()); }
        if let Some(error) = self.failure() { return Err(error); }
        Ok(())
    }

    pub fn set_muted(&self, muted: bool) -> Result<(), String> {
        let _guard = self.inner.gate_lock.lock();
        if self.inner.stopped.load(Ordering::Acquire) { return Err("Native Voice has stopped".into()); }
        self.inner.gate.set_property("mute", muted);
        Ok(())
    }
    pub fn gate_and_cancel(&self) { self.inner.gate_and_cancel(); }
    pub fn stop(&self) -> Result<(), String> { self.inner.stop() }
    pub fn failure(&self) -> Option<String> { self.inner.failure.lock().clone() }
    pub fn connected(&self) -> bool {
        !self.inner.stopped.load(Ordering::Acquire) && self.inner.peer.property::<webrtc::WebRTCPeerConnectionState>("connection-state") == webrtc::WebRTCPeerConnectionState::Connected
    }
}
