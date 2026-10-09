use super::*;
use crate::browser::{BrowserRuntimeAttachment, BrowserService};
use cockpit_protocol::{
    browser::{
        BrowserConnectionState, BrowserFeedbackAckRequest, BrowserFeedbackLookup, BrowserResponse,
        BrowserTarget, BrowserWorkScope,
    },
    browser_feedback::BrowserFeedbackAck,
    browser_view::{
        BrowserDraftRecoveryAction, BrowserDraftRecoveryRequest, BrowserViewCaptureCommand,
        BrowserViewCommandOutcome, BrowserViewDocumentCommandContext, BrowserViewFrameDescriptor,
    },
};

impl BrowserService {
    /// Handles only owner-persisted annotation commands. The host forwards all
    /// browser input and helper commands through their separate transport.
    pub async fn browser_annotation_command(
        &self,
        target: &BrowserTarget,
        attachment: &BrowserRuntimeAttachment,
        context: BrowserViewDocumentCommandContext,
        draft_id: Option<&str>,
        expected_revision: Option<u64>,
        command: BrowserViewDraftCommand,
    ) -> Result<BrowserViewCommandOutcome, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let receipt = self.annotation_receipt(target, attachment).await?;
        let store = self.draft_store()?;
        let association_key = receipt.association_key.clone();
        match command {
            BrowserViewDraftCommand::List => Ok(BrowserViewCommandOutcome::DraftInventory {
                inventory: store.list(&association_key)?,
            }),
            BrowserViewDraftCommand::RetryPending => Ok(BrowserViewCommandOutcome::Capture {
                capture: store.retry_pending(&association_key)?,
            }),
            BrowserViewDraftCommand::DiscardPending => {
                store.discard_pending(&association_key)?;
                Ok(BrowserViewCommandOutcome::Capture {
                    capture: BrowserViewCaptureOutcome::Absent,
                })
            }
            BrowserViewDraftCommand::SaveCapture {
                submission,
                annotation_ids,
                provenance,
            } => {
                let draft_id = required_draft_id(draft_id)?;
                let expected_revision = required_revision(expected_revision)?;
                Ok(BrowserViewCommandOutcome::Capture {
                    capture: store.save_capture(
                        draft_id,
                        expected_revision,
                        annotation_ids,
                        submission,
                        provenance,
                    )?,
                })
            }
            command => {
                if context.target_id != attachment.target_id {
                    return Err(InspectionError::new(
                        "browser_draft_target",
                        "Draft command targets another browser tab",
                    ));
                }
                let identity = BrowserDraftIdentity {
                    association_key,
                    browser_incarnation: attachment.browser_incarnation.clone(),
                    target_id: context.target_id,
                    document_generation: context.document_generation,
                };
                match command {
                    BrowserViewDraftCommand::Open {
                        draft_id: requested,
                    } => Ok(BrowserViewCommandOutcome::Draft {
                        draft: store.open(&identity, requested)?,
                    }),
                    command => Ok(BrowserViewCommandOutcome::Draft {
                        draft: store.mutate(
                            &identity,
                            required_draft_id(draft_id)?,
                            required_revision(expected_revision)?,
                            command,
                        )?,
                    }),
                }
            }
        }
    }

    /// Records a capture transaction only after the helper has returned the
    /// descriptor for the pixels that will be composed. A later navigation
    /// cannot alter that stored identity.
    pub async fn browser_prepare_capture(
        &self,
        target: &BrowserTarget,
        attachment: &BrowserRuntimeAttachment,
        command: &BrowserViewCaptureCommand,
        capture_id: &str,
        descriptor: &BrowserViewFrameDescriptor,
        frame_id: String,
        frame_generation: u64,
    ) -> Result<(), InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let receipt = self.annotation_receipt(target, attachment).await?;
        if command.location.target_id != attachment.target_id
            || descriptor.target_id != attachment.target_id
            || command.location.document_generation != descriptor.document_generation
            || command.location.viewport_revision != descriptor.viewport_revision
            || command.location.presented_frame_sequence != descriptor.frame_sequence
            || command.location.lease_generation == 0
        {
            return Err(InspectionError::new(
                "browser_draft_capture",
                "Capture preparation no longer matches the presented frame",
            ));
        }
        let identity = BrowserDraftIdentity {
            association_key: receipt.association_key.clone(),
            browser_incarnation: attachment.browser_incarnation.clone(),
            target_id: attachment.target_id.clone(),
            document_generation: descriptor.document_generation,
        };
        let address = self.association(
            &receipt,
            cockpit_protocol::browser::BrowserConnectionState::Open,
        );
        let context = BrowserCaptureContext {
            association_key: address.association_key,
            session_id: address.session_id,
            space_id: address.space_id,
            space_label: address.space_label,
            playwright_session: address.playwright_session,
            working_directory: address.working_directory,
            invocation: address.invocation,
            browser_instance: attachment.browser_incarnation.clone(),
            inline_provenance: Some(BrowserInlineCaptureProvenance {
                target_id: descriptor.target_id.clone(),
                frame_id,
                document_generation: descriptor.document_generation,
                frame_generation,
                stream_epoch: descriptor.stream_epoch,
                frame_sequence: descriptor.frame_sequence,
                viewport_revision: descriptor.viewport_revision,
                pixel_captured_at_micros: descriptor.capture_timestamp_micros,
                capture_as_shown: command.capture_as_shown,
            }),
        };
        self.draft_store()?.prepare_capture(
            BrowserDraftCaptureContext { identity, context },
            &command.draft_id,
            command.draft_revision,
            command.annotation_ids.clone(),
            capture_id.to_owned(),
        )
    }
    /// Recovers drafts and frozen captures for a tab without launching or
    /// attaching a browser.
    pub async fn browser_draft_recovery(
        &self,
        request: BrowserDraftRecoveryRequest,
    ) -> Result<BrowserViewCommandOutcome, InspectionError> {
        request
            .validate()
            .map_err(|message| InspectionError::new("browser_draft_command", message))?;
        let _operation = self.operation_lock.lock().await;
        let resolved = self.resolve_work_scope(&request.scope).await?;
        let association_key = resolved.association_key;
        let store = self.draft_store()?;
        let outcome: Result<BrowserViewCommandOutcome, InspectionError> = match request.action {
            BrowserDraftRecoveryAction::List => Ok(BrowserViewCommandOutcome::DraftInventory {
                inventory: store.list(&association_key)?,
            }),
            BrowserDraftRecoveryAction::RetryPending => Ok(BrowserViewCommandOutcome::Capture {
                capture: store.retry_pending(&association_key)?,
            }),
            BrowserDraftRecoveryAction::DiscardPending => {
                store.discard_pending(&association_key)?;
                Ok(BrowserViewCommandOutcome::Capture {
                    capture: BrowserViewCaptureOutcome::Absent,
                })
            }
            BrowserDraftRecoveryAction::SetEditor {
                draft_id,
                expected_revision,
                editor,
            } => Ok(BrowserViewCommandOutcome::Draft {
                draft: store.set_editor_recovery(
                    &association_key,
                    &draft_id,
                    expected_revision,
                    editor,
                )?,
            }),
            BrowserDraftRecoveryAction::UpsertAnnotation {
                draft_id,
                expected_revision,
                annotation,
            } => Ok(BrowserViewCommandOutcome::Draft {
                draft: store.upsert_annotation_recovery(
                    &association_key,
                    &draft_id,
                    expected_revision,
                    annotation,
                )?,
            }),
            BrowserDraftRecoveryAction::RemoveAnnotation {
                draft_id,
                expected_revision,
                annotation_id,
            } => Ok(BrowserViewCommandOutcome::Draft {
                draft: store.remove_annotation_recovery(
                    &association_key,
                    &draft_id,
                    expected_revision,
                    annotation_id,
                )?,
            }),
            BrowserDraftRecoveryAction::DiscardDraft {
                draft_id,
                expected_revision,
            } => {
                store.discard_draft(&association_key, &draft_id, expected_revision)?;
                Ok(BrowserViewCommandOutcome::DraftInventory {
                    inventory: store.list(&association_key)?,
                })
            }
        };
        let outcome = outcome?;
        Ok(outcome)
    }

    /// Reads the tab's feedback and drafts without launching a browser.
    pub async fn feedback(
        &self,
        scope: &BrowserWorkScope,
    ) -> Result<BrowserFeedbackLookup, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let resolved = self.resolve_work_scope(scope).await?;
        let key = resolved.association_key;
        let browser = match self.load(&key)? {
            Some(mut receipt) => self.status(&mut receipt).await?,
            None => BrowserResponse {
                association: None,
                connection: BrowserConnectionState::Absent,
                message: "No browser association exists for this tab".into(),
                cleanup: cockpit_protocol::browser::BrowserCleanupState::None,
                cleanup_reason: None,
            },
        };
        let feedback = self.feedback.list(&key)?;
        let mut deliveries = self
            .feedback
            .list_delivery_statuses(&key, &feedback.captures)?;
        if self.settle_interrupted_deliveries(&deliveries)? {
            deliveries = self
                .feedback
                .list_delivery_statuses(&key, &feedback.captures)?;
        }
        Ok(BrowserFeedbackLookup {
            browser,
            feedback,
            deliveries,
            drafts: Some(self.draft_store()?.list(&key)?),
        })
    }

    pub async fn acknowledge_feedback(
        &self,
        request: BrowserFeedbackAckRequest,
    ) -> Result<BrowserFeedbackAck, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let resolved = self.resolve_work_scope(&request.scope).await?;
        let key = resolved.association_key;
        let ack = self.feedback.ack(&key, &request.ids)?;
        Ok(ack)
    }

    pub fn prune_feedback(&self) -> Result<(), InspectionError> {
        self.feedback.prune()
    }

    pub fn browser_state_directory(&self) -> &Path {
        self.root.as_ref()
    }

    pub(in crate::browser) fn draft_store(&self) -> Result<BrowserDraftStore, InspectionError> {
        let state_root = self.root.parent().ok_or_else(|| {
            InspectionError::new("browser_draft_root", "Browser state root has no parent")
        })?;
        BrowserDraftStore::new(state_root.to_path_buf(), Arc::clone(&self.feedback))
    }

    async fn annotation_receipt(
        &self,
        target: &BrowserTarget,
        attachment: &BrowserRuntimeAttachment,
    ) -> Result<super::super::BrowserReceipt, InspectionError> {
        let resolved = self.resolve_target(target).await?;
        let association_key = super::super::association_key(
            &resolved.endpoint_identity,
            &resolved.session_id,
            &resolved.tab_id,
        );
        if attachment.association_key != association_key {
            return Err(InspectionError::new(
                "browser_draft_association",
                "Browser view belongs to another tab association",
            ));
        }
        let receipt = self.load(&association_key)?.ok_or_else(|| {
            InspectionError::new(
                "browser_draft_association",
                "Browser association is unavailable",
            )
        })?;
        if receipt.incarnation.as_deref() != Some(attachment.browser_incarnation.as_str()) {
            return Err(InspectionError::new(
                "browser_draft_incarnation",
                "Browser view belongs to a previous browser incarnation",
            ));
        }
        Ok(receipt)
    }
}

fn required_draft_id(draft_id: Option<&str>) -> Result<&str, InspectionError> {
    draft_id.ok_or_else(|| InspectionError::new("browser_draft_command", "Draft ID is required"))
}

fn required_revision(revision: Option<u64>) -> Result<u64, InspectionError> {
    revision.ok_or_else(|| {
        InspectionError::new(
            "browser_draft_command",
            "Expected draft revision is required",
        )
    })
}
