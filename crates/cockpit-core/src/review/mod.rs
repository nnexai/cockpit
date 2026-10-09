mod cache;
mod git;
mod parse;
mod safe_fs;
mod snapshot;
mod source;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cockpit_protocol::projects::{ProjectConfiguration, ProjectDiagnostic};
use cockpit_protocol::review::{ReviewFileDiff, ReviewFileRequest, ReviewSide};

use crate::InspectionError;
use crate::context::ContextService;
use crate::project_store::ProjectStore;

use cache::{ReviewCacheEntry, StoredSnapshot};
use git::RevisionTokens;
use parse::truncated_diff;
pub(crate) use safe_fs::checkout_source_id;
use snapshot::Change;

#[derive(Clone)]
pub struct ReviewService {
    configuration: ProjectConfiguration,
    context: Arc<ContextService>,
    store: ProjectStore,
    snapshot_cache: Arc<Mutex<VecDeque<ReviewCacheEntry>>>,
}

/// Fresh runtime evidence for a Reviewr comment batch. `source_id` identifies
/// the actual checkout directory, rather than a mutable review snapshot.
#[derive(Debug, Clone)]
pub(crate) struct ReviewCommentEvidence {
    pub binding_id: String,
    pub session_id: String,
    pub viewer_id: String,
    pub server_instance: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub checkout_path: String,
    pub repository_id: String,
    pub source_id: String,
}

/// A full, frozen review-side source document. Callers select physical lines
/// from `text` using the same capture helper as Context comments.
#[derive(Debug, Clone)]
pub(crate) struct ReviewSourceDocument {
    pub source_id: String,
    pub path: String,
    pub revision: String,
    pub content_hash: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewSourceState {
    Current,
    Changed,
}

impl ReviewService {
    pub fn new(
        configuration: ProjectConfiguration,
        context: Arc<ContextService>,
    ) -> Result<Self, InspectionError> {
        let store = ProjectStore::new(Path::new(&configuration.state_root).join("review"))?;
        Ok(Self {
            configuration,
            context,
            store,
            snapshot_cache: Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    pub async fn file(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ReviewFileRequest,
    ) -> Result<ReviewFileDiff, InspectionError> {
        if request.binding_id.is_empty() || request.review_id.is_empty() {
            return Err(InspectionError::new(
                "review_invalid_request",
                "review file request is incomplete",
            ));
        }
        if request.source_side.is_none() && request.source_offset != 0 {
            return Err(InspectionError::new(
                "review_invalid_request",
                "source offset requires a source side",
            ));
        }
        if request.source_side.is_none() && request.source_revision.is_some() {
            return Err(InspectionError::new(
                "review_invalid_request",
                "source revision requires a source side",
            ));
        }
        let stored = self
            .active_snapshot(
                session_id,
                pane_id,
                &request.binding_id,
                &request.review_id,
                request.generation,
            )
            .await?;
        let response = self
            .materialize_file(
                stored,
                &request.file_id,
                request.source_side,
                request.source_offset,
                request.source_revision.as_deref(),
            )
            .await?;
        self.authorize_review_viewer(session_id, pane_id, &request.binding_id)
            .await?;
        Ok(response)
    }

    /// Resolve one file from the immutable change inventory. The first access
    /// performs the bounded Git/source read and persists that payload so later
    /// file switches, comments, and reopens reuse the same frozen content.
    pub(super) async fn materialize_file(
        &self,
        stored: StoredSnapshot,
        file_id: &str,
        source_side: Option<ReviewSide>,
        source_offset: u32,
        source_revision: Option<&str>,
    ) -> Result<ReviewFileDiff, InspectionError> {
        if let Some(diff) = self.load_file_cache(&stored.snapshot, file_id).await? {
            return self
                .continue_source(&stored, diff, source_side, source_offset, source_revision)
                .await;
        }
        let change = stored.changes.get(file_id).cloned().ok_or_else(|| {
            InspectionError::new(
                "review_file_not_found",
                "review file is not in this immutable snapshot",
            )
        })?;
        let revisions = RevisionTokens {
            head: stored.snapshot.head_revision.clone(),
            index: stored.snapshot.index_revision.clone(),
            worktree: stored.snapshot.worktree_revision.clone(),
        };
        self.verify_worktree_revisions(Path::new(&stored.snapshot.checkout_path), &change)
            .await?;
        let (mut diff, _) = match self
            .diff(
                Path::new(&stored.snapshot.checkout_path),
                file_id,
                &change,
                stored.snapshot.base_revision.as_deref(),
                &revisions,
            )
            .await
        {
            Ok(value) => value,
            Err(error) if error.code == "bounded_output" => (
                truncated_diff(
                    file_id,
                    &change,
                    stored.snapshot.base_revision.as_deref(),
                    &revisions,
                    "review_diff_bounded",
                    "file diff exceeds the configured read limit",
                ),
                true,
            ),
            Err(error) if error.code == "review_unreadable" => (
                truncated_diff(
                    file_id,
                    &change,
                    stored.snapshot.base_revision.as_deref(),
                    &revisions,
                    "review_unreadable",
                    "file is unavailable or nonregular",
                ),
                false,
            ),
            Err(error) => return Err(error),
        };
        diff.review_id = stored.snapshot.review_id.clone();
        diff.generation = stored.snapshot.generation;
        diff.binding_id = stored.snapshot.binding_id.clone();
        diff.session_id = stored.snapshot.session_id.clone();
        diff.viewer_id = stored.snapshot.viewer_id.clone();
        self.save_file_cache(&stored.snapshot.review_id, &diff)
            .await?;
        let response = self
            .continue_source(&stored, diff, source_side, source_offset, source_revision)
            .await?;
        Ok(response)
    }

    /// Return a full immutable side document for a sealed comment operation.
    /// It intentionally does not consult Git or the worktree: the persisted
    /// review snapshot is the only source of comment text.
    pub(crate) async fn capture_with_evidence(
        &self,
        evidence: &ReviewCommentEvidence,
        review_id: &str,
        generation: u32,
        file_id: &str,
        side: ReviewSide,
    ) -> Result<ReviewSourceDocument, InspectionError> {
        let stored = self
            .active_snapshot_with_evidence(evidence, review_id, generation)
            .await?;
        let diff = self
            .materialize_file(stored, file_id, None, 0, None)
            .await?;
        let (path, revision, content_hash, text, total_lines, truncated) = match side {
            ReviewSide::Old => (
                diff.file.old_path.as_deref(),
                diff.file.old_revision.as_deref(),
                diff.old_source_hash.as_deref(),
                diff.old_source.as_deref(),
                diff.old_total_lines,
                diff.old_source_truncated,
            ),
            ReviewSide::New => (
                diff.file.new_path.as_deref(),
                diff.file.new_revision.as_deref(),
                diff.new_source_hash.as_deref(),
                diff.new_source.as_deref(),
                diff.new_total_lines,
                diff.new_source_truncated,
            ),
        };
        if truncated {
            return Err(InspectionError::new(
                "review_source_truncated",
                "the frozen review source exceeds its capture limit",
            ));
        }
        let (path, revision, content_hash, text, _total_lines) =
            match (path, revision, content_hash, text, total_lines) {
                (Some(path), Some(revision), Some(content_hash), Some(text), Some(total_lines)) => {
                    (path, revision, content_hash, text, total_lines)
                }
                _ => {
                    return Err(InspectionError::new(
                        "review_source_unavailable",
                        "this review side has no textual immutable source",
                    ));
                }
            };
        Ok(ReviewSourceDocument {
            source_id: evidence.source_id.clone(),
            path: path.to_owned(),
            revision: revision.to_owned(),
            content_hash: content_hash.to_owned(),
            text: text.to_owned(),
        })
    }

    /// Re-read only bounded Git revision evidence under a sealed checkout.
    /// Callers can cache the result per review id while refreshing a comment
    /// batch; no remapping or snapshot replacement occurs here.
    pub(crate) async fn source_state_with_evidence(
        &self,
        evidence: &ReviewCommentEvidence,
        review_id: &str,
        generation: u32,
        file_id: &str,
        side: ReviewSide,
    ) -> Result<ReviewSourceState, InspectionError> {
        let stored = self
            .active_snapshot_with_evidence(evidence, review_id, generation)
            .await?;
        let checkout = PathBuf::from(&stored.snapshot.checkout_path);
        let base_revision = stored.snapshot.base_revision.clone();
        let diff = self
            .materialize_file(stored, file_id, None, 0, None)
            .await?;
        let change = Change {
            status: diff.file.status,
            comparison: diff.file.comparison,
            old_path: diff.file.old_path.clone(),
            new_path: diff.file.new_path.clone(),
            old_revision: diff.file.old_revision.clone(),
            new_revision: diff.file.new_revision.clone(),
        };
        let revisions = RevisionTokens {
            head: self
                .git_text_optional(&checkout, &["rev-parse", "--verify", "HEAD"])
                .await?,
            index: String::new(),
            worktree: String::new(),
        };
        let current = self
            .snapshot_source(
                &checkout,
                &change,
                base_revision.as_deref(),
                &revisions,
                side,
            )
            .await?;
        let frozen_hash = match side {
            ReviewSide::Old => &diff.old_source_hash,
            ReviewSide::New => &diff.new_source_hash,
        };
        if current.hash.is_none() || current.truncated {
            return Err(InspectionError::new(
                "review_source_unavailable",
                "anchored source is no longer readable",
            ));
        }
        Ok(if &current.hash == frozen_hash {
            ReviewSourceState::Current
        } else {
            ReviewSourceState::Changed
        })
    }
}

fn diagnostic(code: &str, message: &str, path: Option<&str>) -> ProjectDiagnostic {
    ProjectDiagnostic {
        code: code.to_owned(),
        message: message.to_owned(),
        path: path.map(str::to_owned),
    }
}

#[cfg(test)]
mod test_support;
