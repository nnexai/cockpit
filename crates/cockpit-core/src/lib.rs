use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use cockpit_protocol::v1::{
    CockpitMode, FocusRequest, FocusResponse, HerdrCompatibility, PaneMoveDestination,
    ResourceMutationRequest, ResourceMutationResponse, SessionListResponse,
    SessionSnapshotResponse, StatusResponse, TerminalCommand, TerminalOpenRequest,
    TerminalStreamMessage,
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
    /// Compatibility is scoped to a session: a failed or incompatible
    /// session never poisons another session's cache entry.
    session_compatibility: Arc<RwLock<HashMap<String, HerdrCompatibility>>>,
    /// Installation-wide compatibility is retained separately for status and
    /// the session-list operation, which do not have a session identifier.
    compatibility: Arc<RwLock<Option<HerdrCompatibility>>>,
}

impl CockpitService {
    pub fn new(mode: CockpitMode, adapter: Arc<dyn HerdrAdapter>) -> Self {
        Self {
            mode,
            adapter,
            session_compatibility: Arc::new(RwLock::new(HashMap::new())),
            compatibility: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn status(&self) -> StatusResponse {
        let herdr = if self.mode == CockpitMode::Test {
            HerdrCompatibility::Unavailable {
                code: "live_inspection_disabled".to_owned(),
                message: "live Herdr inspection is disabled in test mode".to_owned(),
            }
        } else {
            match self.adapter.inspect().await {
                Ok(status) => status,
                Err(error) => HerdrCompatibility::Unavailable {
                    code: error.code,
                    message: error.message,
                },
            }
        };
        self.update_compatibility(&herdr).await;

        StatusResponse {
            protocol_version: "1".to_owned(),
            cockpit_version: env!("CARGO_PKG_VERSION").to_owned(),
            mode: self.mode,
            herdr,
        }
    }

    /// Return the named sessions after installation compatibility is checked.
    pub async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
        self.ensure_live()?;
        self.inspect_if_needed().await?;
        self.adapter.sessions().await
    }

    pub async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<SessionSnapshotResponse, InspectionError> {
        self.ensure_live()?;
        validate_session_id(session_id)?;
        self.inspect_session_if_needed(session_id).await?;
        let snapshot = self.adapter.session_snapshot(session_id).await?;
        validate_snapshot_session(session_id, &snapshot)?;
        Ok(snapshot)
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
        let response = self.adapter.focus(session_id, request).await?;
        if response.session_id != session_id
            || response.kind != request.kind
            || response.target_id != request.target_id
        {
            return Err(InspectionError::new(
                "invalid_focus_response",
                "Herdr returned a focus response for a different resource",
            ));
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
        let response = self.adapter.mutate(session_id, request).await?;
        if response.session_id != session_id {
            return Err(InspectionError::new(
                "invalid_mutation_response",
                "Herdr returned a mutation response for a different session",
            ));
        }
        validate_snapshot_session(session_id, &response.snapshot)?;
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
        self.adapter.subscribe_session(session_id, snapshot).await
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
        let snapshot = self.adapter.session_snapshot(&request.session_id).await?;
        validate_terminal_pane_visible(&request.session_id, &request.pane_id, &snapshot)?;
        self.adapter.open_terminal(request).await
    }

    async fn inspect_if_needed(&self) -> Result<(), InspectionError> {
        let compatible = self
            .compatibility
            .read()
            .await
            .as_ref()
            .is_some_and(|status| matches!(status, HerdrCompatibility::Compatible { .. }));
        if compatible {
            return Ok(());
        }

        match self.adapter.inspect().await {
            Ok(status @ HerdrCompatibility::Compatible { .. }) => {
                self.update_compatibility(&status).await;
                Ok(())
            }
            Ok(HerdrCompatibility::Incompatible { code, message, .. })
            | Ok(HerdrCompatibility::Unavailable { code, message }) => {
                self.clear_compatibility().await;
                Err(InspectionError::new(code, message))
            }
            Err(error) => {
                self.clear_compatibility().await;
                Err(error)
            }
        }
    }

    async fn inspect_session_if_needed(&self, session_id: &str) -> Result<(), InspectionError> {
        let compatible = self
            .session_compatibility
            .read()
            .await
            .get(session_id)
            .is_some_and(|status| matches!(status, HerdrCompatibility::Compatible { .. }));
        if compatible {
            return Ok(());
        }

        match self.adapter.inspect_session(session_id).await {
            Ok(status @ HerdrCompatibility::Compatible { .. }) => {
                self.session_compatibility
                    .write()
                    .await
                    .insert(session_id.to_owned(), status);
                Ok(())
            }
            Ok(HerdrCompatibility::Incompatible { code, message, .. })
            | Ok(HerdrCompatibility::Unavailable { code, message }) => {
                self.session_compatibility.write().await.remove(session_id);
                Err(InspectionError::new(code, message))
            }
            Err(error) => {
                self.session_compatibility.write().await.remove(session_id);
                Err(error)
            }
        }
    }

    async fn update_compatibility(&self, status: &HerdrCompatibility) {
        let session_cache_must_clear = !matches!(status, HerdrCompatibility::Compatible { .. });
        {
            let mut cached = self.compatibility.write().await;
            *cached = if session_cache_must_clear {
                None
            } else {
                Some(status.clone())
            };
        }
        if session_cache_must_clear {
            self.session_compatibility.write().await.clear();
        }
    }

    async fn clear_compatibility(&self) {
        *self.compatibility.write().await = None;
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
