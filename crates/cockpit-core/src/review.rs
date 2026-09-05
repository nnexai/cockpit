use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::context::{DetectionConfidence, ExtensionKind};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectDiagnostic, RepositoryCandidate};
use cockpit_protocol::review::{
    ReviewChangedFile, ReviewComparison, ReviewDiffLine, ReviewDiffLineKind, ReviewFileDiff,
    ReviewFileRequest, ReviewFileStatus, ReviewHunk, ReviewSide, ReviewSnapshot,
    ReviewSnapshotRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use uuid::Uuid;

use crate::context::ContextService;
use crate::extension_adapter::ExtensionHerdrAdapter;
use crate::project_store::{ProjectStore, atomic_write_json, read_json_bounded, timestamp};
use crate::repositories::RepositoryCatalog;
use crate::{InspectionError, process::run_bounded_command};

const MAX_SNAPSHOTS: usize = 8;
const MAX_FILES: usize = 256;
const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 512 * 1024;
const MAX_HUNKS: usize = 2048;
const MAX_STORED_SNAPSHOT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SNAPSHOT_ENTRIES: usize = MAX_SNAPSHOTS * 4;

#[derive(Clone)]
pub struct ReviewService {
    configuration: ProjectConfiguration,
    adapter: Arc<dyn ExtensionHerdrAdapter>,
    context: Arc<ContextService>,
    store: ProjectStore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSnapshot {
    snapshot: ReviewSnapshot,
    diffs: BTreeMap<String, ReviewFileDiff>,
    created_at: String,
}

#[derive(Debug, Clone)]
struct Change {
    status: ReviewFileStatus,
    comparison: ReviewComparison,
    old_path: Option<String>,
    new_path: Option<String>,
}

/// Fresh runtime evidence for a Reviewr comment batch. `source_id` identifies
/// the actual checkout directory, rather than a mutable review snapshot.
#[derive(Debug, Clone)]
pub(crate) struct ReviewCommentEvidence {
    pub binding_id: String,
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

#[derive(Debug, Clone)]
struct FrozenSource {
    text: Option<String>,
    hash: Option<String>,
    total_lines: Option<u32>,
    truncated: bool,
    diagnostic: Option<ProjectDiagnostic>,
}

impl ReviewService {
    pub fn new(
        configuration: ProjectConfiguration,
        adapter: Arc<dyn ExtensionHerdrAdapter>,
        context: Arc<ContextService>,
    ) -> Result<Self, InspectionError> {
        let store = ProjectStore::new(Path::new(&configuration.state_root).join("review"))?;
        Ok(Self {
            configuration,
            adapter,
            context,
            store,
        })
    }

    pub async fn snapshot(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ReviewSnapshotRequest,
    ) -> Result<ReviewSnapshot, InspectionError> {
        validate_snapshot_request(request)?;
        let presentation = self
            .authorize_review_pane(session_id, pane_id, &request.binding_id)
            .await?;
        // ContextService owns verified extension classification. The adapter is
        // used only for the Reviewr cwd needed to select a registered Git worktree.
        let evidence = self
            .adapter
            .inspect_extension_pane(session_id, pane_id)
            .await?;
        if evidence.pane_id != pane_id
            || evidence.extension != Some(ExtensionKind::Review)
            || !verified(evidence.confidence)
        {
            return Err(InspectionError::new(
                "review_unavailable",
                "Reviewr process evidence changed while preparing review",
            ));
        }
        let confirmed = self
            .authorize_review_pane(session_id, pane_id, &request.binding_id)
            .await?;
        if confirmed.terminal_id != presentation.terminal_id {
            return Err(InspectionError::new(
                "review_unavailable",
                "Reviewr pane identity changed while preparing review",
            ));
        }
        let repository = self
            .resolve_checkout(
                &evidence.cwd,
                &evidence.foreground_cwd,
                &request.repository_id,
            )
            .await?;
        let checkout = PathBuf::from(&repository.checkout_path);
        let binding_id = request.binding_id.clone();

        for attempt in 0..2 {
            let before = self.revision_tokens(&checkout).await?;
            let (files, mut diffs, mut diagnostics, truncated, base_revision) =
                self.collect(&checkout, request, &before).await?;
            let after = self.revision_tokens(&checkout).await?;
            if before == after {
                let review_id = Uuid::new_v4().to_string();
                let generation = 1;
                for diff in diffs.values_mut() {
                    diff.review_id = review_id.clone();
                    diff.generation = generation;
                    diff.binding_id = binding_id.clone();
                    diff.session_id = session_id.to_owned();
                    diff.pane_id = pane_id.to_owned();
                }
                let snapshot = ReviewSnapshot {
                    binding_id,
                    session_id: session_id.to_owned(),
                    pane_id: pane_id.to_owned(),
                    review_id: review_id.clone(),
                    generation,
                    repository_id: repository.repository_id,
                    checkout_path: repository.checkout_path,
                    source_id: checkout_source_id(&checkout)?,
                    comparison: request.comparison,
                    base_revision,
                    head_revision: before.head,
                    index_revision: before.index,
                    worktree_revision: before.worktree,
                    files,
                    truncated,
                    diagnostics: std::mem::take(&mut diagnostics),
                };
                self.save_snapshot(StoredSnapshot {
                    snapshot: snapshot.clone(),
                    diffs,
                    created_at: timestamp(),
                })
                .await?;
                return Ok(snapshot);
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
        let stored = self
            .active_snapshot(
                session_id,
                pane_id,
                &request.binding_id,
                &request.review_id,
                request.generation,
            )
            .await?;
        stored.diffs.get(&request.file_id).cloned().ok_or_else(|| {
            InspectionError::new(
                "review_file_not_found",
                "review file is not in this immutable snapshot",
            )
        })
    }

    /// Obtain fresh pane and checkout proof for a Reviewr comment batch.
    /// The caller never supplies an endpoint, process, or checkout identity.
    pub(crate) async fn comment_evidence(
        &self,
        session_id: &str,
        pane_id: &str,
        binding_id: &str,
    ) -> Result<ReviewCommentEvidence, InspectionError> {
        let presentation = self
            .authorize_review_pane(session_id, pane_id, binding_id)
            .await?;
        let evidence = self
            .adapter
            .inspect_extension_pane(session_id, pane_id)
            .await?;
        if evidence.pane_id != pane_id
            || evidence.terminal_id != presentation.terminal_id
            || evidence.extension != Some(ExtensionKind::Review)
            || !verified(evidence.confidence)
        {
            return Err(InspectionError::new(
                "review_unavailable",
                "Reviewr process evidence changed while preparing comments",
            ));
        }
        let confirmed = self
            .authorize_review_pane(session_id, pane_id, binding_id)
            .await?;
        if confirmed.terminal_id != presentation.terminal_id {
            return Err(InspectionError::new(
                "review_unavailable",
                "Reviewr pane identity changed while preparing comments",
            ));
        }
        let repository = self
            .discover_checkout(&evidence.cwd, &evidence.foreground_cwd)
            .await?;
        let checkout_path = PathBuf::from(&repository.checkout_path);
        let source_id = checkout_source_id(&checkout_path)?;
        Ok(ReviewCommentEvidence {
            binding_id: presentation.binding_id,
            terminal_id: presentation.terminal_id,
            workspace_id: evidence.workspace_id,
            tab_id: evidence.tab_id,
            checkout_path: checkout_path.to_string_lossy().into_owned(),
            repository_id: repository.repository_id,
            source_id,
        })
    }

    /// Return a full immutable side document for generic comment capture.
    /// It intentionally does not consult Git or the worktree: the persisted
    /// review snapshot is the only source of comment text.
    pub(crate) async fn capture(
        &self,
        session_id: &str,
        pane_id: &str,
        binding_id: &str,
        review_id: &str,
        generation: u32,
        file_id: &str,
        side: ReviewSide,
    ) -> Result<ReviewSourceDocument, InspectionError> {
        let evidence = self
            .comment_evidence(session_id, pane_id, binding_id)
            .await?;
        let stored = self
            .active_snapshot(session_id, pane_id, binding_id, review_id, generation)
            .await?;
        let snapshot_checkout = PathBuf::from(&stored.snapshot.checkout_path);
        if stored.snapshot.repository_id != evidence.repository_id
            || checkout_source_id(&snapshot_checkout)? != evidence.source_id
        {
            return Err(InspectionError::new(
                "review_checkout_mismatch",
                "Reviewr checkout was replaced or changed since this snapshot",
            ));
        }
        let diff = stored.diffs.get(file_id).ok_or_else(|| {
            InspectionError::new(
                "review_file_not_found",
                "review file is not in this immutable snapshot",
            )
        })?;
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
            source_id: evidence.source_id,
            path: path.to_owned(),
            revision: revision.to_owned(),
            content_hash: content_hash.to_owned(),
            text: text.to_owned(),
        })
    }

    /// Re-read only bounded Git revision evidence under the same verified
    /// checkout. Callers can cache the result per review id while refreshing a
    /// comment batch; no remapping or snapshot replacement occurs here.
    pub(crate) async fn source_state(
        &self,
        session_id: &str,
        pane_id: &str,
        binding_id: &str,
        review_id: &str,
        generation: u32,
        file_id: &str,
        side: ReviewSide,
    ) -> Result<ReviewSourceState, InspectionError> {
        let evidence = self
            .comment_evidence(session_id, pane_id, binding_id)
            .await?;
        let stored = self
            .active_snapshot(session_id, pane_id, binding_id, review_id, generation)
            .await?;
        let checkout = PathBuf::from(&stored.snapshot.checkout_path);
        if stored.snapshot.repository_id != evidence.repository_id
            || checkout_source_id(&checkout)? != evidence.source_id
        {
            return Err(InspectionError::new(
                "review_checkout_mismatch",
                "Reviewr checkout was replaced or changed since this snapshot",
            ));
        }
        let diff = stored.diffs.get(file_id).ok_or_else(|| {
            InspectionError::new("review_file_not_found", "review file is unavailable")
        })?;
        let change = Change {
            status: diff.file.status,
            comparison: diff.file.comparison,
            old_path: diff.file.old_path.clone(),
            new_path: diff.file.new_path.clone(),
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
                stored.snapshot.base_revision.as_deref(),
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

    async fn active_snapshot(
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
            .authorize_review_pane(session_id, pane_id, binding_id)
            .await?;
        let snapshot = &stored.snapshot;
        if snapshot.binding_id != binding_id
            || snapshot.session_id != session_id
            || snapshot.pane_id != pane_id
        {
            return Err(InspectionError::new(
                "review_snapshot_mismatch",
                "review snapshot belongs to another pane identity",
            ));
        }
        if fresh.binding_id != snapshot.binding_id || snapshot.generation != generation {
            return Err(InspectionError::new(
                "stale_generation",
                "review snapshot generation is no longer current",
            ));
        }
        Ok(stored)
    }

    async fn authorize_review_pane(
        &self,
        session_id: &str,
        pane_id: &str,
        binding_id: &str,
    ) -> Result<cockpit_protocol::context::PanePresentation, InspectionError> {
        let fresh = self.context.inspect_pane(session_id, pane_id).await?;
        if fresh.binding_id != binding_id
            || fresh.session_id != session_id
            || fresh.pane_id != pane_id
            || fresh.extension != Some(ExtensionKind::Review)
            || fresh.renderer != Some(ExtensionKind::Review)
        {
            return Err(InspectionError::new(
                "review_snapshot_mismatch",
                "Reviewr pane identity is no longer current",
            ));
        }
        Ok(fresh)
    }

    async fn resolve_checkout(
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

    async fn discover_checkout(
        &self,
        cwd: &Option<String>,
        foreground_cwd: &Option<String>,
    ) -> Result<RepositoryCandidate, InspectionError> {
        let pane_canonical = verified_checkout_path(cwd, foreground_cwd)?;
        RepositoryCatalog::new(self.configuration.clone())
            .discover_checkout(&pane_canonical)
            .await
    }

    async fn collect(
        &self,
        checkout: &Path,
        request: &ReviewSnapshotRequest,
        revisions: &RevisionTokens,
    ) -> Result<
        (
            Vec<ReviewChangedFile>,
            BTreeMap<String, ReviewFileDiff>,
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
        let base_revision = if request.comparison == ReviewComparison::Branch {
            Some(
                self.git_text(
                    checkout,
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
        let mut files = Vec::new();
        let mut diffs = BTreeMap::new();
        let mut diagnostics = Vec::new();
        let mut truncated = false;
        let mut stored_bytes = 0usize;
        for comparison in comparisons {
            let changes = self
                .changes(checkout, comparison, base_revision.as_deref())
                .await?;
            for change in changes {
                if files.len() >= MAX_FILES {
                    truncated = true;
                    diagnostics.push(diagnostic(
                        "review_files_bounded",
                        "review file list reached its limit",
                        None,
                    ));
                    break;
                }
                let file_id = file_id(comparison, &change.old_path, &change.new_path);
                let (mut diff, file_truncated) = match self
                    .diff(
                        checkout,
                        &file_id,
                        &change,
                        base_revision.as_deref(),
                        revisions,
                    )
                    .await
                {
                    Ok(value) => value,
                    Err(error) if error.code == "bounded_output" => (
                        truncated_diff(
                            &file_id,
                            &change,
                            base_revision.as_deref(),
                            revisions,
                            "review_diff_bounded",
                            "file diff exceeds the configured read limit",
                        ),
                        true,
                    ),
                    Err(error) if error.code == "review_unreadable" => (
                        truncated_diff(
                            &file_id,
                            &change,
                            base_revision.as_deref(),
                            revisions,
                            "review_unreadable",
                            "file is unavailable or nonregular",
                        ),
                        false,
                    ),
                    Err(error) => return Err(error),
                };
                let encoded = serde_json::to_vec(&diff)
                    .map_err(|error| InspectionError::new("review_write", error.to_string()))?;
                if stored_bytes.saturating_add(encoded.len()) > MAX_STORED_SNAPSHOT_BYTES as usize {
                    discard_snapshot_payload(&mut diff);
                    truncated = true;
                }
                let encoded = serde_json::to_vec(&diff)
                    .map_err(|error| InspectionError::new("review_write", error.to_string()))?;
                if stored_bytes.saturating_add(encoded.len()) > MAX_STORED_SNAPSHOT_BYTES as usize {
                    return Err(InspectionError::new(
                        "review_snapshot_bounded",
                        "review snapshot metadata exceeded its aggregate limit",
                    ));
                }
                stored_bytes += encoded.len();
                truncated |= file_truncated;
                files.push(diff.file.clone());
                diffs.insert(file_id, diff);
            }
        }
        Ok((files, diffs, diagnostics, truncated, base_revision))
    }

    async fn changes(
        &self,
        checkout: &Path,
        comparison: ReviewComparison,
        base: Option<&str>,
    ) -> Result<Vec<Change>, InspectionError> {
        if comparison == ReviewComparison::Untracked {
            let output = self
                .git(
                    checkout,
                    &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
                )
                .await?;
            return parse_untracked(&output.stdout);
        }
        let mut args = vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--name-status",
            "-z",
            "--find-renames=50%",
        ];
        match comparison {
            ReviewComparison::Staged => args.push("--cached"),
            ReviewComparison::Branch => {
                args.push(base.expect("branch base"));
                args.push("HEAD");
            }
            ReviewComparison::Unstaged => {}
            _ => unreachable!(),
        }
        args.push("--");
        let output = self.git(checkout, &args).await?;
        parse_name_status(&output.stdout, comparison)
    }

    async fn diff(
        &self,
        checkout: &Path,
        file_id: &str,
        change: &Change,
        base: Option<&str>,
        revisions: &RevisionTokens,
    ) -> Result<(ReviewFileDiff, bool), InspectionError> {
        let path = change
            .new_path
            .as_deref()
            .or(change.old_path.as_deref())
            .ok_or_else(|| {
                InspectionError::new("review_invalid_path", "review change omitted both paths")
            })?;
        let mut diagnostics = Vec::new();
        let (mut hunks, binary, truncated, status) = if change.comparison
            == ReviewComparison::Untracked
        {
            let (hunks, binary, truncated) = untracked_hunk(checkout, path)?;
            (hunks, binary, truncated, change.status)
        } else {
            let mut args = vec![
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--unified=3",
                "--find-renames=50%",
            ];
            match change.comparison {
                ReviewComparison::Staged => args.push("--cached"),
                ReviewComparison::Branch => {
                    args.push(base.expect("branch base"));
                    args.push("HEAD");
                }
                ReviewComparison::Unstaged => {}
                _ => unreachable!(),
            }
            args.push("--");
            // Git pathspec magic must not reinterpret a repository filename.
            let literal_path = format!(":(literal){path}");
            args.push(&literal_path);
            let output = self.git_with_limit(checkout, &args, MAX_DIFF_BYTES).await?;
            let text = String::from_utf8(output.stdout).map_err(|_| {
                InspectionError::new("review_binary_diff", "Git returned non-textual diff output")
            })?;
            let (hunks, binary, truncated) = parse_unified(&text, change, MAX_HUNKS)?;
            let status = if text.lines().any(|line| {
                (line.starts_with("index ") && line.ends_with(" 160000"))
                    || line == "new file mode 160000"
                    || line == "deleted file mode 160000"
            }) {
                ReviewFileStatus::Submodule
            } else if text.contains("old mode ") && text.contains("new mode ") && hunks.is_empty() {
                ReviewFileStatus::ModeOnly
            } else {
                change.status
            };
            (hunks, binary, truncated, status)
        };
        if matches!(
            status,
            ReviewFileStatus::ModeOnly | ReviewFileStatus::Submodule
        ) {
            hunks.clear();
            diagnostics.push(diagnostic(
                "review_non_anchorable",
                "mode-only and submodule changes have no textual source anchors",
                Some(path),
            ));
        }
        if binary {
            diagnostics.push(diagnostic(
                "review_binary",
                "binary content has no text anchors",
                Some(path),
            ));
        }
        let old_revision = match change.comparison {
            ReviewComparison::Staged => revisions.head.clone(),
            ReviewComparison::Branch => base.map(str::to_owned),
            ReviewComparison::Unstaged => Some(revisions.index.clone()),
            ReviewComparison::Untracked => None,
            ReviewComparison::AllLocal => None,
        };
        let new_revision = match change.comparison {
            ReviewComparison::Staged => Some(revisions.index.clone()),
            ReviewComparison::Branch => revisions.head.clone(),
            ReviewComparison::Unstaged | ReviewComparison::Untracked => {
                Some(revisions.worktree.clone())
            }
            ReviewComparison::AllLocal => None,
        };
        let source_anchorable = !binary
            && !matches!(
                status,
                ReviewFileStatus::ModeOnly | ReviewFileStatus::Submodule
            );
        let old_source = if source_anchorable {
            self.snapshot_source(checkout, change, base, revisions, ReviewSide::Old)
                .await?
        } else {
            FrozenSource::absent()
        };
        let new_source = if source_anchorable {
            self.snapshot_source(checkout, change, base, revisions, ReviewSide::New)
                .await?
        } else {
            FrozenSource::absent()
        };
        if let Some(diagnostic) = old_source.diagnostic.clone() {
            diagnostics.push(diagnostic);
        }
        if let Some(diagnostic) = new_source.diagnostic.clone() {
            diagnostics.push(diagnostic);
        }
        let source_truncated = old_source.truncated || new_source.truncated;
        let file = ReviewChangedFile {
            file_id: file_id.to_owned(),
            comparison: change.comparison,
            status: if binary {
                ReviewFileStatus::Binary
            } else {
                status
            },
            old_path: change.old_path.clone(),
            new_path: change.new_path.clone(),
            binary,
            summary: summary(change),
            old_revision,
            new_revision,
        };
        Ok((
            ReviewFileDiff {
                binding_id: String::new(),
                session_id: String::new(),
                pane_id: String::new(),
                review_id: String::new(),
                generation: 0,
                file,
                hunks,
                old_source: old_source.text,
                new_source: new_source.text,
                old_source_hash: old_source.hash,
                new_source_hash: new_source.hash,
                old_total_lines: old_source.total_lines,
                new_total_lines: new_source.total_lines,
                old_source_truncated: old_source.truncated,
                new_source_truncated: new_source.truncated,
                truncated: truncated || source_truncated,
                diagnostics,
            },
            truncated || source_truncated,
        ))
    }

    async fn snapshot_source(
        &self,
        checkout: &Path,
        change: &Change,
        base: Option<&str>,
        revisions: &RevisionTokens,
        side: ReviewSide,
    ) -> Result<FrozenSource, InspectionError> {
        let (path, object) = match (change.comparison, side) {
            (_, ReviewSide::Old) if change.old_path.is_none() => return Ok(FrozenSource::absent()),
            (_, ReviewSide::New) if change.new_path.is_none() => return Ok(FrozenSource::absent()),
            (ReviewComparison::Staged, ReviewSide::Old) => match revisions.head.as_deref() {
                Some(head) => (
                    change.old_path.as_deref().expect("path checked"),
                    SourceObject::Git(head),
                ),
                None => return Ok(FrozenSource::absent()),
            },
            (ReviewComparison::Staged, ReviewSide::New) => (
                change.new_path.as_deref().expect("path checked"),
                SourceObject::Index,
            ),
            (ReviewComparison::Unstaged, ReviewSide::Old) => (
                change.old_path.as_deref().expect("path checked"),
                SourceObject::Index,
            ),
            (ReviewComparison::Unstaged, ReviewSide::New) => (
                change.new_path.as_deref().expect("path checked"),
                SourceObject::Worktree,
            ),
            (ReviewComparison::Branch, ReviewSide::Old) => (
                change.old_path.as_deref().expect("path checked"),
                SourceObject::Git(base.expect("branch base")),
            ),
            (ReviewComparison::Branch, ReviewSide::New) => match revisions.head.as_deref() {
                Some(head) => (
                    change.new_path.as_deref().expect("path checked"),
                    SourceObject::Git(head),
                ),
                None => return Ok(FrozenSource::absent()),
            },
            (ReviewComparison::Untracked, ReviewSide::New) => (
                change.new_path.as_deref().expect("path checked"),
                SourceObject::Worktree,
            ),
            (ReviewComparison::Untracked, ReviewSide::Old) | (ReviewComparison::AllLocal, _) => {
                return Ok(FrozenSource::absent());
            }
        };
        match object {
            SourceObject::Git(revision) => self.git_source(checkout, revision, path).await,
            SourceObject::Index => self.git_source(checkout, ":", path).await,
            SourceObject::Worktree => {
                let checkout = checkout.to_path_buf();
                let path = path.to_owned();
                tokio::task::spawn_blocking(move || read_worktree_source(&checkout, &path))
                    .await
                    .map_err(|error| InspectionError::new("review_task", error.to_string()))?
            }
        }
    }

    async fn git_source(
        &self,
        checkout: &Path,
        revision: &str,
        path: &str,
    ) -> Result<FrozenSource, InspectionError> {
        let object = if revision == ":" {
            format!(":{path}")
        } else {
            format!("{revision}:{path}")
        };
        match self
            .git_with_limit(
                checkout,
                &[
                    "show",
                    "--no-textconv",
                    "--format=",
                    "--end-of-options",
                    &object,
                ],
                MAX_FILE_BYTES,
            )
            .await
        {
            Ok(output) if output.status.success() => Ok(FrozenSource::from_bytes(output.stdout)),
            Ok(_) => Ok(FrozenSource::unavailable(
                path,
                "Git could not read the immutable source for this review side",
            )),
            Err(error) if error.code == "bounded_output" => Ok(FrozenSource::truncated(path)),
            Err(error) => Err(error),
        }
    }

    async fn revision_tokens(&self, checkout: &Path) -> Result<RevisionTokens, InspectionError> {
        let head = self
            .git_text_optional(checkout, &["rev-parse", "--verify", "HEAD"])
            .await?;
        // `write-tree` would create an object in .git/objects. A review is
        // read-only, so retain a bounded digest of index entries instead.
        let index_entries = self.git(checkout, &["ls-files", "--stage", "-z"]).await?;
        let index = format!("sha256:{:x}", Sha256::digest(&index_entries.stdout));
        let unstaged = self
            .git(
                checkout,
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--binary",
                    "--no-color",
                    "--",
                ],
            )
            .await?;
        let untracked = self
            .git(
                checkout,
                &["ls-files", "--others", "--exclude-standard", "-z"],
            )
            .await?;
        let worktree = worktree_token(checkout, &unstaged.stdout, &untracked.stdout)?;
        Ok(RevisionTokens {
            head,
            index,
            worktree,
        })
    }

    async fn git_text(&self, checkout: &Path, args: &[&str]) -> Result<String, InspectionError> {
        let output = self.git(checkout, args).await?;
        if !output.status.success() {
            return Err(InspectionError::new(
                "review_git",
                "Git could not resolve the requested immutable revision",
            ));
        }
        String::from_utf8(output.stdout)
            .map(|text| text.trim().to_owned())
            .map_err(|_| InspectionError::new("review_git", "Git emitted invalid text"))
    }

    async fn git_text_optional(
        &self,
        checkout: &Path,
        args: &[&str],
    ) -> Result<Option<String>, InspectionError> {
        let output = self.git(checkout, args).await?;
        if !output.status.success() {
            return Ok(None);
        }
        String::from_utf8(output.stdout)
            .map(|text| Some(text.trim().to_owned()))
            .map_err(|_| InspectionError::new("review_git", "Git emitted invalid text"))
    }

    async fn git(
        &self,
        checkout: &Path,
        args: &[&str],
    ) -> Result<std::process::Output, InspectionError> {
        self.git_with_limit(
            checkout,
            args,
            self.configuration.limits.git_output_bytes as usize,
        )
        .await
    }

    async fn git_with_limit(
        &self,
        checkout: &Path,
        args: &[&str],
        limit: usize,
    ) -> Result<std::process::Output, InspectionError> {
        let mut command = Command::new("git");
        command
            .current_dir(checkout)
            .arg("-c")
            .arg("core.hooksPath=/dev/null")
            .arg("-c")
            .arg("core.fsmonitor=false")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "")
            .args(args);
        run_bounded_command(
            command,
            limit,
            limit,
            Duration::from_millis(self.configuration.limits.git_timeout_ms as u64),
            "review git",
        )
        .await
    }

    async fn save_snapshot(&self, stored: StoredSnapshot) -> Result<(), InspectionError> {
        let state = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let name = snapshot_name(&stored.snapshot.review_id)?;
            let serialized = serde_json::to_vec(&stored)
                .map_err(|error| InspectionError::new("review_write", error.to_string()))?;
            if serialized.len() > MAX_STORED_SNAPSHOT_BYTES as usize {
                return Err(InspectionError::new(
                    "review_snapshot_bounded",
                    "review snapshot exceeds its aggregate storage limit",
                ));
            }
            atomic_write_json(state.state_dir(), &name, &stored)
                .map_err(|error| InspectionError::new("review_write", error.to_string()))?;
            prune_snapshots(&state)
        })
        .await
        .map_err(|error| InspectionError::new("review_task", error.to_string()))?
    }

    async fn load_snapshot(
        &self,
        review_id: &str,
    ) -> Result<Option<StoredSnapshot>, InspectionError> {
        let review_id = review_id.to_owned();
        let state = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let name = snapshot_name(&review_id)?;
            match state.state_dir().symlink_metadata(&name) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    read_json_bounded(state.state_dir(), &name, MAX_STORED_SNAPSHOT_BYTES).map(Some)
                }
                Ok(_) => Err(InspectionError::new(
                    "unsafe_path",
                    "review snapshot is not a regular file",
                )),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(InspectionError::new("review_read", error.to_string())),
            }
        })
        .await
        .map_err(|error| InspectionError::new("review_task", error.to_string()))?
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RevisionTokens {
    head: Option<String>,
    index: String,
    worktree: String,
}

#[derive(Debug, Clone, Copy)]
enum SourceObject<'a> {
    Git(&'a str),
    Index,
    Worktree,
}

impl FrozenSource {
    fn absent() -> Self {
        Self {
            text: None,
            hash: None,
            total_lines: None,
            truncated: false,
            diagnostic: None,
        }
    }

    fn from_bytes(bytes: Vec<u8>) -> Self {
        let hash = Some(hash_bytes(&bytes));
        match String::from_utf8(bytes) {
            Ok(text) => Self {
                total_lines: Some(physical_line_count(&text)),
                text: Some(text),
                hash,
                truncated: false,
                diagnostic: None,
            },
            Err(_) => Self {
                text: None,
                hash,
                total_lines: None,
                truncated: false,
                diagnostic: Some(diagnostic(
                    "review_source_binary",
                    "review source is not UTF-8 text",
                    None,
                )),
            },
        }
    }

    fn truncated(path: &str) -> Self {
        Self {
            text: None,
            hash: None,
            total_lines: None,
            truncated: true,
            diagnostic: Some(diagnostic(
                "review_source_bounded",
                "review source exceeds the immutable source limit",
                Some(path),
            )),
        }
    }

    fn unavailable(path: &str, message: &str) -> Self {
        Self {
            text: None,
            hash: None,
            total_lines: None,
            truncated: false,
            diagnostic: Some(diagnostic("review_source_unreadable", message, Some(path))),
        }
    }
}

fn physical_line_count(text: &str) -> u32 {
    u32::try_from(text.split_inclusive('\n').count()).unwrap_or(u32::MAX)
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn verified_checkout_path(
    cwd: &Option<String>,
    foreground_cwd: &Option<String>,
) -> Result<PathBuf, InspectionError> {
    let pane_cwd = foreground_cwd.as_ref().or(cwd.as_ref()).ok_or_else(|| {
        InspectionError::new(
            "review_unavailable",
            "verified Reviewr pane did not report a checkout directory",
        )
    })?;
    let pane_path = PathBuf::from(pane_cwd);
    if !pane_path.is_absolute() {
        return Err(InspectionError::new(
            "review_checkout_mismatch",
            "Reviewr checkout path is not absolute",
        ));
    }
    Ok(pane_path)
}

fn checkout_source_id(checkout: &Path) -> Result<String, InspectionError> {
    let metadata = std::fs::symlink_metadata(checkout).map_err(|_| {
        InspectionError::new(
            "review_unavailable",
            "review checkout directory is unavailable",
        )
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(InspectionError::new(
            "review_checkout_mismatch",
            "review checkout identity is not a direct directory",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"cockpit-review-checkout-v1\0");
    hasher.update(checkout.to_string_lossy().as_bytes());
    hasher.update([0]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        hasher.update(metadata.dev().to_le_bytes());
        hasher.update(metadata.ino().to_le_bytes());
    }
    #[cfg(not(unix))]
    {
        hasher.update(metadata.len().to_le_bytes());
        hasher.update(
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|time| time.as_nanos().to_le_bytes().to_vec())
                .unwrap_or_default(),
        );
    }
    Ok(format!("review-{:x}", hasher.finalize()))
}

fn read_worktree_source(checkout: &Path, path: &str) -> Result<FrozenSource, InspectionError> {
    let path = safe_relative_path(path)?;
    let display_path = path.to_string_lossy();
    let (parent, leaf) = open_worktree_parent(checkout, path)?;
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    let file = match parent.open_with(&leaf, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(FrozenSource::unavailable(
                &display_path,
                "working-tree source disappeared while the review was read",
            ));
        }
        Err(_) => {
            return Ok(FrozenSource::unavailable(
                &display_path,
                "working-tree source could not be opened without following links",
            ));
        }
    };
    let metadata = file.metadata().map_err(|_| {
        InspectionError::new(
            "review_unreadable",
            "working-tree source could not be inspected",
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(FrozenSource::unavailable(
            &display_path,
            "working-tree source is not a regular file",
        ));
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Ok(FrozenSource::truncated(&display_path));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_FILE_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            InspectionError::new("review_unreadable", "working-tree source could not be read")
        })?;
    if bytes.len() > MAX_FILE_BYTES {
        return Ok(FrozenSource::truncated(&display_path));
    }
    Ok(FrozenSource::from_bytes(bytes))
}

fn open_worktree_parent(checkout: &Path, path: &Path) -> Result<(Dir, PathBuf), InspectionError> {
    let leaf = path.file_name().map(PathBuf::from).ok_or_else(|| {
        InspectionError::new(
            "review_invalid_path",
            "working-tree source has no file name",
        )
    })?;
    let mut parent = open_dir_nofollow_absolute(checkout)
        .map_err(|_| InspectionError::new("review_unreadable", "review checkout is unavailable"))?;
    let parent_path = path.parent().unwrap_or_else(|| Path::new(""));
    for component in parent_path.components() {
        let Component::Normal(name) = component else {
            return Err(InspectionError::new(
                "review_invalid_path",
                "Git returned an unsafe working-tree path",
            ));
        };
        parent = parent.open_dir_nofollow(Path::new(name)).map_err(|_| {
            InspectionError::new(
                "review_unreadable",
                "working-tree source parent is unavailable",
            )
        })?;
    }
    Ok((parent, leaf))
}

fn open_dir_nofollow_absolute(path: &Path) -> std::io::Result<Dir> {
    let mut dir = Dir::open_ambient_dir(Path::new("/"), cap_std::ambient_authority())?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => dir = dir.open_dir_nofollow(Path::new(name))?,
            Component::ParentDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "unsafe review checkout path",
                ));
            }
        }
    }
    Ok(dir)
}

fn safe_relative_path(path: &str) -> Result<&Path, InspectionError> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(InspectionError::new(
            "review_invalid_path",
            "Git returned an unsafe working-tree path",
        ));
    }
    Ok(path)
}

fn verified(confidence: DetectionConfidence) -> bool {
    matches!(
        confidence,
        DetectionConfidence::VerifiedLaunch | DetectionConfidence::VerifiedProcess
    )
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

fn parse_name_status(
    bytes: &[u8],
    comparison: ReviewComparison,
) -> Result<Vec<Change>, InspectionError> {
    let mut fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut changes = Vec::new();
    while let Some(status) = fields.next() {
        let status = std::str::from_utf8(status).map_err(|_| {
            InspectionError::new("review_path_encoding", "Git path status was not UTF-8")
        })?;
        let code = status
            .as_bytes()
            .first()
            .copied()
            .ok_or_else(|| InspectionError::new("review_git", "Git emitted an empty status"))?;
        let first = fields
            .next()
            .ok_or_else(|| InspectionError::new("review_git", "Git status omitted a path"))?;
        let first = String::from_utf8(first.to_vec()).map_err(|_| {
            InspectionError::new(
                "review_path_encoding",
                "non-UTF-8 paths are not displayable",
            )
        })?;
        let (old_path, new_path, status) = if matches!(code, b'R' | b'C') {
            let second = fields.next().ok_or_else(|| {
                InspectionError::new("review_git", "Git rename status omitted a destination")
            })?;
            let second = String::from_utf8(second.to_vec()).map_err(|_| {
                InspectionError::new(
                    "review_path_encoding",
                    "non-UTF-8 paths are not displayable",
                )
            })?;
            (
                Some(first),
                Some(second),
                if code == b'R' {
                    ReviewFileStatus::Renamed
                } else {
                    ReviewFileStatus::Copied
                },
            )
        } else {
            (
                if code == b'A' {
                    None
                } else {
                    Some(first.clone())
                },
                if code == b'D' { None } else { Some(first) },
                match code {
                    b'A' => ReviewFileStatus::Added,
                    b'M' => ReviewFileStatus::Modified,
                    b'D' => ReviewFileStatus::Deleted,
                    b'T' => ReviewFileStatus::ModeOnly,
                    _ => ReviewFileStatus::Modified,
                },
            )
        };
        changes.push(Change {
            status,
            comparison,
            old_path,
            new_path,
        });
    }
    Ok(changes)
}

fn parse_untracked(bytes: &[u8]) -> Result<Vec<Change>, InspectionError> {
    let mut changes = Vec::new();
    for entry in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        if entry.len() < 4 || &entry[..3] != b"?? " {
            continue;
        }
        let path = String::from_utf8(entry[3..].to_vec()).map_err(|_| {
            InspectionError::new(
                "review_path_encoding",
                "non-UTF-8 paths are not displayable",
            )
        })?;
        changes.push(Change {
            status: ReviewFileStatus::Untracked,
            comparison: ReviewComparison::Untracked,
            old_path: None,
            new_path: Some(path),
        });
    }
    Ok(changes)
}

fn parse_unified(
    text: &str,
    change: &Change,
    max_hunks: usize,
) -> Result<(Vec<ReviewHunk>, bool, bool), InspectionError> {
    let binary = text
        .lines()
        .any(|line| line.starts_with("Binary files ") || line == "GIT binary patch");
    let mut hunks = Vec::new();
    let mut current: Option<ReviewHunk> = None;
    for raw in text.split_inclusive('\n') {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        if line.starts_with("@@") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            if hunks.len() >= max_hunks {
                return Ok((hunks, binary, true));
            }
            let (old_start, new_start) = parse_hunk_header(line)?;
            current = Some(ReviewHunk {
                old_path: change.old_path.clone(),
                new_path: change.new_path.clone(),
                old_start,
                new_start,
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = current.as_mut() else {
            continue;
        };
        let (kind, text) = match line.as_bytes().first().copied() {
            Some(b'+') => (ReviewDiffLineKind::Added, &line[1..]),
            Some(b'-') => (ReviewDiffLineKind::Deleted, &line[1..]),
            Some(b' ') => (ReviewDiffLineKind::Context, &line[1..]),
            _ => continue,
        };
        let old_line = match kind {
            ReviewDiffLineKind::Added => None,
            _ => Some(
                hunk.old_start
                    + hunk
                        .lines
                        .iter()
                        .filter(|line| line.kind != ReviewDiffLineKind::Added)
                        .count() as u32,
            ),
        };
        let new_line = match kind {
            ReviewDiffLineKind::Deleted => None,
            _ => Some(
                hunk.new_start
                    + hunk
                        .lines
                        .iter()
                        .filter(|line| line.kind != ReviewDiffLineKind::Deleted)
                        .count() as u32,
            ),
        };
        hunk.lines.push(ReviewDiffLine {
            kind,
            old_line,
            new_line,
            text: text.to_owned(),
        });
    }
    if let Some(hunk) = current {
        hunks.push(hunk);
    }
    Ok((hunks, binary, false))
}

fn parse_hunk_header(line: &str) -> Result<(u32, u32), InspectionError> {
    let mut parts = line.split_whitespace();
    if parts.next() != Some("@@") {
        return Err(InspectionError::new(
            "review_diff",
            "Git hunk header is malformed",
        ));
    }
    let old = parts
        .next()
        .and_then(|part| part.strip_prefix('-'))
        .ok_or_else(|| InspectionError::new("review_diff", "Git hunk omitted old range"))?;
    let new = parts
        .next()
        .and_then(|part| part.strip_prefix('+'))
        .ok_or_else(|| InspectionError::new("review_diff", "Git hunk omitted new range"))?;
    let parse = |range: &str| {
        range
            .split(',')
            .next()
            .unwrap_or(range)
            .parse::<u32>()
            .map_err(|_| InspectionError::new("review_diff", "Git hunk line number is malformed"))
    };
    Ok((parse(old)?, parse(new)?))
}

fn truncated_diff(
    file_id: &str,
    change: &Change,
    base: Option<&str>,
    revisions: &RevisionTokens,
    code: &str,
    message: &str,
) -> ReviewFileDiff {
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
    let path = change.new_path.as_deref().or(change.old_path.as_deref());
    ReviewFileDiff {
        binding_id: String::new(),
        session_id: String::new(),
        pane_id: String::new(),
        review_id: String::new(),
        generation: 0,
        file: ReviewChangedFile {
            file_id: file_id.to_owned(),
            comparison: change.comparison,
            status: ReviewFileStatus::Unreadable,
            old_path: change.old_path.clone(),
            new_path: change.new_path.clone(),
            binary: false,
            summary: message.to_owned(),
            old_revision,
            new_revision,
        },
        hunks: Vec::new(),
        old_source: None,
        new_source: None,
        old_source_hash: None,
        new_source_hash: None,
        old_total_lines: None,
        new_total_lines: None,
        old_source_truncated: false,
        new_source_truncated: false,
        truncated: code != "review_unreadable",
        diagnostics: vec![diagnostic(code, message, path)],
    }
}

fn discard_snapshot_payload(diff: &mut ReviewFileDiff) {
    diff.hunks.clear();
    if diff.old_source.take().is_some() {
        diff.old_source_hash = None;
        diff.old_total_lines = None;
        diff.old_source_truncated = true;
    }
    if diff.new_source.take().is_some() {
        diff.new_source_hash = None;
        diff.new_total_lines = None;
        diff.new_source_truncated = true;
    }
    diff.truncated = true;
    diff.diagnostics.push(diagnostic(
        "review_snapshot_bounded",
        "file diff and frozen sources were omitted because the aggregate snapshot limit was reached",
        diff.file.new_path.as_deref().or(diff.file.old_path.as_deref()),
    ));
}

fn untracked_hunk(
    checkout: &Path,
    path: &str,
) -> Result<(Vec<ReviewHunk>, bool, bool), InspectionError> {
    let source = read_worktree_source(checkout, path)?;
    if source.truncated {
        return Ok((Vec::new(), false, true));
    }
    let Some(text) = source.text else {
        if source.hash.is_some() {
            return Ok((Vec::new(), true, false));
        }
        return Err(InspectionError::new(
            "review_unreadable",
            "untracked source is unavailable or nonregular",
        ));
    };
    let lines = text
        .split_inclusive('\n')
        .enumerate()
        .map(|(index, raw)| ReviewDiffLine {
            kind: ReviewDiffLineKind::Added,
            old_line: None,
            new_line: Some(index as u32 + 1),
            text: raw.strip_suffix('\n').unwrap_or(raw).to_owned(),
        })
        .collect();
    Ok((
        vec![ReviewHunk {
            old_path: None,
            new_path: Some(path.to_owned()),
            old_start: 0,
            new_start: 1,
            lines,
        }],
        false,
        false,
    ))
}

fn worktree_token(
    checkout: &Path,
    unstaged_diff: &[u8],
    untracked_paths: &[u8],
) -> Result<String, InspectionError> {
    let mut digest = Sha256::new();
    digest.update(b"cockpit-review-worktree-v2\0");
    digest.update(unstaged_diff);
    let mut remaining = 32 * 1024 * 1024u64;
    for (index, raw_path) in untracked_paths
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .enumerate()
    {
        if index >= MAX_FILES {
            return Err(InspectionError::new(
                "review_limit",
                "untracked revision exceeds the 256-file inspection limit",
            ));
        }
        let path = std::str::from_utf8(raw_path).map_err(|_| {
            InspectionError::new(
                "review_path_encoding",
                "non-UTF-8 untracked path cannot form a source anchor",
            )
        })?;
        digest.update(raw_path);
        digest.update([0]);
        let (parent, leaf) = match open_worktree_parent(checkout, safe_relative_path(path)?) {
            Ok(value) => value,
            Err(error) if error.code == "review_unreadable" => {
                digest.update(b"unavailable");
                continue;
            }
            Err(error) => return Err(error),
        };
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No).nonblock(true);
        let file = match parent.open_with(&leaf, &options) {
            Ok(file) => file,
            Err(_) => {
                digest.update(b"unavailable");
                continue;
            }
        };
        let metadata = file.metadata().map_err(|_| {
            InspectionError::new(
                "review_unreadable",
                "untracked source metadata is unavailable",
            )
        })?;
        if !metadata.is_file() {
            digest.update(b"nonregular");
            continue;
        }
        digest.update(raw_path);
        digest.update([0]);
        if metadata.len() > MAX_FILE_BYTES as u64 {
            // Oversized files cannot supply anchors. Their metadata invalidates
            // the bounded placeholder without reading their contents.
            digest.update(b"oversized");
            digest.update(metadata.len().to_le_bytes());
            digest.update(format!("{:?}", metadata.modified()).as_bytes());
        } else {
            let mut bytes = Vec::new();
            file.take(MAX_FILE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| {
                    InspectionError::new("review_unreadable", "untracked source could not be read")
                })?;
            remaining = remaining.checked_sub(bytes.len() as u64).ok_or_else(|| {
                InspectionError::new(
                    "review_limit",
                    "untracked revision exceeds the 32 MiB inspection limit",
                )
            })?;
            digest.update(bytes);
        }
        digest.update([0]);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn file_id(
    comparison: ReviewComparison,
    old_path: &Option<String>,
    new_path: &Option<String>,
) -> String {
    let tuple =
        serde_json::to_vec(&(comparison, old_path, new_path)).expect("review identity serializes");
    format!("file-{:x}", Sha256::digest(tuple))
}

fn summary(change: &Change) -> String {
    format!("{:?} {:?}", change.comparison, change.status).to_lowercase()
}
fn diagnostic(code: &str, message: &str, path: Option<&str>) -> ProjectDiagnostic {
    ProjectDiagnostic {
        code: code.to_owned(),
        message: message.to_owned(),
        path: path.map(str::to_owned),
    }
}
fn snapshot_name(id: &str) -> Result<String, InspectionError> {
    if Uuid::parse_str(id).is_err() {
        return Err(InspectionError::new(
            "review_invalid_id",
            "review snapshot identity must be a UUID",
        ));
    }
    Ok(format!("snapshot-{id}.json"))
}

fn prune_snapshots(state: &ProjectStore) -> Result<(), InspectionError> {
    let mut snapshots = Vec::new();
    let mut entries = 0usize;
    for entry in state
        .state_dir()
        .entries()
        .map_err(|error| InspectionError::new("review_read", error.to_string()))?
    {
        entries = entries.saturating_add(1);
        if entries > MAX_SNAPSHOT_ENTRIES {
            return Err(InspectionError::new(
                "review_bounded",
                "review snapshot directory exceeded its entry limit",
            ));
        }
        let entry =
            entry.map_err(|error| InspectionError::new("review_read", error.to_string()))?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_prefix("snapshot-"))
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if Uuid::parse_str(id).is_err() {
            continue;
        }
        let stored: StoredSnapshot = read_json_bounded(
            state.state_dir(),
            name.to_str().expect("utf-8 checked"),
            MAX_STORED_SNAPSHOT_BYTES,
        )?;
        snapshots.push((name.to_owned(), stored.created_at));
    }
    snapshots.sort_by(|left, right| right.1.cmp(&left.1));
    for (name, _) in snapshots.into_iter().skip(MAX_SNAPSHOTS) {
        state
            .state_dir()
            .remove_file(name)
            .map_err(|error| InspectionError::new("review_write", error.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use cockpit_protocol::{
        projects::{ProjectLimits, ProjectProvider},
        v1::{
            FocusRequest, FocusResponse, HerdrCompatibility, ResourceMutationRequest,
            ResourceMutationResponse, SessionListResponse, SessionSnapshotResponse,
            TerminalOpenRequest,
        },
    };

    use crate::{
        HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession,
        project_adapter::{
            ProjectInventory, ProjectTerminalRequest, ProjectTerminalResult,
            ProjectWorktreeRemoveRequest, ProjectWorktreeRequest, ProjectWorktreeResult,
        },
        projects::ProjectService,
    };

    #[derive(Default)]
    struct NoopAdapter;

    fn unavailable<T>() -> Result<T, InspectionError> {
        Err(InspectionError::new(
            "test_adapter_unused",
            "test adapter method is not expected",
        ))
    }

    #[async_trait::async_trait]
    impl HerdrAdapter for NoopAdapter {
        async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> {
            unavailable()
        }
        async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> {
            unavailable()
        }
        async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
            unavailable()
        }
        async fn session_snapshot(
            &self,
            _: &str,
        ) -> Result<SessionSnapshotResponse, InspectionError> {
            unavailable()
        }
        async fn focus(&self, _: &str, _: &FocusRequest) -> Result<FocusResponse, InspectionError> {
            unavailable()
        }
        async fn mutate(
            &self,
            _: &str,
            _: &ResourceMutationRequest,
        ) -> Result<ResourceMutationResponse, InspectionError> {
            unavailable()
        }
        async fn subscribe_session(
            &self,
            _: &str,
            _: &SessionSnapshotResponse,
        ) -> Result<SessionSubscription, InspectionError> {
            unavailable()
        }
        async fn open_terminal(
            &self,
            _: &TerminalOpenRequest,
        ) -> Result<TerminalSession, InspectionError> {
            unavailable()
        }
    }

    #[async_trait::async_trait]
    impl ProjectHerdrAdapter for NoopAdapter {
        async fn project_inventory(
            &self,
            _: &str,
            _: &str,
        ) -> Result<ProjectInventory, InspectionError> {
            unavailable()
        }
        async fn project_worktree(
            &self,
            _: &str,
            _: &ProjectWorktreeRequest,
        ) -> Result<ProjectWorktreeResult, InspectionError> {
            unavailable()
        }
        async fn project_terminal(
            &self,
            _: &str,
            _: &ProjectTerminalRequest,
        ) -> Result<ProjectTerminalResult, InspectionError> {
            unavailable()
        }
        async fn project_worktree_dirty(
            &self,
            _: &str,
            _: u32,
            _: u32,
        ) -> Result<bool, InspectionError> {
            unavailable()
        }
        async fn project_close_workspace(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<(), InspectionError> {
            unavailable()
        }
        async fn project_remove_worktree(
            &self,
            _: &str,
            _: &ProjectWorktreeRemoveRequest,
        ) -> Result<(), InspectionError> {
            unavailable()
        }
    }

    #[async_trait::async_trait]
    impl ExtensionHerdrAdapter for NoopAdapter {
        async fn inspect_extension_pane(
            &self,
            _: &str,
            _: &str,
        ) -> Result<crate::ExtensionPaneEvidence, InspectionError> {
            unavailable()
        }
        async fn launch_context_pane(
            &self,
            _: &str,
            _: &crate::ExtensionLaunch,
        ) -> Result<crate::ExtensionPaneEvidence, InspectionError> {
            unavailable()
        }
        async fn launch_review_pane(
            &self,
            _: &str,
            _: &crate::ExtensionLaunch,
        ) -> Result<crate::ExtensionPaneEvidence, InspectionError> {
            unavailable()
        }
    }

    fn configuration(root: &Path) -> ProjectConfiguration {
        ProjectConfiguration {
            version: 1,
            repository_roots: vec![root.to_string_lossy().into_owned()],
            worktree_root: root.join("worktrees").to_string_lossy().into_owned(),
            companion_root: root.join("companions").to_string_lossy().into_owned(),
            state_root: root.join("state").to_string_lossy().into_owned(),
            branch_template: "{repo}/{task_id}".to_owned(),
            checkout_template: "{repo}-{task_id}".to_owned(),
            providers: vec![ProjectProvider {
                id: "test".to_owned(),
                base_url: "https://example.test/".to_owned(),
                executable: "false".to_owned(),
                login: None,
            }],
            limits: ProjectLimits {
                catalog_depth: 2,
                catalog_entries: 32,
                git_timeout_ms: 2_000,
                git_output_bytes: 2 * 1024 * 1024,
                operation_timeout_ms: 2_000,
                context_preview_bytes: 1024 * 1024,
                context_preview_lines: 2_000,
                context_directory_entries: 64,
                context_tree_depth: 8,
            },
            origins: BTreeMap::new(),
        }
    }

    fn service(root: &Path) -> ReviewService {
        service_with_configuration(configuration(root))
    }

    fn service_with_configuration(configuration: ProjectConfiguration) -> ReviewService {
        let adapter = Arc::new(NoopAdapter);
        let projects = Arc::new(
            ProjectService::new(configuration.clone(), adapter.clone()).expect("project service"),
        );
        let context = Arc::new(ContextService::new(
            configuration.clone(),
            adapter.clone(),
            projects,
        ));
        ReviewService::new(configuration, adapter, context).expect("review service")
    }

    fn fixture(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("cockpit-review-{label}-{}", Uuid::new_v4()));
        fixture_at(&root);
        root
    }

    fn fixture_at(root: &Path) {
        std::fs::create_dir_all(&root).expect("fixture directory");
        for args in [
            ["init"].as_slice(),
            ["config", "user.email", "fixture@example.test"].as_slice(),
            ["config", "user.name", "Fixture"].as_slice(),
        ] {
            let status = std::process::Command::new("git")
                .current_dir(&root)
                .args(args)
                .status()
                .expect("git starts");
            assert!(status.success(), "git {args:?}");
        }
    }

    fn git_bytes(root: &Path, args: &[&str]) -> Vec<u8> {
        let output = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .expect("git starts");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn commit(root: &Path, message: &str) {
        git_bytes(root, &["add", "--all"]);
        git_bytes(root, &["commit", "-m", message]);
    }

    async fn collect_real(
        service: &ReviewService,
        root: &Path,
        comparison: ReviewComparison,
        base_ref: Option<&str>,
    ) -> BTreeMap<String, ReviewFileDiff> {
        let request = ReviewSnapshotRequest {
            binding_id: "test-binding".to_owned(),
            repository_id: "test-repository".to_owned(),
            comparison,
            base_ref: base_ref.map(str::to_owned),
        };
        let revisions = service
            .revision_tokens(root)
            .await
            .expect("revision tokens");
        let (_, diffs, _, _, _) = service
            .collect(root, &request, &revisions)
            .await
            .expect("real review collection");
        diffs
    }

    #[tokio::test]
    async fn resolves_an_unconfigured_pane_checkout_by_its_opaque_identity() {
        let root = fixture("unconfigured-checkout");
        let pane_cwd = root.join("src");
        std::fs::create_dir(&pane_cwd).expect("pane directory");
        let configured = configuration(&root);
        let repository_id = RepositoryCatalog::new(configured.clone())
            .list()
            .await
            .expect("catalog listing")
            .repositories
            .into_iter()
            .next()
            .expect("fixture checkout")
            .repository_id;
        let mut unconfigured = configured;
        unconfigured.repository_roots.clear();
        let service = service_with_configuration(unconfigured);

        let resolved = service
            .resolve_checkout(
                &Some(pane_cwd.to_string_lossy().into_owned()),
                &None,
                &repository_id,
            )
            .await
            .expect("unconfigured checkout resolves from pane cwd");

        assert_eq!(resolved.repository_id, repository_id);
        assert_eq!(resolved.checkout_path, root.to_string_lossy());
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn discovers_a_linked_worktree_from_a_subdirectory_with_its_catalog_identity() {
        let workspace =
            std::env::temp_dir().join(format!("cockpit-review-worktrees-{}", Uuid::new_v4()));
        let primary = workspace.join("primary");
        fixture_at(&primary);
        std::fs::write(primary.join("base.txt"), "base\n").expect("write base");
        commit(&primary, "base");
        let linked = workspace.join("linked");
        let linked_text = linked.to_string_lossy().into_owned();
        git_bytes(
            &primary,
            &["worktree", "add", "-b", "linked-review", &linked_text],
        );
        let pane_cwd = linked.join("src");
        std::fs::create_dir(&pane_cwd).expect("linked pane directory");

        let mut configuration = configuration(&workspace);
        configuration.repository_roots = vec![workspace.to_string_lossy().into_owned()];
        let catalog = RepositoryCatalog::new(configuration);
        let listed = catalog.list().await.expect("catalog listing");
        let linked_candidate = listed
            .repositories
            .into_iter()
            .find(|candidate| candidate.checkout_path == linked_text)
            .expect("linked worktree in catalog");
        let discovered = catalog
            .discover_checkout(&pane_cwd)
            .await
            .expect("linked worktree from pane cwd");

        assert_eq!(discovered.repository_id, linked_candidate.repository_id);
        assert_eq!(discovered.checkout_path, linked_text);
        assert!(discovered.is_linked_worktree);
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[test]
    fn fixture_preserves_partially_staged_file_as_two_distinct_anchor_scopes() {
        let root = fixture("partial");
        std::fs::write(root.join("note.txt"), "one\ntwo\n").expect("write base");
        git_bytes(&root, &["add", "--", "note.txt"]);
        git_bytes(&root, &["commit", "-m", "base"]);
        std::fs::write(root.join("note.txt"), "one\nstaged\n").expect("write staged");
        git_bytes(&root, &["add", "--", "note.txt"]);
        std::fs::write(root.join("note.txt"), "one\nstaged\nunstaged\n").expect("write unstaged");

        let staged = parse_name_status(
            &git_bytes(&root, &["diff", "--cached", "--name-status", "-z", "--"]),
            ReviewComparison::Staged,
        )
        .expect("staged status");
        let unstaged = parse_name_status(
            &git_bytes(&root, &["diff", "--name-status", "-z", "--"]),
            ReviewComparison::Unstaged,
        )
        .expect("unstaged status");
        assert_eq!(staged.len(), 1);
        assert_eq!(unstaged.len(), 1);
        assert_eq!(staged[0].new_path.as_deref(), Some("note.txt"));
        assert_eq!(unstaged[0].new_path.as_deref(), Some("note.txt"));
        assert_ne!(
            file_id(
                staged[0].comparison,
                &staged[0].old_path,
                &staged[0].new_path
            ),
            file_id(
                unstaged[0].comparison,
                &unstaged[0].old_path,
                &unstaged[0].new_path
            )
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn unified_parser_keeps_canonical_old_and_new_line_numbers() {
        let change = Change {
            status: ReviewFileStatus::Modified,
            comparison: ReviewComparison::Unstaged,
            old_path: Some("old.txt".to_owned()),
            new_path: Some("new.txt".to_owned()),
        };
        let (hunks, binary, truncated) =
            parse_unified("@@ -2,2 +2,3 @@\n same\n-old\n+new\n+tail\n", &change, 8)
                .expect("parse hunk");
        assert!(!binary && !truncated);
        assert_eq!(hunks[0].lines[0].old_line, Some(2));
        assert_eq!(hunks[0].lines[0].new_line, Some(2));
        assert_eq!(hunks[0].lines[1].old_line, Some(3));
        assert_eq!(hunks[0].lines[1].new_line, None);
        assert_eq!(hunks[0].lines[2].old_line, None);
        assert_eq!(hunks[0].lines[2].new_line, Some(3));
        assert_eq!(hunks[0].lines[3].new_line, Some(4));
    }

    #[test]
    fn frozen_worktree_source_preserves_physical_lf_lines_and_raw_cr_data() {
        let root = fixture("frozen-source");
        std::fs::write(root.join("note.txt"), "first\rsecond\r\nthird").expect("write source");
        let source = read_worktree_source(&root, "note.txt").expect("read source");
        assert_eq!(source.text.as_deref(), Some("first\rsecond\r\nthird"));
        assert_eq!(source.total_lines, Some(2));
        assert_eq!(
            source.hash.as_deref(),
            Some("sha256:75c542c465e6b79ba0a1a3bc586de68b17c112285d6828f82e1dbeab9fff38f0")
        );
    }

    #[test]
    fn worktree_token_changes_when_an_untracked_file_bytes_change() {
        let root = fixture("untracked-token");
        std::fs::write(root.join("draft.txt"), "first").expect("write first source");
        let first = worktree_token(&root, b"", b"draft.txt\0").expect("first token");
        std::fs::write(root.join("draft.txt"), "other").expect("write second source");
        let second = worktree_token(&root, b"", b"draft.txt\0").expect("second token");
        assert_ne!(first, second);
    }
    #[test]
    fn oversized_untracked_sources_stay_bounded_and_file_ids_are_transport_safe() {
        let root = fixture("bounded-untracked");
        let file = std::fs::File::create(root.join("huge.bin")).unwrap();
        file.set_len(8 * 1024 * 1024 * 1024).unwrap();
        assert!(worktree_token(&root, b"", b"huge.bin\0").is_ok());
        let (hunks, _, truncated) = untracked_hunk(&root, "huge.bin").unwrap();
        assert!(hunks.is_empty() && truncated);
        let id = file_id(
            ReviewComparison::Unstaged,
            &Some("a\nb".into()),
            &Some("-new.txt".into()),
        );
        assert!(
            id.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn real_collect_keeps_staged_and_unstaged_sources_distinct_without_git_mutation() {
        let root = fixture("real-partial");
        std::fs::write(root.join("note.txt"), "base\n").expect("write base");
        commit(&root, "base");
        std::fs::write(root.join("note.txt"), "staged\n").expect("write staged");
        git_bytes(&root, &["add", "--", "note.txt"]);
        std::fs::write(root.join("note.txt"), "worktree\n").expect("write worktree");
        let before_index = git_bytes(&root, &["ls-files", "--stage", "-z"]);
        let before_status = git_bytes(&root, &["status", "--porcelain=v1", "-z"]);
        let before_objects = git_bytes(&root, &["count-objects", "-v"]);

        let diffs = collect_real(&service(&root), &root, ReviewComparison::AllLocal, None).await;
        let staged = diffs
            .values()
            .find(|diff| diff.file.comparison == ReviewComparison::Staged)
            .expect("staged diff");
        let unstaged = diffs
            .values()
            .find(|diff| diff.file.comparison == ReviewComparison::Unstaged)
            .expect("unstaged diff");
        assert_ne!(staged.file.file_id, unstaged.file.file_id);
        assert_eq!(staged.old_source.as_deref(), Some("base\n"));
        assert_eq!(staged.new_source.as_deref(), Some("staged\n"));
        assert_eq!(unstaged.old_source.as_deref(), Some("staged\n"));
        assert_eq!(unstaged.new_source.as_deref(), Some("worktree\n"));
        assert_eq!(
            git_bytes(&root, &["ls-files", "--stage", "-z"]),
            before_index
        );
        assert_eq!(
            git_bytes(&root, &["status", "--porcelain=v1", "-z"]),
            before_status
        );
        assert_eq!(git_bytes(&root, &["count-objects", "-v"]), before_objects);
    }

    #[tokio::test]
    async fn real_collect_freezes_branch_sources_and_hashes_despite_worktree_edits() {
        let root = fixture("real-branch");
        std::fs::write(root.join("review.txt"), "base\n").expect("write base");
        commit(&root, "base");
        std::fs::write(root.join("review.txt"), "committed review\n")
            .expect("write reviewed commit");
        commit(&root, "reviewed");
        let service = service(&root);
        let diffs = collect_real(&service, &root, ReviewComparison::Branch, Some("HEAD~1")).await;
        let diff = diffs.values().next().expect("branch diff");
        let frozen_hash = diff.new_source_hash.clone().expect("new source hash");
        assert_eq!(diff.old_source.as_deref(), Some("base\n"));
        assert_eq!(diff.new_source.as_deref(), Some("committed review\n"));

        std::fs::write(root.join("review.txt"), "uncommitted\n").expect("edit worktree");
        let head = service
            .git_text(&root, &["rev-parse", "--verify", "HEAD"])
            .await
            .expect("head");
        let source = service
            .git_source(&root, &head, "review.txt")
            .await
            .expect("frozen head source");
        assert_eq!(source.text.as_deref(), Some("committed review\n"));
        assert_eq!(source.hash.as_deref(), Some(frozen_hash.as_str()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn real_collect_preserves_binary_unreadable_and_bounded_untracked_rows() {
        let root = fixture("real-special");
        std::fs::write(root.join("literal.txt"), "ordinary\n").expect("write literal");
        std::fs::write(root.join("binary.dat"), b"before\0").expect("write binary");
        commit(&root, "base");
        std::fs::write(
            root.join("literal.txt"),
            "Binary files are ordinary text\nSubproject commit also ordinary text\n",
        )
        .expect("write literal phrases");
        std::fs::write(root.join("binary.dat"), b"after\0").expect("change binary");
        std::fs::File::create(root.join("large.txt"))
            .expect("large file")
            .set_len((MAX_FILE_BYTES + 1) as u64)
            .expect("size large file");
        std::os::unix::fs::symlink("/outside-review-fixture", root.join("outside-link"))
            .expect("create untracked link");

        let diffs = collect_real(&service(&root), &root, ReviewComparison::AllLocal, None).await;
        let literal = diffs
            .values()
            .find(|diff| diff.file.new_path.as_deref() == Some("literal.txt"))
            .expect("literal row");
        assert_eq!(literal.file.status, ReviewFileStatus::Modified);
        assert!(!literal.file.binary && !literal.hunks.is_empty());
        let binary = diffs
            .values()
            .find(|diff| diff.file.new_path.as_deref() == Some("binary.dat"))
            .expect("binary row");
        assert_eq!(binary.file.status, ReviewFileStatus::Binary);
        let large = diffs
            .values()
            .find(|diff| diff.file.new_path.as_deref() == Some("large.txt"))
            .expect("large row");
        assert!(large.truncated && large.new_source_truncated);
        let link = diffs
            .values()
            .find(|diff| diff.file.new_path.as_deref() == Some("outside-link"))
            .expect("link row");
        assert_eq!(link.file.status, ReviewFileStatus::Unreadable);
        assert!(!link.truncated);
        assert!(link.new_source.is_none());
        assert!(!link.diagnostics.is_empty());
    }
}
