use std::collections::BTreeMap;
use std::path::Path;

use cockpit_protocol::projects::ProjectDiagnostic;
use cockpit_protocol::review::{
    ReviewChangedFile, ReviewComparison, ReviewDiffLine, ReviewDiffLineKind, ReviewFileDiff,
    ReviewFileStatus, ReviewHunk, ReviewSide,
};

use crate::InspectionError;

use super::git::RevisionTokens;
use super::snapshot::Change;
use super::source::{FrozenSource, read_worktree_source};
use super::{ReviewService, diagnostic};

const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
const MAX_HUNKS: usize = 2048;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ChangeStatistics {
    pub(super) additions: Option<u32>,
    pub(super) deletions: Option<u32>,
    pub(super) binary: bool,
}

pub(super) fn parse_numstat(
    bytes: &[u8],
) -> Result<BTreeMap<String, ChangeStatistics>, InspectionError> {
    let invalid =
        || InspectionError::new("review_numstat", "Git returned malformed line statistics");
    let mut records = bytes.split(|byte| *byte == 0).peekable();
    let mut result = BTreeMap::new();
    while let Some(record) = records.next() {
        if record.is_empty() && records.peek().is_none() {
            break;
        }
        let mut fields = record.splitn(3, |byte| *byte == b'\t');
        let added = fields.next().ok_or_else(invalid)?;
        let removed = fields.next().ok_or_else(invalid)?;
        let mut path = fields.next().ok_or_else(invalid)?;
        if path.is_empty() {
            records
                .next()
                .filter(|path| !path.is_empty())
                .ok_or_else(invalid)?;
            path = records
                .next()
                .filter(|path| !path.is_empty())
                .ok_or_else(invalid)?;
        }
        let parse = |value: &[u8]| -> Result<Option<u32>, InspectionError> {
            if value == b"-" {
                return Ok(None);
            }
            std::str::from_utf8(value)
                .map_err(|_| invalid())?
                .parse()
                .map(Some)
                .map_err(|_| invalid())
        };
        result.insert(
            String::from_utf8(path.to_vec()).map_err(|_| invalid())?,
            ChangeStatistics {
                additions: parse(added)?,
                deletions: parse(removed)?,
                binary: added == b"-" || removed == b"-",
            },
        );
    }
    Ok(result)
}

pub(super) fn parse_name_status(
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
            old_revision: None,
            new_revision: None,
        });
    }
    Ok(changes)
}

pub(super) fn parse_untracked(bytes: &[u8]) -> Result<Vec<Change>, InspectionError> {
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
            old_revision: None,
            new_revision: None,
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

fn change_counts(
    hunks: &[ReviewHunk],
    binary: bool,
    truncated: bool,
    status: ReviewFileStatus,
) -> (Option<u32>, Option<u32>) {
    if binary
        || truncated
        || matches!(
            status,
            ReviewFileStatus::ModeOnly | ReviewFileStatus::Submodule | ReviewFileStatus::Unreadable
        )
    {
        return (None, None);
    }
    let additions = hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.kind == ReviewDiffLineKind::Added)
        .count() as u32;
    let deletions = hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.kind == ReviewDiffLineKind::Deleted)
        .count() as u32;
    (Some(additions), Some(deletions))
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

pub(super) fn truncated_diff(
    file_id: &str,
    change: &Change,
    base: Option<&str>,
    revisions: &RevisionTokens,
    code: &str,
    message: &str,
) -> ReviewFileDiff {
    let old_revision = change
        .old_revision
        .clone()
        .or_else(|| match change.comparison {
            ReviewComparison::Staged => revisions.head.clone(),
            ReviewComparison::Branch => base.map(str::to_owned),
            ReviewComparison::Unstaged => Some(revisions.index.clone()),
            ReviewComparison::Untracked | ReviewComparison::AllLocal => None,
        });
    let new_revision = change
        .new_revision
        .clone()
        .or_else(|| match change.comparison {
            ReviewComparison::Staged => Some(revisions.index.clone()),
            ReviewComparison::Branch => revisions.head.clone(),
            ReviewComparison::Unstaged | ReviewComparison::Untracked => {
                Some(revisions.worktree.clone())
            }
            ReviewComparison::AllLocal => None,
        });
    let path = change.new_path.as_deref().or(change.old_path.as_deref());
    ReviewFileDiff {
        binding_id: String::new(),
        session_id: String::new(),
        viewer_id: String::new(),
        review_id: String::new(),
        generation: 0,
        file: ReviewChangedFile {
            file_id: file_id.to_owned(),
            comparison: change.comparison,
            status: ReviewFileStatus::Unreadable,
            old_path: change.old_path.clone(),
            new_path: change.new_path.clone(),
            binary: false,
            additions: None,
            deletions: None,
            summary: message.to_owned(),
            old_revision,
            new_revision,
        },
        hunks: Vec::new(),
        old_source: None,
        new_source: None,
        old_source_hash: None,
        new_source_hash: None,
        old_source_offset: 0,
        new_source_offset: 0,
        old_source_total_bytes: None,
        new_source_total_bytes: None,
        old_total_lines: None,
        new_total_lines: None,
        old_source_truncated: code == "review_diff_bounded",
        new_source_truncated: code == "review_diff_bounded",
        truncated: code != "review_unreadable",
        diagnostics: vec![diagnostic(code, message, path)],
    }
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

pub(super) fn summary(change: &Change) -> String {
    format!("{:?} {:?}", change.comparison, change.status).to_lowercase()
}

impl ReviewService {
    pub(super) async fn diff(
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
        let (mut hunks, binary, truncated, status) =
            self.diff_hunks(checkout, path, change, base).await?;
        let mut diagnostics = diff_diagnostics(path, &mut hunks, binary, status);
        let (old_source, new_source) = self
            .diff_sources(checkout, change, base, revisions, binary, status)
            .await?;
        if let Some(diagnostic) = old_source.diagnostic.clone() {
            diagnostics.push(diagnostic);
        }
        if let Some(diagnostic) = new_source.diagnostic.clone() {
            diagnostics.push(diagnostic);
        }
        Ok(assemble_diff(
            file_id,
            change,
            base,
            revisions,
            hunks,
            binary,
            truncated,
            status,
            old_source,
            new_source,
            diagnostics,
        ))
    }

    async fn diff_hunks(
        &self,
        checkout: &Path,
        path: &str,
        change: &Change,
        base: Option<&str>,
    ) -> Result<(Vec<ReviewHunk>, bool, bool, ReviewFileStatus), InspectionError> {
        if change.comparison == ReviewComparison::Untracked {
            let (hunks, binary, truncated) = untracked_hunk(checkout, path)?;
            return Ok((hunks, binary, truncated, change.status));
        }
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
        Ok((hunks, binary, truncated, status))
    }

    async fn diff_sources(
        &self,
        checkout: &Path,
        change: &Change,
        base: Option<&str>,
        revisions: &RevisionTokens,
        binary: bool,
        status: ReviewFileStatus,
    ) -> Result<(FrozenSource, FrozenSource), InspectionError> {
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
        Ok((old_source, new_source))
    }
}

fn diff_diagnostics(
    path: &str,
    hunks: &mut Vec<ReviewHunk>,
    binary: bool,
    status: ReviewFileStatus,
) -> Vec<ProjectDiagnostic> {
    let mut diagnostics = Vec::new();
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
    diagnostics
}

fn diff_revisions(
    change: &Change,
    base: Option<&str>,
    revisions: &RevisionTokens,
) -> (Option<String>, Option<String>) {
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
    (old_revision, new_revision)
}

fn assemble_diff(
    file_id: &str,
    change: &Change,
    base: Option<&str>,
    revisions: &RevisionTokens,
    hunks: Vec<ReviewHunk>,
    binary: bool,
    truncated: bool,
    status: ReviewFileStatus,
    old_source: FrozenSource,
    new_source: FrozenSource,
    diagnostics: Vec<ProjectDiagnostic>,
) -> (ReviewFileDiff, bool) {
    let (old_revision, new_revision) = diff_revisions(change, base, revisions);
    let source_truncated = old_source.truncated || new_source.truncated;
    let (additions, deletions) =
        change_counts(&hunks, binary, truncated || source_truncated, status);
    let old_revision = old_source.identity.clone().or(old_revision);
    let new_revision = new_source.identity.clone().or(new_revision);
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
        additions,
        deletions,
        summary: summary(change),
        old_revision,
        new_revision,
    };
    (
        ReviewFileDiff {
            binding_id: String::new(),
            session_id: String::new(),
            viewer_id: String::new(),
            review_id: String::new(),
            generation: 0,
            file,
            hunks,
            old_source: old_source.text,
            new_source: new_source.text,
            old_source_hash: old_source.hash,
            new_source_hash: new_source.hash,
            old_source_offset: 0,
            new_source_offset: 0,
            old_source_total_bytes: old_source.total_bytes,
            new_source_total_bytes: new_source.total_bytes,
            old_total_lines: old_source.total_lines,
            new_total_lines: new_source.total_lines,
            old_source_truncated: old_source.truncated,
            new_source_truncated: new_source.truncated,
            truncated: truncated || source_truncated,
            diagnostics,
        },
        truncated || source_truncated,
    )
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
