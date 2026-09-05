use std::io;
use std::path::Path;

use cockpit_protocol::comments::{CommentAnchor, CommentBatch, CommentDraft, CommentOwner};
use uuid::Uuid;

use crate::InspectionError;
use crate::project_store::{ProjectStore, atomic_write_json, read_json_bounded, timestamp};

const MAX_DRAFTS_PER_BATCH: usize = 64;
const MAX_COMMENT_TEXT_BYTES: usize = 8 * 1024;
const MAX_SELECTED_LINES: usize = 20_000;
const MAX_BATCH_BYTES: u64 = 4 * 1024 * 1024;
const MAX_BATCHES: usize = 256;
const MAX_SCANNED_ENTRIES: usize = 4096;

/// Durable, process- and host-shared storage for reference comment batches.
///
/// All filesystem work is performed in `spawn_blocking`; the open descriptor and
/// kernel locks are retained by the underlying ProjectStore.
#[derive(Debug, Clone)]
pub(super) struct CommentStore {
    state: ProjectStore,
}

impl CommentStore {
    pub(super) fn new(root: &Path) -> Result<Self, InspectionError> {
        Ok(Self {
            state: ProjectStore::new(root)?,
        })
    }

    pub(super) async fn list(&self) -> Result<(Vec<CommentBatch>, bool), InspectionError> {
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = state.acquire_named_lock(".comments.lock", "comments_lock")?;
            scan_batches(&state)
        })
        .await
        .map_err(|error| InspectionError::new("comments_task", error.to_string()))?
    }

    pub(super) async fn load(
        &self,
        batch_id: &str,
    ) -> Result<Option<CommentBatch>, InspectionError> {
        let batch_id = batch_id.to_owned();
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || {
            validate_uuid(&batch_id, "batch")?;
            let _lock = state.acquire_record_lock(&batch_id)?;
            read_batch(&state, &batch_id)
        })
        .await
        .map_err(|error| InspectionError::new("comments_task", error.to_string()))?
    }

    pub(super) async fn commit(
        &self,
        batch: CommentBatch,
        expected_generation: u32,
    ) -> Result<CommentBatch, InspectionError> {
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || commit_blocking(&state, batch, expected_generation))
            .await
            .map_err(|error| InspectionError::new("comments_task", error.to_string()))?
    }
}

fn commit_blocking(
    state: &ProjectStore,
    mut batch: CommentBatch,
    expected_generation: u32,
) -> Result<CommentBatch, InspectionError> {
    validate_batch(&batch)?;

    // The collection lock makes creation collision checks atomic across hosts.
    let _collection_lock = state.acquire_named_lock(".comments.lock", "comments_lock")?;
    let _record_lock = state.acquire_record_lock(&batch.batch_id)?;
    let name = record_name(&batch.batch_id);
    let existing = read_batch(state, &batch.batch_id)?;

    let next_generation = match existing.as_ref() {
        Some(current) => {
            if current.generation != expected_generation {
                return Err(InspectionError::new(
                    "stale_generation",
                    "comment batch generation is no longer current",
                ));
            }
            if current.owner.source_kind != batch.owner.source_kind
                || current.owner.source_id != batch.owner.source_id
            {
                return Err(InspectionError::new(
                    "owner_mismatch",
                    "comment source identity cannot change",
                ));
            }
            current.generation.checked_add(1).ok_or_else(|| {
                InspectionError::new("invalid_generation", "comment batch generation overflow")
            })?
        }
        None => {
            if expected_generation != 0 {
                return Err(InspectionError::new(
                    "stale_generation",
                    "new comment batch must start at generation zero",
                ));
            }
            let (others, truncated) = scan_batches(state)?;
            if truncated || others.len() >= MAX_BATCHES {
                return Err(InspectionError::new(
                    "comments_batches_bounded",
                    "comment store has reached its batch limit",
                ));
            }
            if others.iter().any(|other| other.owner == batch.owner) {
                return Err(InspectionError::new(
                    "duplicate_owner",
                    "a comment batch already exists for this owner",
                ));
            }
            1
        }
    };

    batch.generation = next_generation;
    batch.live_attachment = None;
    batch.updated_at = timestamp();
    validate_batch(&batch)?;
    let bytes = serde_json::to_vec_pretty(&batch)
        .map_err(|error| InspectionError::new("comments_write", error.to_string()))?;
    if bytes.len() as u64 > MAX_BATCH_BYTES {
        return Err(InspectionError::new(
            "comments_record_bounded",
            "comment batch exceeds its serialized byte limit",
        ));
    }
    atomic_write_json(state.state_dir(), &name, &batch).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidInput {
            InspectionError::new(
                "unsafe_path",
                "comment batch destination is not a regular file",
            )
        } else {
            InspectionError::new("comments_write", error.to_string())
        }
    })?;
    Ok(batch)
}

fn read_batch(
    state: &ProjectStore,
    batch_id: &str,
) -> Result<Option<CommentBatch>, InspectionError> {
    let name = record_name(batch_id);
    match state.state_dir().symlink_metadata(&name) {
        Ok(_) => {
            let mut batch: CommentBatch =
                read_json_bounded(state.state_dir(), &name, MAX_BATCH_BYTES)?;
            validate_batch(&batch)?;
            if batch.batch_id != batch_id {
                return Err(InspectionError::new(
                    "comments_corrupt",
                    "comment record and filename identities differ",
                ));
            }
            // Attachments are runtime evidence and are never trusted from disk.
            batch.live_attachment = None;
            Ok(Some(batch))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(InspectionError::new("comments_read", error.to_string())),
    }
}

fn scan_batches(state: &ProjectStore) -> Result<(Vec<CommentBatch>, bool), InspectionError> {
    let entries = state
        .state_dir()
        .entries()
        .map_err(|error| InspectionError::new("comments_read", error.to_string()))?;
    let mut names = Vec::new();
    let mut scanned = 0usize;
    for entry in entries {
        scanned = scanned.saturating_add(1);
        if scanned > MAX_SCANNED_ENTRIES {
            return Err(InspectionError::new(
                "comments_lookup_bounded",
                "comment lookup exceeded its bounded directory-entry limit",
            ));
        }
        let entry =
            entry.map_err(|error| InspectionError::new("comments_read", error.to_string()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if Uuid::parse_str(stem).is_err() {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| InspectionError::new("comments_read", error.to_string()))?;
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(InspectionError::new(
                "unsafe_path",
                "comment batch is not a regular file",
            ));
        }
        names.push(stem.to_owned());
    }
    names.sort();
    let truncated = names.len() > MAX_BATCHES;
    if truncated {
        names.truncate(MAX_BATCHES);
    }
    let mut batches = Vec::with_capacity(names.len());
    for id in names {
        let Some(batch) = read_batch(state, &id)? else {
            continue;
        };
        if batch.batch_id != id {
            return Err(InspectionError::new(
                "comments_corrupt",
                "comment record and filename identities differ",
            ));
        }
        batches.push(batch);
    }
    batches.sort_by(|left, right| {
        left.updated_at
            .cmp(&right.updated_at)
            .then(left.batch_id.cmp(&right.batch_id))
    });
    Ok((batches, truncated))
}

fn validate_batch(batch: &CommentBatch) -> Result<(), InspectionError> {
    validate_uuid(&batch.batch_id, "batch")?;
    validate_owner(&batch.owner)?;
    if batch.drafts.len() > MAX_DRAFTS_PER_BATCH {
        return Err(InspectionError::new(
            "comments_drafts_bounded",
            "comment batch exceeds its draft limit",
        ));
    }
    let mut ids = std::collections::HashSet::with_capacity(batch.drafts.len());
    for draft in &batch.drafts {
        validate_draft(draft)?;
        if !ids.insert(draft.draft_id.as_str()) {
            return Err(InspectionError::new(
                "comments_corrupt",
                "comment draft identifiers must be unique",
            ));
        }
    }
    Ok(())
}

fn validate_owner(owner: &CommentOwner) -> Result<(), InspectionError> {
    validate_text_id(&owner.session_id, "session")?;
    validate_text_id(&owner.pane_id, "pane")?;
    validate_text_id(&owner.terminal_id, "terminal")?;
    validate_text_id(&owner.source_id, "source")
}

fn validate_draft(draft: &CommentDraft) -> Result<(), InspectionError> {
    validate_uuid(&draft.draft_id, "draft")?;
    if draft.comment_text.as_bytes().len() > MAX_COMMENT_TEXT_BYTES {
        return Err(InspectionError::new(
            "comments_text_bounded",
            "comment text exceeds its byte limit",
        ));
    }
    validate_text_id(&draft.file_ref.root_id, "root")?;
    if let Some(review) = &draft.file_ref.review {
        validate_text_id(&review.review_id, "review")?;
        validate_text_id(&review.file_id, "review file")?;
        if review.generation == 0 {
            return Err(InspectionError::new(
                "comments_invalid_record",
                "review generation must be positive",
            ));
        }
    }
    if draft.file_ref.path.is_empty() || draft.file_ref.path.len() > 4096 {
        return Err(InspectionError::new(
            "comments_invalid_path",
            "comment path is invalid",
        ));
    }
    if draft.file_ref.absolute_path.is_empty() || draft.file_ref.absolute_path.len() > 16 * 1024 {
        return Err(InspectionError::new(
            "comments_invalid_path",
            "comment absolute path is invalid",
        ));
    }
    if draft.file_ref.revision.len() > 4096
        || draft
            .file_ref
            .content_hash
            .as_ref()
            .is_some_and(|hash| hash.len() > 4096)
    {
        return Err(InspectionError::new(
            "comments_metadata_bounded",
            "comment file metadata exceeds its limit",
        ));
    }
    if let CommentAnchor::Lines {
        start_line,
        end_line,
        selected_lines,
    } = &draft.anchor
    {
        let expected_lines = end_line.saturating_sub(*start_line).saturating_add(1);
        if *start_line == 0
            || *end_line < *start_line
            || selected_lines.len() > MAX_SELECTED_LINES
            || u32::try_from(selected_lines.len()).ok() != Some(expected_lines)
        {
            return Err(InspectionError::new(
                "comments_anchor_invalid",
                "comment line anchor is invalid",
            ));
        }
    }
    Ok(())
}

fn validate_uuid(value: &str, kind: &str) -> Result<(), InspectionError> {
    if Uuid::parse_str(value).is_err() {
        return Err(InspectionError::new(
            "comments_invalid_id",
            format!("comment {kind} identity must be a UUID"),
        ));
    }
    Ok(())
}

fn validate_text_id(value: &str, kind: &str) -> Result<(), InspectionError> {
    if value.is_empty()
        || value.len() > 256
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(InspectionError::new(
            "comments_invalid_identity",
            format!("comment {kind} identity is invalid"),
        ));
    }
    Ok(())
}

fn record_name(id: &str) -> String {
    format!("{id}.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::comments::{CommentFileRef, CommentLocation, CommentSourceState};
    use cockpit_protocol::context::ExtensionKind;
    use std::fs;

    fn temp_root(label: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("cockpit-comment-store-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("temporary root");
        root
    }

    fn batch(owner_suffix: &str, text: &str) -> CommentBatch {
        CommentBatch {
            batch_id: Uuid::new_v4().to_string(),
            generation: 0,
            owner: CommentOwner {
                session_id: format!("session-{owner_suffix}"),
                pane_id: format!("pane-{owner_suffix}"),
                terminal_id: format!("terminal-{owner_suffix}"),
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
                comment_text: text.to_owned(),
                source_state: CommentSourceState::Current,
                updated_at: "1".to_owned(),
            }],
            updated_at: "1".to_owned(),
        }
    }

    #[tokio::test]
    async fn compare_and_swap_preserves_old_batch_on_stale_commit() {
        let root = temp_root("cas");
        let store = CommentStore::new(&root).expect("store");
        let original = batch("cas", "original");
        let committed = store
            .commit(original.clone(), 0)
            .await
            .expect("initial commit");
        let error = store
            .commit(original, 0)
            .await
            .expect_err("stale commit must fail");
        assert_eq!(error.code, "stale_generation");
        let restarted = CommentStore::new(&root).expect("restart store");
        let loaded = restarted
            .load(&committed.batch_id)
            .await
            .expect("load after restart")
            .expect("batch");
        assert_eq!(loaded.drafts[0].comment_text, "original");
        assert_eq!(loaded.generation, 1);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_batch_record_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = temp_root("symlink");
        let store = CommentStore::new(&root).expect("store");
        let committed = store
            .commit(batch("symlink", "text"), 0)
            .await
            .expect("commit");
        let record = root.join(record_name(&committed.batch_id));
        let target = root.join("target.json");
        fs::rename(&record, &target).expect("move record");
        symlink(&target, &record).expect("symlink record");
        let error = store
            .load(&committed.batch_id)
            .await
            .expect_err("symlink must fail");
        assert_eq!(error.code, "unsafe_path");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn oversized_comment_is_rejected_without_creating_record() {
        let root = temp_root("limit");
        let store = CommentStore::new(&root).expect("store");
        let oversized = "x".repeat(MAX_COMMENT_TEXT_BYTES + 1);
        let batch = batch("limit", &oversized);
        let id = batch.batch_id.clone();
        let error = store
            .commit(batch, 0)
            .await
            .expect_err("oversized comment must fail");
        assert_eq!(error.code, "comments_text_bounded");
        assert!(
            !root.join(record_name(&id)).exists(),
            "failed commit must not create a record"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn anchors_match_the_configured_context_line_maximum() {
        let root = temp_root("anchor-lines");
        let store = CommentStore::new(&root).expect("store");
        let mut accepted = batch("anchor-lines", "text");
        accepted.drafts[0].anchor = CommentAnchor::Lines {
            start_line: 1,
            end_line: MAX_SELECTED_LINES as u32,
            selected_lines: vec!["line\n".to_owned(); MAX_SELECTED_LINES],
        };
        store.commit(accepted, 0).await.expect("maximum anchor");

        let mut mismatched = batch("anchor-mismatch", "text");
        mismatched.drafts[0].anchor = CommentAnchor::Lines {
            start_line: 1,
            end_line: 2,
            selected_lines: vec!["line\n".to_owned()],
        };
        let error = store
            .commit(mismatched, 0)
            .await
            .expect_err("line anchors must retain every selected line");
        assert_eq!(error.code, "comments_anchor_invalid");
        fs::remove_dir_all(root).expect("cleanup");
    }
}
