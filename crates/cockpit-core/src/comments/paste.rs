use std::path::Path;

use cockpit_protocol::comment_paste::{
    CommentPasteMarkPastedRequest, CommentPastePrepareRequest, CommentPastePrepareResponse,
    CommentPasteReceipt, CommentPasteSendRequest, CommentPasteState, CommentPasteTarget,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::InspectionError;
use crate::project_store::{ProjectStore, atomic_write_json, read_json_bounded, timestamp};

use super::format::{PREVIEW_LIMIT_BYTES, format_batch};
use super::{CommentsService, require_owner};

const MAX_TARGETS: usize = 64;
const MAX_ID_BYTES: usize = 128;
const MAX_RECEIPTS: usize = 64;
const MAX_RECEIPT_RECORDS: usize = 128;
const MAX_RECEIPT_SCAN: usize = 4096;
const MAX_STORED_RECEIPT_BYTES: u64 = (4 * 1024 * 1024) + (64 * 1024);
const LOCK_STRIPES: usize = 64;
const MAX_STATE_ENTRIES: usize = MAX_RECEIPT_SCAN + (LOCK_STRIPES * 3) + 1;
const RECONCILIATION_PREFIX: &str = "reconciliation required:";

#[derive(Debug, Clone)]
pub(super) struct PasteStore {
    state: ProjectStore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReceipt {
    receipt: CommentPasteReceipt,
    /// Frozen before raw dispatch so user resolution can remove only drafts
    /// that still exactly match the payload sent to Herdr.
    #[serde(default)]
    sent_drafts: Option<Vec<cockpit_protocol::comments::CommentDraft>>,
}

impl PasteStore {
    pub(super) fn new(root: &Path) -> Result<Self, InspectionError> {
        Ok(Self {
            state: ProjectStore::new(root.join("paste"))?,
        })
    }

    async fn lease(
        &self,
        target: &CommentPasteTarget,
        batch_id: &str,
    ) -> Result<PasteLease, InspectionError> {
        let state = self.state.clone();
        let target_name = target_lock_name(target);
        let batch_name = batch_lock_name(batch_id)?;
        tokio::task::spawn_blocking(move || {
            // Domains have distinct fixed stripe namespaces, so acquiring batch
            // then target cannot self-deadlock when both hashes choose index N.
            let batch = state.acquire_named_lock(&batch_name, "comments_paste_lock")?;
            let target = state.acquire_named_lock(&target_name, "comments_paste_lock")?;
            Ok(PasteLease {
                _batch: batch,
                _target: target,
            })
        })
        .await
        .map_err(|error| InspectionError::new("comments_paste_task", error.to_string()))?
    }

    async fn load(
        &self,
        operation_id: &str,
    ) -> Result<Option<CommentPasteReceipt>, InspectionError> {
        let operation_id = operation_id.to_owned();
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || load_receipt(&state, &operation_id, true))
            .await
            .map_err(|error| InspectionError::new("comments_paste_task", error.to_string()))?
    }

    async fn load_stored(
        &self,
        operation_id: &str,
    ) -> Result<Option<StoredReceipt>, InspectionError> {
        let operation_id = operation_id.to_owned();
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || load_stored_receipt(&state, &operation_id, true))
            .await
            .map_err(|error| InspectionError::new("comments_paste_task", error.to_string()))?
    }

    async fn inspect_stored(
        &self,
        operation_id: &str,
    ) -> Result<Option<StoredReceipt>, InspectionError> {
        let operation_id = operation_id.to_owned();
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || load_stored_receipt(&state, &operation_id, false))
            .await
            .map_err(|error| InspectionError::new("comments_paste_task", error.to_string()))?
    }

    async fn save(
        &self,
        receipt: CommentPasteReceipt,
    ) -> Result<CommentPasteReceipt, InspectionError> {
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || save_receipt(&state, receipt, None))
            .await
            .map_err(|error| InspectionError::new("comments_paste_task", error.to_string()))?
    }

    async fn save_frozen(
        &self,
        receipt: CommentPasteReceipt,
        sent_drafts: Vec<cockpit_protocol::comments::CommentDraft>,
    ) -> Result<CommentPasteReceipt, InspectionError> {
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || save_receipt(&state, receipt, Some(sent_drafts)))
            .await
            .map_err(|error| InspectionError::new("comments_paste_task", error.to_string()))?
    }

    async fn list_for_batch(
        &self,
        batch_id: &str,
    ) -> Result<Vec<CommentPasteReceipt>, InspectionError> {
        let batch_id = batch_id.to_owned();
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || list_receipts_for_batch(&state, &batch_id))
            .await
            .map_err(|error| InspectionError::new("comments_paste_task", error.to_string()))?
    }

    /// Internal safety checks must inspect the complete bounded receipt set;
    /// presentation is truncated only after these checks have run.
    async fn all_for_batch(
        &self,
        batch_id: &str,
    ) -> Result<Vec<CommentPasteReceipt>, InspectionError> {
        let batch_id = batch_id.to_owned();
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || all_receipts_for_batch(&state, &batch_id))
            .await
            .map_err(|error| InspectionError::new("comments_paste_task", error.to_string()))?
    }
}

struct PasteLease {
    _batch: crate::project_store::LockGuard,
    _target: crate::project_store::LockGuard,
}

fn valid_id(value: &str, kind: &str) -> Result<(), InspectionError> {
    if value.is_empty() || value.len() > MAX_ID_BYTES || value.contains(['/', '\\', '\0']) {
        return Err(InspectionError::new(
            "comments_paste_invalid_id",
            format!("paste {kind} is invalid"),
        ));
    }
    Ok(())
}

fn validate_target(target: &CommentPasteTarget) -> Result<(), InspectionError> {
    for value in [
        &target.endpoint_identity,
        &target.session_id,
        &target.workspace_id,
        &target.tab_id,
        &target.pane_id,
        &target.terminal_id,
        &target.agent_label,
        &target.agent_fingerprint,
    ] {
        if value.is_empty() || value.len() > 4096 || value.contains('\0') {
            return Err(InspectionError::new(
                "comments_paste_invalid_target",
                "paste target identity is invalid",
            ));
        }
    }
    if !target.agent_fingerprint.starts_with("sha256:") || target.agent_fingerprint.len() != 71 {
        return Err(InspectionError::new(
            "comments_paste_invalid_target",
            "paste target fingerprint is invalid",
        ));
    }
    Ok(())
}

fn receipt_name(operation_id: &str) -> Result<String, InspectionError> {
    valid_id(operation_id, "operation identity")?;
    if Uuid::parse_str(operation_id).is_err() {
        return Err(InspectionError::new(
            "comments_paste_invalid_id",
            "paste operation identity must be a UUID",
        ));
    }
    Ok(format!("receipt-{operation_id}.json"))
}

fn batch_lock_name(batch_id: &str) -> Result<String, InspectionError> {
    valid_id(batch_id, "batch identity")?;
    Ok(striped_lock_name("batch", batch_id.as_bytes()))
}

fn target_lock_name(target: &CommentPasteTarget) -> String {
    let mut digest = Sha256::new();
    for part in [
        target.endpoint_identity.as_bytes(),
        target.session_id.as_bytes(),
        target.workspace_id.as_bytes(),
        target.tab_id.as_bytes(),
        target.pane_id.as_bytes(),
        target.terminal_id.as_bytes(),
        target.agent_fingerprint.as_bytes(),
    ] {
        digest.update(part);
        digest.update([0]);
    }
    striped_lock_name("target", digest.finalize().as_slice())
}

fn receipt_lock_name(operation_id: &str) -> String {
    striped_lock_name("receipt", operation_id.as_bytes())
}

fn striped_lock_name(domain: &str, key: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain.as_bytes());
    digest.update([0]);
    digest.update(key);
    let stripe = usize::from(digest.finalize()[0]) % LOCK_STRIPES;
    format!(".paste-{domain}-{stripe:02}.lock")
}

fn load_receipt(
    state: &ProjectStore,
    operation_id: &str,
    recover_pending: bool,
) -> Result<Option<CommentPasteReceipt>, InspectionError> {
    Ok(load_stored_receipt(state, operation_id, recover_pending)?.map(|record| record.receipt))
}

fn load_stored_receipt(
    state: &ProjectStore,
    operation_id: &str,
    recover_pending: bool,
) -> Result<Option<StoredReceipt>, InspectionError> {
    let name = receipt_name(operation_id)?;
    match state.state_dir().symlink_metadata(&name) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(InspectionError::new(
                "comments_paste_read",
                error.to_string(),
            ));
        }
    }
    let mut record = match read_json_bounded::<StoredReceipt>(
        state.state_dir(),
        &name,
        MAX_STORED_RECEIPT_BYTES,
    ) {
        Ok(record) => record,
        Err(error) => return Err(error),
    };
    if recover_pending && record.receipt.state == CommentPasteState::Pending {
        record.receipt.state = CommentPasteState::OutcomeUnknown;
        record.receipt.completed_at = Some(timestamp());
        record.receipt.message = Some(
            "Cockpit restarted while delivery was pending; inspect the terminal before retrying."
                .to_owned(),
        );
        write_stored_receipt(state, &name, &record)?;
    }
    Ok(Some(record))
}

fn write_stored_receipt(
    state: &ProjectStore,
    name: &str,
    record: &StoredReceipt,
) -> Result<(), InspectionError> {
    atomic_write_json(state.state_dir(), name, record)
        .map_err(|error| InspectionError::new("comments_paste_write", error.to_string()))
}

fn save_receipt(
    state: &ProjectStore,
    receipt: CommentPasteReceipt,
    frozen_drafts: Option<Vec<cockpit_protocol::comments::CommentDraft>>,
) -> Result<CommentPasteReceipt, InspectionError> {
    let name = receipt_name(&receipt.operation_id)?;
    let _lock = state.acquire_named_lock(
        &receipt_lock_name(&receipt.operation_id),
        "comments_paste_lock",
    )?;
    let _receipts_lock = state.acquire_named_lock(".paste-receipts.lock", "comments_paste_lock")?;
    if let Some(mut existing) = load_stored_receipt(state, &receipt.operation_id, false)? {
        let existing_receipt = &existing.receipt;
        if existing_receipt.request_id != receipt.request_id
            || existing_receipt.batch_id != receipt.batch_id
            || existing_receipt.payload_hash != receipt.payload_hash
            || !same_target(&existing_receipt.target, &receipt.target)
        {
            return Err(InspectionError::new(
                "comments_paste_operation_conflict",
                "paste operation identity was already used for another request",
            ));
        }
        let explicit_resolution = existing_receipt.state == CommentPasteState::OutcomeUnknown
            && receipt.state == CommentPasteState::Accepted
            && receipt.user_confirmed;
        if (existing_receipt.state == CommentPasteState::Accepted
            && receipt.state == CommentPasteState::Accepted)
            || explicit_resolution
        {
            existing.receipt = receipt.clone();
            if frozen_drafts.is_some() {
                existing.sent_drafts = frozen_drafts;
            }
            write_stored_receipt(state, &name, &existing)?;
            prune_completed_receipts(state)?;
            return Ok(receipt);
        }
        if existing_receipt.state != CommentPasteState::Pending {
            return Ok(existing.receipt);
        }
        existing.receipt = receipt.clone();
        if frozen_drafts.is_some() {
            existing.sent_drafts = frozen_drafts;
        }
        write_stored_receipt(state, &name, &existing)?;
        prune_completed_receipts(state)?;
        return Ok(receipt);
    }
    write_stored_receipt(
        state,
        &name,
        &StoredReceipt {
            receipt: receipt.clone(),
            sent_drafts: frozen_drafts,
        },
    )?;
    prune_completed_receipts(state)?;
    Ok(receipt)
}

fn list_receipts_for_batch(
    state: &ProjectStore,
    batch_id: &str,
) -> Result<Vec<CommentPasteReceipt>, InspectionError> {
    let mut receipts = all_receipts_for_batch(state, batch_id)?;
    // Actionable receipts must remain reachable even after newer completed history.
    receipts.sort_by_key(|receipt| {
        !(!receipt.user_confirmed
            && (matches!(
                receipt.state,
                CommentPasteState::Pending | CommentPasteState::OutcomeUnknown
            ) || receipt_requires_reconciliation(receipt)))
    });
    receipts.truncate(MAX_RECEIPTS);
    Ok(receipts)
}

fn all_receipts_for_batch(
    state: &ProjectStore,
    batch_id: &str,
) -> Result<Vec<CommentPasteReceipt>, InspectionError> {
    let _lock = state.acquire_named_lock(".paste-receipts.lock", "comments_paste_lock")?;
    let entries = state
        .state_dir()
        .entries()
        .map_err(|error| InspectionError::new("comments_paste_read", error.to_string()))?;
    let mut operation_ids = Vec::new();
    let mut seen = 0usize;
    let mut entries_seen = 0usize;
    for entry in entries {
        entries_seen = entries_seen.saturating_add(1);
        if entries_seen > MAX_STATE_ENTRIES {
            return Err(InspectionError::new(
                "comments_paste_bounded",
                "paste receipt directory exceeded its bounded entry limit",
            ));
        }
        let name = entry
            .as_ref()
            .ok()
            .and_then(|entry| entry.file_name().to_str().map(str::to_owned));
        let is_receipt = name
            .as_deref()
            .is_some_and(|name| name.starts_with("receipt-") && name.ends_with(".json"));
        if is_receipt {
            seen = seen.saturating_add(1);
        }
        if seen > MAX_RECEIPT_SCAN {
            return Err(InspectionError::new(
                "comments_paste_bounded",
                "paste receipt lookup exceeded its bounded entry limit",
            ));
        }
        let entry = entry
            .map_err(|error| InspectionError::new("comments_paste_read", error.to_string()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(operation_id) = name
            .strip_prefix("receipt-")
            .and_then(|value| value.strip_suffix(".json"))
        else {
            continue;
        };
        if Uuid::parse_str(operation_id).is_err() {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| InspectionError::new("comments_paste_read", error.to_string()))?;
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(InspectionError::new(
                "unsafe_path",
                "paste receipt is not a regular file",
            ));
        }
        operation_ids.push(operation_id.to_owned());
    }
    operation_ids.sort();
    let mut receipts = Vec::new();
    for operation_id in operation_ids {
        let Some(receipt) = load_receipt(state, &operation_id, false)? else {
            continue;
        };
        if receipt.batch_id == batch_id {
            receipts.push(receipt);
        }
    }
    receipts.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then(right.operation_id.cmp(&left.operation_id))
    });
    Ok(receipts)
}

/// Retain recent completed history while keeping unresolved delivery records
/// indefinitely. Lock inodes are deliberately never removed: deleting a path
/// while another process holds its flock would create a second lock identity.
fn prune_completed_receipts(state: &ProjectStore) -> Result<(), InspectionError> {
    let entries = state
        .state_dir()
        .entries()
        .map_err(|error| InspectionError::new("comments_paste_read", error.to_string()))?;
    let mut completed = Vec::new();
    let mut total = 0usize;
    let mut entries_seen = 0usize;
    for entry in entries {
        entries_seen = entries_seen.saturating_add(1);
        if entries_seen > MAX_STATE_ENTRIES {
            return Err(InspectionError::new(
                "comments_paste_bounded",
                "paste receipt directory exceeded its bounded entry limit",
            ));
        }
        let entry = entry
            .map_err(|error| InspectionError::new("comments_paste_read", error.to_string()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(operation_id) = name
            .strip_prefix("receipt-")
            .and_then(|value| value.strip_suffix(".json"))
        else {
            continue;
        };
        if Uuid::parse_str(operation_id).is_err() {
            continue;
        }
        total = total.saturating_add(1);
        let file_type = entry
            .file_type()
            .map_err(|error| InspectionError::new("comments_paste_read", error.to_string()))?;
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(InspectionError::new(
                "unsafe_path",
                "paste receipt is not a regular file",
            ));
        }
        let Some(receipt) = load_receipt(state, operation_id, false)? else {
            continue;
        };
        if !matches!(
            receipt.state,
            CommentPasteState::Pending | CommentPasteState::OutcomeUnknown
        ) && !receipt_requires_reconciliation(&receipt)
        {
            completed.push((name.to_owned(), receipt));
        }
    }
    let removable = total.saturating_sub(MAX_RECEIPT_RECORDS);
    if removable == 0 {
        return Ok(());
    }
    completed.sort_by(|(_, left), (_, right)| {
        left.created_at
            .cmp(&right.created_at)
            .then(left.operation_id.cmp(&right.operation_id))
    });
    for (name, _) in completed.into_iter().take(removable) {
        state
            .state_dir()
            .remove_file(name)
            .map_err(|error| InspectionError::new("comments_paste_write", error.to_string()))?;
    }
    Ok(())
}

fn hash_payload(payload: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(payload.as_bytes());
    format!("sha256:{:x}", digest.finalize())
}

/// The label is presentation metadata, not target-incarnation evidence.
fn same_target(left: &CommentPasteTarget, right: &CommentPasteTarget) -> bool {
    left.endpoint_identity == right.endpoint_identity
        && left.session_id == right.session_id
        && left.workspace_id == right.workspace_id
        && left.tab_id == right.tab_id
        && left.pane_id == right.pane_id
        && left.terminal_id == right.terminal_id
        && left.agent_fingerprint == right.agent_fingerprint
}

fn rejection(request: &CommentPasteSendRequest, message: impl Into<String>) -> CommentPasteReceipt {
    let now = timestamp();
    CommentPasteReceipt {
        operation_id: request.operation_id.clone(),
        request_id: request.request_id.clone(),
        batch_id: request.batch.batch_id.clone(),
        batch_generation: request.batch.expected_generation,
        payload_hash: request.expected_payload_hash.clone(),
        target: request.target.clone(),
        state: CommentPasteState::Rejected,
        sent_draft_ids: Vec::new(),
        created_at: now.clone(),
        completed_at: Some(now),
        message: Some(message.into()),
        user_confirmed: false,
    }
}

fn outcome_unknown(
    mut receipt: CommentPasteReceipt,
    message: impl Into<String>,
) -> CommentPasteReceipt {
    receipt.state = CommentPasteState::OutcomeUnknown;
    receipt.completed_at = Some(timestamp());
    receipt.message = Some(message.into());
    receipt
}

fn reconciliation_message(detail: impl std::fmt::Display) -> String {
    format!(
        "{RECONCILIATION_PREFIX} Herdr accepted one raw bracketed-paste write; no Enter was sent. {detail}"
    )
}

fn receipt_requires_reconciliation(receipt: &CommentPasteReceipt) -> bool {
    !receipt.user_confirmed
        && receipt.state == CommentPasteState::Accepted
        && receipt
            .message
            .as_deref()
            .is_some_and(|message| message.starts_with(RECONCILIATION_PREFIX))
}

fn has_unarchived_sent_drafts(
    batch: &cockpit_protocol::comments::CommentBatch,
    receipt: &CommentPasteReceipt,
) -> bool {
    receipt
        .sent_draft_ids
        .iter()
        .any(|id| batch.drafts.iter().any(|draft| &draft.draft_id == id))
}

/// Remove only drafts that have not changed since the accepted payload was
/// frozen. A concurrent edit with the same draft ID stays durable and forces
/// an explicit reconciliation before it can be exported again.
fn archive_matching_drafts(
    current: &mut cockpit_protocol::comments::CommentBatch,
    sent_batch: &cockpit_protocol::comments::CommentBatch,
    receipt: &CommentPasteReceipt,
) -> ArchiveOutcome {
    current.drafts.retain(|draft| {
        let Some(sent) = sent_batch
            .drafts
            .iter()
            .find(|sent| sent.draft_id == draft.draft_id)
        else {
            return true;
        };
        !receipt
            .sent_draft_ids
            .iter()
            .any(|id| id == &draft.draft_id)
            || !same_sent_draft(draft, sent)
    });
    if has_unarchived_sent_drafts(current, receipt) {
        ArchiveOutcome::ReconciliationRequired
    } else {
        ArchiveOutcome::Archived
    }
}

/// Errors from the raw adapter after it has been called are ambiguous unless
/// the adapter can prove it refused before dispatch. In particular a malformed
/// or negative acknowledgement may arrive after Herdr queued the bytes.
fn is_definitive_pre_dispatch(error: &InspectionError) -> bool {
    matches!(
        error.code.as_str(),
        "request_not_dispatched" | "comments_paste_input_bounded" | "comments_paste_framing"
    )
}

fn same_anchor(
    left: &cockpit_protocol::comments::CommentAnchor,
    right: &cockpit_protocol::comments::CommentAnchor,
) -> bool {
    use cockpit_protocol::comments::CommentAnchor;
    match (left, right) {
        (CommentAnchor::WholeFile, CommentAnchor::WholeFile) => true,
        (
            CommentAnchor::Lines {
                start_line: left_start,
                end_line: left_end,
                selected_lines: left_lines,
            },
            CommentAnchor::Lines {
                start_line: right_start,
                end_line: right_end,
                selected_lines: right_lines,
            },
        ) => left_start == right_start && left_end == right_end && left_lines == right_lines,
        _ => false,
    }
}

fn same_sent_draft(
    left: &cockpit_protocol::comments::CommentDraft,
    right: &cockpit_protocol::comments::CommentDraft,
) -> bool {
    left.draft_id == right.draft_id
        && left.file_ref.review == right.file_ref.review
        && left.file_ref.root_id == right.file_ref.root_id
        && left.file_ref.path == right.file_ref.path
        && left.file_ref.absolute_path == right.file_ref.absolute_path
        && left.file_ref.revision == right.file_ref.revision
        && left.file_ref.content_hash == right.file_ref.content_hash
        && same_anchor(&left.anchor, &right.anchor)
        && left.comment_text == right.comment_text
        && left.updated_at == right.updated_at
}

impl CommentsService {
    /// Return a fresh, host-derived target list and the exact preview payload digest.
    pub async fn paste_prepare(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &CommentPastePrepareRequest,
    ) -> Result<CommentPastePrepareResponse, InspectionError> {
        let adapter = self.paste_adapter.as_ref().ok_or_else(|| {
            InspectionError::new(
                "comments_paste_unavailable",
                "acknowledged Herdr paste is not configured",
            )
        })?;
        let (attachment, evidence) = self
            .attachment(session_id, pane_id, &request.batch.scope)
            .await?;
        let mut batch = self
            .store
            .load(&request.batch.batch_id)
            .await?
            .ok_or_else(|| {
                InspectionError::new("comments_batch_not_found", "comment batch does not exist")
            })?;
        require_owner(&batch, &attachment, &evidence)?;
        if batch.generation != request.batch.expected_generation {
            return Err(InspectionError::new(
                "stale_generation",
                "comment batch generation is no longer current",
            ));
        }
        self.refresh_states(&mut batch, &evidence).await;
        let preview = format_batch(&batch, request.retain_stale_excerpts);
        if preview.payload_bytes > super::format::MAX_PREVIEW_PAYLOAD_BYTES {
            return Err(InspectionError::new(
                "comments_preview_bounded",
                "comment preview exceeds the 4 MiB response payload limit",
            ));
        }
        let payload_hash = hash_payload(&preview.payload);
        let receipts = self.recover_paste_receipts(&batch.batch_id).await?;
        let needs_reconciliation = self
            .paste_store
            .all_for_batch(&batch.batch_id)
            .await?
            .iter()
            .any(|receipt| {
                !receipt.user_confirmed
                    && receipt.state == CommentPasteState::Accepted
                    && has_unarchived_sent_drafts(&batch, receipt)
            });
        let targets = adapter
            .comment_paste_targets(session_id)
            .await?
            .into_iter()
            .filter(|target| {
                target.session_id == session_id
                    && target.workspace_id == attachment.location.workspace_id
                    && target.tab_id == attachment.location.tab_id
            })
            .take(MAX_TARGETS)
            .collect::<Vec<_>>();
        let reason = if needs_reconciliation {
            Some("A prior accepted paste could not archive every matching draft; reconcile that receipt before another paste.".to_owned())
        } else if !preview.exportable {
            preview.reason
        } else if targets.is_empty() {
            Some("No current agent target is available in this Context tab.".to_owned())
        } else {
            None
        };
        Ok(CommentPastePrepareResponse {
            batch_id: batch.batch_id,
            generation: batch.generation,
            payload_hash,
            payload_bytes: preview.payload_bytes,
            framed_bytes: preview.framed_bytes,
            limit_bytes: PREVIEW_LIMIT_BYTES,
            targets,
            receipts,
            paste_available: reason.is_none(),
            reason,
        })
    }

    /// Execute exactly one user-requested, acknowledged raw bracketed paste.
    pub async fn paste_send(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &CommentPasteSendRequest,
    ) -> Result<CommentPasteReceipt, InspectionError> {
        valid_id(&request.request_id, "request identity")?;
        // Validate the operation identity before any target lease or adapter
        // call. Otherwise an invalid ID could reach Herdr and only fail while
        // persisting the receipt, leaving a dispatched paste unreconcilable.
        receipt_name(&request.operation_id)?;
        validate_target(&request.target)?;
        let adapter = self
            .paste_adapter
            .as_ref()
            .ok_or_else(|| {
                InspectionError::new(
                    "comments_paste_unavailable",
                    "acknowledged Herdr paste is not configured",
                )
            })?
            .clone();
        if request.target.session_id != session_id {
            return Err(InspectionError::new(
                "comments_paste_target_mismatch",
                "paste target is from another session",
            ));
        }
        let _lease = self
            .paste_store
            .lease(&request.target, &request.batch.batch_id)
            .await?;
        if let Some(receipt) = self.paste_store.load(&request.operation_id).await? {
            return Ok(receipt);
        }
        let (attachment, evidence) = self
            .attachment(session_id, pane_id, &request.batch.scope)
            .await?;
        if request.target.workspace_id != attachment.location.workspace_id
            || request.target.tab_id != attachment.location.tab_id
        {
            return self
                .paste_store
                .save(rejection(
                    request,
                    "paste target is no longer in this Context tab",
                ))
                .await;
        }
        let targets = adapter.comment_paste_targets(session_id).await?;
        if !targets
            .iter()
            .any(|target| same_target(target, &request.target))
        {
            return self
                .paste_store
                .save(rejection(
                    request,
                    "paste target no longer has a verified agent identity",
                ))
                .await;
        }
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
        let mut batch = self
            .store
            .load(&request.batch.batch_id)
            .await?
            .ok_or_else(|| {
                InspectionError::new("comments_batch_not_found", "comment batch does not exist")
            })?;
        require_owner(&batch, &attachment, &evidence)?;
        if batch.generation != request.batch.expected_generation {
            return self
                .paste_store
                .save(rejection(
                    request,
                    "comment batch changed after preview; prepare it again before pasting",
                ))
                .await;
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
        // This is intentionally immediately before dispatch: never send a stale source as current.
        self.refresh_states(&mut batch, &evidence).await;
        let preview = format_batch(&batch, request.retain_stale_excerpts);
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
                .await;
        }
        if payload_hash != request.expected_payload_hash {
            return self
                .paste_store
                .save(rejection(
                    request,
                    "the exact preview payload changed; prepare and review it again",
                ))
                .await;
        }
        if preview.payload.contains('\u{1b}') || preview.payload.contains("\u{1b}[201~") {
            return self
                .paste_store
                .save(rejection(
                    request,
                    "paste payload contains an unsafe bracketed-paste terminator",
                ))
                .await;
        }
        let framed = format!("\u{1b}[200~{}\u{1b}[201~", preview.payload);
        if framed.as_bytes().len() != preview.framed_bytes as usize {
            return Err(InspectionError::new(
                "comments_paste_framing",
                "paste framing byte accounting disagreed with preview",
            ));
        }
        if let Err(error) = adapter.focus_comment_paste_target(&request.target).await {
            return self
                .paste_store
                .save(rejection(
                    request,
                    format!("Herdr did not acknowledge target focus: {}", error.message),
                ))
                .await;
        }
        if let Err(error) = adapter
            .confirm_comment_paste_target_focus(&request.target)
            .await
        {
            return self
                .paste_store
                .save(rejection(
                    request,
                    format!("paste target lost confirmed focus: {}", error.message),
                ))
                .await;
        }
        self.revalidate_source_evidence(session_id, pane_id, &evidence)
            .await?;
        let pending = CommentPasteReceipt {
            operation_id: request.operation_id.clone(),
            request_id: request.request_id.clone(),
            batch_id: batch.batch_id.clone(),
            batch_generation: batch.generation,
            payload_hash,
            target: request.target.clone(),
            state: CommentPasteState::Pending,
            sent_draft_ids: batch
                .drafts
                .iter()
                .map(|draft| draft.draft_id.clone())
                .collect(),
            created_at: timestamp(),
            completed_at: None,
            message: None,
            user_confirmed: false,
        };
        let pending = self
            .paste_store
            .save_frozen(pending, batch.drafts.clone())
            .await?;
        match adapter.send_comment_paste(&request.target, &framed).await {
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
                match self.archive_accepted_drafts(&batch, &accepted).await {
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

    /// Record the user's inspected resolution without contacting Herdr or
    /// retrying the raw write. Only frozen drafts that still match byte-for-byte
    /// may be removed; later edits remain in the batch.
    pub async fn paste_mark_pasted(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &CommentPasteMarkPastedRequest,
    ) -> Result<CommentPasteReceipt, InspectionError> {
        valid_id(&request.operation_id, "operation identity")?;
        let (attachment, evidence) = self
            .attachment(session_id, pane_id, &request.batch.scope)
            .await?;
        let initial = self
            .paste_store
            .inspect_stored(&request.operation_id)
            .await?
            .ok_or_else(|| {
                InspectionError::new(
                    "comments_paste_receipt_not_found",
                    "paste receipt does not exist",
                )
            })?;
        if initial.receipt.batch_id != request.batch.batch_id {
            return Err(InspectionError::new(
                "comments_paste_operation_conflict",
                "paste receipt belongs to another comment batch",
            ));
        }
        // A live sender holds this same target/batch lease. Waiting before
        // recovery ensures a visible Pending receipt is never resolved while a
        // raw write may still be in progress.
        let _lease = self
            .paste_store
            .lease(&initial.receipt.target, &request.batch.batch_id)
            .await?;
        let mut stored = self
            .paste_store
            .load_stored(&request.operation_id)
            .await?
            .ok_or_else(|| {
                InspectionError::new(
                    "comments_paste_receipt_not_found",
                    "paste receipt disappeared",
                )
            })?;
        if stored.receipt.batch_id != request.batch.batch_id {
            return Err(InspectionError::new(
                "comments_paste_operation_conflict",
                "paste receipt belongs to another comment batch",
            ));
        }
        let current = self
            .store
            .load(&request.batch.batch_id)
            .await?
            .ok_or_else(|| {
                InspectionError::new("comments_batch_not_found", "comment batch does not exist")
            })?;
        require_owner(&current, &attachment, &evidence)?;
        if current.generation != request.batch.expected_generation {
            return Err(InspectionError::new(
                "stale_generation",
                "comment batch generation is no longer current",
            ));
        }
        if stored.receipt.user_confirmed {
            return Ok(stored.receipt);
        }
        if !matches!(
            stored.receipt.state,
            CommentPasteState::OutcomeUnknown | CommentPasteState::Accepted
        ) {
            return Err(InspectionError::new(
                "comments_paste_resolution_invalid",
                "only an unknown or reconciliation-required accepted paste can be marked pasted",
            ));
        }
        self.revalidate_source_evidence(session_id, pane_id, &evidence)
            .await?;

        let outcome = if let Some(frozen_drafts) = stored.sent_drafts.as_ref() {
            let mut frozen_batch = current.clone();
            frozen_batch.drafts = frozen_drafts.clone();
            self.archive_accepted_drafts(&frozen_batch, &stored.receipt)
                .await?
        } else {
            ArchiveOutcome::MissingFrozenSnapshot
        };
        stored.receipt.state = CommentPasteState::Accepted;
        stored.receipt.user_confirmed = true;
        stored.receipt.completed_at = Some(timestamp());
        stored.receipt.message = Some(match outcome {
            ArchiveOutcome::Archived => "User marked the inspected paste as delivered; matching frozen drafts were archived.".to_owned(),
            ArchiveOutcome::ReconciliationRequired => "User marked the inspected paste as delivered; edited drafts were retained for further work.".to_owned(),
            ArchiveOutcome::MissingFrozenSnapshot => "User marked this older receipt as delivered after inspection; its frozen draft snapshot is unavailable, so no drafts were archived.".to_owned(),
        });
        self.paste_store.save(stored.receipt).await
    }

    async fn recover_paste_receipts(
        &self,
        batch_id: &str,
    ) -> Result<Vec<CommentPasteReceipt>, InspectionError> {
        let mut receipts = self.paste_store.list_for_batch(batch_id).await?;
        for receipt in &mut receipts {
            if receipt.state != CommentPasteState::Pending {
                continue;
            }
            // Waiting for the target/batch lease distinguishes a live send from a
            // stale record left by a process crash. Keep the lease through recovery.
            let _lease = self.paste_store.lease(&receipt.target, batch_id).await?;
            if let Some(recovered) = self.paste_store.load(&receipt.operation_id).await? {
                *receipt = recovered;
            }
        }
        Ok(receipts)
    }

    async fn archive_accepted_drafts(
        &self,
        sent_batch: &cockpit_protocol::comments::CommentBatch,
        receipt: &CommentPasteReceipt,
    ) -> Result<ArchiveOutcome, InspectionError> {
        for _ in 0..3 {
            let Some(mut current) = self.store.load(&sent_batch.batch_id).await? else {
                return Ok(ArchiveOutcome::Archived);
            };
            let before = current.drafts.len();
            let outcome = archive_matching_drafts(&mut current, sent_batch, receipt);
            if current.drafts.len() == before {
                return Ok(outcome);
            }
            let generation = current.generation;
            match self.store.commit(current, generation).await {
                Ok(current) => {
                    return Ok(if has_unarchived_sent_drafts(&current, receipt) {
                        ArchiveOutcome::ReconciliationRequired
                    } else {
                        ArchiveOutcome::Archived
                    });
                }
                Err(error) if error.code == "stale_generation" => continue,
                Err(error) => return Err(error),
            }
        }
        Err(InspectionError::new(
            "comments_paste_archive_conflict",
            "accepted paste could not archive sent drafts because the batch kept changing",
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveOutcome {
    Archived,
    ReconciliationRequired,
    MissingFrozenSnapshot,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::comment_paste::CommentPasteTarget;
    use cockpit_protocol::comments::{
        CommentAnchor, CommentBatch, CommentDraft, CommentFileRef, CommentLocation, CommentOwner,
        CommentSourceState,
    };
    use cockpit_protocol::context::ExtensionKind;
    use std::fs;

    fn temp_root(label: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("cockpit-comment-paste-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("temporary root");
        root
    }

    fn target() -> CommentPasteTarget {
        CommentPasteTarget {
            endpoint_identity: "endpoint".to_owned(),
            session_id: "session".to_owned(),
            workspace_id: "workspace".to_owned(),
            tab_id: "tab".to_owned(),
            pane_id: "pane".to_owned(),
            terminal_id: "terminal".to_owned(),
            agent_label: "codex".to_owned(),
            agent_fingerprint: format!("sha256:{}", "a".repeat(64)),
        }
    }

    fn batch() -> CommentBatch {
        CommentBatch {
            batch_id: Uuid::new_v4().to_string(),
            generation: 1,
            owner: CommentOwner {
                session_id: "session".to_owned(),
                pane_id: "context".to_owned(),
                terminal_id: "terminal".to_owned(),
                source_kind: ExtensionKind::Context,
                source_id: Uuid::new_v4().to_string(),
            },
            last_known_location: CommentLocation {
                workspace_id: "workspace".to_owned(),
                tab_id: "tab".to_owned(),
            },
            live_attachment: None,
            drafts: vec![CommentDraft {
                draft_id: Uuid::new_v4().to_string(),
                file_ref: CommentFileRef {
                    review: None,
                    root_id: "root".to_owned(),
                    path: "file.txt".to_owned(),
                    absolute_path: "/repo/file.txt".to_owned(),
                    revision: "revision".to_owned(),
                    content_hash: None,
                },
                anchor: CommentAnchor::WholeFile,
                comment_text: "original".to_owned(),
                source_state: CommentSourceState::Current,
                updated_at: "1".to_owned(),
            }],
            updated_at: "1".to_owned(),
        }
    }

    fn accepted_receipt(batch: &CommentBatch) -> CommentPasteReceipt {
        CommentPasteReceipt {
            operation_id: Uuid::new_v4().to_string(),
            request_id: Uuid::new_v4().to_string(),
            batch_id: batch.batch_id.clone(),
            batch_generation: batch.generation,
            payload_hash: format!("sha256:{}", "b".repeat(64)),
            target: target(),
            state: CommentPasteState::Accepted,
            sent_draft_ids: batch
                .drafts
                .iter()
                .map(|draft| draft.draft_id.clone())
                .collect(),
            created_at: timestamp(),
            completed_at: Some(timestamp()),
            message: Some("accepted".to_owned()),
            user_confirmed: false,
        }
    }

    #[test]
    fn concurrent_draft_edit_requires_reconciliation_and_cannot_reenter_payload() {
        let sent = batch();
        let receipt = accepted_receipt(&sent);
        let mut concurrently_edited = sent.clone();
        concurrently_edited.generation += 1;
        concurrently_edited.drafts[0].comment_text = "edited after send".to_owned();
        concurrently_edited.drafts[0].updated_at = "2".to_owned();

        assert_eq!(
            archive_matching_drafts(&mut concurrently_edited, &sent, &receipt),
            ArchiveOutcome::ReconciliationRequired,
        );
        assert_eq!(
            concurrently_edited.drafts.len(),
            1,
            "the newer draft survives"
        );
        assert!(has_unarchived_sent_drafts(&concurrently_edited, &receipt));
    }

    #[test]
    fn frozen_drafts_survive_restart_for_explicit_resolution() {
        let root = temp_root("frozen-restart");
        let store = PasteStore::new(&root).expect("paste store");
        let batch = batch();
        let receipt = accepted_receipt(&batch);
        save_receipt(&store.state, receipt.clone(), Some(batch.drafts.clone()))
            .expect("store frozen drafts");

        let restarted = PasteStore::new(&root).expect("restart paste store");
        let stored = load_stored_receipt(&restarted.state, &receipt.operation_id, false)
            .expect("read stored receipt")
            .expect("receipt");
        assert_eq!(stored.receipt.operation_id, receipt.operation_id);
        let frozen = stored.sent_drafts.as_ref().expect("frozen drafts");
        assert_eq!(frozen.len(), 1);
        assert_eq!(frozen[0].draft_id, batch.drafts[0].draft_id);
        assert_eq!(frozen[0].comment_text, batch.drafts[0].comment_text);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn user_confirmed_resolution_preserves_frozen_drafts_without_a_send() {
        let root = temp_root("user-confirmed");
        let store = PasteStore::new(&root).expect("paste store");
        let batch = batch();
        let mut receipt = accepted_receipt(&batch);
        receipt.state = CommentPasteState::OutcomeUnknown;
        receipt.message = Some("connection dropped after dispatch".to_owned());
        save_receipt(&store.state, receipt.clone(), Some(batch.drafts.clone()))
            .expect("persist uncertain receipt");

        receipt.state = CommentPasteState::Accepted;
        receipt.user_confirmed = true;
        receipt.message = Some("user marked pasted".to_owned());
        save_receipt(&store.state, receipt.clone(), None).expect("persist explicit resolution");

        let stored = load_stored_receipt(&store.state, &receipt.operation_id, false)
            .expect("read resolved receipt")
            .expect("receipt");
        assert!(stored.receipt.user_confirmed);
        assert!(!receipt_requires_reconciliation(&stored.receipt));
        assert_eq!(
            stored.sent_drafts.as_ref().expect("frozen drafts")[0].draft_id,
            batch.drafts[0].draft_id
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn only_proven_pre_dispatch_errors_are_rejected() {
        for code in [
            "request_not_dispatched",
            "comments_paste_input_bounded",
            "comments_paste_framing",
        ] {
            assert!(
                is_definitive_pre_dispatch(&InspectionError::new(code, "before write")),
                "{code}"
            );
        }
        for code in [
            "comments_paste_malformed_ack",
            "comments_paste_rejected",
            "stale_identity",
            "bounded_output",
            "socket_error",
        ] {
            assert!(
                !is_definitive_pre_dispatch(&InspectionError::new(code, "write may have queued")),
                "{code}"
            );
        }
    }

    #[test]
    fn completed_receipt_retention_keeps_normal_sends_bounded_without_unlinking_locks() {
        let root = temp_root("retention");
        let store = PasteStore::new(&root).expect("paste store");
        let batch = batch();
        for _ in 0..(MAX_RECEIPT_RECORDS + 12) {
            save_receipt(&store.state, accepted_receipt(&batch), None)
                .expect("completed receipt remains writable");
        }
        let names = fs::read_dir(root.join("paste"))
            .expect("paste state")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .into_string()
                    .expect("utf-8 name")
            })
            .collect::<Vec<_>>();
        assert!(
            names
                .iter()
                .filter(|name| name.starts_with("receipt-") && name.ends_with(".json"))
                .count()
                <= MAX_RECEIPT_RECORDS
        );
        let lock_count = names
            .iter()
            .filter(|name| name.starts_with(".paste-") && name.ends_with(".lock"))
            .count();
        assert!(
            lock_count <= LOCK_STRIPES + 1,
            "receipt locking uses a fixed namespace"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn old_unknown_receipt_is_checked_before_the_recent_history_is_truncated() {
        let root = temp_root("hidden-unknown");
        let store = PasteStore::new(&root).expect("paste store");
        let batch = batch();
        let mut unknown = accepted_receipt(&batch);
        unknown.state = CommentPasteState::OutcomeUnknown;
        unknown.created_at = "0000-oldest-unknown".to_owned();
        unknown.message = Some("connection dropped after dispatch".to_owned());
        save_receipt(&store.state, unknown.clone(), None).expect("unknown receipt");
        for index in 0..MAX_RECEIPTS {
            let mut rejected = accepted_receipt(&batch);
            rejected.state = CommentPasteState::Rejected;
            rejected.created_at = format!("9999-rejected-{index:03}");
            rejected.message = Some("rejected before dispatch".to_owned());
            save_receipt(&store.state, rejected, None).expect("recent rejected receipt");
        }

        let visible =
            list_receipts_for_batch(&store.state, &batch.batch_id).expect("recent history");
        assert_eq!(visible.len(), MAX_RECEIPTS);
        assert!(
            visible
                .iter()
                .any(|receipt| receipt.operation_id == unknown.operation_id)
        );
        let internal = all_receipts_for_batch(&store.state, &batch.batch_id)
            .expect("complete bounded safety scan");
        assert!(
            internal
                .iter()
                .any(|receipt| receipt.operation_id == unknown.operation_id
                    && receipt.state == CommentPasteState::OutcomeUnknown)
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}
