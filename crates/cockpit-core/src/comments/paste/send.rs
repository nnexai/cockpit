use cockpit_protocol::comment_paste::{
    CommentPasteReceipt, CommentPasteSendRequest, CommentPasteState,
};
use cockpit_protocol::comments::{CommentAttachment, CommentBatch};

use crate::InspectionError;
use crate::project_store::timestamp;

use super::super::CommentEvidence;
use super::{
    ArchiveOutcome, CommentsService, PREVIEW_LIMIT_BYTES, format_batch, has_unarchived_sent_drafts,
    hash_payload, is_definitive_pre_dispatch, outcome_unknown, reconciliation_message, rejection,
    require_owner, same_target,
};

pub(super) enum BatchCheck {
    Ready(CommentBatch),
    Rejected(CommentPasteReceipt),
}

pub(super) enum PayloadCheck {
    Ready {
        framed: String,
        payload_hash: String,
    },
    Rejected(CommentPasteReceipt),
}

impl CommentsService {
    pub(super) async fn checked_paste_batch(
        &self,
        request: &CommentPasteSendRequest,
        attachment: &CommentAttachment,
        evidence: &CommentEvidence,
    ) -> Result<BatchCheck, InspectionError> {
        let mut has_unknown = false;
        for receipt in self
            .paste_store
            .all_for_batch(&request.batch.batch_id)
            .await?
        {
            if receipt.operation_id == request.operation_id
                || !same_target(&receipt.target, &request.target)
            {
                continue;
            }
            // The current target/batch lease is held, so a pending receipt here
            // cannot belong to a live same-target sender.
            let receipt = if receipt.state == CommentPasteState::Pending {
                self.paste_store
                    .load(&receipt.operation_id)
                    .await?
                    .unwrap_or(receipt)
            } else {
                receipt
            };
            if receipt.state == CommentPasteState::OutcomeUnknown {
                has_unknown = true;
                break;
            }
        }
        if has_unknown && !request.acknowledge_duplicate_risk {
            return Err(InspectionError::new(
                "comments_paste_duplicate_risk",
                "a prior paste outcome is unknown; inspect the terminal and explicitly acknowledge duplicate risk before retrying",
            ));
        }
        let batch = self
            .store
            .load(&request.batch.batch_id)
            .await?
            .ok_or_else(|| {
                InspectionError::new("comments_batch_not_found", "comment batch does not exist")
            })?;
        require_owner(&batch, attachment, evidence)?;
        if batch.generation != request.batch.expected_generation {
            return self
                .paste_store
                .save(rejection(
                    request,
                    "comment batch changed after preview; prepare it again before pasting",
                ))
                .await
                .map(BatchCheck::Rejected);
        }
        if self
            .paste_store
            .all_for_batch(&batch.batch_id)
            .await?
            .iter()
            .any(|receipt| {
                !receipt.user_confirmed
                    && receipt.state == CommentPasteState::Accepted
                    && has_unarchived_sent_drafts(&batch, receipt)
            })
        {
            return Err(InspectionError::new(
                "comments_paste_reconciliation_required",
                "a prior accepted paste still has unarchived drafts; reconcile it before another paste",
            ));
        }
        Ok(BatchCheck::Ready(batch))
    }

    pub(super) async fn prepare_paste_payload(
        &self,
        request: &CommentPasteSendRequest,
        batch: &mut CommentBatch,
        evidence: &CommentEvidence,
    ) -> Result<PayloadCheck, InspectionError> {
        // This is intentionally immediately before dispatch: never send a stale source as current.
        self.refresh_states(batch, evidence).await;
        let preview = format_batch(batch, request.retain_stale_excerpts);
        let payload_hash = hash_payload(&preview.payload);
        if !preview.exportable || preview.framed_bytes > PREVIEW_LIMIT_BYTES {
            return self
                .paste_store
                .save(rejection(
                    request,
                    preview
                        .reason
                        .unwrap_or_else(|| "paste payload exceeds its bound".to_owned()),
                ))
                .await
                .map(PayloadCheck::Rejected);
        }
        if payload_hash != request.expected_payload_hash {
            return self
                .paste_store
                .save(rejection(
                    request,
                    "the exact preview payload changed; prepare and review it again",
                ))
                .await
                .map(PayloadCheck::Rejected);
        }
        if preview.payload.contains('\u{1b}') || preview.payload.contains("\u{1b}[201~") {
            return self
                .paste_store
                .save(rejection(
                    request,
                    "paste payload contains an unsafe bracketed-paste terminator",
                ))
                .await
                .map(PayloadCheck::Rejected);
        }
        let framed = format!("\u{1b}[200~{}\u{1b}[201~", preview.payload);
        if framed.as_bytes().len() != preview.framed_bytes as usize {
            return Err(InspectionError::new(
                "comments_paste_framing",
                "paste framing byte accounting disagreed with preview",
            ));
        }
        Ok(PayloadCheck::Ready {
            framed,
            payload_hash,
        })
    }

    pub(super) async fn complete_paste(
        &self,
        request: &CommentPasteSendRequest,
        batch: &CommentBatch,
        pending: CommentPasteReceipt,
        outcome: Result<(), InspectionError>,
    ) -> Result<CommentPasteReceipt, InspectionError> {
        match outcome {
            Ok(()) => {
                let mut accepted = pending;
                accepted.state = CommentPasteState::Accepted;
                accepted.completed_at = Some(timestamp());
                accepted.message = Some(
                    "Herdr accepted one raw bracketed-paste write; no Enter was sent.".to_owned(),
                );
                let mut accepted = self.paste_store.save(accepted).await?;
                // The receipt is durable before archive CAS. Later edits survive;
                // only byte-for-byte matching sent drafts may be removed.
                match self.archive_accepted_drafts(batch, &accepted).await {
                    Ok(ArchiveOutcome::Archived) => {}
                    Ok(ArchiveOutcome::ReconciliationRequired) => {
                        accepted.message = Some(reconciliation_message(
                            "A newer draft with a sent identity was retained; reconcile this receipt before another paste.",
                        ));
                        accepted = self.paste_store.save(accepted).await?;
                    }
                    Ok(ArchiveOutcome::MissingFrozenSnapshot) => {
                        accepted.message = Some(reconciliation_message(
                            "The frozen draft snapshot is unavailable; no drafts were archived.",
                        ));
                        accepted = self.paste_store.save(accepted).await?;
                    }
                    Err(error) => {
                        accepted.message = Some(reconciliation_message(format!(
                            "The batch kept changing while Cockpit archived sent drafts: {}",
                            error.message,
                        )));
                        accepted = self.paste_store.save(accepted).await?;
                    }
                }
                Ok(accepted)
            }
            Err(error) if is_definitive_pre_dispatch(&error) => {
                self.paste_store
                    .save(rejection(
                        request,
                        format!(
                            "Herdr rejected the paste before dispatch: {}",
                            error.message
                        ),
                    ))
                    .await
            }
            Err(error) => {
                self.paste_store
                    .save(outcome_unknown(
                        pending,
                        format!("paste dispatch outcome is unknown: {}", error.message),
                    ))
                    .await
            }
        }
    }
}
