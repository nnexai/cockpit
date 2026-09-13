use async_trait::async_trait;
use cockpit_protocol::browser::{
    BrowserAction, BrowserAssociation, BrowserConnectionState, BrowserRequest, BrowserResponse,
    BrowserTarget,
};
use cockpit_protocol::browser_view::BrowserViewOpenRequest;
use cockpit_protocol::v1::SessionSnapshotResponse;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UnixStream},
    sync::Mutex,
};
use uuid::Uuid;

use crate::InspectionError;
use crate::config::BrowserConfiguration;
use crate::process::run_bounded_command;

mod delivery;
pub mod drafts;

const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
const MAX_ASSOCIATIONS: usize = 1024;
const CLI_OUTPUT_LIMIT: usize = 64 * 1024;
const CLI_TIMEOUT: Duration = Duration::from_secs(15);
const REQUIRED_PLAYWRIGHT_CLI_VERSION: &str = "0.1.5";
const SOCKET_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_OPENED_TAB_BYTES: usize = 20;

/// A snapshot pinned to the exact Herdr endpoint that served it.
#[derive(Clone, Debug)]
pub struct BrowserHerdrSnapshot {
    pub endpoint_identity: String,
    pub endpoint_path: String,
    pub snapshot: SessionSnapshotResponse,
}

/// The browser service's fresh-only Herdr authority.
#[async_trait]
pub trait BrowserHerdrAdapter: Send + Sync {
    async fn browser_snapshot(
        &self,
        session_id: &str,
    ) -> Result<BrowserHerdrSnapshot, InspectionError>;
}

#[derive(Clone)]
pub struct BrowserService {
    configuration: BrowserConfiguration,
    root: Arc<PathBuf>,
    owner_id: String,
    adapter: Arc<dyn BrowserHerdrAdapter>,
    operation_lock: Arc<Mutex<()>>,
    shutting_down: Arc<AtomicBool>,
    feedback: Arc<crate::browser_feedback::BrowserFeedbackStore>,
    paste_adapter: Option<Arc<dyn crate::paste_adapter::CommentPasteAdapter>>,
}

/// A host-only capability for attaching the private inline helper. This type is
/// intentionally assembled only after fresh endpoint and target ownership
/// checks.
#[derive(Clone)]
pub struct BrowserRuntimeAttachment {
    pub association_key: String,
    pub browser_incarnation: String,
    pub session_id: String,
    pub space_id: String,
    pub profile_path: PathBuf,
    pub cdp_endpoint: String,
    /// Stable CDP target identity selected for this association.
    pub target_id: String,
    pub playwright_core: Option<PathBuf>,
    pub node_executable: Option<PathBuf>,
    pub helper_module: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BrowserReceipt {
    association_key: String,
    owner_id: String,
    endpoint_identity: String,
    endpoint_path: String,
    session_id: String,
    space_id: String,
    space_label: String,
    playwright_session: String,
    working_directory: String,
    profile_path: String,
    config_path: String,
    /// The loopback endpoint and browser identity captured after a verified
    /// inline launch. Both must still match before a helper can attach.
    #[serde(default)]
    cdp_endpoint: Option<String>,
    #[serde(default)]
    cdp_browser_identity: Option<String>,
    intent: ReceiptIntent,
    state: ReceiptState,
    /// Stable CDP target identity; absent in legacy receipts and unresolved opens.
    #[serde(default)]
    target_id: Option<String>,
    /// Historical Playwright CLI tab index, never used as attachment authority.
    opened_tab: Option<String>,
    incarnation: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReceiptState {
    PendingOpen,
    Open,
    Closing,
    Closed,
    OutcomeUnknown,
    Disconnected,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReceiptIntent {
    None,
    Launch,
    LaunchNavigation,
    TabNew,
    Close,
}

impl BrowserService {
    pub fn new(
        configuration: BrowserConfiguration,
        state_root: PathBuf,
        adapter: Arc<dyn BrowserHerdrAdapter>,
    ) -> Result<Self, InspectionError> {
        let root = state_root.join("browser");
        prepare_root(&root)?;
        let root = root.canonicalize().map_err(|_| {
            InspectionError::new(
                "browser_state_unavailable",
                "cannot resolve browser state directory",
            )
        })?;
        for name in [
            "associations",
            "profiles",
            "workspaces",
            "configs",
        ] {
            prepare_root(&root.join(name))?;
        }
        let feedback = crate::browser_feedback::BrowserFeedbackStore::new(
            state_root,
            crate::browser_feedback::BrowserFeedbackOptions {
                retention_seconds: configuration.feedback_retention_seconds,
                max_store_bytes: configuration.feedback_max_store_bytes,
            },
        )?;
        Ok(Self {
            configuration,
            root: Arc::new(root),
            owner_id: Uuid::new_v4().to_string(),
            adapter,
            operation_lock: Arc::new(Mutex::new(())),
            shutting_down: Arc::new(AtomicBool::new(false)),
            feedback: Arc::new(feedback),
            paste_adapter: None,
        })
    }

    pub fn with_paste_adapter(
        mut self,
        adapter: Arc<dyn crate::paste_adapter::CommentPasteAdapter>,
    ) -> Self {
        self.paste_adapter = Some(adapter);
        self
    }

    pub async fn execute(
        &self,
        request: BrowserRequest,
    ) -> Result<BrowserResponse, InspectionError> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(InspectionError::new(
                "browser_runtime_stopping",
                "browser runtime is shutting down",
            ));
        }
        let _operation = self.operation_lock.lock().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(InspectionError::new(
                "browser_runtime_stopping",
                "browser runtime is shutting down",
            ));
        }
        let snapshot = self.resolve_target(&request.target).await?;
        let key = association_key(
            &snapshot.endpoint_identity,
            &snapshot.session_id,
            &snapshot.space_id,
        );
        let mut receipt = match self.load(&key)? {
            Some(receipt) => receipt,
            None if matches!(&request.action, BrowserAction::Open { .. }) => {
                if self.load_all()?.len() >= MAX_ASSOCIATIONS {
                    return Err(InspectionError::new(
                        "browser_state_limit",
                        "too many browser associations",
                    ));
                }
                self.load_or_create(&snapshot)?
            }
            None => {
                return Ok(BrowserResponse {
                    association: None,
                    connection: BrowserConnectionState::Absent,
                    message: "no browser association exists for this Space".into(),
                });
            }
        };
        receipt.space_label = snapshot.space_label;
        self.store(&receipt)?;

        match request.action {
            BrowserAction::Open { url } => self.open(&mut receipt, url.as_deref()).await,
            BrowserAction::Status => self.status(&mut receipt).await,
            BrowserAction::Close => self.close(&mut receipt).await,
        }
    }

    /// Produces an in-memory, profile- and incarnation-bound attachment only
    /// after freshly resolving Herdr authority. It deliberately does not start
    /// or communicate with the helper while the operation lock is held.
    #[doc(hidden)]
    pub async fn browser_runtime_attachment(
        &self,
        request: &BrowserViewOpenRequest,
    ) -> Result<BrowserRuntimeAttachment, InspectionError> {
        request
            .validate()
            .map_err(|message| InspectionError::new("invalid_browser_view_request", message))?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(InspectionError::new(
                "browser_runtime_stopping",
                "browser runtime is shutting down",
            ));
        }
        let _operation = self.operation_lock.lock().await;
        let target = self.resolve_target(&request.target).await?;
        let key = association_key(
            &target.endpoint_identity,
            &target.session_id,
            &target.space_id,
        );
        let mut receipt = self.load(&key)?.ok_or_else(|| {
            InspectionError::new(
                "browser_view_association_absent",
                "Open the browser association before attaching an inline browser view",
            )
        })?;
        if receipt.state != ReceiptState::Open {
            return Err(InspectionError::new(
                "browser_view_association_unavailable",
                "Inline browser views require a verified open inline association",
            ));
        }
        let incarnation = self.inspect_live(&receipt).await?;
        if receipt.incarnation.as_deref() != Some(&incarnation) {
            return Err(InspectionError::new(
                "browser_receipt_replaced",
                "Browser incarnation changed before inline attachment",
            ));
        }
        let cdp_endpoint = receipt.cdp_endpoint.clone().ok_or_else(|| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Inline browser CDP endpoint is absent",
            )
        })?;
        let target_id = receipt.target_id.clone().ok_or_else(|| {
            InspectionError::new(
                "browser_target_unresolved",
                "Inline browser target identity is absent; reopen the browser association before attaching",
            )
        })?;
        self.store(&receipt)?;
        let cli = resolve_executable(&self.configuration.playwright_cli, "Playwright CLI")?;
        let configured_core = self
            .configuration
            .playwright_core
            .as_ref()
            .map(|path| resolve_playwright_core(path))
            .transpose()?;
        let helper_module = self
            .configuration
            .browser_helper
            .as_ref()
            .map(|path| resolve_regular_file(path, "Browser helper"))
            .transpose()?;
        let node_executable = self
            .configuration
            .node_executable
            .as_ref()
            .map(|path| resolve_executable(path, "Node runtime"))
            .transpose()?;
        Ok(BrowserRuntimeAttachment {
            association_key: receipt.association_key,
            browser_incarnation: incarnation,
            session_id: receipt.session_id,
            space_id: receipt.space_id,
            profile_path: PathBuf::from(receipt.profile_path),
            cdp_endpoint,
            target_id,
            playwright_core: configured_core.or_else(|| locate_playwright_core(&cli)),
            node_executable,
            helper_module,
        })
    }

    /// Reconcile durable associations without treating a transient Herdr failure
    /// or a restarted endpoint as permission to close or transfer an association.
    pub async fn reconcile(&self) -> Result<(), InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let receipts = self.load_all()?;
        let mut first_error = None;
        for mut receipt in receipts {
            let snapshot = match self.adapter.browser_snapshot(&receipt.session_id).await {
                Ok(snapshot) => snapshot,
                Err(_) => continue,
            };
            let result = if snapshot.endpoint_identity != receipt.endpoint_identity
                || snapshot.endpoint_path != receipt.endpoint_path
            {
                receipt.state = ReceiptState::Disconnected;
                Ok(())
            } else if !snapshot
                .snapshot
                .spaces
                .iter()
                .any(|space| space.id == receipt.space_id)
            {
                self.close(&mut receipt).await.map(|_| ())
            } else {
                receipt.space_label = snapshot
                    .snapshot
                    .spaces
                    .iter()
                    .find(|space| space.id == receipt.space_id)
                    .map(|space| space.label.clone())
                    .unwrap_or_else(|| receipt.space_label.clone());
                self.status(&mut receipt).await.map(|_| ())
            };
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
            if let Err(error) = self.store(&receipt) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Close only sessions whose daemon receipt and current socket peer still
    /// prove they are this runtime's exact, persistent association.
    pub async fn shutdown(&self) -> Result<(), InspectionError> {
        self.shutting_down.store(true, Ordering::Release);
        let _operation = self.operation_lock.lock().await;
        let receipts = self.load_all()?;
        let mut first_error = None;
        for mut receipt in receipts {
            if let Err(error) = self.close(&mut receipt).await {
                first_error.get_or_insert(error);
            }
            if let Err(error) = self.store(&receipt) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn resolve_target(
        &self,
        target: &BrowserTarget,
    ) -> Result<ResolvedTarget, InspectionError> {
        let exactly_one = target.space_id.is_some() ^ target.pane_id.is_some();
        if !exactly_one {
            return Err(InspectionError::new(
                "invalid_browser_target",
                "browser target must name exactly one Space or pane",
            ));
        }
        validate_id(&target.session_id, "session")?;
        let source = self.adapter.browser_snapshot(&target.session_id).await?;
        if source.snapshot.session_id != target.session_id {
            return Err(InspectionError::new(
                "session_mismatch",
                "Herdr snapshot belongs to another session",
            ));
        }
        if let Some(path) = &target.endpoint_path
            && path != &source.endpoint_path
        {
            return Err(InspectionError::new(
                "stale_endpoint",
                "browser target endpoint does not match the fresh Herdr endpoint",
            ));
        }
        let space_id = match (&target.space_id, &target.pane_id) {
            (Some(space_id), None) => space_id.clone(),
            (None, Some(pane_id)) => source
                .snapshot
                .panes
                .iter()
                .find(|pane| pane.id == *pane_id)
                .map(|pane| pane.space_id.clone())
                .ok_or_else(|| {
                    InspectionError::new("pane_not_visible", "browser target pane is absent")
                })?,
            _ => unreachable!(),
        };
        validate_id(&space_id, "Space")?;
        let space = source
            .snapshot
            .spaces
            .iter()
            .find(|space| space.id == space_id)
            .ok_or_else(|| {
                InspectionError::new("space_not_visible", "browser target Space is absent")
            })?;
        Ok(ResolvedTarget {
            endpoint_identity: source.endpoint_identity,
            endpoint_path: source.endpoint_path,
            session_id: target.session_id.clone(),
            space_id,
            space_label: space.label.clone(),
        })
    }

    fn load_or_create(&self, target: &ResolvedTarget) -> Result<BrowserReceipt, InspectionError> {
        let key = association_key(
            &target.endpoint_identity,
            &target.session_id,
            &target.space_id,
        );
        if let Some(receipt) = self.load(&key)? {
            if receipt.endpoint_identity == target.endpoint_identity
                && receipt.session_id == target.session_id
                && receipt.space_id == target.space_id
            {
                return Ok(receipt);
            }
            return Err(InspectionError::new(
                "association_identity_conflict",
                "browser receipt identity is inconsistent",
            ));
        }
        let working_directory = self.root.join("workspaces").join(&key);
        prepare_root(&working_directory)?;
        prepare_root(&working_directory.join(".playwright"))?;
        let profile_path = self.root.join("profiles").join(&key);
        prepare_root(&profile_path)?;
        let config_path = self.root.join("configs").join(format!("{key}.json"));
        atomic_write_json(
            &config_path,
            &launch_configuration(&self.configuration)?,
        )?;
        let receipt = BrowserReceipt {
            association_key: key.clone(),
            owner_id: self.owner_id.clone(),
            endpoint_identity: target.endpoint_identity.clone(),
            endpoint_path: target.endpoint_path.clone(),
            session_id: target.session_id.clone(),
            space_id: target.space_id.clone(),
            space_label: target.space_label.clone(),
            playwright_session: format!("cockpit-{key}"),
            working_directory: path_string(&working_directory)?,
            profile_path: path_string(&profile_path)?,
            config_path: path_string(&config_path)?,
            cdp_endpoint: None,
            cdp_browser_identity: None,
            intent: ReceiptIntent::None,
            state: ReceiptState::Closed,
            target_id: None,
            opened_tab: None,
            incarnation: None,
        };
        self.store(&receipt)?;
        Ok(receipt)
    }

    async fn open(
        &self,
        receipt: &mut BrowserReceipt,
        url: Option<&str>,
    ) -> Result<BrowserResponse, InspectionError> {
        if let Some(url) = url {
            validate_url(url)?;
        }
        if url.is_none() {
            receipt.opened_tab = None;
        }
        if receipt.intent != ReceiptIntent::None
            || matches!(
                receipt.state,
                ReceiptState::OutcomeUnknown | ReceiptState::PendingOpen | ReceiptState::Closing
            )
        {
            let response = self.status(receipt).await?;
            if receipt.intent != ReceiptIntent::None
                || response.connection == BrowserConnectionState::OutcomeUnknown
            {
                return Ok(response);
            }
        }
        match self.inspect_live(receipt).await {
            Ok(incarnation) => {
                receipt.state = ReceiptState::Open;
                receipt.owner_id = self.owner_id.clone();
                receipt.incarnation = Some(incarnation);
                self.bind_inline_cdp(receipt, false).await?;
                if receipt.target_id.is_none() {
                    self.resolve_inline_target(receipt, None).await?;
                }
                if let Some(url) = url {
                    let previous_target = receipt.target_id.clone();
                    receipt.intent = ReceiptIntent::TabNew;
                    self.store(receipt)?;
                    let args = [
                        format!("-s={}", receipt.playwright_session),
                        "tab-new".into(),
                        url.into(),
                    ];
                    match self.run_cli(receipt, &args).await {
                        Ok(output) => {
                            receipt.opened_tab = tab_index(&output);
                            self.resolve_inline_target(receipt, previous_target.as_deref()).await?;
                            receipt.intent = ReceiptIntent::None;
                        }
                        Err(error) => {
                            receipt.state = ReceiptState::OutcomeUnknown;
                            self.store(receipt)?;
                            return Err(error);
                        }
                    }
                }
                self.store(receipt)?;
                Ok(self.response(receipt, BrowserConnectionState::Open, "browser is open"))
            }
            Err(error) if !may_launch_after_inspection_failure(&error) => Err(error),
            Err(_) => {
                self.ensure_cli_version().await?;
                receipt.state = ReceiptState::PendingOpen;
                receipt.intent = if url.is_some() {
                    ReceiptIntent::LaunchNavigation
                } else {
                    ReceiptIntent::Launch
                };
                receipt.owner_id = self.owner_id.clone();
                receipt.incarnation = None;
                receipt.opened_tab = None;
                receipt.cdp_endpoint = None;
                receipt.cdp_browser_identity = None;
                self.store(receipt)?;
                let mut args = vec![format!("-s={}", receipt.playwright_session), "open".into()];
                if let Some(url) = url {
                    args.push(url.into());
                }
                args.extend([
                    format!("--profile={}", receipt.profile_path),
                    format!("--config={}", receipt.config_path),
                ]);
                match self.run_cli(receipt, &args).await {
                    Ok(output) => match self.inspect_live(receipt).await {
                        Ok(incarnation) => {
                            if url.is_some() {
                                // CLI 0.1.5 navigates tab zero at startup and omits the tab list when only one tab exists.
                                receipt.opened_tab =
                                    Some(tab_index(&output).unwrap_or_else(|| "0".to_owned()));
                            }
                            receipt.state = ReceiptState::Open;
                            receipt.intent = ReceiptIntent::None;
                            receipt.incarnation = Some(incarnation);
                            self.bind_inline_cdp(receipt, true).await?;
                            self.resolve_inline_target(receipt, None).await?;
                            self.store(receipt)?;
                            Ok(self.response(
                                receipt,
                                BrowserConnectionState::Open,
                                "browser opened",
                            ))
                        }
                        Err(_) => {
                            receipt.state = ReceiptState::OutcomeUnknown;
                            self.store(receipt)?;
                            Ok(self.response(
                                receipt,
                                BrowserConnectionState::OutcomeUnknown,
                                "browser launch completed without verifiable ownership; reconcile before retrying",
                            ))
                        }
                    },
                    Err(error) => {
                        receipt.state = if error.code == "browser_launch_failed" {
                            receipt.intent = ReceiptIntent::None;
                            ReceiptState::Closed
                        } else {
                            ReceiptState::OutcomeUnknown
                        };
                        self.store(receipt)?;
                        Err(error)
                    }
                }
            }
        }
    }

    async fn status(
        &self,
        receipt: &mut BrowserReceipt,
    ) -> Result<BrowserResponse, InspectionError> {
        let live = self.inspect_live(receipt).await;
        receipt.opened_tab = None;
        if receipt.intent == ReceiptIntent::Launch {
            if let Ok(incarnation) = &live {
                receipt.state = ReceiptState::Open;
                receipt.owner_id = self.owner_id.clone();
                receipt.intent = ReceiptIntent::None;
                receipt.incarnation = Some(incarnation.clone());
                self.bind_inline_cdp(receipt, true).await?;
                self.store(receipt)?;
                return Ok(self.response(
                    receipt,
                    BrowserConnectionState::Open,
                    "browser launch was reconciled",
                ));
            }
        }
        if matches!(
            receipt.intent,
            ReceiptIntent::LaunchNavigation | ReceiptIntent::TabNew
        ) {
            if let Ok(incarnation) = &live {
                receipt.incarnation = Some(incarnation.clone());
                receipt.owner_id = self.owner_id.clone();
                self.bind_inline_cdp(receipt, true).await?;
            }
            receipt.state = ReceiptState::OutcomeUnknown;
            self.store(receipt)?;
            return Ok(self.response(
                receipt,
                BrowserConnectionState::OutcomeUnknown,
                "The previous URL action outcome is unknown; inspect the browser with Playwright CLI. No navigation was retried",
            ));
        }
        if receipt.intent == ReceiptIntent::Close
            && live
                .as_ref()
                .is_err_and(may_launch_after_inspection_failure)
        {
            receipt.state = ReceiptState::Closed;
            receipt.intent = ReceiptIntent::None;
            receipt.incarnation = None;
            receipt.cdp_endpoint = None;
            receipt.cdp_browser_identity = None;
            self.store(receipt)?;
            return Ok(self.response(
                receipt,
                BrowserConnectionState::Closed,
                "browser close was reconciled; profile retained",
            ));
        }
        if matches!(
            receipt.state,
            ReceiptState::OutcomeUnknown | ReceiptState::PendingOpen | ReceiptState::Closing
        ) {
            return Ok(self.response(
                receipt,
                BrowserConnectionState::OutcomeUnknown,
                if live.is_ok() {
                    "browser is reachable but the prior outcome remains unknown"
                } else {
                    "browser outcome remains unknown; reconciliation is required"
                },
            ));
        }
        match live {
            Ok(incarnation) => {
                receipt.state = ReceiptState::Open;
                receipt.owner_id = self.owner_id.clone();
                receipt.incarnation = Some(incarnation);
                self.bind_inline_cdp(receipt, false).await?;
                self.store(receipt)?;
                Ok(self.response(receipt, BrowserConnectionState::Open, "browser is open"))
            }
            Err(error) if may_launch_after_inspection_failure(&error) => {
                receipt.state = ReceiptState::Closed;
                receipt.intent = ReceiptIntent::None;
                receipt.incarnation = None;
                receipt.cdp_endpoint = None;
                receipt.cdp_browser_identity = None;
                self.store(receipt)?;
                Ok(self.response(
                    receipt,
                    BrowserConnectionState::Closed,
                    "browser is closed; profile retained",
                ))
            }
            Err(error) => {
                receipt.state = ReceiptState::Disconnected;
                self.store(receipt)?;
                Ok(self.response(
                    receipt,
                    BrowserConnectionState::Disconnected,
                    &error.message,
                ))
            }
        }
    }

    async fn close(
        &self,
        receipt: &mut BrowserReceipt,
    ) -> Result<BrowserResponse, InspectionError> {
        receipt.opened_tab = None;
        if matches!(receipt.intent, ReceiptIntent::Launch | ReceiptIntent::Close)
            || receipt.state == ReceiptState::Closing
        {
            let response = self.status(receipt).await?;
            if receipt.state != ReceiptState::Open || receipt.intent != ReceiptIntent::None {
                return Ok(response);
            }
        }
        match self.inspect_live(receipt).await {
            Ok(incarnation) => receipt.incarnation = Some(incarnation),
            Err(error) if may_launch_after_inspection_failure(&error) => {
                receipt.state = ReceiptState::Closed;
                receipt.intent = ReceiptIntent::None;
                receipt.incarnation = None;
                receipt.cdp_endpoint = None;
                receipt.cdp_browser_identity = None;
                self.store(receipt)?;
                return Ok(self.response(
                    receipt,
                    BrowserConnectionState::Closed,
                    "browser is closed; profile retained",
                ));
            }
            Err(error) => return Err(error),
        }
        receipt.state = ReceiptState::Closing;
        receipt.owner_id = self.owner_id.clone();
        receipt.intent = ReceiptIntent::Close;
        self.store(receipt)?;
        let args = [format!("-s={}", receipt.playwright_session), "close".into()];
        match self.run_cli(receipt, &args).await {
            Ok(_)
                if self
                    .inspect_live(receipt)
                    .await
                    .as_ref()
                    .is_err_and(may_launch_after_inspection_failure) =>
            {
                receipt.state = ReceiptState::Closed;
                receipt.intent = ReceiptIntent::None;
                receipt.incarnation = None;
                receipt.cdp_endpoint = None;
                receipt.cdp_browser_identity = None;
                self.store(receipt)?;
                Ok(self.response(
                    receipt,
                    BrowserConnectionState::Closed,
                    "browser closed; profile retained",
                ))
            }
            Ok(_) => {
                receipt.state = ReceiptState::OutcomeUnknown;
                self.store(receipt)?;
                Ok(self.response(
                    receipt,
                    BrowserConnectionState::OutcomeUnknown,
                    "close completed without confirming daemon shutdown",
                ))
            }
            Err(error) => {
                receipt.state = ReceiptState::OutcomeUnknown;
                self.store(receipt)?;
                Err(error)
            }
        }
    }


    async fn run_cli_output(
        &self,
        receipt: &BrowserReceipt,
        args: &[String],
    ) -> Result<std::process::Output, InspectionError> {
        let executable = resolve_executable(&self.configuration.playwright_cli, "Playwright CLI")?;
        let mut command = tokio::process::Command::new(executable);
        command.current_dir(&receipt.working_directory).args(args);
        run_bounded_command(
            command,
            CLI_OUTPUT_LIMIT,
            CLI_OUTPUT_LIMIT,
            CLI_TIMEOUT,
            "Playwright CLI",
        )
        .await
    }

    async fn run_cli(
        &self,
        receipt: &BrowserReceipt,
        args: &[String],
    ) -> Result<String, InspectionError> {
        let output = self.run_cli_output(receipt, args).await?;
        if !output.status.success() {
            if args.get(1).is_some_and(|action| action == "open")
                && String::from_utf8_lossy(&output.stderr)
                    .contains("Error: Failed to launch the browser process.")
            {
                return Err(InspectionError::new(
                    "browser_launch_failed",
                    "Chromium could not start; check the configured executable and desktop display",
                ));
            }
            return Err(InspectionError::new(
                "browser_cli_failed",
                format!("Playwright CLI exited with status {}", output.status),
            ));
        }
        let text = String::from_utf8(output.stdout).map_err(|_| {
            InspectionError::new(
                "browser_cli_invalid_output",
                "Playwright CLI output was not UTF-8",
            )
        })?;
        // CLI 0.1.5 reports tool failures in stdout while exiting successfully.
        if text.lines().any(|line| line == "### Error") {
            let detail = cli_error_detail(&text);
            let message = detail.map_or_else(
                || {
                    "Playwright reported a browser operation failure; inspect the browser before another URL action"
                        .to_owned()
                },
                |detail| {
                    format!(
                        "Playwright reported a browser operation failure: {detail}"
                    )
                },
            );
            return Err(InspectionError::new("browser_action_failed", message));
        }
        Ok(text)
    }

    async fn ensure_cli_version(&self) -> Result<(), InspectionError> {
        let executable = resolve_executable(&self.configuration.playwright_cli, "Playwright CLI")?;
        let mut command = tokio::process::Command::new(executable);
        command.arg("--version");
        let output =
            run_bounded_command(command, 1024, 1024, CLI_TIMEOUT, "Playwright CLI").await?;
        let version = String::from_utf8_lossy(&output.stdout);
        let reported_version = version.lines().next().unwrap_or("").trim();
        if !output.status.success() || !compatible_playwright_cli_version(reported_version) {
            return Err(InspectionError::new(
                "browser_cli_incompatible",
                format!(
                    "Playwright CLI {REQUIRED_PLAYWRIGHT_CLI_VERSION} or a later 0.1.x bugfix release is required (found {reported_version})",
                ),
            ));
        }
        Ok(())
    }
    async fn bind_inline_cdp(
        &self,
        receipt: &mut BrowserReceipt,
        allow_unrecorded: bool,
    ) -> Result<(), InspectionError> {
        let binding = wait_for_cdp_binding(Path::new(&receipt.profile_path)).await?;
        if (receipt.cdp_endpoint.is_none() || receipt.cdp_browser_identity.is_none())
            && !allow_unrecorded
        {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Inline CDP ownership was not recorded for this browser incarnation",
            ));
        }
        if receipt
            .cdp_endpoint
            .as_deref()
            .is_some_and(|endpoint| endpoint != binding.endpoint)
            || receipt
                .cdp_browser_identity
                .as_deref()
                .is_some_and(|identity| identity != binding.browser_identity)
        {
            return Err(InspectionError::new(
                "browser_receipt_replaced",
                "Chromium CDP endpoint or browser identity changed",
            ));
        }
        receipt.cdp_endpoint = Some(binding.endpoint);
        receipt.cdp_browser_identity = Some(binding.browser_identity);
        Ok(())
    }

    async fn resolve_inline_target(
        &self,
        receipt: &mut BrowserReceipt,
        previous: Option<&str>,
    ) -> Result<(), InspectionError> {
        let endpoint = receipt.cdp_endpoint.as_deref().ok_or_else(|| {
            InspectionError::new("browser_cdp_unavailable", "Inline CDP endpoint is absent")
        })?;
        let target_id = cdp_page_target(endpoint, previous).await?;
        receipt.target_id = Some(target_id);
        Ok(())
    }

    async fn inspect_live(&self, receipt: &BrowserReceipt) -> Result<String, InspectionError> {
        let daemon = daemon_receipt(receipt)?;
        let stream = tokio::time::timeout(SOCKET_TIMEOUT, UnixStream::connect(&daemon.socket_path))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "browser_daemon_unavailable",
                    "Playwright daemon connection timed out",
                )
            })?
            .map_err(|error| {
                InspectionError::new(
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) {
                        "browser_daemon_closed"
                    } else {
                        "browser_daemon_unavailable"
                    },
                    "Playwright daemon is not reachable",
                )
            })?;
        let credentials = stream.peer_cred().map_err(|_| {
            InspectionError::new(
                "browser_daemon_unavailable",
                "cannot inspect Playwright daemon peer",
            )
        })?;
        if credentials.uid() != current_uid() {
            return Err(InspectionError::new(
                "browser_daemon_foreign",
                "Playwright daemon is owned by another user",
            ));
        }
        let pid = credentials.pid().ok_or_else(|| {
            InspectionError::new(
                "browser_daemon_unavailable",
                "Playwright daemon peer has no PID",
            )
        })?;
        let start = process_start_identity(pid).ok_or_else(|| {
            InspectionError::new(
                "browser_daemon_unavailable",
                "cannot verify Playwright daemon process generation",
            )
        })?;
        let incarnation = format!("pid={pid}:start={start}:receipt={}", daemon.hash);
        if receipt.incarnation.is_none()
            && !matches!(
                receipt.intent,
                ReceiptIntent::Launch | ReceiptIntent::LaunchNavigation
            )
        {
            return Err(InspectionError::new(
                "browser_unowned_daemon",
                "A browser was started outside this association's launch; refusing to adopt it",
            ));
        }
        if receipt.incarnation.is_some() && receipt.incarnation.as_deref() != Some(&incarnation) {
            return Err(InspectionError::new(
                "browser_receipt_replaced",
                "Playwright daemon receipt or process generation changed",
            ));
        }

        let probe = self
            .run_cli_output(
                receipt,
                &[
                    format!("-s={}", receipt.playwright_session),
                    "cookie-get".into(),
                    "__cockpit_liveness_probe__".into(),
                ],
            )
            .await?;
        let stdout = String::from_utf8_lossy(&probe.stdout);
        let stderr = String::from_utf8_lossy(&probe.stderr);
        if !probe.status.success() {
            if cli_reports_browser_closed(&stdout) || cli_reports_browser_closed(&stderr) {
                return Err(InspectionError::new(
                    "browser_daemon_closed",
                    "Playwright browser session reports that Chromium is closed",
                ));
            }
            return Err(InspectionError::new(
                "browser_cli_failed",
                format!("Playwright CLI exited with status {}", probe.status),
            ));
        }
        if let Some(detail) = cli_error_detail(&stdout) {
            if cli_reports_browser_closed(&detail) {
                return Err(InspectionError::new(
                    "browser_daemon_closed",
                    "Playwright browser session reports that Chromium is closed",
                ));
            }
            return Err(InspectionError::new(
                "browser_action_failed",
                format!("Playwright reported a browser operation failure: {detail}"),
            ));
        }
        Ok(incarnation)
    }

    fn response(
        &self,
        receipt: &BrowserReceipt,
        connection: BrowserConnectionState,
        message: &str,
    ) -> BrowserResponse {
        BrowserResponse {
            association: Some(self.association(receipt, connection)),
            connection,
            message: message.into(),
        }
    }

    fn association(
        &self,
        receipt: &BrowserReceipt,
        connection: BrowserConnectionState,
    ) -> BrowserAssociation {
        BrowserAssociation {
            association_key: receipt.association_key.clone(),
            owner_id: receipt.owner_id.clone(),
            session_id: receipt.session_id.clone(),
            space_id: receipt.space_id.clone(),
            space_label: receipt.space_label.clone(),
            playwright_session: receipt.playwright_session.clone(),
            working_directory: receipt.working_directory.clone(),
            profile_path: receipt.profile_path.clone(),
            invocation: format!(
                "cd -- {} && {} -s={} <command>",
                shell_quote(&receipt.working_directory),
                shell_quote(&self.configuration.playwright_cli.display().to_string()),
                shell_quote(&receipt.playwright_session),
            ),
            connection,
            incarnation: receipt.incarnation.clone(),
            opened_tab: receipt.opened_tab.clone(),
        }
    }

    fn association_path(&self, key: &str) -> PathBuf {
        self.root.join("associations").join(format!("{key}.json"))
    }
    fn store(&self, receipt: &BrowserReceipt) -> Result<(), InspectionError> {
        atomic_write_json(&self.association_path(&receipt.association_key), receipt)
    }
    fn load(&self, key: &str) -> Result<Option<BrowserReceipt>, InspectionError> {
        if key.len() != 24 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(InspectionError::new(
                "browser_state_corrupt",
                "Invalid browser association key",
            ));
        }
        let Some(receipt) = read_json::<BrowserReceipt>(&self.association_path(key))? else {
            return Ok(None);
        };
        if receipt.association_key != key
            || association_key(
                &receipt.endpoint_identity,
                &receipt.session_id,
                &receipt.space_id,
            ) != key
            || receipt.playwright_session != format!("cockpit-{key}")
            || Path::new(&receipt.working_directory) != self.root.join("workspaces").join(key)
            || Path::new(&receipt.profile_path) != self.root.join("profiles").join(key)
            || Path::new(&receipt.config_path)
                != self.root.join("configs").join(format!("{key}.json"))
        {
            return Err(InspectionError::new(
                "browser_state_corrupt",
                "Browser receipt does not match its association",
            ));
        }
        for path in [&receipt.working_directory, &receipt.profile_path] {
            crate::project_store::open_dir_nofollow_absolute(Path::new(path)).map_err(|_| {
                InspectionError::new(
                    "unsafe_path",
                    "Browser profile or working directory is unsafe",
                )
            })?;
        }
        if !receipt_config_has_inline_debugging(&receipt)? {
            return Err(InspectionError::new(
                "browser_receipt_mismatch",
                "Inline browser launch configuration lacks the private loopback CDP binding",
            ));
        }
        Ok(Some(receipt))
    }
    fn load_all(&self) -> Result<Vec<BrowserReceipt>, InspectionError> {
        let directory = self.root.join("associations");
        let mut values = Vec::new();
        for (index, entry) in fs::read_dir(&directory)
            .map_err(|_| {
                InspectionError::new(
                    "browser_state_unavailable",
                    "cannot list browser associations",
                )
            })?
            .enumerate()
        {
            if index >= MAX_ASSOCIATIONS {
                return Err(InspectionError::new(
                    "browser_state_limit",
                    "too many browser associations",
                ));
            }
            let path = entry
                .map_err(|_| {
                    InspectionError::new(
                        "browser_state_unavailable",
                        "cannot inspect browser association",
                    )
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                let key = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| {
                        InspectionError::new(
                            "browser_state_corrupt",
                            "Invalid browser receipt filename",
                        )
                    })?;
                if let Some(value) = self.load(key)? {
                    values.push(value);
                }
            }
        }
        Ok(values)
    }
}

#[derive(Clone)]
struct ResolvedTarget {
    endpoint_identity: String,
    endpoint_path: String,
    session_id: String,
    space_id: String,
    space_label: String,
}

struct DaemonReceipt {
    socket_path: PathBuf,
    hash: String,
}

fn daemon_receipt(receipt: &BrowserReceipt) -> Result<DaemonReceipt, InspectionError> {
    let workspace = Path::new(&receipt.working_directory);
    let daemon_dir = if let Some(path) =
        env::var_os("PLAYWRIGHT_DAEMON_SESSION_DIR").filter(|path| !path.is_empty())
    {
        PathBuf::from(path)
    } else {
        #[cfg(target_os = "macos")]
        let cache = env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"));
        #[cfg(not(target_os = "macos"))]
        let cache = env::var_os("XDG_CACHE_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")));
        let cache = cache.ok_or_else(|| {
            InspectionError::new(
                "browser_daemon_unavailable",
                "cache directory is unavailable",
            )
        })?;
        cache.join("ms-playwright/daemon")
    };
    let hash = format!("{:x}", Sha1::digest(path_string(workspace)?.as_bytes()));
    let path = daemon_dir
        .join(&hash[..16])
        .join(format!("{}.session", receipt.playwright_session));
    if fs::symlink_metadata(&path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return Err(InspectionError::new(
            "browser_daemon_closed",
            "Playwright daemon receipt is absent",
        ));
    }
    let bytes = read_regular(&path, MAX_RECEIPT_BYTES)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        InspectionError::new(
            "browser_receipt_corrupt",
            "Playwright daemon receipt is invalid",
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        InspectionError::new(
            "browser_receipt_corrupt",
            "Playwright daemon receipt is not an object",
        )
    })?;
    let string = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                InspectionError::new(
                    "browser_receipt_corrupt",
                    format!("Playwright daemon receipt lacks {key}"),
                )
            })
    };
    if string("workspaceDir")? != receipt.working_directory {
        return Err(InspectionError::new(
            "browser_receipt_mismatch",
            "Playwright daemon workspace does not match association",
        ));
    }
    let socket_path = PathBuf::from(string("socketPath")?);
    let user_data = object
        .get("browser")
        .and_then(Value::as_object)
        .and_then(|browser| browser.get("userDataDir"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_receipt_corrupt",
                "Playwright daemon receipt lacks profile",
            )
        })?;
    if user_data != receipt.profile_path {
        return Err(InspectionError::new(
            "browser_receipt_mismatch",
            "Playwright daemon profile does not match association",
        ));
    }
    let executable = object
        .get("browser")
        .and_then(Value::as_object)
        .and_then(|browser| browser.get("launchOptions"))
        .and_then(Value::as_object)
        .and_then(|options| options.get("executablePath"))
        .and_then(Value::as_str);
    if receipt_config_executable(receipt)?
        .is_some_and(|expected| Some(expected.as_str()) != executable)
    {
        return Err(InspectionError::new(
            "browser_receipt_mismatch",
            "Playwright daemon executable does not match association config",
        ));
    }
    Ok(DaemonReceipt {
        socket_path,
        hash: format!("{:x}", Sha256::digest(&bytes)),
    })
}

fn receipt_config_executable(receipt: &BrowserReceipt) -> Result<Option<String>, InspectionError> {
    let config: Value = serde_json::from_slice(&read_regular(
        Path::new(&receipt.config_path),
        MAX_RECEIPT_BYTES,
    )?)
    .map_err(|_| {
        InspectionError::new(
            "browser_config_corrupt",
            "browser launch configuration is invalid",
        )
    })?;
    match config.pointer("/browser/launchOptions/executablePath") {
        None => Ok(None),
        Some(Value::String(path)) => Ok(Some(path.clone())),
        Some(_) => Err(InspectionError::new(
            "browser_config_corrupt",
            "browser executable override is not a path",
        )),
    }
}

fn launch_configuration(
    configuration: &BrowserConfiguration,
) -> Result<Value, InspectionError> {
    let mut config = serde_json::json!({"browser": {"launchOptions": {}}});
    if let Some(path) = &configuration.chromium_executable {
        config["browser"]["launchOptions"]["executablePath"] = Value::String(path_string(
            &resolve_executable(path, "Chromium executable")?,
        )?);
    }
    // Chromium allocates the port and writes it to the association profile.
    // This avoids a global fixed-port collision and never exposes CDP beyond
    // the local owner helper.
    config["browser"]["launchOptions"]["args"] = serde_json::json!([
        "--remote-debugging-address=127.0.0.1",
        "--remote-debugging-port=0",
        "--force-dark-mode"
    ]);
    Ok(config)
}

fn receipt_config_has_inline_debugging(receipt: &BrowserReceipt) -> Result<bool, InspectionError> {
    let config: Value = serde_json::from_slice(&read_regular(
        Path::new(&receipt.config_path),
        MAX_RECEIPT_BYTES,
    )?)
    .map_err(|_| {
        InspectionError::new(
            "browser_config_corrupt",
            "browser launch configuration is invalid",
        )
    })?;
    let Some(args) = config
        .pointer("/browser/launchOptions/args")
        .and_then(Value::as_array)
    else {
        return Ok(false);
    };
    Ok(args.iter().any(|argument| argument.as_str() == Some("--remote-debugging-address=127.0.0.1"))
        && args.iter().any(|argument| argument.as_str() == Some("--remote-debugging-port=0")))
}
#[derive(Debug)]
struct CdpBinding {
    endpoint: String,
    browser_identity: String,
}

async fn wait_for_cdp_binding(profile: &Path) -> Result<CdpBinding, InspectionError> {
    let deadline = tokio::time::Instant::now() + SOCKET_TIMEOUT;
    loop {
        match cdp_binding(profile).await {
            Ok(binding) => return Ok(binding),
            Err(error) if error.code == "browser_cdp_unavailable" => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

async fn cdp_binding(profile: &Path) -> Result<CdpBinding, InspectionError> {
    let port_file = profile.join("DevToolsActivePort");
    let bytes = read_regular(&port_file, 512).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium did not publish a bounded DevToolsActivePort record",
        )
    })?;
    let record = std::str::from_utf8(&bytes).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium DevToolsActivePort record is not UTF-8",
        )
    })?;
    let mut lines = record.lines();
    let port = lines
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium DevToolsActivePort does not contain a valid port",
            )
        })?;
    let browser_path = lines
        .next()
        .filter(|path| path.starts_with("/devtools/browser/") && path.len() <= 512)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium DevToolsActivePort does not contain a browser identity",
            )
        })?
        .to_owned();
    if lines.next().is_some() {
        return Err(InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium DevToolsActivePort record has unexpected fields",
        ));
    }
    let endpoint = format!("http://127.0.0.1:{port}");
    let mut stream = tokio::time::timeout(SOCKET_TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
        .await
        .map_err(|_| InspectionError::new("browser_cdp_unavailable", "Chromium CDP timed out"))?
        .map_err(|_| {
            InspectionError::new("browser_cdp_unavailable", "Chromium CDP is not reachable")
        })?;
    let request = format!(
        "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    tokio::time::timeout(SOCKET_TIMEOUT, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| {
            InspectionError::new("browser_cdp_unavailable", "Chromium CDP write timed out")
        })?
        .map_err(|_| {
            InspectionError::new("browser_cdp_unavailable", "Chromium CDP write failed")
        })?;
    let mut response = Vec::with_capacity(2048);
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let count = tokio::time::timeout(SOCKET_TIMEOUT, stream.read(&mut buffer))
            .await
            .map_err(|_| {
                InspectionError::new("browser_cdp_unavailable", "Chromium CDP read timed out")
            })?
            .map_err(|_| {
                InspectionError::new("browser_cdp_unavailable", "Chromium CDP read failed")
            })?;
        if count == 0 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP returned truncated HTTP",
            ));
        }
        if response.len() + count > 64 * 1024 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP version response exceeds bounds",
            ));
        }
        response.extend_from_slice(&buffer[..count]);
        if let Some(position) = response.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
    };
    let headers = std::str::from_utf8(&response[..header_end]).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP returned invalid HTTP headers",
        )
    })?;
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length:")
                .or_else(|| line.strip_prefix("content-length:"))
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|length| *length <= 64 * 1024)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP response lacks a bounded Content-Length",
            )
        })?;
    let body_start = header_end + 4;
    while response.len() < body_start + content_length {
        let count = tokio::time::timeout(SOCKET_TIMEOUT, stream.read(&mut buffer))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP body read timed out",
                )
            })?
            .map_err(|_| {
                InspectionError::new("browser_cdp_unavailable", "Chromium CDP body read failed")
            })?;
        if count == 0 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP returned truncated body",
            ));
        }
        if response.len() + count > body_start + content_length {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP body exceeds Content-Length",
            ));
        }
        response.extend_from_slice(&buffer[..count]);
    }
    let body = &response[body_start..body_start + content_length];
    let value: Value = serde_json::from_slice(body).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP returned invalid JSON",
        )
    })?;
    let websocket = value
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP version response lacks browser websocket identity",
            )
        })?;
    let websocket = url::Url::parse(websocket).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP websocket URL is invalid",
        )
    })?;
    if websocket.scheme() != "ws"
        || websocket.host_str() != Some("127.0.0.1")
        || websocket.port() != Some(port)
        || websocket.path() != browser_path
    {
        return Err(InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP websocket identity is not the loopback profile binding",
        ));
    }
    Ok(CdpBinding {
        endpoint,
        browser_identity: browser_path,
    })
}

async fn cdp_page_target(
    endpoint: &str,
    previous: Option<&str>,
) -> Result<String, InspectionError> {
    let url = url::Url::parse(endpoint).map_err(|_| {
        InspectionError::new("browser_cdp_unavailable", "Inline CDP endpoint is invalid")
    })?;
    if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") {
        return Err(InspectionError::new(
            "browser_cdp_unavailable",
            "Inline CDP endpoint is not loopback",
        ));
    }
    let port = url.port().ok_or_else(|| {
        InspectionError::new("browser_cdp_unavailable", "Inline CDP endpoint has no port")
    })?;
    let mut stream =
        tokio::time::timeout(SOCKET_TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP target listing timed out",
                )
            })?
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP target listing is unreachable",
                )
            })?;
    let request =
        format!("GET /json/list HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    tokio::time::timeout(SOCKET_TIMEOUT, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target-list write timed out",
            )
        })?
        .map_err(|_| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target-list write failed",
            )
        })?;

    let mut response = Vec::with_capacity(4096);
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let count = tokio::time::timeout(SOCKET_TIMEOUT, stream.read(&mut buffer))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP target-list read timed out",
                )
            })?
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP target-list read failed",
                )
            })?;
        if count == 0 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP returned truncated target-list HTTP",
            ));
        }
        if response.len() + count > 64 * 1024 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target-list response exceeds bounds",
            ));
        }
        response.extend_from_slice(&buffer[..count]);
        if let Some(position) = response.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
    };
    let headers = std::str::from_utf8(&response[..header_end]).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP target-list returned invalid HTTP headers",
        )
    })?;
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then_some(value.trim())
        })
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|length| *length <= 256 * 1024)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target-list response lacks a bounded Content-Length",
            )
        })?;
    let body_start = header_end + 4;
    while response.len() < body_start + content_length {
        let count = tokio::time::timeout(SOCKET_TIMEOUT, stream.read(&mut buffer))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP target-list body read timed out",
                )
            })?
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP target-list body read failed",
                )
            })?;
        if count == 0 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP returned truncated target-list body",
            ));
        }
        if response.len() + count > body_start + content_length {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target-list body exceeds Content-Length",
            ));
        }
        response.extend_from_slice(&buffer[..count]);
    }
    let body = &response[body_start..body_start + content_length];
    let targets: Vec<Value> = serde_json::from_slice(body).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP target list is invalid",
        )
    })?;
    let pages: Vec<String> = targets
        .into_iter()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
        .filter_map(|target| {
            target
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    let selected: Vec<String> = match previous {
        Some(previous) => pages.into_iter().filter(|target| target != previous).collect(),
        None => pages,
    };
    if selected.len() != 1 {
        return Err(InspectionError::new(
            "browser_target_unresolved",
            "CDP could not identify one stable page target for this browser operation",
        ));
    }
    Ok(selected.into_iter().next().expect("one target"))
}

fn locate_playwright_core(cli: &Path) -> Option<PathBuf> {
    let mut directory = cli.parent()?;
    for _ in 0..8 {
        let candidate = if directory
            .file_name()
            .is_some_and(|name| name == "node_modules")
        {
            directory.join("playwright-core")
        } else {
            directory.join("node_modules/playwright-core")
        };
        if candidate.join("package.json").is_file() {
            return candidate.canonicalize().ok();
        }
        directory = directory.parent()?;
    }
    None
}

fn compatible_playwright_cli_version(version: &str) -> bool {
    let mut parts = version.split('.');
    let Some(major) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return false;
    };
    let Some(minor) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return false;
    };
    let Some(patch) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return false;
    };
    major == 0 && minor == 1 && patch >= 5 && parts.next().is_none()
}

fn resolve_regular_file(path: &Path, label: &str) -> Result<PathBuf, InspectionError> {
    let resolved = path.canonicalize().map_err(|_| {
        InspectionError::new(
            "browser_tool_missing",
            format!("cannot resolve {label}: {}", path.display()),
        )
    })?;
    if !fs::metadata(&resolved).is_ok_and(|metadata| metadata.is_file()) {
        return Err(InspectionError::new(
            "browser_tool_missing",
            format!("{label} is not a regular file: {}", resolved.display()),
        ));
    }
    Ok(resolved)
}

fn resolve_playwright_core(path: &Path) -> Result<PathBuf, InspectionError> {
    let resolved = path.canonicalize().map_err(|_| {
        InspectionError::new(
            "browser_tool_missing",
            format!("cannot resolve Playwright-core: {}", path.display()),
        )
    })?;
    if !resolved.join("package.json").is_file() {
        return Err(InspectionError::new(
            "browser_tool_missing",
            "Configured Playwright-core path does not contain package.json",
        ));
    }
    Ok(resolved)
}
fn resolve_executable(path: &Path, label: &str) -> Result<PathBuf, InspectionError> {
    let candidate = if path.components().count() == 1 {
        env::var_os("PATH")
            .and_then(|paths| {
                env::split_paths(&paths)
                    .map(|directory| directory.join(path))
                    .find(|candidate| is_executable(candidate))
            })
            .ok_or_else(|| {
                InspectionError::new(
                    "browser_tool_missing",
                    format!("{label} is not installed; configure its absolute path"),
                )
            })?
    } else {
        path.to_owned()
    };
    if !is_executable(&candidate) {
        return Err(InspectionError::new(
            "browser_tool_missing",
            format!("{label} is not an executable file: {}", candidate.display()),
        ));
    }
    candidate.canonicalize().map_err(|_| {
        InspectionError::new(
            "browser_tool_missing",
            format!("cannot resolve {label}: {}", candidate.display()),
        )
    })
}

fn association_key(endpoint_identity: &str, session_id: &str, space_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(endpoint_identity.as_bytes());
    hasher.update([0]);
    hasher.update(session_id.as_bytes());
    hasher.update([0]);
    hasher.update(space_id.as_bytes());
    format!("{:x}", hasher.finalize())[..24].to_owned()
}

fn may_launch_after_inspection_failure(error: &InspectionError) -> bool {
    error.code == "browser_daemon_closed"
}

fn cli_error_detail(text: &str) -> Option<String> {
    let mut lines = text.lines();
    lines
        .position(|line| line == "### Error")
        .and_then(|_| lines.map(str::trim).find(|line| !line.is_empty()))
        .map(|line| {
            line.strip_prefix("Error: ")
                .unwrap_or(line)
                .chars()
                .take(512)
                .collect()
        })
}

fn cli_reports_browser_closed(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("target page, context or browser has been closed")
        || text.contains("browser context was closed")
        || text.contains("browser has been closed")
        || (text.contains("browser '") && text.contains(" is not open"))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

fn validate_id(value: &str, label: &str) -> Result<(), InspectionError> {
    if value.is_empty() || value.len() > 256 || value.contains(['/', '\\', '\0']) {
        Err(InspectionError::new(
            "invalid_browser_target",
            format!("invalid {label} ID"),
        ))
    } else {
        Ok(())
    }
}
fn validate_url(value: &str) -> Result<(), InspectionError> {
    let parsed = url::Url::parse(value)
        .map_err(|_| InspectionError::new("invalid_browser_url", "browser URL must be absolute"))?;
    if !matches!(parsed.scheme(), "http" | "https" | "about") {
        return Err(InspectionError::new(
            "invalid_browser_url",
            "browser URL has an unsupported or unsafe scheme",
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(InspectionError::new(
            "invalid_browser_url",
            "browser URL must not contain userinfo",
        ));
    }
    Ok(())
}
fn tab_index(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let index = line.trim().strip_prefix("- ")?.split_once(": (current)")?.0;
        (!index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| index.to_owned())
    })
}

fn validated_opened_tab(value: Option<&str>) -> Result<Option<String>, InspectionError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty()
        || value.len() > MAX_OPENED_TAB_BYTES
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || value.parse::<u64>().is_err()
    {
        return Err(InspectionError::new(
            "browser_receipt_corrupt",
            "Browser receipt opened tab index is invalid or exceeds its bound",
        ));
    }
    Ok(Some(value.to_owned()))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\\"'\\\"'"))
}
fn path_string(path: &Path) -> Result<String, InspectionError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| InspectionError::new("invalid_browser_path", "browser path is not UTF-8"))
}
fn prepare_root(path: &Path) -> Result<(), InspectionError> {
    let (_, directory) = crate::project_store::prepare_root(path, "browser")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let directory = directory
            .open(".")
            .map_err(|_| {
                InspectionError::new(
                    "browser_state_unavailable",
                    "Cannot open browser state directory",
                )
            })?
            .into_std();
        if directory
            .metadata()
            .map_err(|_| {
                InspectionError::new(
                    "browser_state_unavailable",
                    "Cannot inspect browser state owner",
                )
            })?
            .uid()
            != current_uid()
        {
            return Err(InspectionError::new(
                "unsafe_path",
                "Browser state belongs to another user",
            ));
        }
        directory
            .set_permissions(fs::Permissions::from_mode(0o700))
            .map_err(|_| {
                InspectionError::new(
                    "browser_state_unavailable",
                    "Cannot secure browser state directory",
                )
            })?;
    }
    Ok(())
}
fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), InspectionError> {
    let parent = path
        .parent()
        .ok_or_else(|| InspectionError::new("unsafe_path", "Browser state has no parent"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| InspectionError::new("unsafe_path", "Invalid browser state filename"))?;
    let directory = crate::project_store::open_dir_nofollow_absolute(parent)
        .map_err(|_| InspectionError::new("unsafe_path", "Browser state directory is unsafe"))?;
    crate::project_store::atomic_write_json(&directory, name, value)
        .map_err(|_| InspectionError::new("browser_state_write", "Cannot publish browser state"))?;
    directory
        .open(name)
        .and_then(|file| file.sync_all())
        .and_then(|_| directory.open(".").and_then(|file| file.sync_all()))
        .map_err(|_| InspectionError::new("browser_state_write", "Cannot sync browser state"))?;
    Ok(())
}
fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, InspectionError> {
    use cap_fs_ext::{OpenOptionsFollowExt, OpenOptionsSyncExt};
    let parent = path
        .parent()
        .ok_or_else(|| InspectionError::new("unsafe_path", "Browser state has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| InspectionError::new("unsafe_path", "Invalid browser state filename"))?;
    let directory = crate::project_store::open_dir_nofollow_absolute(parent)
        .map_err(|_| InspectionError::new("unsafe_path", "Browser state directory is unsafe"))?;
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let file = directory.open_with(name, &options).map_err(|_| {
        InspectionError::new("browser_state_read", "Cannot open browser state record")
    })?;
    let metadata = file.metadata().map_err(|_| {
        InspectionError::new("browser_state_read", "Cannot inspect browser state record")
    })?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(InspectionError::new(
            "unsafe_path",
            "Browser state record is not a bounded regular file",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|_| {
        InspectionError::new("browser_state_read", "cannot read browser state record")
    })?;
    if bytes.len() as u64 > limit {
        return Err(InspectionError::new(
            "browser_state_limit",
            "browser state record is too large",
        ));
    }
    Ok(bytes)
}
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, InspectionError> {
    if fs::symlink_metadata(path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) {
        return Ok(None);
    }
    serde_json::from_slice(&read_regular(path, MAX_RECEIPT_BYTES)?)
        .map(Some)
        .map_err(|_| {
            InspectionError::new("browser_state_corrupt", "browser association is invalid")
        })
}
#[cfg(unix)]
fn current_uid() -> u32 {
    nix::unistd::Uid::current().as_raw()
}
#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}
#[cfg(target_os = "linux")]
fn process_start_identity(pid: i32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    stat.get(close + 2..)?
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}
#[cfg(target_os = "macos")]
fn process_start_identity(pid: i32) -> Option<u64> {
    use libproc::{bsd_info::BSDInfo, proc_pid::pidinfo};

    let info = pidinfo::<BSDInfo>(pid, 0).ok()?;

    if info.pbi_start_tvusec >= 1_000_000 {
        return None;
    }
    info.pbi_start_tvsec
        .checked_shl(20)
        .and_then(|value| value.checked_add(info.pbi_start_tvusec))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_start_identity(_pid: i32) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::association_key;
    #[test]
    fn playwright_cli_accepts_later_bugfix_releases_only() {
        assert!(super::compatible_playwright_cli_version("0.1.5"));
        assert!(super::compatible_playwright_cli_version("0.1.17"));
        assert!(!super::compatible_playwright_cli_version("0.1.4"));
        assert!(!super::compatible_playwright_cli_version("0.2.0"));
        assert!(!super::compatible_playwright_cli_version("1.1.5"));
        assert!(!super::compatible_playwright_cli_version("0.1"));
        assert!(!super::compatible_playwright_cli_version("0.1.17.1"));
        assert!(!super::compatible_playwright_cli_version("0.1.17-alpha"));
    }

    #[test]
    fn association_key_changes_with_endpoint_process_generation() {
        let first = association_key(
            "unix-socket:/run/herdr.sock:pid=41:uid=1000:gid=1000:start=101",
            "daily",
            "w1",
        );
        let restarted = association_key(
            "unix-socket:/run/herdr.sock:pid=41:uid=1000:gid=1000:start=102",
            "daily",
            "w1",
        );
        assert_ne!(first, restarted);
    }

    #[test]
    fn opened_tab_address_selects_current_tab_not_first_tab() {
        let output = "### Result\n- 0: [](about:blank)\n- 1: [Checkout](http://localhost/checkout)\n- 2: (current) [Checkout](http://localhost/checkout)\n";
        assert_eq!(super::tab_index(output).as_deref(), Some("2"));
    }

    #[test]
    fn inspection_uncertainty_never_authorizes_a_replacement_launch() {
        for code in [
            "browser_daemon_unavailable",
            "browser_receipt_replaced",
            "browser_unowned_daemon",
            "unsafe_path",
        ] {
            assert!(!super::may_launch_after_inspection_failure(
                &crate::InspectionError::new(code, "verification failed")
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn browser_state_refuses_symlinked_parents_without_creating_targets() {
        let root =
            std::env::temp_dir().join(format!("cockpit-browser-symlink-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(super::prepare_root(&link.join("browser")).is_err());
        assert!(!outside.join("browser").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
