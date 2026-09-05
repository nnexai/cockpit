mod format;
mod paste;
pub(super) mod store;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::task::JoinSet;

use cockpit_protocol::comments::{
    CommentAnchor, CommentAttachment, CommentBatch, CommentBatchList, CommentBatchMutation,
    CommentBatchRequest, CommentBatchSummary, CommentCapture, CommentDraft, CommentFileRef,
    CommentLocation, CommentOwner, CommentPreview, CommentPreviewRequest, CommentRemoveRequest,
    CommentRequestScope, CommentSourceState, CommentUpsertRequest,
};
use cockpit_protocol::context::{ContextDocumentRequest, ExtensionKind};
use cockpit_protocol::projects::ProjectConfiguration;
use uuid::Uuid;

use crate::InspectionError;
use crate::context::ContextService;
use crate::paste_adapter::CommentPasteAdapter;
use crate::project_store::timestamp;

use self::format::{capture_lines, format_batch};
use self::paste::PasteStore;
use self::store::CommentStore;

const MAX_DRAFTS: usize = 64;
const MAX_COMMENT_BYTES: usize = 8 * 1024;
const MAX_REVIEW_STATE_READS_IN_FLIGHT: usize = 4;

type ReviewStateKey = (String, u32, String, bool);

struct CommentEvidence {
    binding_id: String,
    terminal_id: String,
    workspace_id: String,
    tab_id: String,
    root_id: String,
    companion_id: String,
    companion_path: String,
    source_kind: ExtensionKind,
    review: Option<crate::review::ReviewCommentEvidence>,
}

struct CapturedDocument {
    root_id: String,
    path: String,
    revision: String,
    content_hash: Option<String>,
    text: Option<String>,
    truncated: bool,
}

#[derive(Clone)]
pub struct CommentsService {
    store: CommentStore,
    paste_store: PasteStore,
    context: Arc<ContextService>,
    paste_adapter: Option<Arc<dyn CommentPasteAdapter>>,
    reviews: Option<Arc<crate::review::ReviewService>>,
}

impl CommentsService {
    pub fn new(
        configuration: ProjectConfiguration,
        context: Arc<ContextService>,
    ) -> Result<Self, InspectionError> {
        let root = Path::new(&configuration.state_root).join("comments");
        Ok(Self {
            store: CommentStore::new(&root)?,
            paste_store: PasteStore::new(&root)?,
            context,
            paste_adapter: None,
            reviews: None,
        })
    }

    pub fn with_reviews(mut self, reviews: Arc<crate::review::ReviewService>) -> Self {
        self.reviews = Some(reviews);
        self
    }

    /// Add the separately capability-gated raw-byte paste adapter.
    pub fn with_paste_adapter(mut self, adapter: Arc<dyn CommentPasteAdapter>) -> Self {
        self.paste_adapter = Some(adapter);
        self
    }

    pub async fn list(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &CommentRequestScope,
    ) -> Result<CommentBatchList, InspectionError> {
        let (attachment, _) = self.attachment(session_id, pane_id, request).await?;
        let (batches, truncated) = self.store.list().await?;
        let batches = batches
            .into_iter()
            .map(|batch| CommentBatchSummary {
                batch_id: batch.batch_id,
                generation: batch.generation,
                owner: batch.owner,
                last_known_location: batch.last_known_location,
                draft_count: batch.drafts.len() as u32,
                updated_at: batch.updated_at,
            })
            .collect();
        Ok(CommentBatchList {
            attachment,
            batches,
            truncated,
        })
    }

    pub async fn batch(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &CommentBatchRequest,
    ) -> Result<CommentBatch, InspectionError> {
        let (attachment, evidence) = self.attachment(session_id, pane_id, &request.scope).await?;
        let Some(batch_id) = request.batch_id.as_deref() else {
            let (batches, _) = self.store.list().await?;
            if let Some(mut batch) = batches.into_iter().find(|batch| {
                same_owner(&batch.owner, &attachment.owner)
                    && batch.last_known_location == attachment.location
            }) {
                if same_attachment(&batch, &attachment, &evidence) {
                    batch.live_attachment = Some(attachment);
                    self.refresh_states(&mut batch, &evidence).await;
                } else {
                    // A matching owner with a replaced root is recoverable but detached.
                    batch.live_attachment = None;
                }
                return Ok(batch);
            }
            return Ok(empty_batch(&attachment));
        };
        let mut batch = self.store.load(batch_id).await?.ok_or_else(|| {
            InspectionError::new("comments_batch_not_found", "comment batch does not exist")
        })?;
        if same_attachment(&batch, &attachment, &evidence) {
            batch.live_attachment = Some(attachment);
            self.refresh_states(&mut batch, &evidence).await;
        } else {
            // Recovery is intentionally inspectable but never implicitly attached.
            batch.live_attachment = None;
        }
        Ok(batch)
    }

    pub async fn upsert(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &CommentUpsertRequest,
    ) -> Result<CommentBatch, InspectionError> {
        validate_comment_text(&request.comment_text)?;
        let (attachment, evidence) = self
            .attachment(session_id, pane_id, &request.batch.scope)
            .await?;
        let mut batch = match self.store.load(&request.batch.batch_id).await? {
            Some(batch) => {
                require_owner(&batch, &attachment, &evidence)?;
                batch
            }
            None if request.batch.expected_generation == 0 => {
                empty_batch_with_id(&attachment, &request.batch.batch_id)
            }
            None => {
                return Err(InspectionError::new(
                    "comments_batch_not_found",
                    "comment batch does not exist",
                ));
            }
        };
        if batch.generation != request.batch.expected_generation {
            return Err(InspectionError::new(
                "stale_generation",
                "comment batch generation is no longer current",
            ));
        }
        if request.draft_id.is_some() && request.capture.is_some() {
            return Err(InspectionError::new(
                "comments_invalid_capture",
                "editing a comment cannot include a new source capture",
            ));
        }
        match request.draft_id.as_deref() {
            Some(draft_id) => {
                let draft = batch
                    .drafts
                    .iter_mut()
                    .find(|draft| draft.draft_id == draft_id)
                    .ok_or_else(|| {
                        InspectionError::new(
                            "comments_draft_not_found",
                            "comment draft does not exist",
                        )
                    })?;
                // An edit changes only prose; its captured source is immutable.
                draft.comment_text = request.comment_text.clone();
                draft.updated_at = timestamp();
            }
            None => {
                if batch.drafts.len() >= MAX_DRAFTS {
                    return Err(InspectionError::new(
                        "comments_limit",
                        "comment batch draft limit exceeded",
                    ));
                }
                let capture = request.capture.as_ref().ok_or_else(|| {
                    InspectionError::new(
                        "comments_capture_required",
                        "new drafts require a source capture",
                    )
                })?;
                let draft = self
                    .capture_draft(
                        session_id,
                        pane_id,
                        capture,
                        &request.comment_text,
                        &evidence,
                    )
                    .await?;
                batch.drafts.push(draft);
            }
        }
        self.refresh_states(&mut batch, &evidence).await;
        self.revalidate_source_evidence(session_id, pane_id, &evidence)
            .await?;
        let committed = self
            .store
            .commit(batch, request.batch.expected_generation)
            .await?;
        Ok(with_attachment(committed, attachment))
    }

    /// Explicitly discard a saved batch, including detached recovery batches.
    /// Fresh pane proof gates access; generation checking protects concurrent edits.
    pub async fn discard(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &CommentBatchMutation,
    ) -> Result<CommentBatchList, InspectionError> {
        let (_, evidence) = self.attachment(session_id, pane_id, &request.scope).await?;
        self.revalidate_source_evidence(session_id, pane_id, &evidence)
            .await?;
        let mut remaining = self.list(session_id, pane_id, &request.scope).await?;
        self.store
            .discard(&request.batch_id, request.expected_generation)
            .await?;
        remaining
            .batches
            .retain(|batch| batch.batch_id != request.batch_id);
        Ok(remaining)
    }

    pub async fn remove(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &CommentRemoveRequest,
    ) -> Result<CommentBatch, InspectionError> {
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
        let before = batch.drafts.len();
        batch
            .drafts
            .retain(|draft| draft.draft_id != request.draft_id);
        if batch.drafts.len() == before {
            return Err(InspectionError::new(
                "comments_draft_not_found",
                "comment draft does not exist",
            ));
        }
        self.refresh_states(&mut batch, &evidence).await;
        self.revalidate_source_evidence(session_id, pane_id, &evidence)
            .await?;
        let committed = self
            .store
            .commit(batch, request.batch.expected_generation)
            .await?;
        Ok(with_attachment(committed, attachment))
    }

    pub async fn attach(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &CommentBatchMutation,
    ) -> Result<CommentBatch, InspectionError> {
        let (attachment, evidence) = self.attachment(session_id, pane_id, &request.scope).await?;
        let mut batch = self.store.load(&request.batch_id).await?.ok_or_else(|| {
            InspectionError::new("comments_batch_not_found", "comment batch does not exist")
        })?;
        if batch.owner.source_kind != attachment.owner.source_kind
            || batch.owner.source_id != attachment.owner.source_id
            || !captured_roots_match(&batch, &evidence)
        {
            return Err(InspectionError::new(
                "comments_source_mismatch",
                "a detached batch can only reattach to its original Context source root",
            ));
        }
        if batch.generation != request.expected_generation {
            return Err(InspectionError::new(
                "stale_generation",
                "comment batch generation is no longer current",
            ));
        }
        batch.owner = attachment.owner.clone();
        batch.last_known_location = attachment.location.clone();
        self.refresh_states(&mut batch, &evidence).await;
        self.revalidate_source_evidence(session_id, pane_id, &evidence)
            .await?;
        let committed = self
            .store
            .commit(batch, request.expected_generation)
            .await?;
        Ok(with_attachment(committed, attachment))
    }

    pub async fn preview(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &CommentPreviewRequest,
    ) -> Result<CommentPreview, InspectionError> {
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
        let formatted = format_batch(&batch, request.retain_stale_excerpts);
        if formatted.payload_bytes > format::MAX_PREVIEW_PAYLOAD_BYTES {
            return Err(InspectionError::new(
                "comments_preview_bounded",
                "comment preview exceeds the 4 MiB response payload limit",
            ));
        }
        Ok(CommentPreview {
            batch_id: batch.batch_id,
            generation: batch.generation,
            payload: formatted.payload,
            payload_bytes: formatted.payload_bytes,
            framed_bytes: formatted.framed_bytes,
            limit_bytes: format::PREVIEW_LIMIT_BYTES,
            sanitized_controls: formatted.sanitized_controls,
            stale_draft_ids: formatted.stale_draft_ids,
            exportable: formatted.exportable,
            reason: formatted.reason,
        })
    }

    async fn attachment(
        &self,
        session_id: &str,
        pane_id: &str,
        scope: &CommentRequestScope,
    ) -> Result<(CommentAttachment, CommentEvidence), InspectionError> {
        if scope.binding_id.is_empty() || scope.client_id.is_empty() {
            return Err(InspectionError::new(
                "comments_invalid_scope",
                "comment scope is incomplete",
            ));
        }
        let (presentation, runtime_evidence) = self
            .context
            .inspect_pane_with_evidence(session_id, pane_id)
            .await?;
        let evidence = if presentation.renderer == Some(ExtensionKind::Review) {
            let review = self.reviews.as_ref().ok_or_else(|| {
                InspectionError::new("review_unavailable", "Review comments are not configured")
            })?;
            let value = review
                .comment_evidence_for_presentation(
                    session_id,
                    pane_id,
                    &scope.binding_id,
                    &presentation,
                    &runtime_evidence,
                )
                .await?;
            CommentEvidence {
                binding_id: value.binding_id.clone(),
                terminal_id: value.terminal_id.clone(),
                workspace_id: value.workspace_id.clone(),
                tab_id: value.tab_id.clone(),
                root_id: value.source_id.clone(),
                companion_id: value.source_id.clone(),
                companion_path: value.checkout_path.clone(),
                source_kind: ExtensionKind::Review,
                review: Some(value),
            }
        } else {
            let value = self.context.comment_evidence_for_presentation(
                &presentation,
                &runtime_evidence,
                &scope.binding_id,
            )?;
            CommentEvidence {
                binding_id: value.binding_id,
                terminal_id: value.terminal_id,
                workspace_id: value.workspace_id,
                tab_id: value.tab_id,
                root_id: value.root_id,
                companion_id: value.companion_id,
                companion_path: value.companion_path,
                source_kind: ExtensionKind::Context,
                review: None,
            }
        };
        let owner = CommentOwner {
            session_id: session_id.to_owned(),
            pane_id: pane_id.to_owned(),
            terminal_id: evidence.terminal_id.clone(),
            source_kind: evidence.source_kind,
            source_id: evidence.companion_id.clone(),
        };
        let attachment = CommentAttachment {
            owner,
            location: CommentLocation {
                workspace_id: evidence.workspace_id.clone(),
                tab_id: evidence.tab_id.clone(),
            },
            binding_id: evidence.binding_id.clone(),
            client_id: scope.client_id.clone(),
        };
        Ok((attachment, evidence))
    }
    async fn capture_draft(
        &self,
        session_id: &str,
        pane_id: &str,
        capture: &CommentCapture,
        comment_text: &str,
        evidence: &CommentEvidence,
    ) -> Result<CommentDraft, InspectionError> {
        if capture.root_id != evidence.root_id {
            return Err(InspectionError::new(
                "comments_source_mismatch",
                "capture root is not the currently verified browsing root",
            ));
        }
        let document = if evidence.source_kind == ExtensionKind::Review {
            let reference = capture.review.as_ref().ok_or_else(|| {
                InspectionError::new(
                    "comments_capture_required",
                    "Review capture needs an immutable snapshot and side",
                )
            })?;
            let review = self.reviews.as_ref().ok_or_else(|| {
                InspectionError::new("review_unavailable", "Review comments are not configured")
            })?;
            let value = review
                .capture_with_evidence(
                    evidence.review.as_ref().ok_or_else(|| {
                        InspectionError::new(
                            "review_unavailable",
                            "Review evidence is not available for this comment operation",
                        )
                    })?,
                    &reference.review_id,
                    reference.generation,
                    &reference.file_id,
                    reference.side,
                )
                .await?;
            if value.source_id != evidence.root_id
                || value.path != capture.path
                || value.revision != capture.expected_revision
            {
                return Err(InspectionError::new(
                    "comments_stale_source",
                    "Review source identity does not match the captured side",
                ));
            }
            CapturedDocument {
                root_id: value.source_id,
                path: value.path,
                revision: value.revision,
                content_hash: Some(value.content_hash),
                text: Some(value.text),
                truncated: false,
            }
        } else {
            if capture.review.is_some() {
                return Err(InspectionError::new(
                    "comments_source_mismatch",
                    "Context capture cannot carry a Review anchor",
                ));
            }
            let value = self
                .context
                .document(
                    session_id,
                    pane_id,
                    &ContextDocumentRequest {
                        binding_id: evidence.binding_id.clone(),
                        root_id: evidence.root_id.clone(),
                        path: capture.path.clone(),
                        expected_revision: Some(capture.expected_revision.clone()),
                    },
                )
                .await
                .map_err(|error| {
                    if error.code == "context_stale_revision"
                        || error.code == "context_changed_during_read"
                    {
                        InspectionError::new("comments_stale_source", error.message)
                    } else {
                        InspectionError::new("comments_capture_unavailable", error.message)
                    }
                })?;
            CapturedDocument {
                root_id: value.root_id,
                path: value.path,
                revision: value.revision,
                content_hash: value.content_hash,
                text: value.text,
                truncated: value.truncated,
            }
        };
        let truncated = document.truncated;
        let text = match document.text {
            Some(text) => text,
            None => {
                return Err(InspectionError::new(
                    if truncated {
                        "comments_source_truncated"
                    } else {
                        "comments_capture_unavailable"
                    },
                    "the selected Context document is not available as complete UTF-8 text",
                ));
            }
        };
        if truncated {
            return Err(InspectionError::new(
                "comments_source_truncated",
                "the selected Context document exceeds its bounded read limit",
            ));
        }
        let anchor = match (capture.start_line, capture.end_line) {
            (None, None) => CommentAnchor::WholeFile,
            (Some(start_line), Some(end_line)) => {
                let selected_lines = capture_lines(&text, start_line, end_line)
                    .map_err(|message| InspectionError::new("comments_invalid_range", message))?;
                CommentAnchor::Lines {
                    start_line,
                    end_line,
                    selected_lines,
                }
            }
            _ => {
                return Err(InspectionError::new(
                    "comments_invalid_range",
                    "line capture must provide both bounds or neither",
                ));
            }
        };
        let absolute_path = Path::new(&evidence.companion_path)
            .join(&document.path)
            .to_string_lossy()
            .into_owned();
        Ok(CommentDraft {
            draft_id: Uuid::new_v4().to_string(),
            file_ref: CommentFileRef {
                review: capture.review.clone(),
                root_id: document.root_id,
                path: document.path,
                absolute_path,
                revision: document.revision,
                content_hash: document.content_hash,
            },
            anchor,
            comment_text: comment_text.to_owned(),
            source_state: CommentSourceState::Current,
            updated_at: timestamp(),
        })
    }

    async fn refresh_states(&self, batch: &mut CommentBatch, evidence: &CommentEvidence) {
        if evidence.source_kind == ExtensionKind::Review {
            let Some(operation) = evidence.review.clone() else {
                for draft in &mut batch.drafts {
                    draft.source_state = CommentSourceState::Unavailable;
                }
                return;
            };
            let Some(review) = self.reviews.as_ref().cloned() else {
                for draft in &mut batch.drafts {
                    draft.source_state = CommentSourceState::Unavailable;
                }
                return;
            };
            let mut unique = HashMap::new();
            for draft in &batch.drafts {
                if draft.file_ref.root_id != evidence.root_id {
                    continue;
                }
                let Some(reference) = draft.file_ref.review.as_ref() else {
                    continue;
                };
                let key = (
                    reference.review_id.clone(),
                    reference.generation,
                    reference.file_id.clone(),
                    reference.side == cockpit_protocol::review::ReviewSide::Old,
                );
                unique.entry(key).or_insert_with(|| reference.clone());
            }
            let mut states = HashMap::new();
            let mut pending = unique.into_iter();
            let mut reads = JoinSet::new();
            loop {
                while reads.len() < MAX_REVIEW_STATE_READS_IN_FLIGHT {
                    let Some((key, reference)) = pending.next() else {
                        break;
                    };
                    let review = review.clone();
                    let operation = operation.clone();
                    reads.spawn(async move {
                        let result = review
                            .source_state_with_evidence(
                                &operation,
                                &reference.review_id,
                                reference.generation,
                                &reference.file_id,
                                reference.side,
                            )
                            .await;
                        (key, result)
                    });
                }
                let Some(result) = reads.join_next().await else {
                    break;
                };
                if let Ok((key, result)) = result {
                    let state = match result {
                        Ok(crate::review::ReviewSourceState::Current) => {
                            CommentSourceState::Current
                        }
                        Ok(crate::review::ReviewSourceState::Changed) => {
                            CommentSourceState::Changed
                        }
                        Err(_) => CommentSourceState::Unavailable,
                    };
                    states.insert(key, state);
                }
            }
            for draft in &mut batch.drafts {
                let Some(reference) = draft.file_ref.review.as_ref() else {
                    draft.source_state = CommentSourceState::Unavailable;
                    continue;
                };
                let key: ReviewStateKey = (
                    reference.review_id.clone(),
                    reference.generation,
                    reference.file_id.clone(),
                    reference.side == cockpit_protocol::review::ReviewSide::Old,
                );
                draft.source_state = if draft.file_ref.root_id == evidence.root_id {
                    states
                        .get(&key)
                        .copied()
                        .unwrap_or(CommentSourceState::Unavailable)
                } else {
                    CommentSourceState::Unavailable
                };
            }
            return;
        }
        let session_id = batch.owner.session_id.clone();
        let pane_id = batch.owner.pane_id.clone();
        let mut reads: HashMap<(String, String), Result<(String, Option<String>), String>> =
            HashMap::new();
        for draft in &mut batch.drafts {
            if draft.file_ref.root_id != evidence.root_id {
                draft.source_state = CommentSourceState::Unavailable;
                continue;
            }
            let key = (draft.file_ref.root_id.clone(), draft.file_ref.path.clone());
            if !reads.contains_key(&key) {
                let result = self
                    .context
                    .document(
                        &session_id,
                        &pane_id,
                        &ContextDocumentRequest {
                            binding_id: evidence.binding_id.clone(),
                            root_id: evidence.root_id.clone(),
                            path: draft.file_ref.path.clone(),
                            expected_revision: None,
                        },
                    )
                    .await
                    .map(|document| (document.revision, document.content_hash))
                    .map_err(|error| error.code);
                reads.insert(key.clone(), result);
            }
            let result = reads
                .get(&key)
                .expect("source read inserted before state comparison");
            draft.source_state = match result {
                Ok((revision, content_hash))
                    if revision == &draft.file_ref.revision
                        && content_hash == &draft.file_ref.content_hash =>
                {
                    CommentSourceState::Current
                }
                Ok(_) => CommentSourceState::Changed,
                Err(code) if code == "context_file_missing" => CommentSourceState::Missing,
                Err(_) => CommentSourceState::Unavailable,
            };
        }
    }

    async fn revalidate_source_evidence(
        &self,
        session_id: &str,
        pane_id: &str,
        evidence: &CommentEvidence,
    ) -> Result<(), InspectionError> {
        let Some(expected) = evidence.review.as_ref() else {
            let current = self
                .context
                .comment_evidence(session_id, pane_id, &evidence.binding_id)
                .await?;
            if current.terminal_id != evidence.terminal_id
                || current.workspace_id != evidence.workspace_id
                || current.tab_id != evidence.tab_id
                || current.root_id != evidence.root_id
                || current.companion_id != evidence.companion_id
                || current.companion_path != evidence.companion_path
            {
                return Err(InspectionError::new(
                    "comments_detached",
                    "file-viewer pane or browsing root changed during this comment operation",
                ));
            }
            return Ok(());
        };
        let review = self.reviews.as_ref().ok_or_else(|| {
            InspectionError::new("review_unavailable", "Review comments are not configured")
        })?;
        let actual = review
            .comment_evidence(session_id, pane_id, &expected.binding_id)
            .await?;
        if !same_review_evidence(expected, &actual) {
            return Err(InspectionError::new(
                "comments_detached",
                "Reviewr pane or checkout changed while saving this comment",
            ));
        }
        Ok(())
    }
}

fn same_review_evidence(
    expected: &crate::review::ReviewCommentEvidence,
    actual: &crate::review::ReviewCommentEvidence,
) -> bool {
    expected.binding_id == actual.binding_id
        && expected.session_id == actual.session_id
        && expected.pane_id == actual.pane_id
        && expected.terminal_id == actual.terminal_id
        && expected.workspace_id == actual.workspace_id
        && expected.tab_id == actual.tab_id
        && expected.checkout_path == actual.checkout_path
        && expected.repository_id == actual.repository_id
        && expected.source_id == actual.source_id
}

fn empty_batch(attachment: &CommentAttachment) -> CommentBatch {
    empty_batch_with_id(attachment, &Uuid::new_v4().to_string())
}

fn empty_batch_with_id(attachment: &CommentAttachment, batch_id: &str) -> CommentBatch {
    CommentBatch {
        batch_id: batch_id.to_owned(),
        generation: 0,
        owner: attachment.owner.clone(),
        last_known_location: attachment.location.clone(),
        live_attachment: Some(attachment.clone()),
        drafts: Vec::new(),
        updated_at: timestamp(),
    }
}

fn with_attachment(mut batch: CommentBatch, attachment: CommentAttachment) -> CommentBatch {
    batch.live_attachment = Some(attachment);
    batch
}

fn same_owner(left: &CommentOwner, right: &CommentOwner) -> bool {
    left.session_id == right.session_id
        && left.pane_id == right.pane_id
        && left.terminal_id == right.terminal_id
        && left.source_kind == right.source_kind
        && left.source_id == right.source_id
}

fn captured_roots_match(batch: &CommentBatch, evidence: &CommentEvidence) -> bool {
    batch
        .drafts
        .iter()
        .all(|draft| draft.file_ref.root_id == evidence.root_id)
}

fn same_attachment(
    batch: &CommentBatch,
    attachment: &CommentAttachment,
    evidence: &CommentEvidence,
) -> bool {
    same_owner(&batch.owner, &attachment.owner)
        && batch.last_known_location == attachment.location
        && captured_roots_match(batch, evidence)
}

fn require_owner(
    batch: &CommentBatch,
    attachment: &CommentAttachment,
    evidence: &CommentEvidence,
) -> Result<(), InspectionError> {
    if !same_owner(&batch.owner, &attachment.owner)
        || batch.last_known_location != attachment.location
    {
        return Err(InspectionError::new(
            "comments_detached",
            "comment batch is detached from this Context pane or tab",
        ));
    }
    if !captured_roots_match(batch, evidence) {
        return Err(InspectionError::new(
            "comments_source_mismatch",
            "comment batch belongs to a replaced Context source root",
        ));
    }
    Ok(())
}

fn validate_comment_text(text: &str) -> Result<(), InspectionError> {
    if text.as_bytes().len() > MAX_COMMENT_BYTES {
        return Err(InspectionError::new(
            "comments_limit",
            "comment text exceeds the 8 KiB limit",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment() -> CommentAttachment {
        CommentAttachment {
            owner: CommentOwner {
                session_id: "session".to_owned(),
                pane_id: "pane".to_owned(),
                terminal_id: "terminal".to_owned(),
                source_kind: ExtensionKind::Context,
                source_id: "companion".to_owned(),
            },
            location: CommentLocation {
                workspace_id: "workspace".to_owned(),
                tab_id: "tab".to_owned(),
            },
            binding_id: "binding".to_owned(),
            client_id: "client".to_owned(),
        }
    }

    fn evidence(root_id: &str) -> CommentEvidence {
        CommentEvidence {
            source_kind: ExtensionKind::Context,
            review: None,
            binding_id: "binding".to_owned(),
            terminal_id: "terminal".to_owned(),
            workspace_id: "workspace".to_owned(),
            tab_id: "tab".to_owned(),
            root_id: root_id.to_owned(),
            companion_id: "companion".to_owned(),
            companion_path: "/companion".to_owned(),
        }
    }

    #[test]
    fn initial_batch_has_a_transport_valid_timestamp() {
        let batch = empty_batch(&attachment());
        assert_eq!(batch.generation, 0);
        assert!(batch.drafts.is_empty());
        assert!(batch.updated_at.parse::<u128>().is_ok());
        assert!(batch.live_attachment.is_some());
    }

    #[test]
    fn captured_root_identity_blocks_replacement_but_keeps_empty_batch_attachable() {
        let attachment = attachment();
        let mut batch = empty_batch(&attachment);
        assert!(captured_roots_match(&batch, &evidence("root-a")));
        batch.drafts.push(CommentDraft {
            draft_id: Uuid::new_v4().to_string(),
            file_ref: CommentFileRef {
                review: None,
                root_id: "root-a".to_owned(),
                path: "notes.md".to_owned(),
                absolute_path: "/companion/notes.md".to_owned(),
                revision: "rev".to_owned(),
                content_hash: None,
            },
            anchor: CommentAnchor::WholeFile,
            comment_text: "review".to_owned(),
            source_state: CommentSourceState::Current,
            updated_at: String::new(),
        });
        assert!(captured_roots_match(&batch, &evidence("root-a")));
        assert!(!captured_roots_match(&batch, &evidence("root-b")));
    }

    #[test]
    fn scoped_review_evidence_rejects_a_changed_checkout_or_pane() {
        let expected = crate::review::ReviewCommentEvidence {
            binding_id: "binding".to_owned(),
            session_id: "session".to_owned(),
            pane_id: "pane".to_owned(),
            terminal_id: "terminal".to_owned(),
            workspace_id: "workspace".to_owned(),
            tab_id: "tab".to_owned(),
            checkout_path: "/checkout".to_owned(),
            repository_id: "repository".to_owned(),
            source_id: "source-a".to_owned(),
        };
        let mut changed = expected.clone();
        changed.source_id = "source-b".to_owned();
        assert!(!same_review_evidence(&expected, &changed));
        let mut changed = expected.clone();
        changed.terminal_id = "replacement-terminal".to_owned();
        assert!(!same_review_evidence(&expected, &changed));
    }
}
