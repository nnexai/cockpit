use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use cap_fs_ext::OpenOptionsFollowExt;
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::browser_feedback::{
    BrowserAnnotation, BrowserCaptureContext, BrowserCaptureSaved, BrowserCaptureSubmission,
    BrowserFeedbackAck, BrowserFeedbackCapture, BrowserFeedbackResponse, BrowserPageEvidence,
    BrowserPoint, BrowserRect, BrowserViewport,
};
use cockpit_protocol::browser::BrowserFeedbackDeliveryStatus;
use cockpit_protocol::comment_paste::{CommentPasteState, CommentPasteTarget};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::InspectionError;
use crate::project_store::{
    atomic_write_json, open_dir_nofollow_absolute, prepare_root, read_json_bounded,
};

const DEFAULT_RETENTION_SECONDS: u64 = 3_600;
const DEFAULT_MAX_STORE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DELIVERY_BYTES: u64 = 64 * 1024;
const MAX_PNG_BYTES: usize = 4 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 6 * 1024 * 1024;
const MAX_DIMENSION: u32 = 8_192;
const MAX_PIXELS: u64 = 16_000_000;
const MAX_ANNOTATIONS: usize = 64;
const MAX_POINTS: usize = 8_192;
const MAX_PENDING_CAPTURES: usize = 64;
const MAX_RECORD_BYTES: u64 = MAX_REQUEST_BYTES as u64;
const MAX_ENTRIES: usize = 8_192;
const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

#[derive(Clone, Debug)]
pub struct BrowserFeedbackOptions {
    pub retention_seconds: u64,
    pub max_store_bytes: u64,
}

impl Default for BrowserFeedbackOptions {
    fn default() -> Self {
        Self {
            retention_seconds: DEFAULT_RETENTION_SECONDS,
            max_store_bytes: DEFAULT_MAX_STORE_BYTES,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCapture {
    id: String,
    context: BrowserCaptureContext,
    page: BrowserPageEvidence,
    annotations: Vec<BrowserAnnotation>,
    pending_ids: Vec<String>,
    image_name: String,
    payload_digest: String,
    created_at: u64,
    expires_at: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserDeliveryReceipt {
    pub operation_id: String,
    pub association_key: String,
    pub selected_ids: Vec<String>,
    pub state: CommentPasteState,
    pub target: Option<CommentPasteTarget>,
    pub acknowledged_ids: Vec<String>,
    pub message: String,
    pub created_at: u64,
    pub updated_at: u64,
}

pub struct BrowserFeedbackStore {
    feedback_root: PathBuf,
    artifacts_root: PathBuf,
    options: BrowserFeedbackOptions,
    mutation: Mutex<()>,
}

impl std::fmt::Debug for BrowserFeedbackStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserFeedbackStore")
            .field("feedback_root", &self.feedback_root)
            .field("artifacts_root", &self.artifacts_root)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl BrowserFeedbackStore {
    pub fn new(
        state_root: PathBuf,
        options: BrowserFeedbackOptions,
    ) -> Result<Self, InspectionError> {
        if options.retention_seconds == 0 || options.max_store_bytes == 0 {
            return Err(InspectionError::new(
                "browser_feedback_options",
                "retention and store limits must be positive",
            ));
        }
        let browser_root = state_root.join("browser");
        let (_, _) = prepare_root(&browser_root, "browser")?;
        let (feedback_root, _) = prepare_root(&browser_root.join("feedback"), "browser feedback")?;
        let (artifacts_root, _) =
            prepare_root(&browser_root.join("artifacts"), "browser artifacts")?;
        Ok(Self {
            feedback_root,
            artifacts_root,
            options,
            mutation: Mutex::new(()),
        })
    }

    pub fn save(
        &self,
        context: BrowserCaptureContext,
        submission: BrowserCaptureSubmission,
    ) -> Result<BrowserCaptureSaved, InspectionError> {
        let _guard = self.lock_mutation()?;
        self.prune_locked()?;
        validate_capture(&context, &submission)?;
        let png = decode_png(&submission.png_base64, &submission.page)?;
        let digest = payload_digest(&context, &submission)?;
        let record_name = record_name(&submission.capture_id);
        let feedback = self.feedback_dir()?;

        if let Some(existing) = self.load_capture_if_present(&feedback, &record_name)? {
            if existing.payload_digest != digest
                || existing.context != context
                || existing.page != submission.page
                || existing.annotations != submission.annotations
            {
                return Err(InspectionError::new(
                    "browser_feedback_conflict",
                    "capture_id was already saved with different evidence",
                ));
            }
            return Ok(self.saved_result(&existing));
        }

        let records = self.load_all_captures(&feedback)?;
        let mut annotation_ids = HashSet::new();
        let mut pending_captures = 0usize;
        for record in &records {
            if record.context.association_key != context.association_key {
                continue;
            }
            for annotation in &record.annotations {
                if !annotation_ids.insert(annotation.id.clone()) {
                    return Err(InspectionError::new(
                        "browser_feedback_corrupt",
                        "annotation IDs are not globally unique within an association",
                    ));
                }
            }
            if !record.pending_ids.is_empty() {
                pending_captures = pending_captures.saturating_add(1);
            }
        }
        for annotation in &submission.annotations {
            if !annotation_ids.insert(annotation.id.clone()) {
                return Err(InspectionError::new(
                    "browser_feedback_annotation_conflict",
                    "annotation ID was already saved for this association",
                ));
            }
        }
        // Reject before writing either artifact so the bounded store never evicts evidence.
        if !submission.annotations.is_empty() && pending_captures >= MAX_PENDING_CAPTURES {
            return Err(InspectionError::new(
                "browser_feedback_pending_limit",
                format!(
                    "association already has the maximum of {MAX_PENDING_CAPTURES} pending captures"
                ),
            ));
        }
        let now = unix_now()?;
        let pending_ids = submission
            .annotations
            .iter()
            .map(|a| a.id.clone())
            .collect();
        let stored = StoredCapture {
            id: submission.capture_id.clone(),
            context,
            page: submission.page.clone(),
            annotations: submission.annotations.clone(),
            pending_ids,
            image_name: image_name(&submission.capture_id),
            payload_digest: digest,
            created_at: now,
            expires_at: if submission.annotations.is_empty() {
                Some(expiry(now, self.options.retention_seconds))
            } else {
                None
            },
        };
        let serialized = serde_json::to_vec_pretty(&stored)
            .map_err(|error| InspectionError::new("browser_feedback_write", error.to_string()))?;
        if serialized.len() as u64 > MAX_RECORD_BYTES {
            return Err(InspectionError::new(
                "browser_feedback_record_limit",
                "feedback record exceeds its bounded size",
            ));
        }
        let usage = self.storage_usage()?;
        let required = usage
            .saturating_add(png.len() as u64)
            .saturating_add(serialized.len() as u64);
        if required > self.options.max_store_bytes {
            return Err(InspectionError::new(
                "browser_feedback_store_limit",
                "saving this capture would exceed the configured store limit",
            ));
        }

        let image_name = stored.image_name.clone();
        if let Err(error) = atomic_write_bytes(&self.artifacts_dir()?, &image_name, &png) {
            return Err(error);
        }
        if let Err(error) = atomic_write_json(&feedback, &record_name, &stored) {
            let _ = self.artifacts_dir()?.remove_file(&image_name);
            return Err(InspectionError::new(
                "browser_feedback_write",
                error.to_string(),
            ));
        }
        Ok(self.saved_result(&stored))
    }

    pub fn list(&self, association_key: &str) -> Result<BrowserFeedbackResponse, InspectionError> {
        validate_association_key(association_key)?;
        let _guard = self.lock_mutation()?;
        let feedback = self.feedback_dir()?;
        let now = unix_now()?;
        let mut captures = Vec::new();
        let mut pending_count = 0usize;
        for stored in self.load_all_captures(&feedback)? {
            if stored.context.association_key != association_key
                || stored.pending_ids.is_empty()
                || stored.expires_at.is_some_and(|expires| expires <= now)
            {
                continue;
            }
            pending_count = pending_count.saturating_add(stored.pending_ids.len());
            captures.push(self.public_capture(&stored));
        }
        captures.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(BrowserFeedbackResponse {
            captures,
            pending_count,
            retention_seconds: self.options.retention_seconds,
        })
    }
    pub(crate) fn read_image(
        &self,
        association_key: &str,
        capture_id: &str,
    ) -> Result<Vec<u8>, InspectionError> {
        validate_association_key(association_key)?;
        validate_uuid(capture_id, "capture ID")?;
        let _guard = self.lock_mutation()?;
        let feedback = self.feedback_dir()?;
        let stored = self
            .load_capture_if_present(&feedback, &record_name(capture_id))?
            .ok_or_else(|| {
                InspectionError::new("browser_feedback_not_found", "capture is unavailable")
            })?;
        if stored.context.association_key != association_key {
            return Err(InspectionError::new(
                "browser_feedback_not_found",
                "capture is unavailable",
            ));
        }
        let now = unix_now()?;
        if stored.expires_at.is_some_and(|expires| expires <= now) {
            return Err(InspectionError::new(
                "browser_feedback_not_found",
                "capture is unavailable",
            ));
        }
        let artifacts = self.artifacts_dir()?;
        let mut options = OpenOptions::new();
        options.read(true).follow(cap_fs_ext::FollowSymlinks::No);
        let mut file = artifacts
            .open_with(&stored.image_name, &options)
            .map_err(|error| {
                InspectionError::new(
                    "browser_feedback_read",
                    format!("capture image is unavailable: {error}"),
                )
            })?;
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut file)
            .take((MAX_PNG_BYTES as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| InspectionError::new("browser_feedback_read", error.to_string()))?;
        if bytes.len() > MAX_PNG_BYTES {
            return Err(InspectionError::new(
                "unsafe_path",
                "capture image is not bounded",
            ));
        }
        Ok(bytes)
    }

    pub(crate) fn load_delivery(
        &self,
        operation_id: &str,
    ) -> Result<Option<BrowserDeliveryReceipt>, InspectionError> {
        validate_operation_id(operation_id)?;
        let _guard = self.lock_mutation()?;
        let feedback = self.feedback_dir()?;
        let name = delivery_record_name(operation_id);
        match feedback.symlink_metadata(&name) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
                InspectionError::new("unsafe_path", "delivery receipt is not a regular file"),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(InspectionError::new(
                "browser_feedback_read",
                error.to_string(),
            )),
            Ok(_) => {
                let receipt: BrowserDeliveryReceipt =
                    read_json_bounded(&feedback, &name, MAX_DELIVERY_BYTES)?;
                validate_delivery(&receipt)?;
                if receipt.operation_id != operation_id {
                    return Err(InspectionError::new(
                        "browser_feedback_corrupt",
                        "delivery receipt identity is invalid",
                    ));
                }
                Ok(Some(receipt))
            }
        }
    }

    pub(crate) fn save_delivery(
        &self,
        mut receipt: BrowserDeliveryReceipt,
    ) -> Result<BrowserDeliveryReceipt, InspectionError> {
        validate_delivery(&receipt)?;
        let _guard = self.lock_mutation()?;
        let now = unix_now()?;
        if receipt.created_at == 0 {
            receipt.created_at = now;
        }
        receipt.updated_at = now;
        let serialized = serde_json::to_vec_pretty(&receipt)
            .map_err(|error| InspectionError::new("browser_feedback_write", error.to_string()))?;
        if serialized.len() > MAX_DELIVERY_BYTES as usize {
            return Err(InspectionError::new(
                "browser_feedback_receipt_limit",
                "delivery receipt exceeds its bounded size",
            ));
        }
        let feedback = self.feedback_dir()?;
        let record = delivery_record_name(&receipt.operation_id);
        let previous_size = match feedback.symlink_metadata(&record) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "delivery receipt is not a regular file",
                ));
            }
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => {
                return Err(InspectionError::new(
                    "browser_feedback_read",
                    error.to_string(),
                ));
            }
        };
        let usage = self.storage_usage()?.saturating_sub(previous_size);
        if usage.saturating_add(serialized.len() as u64) > self.options.max_store_bytes {
            return Err(InspectionError::new(
                "browser_feedback_store_limit",
                "saving this delivery receipt would exceed the configured store limit",
            ));
        }
        atomic_write_json(&feedback, &record, &receipt)
            .map_err(|error| InspectionError::new("browser_feedback_write", error.to_string()))?;
        Ok(receipt)
    }

    pub fn ack(
        &self,
        association_key: &str,
        ids: &[String],
    ) -> Result<BrowserFeedbackAck, InspectionError> {
        validate_association_key(association_key)?;
        if ids.len() > MAX_PENDING_CAPTURES * MAX_ANNOTATIONS {
            return Err(InspectionError::new(
                "browser_feedback_ack_limit",
                "acknowledgement contains too many IDs",
            ));
        }
        for id in ids {
            validate_uuid(id, "annotation ID")?;
        }
        let _guard = self.lock_mutation()?;
        self.prune_locked()?;
        let feedback = self.feedback_dir()?;
        let requested: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let mut acknowledged = Vec::new();
        let now = unix_now()?;
        for mut stored in self.load_all_captures(&feedback)? {
            if stored.context.association_key != association_key {
                continue;
            }
            let before = stored.pending_ids.len();
            stored.pending_ids.retain(|id| {
                if requested.contains(id.as_str()) {
                    acknowledged.push(id.clone());
                    false
                } else {
                    true
                }
            });
            if stored.pending_ids.len() == before {
                continue;
            }
            if stored.pending_ids.is_empty() && before != 0 && stored.expires_at.is_none() {
                stored.expires_at = Some(expiry(now, self.options.retention_seconds));
            }
            atomic_write_json(&feedback, &record_name(&stored.id), &stored).map_err(|error| {
                InspectionError::new("browser_feedback_write", error.to_string())
            })?;
        }
        acknowledged.sort();
        acknowledged.dedup();
        let remaining = self
            .load_all_captures(&feedback)?
            .into_iter()
            .filter(|capture| capture.context.association_key == association_key)
            .map(|capture| capture.pending_ids.len())
            .sum();
        Ok(BrowserFeedbackAck {
            acknowledged_ids: acknowledged,
            remaining,
        })
    }

    pub fn prune(&self) -> Result<(), InspectionError> {
        let _guard = self.lock_mutation()?;
        self.prune_locked()
    }

    fn prune_locked(&self) -> Result<(), InspectionError> {
        let feedback = self.feedback_dir()?;
        let artifacts = self.artifacts_dir()?;
        let now = unix_now()?;
        for stored in self.load_all_captures(&feedback)? {
            if stored.expires_at.is_some_and(|expires| expires <= now) {
                remove_regular_file(
                    &feedback,
                    &record_name(&stored.id),
                    "browser_feedback_prune",
                )?;
                remove_regular_file(&artifacts, &stored.image_name, "browser_feedback_prune")?;
            }
        }
        self.prune_delivery_locked(&feedback, now)?;
        Ok(())
    }

    fn prune_delivery_locked(&self, feedback: &Dir, now: u64) -> Result<(), InspectionError> {
        for (index, entry) in feedback
            .entries()
            .map_err(|error| InspectionError::new("browser_feedback_read", error.to_string()))?
            .enumerate()
        {
            if index >= MAX_ENTRIES {
                return Err(InspectionError::new(
                    "browser_feedback_bounded",
                    "feedback directory exceeded its entry limit",
                ));
            }
            let entry = entry.map_err(|error| {
                InspectionError::new("browser_feedback_read", error.to_string())
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(operation_id) = name
                .strip_prefix("delivery-")
                .and_then(|value| value.strip_suffix(".json"))
            else {
                continue;
            };
            validate_operation_id(operation_id)?;
            let file_type = entry.file_type().map_err(|error| {
                InspectionError::new("browser_feedback_read", error.to_string())
            })?;
            if file_type.is_symlink() || !file_type.is_file() {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "delivery receipt is not a regular file",
                ));
            }
            let receipt: BrowserDeliveryReceipt =
                read_json_bounded(feedback, name, MAX_DELIVERY_BYTES)?;
            validate_delivery(&receipt)?;
            if receipt.operation_id != operation_id {
                return Err(InspectionError::new(
                    "browser_feedback_corrupt",
                    "delivery receipt identity is invalid",
                ));
            }
            if matches!(
                receipt.state,
                CommentPasteState::Accepted | CommentPasteState::Rejected
            ) && receipt
                .updated_at
                .saturating_add(self.options.retention_seconds)
                <= now
            {
                remove_regular_file(feedback, name, "browser_feedback_prune")?;
            }
        }
        Ok(())
    }

    fn lock_mutation(&self) -> Result<std::sync::MutexGuard<'_, ()>, InspectionError> {
        self.mutation.lock().map_err(|_| {
            InspectionError::new("browser_feedback_lock", "feedback store lock was poisoned")
        })
    }

    fn feedback_dir(&self) -> Result<Dir, InspectionError> {
        open_dir_nofollow_absolute(&self.feedback_root)
            .map_err(|error| InspectionError::new("browser_feedback_read", error.to_string()))
    }

    fn artifacts_dir(&self) -> Result<Dir, InspectionError> {
        open_dir_nofollow_absolute(&self.artifacts_root)
            .map_err(|error| InspectionError::new("browser_feedback_read", error.to_string()))
    }

    fn load_capture_if_present(
        &self,
        dir: &Dir,
        name: &str,
    ) -> Result<Option<StoredCapture>, InspectionError> {
        match dir.symlink_metadata(name) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
                InspectionError::new("unsafe_path", "feedback record is not a regular file"),
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(InspectionError::new(
                "browser_feedback_read",
                error.to_string(),
            )),
            Ok(_) => {
                let stored: StoredCapture = read_json_bounded(dir, name, MAX_RECORD_BYTES)?;
                let id = name
                    .strip_prefix("capture-")
                    .and_then(|value| value.strip_suffix(".json"))
                    .ok_or_else(|| {
                        InspectionError::new(
                            "browser_feedback_corrupt",
                            "invalid feedback record name",
                        )
                    })?;
                validate_stored(id, &stored)?;
                let artifacts = self.artifacts_dir()?;
                let image = artifacts
                    .symlink_metadata(&stored.image_name)
                    .map_err(|error| {
                        InspectionError::new(
                            "browser_feedback_read",
                            format!("capture image is unavailable: {error}"),
                        )
                    })?;
                if image.file_type().is_symlink()
                    || !image.is_file()
                    || image.len() > MAX_PNG_BYTES as u64
                {
                    return Err(InspectionError::new(
                        "unsafe_path",
                        "capture image is not a bounded regular file",
                    ));
                }
                Ok(Some(stored))
            }
        }
    }

    fn load_all_captures(&self, dir: &Dir) -> Result<Vec<StoredCapture>, InspectionError> {
        let artifacts = self.artifacts_dir()?;
        let mut captures = Vec::new();
        for (index, entry) in dir
            .entries()
            .map_err(|error| InspectionError::new("browser_feedback_read", error.to_string()))?
            .enumerate()
        {
            if index >= MAX_ENTRIES {
                return Err(InspectionError::new(
                    "browser_feedback_bounded",
                    "feedback directory exceeded its entry limit",
                ));
            }
            let entry = entry.map_err(|error| {
                InspectionError::new("browser_feedback_read", error.to_string())
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(id) = name
                .strip_prefix("capture-")
                .and_then(|value| value.strip_suffix(".json"))
            else {
                continue;
            };
            validate_uuid(id, "capture ID")?;
            let file_type = entry.file_type().map_err(|error| {
                InspectionError::new("browser_feedback_read", error.to_string())
            })?;
            if file_type.is_symlink() || !file_type.is_file() {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "feedback record is not a regular file",
                ));
            }
            let stored: StoredCapture = read_json_bounded(dir, name, MAX_RECORD_BYTES)?;
            validate_stored(id, &stored)?;
            let image = artifacts
                .symlink_metadata(&stored.image_name)
                .map_err(|error| {
                    InspectionError::new(
                        "browser_feedback_read",
                        format!("capture image is unavailable: {error}"),
                    )
                })?;
            if image.file_type().is_symlink()
                || !image.is_file()
                || image.len() > MAX_PNG_BYTES as u64
            {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "capture image is not a bounded regular file",
                ));
            }
            captures.push(stored);
        }
        Ok(captures)
    }

    fn storage_usage(&self) -> Result<u64, InspectionError> {
        let mut total = 0u64;
        for root in [&self.feedback_root, &self.artifacts_root] {
            let dir = open_dir_nofollow_absolute(root).map_err(|error| {
                InspectionError::new("browser_feedback_read", error.to_string())
            })?;
            for (index, entry) in dir
                .entries()
                .map_err(|error| InspectionError::new("browser_feedback_read", error.to_string()))?
                .enumerate()
            {
                if index >= MAX_ENTRIES {
                    return Err(InspectionError::new(
                        "browser_feedback_bounded",
                        "feedback storage exceeded its entry limit",
                    ));
                }
                let entry = entry.map_err(|error| {
                    InspectionError::new("browser_feedback_read", error.to_string())
                })?;
                let file_type = entry.file_type().map_err(|error| {
                    InspectionError::new("browser_feedback_read", error.to_string())
                })?;
                if file_type.is_symlink() || !file_type.is_file() {
                    return Err(InspectionError::new(
                        "unsafe_path",
                        "feedback storage contains a non-regular file",
                    ));
                }
                total = total.saturating_add(
                    entry
                        .metadata()
                        .map_err(|error| {
                            InspectionError::new("browser_feedback_read", error.to_string())
                        })?
                        .len(),
                );
            }
        }
        Ok(total)
    }

    fn saved_result(&self, stored: &StoredCapture) -> BrowserCaptureSaved {
        BrowserCaptureSaved {
            capture_id: stored.id.clone(),
            annotation_ids: stored.annotations.iter().map(|a| a.id.clone()).collect(),
            image_path: self.image_path(&stored.image_name),
            pending_count: stored.pending_ids.len(),
        }
    }

    fn public_capture(&self, stored: &StoredCapture) -> BrowserFeedbackCapture {
        BrowserFeedbackCapture {
            id: stored.id.clone(),
            context: stored.context.clone(),
            page: stored.page.clone(),
            annotations: stored.annotations.clone(),
            pending_ids: stored.pending_ids.clone(),
            image_path: self.image_path(&stored.image_name),
        }
    }

    fn image_path(&self, name: &str) -> String {
        self.artifacts_root
            .join(name)
            .to_string_lossy()
            .into_owned()
    }
    pub(crate) fn list_delivery_statuses(
        &self,
        association_key: &str,
        captures: &[BrowserFeedbackCapture],
    ) -> Result<Vec<BrowserFeedbackDeliveryStatus>, InspectionError> {
        validate_association_key(association_key)?;
        if captures.len() > MAX_PENDING_CAPTURES {
            return Err(InspectionError::new(
                "browser_feedback_bounded",
                "association exceeded its pending capture limit",
            ));
        }
        if captures
            .iter()
            .any(|capture| capture.context.association_key != association_key)
        {
            return Err(InspectionError::new(
                "browser_feedback_corrupt",
                "pending captures contain a different association",
            ));
        }

        let mut capture_by_annotation = HashMap::new();
        for (index, capture) in captures.iter().enumerate() {
            for id in &capture.pending_ids {
                if capture_by_annotation.insert(id.as_str(), index).is_some() {
                    return Err(InspectionError::new(
                        "browser_feedback_corrupt",
                        "pending annotation IDs are not unique within an association",
                    ));
                }
            }
        }
        let mut selected: Vec<Option<(u8, u64, String, BrowserFeedbackDeliveryStatus)>> =
            vec![None; captures.len()];

        let _guard = self.lock_mutation()?;
        let feedback = self.feedback_dir()?;
        for (index, entry) in feedback
            .entries()
            .map_err(|error| InspectionError::new("browser_feedback_read", error.to_string()))?
            .enumerate()
        {
            if index >= MAX_ENTRIES {
                return Err(InspectionError::new(
                    "browser_feedback_bounded",
                    "feedback directory exceeded its entry limit",
                ));
            }
            let entry = entry.map_err(|error| {
                InspectionError::new("browser_feedback_read", error.to_string())
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(operation_id) = name
                .strip_prefix("delivery-")
                .and_then(|value| value.strip_suffix(".json"))
            else {
                continue;
            };
            validate_operation_id(operation_id)?;
            let file_type = entry.file_type().map_err(|error| {
                InspectionError::new("browser_feedback_read", error.to_string())
            })?;
            if file_type.is_symlink() || !file_type.is_file() {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "delivery receipt is not a regular file",
                ));
            }
            let receipt: BrowserDeliveryReceipt =
                read_json_bounded(&feedback, name, MAX_DELIVERY_BYTES)?;
            validate_delivery(&receipt)?;
            if receipt.operation_id != operation_id {
                return Err(InspectionError::new(
                    "browser_feedback_corrupt",
                    "delivery receipt identity is invalid",
                ));
            }
            if receipt.association_key != association_key {
                continue;
            }

            let priority = u8::from(matches!(
                receipt.state,
                CommentPasteState::Pending | CommentPasteState::OutcomeUnknown
            ));
            for id in &receipt.selected_ids {
                let Some(&capture_index) = capture_by_annotation.get(id.as_str()) else {
                    continue;
                };
                let current = &selected[capture_index];
                let is_newer = current.as_ref().map_or(true, |(current_priority, updated_at, op, _)| {
                    priority > *current_priority
                        || (priority == *current_priority
                            && (receipt.updated_at > *updated_at
                                || (receipt.updated_at == *updated_at
                                    && receipt.operation_id.as_str() > op.as_str())))
                });
                if is_newer {
                    selected[capture_index] = Some((
                        priority,
                        receipt.updated_at,
                        receipt.operation_id.clone(),
                        BrowserFeedbackDeliveryStatus {
                            capture_id: captures[capture_index].id.clone(),
                            operation_id: receipt.operation_id.clone(),
                            selected_ids: receipt.selected_ids.clone(),
                            state: receipt.state,
                            message: receipt.message.clone(),
                        },
                    ));
                }
            }
        }
        Ok(selected
            .into_iter()
            .flatten()
            .map(|(_, _, _, status)| status)
            .collect())
    }

    pub(crate) fn has_delivery_overlap(
        &self,
        association_key: &str,
        ids: &[String],
    ) -> Result<bool, InspectionError> {
        validate_association_key(association_key)?;
        let requested: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let _guard = self.lock_mutation()?;
        let feedback = self.feedback_dir()?;
        for (index, entry) in feedback
            .entries()
            .map_err(|error| InspectionError::new("browser_feedback_read", error.to_string()))?
            .enumerate()
        {
            if index >= MAX_ENTRIES {
                return Err(InspectionError::new(
                    "browser_feedback_bounded",
                    "feedback directory exceeded its entry limit",
                ));
            }
            let entry = entry.map_err(|error| {
                InspectionError::new("browser_feedback_read", error.to_string())
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(operation_id) = name
                .strip_prefix("delivery-")
                .and_then(|value| value.strip_suffix(".json"))
            else {
                continue;
            };
            validate_operation_id(operation_id)?;
            let file_type = entry.file_type().map_err(|error| {
                InspectionError::new("browser_feedback_read", error.to_string())
            })?;
            if file_type.is_symlink() || !file_type.is_file() {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "delivery receipt is not a regular file",
                ));
            }
            let receipt: BrowserDeliveryReceipt =
                read_json_bounded(&feedback, name, MAX_DELIVERY_BYTES)?;
            validate_delivery(&receipt)?;
            if receipt.association_key == association_key
                && matches!(
                    receipt.state,
                    CommentPasteState::Pending | CommentPasteState::OutcomeUnknown
                )
                && receipt
                    .selected_ids
                    .iter()
                    .any(|id| requested.contains(id.as_str()))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn validate_capture(
    context: &BrowserCaptureContext,
    submission: &BrowserCaptureSubmission,
) -> Result<(), InspectionError> {
    validate_association_key(&context.association_key)?;
    validate_text(&context.browser_instance, "browser instance", 512)?;
    validate_text(&submission.browser_instance, "browser instance", 512)?;
    if context.association_key != submission.association_key
        || context.browser_instance != submission.browser_instance
    {
        return Err(InspectionError::new(
            "browser_feedback_association",
            "capture context does not match submission",
        ));
    }
    validate_uuid(&submission.capture_id, "capture ID")?;
    validate_text(&context.session_id, "session ID", 256)?;
    validate_text(&context.space_id, "space ID", 256)?;
    validate_text(&context.space_label, "space label", 512)?;
    validate_text(&context.playwright_session, "playwright session", 256)?;
    validate_text(&context.working_directory, "working directory", 4096)?;
    validate_text(&context.invocation, "invocation", 4096)?;
    validate_page(&submission.page)?;
    if submission.annotations.len() > MAX_ANNOTATIONS {
        return Err(InspectionError::new(
            "browser_feedback_annotations",
            "capture has too many annotations",
        ));
    }
    let mut points = 0usize;
    let mut ids = HashSet::new();
    for annotation in &submission.annotations {
        validate_annotation(annotation, &submission.page)?;
        points = points.saturating_add(annotation.points.len());
        if !ids.insert(annotation.id.as_str()) {
            return Err(InspectionError::new(
                "browser_feedback_annotations",
                "annotation IDs must be unique",
            ));
        }
    }
    if points > MAX_POINTS {
        return Err(InspectionError::new(
            "browser_feedback_points",
            "capture has too many drawing points",
        ));
    }
    let request_bytes = serde_json::to_vec(submission)
        .map_err(|error| InspectionError::new("browser_feedback_request", error.to_string()))?;
    if request_bytes.len() > MAX_REQUEST_BYTES {
        return Err(InspectionError::new(
            "browser_feedback_request",
            "capture request exceeds its byte limit",
        ));
    }
    if submission.png_base64.len() > MAX_REQUEST_BYTES {
        return Err(InspectionError::new(
            "browser_feedback_request",
            "capture request exceeds its byte limit",
        ));
    }
    Ok(())
}

fn validate_page(page: &BrowserPageEvidence) -> Result<(), InspectionError> {
    validate_text(&page.url, "page URL", 8 * 1024)?;
    validate_text(&page.title, "page title", 2 * 1024)?;
    validate_text(&page.document_id, "document ID", 256)?;
    validate_text(&page.captured_at, "capture timestamp", 128)?;
    validate_viewport(&page.viewport)?;
    if page.image_width == 0
        || page.image_height == 0
        || page.image_width > MAX_DIMENSION
        || page.image_height > MAX_DIMENSION
    {
        return Err(InspectionError::new(
            "browser_feedback_dimensions",
            "image dimensions are outside the supported bounds",
        ));
    }
    if u64::from(page.image_width) * u64::from(page.image_height) > MAX_PIXELS {
        return Err(InspectionError::new(
            "browser_feedback_dimensions",
            "image has too many pixels",
        ));
    }
    Ok(())
}

fn validate_viewport(viewport: &BrowserViewport) -> Result<(), InspectionError> {
    for (name, value) in [
        ("viewport width", viewport.width),
        ("viewport height", viewport.height),
        ("scroll x", viewport.scroll_x),
        ("scroll y", viewport.scroll_y),
        ("device pixel ratio", viewport.device_pixel_ratio),
        ("visual scale", viewport.visual_scale),
    ] {
        if !value.is_finite() {
            return Err(InspectionError::new(
                "browser_feedback_geometry",
                format!("{name} must be finite"),
            ));
        }
    }
    if viewport.width <= 0.0
        || viewport.height <= 0.0
        || viewport.device_pixel_ratio <= 0.0
        || viewport.visual_scale <= 0.0
    {
        return Err(InspectionError::new(
            "browser_feedback_geometry",
            "viewport dimensions and scales must be positive",
        ));
    }
    Ok(())
}

fn validate_annotation(
    annotation: &BrowserAnnotation,
    page: &BrowserPageEvidence,
) -> Result<(), InspectionError> {
    validate_uuid(&annotation.id, "annotation ID")?;
    validate_text(&annotation.comment, "annotation comment", 4 * 1024)?;
    validate_text(&annotation.color, "annotation color", 128)?;
    for point in &annotation.points {
        validate_point(point, page.image_width, page.image_height)?;
    }
    if let Some(bounds) = &annotation.bounds {
        validate_rect(bounds, page.image_width, page.image_height)?;
    }
    if let Some(element) = &annotation.element {
        validate_text(&element.tag, "element tag", 128)?;
        validate_display_text(&element.text, "element text", 8 * 1024)?;
        validate_optional_text(element.role.as_deref(), "element role", 512)?;
        validate_optional_text(element.name.as_deref(), "element name", 512)?;
        validate_display_text(&element.excerpt, "element excerpt", 4 * 1024)?;
        if element.locators.len() > 16 {
            return Err(InspectionError::new(
                "browser_feedback_element",
                "element has too many locators",
            ));
        }
        for locator in &element.locators {
            validate_text(locator, "element locator", 1024)?;
        }
    }
    Ok(())
}

fn validate_point(point: &BrowserPoint, width: u32, height: u32) -> Result<(), InspectionError> {
    if !point.x.is_finite()
        || !point.y.is_finite()
        || point.x < 0.0
        || point.y < 0.0
        || point.x > f64::from(width)
        || point.y > f64::from(height)
    {
        return Err(InspectionError::new(
            "browser_feedback_geometry",
            "point is outside the captured image",
        ));
    }
    Ok(())
}

fn validate_rect(rect: &BrowserRect, width: u32, height: u32) -> Result<(), InspectionError> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.x < 0.0
        || rect.y < 0.0
        || rect.width < 0.0
        || rect.height < 0.0
        || rect.x + rect.width > f64::from(width)
        || rect.y + rect.height > f64::from(height)
    {
        return Err(InspectionError::new(
            "browser_feedback_geometry",
            "annotation bounds are outside the captured image",
        ));
    }
    Ok(())
}

fn validate_stored(id: &str, stored: &StoredCapture) -> Result<(), InspectionError> {
    if id != stored.id || stored.image_name != image_name(id) {
        return Err(InspectionError::new(
            "browser_feedback_corrupt",
            "feedback record identity is invalid",
        ));
    }
    validate_uuid(&stored.id, "capture ID")?;
    validate_association_key(&stored.context.association_key)?;
    validate_text(&stored.context.browser_instance, "browser instance", 512)?;
    validate_text(&stored.context.session_id, "session ID", 256)?;
    validate_text(&stored.context.space_id, "space ID", 256)?;
    validate_text(&stored.context.space_label, "space label", 512)?;
    validate_text(
        &stored.context.playwright_session,
        "playwright session",
        256,
    )?;
    validate_text(&stored.context.working_directory, "working directory", 4096)?;
    validate_text(&stored.context.invocation, "invocation", 4096)?;
    validate_page(&stored.page)?;
    if stored.annotations.len() > MAX_ANNOTATIONS {
        return Err(InspectionError::new(
            "browser_feedback_corrupt",
            "feedback record has too many annotations",
        ));
    }
    let mut annotation_ids = HashSet::new();
    for annotation in &stored.annotations {
        validate_annotation(annotation, &stored.page)?;
        if !annotation_ids.insert(annotation.id.as_str()) {
            return Err(InspectionError::new(
                "browser_feedback_corrupt",
                "feedback annotation IDs are duplicated",
            ));
        }
    }
    let pending: HashSet<&str> = stored.pending_ids.iter().map(String::as_str).collect();
    if pending.len() != stored.pending_ids.len()
        || stored
            .pending_ids
            .iter()
            .any(|id| !annotation_ids.contains(id.as_str()))
        || (stored.pending_ids.is_empty() && stored.expires_at.is_none())
        || (!stored.pending_ids.is_empty() && stored.expires_at.is_some())
    {
        return Err(InspectionError::new(
            "browser_feedback_corrupt",
            "feedback record pending IDs are invalid",
        ));
    }
    Ok(())
}

fn decode_png(encoded: &str, page: &BrowserPageEvidence) -> Result<Vec<u8>, InspectionError> {
    let bytes = BASE64
        .decode(encoded)
        .map_err(|error| InspectionError::new("browser_feedback_png", error.to_string()))?;
    if bytes.len() > MAX_PNG_BYTES
        || bytes.len() < 24
        || !bytes.starts_with(&PNG_SIGNATURE)
        || u32::from_be_bytes(bytes[8..12].try_into().expect("fixed PNG header")) != 13
        || &bytes[12..16] != b"IHDR"
    {
        return Err(InspectionError::new(
            "browser_feedback_png",
            "capture is not a bounded PNG image",
        ));
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("fixed PNG header"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("fixed PNG header"));
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(InspectionError::new(
            "browser_feedback_dimensions",
            "PNG dimensions are outside the supported bounds",
        ));
    }
    if width != page.image_width || height != page.image_height {
        return Err(InspectionError::new(
            "browser_feedback_dimensions",
            "PNG dimensions do not match page evidence",
        ));
    }
    Ok(bytes)
}

fn payload_digest(
    context: &BrowserCaptureContext,
    submission: &BrowserCaptureSubmission,
) -> Result<String, InspectionError> {
    let mut hasher = Sha256::new();
    let context_bytes = serde_json::to_vec(context)
        .map_err(|error| InspectionError::new("browser_feedback_write", error.to_string()))?;
    let submission_bytes = serde_json::to_vec(submission)
        .map_err(|error| InspectionError::new("browser_feedback_write", error.to_string()))?;
    hasher.update((context_bytes.len() as u64).to_be_bytes());
    hasher.update(context_bytes);
    hasher.update(submission_bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn atomic_write_bytes(dir: &Dir, name: &str, bytes: &[u8]) -> Result<(), InspectionError> {
    let temporary = format!(".{name}.{}.tmp", Uuid::new_v4());
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(cap_fs_ext::FollowSymlinks::No);
    let result = (|| {
        let mut file = dir
            .open_with(&temporary, &options)
            .map_err(|error| InspectionError::new("browser_feedback_write", error.to_string()))?;
        file.write_all(bytes)
            .map_err(|error| InspectionError::new("browser_feedback_write", error.to_string()))?;
        file.sync_all()
            .map_err(|error| InspectionError::new("browser_feedback_write", error.to_string()))?;
        dir.rename(&temporary, dir, name)
            .map_err(|error| InspectionError::new("browser_feedback_write", error.to_string()))
    })();
    if result.is_err() {
        let _ = dir.remove_file(&temporary);
    }
    result
}

fn remove_regular_file(dir: &Dir, name: &str, code: &str) -> Result<(), InspectionError> {
    match dir.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            InspectionError::new("unsafe_path", "feedback artifact is not a regular file"),
        ),
        Ok(_) => dir
            .remove_file(name)
            .map_err(|error| InspectionError::new(code, error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(InspectionError::new(code, error.to_string())),
    }
}

fn record_name(id: &str) -> String {
    format!("capture-{id}.json")
}

fn image_name(id: &str) -> String {
    format!("capture-{id}.png")
}

fn expiry(now: u64, retention: u64) -> u64 {
    now.saturating_add(retention)
}

fn unix_now() -> Result<u64, InspectionError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| InspectionError::new("browser_feedback_clock", error.to_string()))
}

fn validate_association_key(value: &str) -> Result<(), InspectionError> {
    if value.len() != 24 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(InspectionError::new(
            "browser_feedback_association",
            "association key must be 24 hexadecimal characters",
        ));
    }
    Ok(())
}

fn validate_uuid(value: &str, field: &str) -> Result<(), InspectionError> {
    if Uuid::parse_str(value).is_err() {
        return Err(InspectionError::new(
            "browser_feedback_id",
            format!("{field} must be a UUID"),
        ));
    }
    Ok(())
}

fn validate_text(value: &str, field: &str, max_bytes: usize) -> Result<(), InspectionError> {
    if value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(InspectionError::new(
            "browser_feedback_text",
            format!("{field} is oversized or contains control characters"),
        ));
    }
    Ok(())
}

/// Browser DOM text normally includes line breaks between block elements. It
/// is serialized as JSON into a bracketed terminal paste, so horizontal and
/// vertical whitespace is data, while all other control characters remain
/// forbidden.
fn validate_display_text(value: &str, field: &str, max_bytes: usize) -> Result<(), InspectionError> {
    if value.len() > max_bytes || value.chars().any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t')) {
        return Err(InspectionError::new(
            "browser_feedback_text",
            format!("{field} is oversized or contains control characters"),
        ));
    }
    Ok(())
}

fn validate_optional_text(
    value: Option<&str>,
    field: &str,
    max_bytes: usize,
) -> Result<(), InspectionError> {
    if let Some(value) = value {
        if value.len() > max_bytes || value.chars().any(char::is_control) {
            return Err(InspectionError::new(
                "browser_feedback_text",
                format!("{field} is oversized or contains control characters"),
            ));
        }
    }
    Ok(())
}
fn validate_operation_id(value: &str) -> Result<(), InspectionError> {
    if value.is_empty() || value.len() > 128 || value.contains(['/', '\\', '\0']) {
        return Err(InspectionError::new(
            "browser_feedback_operation",
            "operation ID is invalid",
        ));
    }
    Ok(())
}

fn validate_delivery(receipt: &BrowserDeliveryReceipt) -> Result<(), InspectionError> {
    validate_operation_id(&receipt.operation_id)?;
    validate_association_key(&receipt.association_key)?;
    if receipt.selected_ids.len() > MAX_PENDING_CAPTURES * MAX_ANNOTATIONS {
        return Err(InspectionError::new(
            "browser_feedback_receipt",
            "delivery receipt contains too many selected IDs",
        ));
    }
    let selected: HashSet<&str> = receipt.selected_ids.iter().map(String::as_str).collect();
    if selected.len() != receipt.selected_ids.len() {
        return Err(InspectionError::new(
            "browser_feedback_receipt",
            "delivery receipt selection contains duplicates",
        ));
    }
    for id in &receipt.selected_ids {
        validate_uuid(id, "annotation ID")?;
    }
    let acknowledged: HashSet<&str> = receipt
        .acknowledged_ids
        .iter()
        .map(String::as_str)
        .collect();
    if acknowledged.len() != receipt.acknowledged_ids.len()
        || receipt
            .acknowledged_ids
            .iter()
            .any(|id| !selected.contains(id.as_str()))
    {
        return Err(InspectionError::new(
            "browser_feedback_receipt",
            "delivery receipt acknowledgements are invalid",
        ));
    }
    if receipt.message.len() > 8 * 1024 || receipt.message.chars().any(char::is_control) {
        return Err(InspectionError::new(
            "browser_feedback_receipt",
            "delivery receipt message is invalid",
        ));
    }
    if receipt.updated_at < receipt.created_at {
        return Err(InspectionError::new(
            "browser_feedback_receipt",
            "delivery receipt timestamps are invalid",
        ));
    }
    if matches!(
        receipt.state,
        CommentPasteState::Accepted | CommentPasteState::OutcomeUnknown
    ) && receipt.target.is_none()
    {
        return Err(InspectionError::new(
            "browser_feedback_receipt",
            "delivery receipt target is missing",
        ));
    }
    if let Some(target) = &receipt.target {
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
                    "browser_feedback_receipt",
                    "delivery receipt target is invalid",
                ));
            }
        }
    }
    Ok(())
}

fn delivery_record_name(operation_id: &str) -> String {
    format!("delivery-{operation_id}.json")
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn element_evidence_preserves_normal_browser_line_breaks() {
        assert!(validate_display_text("Heading\n\nParagraph\tvalue", "element text", 128).is_ok());
        assert!(validate_display_text("escape\u{1b}[2J", "element text", 128).is_err());
    }

    #[test]
    fn delivery_status_prefers_unknown_and_stays_association_scoped() {
        let root =
            std::env::temp_dir().join(format!("cockpit-feedback-lookup-{}", Uuid::new_v4()));
        let store =
            BrowserFeedbackStore::new(root.clone(), BrowserFeedbackOptions::default()).unwrap();
        let key = "0123456789abcdef01234567";
        let other_key = "fedcba9876543210fedcba98";
        let annotation_id = Uuid::new_v4().to_string();
        let unknown_operation = Uuid::new_v4().to_string();
        let rejected_operation = Uuid::new_v4().to_string();
        let foreign_operation = Uuid::new_v4().to_string();
        let target = CommentPasteTarget {
            endpoint_identity: "disposable-endpoint".into(),
            session_id: "disposable-session".into(),
            workspace_id: "w1".into(),
            tab_id: "w1:t1".into(),
            pane_id: "w1:p1".into(),
            terminal_id: "disposable-terminal".into(),
            agent_label: "omp".into(),
            agent_fingerprint: "disposable-agent".into(),
        };
        let receipt = |operation_id: String,
                       association_key: &str,
                       state: CommentPasteState,
                       created_at: u64,
                       target: Option<CommentPasteTarget>| {
            BrowserDeliveryReceipt {
                operation_id,
                association_key: association_key.into(),
                selected_ids: vec![annotation_id.clone()],
                state,
                target,
                acknowledged_ids: vec![],
                message: format!("{state:?}"),
                created_at,
                updated_at: created_at,
            }
        };
        let feedback = store.feedback_dir().unwrap();
        for record in [
            receipt(
                unknown_operation.clone(),
                key,
                CommentPasteState::OutcomeUnknown,
                10,
                Some(target.clone()),
            ),
            receipt(
                rejected_operation.clone(),
                key,
                CommentPasteState::Rejected,
                20,
                None,
            ),
            receipt(
                foreign_operation,
                other_key,
                CommentPasteState::OutcomeUnknown,
                30,
                Some(target),
            ),
        ] {
            atomic_write_json(
                &feedback,
                &delivery_record_name(&record.operation_id),
                &record,
            )
            .unwrap();
        }
        let capture = BrowserFeedbackCapture {
            id: Uuid::new_v4().to_string(),
            context: BrowserCaptureContext {
                association_key: key.into(),
                session_id: "session".into(),
                space_id: "space".into(),
                space_label: "Space".into(),
                playwright_session: "playwright".into(),
                working_directory: "/workspace".into(),
                invocation: "test".into(),
                browser_instance: Uuid::new_v4().to_string(),
                inline_provenance: None,
            },
            page: BrowserPageEvidence {
                url: "https://example.test".into(),
                title: "Test".into(),
                tab_id: None,
                document_id: "document".into(),
                captured_at: "now".into(),
                viewport: BrowserViewport {
                    width: 1.0,
                    height: 1.0,
                    scroll_x: 0.0,
                    scroll_y: 0.0,
                    device_pixel_ratio: 1.0,
                    visual_scale: 1.0,
                },
                image_width: 1,
                image_height: 1,
            },
            annotations: vec![],
            pending_ids: vec![annotation_id.clone()],
            image_path: String::new(),
        };

        let first = store
            .list_delivery_statuses(key, std::slice::from_ref(&capture))
            .unwrap();
        let second = store
            .list_delivery_statuses(key, std::slice::from_ref(&capture))
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].capture_id, capture.id);
        assert_eq!(first[0].operation_id, unknown_operation);
        assert_eq!(first[0].selected_ids, vec![annotation_id]);
        assert_eq!(first[0].state, CommentPasteState::OutcomeUnknown);
        assert_eq!(first[0].message, "OutcomeUnknown");
    }

    #[test]
    fn unknown_delivery_survives_acknowledgement_and_retention() {
        let root =
            std::env::temp_dir().join(format!("cockpit-feedback-retention-{}", Uuid::new_v4()));
        let store =
            BrowserFeedbackStore::new(root.clone(), BrowserFeedbackOptions::default()).unwrap();
        let key = "0123456789abcdef01234567";
        let id = Uuid::new_v4().to_string();
        let receipt = store
            .save_delivery(BrowserDeliveryReceipt {
                operation_id: Uuid::new_v4().to_string(),
                association_key: key.into(),
                selected_ids: vec![id.clone()],
                state: CommentPasteState::OutcomeUnknown,
                target: Some(CommentPasteTarget {
                    endpoint_identity: "disposable-endpoint".into(),
                    session_id: "disposable-session".into(),
                    workspace_id: "w1".into(),
                    tab_id: "w1:t1".into(),
                    pane_id: "w1:p1".into(),
                    terminal_id: "disposable-terminal".into(),
                    agent_label: "omp".into(),
                    agent_fingerprint: "disposable-agent".into(),
                }),
                acknowledged_ids: vec![],
                message: "Dispatch outcome unknown".into(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store.ack(key, &[id]).unwrap();
        store
            .prune_delivery_locked(
                &store.feedback_dir().unwrap(),
                receipt.updated_at + store.options.retention_seconds + 1,
            )
            .unwrap();
        let retained = store.load_delivery(&receipt.operation_id).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(
            retained.map(|receipt| receipt.state),
            Some(CommentPasteState::OutcomeUnknown)
        );
    }

    #[test]
    fn pending_capture_capacity_preserves_existing_evidence() {
        use base64::Engine as _;

        let root =
            std::env::temp_dir().join(format!("cockpit-feedback-capacity-{}", Uuid::new_v4()));
        let store =
            BrowserFeedbackStore::new(root.clone(), BrowserFeedbackOptions::default()).unwrap();
        let association_key = "0123456789abcdef01234567";
        let browser_instance = Uuid::new_v4().to_string();
        let png_base64 = BASE64.encode([
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0, 1, 0, 0,
            0, 1,
        ]);

        for _ in 0..MAX_PENDING_CAPTURES {
            let capture_id = Uuid::new_v4().to_string();
            let annotation_id = Uuid::new_v4().to_string();
            store
                .save(
                    BrowserCaptureContext {
                        association_key: association_key.into(),
                        session_id: "session".into(),
                        space_id: "space".into(),
                        space_label: "Space".into(),
                        playwright_session: "cockpit-test".into(),
                        working_directory: "/tmp".into(),
                        invocation: "playwright-cli".into(),
                        browser_instance: browser_instance.clone(),
                        inline_provenance: None,
                    },
                    BrowserCaptureSubmission {
                        association_key: association_key.into(),
                        browser_instance: browser_instance.clone(),
                        capture_id,
                        page: BrowserPageEvidence {
                            url: "http://localhost/".into(),
                            title: "Test".into(),
                            tab_id: Some(1),
                            document_id: "document".into(),
                            captured_at: "2026-09-09T00:00:00Z".into(),
                            viewport: BrowserViewport {
                                width: 1.0,
                                height: 1.0,
                                scroll_x: 0.0,
                                scroll_y: 0.0,
                                device_pixel_ratio: 1.0,
                                visual_scale: 1.0,
                            },
                            image_width: 1,
                            image_height: 1,
                        },
                        annotations: vec![BrowserAnnotation {
                            id: annotation_id,
                            kind:
                                cockpit_protocol::browser_feedback::BrowserAnnotationKind::Freehand,
                            comment: "comment".into(),
                            color: "red".into(),
                            points: Vec::new(),
                            bounds: None,
                            element: None,
                        }],
                        png_base64: png_base64.clone(),
                    },
                )
                .unwrap();
        }

        let rejected_capture_id = Uuid::new_v4().to_string();
        let rejected = store
            .save(
                BrowserCaptureContext {
                    association_key: association_key.into(),
                    session_id: "session".into(),
                    space_id: "space".into(),
                    space_label: "Space".into(),
                    playwright_session: "cockpit-test".into(),
                    working_directory: "/tmp".into(),
                    invocation: "playwright-cli".into(),
                    browser_instance: browser_instance.clone(),
                    inline_provenance: None,
                },
                BrowserCaptureSubmission {
                    association_key: association_key.into(),
                    browser_instance,
                    capture_id: rejected_capture_id.clone(),
                    page: BrowserPageEvidence {
                        url: "http://localhost/".into(),
                        title: "Test".into(),
                        tab_id: Some(1),
                        document_id: "document".into(),
                        captured_at: "2026-09-09T00:00:00Z".into(),
                        viewport: BrowserViewport {
                            width: 1.0,
                            height: 1.0,
                            scroll_x: 0.0,
                            scroll_y: 0.0,
                            device_pixel_ratio: 1.0,
                            visual_scale: 1.0,
                        },
                        image_width: 1,
                        image_height: 1,
                    },
                    annotations: vec![BrowserAnnotation {
                        id: Uuid::new_v4().to_string(),
                        kind: cockpit_protocol::browser_feedback::BrowserAnnotationKind::Freehand,
                        comment: "comment".into(),
                        color: "red".into(),
                        points: Vec::new(),
                        bounds: None,
                        element: None,
                    }],
                    png_base64,
                },
            )
            .expect_err("the 65th pending capture must be rejected");
        assert_eq!(rejected.code, "browser_feedback_pending_limit");

        let response = store.list(association_key).unwrap();
        assert_eq!(response.captures.len(), MAX_PENDING_CAPTURES);
        assert_eq!(response.pending_count, MAX_PENDING_CAPTURES);
        assert_eq!(
            store
                .read_image(association_key, &rejected_capture_id)
                .expect_err("rejected capture must not leave an artifact")
                .code,
            "browser_feedback_not_found"
        );
        let retained_image = store
            .read_image(association_key, &response.captures[0].id)
            .unwrap();
        assert_eq!(
            retained_image.as_slice(),
            &[
                137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0, 1,
                0, 0, 0, 1,
            ]
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
