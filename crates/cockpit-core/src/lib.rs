pub mod browser;
pub mod browser_feedback;

pub mod comments;
pub mod config;
pub mod context;
pub mod context_assets;
pub mod context_media;
pub mod context_search;
pub mod extension_adapter;
pub mod paste_adapter;
pub mod process;
pub mod project_adapter;
mod project_store;
pub mod project_teardown;
pub mod projects;
pub mod repositories;
pub mod review;
pub mod sources;
pub mod space_git;

pub use browser::{BrowserHerdrAdapter, BrowserHerdrSnapshot, BrowserService};
pub use extension_adapter::{ExtensionHerdrAdapter, ExtensionLaunch, ExtensionPaneEvidence};
pub use paste_adapter::CommentPasteAdapter;
pub use project_adapter::ProjectHerdrAdapter;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use cockpit_protocol::v1::{
    CockpitCapabilities, CockpitMode, FocusRequest, FocusResponse, HerdrCompatibility,
    PaneMoveDestination, ResourceMutationRequest, ResourceMutationResponse, SessionListResponse,
    SessionSnapshotResponse, SpaceGitStatusResponse, StatusResponse, TerminalCommand,
    TerminalOpenRequest, TerminalStreamMessage,
};
use tokio::sync::{RwLock, mpsc};

/// A failure while obtaining Herdr's compatibility status or session state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionError {
    pub code: String,
    pub message: String,
}

impl InspectionError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for InspectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for InspectionError {}

/// A change notification from a named Herdr session.
///
/// Herdr 0.8.2 does not expose a lifecycle cursor. The adapter preserves
/// arrival order and reports only whether the cached snapshot should be
/// refreshed or the stream's connection state changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionChange {
    Changed,
    Stale { code: String, message: String },
    Disconnected { code: String, message: String },
}

#[derive(Debug)]
pub struct SessionSubscription {
    pub messages: mpsc::Receiver<SessionChange>,
}

/// A live terminal attachment owned by its caller.
#[derive(Debug)]
pub struct TerminalSession {
    pub stream_id: String,
    pub messages: mpsc::Receiver<TerminalStreamMessage>,
    pub commands: mpsc::Sender<TerminalCommand>,
}

/// Application-facing operations needed from a Herdr adapter.
#[async_trait]
pub trait HerdrAdapter: Send + Sync {
    async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError>;
    async fn inspect_session(
        &self,
        session_id: &str,
    ) -> Result<HerdrCompatibility, InspectionError>;
    async fn sessions(&self) -> Result<SessionListResponse, InspectionError>;
    async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<SessionSnapshotResponse, InspectionError>;
    async fn focus(
        &self,
        session_id: &str,
        request: &FocusRequest,
    ) -> Result<FocusResponse, InspectionError>;
    async fn mutate(
        &self,
        session_id: &str,
        request: &ResourceMutationRequest,
    ) -> Result<ResourceMutationResponse, InspectionError>;
    async fn subscribe_session(
        &self,
        session_id: &str,
        snapshot: &SessionSnapshotResponse,
    ) -> Result<SessionSubscription, InspectionError>;
    async fn open_terminal(
        &self,
        request: &TerminalOpenRequest,
    ) -> Result<TerminalSession, InspectionError>;
}

/// Application service shared by every host transport.
#[derive(Clone)]
pub struct CockpitService {
    mode: CockpitMode,
    adapter: Arc<dyn HerdrAdapter>,
    compatibility: Arc<RwLock<CompatibilityCache>>,
    projects: Option<Arc<projects::ProjectService>>,
    contexts: Option<Arc<context::ContextService>>,
    comments: Option<Arc<comments::CommentsService>>,
    reviews: Option<Arc<review::ReviewService>>,
}

#[derive(Default)]
struct CompatibilityCache {
    installation: Option<HerdrCompatibility>,
    sessions: HashMap<String, HerdrCompatibility>,
    installation_generation: u64,
    session_generations: HashMap<String, u64>,
}

fn invalidates_session(error: &InspectionError) -> bool {
    matches!(
        error.code.as_str(),
        "disconnected"
            | "connection_failed"
            | "request_not_dispatched"
            | "request_outcome_unknown"
            | "response_timeout"
            | "subscription_setup_failed"
            | "subscription_setup_timeout"
            | "malformed_json"
            | "malformed_response"
            | "malformed_event"
            | "bounded_output"
            | "session_mismatch"
            | "session_identity_mismatch"
            | "terminal_attach_failed"
            | "terminal_disconnected"
            | "invalid_session_snapshot"
            | "invalid_focus_response"
            | "invalid_mutation_response"
    )
}

fn invalidates_installation(error: &InspectionError) -> bool {
    matches!(
        error.code.as_str(),
        "disconnected"
            | "connection_failed"
            | "request_not_dispatched"
            | "request_outcome_unknown"
            | "response_timeout"
            | "malformed_json"
            | "malformed_response"
            | "bounded_output"
            | "execution_failed"
            | "execution_timeout"
            | "server_not_running"
            | "endpoint_unavailable"
    )
}

impl CockpitService {
    pub fn new(mode: CockpitMode, adapter: Arc<dyn HerdrAdapter>) -> Self {
        Self {
            mode,
            adapter,
            compatibility: Arc::new(RwLock::new(CompatibilityCache::default())),
            projects: None,
            contexts: None,
            comments: None,
            reviews: None,
        }
    }

    pub fn with_projects(mut self, projects: projects::ProjectService) -> Self {
        self.projects = Some(Arc::new(projects));
        self
    }

    pub fn projects(&self) -> Result<&Arc<projects::ProjectService>, InspectionError> {
        self.projects.as_ref().ok_or_else(|| {
            InspectionError::new(
                "project_configuration_unavailable",
                "Project operations are not configured in this host",
            )
        })
    }

    pub fn with_contexts(mut self, contexts: context::ContextService) -> Self {
        self.contexts = Some(Arc::new(contexts));
        self
    }

    pub fn contexts(&self) -> Result<&Arc<context::ContextService>, InspectionError> {
        self.contexts.as_ref().ok_or_else(|| {
            InspectionError::new(
                "context_configuration_unavailable",
                "Context operations are not configured in this host",
            )
        })
    }

    pub fn with_reviews(mut self, reviews: review::ReviewService) -> Self {
        self.reviews = Some(Arc::new(reviews));
        self
    }

    pub fn reviews(&self) -> Result<&Arc<review::ReviewService>, InspectionError> {
        self.reviews.as_ref().ok_or_else(|| {
            InspectionError::new(
                "review_unavailable",
                "Review operations are not configured in this host",
            )
        })
    }

    pub fn with_comments(mut self, comments: comments::CommentsService) -> Self {
        self.comments = Some(Arc::new(comments));
        self
    }

    pub fn comments(&self) -> Result<&Arc<comments::CommentsService>, InspectionError> {
        self.comments.as_ref().ok_or_else(|| {
            InspectionError::new(
                "comments_configuration_unavailable",
                "Comment operations are not configured in this host",
            )
        })
    }

    pub async fn status(&self) -> StatusResponse {
        let herdr = if self.mode == CockpitMode::Test {
            HerdrCompatibility::Unavailable {
                code: "live_inspection_disabled".to_owned(),
                message: "live Herdr inspection is disabled in test mode".to_owned(),
            }
        } else {
            match self.installation_compatibility().await {
                Ok(status) => status,
                Err(error) => HerdrCompatibility::Unavailable {
                    code: error.code,
                    message: error.message,
                },
            }
        };
        if self.mode == CockpitMode::Test {
            self.update_compatibility(&herdr).await;
        }

        StatusResponse {
            protocol_version: "1".to_owned(),
            cockpit_version: env!("CARGO_PKG_VERSION").to_owned(),
            mode: self.mode,
            capabilities: CockpitCapabilities {
                // Herdr encodes structured pointer events; each terminal stream
                // separately reports whether its application requests mouse input.
                terminal_mouse_input: matches!(herdr, HerdrCompatibility::Compatible { .. }),
            },
            herdr,
        }
    }

    /// Return the named sessions after installation compatibility is checked.
    pub async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
        self.ensure_live()?;
        self.inspect_if_needed().await?;
        let result = self.adapter.sessions().await;
        if let Err(error) = &result
            && invalidates_installation(error)
        {
            self.clear_compatibility().await;
        }
        result
    }

    pub async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<SessionSnapshotResponse, InspectionError> {
        self.ensure_live()?;
        validate_session_id(session_id)?;
        self.inspect_session_if_needed(session_id).await?;
        let snapshot = self
            .session_result(session_id, self.adapter.session_snapshot(session_id).await)
            .await?;
        self.session_result(session_id, validate_snapshot_session(session_id, &snapshot))
            .await?;
        Ok(snapshot)
    }

    /// Branch and upstream position for the session's Space checkouts.
    pub async fn space_git_status(
        &self,
        session_id: &str,
    ) -> Result<SpaceGitStatusResponse, InspectionError> {
        let snapshot = self.session_snapshot(session_id).await?;
        Ok(space_git::read(&snapshot).await)
    }

    pub async fn focus(
        &self,
        session_id: &str,
        request: &FocusRequest,
    ) -> Result<FocusResponse, InspectionError> {
        self.ensure_live()?;
        validate_session_id(session_id)?;
        validate_resource_id(&request.target_id, "target")?;
        self.inspect_session_if_needed(session_id).await?;
        let response = self
            .session_result(session_id, self.adapter.focus(session_id, request).await)
            .await?;
        if response.session_id != session_id
            || response.kind != request.kind
            || response.target_id != request.target_id
        {
            return self
                .session_result(
                    session_id,
                    Err(InspectionError::new(
                        "invalid_focus_response",
                        "Herdr returned a focus response for a different resource",
                    )),
                )
                .await;
        }
        Ok(response)
    }

    pub async fn mutate(
        &self,
        session_id: &str,
        request: &ResourceMutationRequest,
    ) -> Result<ResourceMutationResponse, InspectionError> {
        self.ensure_live()?;
        validate_session_id(session_id)?;
        validate_mutation(request)?;
        self.inspect_session_if_needed(session_id).await?;
        let response = self
            .session_result(session_id, self.adapter.mutate(session_id, request).await)
            .await?;
        if response.session_id != session_id {
            return self
                .session_result(
                    session_id,
                    Err(InspectionError::new(
                        "invalid_mutation_response",
                        "Herdr returned a mutation response for a different session",
                    )),
                )
                .await;
        }
        self.session_result(
            session_id,
            validate_snapshot_session(session_id, &response.snapshot),
        )
        .await?;
        Ok(response)
    }

    pub async fn subscribe_session(
        &self,
        session_id: &str,
        snapshot: &SessionSnapshotResponse,
    ) -> Result<SessionSubscription, InspectionError> {
        self.ensure_live()?;
        validate_session_id(session_id)?;
        validate_snapshot_session(session_id, snapshot)?;
        self.inspect_session_if_needed(session_id).await?;
        let subscription = self
            .session_result(
                session_id,
                self.adapter.subscribe_session(session_id, snapshot).await,
            )
            .await?;
        let SessionSubscription {
            messages: mut source,
        } = subscription;
        let (sender, receiver) = mpsc::channel(32);
        let service = self.clone();
        let session_id = session_id.to_owned();
        tokio::spawn(async move {
            while let Some(change) = source.recv().await {
                if matches!(
                    change,
                    SessionChange::Stale { .. } | SessionChange::Disconnected { .. }
                ) {
                    service.invalidate_session(&session_id).await;
                }
                if sender.send(change).await.is_err() {
                    break;
                }
            }
            service.invalidate_session(&session_id).await;
        });
        Ok(SessionSubscription { messages: receiver })
    }

    pub async fn open_terminal(
        &self,
        request: &TerminalOpenRequest,
    ) -> Result<TerminalSession, InspectionError> {
        self.ensure_live()?;
        validate_session_id(&request.session_id)?;
        validate_resource_id(&request.pane_id, "pane")?;
        if request.cols == 0 || request.rows == 0 {
            return Err(InspectionError::new(
                "invalid_terminal_dimensions",
                "terminal dimensions must be greater than zero",
            ));
        }
        self.inspect_session_if_needed(&request.session_id).await?;
        let snapshot = self
            .session_result(
                &request.session_id,
                self.adapter.session_snapshot(&request.session_id).await,
            )
            .await?;
        self.session_result(
            &request.session_id,
            validate_terminal_pane_visible(&request.session_id, &request.pane_id, &snapshot),
        )
        .await?;
        self.session_result(
            &request.session_id,
            self.adapter.open_terminal(request).await,
        )
        .await
    }

    async fn installation_compatibility(&self) -> Result<HerdrCompatibility, InspectionError> {
        for attempt in 0..2 {
            let (cached, generation) = {
                let cache = self.compatibility.read().await;
                (cache.installation.clone(), cache.installation_generation)
            };
            if let Some(cached) = cached {
                return Ok(cached);
            }

            let result = self.adapter.inspect().await;
            let mut cache = self.compatibility.write().await;
            if cache.installation_generation != generation {
                drop(cache);
                if attempt == 0 {
                    continue;
                }
                return Err(InspectionError::new(
                    "compatibility_changed_during_inspection",
                    "Herdr installation compatibility changed during inspection",
                ));
            }

            cache.installation_generation = cache.installation_generation.wrapping_add(1);
            match &result {
                Ok(HerdrCompatibility::Compatible { .. }) => {
                    cache.installation = result.clone().ok();
                }
                Ok(HerdrCompatibility::Incompatible { .. })
                | Ok(HerdrCompatibility::Unavailable { .. })
                | Err(_) => {
                    cache.installation = None;
                    cache.sessions.clear();
                    for generation in cache.session_generations.values_mut() {
                        *generation = generation.wrapping_add(1);
                    }
                }
            }
            return result;
        }
        unreachable!("bounded compatibility inspection loop always returns")
    }

    async fn inspect_if_needed(&self) -> Result<(), InspectionError> {
        match self.installation_compatibility().await? {
            HerdrCompatibility::Compatible { .. } => Ok(()),
            HerdrCompatibility::Incompatible { code, message, .. }
            | HerdrCompatibility::Unavailable { code, message } => {
                Err(InspectionError::new(code, message))
            }
        }
    }

    async fn inspect_session_if_needed(&self, session_id: &str) -> Result<(), InspectionError> {
        for attempt in 0..2 {
            let (compatible, installation_generation, session_generation) = {
                let cache = self.compatibility.read().await;
                (
                    cache.sessions.get(session_id).is_some_and(|status| {
                        matches!(status, HerdrCompatibility::Compatible { .. })
                    }),
                    cache.installation_generation,
                    cache
                        .session_generations
                        .get(session_id)
                        .copied()
                        .unwrap_or_default(),
                )
            };
            if compatible {
                return Ok(());
            }

            match self.adapter.inspect_session(session_id).await {
                Ok(status @ HerdrCompatibility::Compatible { .. }) => {
                    let mut cache = self.compatibility.write().await;
                    if cache.installation_generation != installation_generation
                        || cache
                            .session_generations
                            .get(session_id)
                            .copied()
                            .unwrap_or_default()
                            != session_generation
                    {
                        drop(cache);
                        if attempt == 0 {
                            continue;
                        }
                        return Err(InspectionError::new(
                            "compatibility_changed_during_inspection",
                            "Herdr session compatibility changed during inspection",
                        ));
                    }
                    cache.sessions.insert(session_id.to_owned(), status);
                    return Ok(());
                }
                Ok(HerdrCompatibility::Incompatible { code, message, .. })
                | Ok(HerdrCompatibility::Unavailable { code, message }) => {
                    self.invalidate_session(session_id).await;
                    return Err(InspectionError::new(code, message));
                }
                Err(error) => {
                    self.invalidate_session(session_id).await;
                    return Err(error);
                }
            }
        }
        unreachable!("bounded session compatibility inspection loop always returns")
    }

    async fn update_compatibility(&self, status: &HerdrCompatibility) {
        let mut cache = self.compatibility.write().await;
        cache.installation_generation = cache.installation_generation.wrapping_add(1);
        if matches!(status, HerdrCompatibility::Compatible { .. }) {
            cache.installation = Some(status.clone());
        } else {
            cache.installation = None;
            cache.sessions.clear();
            for generation in cache.session_generations.values_mut() {
                *generation = generation.wrapping_add(1);
            }
        }
    }

    async fn clear_compatibility(&self) {
        let mut cache = self.compatibility.write().await;
        cache.installation_generation = cache.installation_generation.wrapping_add(1);
        cache.installation = None;
        cache.sessions.clear();
        for generation in cache.session_generations.values_mut() {
            *generation = generation.wrapping_add(1);
        }
    }

    async fn invalidate_session(&self, session_id: &str) {
        let mut cache = self.compatibility.write().await;
        cache.sessions.remove(session_id);
        let generation = cache
            .session_generations
            .entry(session_id.to_owned())
            .or_default();
        *generation = generation.wrapping_add(1);
    }

    async fn session_result<T>(
        &self,
        session_id: &str,
        result: Result<T, InspectionError>,
    ) -> Result<T, InspectionError> {
        if let Err(error) = &result
            && invalidates_session(error)
        {
            self.invalidate_session(session_id).await;
        }
        result
    }

    fn ensure_live(&self) -> Result<(), InspectionError> {
        if self.mode == CockpitMode::Test {
            Err(live_inspection_disabled())
        } else {
            Ok(())
        }
    }
}

fn validate_session_id(session_id: &str) -> Result<(), InspectionError> {
    validate_resource_id(session_id, "session")
}

fn validate_resource_id(value: &str, kind: &str) -> Result<(), InspectionError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(InspectionError::new(
            format!("invalid_{kind}_id"),
            format!("{kind} ID must be 1-128 ASCII alphanumeric characters or ':', '_' or '-'"),
        ))
    }
}
const MAX_LABEL_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 4096;
const MAX_MOVE_BLOCK_IDS: usize = 128;

fn validate_mutation(request: &ResourceMutationRequest) -> Result<(), InspectionError> {
    match request {
        ResourceMutationRequest::SpaceCreate { cwd, label } => {
            validate_optional_text(cwd.as_deref(), "cwd", MAX_PATH_BYTES)?;
            validate_optional_text(label.as_deref(), "label", MAX_LABEL_BYTES)
        }
        ResourceMutationRequest::SpaceRename { space_id, label } => {
            validate_resource_id(space_id, "space")?;
            validate_text(label, "label", MAX_LABEL_BYTES)
        }
        ResourceMutationRequest::SpaceMoveBlock {
            space_ids,
            before_space_id,
        } => {
            if space_ids.is_empty() || space_ids.len() > MAX_MOVE_BLOCK_IDS {
                return Err(InspectionError::new(
                    "invalid_space_move_block",
                    "space move block must contain 1-128 IDs",
                ));
            }
            let mut unique = HashSet::with_capacity(space_ids.len());
            for space_id in space_ids {
                validate_resource_id(space_id, "space")?;
                if !unique.insert(space_id.as_str()) {
                    return Err(InspectionError::new(
                        "invalid_space_move_block",
                        "space move block IDs must be unique",
                    ));
                }
            }
            if let Some(before_space_id) = before_space_id {
                validate_resource_id(before_space_id, "space")?;
                if unique.contains(before_space_id.as_str()) {
                    return Err(InspectionError::new(
                        "invalid_space_move_block",
                        "before_space_id must not be part of the moved block",
                    ));
                }
            }
            Ok(())
        }
        ResourceMutationRequest::SpaceClose { space_id }
        | ResourceMutationRequest::TabCreate { space_id, .. } => {
            validate_resource_id(space_id, "space")?;
            if let ResourceMutationRequest::TabCreate { label, .. } = request {
                validate_optional_text(label.as_deref(), "label", MAX_LABEL_BYTES)?;
            }
            Ok(())
        }
        ResourceMutationRequest::TabRename { tab_id, label } => {
            validate_resource_id(tab_id, "tab")?;
            validate_text(label, "label", MAX_LABEL_BYTES)
        }
        ResourceMutationRequest::TabMove { tab_id, .. }
        | ResourceMutationRequest::TabClose { tab_id } => validate_resource_id(tab_id, "tab"),
        ResourceMutationRequest::PaneSplit { pane_id, ratio, .. } => {
            validate_resource_id(pane_id, "pane")?;
            if let Some(ratio) = ratio
                && (!ratio.is_finite() || *ratio <= 0.0 || *ratio >= 1.0)
            {
                return Err(InspectionError::new(
                    "invalid_pane_ratio",
                    "pane ratio must be finite and between 0 and 1",
                ));
            }
            Ok(())
        }
        ResourceMutationRequest::PaneResize {
            pane_id, amount, ..
        } => {
            validate_resource_id(pane_id, "pane")?;
            if !amount.is_finite() || *amount <= 0.0 {
                return Err(InspectionError::new(
                    "invalid_pane_resize_amount",
                    "pane resize amount must be finite and positive",
                ));
            }
            Ok(())
        }
        ResourceMutationRequest::PaneRename { pane_id, label } => {
            validate_resource_id(pane_id, "pane")?;
            validate_optional_text(label.as_deref(), "label", MAX_LABEL_BYTES)
        }
        ResourceMutationRequest::PaneSwap {
            source_pane_id,
            target_pane_id,
        } => {
            validate_resource_id(source_pane_id, "source_pane")?;
            validate_resource_id(target_pane_id, "target_pane")?;
            if source_pane_id == target_pane_id {
                return Err(InspectionError::new(
                    "invalid_pane_swap",
                    "source and target panes must differ",
                ));
            }
            Ok(())
        }
        ResourceMutationRequest::PaneMove {
            pane_id,
            destination,
        } => {
            validate_resource_id(pane_id, "pane")?;
            validate_pane_destination(destination)
        }
        ResourceMutationRequest::PaneZoom { pane_id, .. }
        | ResourceMutationRequest::PaneClose { pane_id } => validate_resource_id(pane_id, "pane"),
    }
}

fn validate_pane_destination(destination: &PaneMoveDestination) -> Result<(), InspectionError> {
    match destination {
        PaneMoveDestination::ExistingTab {
            tab_id,
            target_pane_id,
            ratio,
            ..
        } => {
            validate_resource_id(tab_id, "tab")?;
            if let Some(target_pane_id) = target_pane_id {
                validate_resource_id(target_pane_id, "target_pane")?;
            }
            if let Some(ratio) = ratio
                && (!ratio.is_finite() || *ratio <= 0.0 || *ratio >= 1.0)
            {
                return Err(InspectionError::new(
                    "invalid_pane_ratio",
                    "pane ratio must be finite and between 0 and 1",
                ));
            }
            Ok(())
        }
        PaneMoveDestination::NewTab { space_id, label } => {
            if let Some(space_id) = space_id {
                validate_resource_id(space_id, "space")?;
            }
            validate_optional_text(label.as_deref(), "label", MAX_LABEL_BYTES)
        }
        PaneMoveDestination::NewSpace { label, tab_label } => {
            validate_optional_text(label.as_deref(), "label", MAX_LABEL_BYTES)?;
            validate_optional_text(tab_label.as_deref(), "tab_label", MAX_LABEL_BYTES)
        }
    }
}

fn validate_optional_text(
    value: Option<&str>,
    field: &str,
    max_bytes: usize,
) -> Result<(), InspectionError> {
    if let Some(value) = value {
        validate_text(value, field, max_bytes)?;
    }
    Ok(())
}

fn validate_text(value: &str, field: &str, max_bytes: usize) -> Result<(), InspectionError> {
    if !value.trim().is_empty() && value.len() <= max_bytes {
        Ok(())
    } else {
        Err(InspectionError::new(
            format!("invalid_{field}"),
            format!("{field} must be nonempty and no longer than {max_bytes} bytes"),
        ))
    }
}

fn validate_snapshot_session(
    session_id: &str,
    snapshot: &SessionSnapshotResponse,
) -> Result<(), InspectionError> {
    if snapshot.session_id == session_id {
        Ok(())
    } else {
        Err(InspectionError::new(
            "invalid_session_snapshot",
            "Herdr returned a snapshot for a different session",
        ))
    }
}
fn validate_terminal_pane_visible(
    session_id: &str,
    pane_id: &str,
    snapshot: &SessionSnapshotResponse,
) -> Result<(), InspectionError> {
    validate_snapshot_session(session_id, snapshot)?;
    let Some(focused_tab_id) = snapshot.focused_tab_id.as_deref() else {
        return Err(pane_not_visible());
    };
    let belongs_to_focused_tab = snapshot
        .panes
        .iter()
        .any(|pane| pane.id == pane_id && pane.tab_id == focused_tab_id);
    let present_in_focused_layout = snapshot.layouts.iter().any(|layout| {
        layout.tab_id == focused_tab_id && layout.panes.iter().any(|pane| pane.pane_id == pane_id)
    });
    if belongs_to_focused_tab && present_in_focused_layout {
        Ok(())
    } else {
        Err(pane_not_visible())
    }
}

fn pane_not_visible() -> InspectionError {
    InspectionError::new(
        "pane_not_visible",
        "terminal pane is not visible in the focused tab",
    )
}

fn live_inspection_disabled() -> InspectionError {
    InspectionError::new(
        "live_inspection_disabled",
        "live Herdr inspection is disabled in test mode",
    )
}
