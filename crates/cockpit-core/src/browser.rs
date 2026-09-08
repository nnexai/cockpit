use async_trait::async_trait;
use cockpit_protocol::browser::{
    BrowserAction, BrowserAssociation, BrowserConnectionState, BrowserRequest, BrowserResponse,
    BrowserTarget,
};
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
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::InspectionError;
use crate::config::BrowserConfiguration;
use crate::process::run_bounded_command;

mod delivery;
mod extension;
pub use extension::{ANNOTATION_EXTENSION_ID, ExtensionStatus};

const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
const MAX_ASSOCIATIONS: usize = 1024;
const CLI_OUTPUT_LIMIT: usize = 64 * 1024;
const CLI_TIMEOUT: Duration = Duration::from_secs(15);
const REQUIRED_PLAYWRIGHT_CLI_VERSION: &str = "0.1.5";
const SOCKET_TIMEOUT: Duration = Duration::from_secs(2);

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
    feedback_endpoint: Arc<OnceLock<String>>,
    paste_adapter: Option<Arc<dyn crate::paste_adapter::CommentPasteAdapter>>,
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
    intent: ReceiptIntent,

    state: ReceiptState,
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
            "extensions",
            "pairings",
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
            feedback_endpoint: Arc::new(OnceLock::new()),
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
            BrowserAction::Show => self.show(&mut receipt).await,
        }
    }

    /// Reconcile durable associations without treating a transient Herdr failure
    /// or a restarted endpoint as permission to close or transfer an association.
    pub async fn reconcile(&self) -> Result<(), InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let receipts = self.load_all()?;
        for mut receipt in receipts {
            let snapshot = match self.adapter.browser_snapshot(&receipt.session_id).await {
                Ok(snapshot) => snapshot,
                Err(_) => continue,
            };
            if snapshot.endpoint_identity != receipt.endpoint_identity
                || snapshot.endpoint_path != receipt.endpoint_path
            {
                receipt.state = ReceiptState::Disconnected;
                self.store(&receipt)?;
                continue;
            }
            if !snapshot
                .snapshot
                .spaces
                .iter()
                .any(|space| space.id == receipt.space_id)
            {
                let _ = self.close(&mut receipt).await?;
            } else {
                receipt.space_label = snapshot
                    .snapshot
                    .spaces
                    .iter()
                    .find(|space| space.id == receipt.space_id)
                    .map(|space| space.label.clone())
                    .unwrap_or_else(|| receipt.space_label.clone());
                let _ = self.status(&mut receipt).await?;
            }
            self.store(&receipt)?;
        }
        Ok(())
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
        atomic_write_json(&config_path, &launch_configuration(&self.configuration)?)?;
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
            state: ReceiptState::Closed,
            opened_tab: None,
            incarnation: None,
            intent: ReceiptIntent::None,
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
                self.store(receipt)?;
                self.ensure_annotation_extension(receipt).await?;
                if let Some(url) = url {
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
                            receipt.intent = ReceiptIntent::None;
                        }
                        Err(error) => {
                            receipt.state = ReceiptState::OutcomeUnknown;
                            self.store(receipt)?;
                            return Err(error);
                        }
                    }
                    self.store(receipt)?;
                }
                Ok(self.response(receipt, BrowserConnectionState::Open, "browser is open"))
            }
            Err(error) if !may_launch_after_inspection_failure(&error) => Err(error),
            Err(_) => {
                self.ensure_cli_version().await?;
                self.prepare_annotation_extension(receipt)?;
                receipt.state = ReceiptState::PendingOpen;
                receipt.intent = if url.is_some() {
                    ReceiptIntent::LaunchNavigation
                } else {
                    ReceiptIntent::Launch
                };
                receipt.owner_id = self.owner_id.clone();
                receipt.incarnation = None;
                receipt.opened_tab = None;
                self.store(receipt)?;
                let mut args = vec![format!("-s={}", receipt.playwright_session), "open".into()];
                if let Some(url) = url {
                    args.push(url.into());
                }
                args.extend([
                    "--headed".into(),
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
                            self.store(receipt)?;
                            self.ensure_annotation_extension(receipt).await?;
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
                self.store(receipt)?;
                Ok(self.response(receipt, BrowserConnectionState::Open, "browser is open"))
            }
            Err(error) if may_launch_after_inspection_failure(&error) => {
                receipt.state = ReceiptState::Closed;
                receipt.intent = ReceiptIntent::None;
                receipt.incarnation = None;
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

    async fn show(&self, receipt: &mut BrowserReceipt) -> Result<BrowserResponse, InspectionError> {
        let status = self.status(receipt).await?;
        let incarnation = match self.inspect_live(receipt).await {
            Ok(value) => value,
            Err(_) => return Ok(status),
        };
        // `show` opens the CLI dashboard. Bring the active page forward instead.
        self.run_cli(
            receipt,
            &[
                format!("-s={}", receipt.playwright_session),
                "run-code".into(),
                "async page => { await page.bringToFront(); }".into(),
            ],
        )
        .await?;
        receipt.owner_id = self.owner_id.clone();
        receipt.incarnation = Some(incarnation);
        self.store(receipt)?;
        Ok(self.response(
            receipt,
            status.connection,
            if status.connection == BrowserConnectionState::OutcomeUnknown {
                "browser window requested; the previous URL action outcome remains unknown"
            } else {
                "browser window requested"
            },
        ))
    }

    async fn run_cli(
        &self,
        receipt: &BrowserReceipt,
        args: &[String],
    ) -> Result<String, InspectionError> {
        let executable = resolve_executable(&self.configuration.playwright_cli, "Playwright CLI")?;
        let mut command = tokio::process::Command::new(executable);
        command.current_dir(&receipt.working_directory).args(args);
        let output = run_bounded_command(
            command,
            CLI_OUTPUT_LIMIT,
            CLI_OUTPUT_LIMIT,
            CLI_TIMEOUT,
            "Playwright CLI",
        )
        .await?;
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
            return Err(InspectionError::new(
                "browser_action_failed",
                "Playwright reported a browser operation failure; inspect the browser before another URL action",
            ));
        }
        Ok(text)
    }

    async fn ensure_cli_version(&self) -> Result<(), InspectionError> {
        let executable = resolve_executable(&self.configuration.playwright_cli, "Playwright CLI")?;
        let mut command = tokio::process::Command::new(executable);
        command.arg("--version");
        let output =
            run_bounded_command(command, 1024, 1024, CLI_TIMEOUT, "Playwright CLI").await?;
        if !output.status.success()
            || String::from_utf8_lossy(&output.stdout).trim() != REQUIRED_PLAYWRIGHT_CLI_VERSION
        {
            return Err(InspectionError::new(
                "browser_cli_incompatible",
                format!("Playwright CLI {REQUIRED_PLAYWRIGHT_CLI_VERSION} is required"),
            ));
        }
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

fn launch_configuration(configuration: &BrowserConfiguration) -> Result<Value, InspectionError> {
    let mut config = serde_json::json!({"browser": {"launchOptions": {}}});
    if let Some(path) = &configuration.chromium_executable {
        config["browser"]["launchOptions"]["executablePath"] = Value::String(path_string(
            &resolve_executable(path, "Chromium executable")?,
        )?);
    }
    Ok(config)
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
    if matches!(parsed.scheme(), "http" | "https" | "file" | "about") {
        Ok(())
    } else {
        Err(InspectionError::new(
            "invalid_browser_url",
            "browser URL has an unsupported scheme",
        ))
    }
}
fn tab_index(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let index = line.trim().strip_prefix("- ")?.split_once(": (current)")?.0;
        (!index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| index.to_owned())
    })
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
