use cockpit_protocol::browser::{
    BrowserAction, BrowserAssociation, BrowserCleanupState, BrowserConnectionState, BrowserRequest,
    BrowserResponse, BrowserTarget, BrowserWorkScope,
};
use cockpit_protocol::browser_view::BrowserViewOpenRequest;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::{sync::Mutex, task::JoinSet};
use uuid::Uuid;

use super::process::{
    locate_playwright_core, may_launch_after_inspection_failure, resolve_executable,
    resolve_playwright_core, resolve_regular_file, shell_quote, tab_index,
};
use super::receipts::{
    BrowserReceipt, MAX_ASSOCIATIONS, ReceiptIntent, ReceiptState, association_key, prepare_root,
};
use super::{
    BrowserHerdrAdapter, BrowserRuntimeAttachment, BrowserService, ResolvedTarget,
    ResolvedWorkScope, STATE_DIRS, cleanup, delivery, validate_url,
};
use crate::{InspectionError, config::BrowserConfiguration};

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
        for name in STATE_DIRS {
            prepare_root(&root.join(name))?;
        }
        let feedback = crate::browser_feedback::BrowserFeedbackStore::new(
            state_root,
            crate::browser_feedback::BrowserFeedbackOptions {
                retention_seconds: configuration.feedback_retention_seconds,
                max_store_bytes: configuration.feedback_max_store_bytes,
            },
        )?;
        delivery::note_process_start();
        Ok(Self {
            configuration,
            root: Arc::new(root),
            owner_id: Uuid::new_v4().to_string(),
            adapter,
            operation_lock: Arc::new(Mutex::new(())),
            shutting_down: Arc::new(AtomicBool::new(false)),
            feedback: Arc::new(feedback),
            paste_adapter: None,
            cleanup_failures: Arc::new(parking_lot::Mutex::new(Vec::new())),
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
        let _operation = self.operation_lock.lock().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(InspectionError::new(
                "browser_runtime_stopping",
                "browser runtime is shutting down",
            ));
        }
        if let BrowserAction::Open { url: Some(url) }
        | BrowserAction::OpenFresh { url: Some(url) } = &request.action
        {
            validate_url(url)?;
        }
        let allow_absent = matches!(
            &request.action,
            BrowserAction::Close | BrowserAction::Status | BrowserAction::Cleanup
        );
        let target = self
            .resolve_target_inner(&request.target, allow_absent)
            .await?;
        if !target.tab_present && !allow_absent {
            return Err(InspectionError::new(
                "tab_not_visible",
                "browser target tab has no terminal",
            ));
        }
        let key = association_key(
            &target.endpoint_identity,
            &target.session_id,
            &target.tab_id,
        );
        let mut receipt = self.load(&key)?;
        if let Some(existing) = &receipt
            && (existing.endpoint_path != target.endpoint_path
                || existing.endpoint_identity != target.endpoint_identity)
        {
            return Err(InspectionError::new(
                "stale_endpoint",
                "browser receipt belongs to another Herdr endpoint",
            ));
        }
        if matches!(&request.action, BrowserAction::OpenFresh { .. })
            && let Some(existing) = &mut receipt
        {
            let response = self.close(existing).await?;
            if response.cleanup != BrowserCleanupState::Done {
                return Ok(response);
            }
            receipt = None;
        }
        if receipt.is_none()
            && matches!(
                &request.action,
                BrowserAction::Open { .. } | BrowserAction::OpenFresh { .. }
            )
        {
            self.load_all()?;
            let directory = crate::project_store::open_dir_nofollow_absolute(
                &self.root.join("tab-associations"),
            )
            .map_err(|error| {
                InspectionError::new("browser_state_unavailable", error.to_string())
            })?;
            let count = directory
                .entries()
                .map_err(|error| {
                    InspectionError::new("browser_state_unavailable", error.to_string())
                })?
                .count();
            if count >= MAX_ASSOCIATIONS {
                return Err(InspectionError::new(
                    "browser_state_limit",
                    "too many browser associations",
                ));
            }
            receipt = Some(self.load_or_create(&target)?);
        }
        let Some(mut receipt) = receipt else {
            return Ok(BrowserResponse {
                association: None,
                connection: BrowserConnectionState::Absent,
                message: "no browser association exists for this tab".into(),
                cleanup: if matches!(
                    request.action,
                    BrowserAction::Close | BrowserAction::Cleanup
                ) {
                    BrowserCleanupState::Done
                } else {
                    BrowserCleanupState::None
                },
                cleanup_reason: None,
            });
        };
        receipt.space_id = target.space_id;
        receipt.space_label = target.space_label;
        receipt.tab_label = target.tab_label;
        self.store(&receipt)?;
        match request.action {
            BrowserAction::OpenFresh { url } => {
                let url = url.unwrap_or_else(|| self.configuration.default_url.clone());
                self.open(&mut receipt, Some(&url)).await
            }
            BrowserAction::Open { url } => {
                if matches!(
                    receipt.state,
                    ReceiptState::CleanupPending | ReceiptState::CleanupFailed
                ) {
                    return Ok(self.response(
                        &receipt,
                        BrowserConnectionState::Closed,
                        "Retry browser cleanup before opening",
                    ));
                }
                self.open(&mut receipt, url.as_deref()).await
            }
            BrowserAction::Status => self.status(&mut receipt).await,
            BrowserAction::Close | BrowserAction::Cleanup => self.close(&mut receipt).await,
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
            &target.tab_id,
        );
        let receipt = self.load(&key)?.ok_or_else(|| {
            InspectionError::new(
                "browser_view_association_absent",
                "Open the browser association before attaching an inline browser view",
            )
        })?;
        cleanup::verify_artifacts(&self.root, &receipt)?;
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
        let playwright_core = match configured_core {
            Some(path) => Some(path),
            None => locate_playwright_core(&cli)
                .map(|path| resolve_playwright_core(&path))
                .transpose()?,
        }
        .ok_or_else(|| {
            InspectionError::new(
                "browser_core_missing",
                "Paired Playwright-core is not installed beside the selected CLI; \
                 set COCKPIT_PLAYWRIGHT_CORE or [browser].playwright_core to its package directory",
            )
        })?;
        let helper_module = self
            .configuration
            .browser_helper
            .as_ref()
            .map(|path| resolve_regular_file(path, "Browser helper"))
            .transpose()?;
        let node_executable = match self.configuration.node_executable.as_ref() {
            Some(path) => resolve_executable(path, "Node runtime")?,
            None => resolve_executable(Path::new("node"), "Node runtime")?,
        };
        Ok(BrowserRuntimeAttachment {
            association_key: receipt.association_key,
            browser_incarnation: incarnation,
            session_id: receipt.session_id,
            space_id: receipt.space_id,
            tab_id: receipt.tab_id,
            endpoint_identity: receipt.endpoint_identity,
            endpoint_path: receipt.endpoint_path,
            profile_path: PathBuf::from(receipt.profile_path),
            cdp_endpoint,
            target_id,
            playwright_core: Some(playwright_core),
            node_executable: Some(node_executable),
            helper_module,
        })
    }

    /// Check the pinned Herdr socket peer without asking it for another snapshot.
    /// Call at command admission and periodically while frame transport is active.
    pub async fn verify_browser_runtime_endpoint(
        &self,
        attachment: &BrowserRuntimeAttachment,
    ) -> Result<(), InspectionError> {
        let (identity, path) = self
            .adapter
            .browser_endpoint_identity(&attachment.session_id)
            .await
            .map_err(|_| {
                InspectionError::new(
                    "stale_browser_endpoint",
                    "Herdr endpoint identity is unavailable; the inline view was revoked",
                )
            })?;
        if path != attachment.endpoint_path
            || identity != attachment.endpoint_identity
            || identity.contains(":start=unavailable")
        {
            return Err(InspectionError::new(
                "stale_browser_endpoint",
                "Herdr endpoint changed; the inline view was revoked",
            ));
        }
        Ok(())
    }

    /// Only fresh membership authorizes retirement; transient failures preserve sessions.
    pub async fn reconcile(&self) -> Result<(), InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let mut first_error = None;
        for mut receipt in self.load_all()? {
            let source = match self.adapter.browser_snapshot(&receipt.session_id).await {
                Ok(source) if source.snapshot.session_id == receipt.session_id => source,
                _ => continue,
            };
            let changed = source.endpoint_identity != receipt.endpoint_identity
                || source.endpoint_path != receipt.endpoint_path;
            let tab = source
                .snapshot
                .tabs
                .iter()
                .find(|tab| tab.id == receipt.tab_id);
            let empty = !source
                .snapshot
                .panes
                .iter()
                .any(|pane| pane.tab_id == receipt.tab_id);
            let result = if changed || tab.is_none() || empty {
                self.close(&mut receipt).await.map(|_| ())
            } else {
                let tab = tab.expect("present tab");
                receipt.space_id = tab.space_id.clone();
                receipt.tab_label = tab.label.clone();
                if let Some(space) = source
                    .snapshot
                    .spaces
                    .iter()
                    .find(|space| space.id == tab.space_id)
                {
                    receipt.space_label = space.label.clone();
                }
                self.status(&mut receipt).await.map(|_| ())
            };
            if let Err(error) = result {
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
        let receipts = self
            .load_all()?
            .into_iter()
            .filter(|receipt| receipt.owner_id == self.owner_id)
            .collect::<Vec<_>>();
        let mut pending = receipts.into_iter().enumerate();
        let mut receipt_errors = std::iter::repeat_with(|| None)
            .take(pending.len())
            .collect::<Vec<Option<InspectionError>>>();
        let mut closing = JoinSet::new();
        let mut task_error = None;

        loop {
            while closing.len() < 4 {
                let Some((index, mut receipt)) = pending.next() else {
                    break;
                };
                let service = self.clone();
                closing.spawn(async move {
                    let mut first_error = None;
                    if let Err(error) = service.close(&mut receipt).await {
                        first_error = Some(error);
                    }
                    (index, first_error)
                });
            }
            let Some(result) = closing.join_next().await else {
                break;
            };
            match result {
                Ok((index, error)) => receipt_errors[index] = error,
                Err(_) => {
                    task_error.get_or_insert_with(|| {
                        InspectionError::new(
                            "browser_shutdown_task_failed",
                            "Browser association shutdown task failed",
                        )
                    });
                }
            }
        }
        receipt_errors
            .into_iter()
            .flatten()
            .next()
            .or(task_error)
            .map_or(Ok(()), Err)
    }

    pub(super) async fn resolve_target(
        &self,
        target: &BrowserTarget,
    ) -> Result<ResolvedTarget, InspectionError> {
        let resolved = self.resolve_target_inner(target, false).await?;
        if !resolved.tab_present {
            return Err(InspectionError::new(
                "tab_not_visible",
                "browser target tab has no terminal",
            ));
        }
        Ok(resolved)
    }

    async fn resolve_target_inner(
        &self,
        target: &BrowserTarget,
        allow_absent: bool,
    ) -> Result<ResolvedTarget, InspectionError> {
        if !(target.tab_id.is_some() ^ target.pane_id.is_some()) {
            return Err(InspectionError::new(
                "invalid_browser_target",
                "browser target must name exactly one tab or pane",
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
        if target
            .endpoint_path
            .as_ref()
            .is_some_and(|path| path != &source.endpoint_path)
        {
            return Err(InspectionError::new(
                "stale_endpoint",
                "browser target endpoint does not match the fresh Herdr endpoint",
            ));
        }
        let tab_id = match (&target.tab_id, &target.pane_id) {
            (Some(tab), None) => tab.clone(),
            (None, Some(pane)) => source
                .snapshot
                .panes
                .iter()
                .find(|p| p.id == *pane)
                .map(|p| p.tab_id.clone())
                .ok_or_else(|| {
                    InspectionError::new("pane_not_visible", "browser target pane is absent")
                })?,
            _ => unreachable!(),
        };
        validate_id(&tab_id, "tab")?;
        if let Some(tab) = source.snapshot.tabs.iter().find(|tab| tab.id == tab_id) {
            let space = source
                .snapshot
                .spaces
                .iter()
                .find(|space| space.id == tab.space_id)
                .ok_or_else(|| {
                    InspectionError::new("space_not_visible", "browser target Space is absent")
                })?;
            let present = source
                .snapshot
                .panes
                .iter()
                .any(|pane| pane.tab_id == tab_id);
            return Ok(ResolvedTarget {
                endpoint_identity: source.endpoint_identity,
                endpoint_path: source.endpoint_path,
                session_id: target.session_id.clone(),
                space_id: tab.space_id.clone(),
                space_label: space.label.clone(),
                tab_id,
                tab_label: tab.label.clone(),
                tab_present: present,
            });
        }
        if allow_absent {
            let key = association_key(&source.endpoint_identity, &target.session_id, &tab_id);
            if let Some(receipt) = self.load(&key)?
                && receipt.endpoint_path == source.endpoint_path
            {
                return Ok(ResolvedTarget {
                    endpoint_identity: source.endpoint_identity,
                    endpoint_path: source.endpoint_path,
                    session_id: target.session_id.clone(),
                    space_id: receipt.space_id,
                    space_label: receipt.space_label,
                    tab_id,
                    tab_label: receipt.tab_label,
                    tab_present: false,
                });
            }
            if self
                .load_all()?
                .iter()
                .any(|receipt| receipt.session_id == target.session_id && receipt.tab_id == tab_id)
            {
                return Err(InspectionError::new(
                    "stale_endpoint",
                    "absent tab receipt belongs to another endpoint",
                ));
            }
        }
        Err(InspectionError::new(
            "tab_not_visible",
            "browser target tab is absent",
        ))
    }

    pub(crate) async fn resolve_work_scope(
        &self,
        scope: &BrowserWorkScope,
    ) -> Result<ResolvedWorkScope, InspectionError> {
        match scope {
            BrowserWorkScope::Tab { target } => {
                let tab = self.resolve_target(target).await?;
                Ok(ResolvedWorkScope {
                    association_key: association_key(
                        &tab.endpoint_identity,
                        &tab.session_id,
                        &tab.tab_id,
                    ),
                    tab,
                })
            }
        }
    }

    async fn open(
        &self,
        receipt: &mut BrowserReceipt,
        url: Option<&str>,
    ) -> Result<BrowserResponse, InspectionError> {
        cleanup::verify_artifacts(&self.root, receipt)?;
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
                            self.resolve_inline_target(receipt, previous_target.as_deref())
                                .await?;
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
                args.push(url.unwrap_or(&self.configuration.default_url).into());
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

    pub(super) async fn status(
        &self,
        receipt: &mut BrowserReceipt,
    ) -> Result<BrowserResponse, InspectionError> {
        if matches!(
            receipt.state,
            ReceiptState::CleanupPending | ReceiptState::CleanupFailed
        ) {
            let connection =
                if receipt.intent == ReceiptIntent::Close || receipt.incarnation.is_some() {
                    BrowserConnectionState::OutcomeUnknown
                } else {
                    BrowserConnectionState::Closed
                };
            return Ok(self.response(receipt, connection, "browser cleanup is incomplete"));
        }
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
            return self.finish_cleanup(receipt).await;
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
                Ok(self.response(receipt, BrowserConnectionState::Closed, "browser is closed"))
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

    pub(super) async fn close(
        &self,
        receipt: &mut BrowserReceipt,
    ) -> Result<BrowserResponse, InspectionError> {
        receipt.opened_tab = None;
        match self.inspect_live(receipt).await {
            Err(error) if may_launch_after_inspection_failure(&error) => {
                return self.finish_cleanup(receipt).await;
            }
            Err(error) => {
                receipt.intent = ReceiptIntent::Close;
                self.record_cleanup_failure(receipt, &error.message, Vec::new())?;
                return Ok(self.response(
                    receipt,
                    BrowserConnectionState::OutcomeUnknown,
                    &error.message,
                ));
            }
            Ok(incarnation) => receipt.incarnation = Some(incarnation),
        }
        receipt.state = ReceiptState::Closing;
        receipt.intent = ReceiptIntent::Close;
        self.store(receipt)?;
        let result = self
            .run_cli(
                receipt,
                &[format!("-s={}", receipt.playwright_session), "close".into()],
            )
            .await;
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if self
                .inspect_live(receipt)
                .await
                .as_ref()
                .is_err_and(may_launch_after_inspection_failure)
            {
                return self.finish_cleanup(receipt).await;
            }
        }
        let reason = result
            .err()
            .map(|error| error.message)
            .unwrap_or_else(|| "browser stop was not confirmed after five checks".into());
        self.record_cleanup_failure(receipt, &reason, Vec::new())?;
        Ok(self.response(receipt, BrowserConnectionState::OutcomeUnknown, &reason))
    }

    pub(super) fn response(
        &self,
        receipt: &BrowserReceipt,
        connection: BrowserConnectionState,
        message: &str,
    ) -> BrowserResponse {
        BrowserResponse {
            association: Some(self.association(receipt, connection)),
            connection,
            message: message.into(),
            cleanup: match receipt.state {
                ReceiptState::CleanupPending => BrowserCleanupState::Pending,
                ReceiptState::CleanupFailed => BrowserCleanupState::Failed,
                _ => BrowserCleanupState::None,
            },
            cleanup_reason: receipt.cleanup_reason.clone(),
        }
    }

    pub(super) fn association(
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
            tab_id: receipt.tab_id.clone(),
            tab_label: receipt.tab_label.clone(),
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
