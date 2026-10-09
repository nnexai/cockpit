use std::collections::BTreeMap;

use cockpit_protocol::review::{ReviewFileDiff, ReviewSnapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::InspectionError;
use crate::project_store::{ProjectStore, atomic_write_json, read_json_bounded};

use super::ReviewService;
use super::git::RevisionTokens;
use super::snapshot::Change;

pub(super) const MAX_SNAPSHOTS: usize = 8;
const MAX_STORED_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;
// Viewer-bound payloads cannot reuse the former pane-bound on-disk schema.
const SNAPSHOT_PREFIX: &str = "snapshot-v2-";
const FILE_CACHE_PREFIX: &str = "file-v2-";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredSnapshot {
    pub(super) snapshot: ReviewSnapshot,
    /// Changed-file metadata is cheap enough to retain for the whole review.
    /// Diffs and frozen sources are populated only for files the user opens.
    pub(super) changes: BTreeMap<String, Change>,
    pub(super) created_at: String,
}

#[derive(Clone)]
pub(super) struct ReviewCacheEntry {
    pub(super) key: String,
    pub(super) revisions: RevisionTokens,
    pub(super) snapshot: ReviewSnapshot,
}

impl ReviewService {
    pub(super) async fn save_file_cache(
        &self,
        review_id: &str,
        diff: &ReviewFileDiff,
    ) -> Result<(), InspectionError> {
        let state = self.store.clone();
        let review_id = review_id.to_owned();
        let diff = diff.clone();
        tokio::task::spawn_blocking(move || {
            let name = file_cache_name(&review_id, &diff.file.file_id)?;
            let serialized = serde_json::to_vec(&diff)
                .map_err(|error| InspectionError::new("review_write", error.to_string()))?;
            if serialized.len() > MAX_STORED_SNAPSHOT_BYTES as usize {
                return Err(InspectionError::new(
                    "review_file_bounded",
                    "review file cache exceeds its per-file storage limit",
                ));
            }
            atomic_write_json(state.state_dir(), &name, &diff)
                .map_err(|error| InspectionError::new("review_write", error.to_string()))
        })
        .await
        .map_err(|error| InspectionError::new("review_task", error.to_string()))?
    }

    pub(super) async fn load_file_cache(
        &self,
        snapshot: &ReviewSnapshot,
        file_id: &str,
    ) -> Result<Option<ReviewFileDiff>, InspectionError> {
        let state = self.store.clone();
        let review_id = snapshot.review_id.clone();
        let binding_id = snapshot.binding_id.clone();
        let session_id = snapshot.session_id.clone();
        let viewer_id = snapshot.viewer_id.clone();
        let generation = snapshot.generation;
        let file_id = file_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let name = file_cache_name(&review_id, &file_id)?;
            match state.state_dir().symlink_metadata(&name) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    let diff: ReviewFileDiff =
                        read_json_bounded(state.state_dir(), &name, MAX_STORED_SNAPSHOT_BYTES)?;
                    if diff.file.file_id != file_id
                        || diff.review_id != review_id
                        || diff.generation != generation
                        || diff.binding_id != binding_id
                        || diff.session_id != session_id
                        || diff.viewer_id != viewer_id
                    {
                        return Err(InspectionError::new(
                            "review_read",
                            "review file cache identity does not match its key",
                        ));
                    }
                    Ok(Some(diff))
                }
                Ok(_) => Err(InspectionError::new(
                    "unsafe_path",
                    "review file cache is not a regular file",
                )),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(InspectionError::new("review_read", error.to_string())),
            }
        })
        .await
        .map_err(|error| InspectionError::new("review_task", error.to_string()))?
    }

    pub(super) async fn save_snapshot(
        &self,
        stored: StoredSnapshot,
    ) -> Result<(), InspectionError> {
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

    pub(super) async fn load_snapshot(
        &self,
        review_id: &str,
    ) -> Result<Option<StoredSnapshot>, InspectionError> {
        let review_id = review_id.to_owned();
        let state = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let name = snapshot_name(&review_id)?;
            match state.state_dir().symlink_metadata(&name) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    let stored: StoredSnapshot =
                        read_json_bounded(state.state_dir(), &name, MAX_STORED_SNAPSHOT_BYTES)?;
                    if stored.snapshot.review_id != review_id {
                        return Err(InspectionError::new(
                            "review_read",
                            "review snapshot identity does not match its key",
                        ));
                    }
                    Ok(Some(stored))
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

fn snapshot_name(id: &str) -> Result<String, InspectionError> {
    if Uuid::parse_str(id).is_err() {
        return Err(InspectionError::new(
            "review_invalid_id",
            "review snapshot identity must be a UUID",
        ));
    }
    Ok(format!("{SNAPSHOT_PREFIX}{id}.json"))
}

fn file_cache_name(review_id: &str, file_id: &str) -> Result<String, InspectionError> {
    let _ = snapshot_name(review_id)?;
    let digest = Sha256::digest(file_id.as_bytes());
    Ok(format!("{FILE_CACHE_PREFIX}{review_id}-{:x}.json", digest))
}

fn snapshot_cache_id<'a>(name: &'a str, prefix: &str) -> Option<&'a str> {
    let id = name.strip_prefix(prefix)?.strip_suffix(".json")?;
    Uuid::parse_str(id).ok().map(|_| id)
}

fn file_cache_review_id<'a>(name: &'a str, prefix: &str) -> Option<&'a str> {
    let key = name.strip_prefix(prefix)?.strip_suffix(".json")?;
    let id = key.get(..36)?;
    Uuid::parse_str(id).ok()?;
    let digest = key.get(36..)?.strip_prefix('-')?;
    (digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(id)
}

fn prune_snapshots(state: &ProjectStore) -> Result<(), InspectionError> {
    let mut snapshots = Vec::new();
    let mut file_caches = Vec::new();
    for entry in state
        .state_dir()
        .entries()
        .map_err(|error| InspectionError::new("review_read", error.to_string()))?
    {
        let entry =
            entry.map_err(|error| InspectionError::new("review_read", error.to_string()))?;
        let name = entry.file_name();
        let Some(name_text) = name.to_str() else {
            continue;
        };
        if let Some(id) = file_cache_review_id(name_text, FILE_CACHE_PREFIX) {
            file_caches.push((name.to_owned(), id.to_owned()));
            continue;
        }
        let Some(id) = snapshot_cache_id(name_text, SNAPSHOT_PREFIX) else {
            continue;
        };
        let stored: StoredSnapshot =
            read_json_bounded(state.state_dir(), name_text, MAX_STORED_SNAPSHOT_BYTES)?;
        snapshots.push((name.to_owned(), id.to_owned(), stored.created_at));
    }
    snapshots.sort_by(|left, right| right.2.cmp(&left.2));
    let retained: std::collections::BTreeSet<String> = snapshots
        .iter()
        .take(MAX_SNAPSHOTS)
        .map(|(_, id, _)| id.clone())
        .collect();
    for (name, id, _) in snapshots.into_iter().skip(MAX_SNAPSHOTS) {
        state
            .state_dir()
            .remove_file(name)
            .map_err(|error| InspectionError::new("review_write", error.to_string()))?;
        for (cache_name, _) in file_caches.iter().filter(|(_, cache_id)| cache_id == &id) {
            state
                .state_dir()
                .remove_file(cache_name)
                .map_err(|error| InspectionError::new("review_write", error.to_string()))?;
        }
    }
    // Orphaned per-file caches from a crash are safe to remove. Caches for
    // retained snapshots remain immutable so frozen comment anchors survive.
    for (cache_name, cache_id) in file_caches {
        if !retained.contains(cache_id.as_str()) {
            let _ = state.state_dir().remove_file(cache_name);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
