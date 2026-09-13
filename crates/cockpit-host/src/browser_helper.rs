use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use cockpit_core::{InspectionError, browser::BrowserRuntimeAttachment};
use cockpit_protocol::browser::BrowserTarget;
use cockpit_protocol::browser_view::{
    BrowserViewCapabilities, BrowserViewCommand, BrowserViewControlState, BrowserViewControlStatus,
    BrowserViewFocusState, BrowserViewIdentity,
};
use cockpit_protocol::browser_view::{
    BrowserViewCommandRequest, BrowserViewCommandResponse, BrowserViewEvent,
    BrowserViewEventMetadata, BrowserViewFrameDescriptor, BrowserViewFrameEnvelopeV2,
    BrowserViewFrameGrant, BrowserViewOpenRequest, BrowserViewSnapshot,
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, broadcast, mpsc, oneshot},
    time::timeout,
};
use uuid::Uuid;

const HELPER_QUEUE: usize = 32;
const EVENT_QUEUE: usize = 64;
const GRANT_LIFETIME: Duration = Duration::from_secs(30);
const HELPER_STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// The owner-only result required by a future host adapter. The endpoint is a
/// loopback WebSocket; consumers send the opaque `frame_grant.grant` as their
/// first WebSocket message, never in a URL.
pub struct BrowserViewOpen {
    pub snapshot: BrowserViewSnapshot,
    pub first_frame: BrowserViewFrameDescriptor,
    pub frame_endpoint: String,
}

/// A cancellable ordered metadata subscription. A consumer first receives the
/// atomic snapshot, then only events with contiguous metadata sequences.
pub struct BrowserViewEvents {
    pub snapshot: BrowserViewSnapshot,
    pub events: broadcast::Receiver<BrowserViewEvent>,
}
pub struct BrowserViewNativeSubscription {
    pub snapshot: BrowserViewSnapshot,
    pub events: broadcast::Receiver<BrowserViewEvent>,
    pub endpoint: String,
    pub grant: BrowserViewFrameGrant,
}
pub(crate) struct BrowserHelperSupervisor {
    state_root: PathBuf,
    /// Incarnation is part of the registry key so a restarted Chromium can
    /// never inherit a helper or frame grant from its predecessor.
    associations: Mutex<HashMap<(String, String), Vec<String>>>,
    views: Mutex<HashMap<String, ManagedView>>,
    native_connections: Mutex<HashMap<String, usize>>,
}

struct ManagedView {
    target: BrowserTarget,
    attachment: BrowserRuntimeAttachment,
    snapshot: Arc<Mutex<BrowserViewSnapshot>>,
    /// Serializes snapshot reads with each broadcast so subscribers get an
    /// atomic baseline and cannot miss the event that follows it.
    barrier: Arc<Mutex<()>>,
    endpoint: String,
    events: broadcast::Sender<BrowserViewEvent>,
    latest_frame: Arc<Mutex<Option<BrowserViewFrameDescriptor>>>,
    input: mpsc::Sender<HelperInput>,
    responses: Arc<Mutex<HashMap<String, oneshot::Sender<BrowserViewCommandResponse>>>>,
    task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    forward_task: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HelperInput {
    Attach {
        association_key: String,
        browser_incarnation: String,
        view_id: String,
        stream_epoch: u64,
        target_id: String,
        cdp_endpoint: String,
        playwright_core: String,
        viewport: HelperViewport,
        frame_grant: BrowserViewFrameGrant,
    },
    Grant {
        grant: BrowserViewFrameGrant,
    },
    AttachView {
        view_id: String,
        stream_epoch: u64,
        frame_grant: BrowserViewFrameGrant,
    },
    DetachView { view_id: String },
    Pause,
    Resume,
    Detach,
    Stop,
    Command {
        request: BrowserViewCommandRequest,
    },
}
#[derive(Serialize)]
struct HelperViewport {
    css_width: u32,
    css_height: u32,
    device_pixel_ratio: f64,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HelperOutput {
    Ready {
        frame_endpoint: String,
        snapshot: BrowserViewSnapshot,
    },
    Event {
        event: BrowserViewEvent,
    },
    Frame {
        descriptor: BrowserViewFrameDescriptor,
    },
    CommandResponse {
        response: BrowserViewCommandResponse,
    },
    Failed {
        code: String,
        message: String,
    },
}
impl BrowserHelperSupervisor {
    pub(crate) fn new(state_root: PathBuf) -> Self {
        Self {
            state_root,
            associations: Mutex::new(HashMap::new()),
            views: Mutex::new(HashMap::new()),
            native_connections: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn open(
        &self,
        attachment: BrowserRuntimeAttachment,
        request: BrowserViewOpenRequest,
    ) -> Result<BrowserViewOpen, InspectionError> {
        let playwright_core = attachment.playwright_core.clone().ok_or_else(|| {
            InspectionError::new(
                "browser_helper_unavailable",
                "Cannot locate the Playwright-core package paired with Playwright CLI",
            )
        })?;
        let node = attachment
            .node_executable
            .clone()
            .unwrap_or_else(|| PathBuf::from("node"));
        let helper = match attachment.helper_module.clone() {
            Some(path) => path,
            None => self.materialize_helper()?,
        };
        let view_id = Uuid::new_v4().to_string();
        let stream_epoch = 1;
        let frame_grant = new_grant(&view_id, stream_epoch)?;
        let association_key = (
            attachment.association_key.clone(),
            attachment.browser_incarnation.clone(),
        );
        while let Some(existing_id) = self.existing_view_id(&association_key).await {
            match self
                .open_shared(
                    existing_id,
                    association_key.clone(),
                    attachment.clone(),
                    request.clone(),
                )
                .await
            {
                Ok(opened) => return Ok(opened),
                Err(error) if error.code == "browser_view_not_found" => continue,
                Err(error) => return Err(error),
            }
        }
        let (input, mut input_rx) = mpsc::channel(HELPER_QUEUE);
        let (events, _) = broadcast::channel(EVENT_QUEUE);
        let (ready_tx, ready_rx) = oneshot::channel();
        let (first_frame_tx, first_frame_rx) = oneshot::channel();
        let mut child = Command::new(node)
            .arg(&helper)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                InspectionError::new(
                    "browser_helper_unavailable",
                    format!("Cannot start private browser helper: {error}"),
                )
            })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            InspectionError::new(
                "browser_helper_unavailable",
                "Private browser helper has no stdin",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            InspectionError::new(
                "browser_helper_unavailable",
                "Private browser helper has no stdout",
            )
        })?;
        let initial = HelperInput::Attach {
            association_key: attachment.association_key.clone(),
            browser_incarnation: attachment.browser_incarnation.clone(),
            view_id: view_id.clone(),
            stream_epoch,
            target_id: attachment.target_id.clone(),
            cdp_endpoint: attachment.cdp_endpoint.clone(),
            playwright_core: path_text(&playwright_core)?,
            viewport: HelperViewport {
                css_width: request.viewport.css_width,
                css_height: request.viewport.css_height,
                device_pixel_ratio: request.viewport.device_pixel_ratio,
            },
            frame_grant: frame_grant.clone(),
        };
        let snapshot = Arc::new(Mutex::new(empty_snapshot()));
        let barrier = Arc::new(Mutex::new(()));
        let latest_frame = Arc::new(Mutex::new(None));
        let responses = Arc::new(Mutex::new(HashMap::new()));
        let task_latest_frame = Arc::clone(&latest_frame);
        let task_snapshot = Arc::clone(&snapshot);
        let task_barrier = Arc::clone(&barrier);
        let task_responses = Arc::clone(&responses);
        let task_events = events.clone();
        let mut task = tokio::spawn(async move {
            run_helper(
                child,
                stdin,
                stdout,
                &mut input_rx,
                initial,
                task_snapshot,
                task_barrier,
                task_responses,
                task_events,
                task_latest_frame,
                ready_tx,
                first_frame_tx,
            )
            .await;
        });
        let (endpoint, _) = match timeout(Duration::from_secs(10), ready_rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                let _ = input.send(HelperInput::Stop).await;
                let _ = timeout(HELPER_STOP_TIMEOUT, &mut task).await;
                task.abort();
                return Err(InspectionError::new(
                    "browser_helper_failed",
                    "Private browser helper exited before attaching",
                ));
            }
            Err(_) => {
                let _ = input.send(HelperInput::Stop).await;
                let _ = timeout(HELPER_STOP_TIMEOUT, &mut task).await;
                task.abort();
                return Err(InspectionError::new(
                    "browser_helper_timeout",
                    "Private browser helper did not attach in time",
                ));
            }
        };
        let first_frame = match timeout(Duration::from_secs(10), first_frame_rx).await {
            Ok(Ok(frame)) => frame,
            Ok(Err(_)) => {
                let _ = input.send(HelperInput::Stop).await;
                let _ = timeout(HELPER_STOP_TIMEOUT, &mut task).await;
                task.abort();
                return Err(InspectionError::new(
                    "browser_helper_failed",
                    "Private browser helper stopped before producing an initial frame",
                ));
            }
            Err(_) => {
                let _ = input.send(HelperInput::Stop).await;
                let _ = timeout(HELPER_STOP_TIMEOUT, &mut task).await;
                task.abort();
                return Err(InspectionError::new(
                    "browser_helper_timeout",
                    "Private browser helper did not produce an initial frame in time",
                ));
            }
        };
        let current_snapshot = {
            let _barrier = barrier.lock().await;
            snapshot.lock().await.clone()
        };
        let managed = ManagedView {
            target: request.target.clone(),
            attachment: attachment.clone(),
            snapshot,
            barrier,
            endpoint: endpoint.clone(),
            events,
            input,
            responses,
            latest_frame,
            task: Arc::new(Mutex::new(Some(task))),
            forward_task: None,
        };
        self.views.lock().await.insert(view_id.clone(), managed);
        self.associations
            .lock()
            .await
            .entry(association_key)
            .or_default()
            .push(view_id);
        Ok(BrowserViewOpen {
            snapshot: current_snapshot,
            first_frame,
            frame_endpoint: endpoint,
        })
    }
    async fn existing_view_id(&self, association_key: &(String, String)) -> Option<String> {
        // Associations can outlive a managed view when its final gateway
        // connection races a subsequent open. Prune those IDs before sharing
        // so a new attach never targets a removed view.
        let mut associations = self.associations.lock().await;
        let ids = associations.get_mut(association_key)?;
        let views = self.views.lock().await;
        ids.retain(|id| views.contains_key(id));
        let existing = ids.first().cloned();
        if ids.is_empty() {
            associations.remove(association_key);
        }
        existing
    }

    pub(crate) async fn events(&self, view_id: &str) -> Result<BrowserViewEvents, InspectionError> {
        let views = self.views.lock().await;
        let view = views.get(view_id).ok_or_else(|| {
            InspectionError::new(
                "browser_view_not_found",
                "Browser view is not attached to this owner",
            )
        })?;
        let _barrier = view.barrier.lock().await;
        let snapshot = view.snapshot.lock().await.clone();
        let events = view.events.subscribe();
        Ok(BrowserViewEvents { snapshot, events })
    }
    pub(crate) async fn native_subscribe(
        &self,
        view_id: &str,
        stream_epoch: u64,
    ) -> Result<BrowserViewNativeSubscription, InspectionError> {
        let views = self.views.lock().await;
        let view = views.get(view_id).ok_or_else(|| {
            InspectionError::new(
                "browser_view_not_found",
                "Browser view is not attached to this owner",
            )
        })?;
        let _barrier = view.barrier.lock().await;
        let snapshot = view.snapshot.lock().await.clone();
        if snapshot.identity.stream_epoch != stream_epoch {
            return Err(InspectionError::new(
                "stale_browser_view",
                "Browser view stream epoch is stale",
            ));
        }
        let grant = snapshot.frame_grant.clone().ok_or_else(|| {
            InspectionError::new(
                "browser_frame_unavailable",
                "Browser view has no frame grant",
            )
        })?;
        let events = view.events.subscribe();
        let endpoint = view.endpoint.clone();
        let mut connections = self.native_connections.lock().await;
        *connections.entry(view_id.to_owned()).or_insert(0) += 1;
        Ok(BrowserViewNativeSubscription {
            snapshot,
            events,
            endpoint,
            grant,
        })
    }

    pub(crate) async fn native_release(&self, view_id: &str) {
        let release = {
            let mut connections = self.native_connections.lock().await;
            let Some(count) = connections.get_mut(view_id) else {
                return;
            };
            if *count > 1 {
                *count -= 1;
                false
            } else {
                connections.remove(view_id);
                true
            }
        };
        if release {
            self.detach(view_id).await;
        }
    }

    async fn open_shared(
        &self,
        existing_id: String,
        association_key: (String, String),
        attachment: BrowserRuntimeAttachment,
        request: BrowserViewOpenRequest,
    ) -> Result<BrowserViewOpen, InspectionError> {
        let (
            target,
            input,
            responses,
            endpoint,
            source_frame,
            task,
            mut baseline,
            first_frame,
            upstream,
        ) = {
            let views = self.views.lock().await;
            let source = views.get(&existing_id).ok_or_else(|| {
                InspectionError::new(
                    "browser_view_not_found",
                    "Shared browser helper disappeared",
                )
            })?;
            let _barrier = source.barrier.lock().await;
            let baseline = source.snapshot.lock().await.clone();
            let first_frame = source.latest_frame.lock().await.clone().ok_or_else(|| {
                InspectionError::new(
                    "browser_helper_failed",
                    "Shared browser helper has no retained frame",
                )
            })?;
            (
                source.target.clone(),
                source.input.clone(),
                Arc::clone(&source.responses),
                source.endpoint.clone(),
                Arc::clone(&source.latest_frame),
                Arc::clone(&source.task),
                baseline,
                first_frame,
                source.events.subscribe(),
            )
        };
        let view_id = Uuid::new_v4().to_string();
        let stream_epoch = 1;
        let grant = new_grant(&view_id, stream_epoch)?;
        input
            .send(HelperInput::Resume)
            .await
            .map_err(|_| {
                InspectionError::new("browser_helper_failed", "Shared browser helper is not running")
            })?;
        input
            .send(HelperInput::AttachView {
                view_id: view_id.clone(),
                stream_epoch,
                frame_grant: grant.clone(),
            })
            .await
            .map_err(|_| {
                InspectionError::new("browser_helper_failed", "Shared browser helper is not running")
            })?;
        baseline.identity.view_id = view_id.clone();
        baseline.identity.stream_epoch = stream_epoch;
        baseline.frame_grant = Some(grant);
        let snapshot = Arc::new(Mutex::new(baseline));
        let barrier = Arc::new(Mutex::new(()));
        let (events, _) = broadcast::channel(EVENT_QUEUE);
        let task_snapshot = Arc::clone(&snapshot);
        let task_barrier = Arc::clone(&barrier);
        let task_events = events.clone();
        let task_view_id = view_id.clone();
        let forward_task = tokio::spawn(async move {
            let mut upstream = upstream;
            while let Ok(event) = upstream.recv().await {
                let _guard = task_barrier.lock().await;
                let mut value = task_snapshot.lock().await;
                apply_event_snapshot(&mut value, &event, true);
                value.identity.view_id = task_view_id.clone();
                value.identity.stream_epoch = stream_epoch;
                value.metadata_sequence = value.metadata_sequence.saturating_add(1);
                let metadata = BrowserViewEventMetadata {
                    view_id: task_view_id.clone(),
                    stream_epoch,
                    metadata_sequence: value.metadata_sequence,
                };
                let event = match replace_event_metadata(event, metadata) {
                    BrowserViewEvent::Attached { metadata, .. } => {
                        BrowserViewEvent::Attached { metadata, snapshot: value.clone() }
                    }
                    event => event,
                };
                let _ = task_events.send(event);
            }
        });
        let current_snapshot = snapshot.lock().await.clone();
        let managed = ManagedView {
            target,
            attachment,
            snapshot,
            barrier,
            endpoint: endpoint.clone(),
            events,
            latest_frame: source_frame,
            input,
            responses,
            task,
            forward_task: Some(forward_task),
        };
        self.views.lock().await.insert(view_id.clone(), managed);
        self.associations
            .lock()
            .await
            .entry(association_key)
            .or_default()
            .push(view_id.clone());
        if request.takeover {
            let response = self
                .command(BrowserViewCommandRequest {
                    view_id: view_id.clone(),
                    stream_epoch,
                    request_id: Uuid::new_v4().to_string(),
                    command: BrowserViewCommand::TakeControl {
                        viewport: request.viewport.clone(),
                    },
                })
                .await?;
            if !matches!(response, BrowserViewCommandResponse::Accepted { .. }) {
                self.detach(&view_id).await;
                return Err(InspectionError::new(
                    "browser_control_required",
                    "Shared browser view could not take the interaction lease",
                ));
            }
        }
        Ok(BrowserViewOpen {
            snapshot: current_snapshot,
            first_frame,
            frame_endpoint: endpoint,
        })
    }
    pub(crate) async fn attachment_context(
        &self,
        view_id: &str,
    ) -> Result<(BrowserTarget, BrowserRuntimeAttachment), InspectionError> {
        let views = self.views.lock().await;
        let view = views.get(view_id).ok_or_else(|| InspectionError::new(
            "browser_view_not_found", "Browser view is not attached to this owner",
        ))?;
        let mut attachment = view.attachment.clone();
        if let Some(target_id) = view.snapshot.lock().await.displayed_target_id.clone() {
            attachment.target_id = target_id;
        }
        Ok((view.target.clone(), attachment))
    }

    pub(crate) async fn frame_endpoint(
        &self,
        grant: &BrowserViewFrameGrant,
    ) -> Result<String, InspectionError> {
        let views = self.views.lock().await;
        let view = views.get(&grant.view_id).ok_or_else(|| {
            InspectionError::new(
                "browser_view_not_found",
                "Browser view is not attached to this owner",
            )
        })?;
        let snapshot = view.snapshot.lock().await;
        if snapshot.identity.stream_epoch != grant.stream_epoch
            || snapshot.frame_grant.as_ref() != Some(grant)
        {
            return Err(InspectionError::new(
                "browser_view_stale_grant",
                "Browser frame grant no longer belongs to this stream",
            ));
        }
        Ok(view.endpoint.clone())
    }

    pub(crate) async fn command(
        &self,
        request: BrowserViewCommandRequest,
    ) -> Result<BrowserViewCommandResponse, InspectionError> {
        request
            .validate()
            .map_err(|message| InspectionError::new("invalid_browser_view_command", message))?;
        let (input, responses, current_epoch, current_sequence) = {
            let views = self.views.lock().await;
            let view = views.get(&request.view_id).ok_or_else(|| {
                InspectionError::new(
                    "browser_view_not_found",
                    "Browser view is not attached to this owner",
                )
            })?;
            let snapshot = view.snapshot.lock().await;
            (
                view.input.clone(),
                Arc::clone(&view.responses),
                snapshot.identity.stream_epoch,
                snapshot.metadata_sequence,
            )
        };
        if request.stream_epoch != current_epoch {
            return Ok(BrowserViewCommandResponse::Stale {
                view_id: request.view_id,
                stream_epoch: request.stream_epoch,
                request_id: request.request_id,
                current_stream_epoch: current_epoch,
                current_metadata_sequence: current_sequence,
                code: "stale_stream".into(),
                message: "Browser view stream identity changed".into(),
            });
        }
        let request_id = request.request_id.clone();
        let (response_tx, response_rx) = oneshot::channel();
        responses
            .lock()
            .await
            .insert(request_id.clone(), response_tx);
        if input
            .send(HelperInput::Command {
                request: request.clone(),
            })
            .await
            .is_err()
        {
            responses.lock().await.remove(&request_id);
            return Err(InspectionError::new(
                "browser_helper_failed",
                "Private browser helper is not running",
            ));
        }
        match timeout(Duration::from_secs(10), response_rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(InspectionError::new(
                "browser_helper_failed",
                "Private browser helper stopped before command response",
            )),
            Err(_) => {
                responses.lock().await.remove(&request_id);
                Ok(BrowserViewCommandResponse::OutcomeUnknown {
                    view_id: request.view_id,
                    stream_epoch: request.stream_epoch,
                    request_id,
                    code: "browser_command_timeout".into(),
                    message: "Private browser helper did not confirm the command outcome".into(),
                })
            }
        }
    }
    pub(crate) async fn detach(&self, view_id: &str) {
        if self.native_connections.lock().await.contains_key(view_id) {
            return;
        }
        let idle = {
            let associations = self.associations.lock().await;
            associations.values().any(|ids| ids.len() == 1 && ids.first().is_some_and(|id| id == view_id))
        };
        if idle {
            let managed = {
                let views = self.views.lock().await;
                views.get(view_id).map(|view| {
                    (
                        view.input.clone(),
                        Arc::clone(&view.snapshot),
                        Arc::clone(&view.barrier),
                        view.events.clone(),
                    )
                })
            };
            if let Some((input, snapshot, barrier, events)) = managed {
                let _ = input.send(HelperInput::Pause).await;
                let _guard = barrier.lock().await;
                let metadata = {
                    let mut value = snapshot.lock().await;
                    value.frame_grant = None;
                    value.metadata_sequence = value.metadata_sequence.saturating_add(1);
                    BrowserViewEventMetadata {
                        view_id: value.identity.view_id.clone(),
                        stream_epoch: value.identity.stream_epoch,
                        metadata_sequence: value.metadata_sequence,
                    }
                };
                let _ = events.send(BrowserViewEvent::FrameTransportRevoked {
                    metadata,
                    code: "browser_view_paused".into(),
                    message: "Browser frame transport paused until the view is shown again".into(),
                });
            }
            return;
        }
        let managed = {
            let mut views = self.views.lock().await;
            views.remove(view_id)
        };
        let Some(managed) = managed else { return };
        let mut associations = self.associations.lock().await;
        for view_ids in associations.values_mut() {
            view_ids.retain(|id| id != view_id);
        }
        let last_view = associations.values().all(Vec::is_empty) || associations.values().all(|ids| ids.is_empty());
        associations.retain(|_, ids| !ids.is_empty());
        drop(associations);
        if let Some(task) = managed.forward_task { task.abort(); }
        if last_view {
            let _ = managed.input.send(HelperInput::Detach).await;
            if let Some(task) = managed.task.lock().await.take() { let _ = timeout(HELPER_STOP_TIMEOUT, task).await; }
        } else { let _ = managed.input.send(HelperInput::DetachView { view_id: view_id.to_owned() }).await; }
    }
    pub(crate) async fn shutdown(&self) {
        self.associations.lock().await.clear();
        self.native_connections.lock().await.clear();
        let views = std::mem::take(&mut *self.views.lock().await);
        let mut stopped = std::collections::HashSet::new();
        for (_, view) in views {
            if let Some(task) = view.forward_task { task.abort(); }
            if stopped.insert(Arc::as_ptr(&view.task) as usize) {
                let _ = view.input.send(HelperInput::Stop).await;
                if let Some(task) = view.task.lock().await.take() { let _ = timeout(HELPER_STOP_TIMEOUT, task).await; }
            }
        }
    }

    fn materialize_helper(&self) -> Result<PathBuf, InspectionError> {
        let directory = self.state_root.join("helpers");
        fs::create_dir_all(&directory).map_err(|error| {
            InspectionError::new("browser_helper_unavailable", error.to_string())
        })?;
        let path = directory.join("browser-helper.mjs");
        fs::write(
            &path,
            include_str!("../../../browser-runtime/browser-helper.mjs"),
        )
        .map_err(|error| InspectionError::new("browser_helper_unavailable", error.to_string()))?;
        Ok(path)
    }
}

async fn run_helper(
    mut child: Child,
    mut stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    input: &mut mpsc::Receiver<HelperInput>,
    initial: HelperInput,
    snapshot: Arc<Mutex<BrowserViewSnapshot>>,
    barrier: Arc<Mutex<()>>,
    responses: Arc<Mutex<HashMap<String, oneshot::Sender<BrowserViewCommandResponse>>>>,
    events: broadcast::Sender<BrowserViewEvent>,
    latest_frame: Arc<Mutex<Option<BrowserViewFrameDescriptor>>>,
    ready: oneshot::Sender<(String, BrowserViewSnapshot)>,
    first_frame: oneshot::Sender<BrowserViewFrameDescriptor>,
) {
    let mut ready = Some(ready);
    let mut first_frame = Some(first_frame);
    // Helper metadata and published metadata use different clocks: the host
    // also advances its sequence for frame descriptors.
    let mut helper_metadata_sequence = 0;
    let mut ready_initialized = false;
    let mut last_frame_sequence = 0u64;
    if write_input(&mut stdin, &initial).await.is_err() {
        return;
    }
    let mut lines = BufReader::new(stdout).lines();
    loop {
        tokio::select! {
            item = input.recv() => match item {
                Some(HelperInput::Stop) | None => { let _ = write_input(&mut stdin, &HelperInput::Stop).await; break; }
                Some(HelperInput::Detach) => { let _ = write_input(&mut stdin, &HelperInput::Detach).await; break; }
                Some(input @ (HelperInput::Pause | HelperInput::Resume)) => { if write_input(&mut stdin, &input).await.is_err() { break; } }
                Some(input) => { if write_input(&mut stdin, &input).await.is_err() { break; } }
            },
            line = lines.next_line() => match line {
                Ok(Some(line)) => match serde_json::from_str::<HelperOutput>(&line) {
                    Ok(HelperOutput::Ready { frame_endpoint, snapshot: next }) => {
                        let _barrier = barrier.lock().await;
                        let current = {
                            let mut value = snapshot.lock().await;
                            let source_sequence = next.metadata_sequence;
                            let publication_sequence = value.metadata_sequence;
                            if !ready_initialized || source_sequence >= helper_metadata_sequence {
                                helper_metadata_sequence =
                                    helper_metadata_sequence.max(source_sequence);
                                *value = next;
                                // Frames and failures advance the host clock but
                                // are not included in helper metadata snapshots.
                                value.metadata_sequence =
                                    publication_sequence.max(value.metadata_sequence);
                            }
                            ready_initialized = true;
                            value.clone()
                        };
                        if let Some(sender) = ready.take() {
                            let _ = sender.send((frame_endpoint, current));
                        }
                    }
                    Ok(HelperOutput::Event { event }) => {
                        let _barrier = barrier.lock().await;
                        let event = {
                            let incoming = event_metadata(&event);
                            let source_sequence = incoming.metadata_sequence;
                            let attached_is_current =
                                source_sequence >= helper_metadata_sequence;
                            helper_metadata_sequence =
                                helper_metadata_sequence.max(source_sequence);
                            let mut value = snapshot.lock().await;
                            let metadata_sequence = source_sequence
                                .max(value.metadata_sequence.saturating_add(1));
                            apply_event_snapshot(&mut value, &event, attached_is_current);
                            value.metadata_sequence = metadata_sequence;
                            let mut metadata = incoming.clone();
                            metadata.metadata_sequence = metadata_sequence;
                            let event = replace_event_metadata(event, metadata);
                            match event {
                                BrowserViewEvent::Attached { metadata, .. } => {
                                    BrowserViewEvent::Attached {
                                        metadata,
                                        snapshot: value.clone(),
                                    }
                                }
                                event => event,
                            }
                        };
                        let _ = events.send(event);
                    }
                    Ok(HelperOutput::Frame { descriptor }) => {
                        let identity_valid = descriptor.validate().is_ok()
                            && descriptor.frame_sequence > last_frame_sequence
                            && {
                                let value = snapshot.lock().await;
                                descriptor.stream_epoch == value.identity.stream_epoch
                                    && value
                                        .displayed_target_id
                                        .as_deref()
                                        .is_some_and(|target| target == descriptor.target_id)
                            };
                        if !identity_valid {
                            send_failed(
                                &snapshot,
                                &barrier,
                                &events,
                                "browser_frame_invalid".into(),
                                "Private browser helper emitted a frame with stale or invalid identity".into(),
                            )
                            .await;
                            continue;
                        }
                        last_frame_sequence = descriptor.frame_sequence;
                        let _barrier = barrier.lock().await;
                        *latest_frame.lock().await = Some(descriptor.clone());
                        if let Some(sender) = first_frame.take() {
                            let _ = sender.send(descriptor.clone());
                        }
                    }
                    Ok(HelperOutput::CommandResponse { response }) => {
                        let request_id = match &response {
                            BrowserViewCommandResponse::Accepted { request_id, .. }
                            | BrowserViewCommandResponse::Rejected { request_id, .. }
                            | BrowserViewCommandResponse::Stale { request_id, .. }
                            | BrowserViewCommandResponse::Unsupported { request_id, .. }
                            | BrowserViewCommandResponse::OutcomeUnknown { request_id, .. } => request_id,
                        };
                        if let Some(sender) = responses.lock().await.remove(request_id) {
                            let _ = sender.send(response);
                        }
                    }
                    Ok(HelperOutput::Failed { code, message }) => {
                        send_failed(&snapshot, &barrier, &events, code, message).await;
                    }
                    Err(_) => {
                        send_failed(
                            &snapshot,
                            &barrier,
                            &events,
                            "browser_helper_protocol".into(),
                            "Private browser helper emitted invalid control data".into(),
                        )
                        .await;
                    }
                },
                _ => break,
            }
        }
    }
    fail_pending_responses(
        &responses,
        &snapshot,
        "browser_helper_stopped",
        "Private browser helper stopped before confirming the command outcome",
    )
    .await;
    send_failed(
        &snapshot,
        &barrier,
        &events,
        "browser_helper_stopped".into(),
        "Private browser helper stopped; frame transport is unavailable".into(),
    )
    .await;
}

async fn write_input(
    stdin: &mut tokio::process::ChildStdin,
    input: &HelperInput,
) -> Result<(), std::io::Error> {
    let encoded = serde_json::to_vec(input).expect("helper input is serializable");
    stdin.write_all(&encoded).await?;
    stdin.write_all(b"\n").await
}
async fn send_failed(
    snapshot: &Arc<Mutex<BrowserViewSnapshot>>,
    barrier: &Arc<Mutex<()>>,
    events: &broadcast::Sender<BrowserViewEvent>,
    code: String,
    message: String,
) {
    let _barrier = barrier.lock().await;
    let metadata = {
        let mut value = snapshot.lock().await;
        value.metadata_sequence = value.metadata_sequence.saturating_add(1);
        BrowserViewEventMetadata {
            view_id: value.identity.view_id.clone(),
            stream_epoch: value.identity.stream_epoch,
            metadata_sequence: value.metadata_sequence,
        }
    };
    let _ = events.send(BrowserViewEvent::Failed {
        metadata,
        code,
        message,
    });
}
async fn fail_pending_responses(
    responses: &Arc<Mutex<HashMap<String, oneshot::Sender<BrowserViewCommandResponse>>>>,
    snapshot: &Arc<Mutex<BrowserViewSnapshot>>,
    code: &str,
    message: &str,
) {
    let pending = {
        let mut responses = responses.lock().await;
        responses.drain().collect::<Vec<_>>()
    };
    if pending.is_empty() {
        return;
    }
    let (view_id, stream_epoch) = {
        let snapshot = snapshot.lock().await;
        (
            snapshot.identity.view_id.clone(),
            snapshot.identity.stream_epoch,
        )
    };
    for (request_id, sender) in pending {
        let _ = sender.send(BrowserViewCommandResponse::OutcomeUnknown {
            view_id: view_id.clone(),
            stream_epoch,
            request_id,
            code: code.to_owned(),
            message: message.to_owned(),
        });
    }
}
fn new_grant(view_id: &str, stream_epoch: u64) -> Result<BrowserViewFrameGrant, InspectionError> {
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            InspectionError::new(
                "browser_helper_unavailable",
                "System clock predates Unix epoch",
            )
        })?
        .saturating_add(GRANT_LIFETIME)
        .as_millis()
        .to_string();
    Ok(BrowserViewFrameGrant {
        view_id: view_id.into(),
        stream_epoch,
        grant: Uuid::new_v4().to_string(),
        expires_at: expires,
        envelope: BrowserViewFrameEnvelopeV2::default(),
    })
}
fn path_text(path: &Path) -> Result<String, InspectionError> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        InspectionError::new("browser_helper_unavailable", "Helper path is not UTF-8")
    })
}
fn empty_snapshot() -> BrowserViewSnapshot {
    BrowserViewSnapshot {
        identity: BrowserViewIdentity {
            association_key: String::new(),
            browser_incarnation: String::new(),
            view_id: String::new(),
            stream_epoch: 0,
        },
        metadata_sequence: 0,
        targets: Vec::new(),
        displayed_target_id: None,
        document: None,
        viewport: None,
        navigation: None,
        cursor: None,
        focus: BrowserViewFocusState {
            page_focused: false,
            editable: false,
            selection_available: false,
            composition_active: false,
        },
        blocker: None,
        capabilities: BrowserViewCapabilities {
            pointer_input: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            keyboard_input: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            text_input: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            composition_input: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            clipboard_read: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            clipboard_write: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            dialogs: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            file_chooser: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            downloads: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            permissions: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            inspection: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            capture: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            drafts: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
            audio: cockpit_protocol::browser_view::BrowserViewCapability::Unavailable,
        },
        control: BrowserViewControlState {
            status: BrowserViewControlStatus::Lost,
            controller_view_id: None,
            lease_generation: 0,
            next_input_sequence: 0,
            can_take_control: false,
        },
        frame_grant: None,
    }
}
fn event_metadata(event: &BrowserViewEvent) -> &BrowserViewEventMetadata {
    match event {
        BrowserViewEvent::Attached { metadata, .. }
        | BrowserViewEvent::TargetsChanged { metadata, .. }
        | BrowserViewEvent::DocumentChanged { metadata, .. }
        | BrowserViewEvent::ViewportChanged { metadata, .. }
        | BrowserViewEvent::NavigationChanged { metadata, .. }
        | BrowserViewEvent::CursorChanged { metadata, .. }
        | BrowserViewEvent::FocusChanged { metadata, .. }
        | BrowserViewEvent::BlockerChanged { metadata, .. }
        | BrowserViewEvent::CapabilitiesChanged { metadata, .. }
        | BrowserViewEvent::ControlChanged { metadata, .. }
        | BrowserViewEvent::FrameDescriptor { metadata, .. }
        | BrowserViewEvent::FrameTransportRevoked { metadata, .. }
        | BrowserViewEvent::Failed { metadata, .. }
        | BrowserViewEvent::Closed { metadata, .. } => metadata,
    }
}
fn apply_event_snapshot(
    snapshot: &mut BrowserViewSnapshot,
    event: &BrowserViewEvent,
    attached_is_current: bool,
) {
    match event {
        BrowserViewEvent::Attached { snapshot: next, .. } if attached_is_current => {
            *snapshot = next.clone();
        }
        BrowserViewEvent::Attached { .. } => {}
        BrowserViewEvent::TargetsChanged {
            targets,
            displayed_target_id,
            ..
        } => {
            snapshot.targets = targets.clone();
            snapshot.displayed_target_id = displayed_target_id.clone();
        }
        BrowserViewEvent::DocumentChanged { document, .. } => {
            snapshot.document = document.clone();
        }
        BrowserViewEvent::ViewportChanged { viewport, .. } => {
            snapshot.viewport = viewport.clone();
        }
        BrowserViewEvent::NavigationChanged { navigation, .. } => {
            snapshot.navigation = navigation.clone();
        }
        BrowserViewEvent::CursorChanged { cursor, .. } => {
            snapshot.cursor = cursor.clone();
        }
        BrowserViewEvent::FocusChanged { focus, .. } => {
            snapshot.focus = focus.clone();
        }
        BrowserViewEvent::BlockerChanged { blocker, .. } => {
            snapshot.blocker = blocker.clone();
        }
        BrowserViewEvent::CapabilitiesChanged { capabilities, .. } => {
            snapshot.capabilities = capabilities.clone();
        }
        BrowserViewEvent::ControlChanged { control, .. } => {
            snapshot.control = control.clone();
        }
        // Frame descriptors are delivered separately from the metadata
        // snapshot. Advancing the sequence is intentional, but descriptor
        // geometry must not overwrite the last authoritative page state.
        BrowserViewEvent::FrameDescriptor { .. }
        | BrowserViewEvent::FrameTransportRevoked { .. }
        | BrowserViewEvent::Failed { .. }
        | BrowserViewEvent::Closed { .. } => {}
    }
}
fn replace_event_metadata(
    event: BrowserViewEvent,
    metadata: BrowserViewEventMetadata,
) -> BrowserViewEvent {
    match event {
        BrowserViewEvent::Attached { mut snapshot, .. } => {
            snapshot.metadata_sequence = metadata.metadata_sequence;
            BrowserViewEvent::Attached { metadata, snapshot }
        }
        BrowserViewEvent::TargetsChanged {
            targets,
            displayed_target_id,
            ..
        } => BrowserViewEvent::TargetsChanged {
            metadata,
            targets,
            displayed_target_id,
        },
        BrowserViewEvent::DocumentChanged { document, .. } => {
            BrowserViewEvent::DocumentChanged { metadata, document }
        }
        BrowserViewEvent::ViewportChanged { viewport, .. } => {
            BrowserViewEvent::ViewportChanged { metadata, viewport }
        }
        BrowserViewEvent::NavigationChanged { navigation, .. } => {
            BrowserViewEvent::NavigationChanged {
                metadata,
                navigation,
            }
        }
        BrowserViewEvent::CursorChanged { cursor, .. } => {
            BrowserViewEvent::CursorChanged { metadata, cursor }
        }
        BrowserViewEvent::FocusChanged { focus, .. } => {
            BrowserViewEvent::FocusChanged { metadata, focus }
        }
        BrowserViewEvent::BlockerChanged { blocker, .. } => {
            BrowserViewEvent::BlockerChanged { metadata, blocker }
        }
        BrowserViewEvent::CapabilitiesChanged { capabilities, .. } => {
            BrowserViewEvent::CapabilitiesChanged {
                metadata,
                capabilities,
            }
        }
        BrowserViewEvent::ControlChanged { control, .. } => {
            BrowserViewEvent::ControlChanged { metadata, control }
        }
        BrowserViewEvent::FrameDescriptor { descriptor, .. } => BrowserViewEvent::FrameDescriptor {
            metadata,
            descriptor,
        },
        BrowserViewEvent::FrameTransportRevoked { code, message, .. } => {
            BrowserViewEvent::FrameTransportRevoked {
                metadata,
                code,
                message,
            }
        }
        BrowserViewEvent::Failed { code, message, .. } => BrowserViewEvent::Failed {
            metadata,
            code,
            message,
        },
        BrowserViewEvent::Closed { reason, .. } => BrowserViewEvent::Closed { metadata, reason },
    }
}
fn command_name(command: &cockpit_protocol::browser_view::BrowserViewCommand) -> &'static str {
    match command {
        cockpit_protocol::browser_view::BrowserViewCommand::TakeControl { .. } => "take_control",
        cockpit_protocol::browser_view::BrowserViewCommand::ReleaseControl { .. } => {
            "release_control"
        }
        cockpit_protocol::browser_view::BrowserViewCommand::Detach => "detach",
        cockpit_protocol::browser_view::BrowserViewCommand::Resize { .. } => "resize",
        cockpit_protocol::browser_view::BrowserViewCommand::Wheel { .. } => "wheel",
        cockpit_protocol::browser_view::BrowserViewCommand::Keyboard { .. } => "keyboard",
        cockpit_protocol::browser_view::BrowserViewCommand::Text { .. } => "text",
        cockpit_protocol::browser_view::BrowserViewCommand::Composition { .. } => "composition",
        cockpit_protocol::browser_view::BrowserViewCommand::Clipboard { .. } => "clipboard",
        cockpit_protocol::browser_view::BrowserViewCommand::Tab { .. } => "tab",
        cockpit_protocol::browser_view::BrowserViewCommand::Dialog { .. } => "dialog",
        cockpit_protocol::browser_view::BrowserViewCommand::File { .. } => "file",
        cockpit_protocol::browser_view::BrowserViewCommand::Download { .. } => "download",
        cockpit_protocol::browser_view::BrowserViewCommand::Permission { .. } => "permission",
        cockpit_protocol::browser_view::BrowserViewCommand::Inspect { .. } => "inspect",
        cockpit_protocol::browser_view::BrowserViewCommand::Capture { .. } => "capture",
        cockpit_protocol::browser_view::BrowserViewCommand::Draft { .. } => "draft",
        cockpit_protocol::browser_view::BrowserViewCommand::Navigation { .. }
        | cockpit_protocol::browser_view::BrowserViewCommand::Pointer { .. } => "implemented",
    }
}
