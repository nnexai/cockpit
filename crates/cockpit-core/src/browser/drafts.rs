//! Owner-persisted inline browser drafts and composed capture recovery.
//!
//! The browser helper owns live pixels. This store owns only structured marks,
//! editor state, and the one frozen PNG that must survive a failed save.

use std::{collections::HashSet, path::{Path, PathBuf}, sync::{Arc, Mutex}};

use cockpit_protocol::{
    browser::{BrowserConnectionState, BrowserFeedbackAckRequest, BrowserFeedbackLookup, BrowserResponse, BrowserTarget},
    browser_feedback::{
        BrowserCaptureContext, BrowserCaptureSaved, BrowserCaptureSubmission, BrowserFeedbackAck,
        BrowserInlineCaptureProvenance,
    },
    browser_view::{
        BrowserDraftRecoveryAction, BrowserDraftRecoveryRequest, BrowserViewCaptureCommand,
        BrowserViewCommandOutcome, BrowserViewDocumentCommandContext, BrowserViewFrameDescriptor,
        BrowserViewCaptureOutcome, BrowserViewDraftAnnotation, BrowserViewDraftCommand,
        BrowserViewDraftEditorState, BrowserViewDraftInventory, BrowserViewDraftState,
        BrowserViewInspectionFreshness, BrowserViewPendingCapture,
    },
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    InspectionError,
    browser_feedback::BrowserFeedbackStore,
    project_store::{atomic_write_json, open_dir_nofollow_absolute, prepare_root, read_json_bounded},
};

use super::{BrowserRuntimeAttachment, BrowserService};

const FORMAT_VERSION: u8 = 1;
const MAX_DRAFTS: usize = 8;
const MAX_ENTRIES: usize = 128;
const MAX_DRAFT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PENDING_BYTES: u64 = 6 * 1024 * 1024;
const MAX_PREPARATION_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserDraftIdentity {
    pub association_key: String,
    pub browser_incarnation: String,
    pub target_id: String,
    pub document_generation: u64,
}

#[derive(Clone, Debug)]
pub struct BrowserDraftCaptureContext {
    pub identity: BrowserDraftIdentity,
    pub context: BrowserCaptureContext,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredDraft {
    format_version: u8,
    identity: StoredIdentity,
    draft_id: String,
    revision: u64,
    annotations: Vec<BrowserViewDraftAnnotation>,
    freshness: BrowserViewInspectionFreshness,
    stale: bool,
    editor: BrowserViewDraftEditorState,
    consumed_annotation_ids: Vec<String>,
    tombstoned: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredIdentity {
    association_key: String,
    browser_incarnation: String,
    target_id: String,
    document_generation: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredPendingCapture {
    format_version: u8,
    association_key: String,
    browser_incarnation: String,
    draft_id: String,
    draft_revision: u64,
    annotation_ids: Vec<String>,
    context: BrowserCaptureContext,
    submission: BrowserCaptureSubmission,
    last_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCapturePreparation {
    format_version: u8,
    association_key: String,
    browser_incarnation: String,
    capture_id: String,
    draft_id: String,
    draft_revision: u64,
    annotation_ids: Vec<String>,
    context: BrowserCaptureContext,
}

pub struct BrowserDraftStore {
    root: PathBuf,
    feedback: Arc<BrowserFeedbackStore>,
    mutation: Mutex<()>,
}

impl BrowserDraftStore {
    pub fn new(
        state_root: PathBuf,
        feedback: Arc<BrowserFeedbackStore>,
    ) -> Result<Self, InspectionError> {
        let browser_root = state_root.join("browser");
        let (browser_root, _) = prepare_root(&browser_root, "browser")?;
        let (root, _) = prepare_root(&browser_root.join("drafts"), "browser drafts")?;
        Ok(Self {
            root,
            feedback,
            mutation: Mutex::new(()),
        })
    }

    pub fn open(
        &self,
        identity: &BrowserDraftIdentity,
        draft_id: Option<String>,
    ) -> Result<BrowserViewDraftState, InspectionError> {
        let _guard = self.lock()?;
        validate_identity(identity)?;
        let mut drafts = self.load_all()?;
        if let Some(draft_id) = draft_id {
            validate_uuid(&draft_id, "draft ID")?;
            let draft = drafts
                .iter()
                .find(|draft| draft.draft_id == draft_id)
                .ok_or_else(|| InspectionError::new("browser_draft_not_found", "Draft is unavailable"))?;
            if draft.tombstoned {
                return Err(InspectionError::new("browser_draft_tombstoned", "Draft was discarded"));
            }
            return Ok(public_draft(draft));
        }
        if let Some(draft) = drafts.iter().find(|draft| {
            !draft.tombstoned && !draft.stale && draft.identity == stored_identity(identity)
        }) {
            return Ok(public_draft(draft));
        }
        self.retire_obsolete(&mut drafts, identity)?;
        let active = drafts.iter().filter(|draft| !draft.tombstoned).count();
        if active >= MAX_DRAFTS {
            return Err(InspectionError::new(
                "browser_draft_capacity",
                "The association has eight unfinished drafts; discard or recover one before creating another",
            ));
        }
        let stored = StoredDraft {
            format_version: FORMAT_VERSION,
            identity: stored_identity(identity),
            draft_id: Uuid::new_v4().to_string(),
            revision: 1,
            annotations: Vec::new(),
            freshness: BrowserViewInspectionFreshness::Fresh,
            stale: false,
            editor: BrowserViewDraftEditorState { selected_annotation_id: None, notes_open: false },
            consumed_annotation_ids: Vec::new(),
            tombstoned: false,
        };
        self.write_draft(&stored)?;
        Ok(public_draft(&stored))
    }
    fn retire_obsolete(
        &self,
        drafts: &mut [StoredDraft],
        identity: &BrowserDraftIdentity,
    ) -> Result<(), InspectionError> {
        let pending = self.load_pending(&identity.association_key)?;
        let preparation = self.load_preparation(&identity.association_key)?;
        let referenced = self.load_capture_references()?;
        let stored = stored_identity(identity);
        for draft in drafts.iter_mut().filter(|draft| {
            !draft.tombstoned
                && draft.identity.association_key == stored.association_key
                && draft.identity.browser_incarnation == stored.browser_incarnation
                && draft.identity.target_id == stored.target_id
                && draft.identity.document_generation != stored.document_generation
        }) {
            draft.annotations.clear();
            draft.editor.selected_annotation_id = None;
            draft.tombstoned = true;
            draft.stale = true;
            draft.freshness = BrowserViewInspectionFreshness::Stale;
            draft.revision = draft.revision.saturating_add(1);
            self.write_draft(draft)?;
        }
        if pending.is_none() {
            if let Some(preparation) = preparation {
                let belongs_to_current = drafts.iter().any(|draft| {
                    draft.draft_id == preparation.draft_id
                        && !draft.tombstoned
                        && draft.identity == stored
                });
                if !belongs_to_current {
                    self.remove_preparation(&identity.association_key)?;
                }
            }
        }
        self.compact_retired(drafts, &referenced, &stored.association_key)
    }

    fn load_capture_references(&self) -> Result<HashSet<String>, InspectionError> {
        let dir = self.dir()?;
        let mut records = Vec::new();
        for (index, entry) in dir.entries()
            .map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))?
            .enumerate()
        {
            if index >= MAX_ENTRIES {
                return Err(InspectionError::new("browser_draft_bounded", "Draft directory exceeded its entry limit"));
            }
            let entry = entry.map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if let Some(association_key) = name.strip_prefix("pending-").and_then(|name| name.strip_suffix(".json")) {
                records.push((true, association_key.to_owned()));
            } else if let Some(association_key) = name.strip_prefix("preparation-").and_then(|name| name.strip_suffix(".json")) {
                records.push((false, association_key.to_owned()));
            }
        }
        let mut referenced = HashSet::new();
        for (pending, association_key) in records {
            validate_association_key(&association_key)?;
            if pending {
                if let Some(capture) = self.load_pending(&association_key)? {
                    referenced.insert(capture.draft_id);
                }
            } else if let Some(capture) = self.load_preparation(&association_key)? {
                referenced.insert(capture.draft_id);
            }
        }
        Ok(referenced)
    }

    fn compact_retired(
        &self,
        drafts: &[StoredDraft],
        referenced: &HashSet<String>,
        association_key: &str,
    ) -> Result<(), InspectionError> {
        for draft in drafts.iter().filter(|draft| draft.tombstoned && draft.identity.association_key == association_key && !referenced.contains(&draft.draft_id)) {
            self.remove_draft(&draft.draft_id)?;
        }
        Ok(())
    }


    pub fn list(&self, association_key: &str) -> Result<BrowserViewDraftInventory, InspectionError> {
        let _guard = self.lock()?;
        validate_association_key(association_key)?;
        let mut drafts: Vec<_> = self
            .load_all()?
            .into_iter()
            .filter(|draft| draft.identity.association_key == association_key && !draft.tombstoned)
            .map(|draft| public_draft(&draft))
            .collect();
        drafts.sort_by(|left, right| left.draft_id.cmp(&right.draft_id));
        Ok(BrowserViewDraftInventory {
            drafts,
            active_draft_limit: MAX_DRAFTS,
            pending_capture: self.load_pending(association_key)?.map(public_pending),
        })
    }

    pub fn mutate(
        &self,
        identity: &BrowserDraftIdentity,
        draft_id: &str,
        expected_revision: u64,
        command: BrowserViewDraftCommand,
    ) -> Result<BrowserViewDraftState, InspectionError> {
        let _guard = self.lock()?;
        validate_identity(identity)?;
        validate_uuid(draft_id, "draft ID")?;
        let mut draft = self.load_draft(draft_id)?.ok_or_else(|| {
            InspectionError::new("browser_draft_not_found", "Draft is unavailable")
        })?;
        if draft.identity != stored_identity(identity) {
            return Err(InspectionError::new("browser_draft_identity", "Draft belongs to another document"));
        }
        if draft.tombstoned {
            return Err(InspectionError::new("browser_draft_tombstoned", "Draft was discarded"));
        }
        if draft.revision != expected_revision {
            return Err(InspectionError::new("browser_draft_revision", "Draft changed; recover the latest revision before editing"));
        }
        match command {
            BrowserViewDraftCommand::UpsertAnnotation { annotation } => {
                validate_annotation(&annotation)?;
                if draft.consumed_annotation_ids.iter().any(|id| id == &annotation.id) {
                    return Err(InspectionError::new("browser_draft_consumed", "A saved or removed annotation cannot be restored by a delayed save"));
                }
                if let Some(current) = draft.annotations.iter_mut().find(|current| current.id == annotation.id) {
                    *current = annotation;
                } else {
                    if draft.annotations.len() >= 64 {
                        return Err(InspectionError::new("browser_draft_annotation_limit", "Draft has the maximum of 64 annotations"));
                    }
                    draft.annotations.push(annotation);
                }
            }
            BrowserViewDraftCommand::RemoveAnnotation { annotation_id } => {
                validate_uuid(&annotation_id, "annotation ID")?;
                let before = draft.annotations.len();
                draft.annotations.retain(|annotation| annotation.id != annotation_id);
                if draft.annotations.len() != before {
                    insert_consumed(&mut draft.consumed_annotation_ids, annotation_id)?;
                }
            }
            BrowserViewDraftCommand::Clear => {
                let ids: Vec<_> = draft.annotations.iter().map(|annotation| annotation.id.clone()).collect();
                draft.annotations.clear();
                for id in ids {
                    insert_consumed(&mut draft.consumed_annotation_ids, id)?;
                }
            }
            BrowserViewDraftCommand::Discard => {
                let ids: Vec<_> = draft.annotations.iter().map(|annotation| annotation.id.clone()).collect();
                for id in ids {
                    insert_consumed(&mut draft.consumed_annotation_ids, id)?;
                }
                draft.annotations.clear();
                draft.tombstoned = true;
            }
            BrowserViewDraftCommand::SetEditor { editor } => {
                if let Some(selected) = &editor.selected_annotation_id
                    && !draft.annotations.iter().any(|annotation| &annotation.id == selected)
                {
                    return Err(InspectionError::new("browser_draft_editor", "The selected annotation is unavailable"));
                }
                draft.editor = editor;
            }
            BrowserViewDraftCommand::Open { .. }
            | BrowserViewDraftCommand::List
            | BrowserViewDraftCommand::SaveCapture { .. }
            | BrowserViewDraftCommand::RetryPending
            | BrowserViewDraftCommand::DiscardPending => {
                return Err(InspectionError::new("browser_draft_command", "That draft command has a dedicated store operation"));
            }
        }
        draft.revision = draft.revision.saturating_add(1);
        self.write_draft(&draft)?;
        Ok(public_draft(&draft))
    }

    pub fn mark_stale(
        &self,
        association_key: &str,
        browser_incarnation: &str,
        target_id: &str,
        current_document_generation: u64,
    ) -> Result<(), InspectionError> {
        let _guard = self.lock()?;
        validate_association_key(association_key)?;
        validate_uuid(browser_incarnation, "browser incarnation")?;
        validate_id(target_id, "target")?;
        for mut draft in self.load_all()? {
            if !draft.tombstoned
                && draft.identity.association_key == association_key
                && draft.identity.browser_incarnation == browser_incarnation
                && draft.identity.target_id == target_id
                && draft.identity.document_generation != current_document_generation
                && !draft.stale
            {
                draft.stale = true;
                draft.freshness = BrowserViewInspectionFreshness::Stale;
                draft.revision = draft.revision.saturating_add(1);
                self.write_draft(&draft)?;
            }
        }
        Ok(())
    }

    pub fn save_capture(
        &self,
        draft_id: &str,
        expected_revision: u64,
        annotation_ids: Vec<String>,
        submission: BrowserCaptureSubmission,
        provenance: BrowserInlineCaptureProvenance,
    ) -> Result<BrowserViewCaptureOutcome, InspectionError> {
        let _guard = self.lock()?;
        let prepared = self.load_preparation(&submission.association_key)?.ok_or_else(|| {
            InspectionError::new("browser_draft_capture", "Capture was not prepared by the browser owner")
        })?;
        if prepared.draft_id != draft_id
            || prepared.draft_revision != expected_revision
            || prepared.annotation_ids != annotation_ids
            || prepared.association_key != submission.association_key
            || prepared.browser_incarnation != submission.browser_instance
            || prepared.capture_id != submission.capture_id
            || prepared.context.inline_provenance.as_ref() != Some(&provenance)
        {
            return Err(InspectionError::new("browser_draft_capture", "Capture does not match its owner-issued preparation"));
        }
        self.persist_pending(&prepared, submission)?;
        self.remove_preparation(&prepared.association_key)?;
        self.save_pending_locked(&prepared.association_key)
    }

    pub fn prepare_capture(
        &self,
        capture: BrowserDraftCaptureContext,
        draft_id: &str,
        expected_revision: u64,
        annotation_ids: Vec<String>,
        capture_id: String,
    ) -> Result<(), InspectionError> {
        let _guard = self.lock()?;
        validate_identity(&capture.identity)?;
        validate_capture_context(&capture.identity, &capture.context)?;
        validate_uuid(draft_id, "draft ID")?;
        validate_uuid(&capture_id, "capture ID")?;
        validate_annotation_ids(&annotation_ids)?;
        let draft = self.load_draft(draft_id)?.ok_or_else(|| {
            InspectionError::new("browser_draft_not_found", "Draft is unavailable")
        })?;
        if draft.identity != stored_identity(&capture.identity) || draft.tombstoned || draft.stale {
            return Err(InspectionError::new("browser_draft_identity", "Capture draft is no longer current"));
        }
        if draft.revision != expected_revision {
            return Err(InspectionError::new("browser_draft_revision", "Draft changed before capture"));
        }
        let selected: HashSet<_> = annotation_ids.iter().collect();
        if selected.len() != annotation_ids.len()
            || draft.annotations.iter().filter(|annotation| selected.contains(&annotation.id)).count() != annotation_ids.len()
        {
            return Err(InspectionError::new("browser_draft_capture", "Capture selection does not match the current draft"));
        }
        if self.load_pending(&capture.identity.association_key)?.is_some() {
            return Err(InspectionError::new("browser_draft_pending", "Save or discard the pending capture before composing another"));
        }
        if let Some(existing) = self.load_preparation(&capture.identity.association_key)? {
            if existing.draft_id == draft_id
                && existing.draft_revision == expected_revision
                && existing.annotation_ids == annotation_ids
                && existing.context == capture.context
                && existing.capture_id == capture_id
            {
                return Ok(());
            }
            return Err(InspectionError::new("browser_draft_capture", "A capture is already prepared for this association"));
        }
        let prepared = StoredCapturePreparation {
            format_version: FORMAT_VERSION,
            association_key: capture.identity.association_key,
            browser_incarnation: capture.identity.browser_incarnation,
            capture_id,
            draft_id: draft_id.to_owned(),
            draft_revision: expected_revision,
            annotation_ids,
            context: capture.context,
        };
        atomic_write_json(&self.dir()?, &preparation_name(&prepared.association_key), &prepared)
            .map_err(|error| InspectionError::new("browser_draft_capture", error.to_string()))
    }

    pub fn retry_pending(
        &self,
        association_key: &str,
    ) -> Result<BrowserViewCaptureOutcome, InspectionError> {
        let _guard = self.lock()?;
        validate_association_key(association_key)?;
        self.save_pending_locked(association_key)
    }

    pub fn discard_pending(&self, association_key: &str) -> Result<(), InspectionError> {
        let _guard = self.lock()?;
        validate_association_key(association_key)?;
        let dir = self.dir()?;
        let name = pending_name(association_key);
        match dir.remove_file(&name) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(InspectionError::new("browser_draft_pending", error.to_string())),
        }?;
        if let Err(error) = dir.remove_file(preparation_name(association_key))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(InspectionError::new("browser_draft_pending", error.to_string()));
        }
        Ok(())
    }

    pub fn discard_draft(
        &self,
        association_key: &str,
        draft_id: &str,
        expected_revision: u64,
    ) -> Result<(), InspectionError> {
        let _guard = self.lock()?;
        validate_association_key(association_key)?;
        validate_uuid(draft_id, "draft ID")?;
        let mut draft = self.load_draft(draft_id)?.ok_or_else(|| {
            InspectionError::new("browser_draft_not_found", "Draft is unavailable")
        })?;
        if draft.identity.association_key != association_key {
            return Err(InspectionError::new("browser_draft_identity", "Draft belongs to another browser association"));
        }
        if draft.revision != expected_revision {
            return Err(InspectionError::new("browser_draft_revision", "Draft changed; recover the latest revision before discarding"));
        }
        let ids: Vec<_> = draft.annotations.iter().map(|annotation| annotation.id.clone()).collect();
        draft.annotations.clear();
        for id in ids { insert_consumed(&mut draft.consumed_annotation_ids, id)?; }
        draft.tombstoned = true;
        draft.revision = draft.revision.saturating_add(1);
        self.write_draft(&draft)
    }

    fn persist_pending(
        &self,
        prepared: &StoredCapturePreparation,
        submission: BrowserCaptureSubmission,
    ) -> Result<(), InspectionError> {
        let submitted: Vec<_> = submission.annotations.iter().map(|annotation| annotation.id.clone()).collect();
        if submitted != prepared.annotation_ids {
            return Err(InspectionError::new("browser_draft_capture", "Composed capture annotations do not match the selected draft marks"));
        }
        if submission.association_key != prepared.association_key
            || submission.browser_instance != prepared.browser_incarnation
        {
            return Err(InspectionError::new("browser_draft_capture", "Capture belongs to another browser association"));
        }
        let name = pending_name(&prepared.association_key);
        if let Some(existing) = self.load_pending(&prepared.association_key)? {
            if existing.submission.capture_id == submission.capture_id
                && existing.submission == submission
                && existing.annotation_ids == prepared.annotation_ids
            {
                return Ok(());
            }
            return Err(InspectionError::new("browser_draft_pending", "Save or discard the pending capture before composing another"));
        }
        let stored = StoredPendingCapture {
            format_version: FORMAT_VERSION,
            association_key: prepared.association_key.clone(),
            browser_incarnation: prepared.browser_incarnation.clone(),
            draft_id: prepared.draft_id.clone(),
            draft_revision: prepared.draft_revision,
            annotation_ids: prepared.annotation_ids.clone(),
            context: prepared.context.clone(),
            submission,
            last_error: None,
        };
        atomic_write_json(&self.dir()?, &name, &stored)
            .map_err(|error| InspectionError::new("browser_draft_pending", error.to_string()))
    }

    fn save_pending_locked(
        &self,
        association_key: &str,
    ) -> Result<BrowserViewCaptureOutcome, InspectionError> {
        let Some(mut pending) = self.load_pending(association_key)? else {
            return Ok(BrowserViewCaptureOutcome::Absent);
        };
        let saved = match self.feedback.save(pending.context.clone(), pending.submission.clone()) {
            Ok(saved) => saved,
            Err(error) => {
                pending.last_error = Some(error.message.chars().take(1_024).collect());
                self.write_pending(&pending)?;
                return Ok(BrowserViewCaptureOutcome::Pending { pending: public_pending(pending) });
            }
        };
        self.apply_capture_receipt(&pending, &saved)?;
        self.remove_pending(association_key)?;
        Ok(BrowserViewCaptureOutcome::Saved { saved })
    }

    fn apply_capture_receipt(
        &self,
        pending: &StoredPendingCapture,
        saved: &BrowserCaptureSaved,
    ) -> Result<(), InspectionError> {
        if saved.capture_id != pending.submission.capture_id || saved.annotation_ids != pending.annotation_ids {
            return Err(InspectionError::new("browser_draft_receipt", "Feedback store returned a mismatched capture receipt"));
        }
        let Some(mut draft) = self.load_draft(&pending.draft_id)? else {
            return Ok(());
        };
        if draft.identity.association_key != pending.association_key || draft.tombstoned {
            return Ok(());
        }
        let selected: HashSet<_> = pending.annotation_ids.iter().collect();
        draft.annotations.retain(|annotation| !selected.contains(&annotation.id));
        for id in &pending.annotation_ids {
            insert_consumed(&mut draft.consumed_annotation_ids, id.clone())?;
        }
        if draft.editor.selected_annotation_id.as_ref().is_some_and(|id| selected.contains(id)) {
            draft.editor.selected_annotation_id = None;
        }
        draft.revision = draft.revision.saturating_add(1);
        self.write_draft(&draft)
    }

    fn dir(&self) -> Result<cap_std::fs::Dir, InspectionError> {
        open_dir_nofollow_absolute(&self.root)
            .map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, InspectionError> {
        self.mutation.lock().map_err(|_| InspectionError::new("browser_draft_lock", "Draft store lock was poisoned"))
    }

    fn load_all(&self) -> Result<Vec<StoredDraft>, InspectionError> {
        let dir = self.dir()?;
        let mut drafts = Vec::new();
        for (index, entry) in dir.entries().map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))?.enumerate() {
            if index >= MAX_ENTRIES {
                return Err(InspectionError::new("browser_draft_bounded", "Draft directory exceeded its entry limit"));
            }
            let entry = entry.map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(id) = name.strip_prefix("draft-").and_then(|name| name.strip_suffix(".json")) else { continue };
            validate_uuid(id, "draft ID")?;
            let type_ = entry.file_type().map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))?;
            if type_.is_symlink() || !type_.is_file() {
                return Err(InspectionError::new("unsafe_path", "Draft record is not a regular file"));
            }
            let draft: StoredDraft = read_json_bounded(&dir, name, MAX_DRAFT_BYTES)?;
            validate_stored_draft(id, &draft)?;
            drafts.push(draft);
        }
        Ok(drafts)
    }

    fn load_draft(&self, draft_id: &str) -> Result<Option<StoredDraft>, InspectionError> {
        let dir = self.dir()?;
        let name = draft_name(draft_id);
        match dir.symlink_metadata(&name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(InspectionError::new("browser_draft_read", error.to_string())),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(InspectionError::new("unsafe_path", "Draft record is not a regular file")),
            Ok(_) => {
                let draft: StoredDraft = read_json_bounded(&dir, &name, MAX_DRAFT_BYTES)?;
                validate_stored_draft(draft_id, &draft)?;
                Ok(Some(draft))
            }
        }
    }

    fn write_draft(&self, draft: &StoredDraft) -> Result<(), InspectionError> {
        validate_stored_draft(&draft.draft_id, draft)?;
        atomic_write_json(&self.dir()?, &draft_name(&draft.draft_id), draft)
            .map_err(|error| InspectionError::new("browser_draft_write", error.to_string()))
    }
    fn remove_draft(&self, draft_id: &str) -> Result<(), InspectionError> {
        match self.dir()?.remove_file(draft_name(draft_id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(InspectionError::new("browser_draft_write", error.to_string())),
        }
    }

    fn load_pending(&self, association_key: &str) -> Result<Option<StoredPendingCapture>, InspectionError> {
        let dir = self.dir()?;
        let name = pending_name(association_key);
        match dir.symlink_metadata(&name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(InspectionError::new("browser_draft_pending", error.to_string())),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(InspectionError::new("unsafe_path", "Pending capture is not a regular file")),
            Ok(_) => {
                let pending: StoredPendingCapture = read_json_bounded(&dir, &name, MAX_PENDING_BYTES)?;
                validate_pending(&pending)?;
                Ok(Some(pending))
            }
        }
    }

    fn remove_pending(&self, association_key: &str) -> Result<(), InspectionError> {
        self.dir()?.remove_file(pending_name(association_key))
            .map_err(|error| InspectionError::new("browser_draft_pending", error.to_string()))
    }

    fn load_preparation(&self, association_key: &str) -> Result<Option<StoredCapturePreparation>, InspectionError> {
        let dir = self.dir()?;
        let name = preparation_name(association_key);
        match dir.symlink_metadata(&name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(InspectionError::new("browser_draft_capture", error.to_string())),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(InspectionError::new("unsafe_path", "Capture preparation is not a regular file")),
            Ok(_) => {
                let prepared: StoredCapturePreparation = read_json_bounded(&dir, &name, MAX_PREPARATION_BYTES)?;
                validate_preparation(&prepared)?;
                Ok(Some(prepared))
            }
        }
    }

    fn remove_preparation(&self, association_key: &str) -> Result<(), InspectionError> {
        self.dir()?.remove_file(preparation_name(association_key))
            .map_err(|error| InspectionError::new("browser_draft_capture", error.to_string()))
    }

    fn write_pending(&self, pending: &StoredPendingCapture) -> Result<(), InspectionError> {
        atomic_write_json(&self.dir()?, &pending_name(&pending.association_key), pending)
            .map_err(|error| InspectionError::new("browser_draft_pending", error.to_string()))
    }

}

fn stored_identity(identity: &BrowserDraftIdentity) -> StoredIdentity {
    StoredIdentity {
        association_key: identity.association_key.clone(),
        browser_incarnation: identity.browser_incarnation.clone(),
        target_id: identity.target_id.clone(),
        document_generation: identity.document_generation,
    }
}

fn public_draft(draft: &StoredDraft) -> BrowserViewDraftState {
    BrowserViewDraftState {
        draft_id: draft.draft_id.clone(),
        target_id: draft.identity.target_id.clone(),
        document_generation: draft.identity.document_generation,
        revision: draft.revision,
        annotations: draft.annotations.clone(),
        freshness: draft.freshness,
        stale: draft.stale,
        editor: draft.editor.clone(),
    }
}

fn public_pending(pending: StoredPendingCapture) -> BrowserViewPendingCapture {
    BrowserViewPendingCapture {
        capture_id: pending.submission.capture_id,
        draft_id: pending.draft_id,
        draft_revision: pending.draft_revision,
        annotation_ids: pending.annotation_ids,
        last_error: pending.last_error,
    }
}

fn validate_capture_context(identity: &BrowserDraftIdentity, context: &BrowserCaptureContext) -> Result<(), InspectionError> {
    if context.association_key != identity.association_key || context.browser_instance != identity.browser_incarnation {
        return Err(InspectionError::new("browser_draft_context", "Capture context belongs to another browser association"));
    }
    let provenance: &BrowserInlineCaptureProvenance = context.inline_provenance.as_ref().ok_or_else(|| {
        InspectionError::new("browser_draft_provenance", "Inline captures require target and frame provenance")
    })?;
    if provenance.target_id != identity.target_id || provenance.document_generation != identity.document_generation {
        return Err(InspectionError::new("browser_draft_provenance", "Capture provenance does not match the draft document"));
    }
    validate_id(&provenance.frame_id, "frame")?;
    Ok(())
}

fn validate_stored_draft(id: &str, draft: &StoredDraft) -> Result<(), InspectionError> {
    if draft.format_version != FORMAT_VERSION || draft.draft_id != id || draft.annotations.len() > 64 {
        return Err(InspectionError::new("browser_draft_corrupt", "Draft record is invalid"));
    }
    validate_identity(&BrowserDraftIdentity {
        association_key: draft.identity.association_key.clone(),
        browser_incarnation: draft.identity.browser_incarnation.clone(),
        target_id: draft.identity.target_id.clone(),
        document_generation: draft.identity.document_generation,
    })?;
    let mut ids = HashSet::new();
    for annotation in &draft.annotations {
        validate_annotation(annotation)?;
        if !ids.insert(&annotation.id) || draft.consumed_annotation_ids.iter().any(|consumed| consumed == &annotation.id) {
            return Err(InspectionError::new("browser_draft_corrupt", "Draft annotation IDs are invalid"));
        }
    }
    validate_annotation_ids(&draft.consumed_annotation_ids)
}

fn validate_pending(pending: &StoredPendingCapture) -> Result<(), InspectionError> {
    if pending.format_version != FORMAT_VERSION
        || pending.submission.association_key != pending.association_key
        || pending.submission.browser_instance != pending.browser_incarnation
    {
        return Err(InspectionError::new("browser_draft_pending", "Pending capture is invalid"));
    }
    validate_association_key(&pending.association_key)?;
    validate_id(&pending.browser_incarnation, "browser incarnation")?;
    validate_uuid(&pending.draft_id, "draft ID")?;
    if pending.context.association_key != pending.association_key
        || pending.context.browser_instance != pending.browser_incarnation
    {
        return Err(InspectionError::new("browser_draft_pending", "Pending capture context is invalid"));
    }
    validate_annotation_ids(&pending.annotation_ids)
}

fn validate_preparation(prepared: &StoredCapturePreparation) -> Result<(), InspectionError> {
    if prepared.format_version != FORMAT_VERSION
        || prepared.context.association_key != prepared.association_key
        || prepared.context.browser_instance != prepared.browser_incarnation
    {
        return Err(InspectionError::new("browser_draft_capture", "Capture preparation is invalid"));
    }
    validate_association_key(&prepared.association_key)?;
    validate_id(&prepared.browser_incarnation, "browser incarnation")?;
    validate_uuid(&prepared.capture_id, "capture ID")?;
    validate_uuid(&prepared.draft_id, "draft ID")?;
    validate_annotation_ids(&prepared.annotation_ids)
}

fn validate_identity(identity: &BrowserDraftIdentity) -> Result<(), InspectionError> {
    validate_association_key(&identity.association_key)?;
    validate_id(&identity.browser_incarnation, "browser incarnation")?;
    validate_id(&identity.target_id, "target")
}

fn validate_annotation(annotation: &BrowserViewDraftAnnotation) -> Result<(), InspectionError> {
    validate_uuid(&annotation.id, "annotation ID")?;
    if annotation.points.len() > 8_192 || annotation.color.len() > 32 || annotation.comment.as_ref().is_some_and(|comment| comment.len() > 64 * 1024) {
        return Err(InspectionError::new("browser_draft_annotation", "Draft annotation exceeds bounds"));
    }
    Ok(())
}

fn validate_annotation_ids(ids: &[String]) -> Result<(), InspectionError> {
    if ids.len() > 64 || ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(InspectionError::new("browser_draft_annotation", "Annotation IDs are invalid"));
    }
    for id in ids { validate_uuid(id, "annotation ID")?; }
    Ok(())
}

fn insert_consumed(ids: &mut Vec<String>, id: String) -> Result<(), InspectionError> {
    validate_uuid(&id, "annotation ID")?;
    if !ids.contains(&id) { ids.push(id); }
    if ids.len() > 512 {
        return Err(InspectionError::new("browser_draft_consumed", "Draft consumed-ID history exceeds its bound"));
    }
    Ok(())
}

fn validate_association_key(value: &str) -> Result<(), InspectionError> {
    if value.len() != 24 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(InspectionError::new("browser_draft_association", "Association key must be 24 hexadecimal characters"));
    }
    Ok(())
}

fn validate_uuid(value: &str, field: &str) -> Result<(), InspectionError> {
    if Uuid::parse_str(value).is_err() {
        return Err(InspectionError::new("browser_draft_id", format!("{field} must be a UUID")));
    }
    Ok(())
}

fn validate_id(value: &str, field: &str) -> Result<(), InspectionError> {
    if value.is_empty() || value.len() > 512 || value.contains(['/', '\\', '\0']) {
        return Err(InspectionError::new("browser_draft_id", format!("{field} ID is invalid")));
    }
    Ok(())
}

fn draft_name(id: &str) -> String { format!("draft-{id}.json") }
fn pending_name(association_key: &str) -> String { format!("pending-{association_key}.json") }
fn preparation_name(association_key: &str) -> String { format!("preparation-{association_key}.json") }

impl BrowserService {
    /// Handles only owner-persisted annotation commands. The host forwards all
    /// browser input and helper commands through their separate transport.
    pub async fn browser_annotation_command(
        &self,
        target: &BrowserTarget,
        attachment: &BrowserRuntimeAttachment,
        context: BrowserViewDocumentCommandContext,
        draft_id: Option<&str>,
        expected_revision: Option<u64>,
        command: BrowserViewDraftCommand,
    ) -> Result<BrowserViewCommandOutcome, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let receipt = self.annotation_receipt(target, attachment).await?;
        let store = self.draft_store()?;
        let association_key = receipt.association_key.clone();
        match command {
            BrowserViewDraftCommand::List => Ok(BrowserViewCommandOutcome::DraftInventory {
                inventory: store.list(&association_key)?,
            }),
            BrowserViewDraftCommand::RetryPending => Ok(BrowserViewCommandOutcome::Capture {
                capture: store.retry_pending(&association_key)?,
            }),
            BrowserViewDraftCommand::DiscardPending => {
                store.discard_pending(&association_key)?;
                Ok(BrowserViewCommandOutcome::Capture { capture: BrowserViewCaptureOutcome::Absent })
            }
            BrowserViewDraftCommand::SaveCapture {
                submission,
                annotation_ids,
                provenance,
            } => {
                let draft_id = required_draft_id(draft_id)?;
                let expected_revision = required_revision(expected_revision)?;
                Ok(BrowserViewCommandOutcome::Capture {
                    capture: store.save_capture(
                        draft_id,
                        expected_revision,
                        annotation_ids,
                        submission,
                        provenance,
                    )?,
                })
            }
            command => {
                if context.target_id != attachment.target_id {
                    return Err(InspectionError::new("browser_draft_target", "Draft command targets another browser tab"));
                }
                let identity = BrowserDraftIdentity {
                    association_key,
                    browser_incarnation: attachment.browser_incarnation.clone(),
                    target_id: context.target_id,
                    document_generation: context.document_generation,
                };
                match command {
                    BrowserViewDraftCommand::Open { draft_id: requested } => {
                        Ok(BrowserViewCommandOutcome::Draft { draft: store.open(&identity, requested)? })
                    }
                    command => Ok(BrowserViewCommandOutcome::Draft {
                        draft: store.mutate(
                            &identity,
                            required_draft_id(draft_id)?,
                            required_revision(expected_revision)?,
                            command,
                        )?,
                    }),
                }
            }
        }
    }

    /// Records a capture transaction only after the helper has returned the
    /// descriptor for the pixels that will be composed. A later navigation
    /// cannot alter that stored identity.
    pub async fn browser_prepare_capture(
        &self,
        target: &BrowserTarget,
        attachment: &BrowserRuntimeAttachment,
        command: &BrowserViewCaptureCommand,
        capture_id: &str,
        descriptor: &BrowserViewFrameDescriptor,
        frame_id: String,
        frame_generation: u64,
    ) -> Result<(), InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let receipt = self.annotation_receipt(target, attachment).await?;
        if command.location.target_id != attachment.target_id
            || descriptor.target_id != attachment.target_id
            || command.location.document_generation != descriptor.document_generation
            || command.location.viewport_revision != descriptor.viewport_revision
            || command.location.presented_frame_sequence != descriptor.frame_sequence
            || command.location.lease_generation == 0
        {
            return Err(InspectionError::new("browser_draft_capture", "Capture preparation no longer matches the presented frame"));
        }
        let identity = BrowserDraftIdentity {
            association_key: receipt.association_key.clone(),
            browser_incarnation: attachment.browser_incarnation.clone(),
            target_id: attachment.target_id.clone(),
            document_generation: descriptor.document_generation,
        };
        let address = self.association(&receipt, cockpit_protocol::browser::BrowserConnectionState::Open);
        let context = BrowserCaptureContext {
            association_key: address.association_key,
            session_id: address.session_id,
            space_id: address.space_id,
            space_label: address.space_label,
            playwright_session: address.playwright_session,
            working_directory: address.working_directory,
            invocation: address.invocation,
            browser_instance: attachment.browser_incarnation.clone(),
            inline_provenance: Some(BrowserInlineCaptureProvenance {
                target_id: descriptor.target_id.clone(),
                frame_id,
                document_generation: descriptor.document_generation,
                frame_generation,
                stream_epoch: descriptor.stream_epoch,
                frame_sequence: descriptor.frame_sequence,
                viewport_revision: descriptor.viewport_revision,
                pixel_captured_at_micros: descriptor.capture_timestamp_micros,
                capture_as_shown: command.capture_as_shown,
            }),
        };
        self.draft_store()?.prepare_capture(
            BrowserDraftCaptureContext { identity, context },
            &command.draft_id,
            command.draft_revision,
            command.annotation_ids.clone(),
            capture_id.to_owned(),
        )
    }

    /// Recovers persisted drafts and frozen captures without launching or
    /// attaching a browser. Target resolution still uses fresh Herdr authority.
    pub async fn browser_draft_recovery(
        &self,
        request: BrowserDraftRecoveryRequest,
    ) -> Result<BrowserViewCommandOutcome, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let resolved = self.resolve_target(&request.target).await?;
        let association_key = super::association_key(
            &resolved.endpoint_identity,
            &resolved.session_id,
            &resolved.space_id,
        );
        let store = self.draft_store()?;
        match request.action {
            BrowserDraftRecoveryAction::List => Ok(BrowserViewCommandOutcome::DraftInventory {
                inventory: store.list(&association_key)?,
            }),
            BrowserDraftRecoveryAction::RetryPending => Ok(BrowserViewCommandOutcome::Capture {
                capture: store.retry_pending(&association_key)?,
            }),
            BrowserDraftRecoveryAction::DiscardPending => {
                store.discard_pending(&association_key)?;
                Ok(BrowserViewCommandOutcome::Capture { capture: BrowserViewCaptureOutcome::Absent })
            }
            BrowserDraftRecoveryAction::DiscardDraft { draft_id, expected_revision } => {
                store.discard_draft(&association_key, &draft_id, expected_revision)?;
                Ok(BrowserViewCommandOutcome::DraftInventory {
                    inventory: store.list(&association_key)?,
                })
            }
        }
    }

    /// Existing saved-feedback discovery remains available even when no inline
    /// browser view is attached. Draft inventory shares the same fresh Space
    /// resolution and is optional for records written before inline drafts.
    pub async fn feedback(
        &self,
        target: &BrowserTarget,
    ) -> Result<BrowserFeedbackLookup, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let target = self.resolve_target(target).await?;
        let key = super::association_key(
            &target.endpoint_identity,
            &target.session_id,
            &target.space_id,
        );
        let browser = match self.load(&key)? {
            Some(mut receipt) => self.status(&mut receipt).await?,
            None => BrowserResponse {
                association: None,
                connection: BrowserConnectionState::Absent,
                message: "No browser association exists for this Space".into(),
            },
        };
        Ok(BrowserFeedbackLookup {
            browser,
            feedback: self.feedback.list(&key)?,
            drafts: Some(self.draft_store()?.list(&key)?),
        })
    }

    pub async fn acknowledge_feedback(
        &self,
        request: BrowserFeedbackAckRequest,
    ) -> Result<BrowserFeedbackAck, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let target = self.resolve_target(&request.target).await?;
        let key = super::association_key(
            &target.endpoint_identity,
            &target.session_id,
            &target.space_id,
        );
        self.feedback.ack(&key, &request.ids)
    }

    pub fn prune_feedback(&self) -> Result<(), InspectionError> {
        self.feedback.prune()
    }

    pub fn browser_state_directory(&self) -> &Path {
        self.root.as_ref()
    }

    fn draft_store(&self) -> Result<BrowserDraftStore, InspectionError> {
        let state_root = self.root.parent().ok_or_else(|| {
            InspectionError::new("browser_draft_root", "Browser state root has no parent")
        })?;
        BrowserDraftStore::new(state_root.to_path_buf(), Arc::clone(&self.feedback))
    }

    async fn annotation_receipt(
        &self,
        target: &BrowserTarget,
        attachment: &BrowserRuntimeAttachment,
    ) -> Result<super::BrowserReceipt, InspectionError> {
        let resolved = self.resolve_target(target).await?;
        let association_key = super::association_key(
            &resolved.endpoint_identity,
            &resolved.session_id,
            &resolved.space_id,
        );
        if attachment.association_key != association_key {
            return Err(InspectionError::new("browser_draft_association", "Browser view belongs to another Space association"));
        }
        let receipt = self.load(&association_key)?.ok_or_else(|| {
            InspectionError::new("browser_draft_association", "Browser association is unavailable")
        })?;
        if receipt.incarnation.as_deref() != Some(attachment.browser_incarnation.as_str()) {
            return Err(InspectionError::new("browser_draft_incarnation", "Browser view belongs to a previous browser incarnation"));
        }
        Ok(receipt)
    }
}

fn required_draft_id(draft_id: Option<&str>) -> Result<&str, InspectionError> {
    draft_id.ok_or_else(|| InspectionError::new("browser_draft_command", "Draft ID is required"))
}

fn required_revision(revision: Option<u64>) -> Result<u64, InspectionError> {
    revision.ok_or_else(|| InspectionError::new("browser_draft_command", "Expected draft revision is required"))
}
