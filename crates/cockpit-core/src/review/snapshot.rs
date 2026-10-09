use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cockpit_protocol::projects::{ProjectDiagnostic, RepositoryCandidate};
use cockpit_protocol::review::{
    ReviewChangedFile, ReviewComparison, ReviewFileStatus, ReviewSnapshot, ReviewSnapshotRequest,
};
use cockpit_protocol::viewer::ViewerKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::InspectionError;
use crate::project_store::timestamp;
use crate::viewer::ViewerAuthorization;

use super::cache::{MAX_SNAPSHOTS, ReviewCacheEntry, StoredSnapshot};
use super::git::{MAX_FILE_LIST_BYTES, RevisionTokens};
use super::parse::summary;
use super::safe_fs::{checkout_source_id, verified_checkout_path};
use super::source::{read_worktree_source, worktree_source_identity};
use super::{ReviewCommentEvidence, ReviewService, diagnostic};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Change {
    pub(super) status: ReviewFileStatus,
    pub(super) comparison: ReviewComparison,
    pub(super) old_path: Option<String>,
    pub(super) new_path: Option<String>,
    /// Per-file source identities captured with the inventory. Global review
    /// tokens only invalidate the inventory; these cursors anchor reads.
    #[serde(default)]
    pub(super) old_revision: Option<String>,
    #[serde(default)]
    pub(super) new_revision: Option<String>,
}

impl ReviewService {
    pub async fn snapshot(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ReviewSnapshotRequest,
    ) -> Result<ReviewSnapshot, InspectionError> {
        validate_snapshot_request(request)?;
        let authorization = self
            .authorize_review_viewer(session_id, pane_id, &request.binding_id)
            .await?;
        let evidence = self
            .comment_evidence_for_authorization(&authorization)
            .await?;
        let repository = self
            .resolve_checkout(
                &authorization.source.cwd,
                &authorization.source.foreground_cwd,
                &request.repository_id,
            )
            .await?;
        if repository.repository_id != evidence.repository_id
            || repository.checkout_path != evidence.checkout_path
        {
            return Err(InspectionError::new(
                "review_checkout_mismatch",
                "selected repository differs from the authorized viewer root",
            ));
        }
        let checkout = PathBuf::from(&repository.checkout_path);
        let binding_id = request.binding_id.clone();
        let source_id = checkout_source_id(&checkout)?;
        for attempt in 0..2 {
            let before = self.revision_tokens(&checkout).await?;
            let base_revision = if request.comparison == ReviewComparison::Branch {
                Some(
                    self.git_text(
                        &checkout,
                        &[
                            "merge-base",
                            "--",
                            request.base_ref.as_deref().expect("validated"),
                            "HEAD",
                        ],
                    )
                    .await?,
                )
            } else {
                None
            };
            let cache_key = format!(
                "{}\0{}\0{}\0{}\0{}\0{}\0{:?}\0{}",
                session_id,
                pane_id,
                request.binding_id,
                request.repository_id,
                source_id,
                repository.repository_id,
                request.comparison,
                base_revision.as_deref().unwrap_or_default(),
            );
            let cached = {
                let mut entries = self
                    .snapshot_cache
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                let index = entries
                    .iter()
                    .position(|entry| entry.key == cache_key && entry.revisions == before);
                index.and_then(|index| entries.remove(index))
            };
            if let Some(entry) = cached {
                if let Some(snapshot) = self
                    .reusable_snapshot(
                        &entry,
                        session_id,
                        pane_id,
                        request,
                        &repository,
                        &source_id,
                        base_revision.as_deref(),
                        &before,
                    )
                    .await?
                {
                    let mut entries = self
                        .snapshot_cache
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    entries.push_front(entry);
                    return Ok(snapshot);
                }
            }
            let (files, changes, mut diagnostics, truncated, collected_base_revision) = self
                .collect(&checkout, request, &before, base_revision.clone())
                .await?;
            let after = self.revision_tokens(&checkout).await?;
            if before == after {
                let review_id = Uuid::new_v4().to_string();
                let generation = 1;
                let snapshot = ReviewSnapshot {
                    binding_id,
                    session_id: session_id.to_owned(),
                    viewer_id: pane_id.to_owned(),
                    review_id: review_id.clone(),
                    generation,
                    repository_id: repository.repository_id,
                    checkout_path: repository.checkout_path,
                    source_id,
                    comparison: request.comparison,
                    base_revision: collected_base_revision,
                    head_revision: before.head.clone(),
                    index_revision: before.index.clone(),
                    worktree_revision: before.worktree.clone(),
                    files,
                    truncated,
                    diagnostics: std::mem::take(&mut diagnostics),
                };
                return self
                    .retain_snapshot(snapshot, changes, cache_key, before)
                    .await;
            }
            if attempt == 1 {
                return Err(InspectionError::new(
                    "review_changed_during_read",
                    "Git index or working tree changed while Cockpit read the review; refresh it again",
                ));
            }
        }
        unreachable!("two attempts return or fail")
    }

    async fn retain_snapshot(
        &self,
        snapshot: ReviewSnapshot,
        changes: BTreeMap<String, Change>,
        cache_key: String,
        revisions: RevisionTokens,
    ) -> Result<ReviewSnapshot, InspectionError> {
        self.save_snapshot(StoredSnapshot {
            snapshot: snapshot.clone(),
            changes,
            created_at: timestamp(),
        })
        .await?;
        self.authorize_review_viewer(
            &snapshot.session_id,
            &snapshot.viewer_id,
            &snapshot.binding_id,
        )
        .await?;
        let mut entries = self
            .snapshot_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        entries.retain(|entry| entry.key != cache_key);
        entries.push_front(ReviewCacheEntry {
            key: cache_key,
            revisions,
            snapshot: snapshot.clone(),
        });
        entries.truncate(MAX_SNAPSHOTS);
        Ok(snapshot)
    }

    async fn reusable_snapshot(
        &self,
        entry: &ReviewCacheEntry,
        session_id: &str,
        pane_id: &str,
        request: &ReviewSnapshotRequest,
        repository: &RepositoryCandidate,
        source_id: &str,
        base_revision: Option<&str>,
        revisions: &RevisionTokens,
    ) -> Result<Option<ReviewSnapshot>, InspectionError> {
        self.authorize_review_viewer(session_id, pane_id, &request.binding_id)
            .await?;
        let Ok(Some(stored)) = self.load_snapshot(&entry.snapshot.review_id).await else {
            return Ok(None);
        };
        let snapshot = stored.snapshot;
        // Untracked tokens hold metadata only (path, length, mtime, inode), so a same-size rewrite that keeps its
        // mtime is invisible to them. Snapshots that contain untracked files are therefore rebuilt, never reused.
        let has_untracked = snapshot.files.iter().any(|file| {
            file.status == ReviewFileStatus::Untracked
                || file.comparison == ReviewComparison::Untracked
        });
        if snapshot.review_id == entry.snapshot.review_id
            && snapshot.binding_id == request.binding_id
            && snapshot.session_id == session_id
            && snapshot.viewer_id == pane_id
            && snapshot.generation == entry.snapshot.generation
            && snapshot.repository_id == repository.repository_id
            && snapshot.checkout_path == repository.checkout_path
            && snapshot.source_id == source_id
            && snapshot.comparison == request.comparison
            && snapshot.base_revision.as_deref() == base_revision
            && !has_untracked
            && snapshot.head_revision == revisions.head
            && snapshot.index_revision == revisions.index
            && snapshot.worktree_revision == revisions.worktree
        {
            self.authorize_review_viewer(session_id, pane_id, &request.binding_id)
                .await?;
            return Ok(Some(snapshot));
        }
        Ok(None)
    }

    /// Obtain fresh viewer and pinned checkout proof for a comment batch.
    pub(crate) async fn comment_evidence(
        &self,
        session_id: &str,
        pane_id: &str,
        binding_id: &str,
    ) -> Result<ReviewCommentEvidence, InspectionError> {
        let authorization = self
            .authorize_review_viewer(session_id, pane_id, binding_id)
            .await?;
        self.comment_evidence_for_authorization(&authorization)
            .await
    }

    pub(crate) async fn comment_evidence_for_authorization(
        &self,
        authorization: &ViewerAuthorization,
    ) -> Result<ReviewCommentEvidence, InspectionError> {
        let context = &authorization.context;
        if context.kind != ViewerKind::Review {
            return Err(InspectionError::new(
                "review_snapshot_mismatch",
                "viewer is not a Review viewer",
            ));
        }
        let root = context
            .default_root_id
            .as_deref()
            .and_then(|id| {
                context.roots.iter().find(|root| {
                    root.root_id == id
                        && root.kind == cockpit_protocol::context::ContextRootKind::Repository
                })
            })
            .ok_or_else(|| {
                InspectionError::new(
                    "review_checkout_mismatch",
                    "Review viewer has no authorized repository checkout",
                )
            })?;
        let checkout_path = PathBuf::from(&root.checkout_path);
        let source_path = verified_checkout_path(
            &authorization.source.cwd,
            &authorization.source.foreground_cwd,
        )?;
        if !source_path.starts_with(&checkout_path) {
            return Err(InspectionError::new(
                "review_checkout_mismatch",
                "pinned source directory differs from the authorized checkout",
            ));
        }
        let source_id = checkout_source_id(&checkout_path)?;
        if source_id != context.source_id {
            return Err(InspectionError::new(
                "context_root_not_authorized",
                "Review checkout identity differs from the pinned viewer source",
            ));
        }
        Ok(ReviewCommentEvidence {
            binding_id: context.binding_id.clone(),
            session_id: context.session_id.clone(),
            viewer_id: context.viewer_id.clone(),
            server_instance: authorization.server_instance.clone(),
            terminal_id: authorization.source.terminal_id.clone(),
            workspace_id: context.space_id.clone(),
            tab_id: context.tab_id.clone(),
            checkout_path: checkout_path.to_string_lossy().into_owned(),
            repository_id: root.repository_id.clone(),
            source_id,
        })
    }

    pub(super) async fn active_snapshot(
        &self,
        session_id: &str,
        pane_id: &str,
        binding_id: &str,
        review_id: &str,
        generation: u32,
    ) -> Result<StoredSnapshot, InspectionError> {
        let stored = self.load_snapshot(review_id).await?.ok_or_else(|| {
            InspectionError::new(
                "review_snapshot_not_found",
                "review snapshot is no longer retained",
            )
        })?;
        let fresh = self
            .comment_evidence(session_id, pane_id, binding_id)
            .await?;
        let snapshot = &stored.snapshot;
        if snapshot.binding_id != binding_id
            || snapshot.session_id != session_id
            || snapshot.viewer_id != pane_id
        {
            return Err(InspectionError::new(
                "review_snapshot_mismatch",
                "review snapshot belongs to another viewer identity",
            ));
        }
        if fresh.binding_id != snapshot.binding_id || snapshot.generation != generation {
            return Err(InspectionError::new(
                "stale_generation",
                "review snapshot generation is no longer current",
            ));
        }
        if snapshot.repository_id != fresh.repository_id
            || snapshot.checkout_path != fresh.checkout_path
            || snapshot.source_id != fresh.source_id
        {
            return Err(InspectionError::new(
                "review_checkout_mismatch",
                "review checkout differs from the authorized viewer root",
            ));
        }
        Ok(stored)
    }

    pub(super) async fn active_snapshot_with_evidence(
        &self,
        evidence: &ReviewCommentEvidence,
        review_id: &str,
        generation: u32,
    ) -> Result<StoredSnapshot, InspectionError> {
        let stored = self.load_snapshot(review_id).await?.ok_or_else(|| {
            InspectionError::new(
                "review_snapshot_not_found",
                "review snapshot is no longer retained",
            )
        })?;
        let snapshot = &stored.snapshot;
        if snapshot.binding_id != evidence.binding_id
            || snapshot.session_id != evidence.session_id
            || snapshot.viewer_id != evidence.viewer_id
        {
            return Err(InspectionError::new(
                "review_snapshot_mismatch",
                "review snapshot belongs to another viewer identity",
            ));
        }
        if snapshot.generation != generation {
            return Err(InspectionError::new(
                "stale_generation",
                "review snapshot generation is no longer current",
            ));
        }
        let checkout = PathBuf::from(&snapshot.checkout_path);
        if snapshot.repository_id != evidence.repository_id
            || checkout_source_id(&checkout)? != evidence.source_id
        {
            return Err(InspectionError::new(
                "review_checkout_mismatch",
                "Reviewr checkout was replaced or changed since this snapshot",
            ));
        }
        Ok(stored)
    }

    pub(super) async fn authorize_review_viewer(
        &self,
        session_id: &str,
        pane_id: &str,
        binding_id: &str,
    ) -> Result<ViewerAuthorization, InspectionError> {
        let authorization = self
            .context
            .authorize_viewer(session_id, pane_id, binding_id)
            .await?;
        if authorization.context.kind != ViewerKind::Review {
            return Err(InspectionError::new(
                "review_snapshot_mismatch",
                "viewer is not a Review viewer",
            ));
        }
        Ok(authorization)
    }

    pub(super) async fn resolve_checkout(
        &self,
        cwd: &Option<String>,
        foreground_cwd: &Option<String>,
        repository_id: &str,
    ) -> Result<RepositoryCandidate, InspectionError> {
        let candidate = self.discover_checkout(cwd, foreground_cwd).await?;
        if candidate.repository_id != repository_id {
            return Err(InspectionError::new(
                "review_checkout_mismatch",
                "Reviewr checkout differs from the selected repository",
            ));
        }
        Ok(candidate)
    }

    pub(super) async fn discover_checkout(
        &self,
        cwd: &Option<String>,
        foreground_cwd: &Option<String>,
    ) -> Result<RepositoryCandidate, InspectionError> {
        let pane_canonical = verified_checkout_path(cwd, foreground_cwd)?;
        self.context.cached_discover_checkout(&pane_canonical).await
    }

    pub(super) async fn collect(
        &self,
        checkout: &Path,
        request: &ReviewSnapshotRequest,
        revisions: &RevisionTokens,
        base_revision: Option<String>,
    ) -> Result<
        (
            Vec<ReviewChangedFile>,
            BTreeMap<String, Change>,
            Vec<ProjectDiagnostic>,
            bool,
            Option<String>,
        ),
        InspectionError,
    > {
        let comparisons = match request.comparison {
            ReviewComparison::AllLocal => vec![
                ReviewComparison::Staged,
                ReviewComparison::Unstaged,
                ReviewComparison::Untracked,
            ],
            value => vec![value],
        };
        let mut files = Vec::new();
        let mut changes_by_id = BTreeMap::new();
        let mut diagnostics = Vec::new();
        let truncated = false;
        for comparison in comparisons {
            let changes = self
                .changes(checkout, comparison, base_revision.as_deref())
                .await?;
            let counts = self
                .change_statistics(checkout, comparison, base_revision.as_deref())
                .await?;
            for mut change in changes {
                capture_inventory_revisions(checkout, &mut change, revisions)?;
                let file_id = file_id(comparison, &change.old_path, &change.new_path);
                let mut file = change_file(&file_id, &change, revisions, base_revision.as_deref());
                let path = change.new_path.as_deref().or(change.old_path.as_deref());
                if let Some(statistics) = path.and_then(|path| counts.get(path)) {
                    file.additions = statistics.additions;
                    file.deletions = statistics.deletions;
                    file.binary = statistics.binary;
                } else if comparison == ReviewComparison::Untracked {
                    if let Some(source) =
                        path.and_then(|path| read_worktree_source(checkout, path).ok())
                    {
                        file.additions = source.total_lines;
                        file.deletions = source.total_lines.map(|_| 0);
                    }
                }
                files.push(file);
                changes_by_id.insert(file_id, change);
            }
        }
        // The inventory is bounded by the Git command output cap, but unlike
        // the former 256-row guard it never silently drops a changed file.
        if serde_json::to_vec(&files)
            .map(|value| value.len() > MAX_FILE_LIST_BYTES)
            .unwrap_or(true)
        {
            diagnostics.push(diagnostic(
                "review_files_bounded",
                "review file inventory exceeds the bounded response size; narrow the comparison",
                None,
            ));
            return Err(InspectionError::new(
                "review_files_bounded",
                "review file inventory exceeds the bounded response size",
            ));
        }
        Ok((files, changes_by_id, diagnostics, truncated, base_revision))
    }
}

fn validate_snapshot_request(request: &ReviewSnapshotRequest) -> Result<(), InspectionError> {
    if request.binding_id.is_empty()
        || request.repository_id.is_empty()
        || request.repository_id.len() > 256
    {
        return Err(InspectionError::new(
            "review_invalid_request",
            "review request is incomplete",
        ));
    }
    if request.comparison == ReviewComparison::Branch
        && request
            .base_ref
            .as_deref()
            .filter(|value| {
                !value.is_empty() && !value.starts_with('-') && !value.contains(char::is_whitespace)
            })
            .is_none()
    {
        return Err(InspectionError::new(
            "review_invalid_base",
            "branch comparison requires a bounded base ref",
        ));
    }
    if request.comparison != ReviewComparison::Branch && request.base_ref.is_some() {
        return Err(InspectionError::new(
            "review_invalid_base",
            "base ref is valid only for branch comparison",
        ));
    }
    Ok(())
}

pub(super) fn file_id(
    comparison: ReviewComparison,
    old_path: &Option<String>,
    new_path: &Option<String>,
) -> String {
    let tuple =
        serde_json::to_vec(&(comparison, old_path, new_path)).expect("review identity serializes");
    format!("file-{:x}", Sha256::digest(tuple))
}

pub(super) fn change_file(
    file_id: &str,
    change: &Change,
    revisions: &RevisionTokens,
    base: Option<&str>,
) -> ReviewChangedFile {
    let old_revision = match change.comparison {
        ReviewComparison::Staged => revisions.head.clone(),
        ReviewComparison::Branch => base.map(str::to_owned),
        ReviewComparison::Unstaged => Some(revisions.index.clone()),
        ReviewComparison::Untracked | ReviewComparison::AllLocal => None,
    };
    let new_revision = match change.comparison {
        ReviewComparison::Staged => Some(revisions.index.clone()),
        ReviewComparison::Branch => revisions.head.clone(),
        ReviewComparison::Unstaged | ReviewComparison::Untracked => {
            Some(revisions.worktree.clone())
        }
        ReviewComparison::AllLocal => None,
    };
    ReviewChangedFile {
        file_id: file_id.to_owned(),
        comparison: change.comparison,
        status: change.status,
        old_path: change.old_path.clone(),
        new_path: change.new_path.clone(),
        binary: false,
        additions: None,
        deletions: None,
        summary: summary(change),
        old_revision,
        new_revision,
    }
}

fn capture_inventory_revisions(
    checkout: &Path,
    change: &mut Change,
    revisions: &RevisionTokens,
) -> Result<(), InspectionError> {
    change.old_revision = match change.comparison {
        ReviewComparison::Staged => revisions.head.clone(),
        ReviewComparison::Unstaged => Some(revisions.index.clone()),
        ReviewComparison::Branch => None,
        ReviewComparison::Untracked | ReviewComparison::AllLocal => None,
    };
    change.new_revision = match change.comparison {
        ReviewComparison::Staged => Some(revisions.index.clone()),
        ReviewComparison::Branch => revisions.head.clone(),
        ReviewComparison::Unstaged | ReviewComparison::Untracked => {
            if let Some(path) = change.new_path.as_deref() {
                match worktree_source_identity(checkout, path) {
                    Some(identity) => Some(identity?),
                    None => None,
                }
            } else {
                None
            }
        }
        ReviewComparison::AllLocal => None,
    };
    Ok(())
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
