use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::Path;

use cockpit_protocol::projects::ProjectDiagnostic;
use cockpit_protocol::review::{ReviewComparison, ReviewFileDiff, ReviewSide};
use sha2::{Digest, Sha256};

use crate::InspectionError;

use super::cache::StoredSnapshot;
use super::git::RevisionTokens;
use super::safe_fs::{open_worktree_file, open_worktree_parent, safe_relative_path};
use super::snapshot::Change;
use super::{ReviewService, diagnostic};

pub(super) const MAX_FILE_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone)]
pub(super) struct FrozenSource {
    pub(super) text: Option<String>,
    pub(super) hash: Option<String>,
    /// For worktree pages this is a metadata cursor; for bounded text it is
    /// the exact content hash. It is checked before every continuation page.
    pub(super) identity: Option<String>,
    pub(super) total_lines: Option<u32>,
    pub(super) total_bytes: Option<u32>,
    pub(super) truncated: bool,
    pub(super) diagnostic: Option<ProjectDiagnostic>,
}

#[derive(Debug, Clone, Copy)]
enum SourceObject<'a> {
    Git(&'a str),
    Index,
    Worktree,
}

impl ReviewService {
    pub(super) async fn continue_source(
        &self,
        stored: &StoredSnapshot,
        mut diff: ReviewFileDiff,
        source_side: Option<ReviewSide>,
        source_offset: u32,
        source_revision: Option<&str>,
    ) -> Result<ReviewFileDiff, InspectionError> {
        let Some(side) = source_side else {
            return Ok(diff);
        };
        let expected = match side {
            ReviewSide::Old => diff.file.old_revision.as_deref(),
            ReviewSide::New => diff.file.new_revision.as_deref(),
        };
        if let Some(requested) = source_revision {
            if Some(requested) != expected {
                return Err(InspectionError::new(
                    "review_source_stale",
                    "source continuation revision does not match the frozen file",
                ));
            }
        }
        let change = stored
            .changes
            .get(&diff.file.file_id)
            .cloned()
            .ok_or_else(|| {
                InspectionError::new(
                    "review_file_not_found",
                    "review file is not in this immutable snapshot",
                )
            })?;
        if source_offset > 0 {
            let total = match side {
                ReviewSide::Old => diff.old_source_total_bytes,
                ReviewSide::New => diff.new_source_total_bytes,
            };
            if total.is_some_and(|total| source_offset > total) {
                return Err(InspectionError::new(
                    "review_invalid_request",
                    "source continuation offset is outside the frozen source",
                ));
            }
            self.verify_source_revision(
                Path::new(&stored.snapshot.checkout_path),
                &change,
                side,
                expected,
            )
            .await?;
        }
        if source_offset == 0
            && match side {
                ReviewSide::Old => diff.old_source.is_some(),
                ReviewSide::New => diff.new_source.is_some(),
            }
        {
            return Ok(diff);
        }
        let revisions = RevisionTokens {
            head: stored.snapshot.head_revision.clone(),
            index: stored.snapshot.index_revision.clone(),
            worktree: stored.snapshot.worktree_revision.clone(),
        };
        let page = self
            .source_page(
                Path::new(&stored.snapshot.checkout_path),
                &change,
                stored.snapshot.base_revision.as_deref(),
                &revisions,
                side,
                source_offset,
            )
            .await?;
        match side {
            ReviewSide::Old => {
                diff.old_source = page.text;
                diff.old_source_hash = page.hash;
                diff.old_total_lines = page.total_lines;
                diff.old_source_offset = source_offset;
                diff.old_source_total_bytes = page.total_bytes;
                diff.old_source_truncated = page.truncated;
            }
            ReviewSide::New => {
                diff.new_source = page.text;
                diff.new_source_hash = page.hash;
                diff.new_total_lines = page.total_lines;
                diff.new_source_offset = source_offset;
                diff.new_source_total_bytes = page.total_bytes;
                diff.new_source_truncated = page.truncated;
            }
        }
        Ok(diff)
    }

    async fn source_page(
        &self,
        checkout: &Path,
        change: &Change,
        base: Option<&str>,
        revisions: &RevisionTokens,
        side: ReviewSide,
        offset: u32,
    ) -> Result<FrozenSource, InspectionError> {
        let (path, object) = match (change.comparison, side) {
            (_, ReviewSide::Old) if change.old_path.is_none() => return Ok(FrozenSource::absent()),
            (_, ReviewSide::New) if change.new_path.is_none() => return Ok(FrozenSource::absent()),
            (ReviewComparison::Untracked, ReviewSide::New)
            | (ReviewComparison::Unstaged, ReviewSide::New) => (
                change.new_path.as_deref().expect("path checked"),
                SourceObject::Worktree,
            ),
            (ReviewComparison::Staged, ReviewSide::Old) => (
                change.old_path.as_deref().expect("path checked"),
                SourceObject::Git(revisions.head.as_deref().expect("head revision")),
            ),
            (ReviewComparison::Staged, ReviewSide::New) => (
                change.new_path.as_deref().expect("path checked"),
                SourceObject::Index,
            ),
            (ReviewComparison::Unstaged, ReviewSide::Old) => (
                change.old_path.as_deref().expect("path checked"),
                SourceObject::Index,
            ),
            (ReviewComparison::Branch, ReviewSide::Old) => (
                change.old_path.as_deref().expect("path checked"),
                SourceObject::Git(base.expect("branch base")),
            ),
            (ReviewComparison::Branch, ReviewSide::New) => (
                change.new_path.as_deref().expect("path checked"),
                SourceObject::Git(revisions.head.as_deref().expect("head revision")),
            ),
            _ => return Ok(FrozenSource::absent()),
        };
        match object {
            SourceObject::Worktree => {
                let checkout = checkout.to_path_buf();
                let path = path.to_owned();
                tokio::task::spawn_blocking(move || {
                    read_worktree_source_page(&checkout, &path, offset)
                })
                .await
                .map_err(|error| InspectionError::new("review_task", error.to_string()))?
            }
            SourceObject::Git(revision) => {
                self.git_source_page(checkout, revision, path, offset).await
            }
            SourceObject::Index => self.git_source_page(checkout, ":", path, offset).await,
        }
    }

    pub(super) async fn snapshot_source(
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

    pub(super) async fn verify_worktree_revisions(
        &self,
        checkout: &Path,
        change: &Change,
    ) -> Result<(), InspectionError> {
        if !matches!(
            change.comparison,
            ReviewComparison::Untracked | ReviewComparison::Unstaged
        ) {
            return Ok(());
        }
        let Some(path) = change.new_path.as_deref() else {
            return Ok(());
        };
        let expected = change.new_revision.as_deref();
        if expected.is_none() {
            return Ok(());
        }
        let checkout = checkout.to_path_buf();
        let path = path.to_owned();
        let current = tokio::task::spawn_blocking(move || {
            worktree_source_identity(&checkout, &path)
                .transpose()
                .map_err(|error| error)
        })
        .await
        .map_err(|error| InspectionError::new("review_task", error.to_string()))??;
        if current.as_deref() != expected {
            return Err(InspectionError::new(
                "review_source_stale",
                "working-tree source changed after the review inventory was captured",
            ));
        }
        Ok(())
    }

    pub(super) async fn verify_source_revision(
        &self,
        checkout: &Path,
        change: &Change,
        side: ReviewSide,
        expected: Option<&str>,
    ) -> Result<(), InspectionError> {
        let is_worktree = matches!(
            (change.comparison, side),
            (ReviewComparison::Untracked, ReviewSide::New)
                | (ReviewComparison::Unstaged, ReviewSide::New)
        );
        if !is_worktree {
            return Ok(());
        }
        let Some(path) = change.new_path.as_deref() else {
            return Ok(());
        };
        let Some(expected) = expected else {
            return Err(InspectionError::new(
                "review_source_stale",
                "working-tree source has no immutable revision cursor",
            ));
        };
        let checkout = checkout.to_path_buf();
        let path = path.to_owned();
        let current = tokio::task::spawn_blocking(move || {
            worktree_source_identity(&checkout, &path)
                .transpose()
                .map_err(|error| error)
        })
        .await
        .map_err(|error| InspectionError::new("review_task", error.to_string()))??;
        if current.as_deref() != Some(expected) {
            return Err(InspectionError::new(
                "review_source_stale",
                "working-tree source changed while the review page was being read",
            ));
        }
        Ok(())
    }
}

impl FrozenSource {
    pub(super) fn absent() -> Self {
        Self {
            text: None,
            hash: None,
            identity: None,
            total_lines: None,
            total_bytes: None,
            truncated: false,
            diagnostic: None,
        }
    }

    pub(super) fn from_bytes(bytes: Vec<u8>) -> Self {
        let hash = Some(hash_bytes(&bytes));
        match String::from_utf8(bytes) {
            Ok(text) => Self {
                total_lines: Some(physical_line_count(&text)),
                total_bytes: u32::try_from(text.len()).ok(),
                text: Some(text),
                hash: hash.clone(),
                identity: hash.clone(),
                truncated: false,
                diagnostic: None,
            },
            Err(_) => Self {
                text: None,
                hash: hash.clone(),
                identity: hash.clone(),
                total_lines: None,
                total_bytes: None,
                truncated: false,
                diagnostic: Some(diagnostic(
                    "review_source_binary",
                    "review source is not UTF-8 text",
                    None,
                )),
            },
        }
    }

    fn from_bytes_with_identity(bytes: Vec<u8>, _metadata_identity: String) -> Self {
        // A bounded body hash is the exact cursor for small files. The
        // metadata identity is only needed for oversized continuation pages.
        Self::from_bytes(bytes)
    }

    fn truncated_with_bytes(path: &str, total_bytes: Option<u32>) -> Self {
        Self {
            text: None,
            hash: None,
            identity: None,
            total_lines: None,
            total_bytes,
            truncated: true,
            diagnostic: Some(diagnostic(
                "review_source_bounded",
                "review source exceeds the immutable source limit",
                Some(path),
            )),
        }
    }

    fn with_identity(mut self, identity: String) -> Self {
        self.identity = Some(identity);
        self
    }

    pub(super) fn unavailable(path: &str, message: &str) -> Self {
        Self {
            text: None,
            hash: None,
            identity: None,
            total_lines: None,
            total_bytes: None,
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

pub(super) fn read_worktree_source(
    checkout: &Path,
    path: &str,
) -> Result<FrozenSource, InspectionError> {
    let path = safe_relative_path(path)?;
    let display_path = path.to_string_lossy();
    let (parent, leaf) = open_worktree_parent(checkout, path)?;
    let mut file = match open_worktree_file(&parent, &leaf) {
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
        return Ok(FrozenSource::truncated_with_bytes(
            &display_path,
            u32::try_from(metadata.len()).ok(),
        )
        .with_identity(worktree_metadata_identity(&metadata)));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&mut file)
        .take(MAX_FILE_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            InspectionError::new("review_unreadable", "working-tree source could not be read")
        })?;
    if bytes.len() > MAX_FILE_BYTES {
        return Ok(FrozenSource::truncated_with_bytes(
            &display_path,
            u32::try_from(metadata.len()).ok(),
        )
        .with_identity(worktree_metadata_identity(&metadata)));
    }
    let after = file.metadata().map_err(|_| {
        InspectionError::new(
            "review_unreadable",
            "working-tree source could not be inspected",
        )
    })?;
    if worktree_metadata_identity(&after) != worktree_metadata_identity(&metadata) {
        return Err(InspectionError::new(
            "review_source_stale",
            "working-tree source changed while it was being read",
        ));
    }
    Ok(FrozenSource::from_bytes_with_identity(
        bytes,
        worktree_metadata_identity(&metadata),
    ))
}

fn worktree_metadata_identity(metadata: &cap_std::fs::Metadata) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"cockpit-review-worktree-v1\0");
    hasher.update(metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified() {
        hasher.update(format!("{modified:?}").as_bytes());
    }
    #[cfg(unix)]
    {
        hasher.update(cap_fs_ext::MetadataExt::dev(metadata).to_le_bytes());
        hasher.update(cap_fs_ext::MetadataExt::ino(metadata).to_le_bytes());
    }
    format!("worktree-{:x}", hasher.finalize())
}

pub(super) fn read_worktree_source_page(
    checkout: &Path,
    path: &str,
    offset: u32,
) -> Result<FrozenSource, InspectionError> {
    let path = safe_relative_path(path)?;
    let display_path = path.to_string_lossy();
    let (parent, leaf) = open_worktree_parent(checkout, path)?;
    let mut file = open_worktree_file(&parent, &leaf).map_err(|_| {
        InspectionError::new(
            "review_unreadable",
            "working-tree source could not be opened",
        )
    })?;
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
    let total_bytes = u32::try_from(metadata.len()).map_err(|_| {
        InspectionError::new(
            "review_source_bounded",
            "working-tree source is too large to page",
        )
    })?;
    let offset = u64::from(offset).min(metadata.len());
    file.seek(SeekFrom::Start(offset)).map_err(|_| {
        InspectionError::new(
            "review_unreadable",
            "working-tree source could not be seeked",
        )
    })?;
    let mut bytes = Vec::with_capacity(MAX_FILE_BYTES.min((metadata.len() - offset) as usize));
    (&mut file)
        .take(MAX_FILE_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            InspectionError::new("review_unreadable", "working-tree source could not be read")
        })?;
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            let valid = error.utf8_error().valid_up_to();
            if valid == 0 && offset > 0 {
                return Err(InspectionError::new(
                    "review_source_page_boundary",
                    "source continuation offset is not a UTF-8 boundary",
                ));
            }
            String::from_utf8(error.into_bytes()[..valid].to_vec()).map_err(|_| {
                InspectionError::new(
                    "review_binary_diff",
                    "working-tree source is not UTF-8 text",
                )
            })?
        }
    };
    let after = file.metadata().map_err(|_| {
        InspectionError::new(
            "review_unreadable",
            "working-tree source could not be inspected",
        )
    })?;
    if worktree_metadata_identity(&after) != worktree_metadata_identity(&metadata) {
        return Err(InspectionError::new(
            "review_source_stale",
            "working-tree source changed while the page was being read",
        ));
    }
    let end = offset.saturating_add(text.len() as u64);
    Ok(FrozenSource {
        text: Some(text),
        hash: None,
        identity: Some(worktree_metadata_identity(&metadata)),
        total_lines: None,
        total_bytes: Some(total_bytes),
        truncated: end < metadata.len(),
        diagnostic: None,
    })
}

pub(super) fn worktree_source_identity(
    checkout: &Path,
    path: &str,
) -> Option<Result<String, InspectionError>> {
    match read_worktree_source(checkout, path) {
        Ok(source) => source.identity.map(Ok),
        Err(error) if error.code == "review_unreadable" => None,
        Err(error) => Some(Err(error)),
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
