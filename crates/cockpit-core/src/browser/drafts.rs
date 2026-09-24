//! Owner-persisted inline browser drafts and composed capture recovery.
//!
//! The browser helper owns live pixels. This store owns only structured marks,
//! editor state, and the one frozen PNG that must survive a failed save.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use cockpit_protocol::{
    browser::{
        BrowserConnectionState, BrowserFeedbackAckRequest, BrowserFeedbackLookup, BrowserResponse,
        BrowserTarget,
    },
    browser_feedback::{
        BrowserAnnotation, BrowserCaptureContext, BrowserCaptureSaved, BrowserCaptureSubmission,
        BrowserFeedbackAck, BrowserInlineCaptureProvenance,
    },
    browser_view::{
        BROWSER_VIEW_MAX_NOTE_TEXT_CODE_UNITS, BrowserDraftRecoveryAction,
        BrowserDraftRecoveryRequest, BrowserViewCaptureCommand, BrowserViewCaptureOutcome,
        BrowserViewCommandOutcome, BrowserViewDocumentCommandContext, BrowserViewDraftAnnotation,
        BrowserViewDraftCommand, BrowserViewDraftEditorState, BrowserViewDraftInventory,
        BrowserViewDraftState, BrowserViewFrameDescriptor, BrowserViewInspectionFreshness,
        BrowserViewPendingCapture,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    InspectionError,
    browser_feedback::BrowserFeedbackStore,
    project_store::{
        atomic_write_bytes, atomic_write_json, open_dir_nofollow_absolute, prepare_root,
        read_json_bounded,
    },
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
    #[serde(default)]
    original_annotation_digests: Vec<String>,
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
    #[serde(default)]
    original_annotation_digests: Vec<String>,
    context: BrowserCaptureContext,
}
enum RecoveryMutation {
    Upsert(BrowserViewDraftAnnotation),
    Remove(String),
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
                .ok_or_else(|| {
                    InspectionError::new("browser_draft_not_found", "Draft is unavailable")
                })?;
            if draft.identity != stored_identity(identity) {
                return Err(InspectionError::new(
                    "browser_draft_identity",
                    "Draft belongs to another browser document",
                ));
            }
            if draft.tombstoned {
                return Err(InspectionError::new(
                    "browser_draft_tombstoned",
                    "Draft was discarded",
                ));
            }
            return Ok(public_draft(draft));
        }
        if let Some(draft) = drafts.iter().find(|draft| {
            !draft.tombstoned && !draft.stale && draft.identity == stored_identity(identity)
        }) {
            return Ok(public_draft(draft));
        }
        let referenced = self.load_capture_references()?;
        self.compact_retired(&mut drafts, &referenced, identity)?;
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
            editor: BrowserViewDraftEditorState {
                selected_annotation_id: None,
                notes_open: false,
                note_annotation_id: None,
                note_text: String::new(),
            },
            consumed_annotation_ids: Vec::new(),
            tombstoned: false,
        };
        self.write_draft(&stored)?;
        Ok(public_draft(&stored))
    }

    fn load_capture_references(&self) -> Result<HashSet<String>, InspectionError> {
        let dir = self.dir()?;
        let mut records = Vec::new();
        for (index, entry) in dir
            .entries()
            .map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))?
            .enumerate()
        {
            if index >= MAX_ENTRIES {
                return Err(InspectionError::new(
                    "browser_draft_bounded",
                    "Draft directory exceeded its entry limit",
                ));
            }
            let entry = entry
                .map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if let Some(association_key) = name
                .strip_prefix("pending-")
                .and_then(|name| name.strip_suffix(".json"))
            {
                records.push((true, association_key.to_owned()));
            } else if let Some(association_key) = name
                .strip_prefix("preparation-")
                .and_then(|name| name.strip_suffix(".json"))
            {
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

    /// Remove empty drafts nobody can use again: retired ones, and ones
    /// from an earlier browser process, whose pages are gone. An empty
    /// draft of this process may belong to another open tab and stays.
    fn compact_retired(
        &self,
        drafts: &mut Vec<StoredDraft>,
        referenced: &HashSet<String>,
        opening: &BrowserDraftIdentity,
    ) -> Result<(), InspectionError> {
        let removable: HashSet<_> = drafts
            .iter()
            .filter(|draft| {
                (draft.tombstoned
                    || draft.stale
                    || draft.identity.browser_incarnation != opening.browser_incarnation)
                    && draft.identity.association_key == opening.association_key
                    && draft.annotations.is_empty()
                    && draft.editor.note_text.is_empty()
                    && draft.editor.note_annotation_id.is_none()
                    && !referenced.contains(&draft.draft_id)
            })
            .map(|draft| draft.draft_id.clone())
            .collect();
        for draft_id in &removable {
            self.remove_draft(draft_id)?;
        }
        drafts.retain(|draft| !removable.contains(&draft.draft_id));
        Ok(())
    }

    pub fn list(
        &self,
        association_key: &str,
    ) -> Result<BrowserViewDraftInventory, InspectionError> {
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
            return Err(InspectionError::new(
                "browser_draft_identity",
                "Draft belongs to another document",
            ));
        }
        if draft.tombstoned {
            return Err(InspectionError::new(
                "browser_draft_tombstoned",
                "Draft was discarded",
            ));
        }
        if draft.revision != expected_revision {
            return Err(InspectionError::new(
                "browser_draft_revision",
                "Draft changed; recover the latest revision before editing",
            ));
        }
        match command {
            BrowserViewDraftCommand::UpsertAnnotation { annotation } => {
                validate_annotation(&annotation)?;
                if draft
                    .consumed_annotation_ids
                    .iter()
                    .any(|id| id == &annotation.id)
                {
                    return Err(InspectionError::new(
                        "browser_draft_consumed",
                        "A saved or removed annotation cannot be restored by a delayed save",
                    ));
                }
                if let Some(current) = draft
                    .annotations
                    .iter_mut()
                    .find(|current| current.id == annotation.id)
                {
                    *current = annotation;
                } else {
                    if draft.annotations.len() >= 64 {
                        return Err(InspectionError::new(
                            "browser_draft_annotation_limit",
                            "Draft has the maximum of 64 annotations",
                        ));
                    }
                    draft.annotations.push(annotation);
                }
            }
            BrowserViewDraftCommand::RemoveAnnotation { annotation_id } => {
                validate_uuid(&annotation_id, "annotation ID")?;
                let before = draft.annotations.len();
                draft
                    .annotations
                    .retain(|annotation| annotation.id != annotation_id);
                if draft.annotations.len() != before {
                    insert_consumed(&mut draft.consumed_annotation_ids, annotation_id.clone())?;
                    if draft.editor.note_annotation_id.as_deref() == Some(annotation_id.as_str()) {
                        draft.editor.note_annotation_id = None;
                        draft.editor.note_text.clear();
                    }
                    if draft.editor.selected_annotation_id.as_deref()
                        == Some(annotation_id.as_str())
                    {
                        draft.editor.selected_annotation_id = None;
                    }
                }
            }
            BrowserViewDraftCommand::Clear => {
                let ids: Vec<_> = draft
                    .annotations
                    .iter()
                    .map(|annotation| annotation.id.clone())
                    .collect();
                draft.annotations.clear();
                for id in ids {
                    insert_consumed(&mut draft.consumed_annotation_ids, id)?;
                }
                draft.editor.selected_annotation_id = None;
                draft.editor.note_annotation_id = None;
                draft.editor.note_text.clear();
            }
            BrowserViewDraftCommand::Discard => {
                let ids: Vec<_> = draft
                    .annotations
                    .iter()
                    .map(|annotation| annotation.id.clone())
                    .collect();
                for id in ids {
                    insert_consumed(&mut draft.consumed_annotation_ids, id)?;
                }
                draft.annotations.clear();
                draft.editor.selected_annotation_id = None;
                draft.editor.note_annotation_id = None;
                draft.editor.note_text.clear();
                draft.tombstoned = true;
            }
            BrowserViewDraftCommand::Open { .. }
            | BrowserViewDraftCommand::List
            | BrowserViewDraftCommand::SaveCapture { .. }
            | BrowserViewDraftCommand::RetryPending
            | BrowserViewDraftCommand::DiscardPending => {
                return Err(InspectionError::new(
                    "browser_draft_command",
                    "That draft command has a dedicated store operation",
                ));
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
        let prepared = self
            .load_preparation(&submission.association_key)?
            .ok_or_else(|| {
                InspectionError::new(
                    "browser_draft_capture",
                    "Capture was not prepared by the browser owner",
                )
            })?;
        if prepared.draft_id != draft_id
            || prepared.draft_revision != expected_revision
            || prepared.annotation_ids != annotation_ids
            || prepared.association_key != submission.association_key
            || prepared.browser_incarnation != submission.browser_instance
            || prepared.capture_id != submission.capture_id
            || prepared.context.inline_provenance.as_ref() != Some(&provenance)
        {
            return Err(InspectionError::new(
                "browser_draft_capture",
                "Capture does not match its owner-issued preparation",
            ));
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
            return Err(InspectionError::new(
                "browser_draft_identity",
                "Capture draft is no longer current",
            ));
        }
        if draft.revision != expected_revision {
            return Err(InspectionError::new(
                "browser_draft_revision",
                "Draft changed before capture",
            ));
        }
        let selected: HashSet<_> = annotation_ids.iter().collect();
        if selected.len() != annotation_ids.len()
            || draft
                .annotations
                .iter()
                .filter(|annotation| selected.contains(&annotation.id))
                .count()
                != annotation_ids.len()
        {
            return Err(InspectionError::new(
                "browser_draft_capture",
                "Capture selection does not match the current draft",
            ));
        }
        if self
            .load_pending(&capture.identity.association_key)?
            .is_some()
        {
            return Err(InspectionError::new(
                "browser_draft_pending",
                "Save or discard the pending capture before composing another",
            ));
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
            return Err(InspectionError::new(
                "browser_draft_capture",
                "A capture is already prepared for this association",
            ));
        }
        let original_annotation_digests = annotation_ids
            .iter()
            .map(|id| {
                let annotation = draft
                    .annotations
                    .iter()
                    .find(|annotation| &annotation.id == id)
                    .expect("selected annotations were validated above");
                annotation_digest(annotation)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let prepared = StoredCapturePreparation {
            format_version: FORMAT_VERSION,
            association_key: capture.identity.association_key,
            browser_incarnation: capture.identity.browser_incarnation,
            capture_id,
            draft_id: draft_id.to_owned(),
            draft_revision: expected_revision,
            annotation_ids,
            original_annotation_digests,
            context: capture.context,
        };
        atomic_write_json(
            &self.dir()?,
            &preparation_name(&prepared.association_key),
            &prepared,
        )
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
            Err(error) => Err(InspectionError::new(
                "browser_draft_pending",
                error.to_string(),
            )),
        }?;
        if let Err(error) = dir.remove_file(preparation_name(association_key))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(InspectionError::new(
                "browser_draft_pending",
                error.to_string(),
            ));
        }
        Ok(())
    }

    fn mutate_recovery(
        &self,
        association_key: &str,
        draft_id: &str,
        expected_revision: u64,
        mutation: RecoveryMutation,
    ) -> Result<BrowserViewDraftState, InspectionError> {
        let _guard = self.lock()?;
        validate_association_key(association_key)?;
        validate_uuid(draft_id, "draft ID")?;
        let mut draft = self.load_draft(draft_id)?.ok_or_else(|| {
            InspectionError::new("browser_draft_not_found", "Draft is unavailable")
        })?;
        if draft.identity.association_key != association_key {
            return Err(InspectionError::new(
                "browser_draft_identity",
                "Draft belongs to another browser association",
            ));
        }
        if draft.tombstoned {
            return Err(InspectionError::new(
                "browser_draft_tombstoned",
                "Draft was discarded",
            ));
        }
        if draft.revision != expected_revision {
            return Err(InspectionError::new(
                "browser_draft_revision",
                "Draft changed; recover the latest revision before editing",
            ));
        }
        match mutation {
            RecoveryMutation::Upsert(annotation) => {
                validate_annotation(&annotation)?;
                if draft
                    .consumed_annotation_ids
                    .iter()
                    .any(|id| id == &annotation.id)
                {
                    return Err(InspectionError::new(
                        "browser_draft_consumed",
                        "A saved or removed annotation cannot be restored by a delayed save",
                    ));
                }
                if let Some(current) = draft
                    .annotations
                    .iter_mut()
                    .find(|current| current.id == annotation.id)
                {
                    *current = annotation;
                } else {
                    if draft.annotations.len() >= 64 {
                        return Err(InspectionError::new(
                            "browser_draft_annotation_limit",
                            "Draft has the maximum of 64 annotations",
                        ));
                    }
                    draft.annotations.push(annotation);
                }
            }
            RecoveryMutation::Remove(annotation_id) => {
                validate_uuid(&annotation_id, "annotation ID")?;
                let before = draft.annotations.len();
                draft
                    .annotations
                    .retain(|annotation| annotation.id != annotation_id);
                if draft.annotations.len() != before {
                    insert_consumed(&mut draft.consumed_annotation_ids, annotation_id.clone())?;
                    if draft.editor.note_annotation_id.as_deref() == Some(annotation_id.as_str()) {
                        draft.editor.note_annotation_id = None;
                        draft.editor.note_text.clear();
                    }
                    if draft.editor.selected_annotation_id.as_deref()
                        == Some(annotation_id.as_str())
                    {
                        draft.editor.selected_annotation_id = None;
                    }
                }
            }
        }
        draft.revision = draft.revision.saturating_add(1);
        self.write_draft(&draft)?;
        Ok(public_draft(&draft))
    }

    pub fn set_editor_recovery(
        &self,
        association_key: &str,
        draft_id: &str,
        expected_revision: u64,
        editor: BrowserViewDraftEditorState,
    ) -> Result<BrowserViewDraftState, InspectionError> {
        let _guard = self.lock()?;
        validate_association_key(association_key)?;
        validate_uuid(draft_id, "draft ID")?;
        let mut draft = self.load_draft(draft_id)?.ok_or_else(|| {
            InspectionError::new("browser_draft_not_found", "Draft is unavailable")
        })?;
        if draft.identity.association_key != association_key {
            return Err(InspectionError::new(
                "browser_draft_identity",
                "Draft belongs to another browser association",
            ));
        }
        if draft.tombstoned {
            return Err(InspectionError::new(
                "browser_draft_tombstoned",
                "Draft was discarded",
            ));
        }
        if draft.revision != expected_revision {
            return Err(InspectionError::new(
                "browser_draft_revision",
                "Draft changed; recover the latest revision before editing",
            ));
        }
        validate_editor(&editor, &draft.annotations)?;
        draft.editor = editor;
        draft.revision = draft.revision.saturating_add(1);
        self.write_draft(&draft)?;
        Ok(public_draft(&draft))
    }
    pub fn upsert_annotation_recovery(
        &self,
        association_key: &str,
        draft_id: &str,
        expected_revision: u64,
        annotation: BrowserViewDraftAnnotation,
    ) -> Result<BrowserViewDraftState, InspectionError> {
        self.mutate_recovery(
            association_key,
            draft_id,
            expected_revision,
            RecoveryMutation::Upsert(annotation),
        )
    }

    pub fn remove_annotation_recovery(
        &self,
        association_key: &str,
        draft_id: &str,
        expected_revision: u64,
        annotation_id: String,
    ) -> Result<BrowserViewDraftState, InspectionError> {
        self.mutate_recovery(
            association_key,
            draft_id,
            expected_revision,
            RecoveryMutation::Remove(annotation_id),
        )
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
            return Err(InspectionError::new(
                "browser_draft_identity",
                "Draft belongs to another browser association",
            ));
        }
        if draft.revision != expected_revision {
            return Err(InspectionError::new(
                "browser_draft_revision",
                "Draft changed; recover the latest revision before discarding",
            ));
        }
        if self
            .load_preparation(association_key)?
            .as_ref()
            .is_some_and(|prepared| prepared.draft_id == draft_id)
        {
            return Err(InspectionError::new(
                "browser_draft_capture",
                "A capture is prepared for this draft; resolve it before discarding",
            ));
        }
        if self
            .load_pending(association_key)?
            .as_ref()
            .is_some_and(|pending| pending.draft_id == draft_id)
        {
            return Err(InspectionError::new(
                "browser_draft_pending",
                "Save or discard this pending capture before discarding its draft",
            ));
        }
        let ids: Vec<_> = draft
            .annotations
            .iter()
            .map(|annotation| annotation.id.clone())
            .collect();
        draft.annotations.clear();
        for id in ids {
            insert_consumed(&mut draft.consumed_annotation_ids, id)?;
        }
        draft.editor.selected_annotation_id = None;
        draft.editor.note_annotation_id = None;
        draft.editor.note_text.clear();
        draft.tombstoned = true;
        draft.revision = draft.revision.saturating_add(1);
        self.write_draft(&draft)
    }

    fn persist_pending(
        &self,
        prepared: &StoredCapturePreparation,
        submission: BrowserCaptureSubmission,
    ) -> Result<(), InspectionError> {
        let submitted: Vec<_> = submission
            .annotations
            .iter()
            .map(|annotation| annotation.id.clone())
            .collect();
        if submitted != prepared.annotation_ids {
            return Err(InspectionError::new(
                "browser_draft_capture",
                "Composed capture annotations do not match the selected draft marks",
            ));
        }
        if submission.association_key != prepared.association_key
            || submission.browser_instance != prepared.browser_incarnation
        {
            return Err(InspectionError::new(
                "browser_draft_capture",
                "Capture belongs to another browser association",
            ));
        }
        if let Some(existing) = self.load_pending(&prepared.association_key)? {
            if existing.submission.capture_id == submission.capture_id
                && existing.submission == submission
                && existing.annotation_ids == prepared.annotation_ids
            {
                return Ok(());
            }
            return Err(InspectionError::new(
                "browser_draft_pending",
                "Save or discard the pending capture before composing another",
            ));
        }
        let stored = StoredPendingCapture {
            format_version: FORMAT_VERSION,
            association_key: prepared.association_key.clone(),
            browser_incarnation: prepared.browser_incarnation.clone(),
            draft_id: prepared.draft_id.clone(),
            draft_revision: prepared.draft_revision,
            annotation_ids: prepared.annotation_ids.clone(),
            original_annotation_digests: prepared.original_annotation_digests.clone(),
            context: prepared.context.clone(),
            submission,
            last_error: None,
        };
        self.write_pending(&stored)
    }
    fn save_pending_locked(
        &self,
        association_key: &str,
    ) -> Result<BrowserViewCaptureOutcome, InspectionError> {
        let Some(mut pending) = self.load_pending(association_key)? else {
            return Ok(BrowserViewCaptureOutcome::Absent);
        };
        let saved = match self
            .feedback
            .save(pending.context.clone(), pending.submission.clone())
        {
            Ok(saved) => saved,
            Err(error) => {
                pending.last_error = Some(error.message.chars().take(1_024).collect());
                self.write_pending(&pending)?;
                return Ok(BrowserViewCaptureOutcome::Pending {
                    pending: public_pending(pending),
                });
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
        if saved.capture_id != pending.submission.capture_id
            || saved.annotation_ids != pending.annotation_ids
        {
            return Err(InspectionError::new(
                "browser_draft_receipt",
                "Feedback store returned a mismatched capture receipt",
            ));
        }
        let Some(mut draft) = self.load_draft(&pending.draft_id)? else {
            return Ok(());
        };
        if draft.identity.association_key != pending.association_key || draft.tombstoned {
            return Ok(());
        }
        let mut consumed = Vec::new();
        for (index, id) in pending.annotation_ids.iter().enumerate() {
            let Some(current) = draft
                .annotations
                .iter()
                .find(|annotation| &annotation.id == id)
            else {
                consumed.push(id.clone());
                continue;
            };
            let Some(frozen) = pending
                .submission
                .annotations
                .iter()
                .find(|annotation| &annotation.id == id)
            else {
                continue;
            };
            let note_changed = draft.editor.note_annotation_id.as_deref() == Some(id.as_str())
                && draft.editor.note_text != frozen.comment;
            // The captured marks are projected into PNG pixel coordinates.
            // Compare the original document-space mark frozen at preparation,
            // not its transformed public submission, before consuming it.
            let unchanged = if let Some(original) = pending.original_annotation_digests.get(index) {
                annotation_digest(current)? == *original
            } else if pending.original_annotation_digests.is_empty() {
                // Older pending captures lack frozen hashes. A matching store
                // revision proves no edit; otherwise preserve uncertain work.
                draft.revision == pending.draft_revision
                    || draft_annotation_matches_capture(current, frozen)
            } else {
                false
            };
            if unchanged && !note_changed {
                consumed.push(id.clone());
            }
        }
        let selected: HashSet<String> = consumed.iter().cloned().collect();
        let before_annotations = draft.annotations.len();
        draft
            .annotations
            .retain(|annotation| !selected.contains(&annotation.id));
        let mut changed = draft.annotations.len() != before_annotations;
        for id in consumed {
            let before = draft.consumed_annotation_ids.len();
            insert_consumed(&mut draft.consumed_annotation_ids, id)?;
            changed |= draft.consumed_annotation_ids.len() != before;
        }
        if draft
            .editor
            .selected_annotation_id
            .as_ref()
            .is_some_and(|id| selected.contains(id))
        {
            draft.editor.selected_annotation_id = None;
            changed = true;
        }
        if draft
            .editor
            .note_annotation_id
            .as_ref()
            .is_some_and(|id| selected.contains(id))
        {
            draft.editor.note_annotation_id = None;
            draft.editor.note_text.clear();
            changed = true;
        }
        if !changed {
            return Ok(());
        }
        draft.revision = draft.revision.saturating_add(1);
        self.write_draft(&draft)
    }

    fn dir(&self) -> Result<cap_std::fs::Dir, InspectionError> {
        open_dir_nofollow_absolute(&self.root)
            .map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, InspectionError> {
        self.mutation.lock().map_err(|_| {
            InspectionError::new("browser_draft_lock", "Draft store lock was poisoned")
        })
    }

    fn load_all(&self) -> Result<Vec<StoredDraft>, InspectionError> {
        let dir = self.dir()?;
        let mut drafts = Vec::new();
        for (index, entry) in dir
            .entries()
            .map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))?
            .enumerate()
        {
            if index >= MAX_ENTRIES {
                return Err(InspectionError::new(
                    "browser_draft_bounded",
                    "Draft directory exceeded its entry limit",
                ));
            }
            let entry = entry
                .map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(id) = name
                .strip_prefix("draft-")
                .and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            validate_uuid(id, "draft ID")?;
            let type_ = entry
                .file_type()
                .map_err(|error| InspectionError::new("browser_draft_read", error.to_string()))?;
            if type_.is_symlink() || !type_.is_file() {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "Draft record is not a regular file",
                ));
            }
            let mut draft: StoredDraft = read_json_bounded(&dir, name, MAX_DRAFT_BYTES)?;
            normalize_legacy_editor(&mut draft);
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
            Err(error) => Err(InspectionError::new(
                "browser_draft_read",
                error.to_string(),
            )),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
                InspectionError::new("unsafe_path", "Draft record is not a regular file"),
            ),
            Ok(_) => {
                let mut draft: StoredDraft = read_json_bounded(&dir, &name, MAX_DRAFT_BYTES)?;
                normalize_legacy_editor(&mut draft);
                validate_stored_draft(draft_id, &draft)?;
                Ok(Some(draft))
            }
        }
    }

    fn write_draft(&self, draft: &StoredDraft) -> Result<(), InspectionError> {
        validate_stored_draft(&draft.draft_id, draft)?;
        let bytes = bounded_record(
            draft,
            MAX_DRAFT_BYTES,
            "browser_draft_capacity",
            "Draft exceeds its durable storage limit",
        )?;
        atomic_write_bytes(&self.dir()?, &draft_name(&draft.draft_id), &bytes)
            .map_err(|error| InspectionError::new("browser_draft_write", error.to_string()))
    }
    fn remove_draft(&self, draft_id: &str) -> Result<(), InspectionError> {
        match self.dir()?.remove_file(draft_name(draft_id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(InspectionError::new(
                "browser_draft_write",
                error.to_string(),
            )),
        }
    }

    fn load_pending(
        &self,
        association_key: &str,
    ) -> Result<Option<StoredPendingCapture>, InspectionError> {
        let dir = self.dir()?;
        let name = pending_name(association_key);
        match dir.symlink_metadata(&name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(InspectionError::new(
                "browser_draft_pending",
                error.to_string(),
            )),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
                InspectionError::new("unsafe_path", "Pending capture is not a regular file"),
            ),
            Ok(_) => {
                let pending: StoredPendingCapture =
                    read_json_bounded(&dir, &name, MAX_PENDING_BYTES)?;
                validate_pending(&pending)?;
                Ok(Some(pending))
            }
        }
    }

    fn remove_pending(&self, association_key: &str) -> Result<(), InspectionError> {
        self.dir()?
            .remove_file(pending_name(association_key))
            .map_err(|error| InspectionError::new("browser_draft_pending", error.to_string()))
    }

    fn load_preparation(
        &self,
        association_key: &str,
    ) -> Result<Option<StoredCapturePreparation>, InspectionError> {
        let dir = self.dir()?;
        let name = preparation_name(association_key);
        match dir.symlink_metadata(&name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(InspectionError::new(
                "browser_draft_capture",
                error.to_string(),
            )),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
                InspectionError::new("unsafe_path", "Capture preparation is not a regular file"),
            ),
            Ok(_) => {
                let prepared: StoredCapturePreparation =
                    read_json_bounded(&dir, &name, MAX_PREPARATION_BYTES)?;
                validate_preparation(&prepared)?;
                Ok(Some(prepared))
            }
        }
    }

    fn remove_preparation(&self, association_key: &str) -> Result<(), InspectionError> {
        self.dir()?
            .remove_file(preparation_name(association_key))
            .map_err(|error| InspectionError::new("browser_draft_capture", error.to_string()))
    }

    fn write_pending(&self, pending: &StoredPendingCapture) -> Result<(), InspectionError> {
        let bytes = bounded_record(
            pending,
            MAX_PENDING_BYTES,
            "browser_draft_capacity",
            "Pending capture exceeds its durable storage limit",
        )?;
        atomic_write_bytes(
            &self.dir()?,
            &pending_name(&pending.association_key),
            &bytes,
        )
        .map_err(|error| InspectionError::new("browser_draft_pending", error.to_string()))
    }
}

fn bounded_record<T: Serialize>(
    value: &T,
    limit: u64,
    code: &str,
    message: &str,
) -> Result<Vec<u8>, InspectionError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| InspectionError::new(code, error.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(InspectionError::new(code, message));
    }
    Ok(bytes)
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
        association_key: pending.association_key,
        browser_incarnation: pending.browser_incarnation,
        capture_id: pending.submission.capture_id,
        draft_id: pending.draft_id,
        draft_revision: pending.draft_revision,
        annotation_ids: pending.annotation_ids,
        last_error: pending.last_error,
    }
}
fn annotation_digest(annotation: &BrowserViewDraftAnnotation) -> Result<String, InspectionError> {
    let bytes = serde_json::to_vec(annotation)
        .map_err(|error| InspectionError::new("browser_draft_capture", error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

fn valid_annotation_digests(ids: &[String], digests: &[String]) -> bool {
    digests.is_empty()
        || (digests.len() == ids.len()
            && digests.iter().all(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            }))
}

fn draft_annotation_matches_capture(
    draft: &BrowserViewDraftAnnotation,
    capture: &BrowserAnnotation,
) -> bool {
    draft.id == capture.id
        && draft.kind == capture.kind
        && draft.color == capture.color
        && draft.points == capture.points
        && draft.bounds == capture.bounds
        && draft.evidence == capture.element
        && draft.comment.as_deref().unwrap_or_default() == capture.comment
}

fn validate_capture_context(
    identity: &BrowserDraftIdentity,
    context: &BrowserCaptureContext,
) -> Result<(), InspectionError> {
    if context.association_key != identity.association_key
        || context.browser_instance != identity.browser_incarnation
    {
        return Err(InspectionError::new(
            "browser_draft_context",
            "Capture context belongs to another browser association",
        ));
    }
    let provenance: &BrowserInlineCaptureProvenance =
        context.inline_provenance.as_ref().ok_or_else(|| {
            InspectionError::new(
                "browser_draft_provenance",
                "Inline captures require target and frame provenance",
            )
        })?;
    if provenance.target_id != identity.target_id
        || provenance.document_generation != identity.document_generation
    {
        return Err(InspectionError::new(
            "browser_draft_provenance",
            "Capture provenance does not match the draft document",
        ));
    }
    validate_id(&provenance.frame_id, "frame")?;
    Ok(())
}

fn normalize_legacy_editor(draft: &mut StoredDraft) {
    if draft
        .editor
        .selected_annotation_id
        .as_ref()
        .is_some_and(|id| {
            !draft
                .annotations
                .iter()
                .any(|annotation| &annotation.id == id)
        })
    {
        draft.editor.selected_annotation_id = None;
    }
}

fn validate_stored_draft(id: &str, draft: &StoredDraft) -> Result<(), InspectionError> {
    if draft.format_version != FORMAT_VERSION
        || draft.draft_id != id
        || draft.annotations.len() > 64
    {
        return Err(InspectionError::new(
            "browser_draft_corrupt",
            "Draft record is invalid",
        ));
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
        if !ids.insert(&annotation.id)
            || draft
                .consumed_annotation_ids
                .iter()
                .any(|consumed| consumed == &annotation.id)
        {
            return Err(InspectionError::new(
                "browser_draft_corrupt",
                "Draft annotation IDs are invalid",
            ));
        }
    }
    validate_editor(&draft.editor, &draft.annotations)?;
    validate_annotation_ids(&draft.consumed_annotation_ids)
}

fn validate_pending(pending: &StoredPendingCapture) -> Result<(), InspectionError> {
    if pending.format_version != FORMAT_VERSION
        || pending.submission.association_key != pending.association_key
        || pending.submission.browser_instance != pending.browser_incarnation
    {
        return Err(InspectionError::new(
            "browser_draft_pending",
            "Pending capture is invalid",
        ));
    }
    validate_association_key(&pending.association_key)?;
    validate_id(&pending.browser_incarnation, "browser incarnation")?;
    validate_uuid(&pending.draft_id, "draft ID")?;
    if pending.context.association_key != pending.association_key
        || pending.context.browser_instance != pending.browser_incarnation
    {
        return Err(InspectionError::new(
            "browser_draft_pending",
            "Pending capture context is invalid",
        ));
    }
    validate_annotation_ids(&pending.annotation_ids)?;
    if !valid_annotation_digests(
        &pending.annotation_ids,
        &pending.original_annotation_digests,
    ) {
        return Err(InspectionError::new(
            "browser_draft_pending",
            "Pending capture digests are invalid",
        ));
    }
    Ok(())
}

fn validate_preparation(prepared: &StoredCapturePreparation) -> Result<(), InspectionError> {
    if prepared.format_version != FORMAT_VERSION
        || prepared.context.association_key != prepared.association_key
        || prepared.context.browser_instance != prepared.browser_incarnation
    {
        return Err(InspectionError::new(
            "browser_draft_capture",
            "Capture preparation is invalid",
        ));
    }
    validate_association_key(&prepared.association_key)?;
    validate_id(&prepared.browser_incarnation, "browser incarnation")?;
    validate_uuid(&prepared.capture_id, "capture ID")?;
    validate_uuid(&prepared.draft_id, "draft ID")?;
    validate_annotation_ids(&prepared.annotation_ids)?;
    if !valid_annotation_digests(
        &prepared.annotation_ids,
        &prepared.original_annotation_digests,
    ) {
        return Err(InspectionError::new(
            "browser_draft_capture",
            "Capture preparation digests are invalid",
        ));
    }
    Ok(())
}

fn validate_identity(identity: &BrowserDraftIdentity) -> Result<(), InspectionError> {
    validate_association_key(&identity.association_key)?;
    validate_id(&identity.browser_incarnation, "browser incarnation")?;
    validate_id(&identity.target_id, "target")
}

fn validate_editor(
    editor: &BrowserViewDraftEditorState,
    annotations: &[BrowserViewDraftAnnotation],
) -> Result<(), InspectionError> {
    for annotation_id in [
        editor.selected_annotation_id.as_ref(),
        editor.note_annotation_id.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_uuid(annotation_id, "editor annotation ID")?;
        if !annotations
            .iter()
            .any(|annotation| &annotation.id == annotation_id)
        {
            return Err(InspectionError::new(
                "browser_draft_editor",
                "The editor annotation is unavailable",
            ));
        }
    }
    if editor.note_annotation_id.is_none() && !editor.note_text.is_empty() {
        return Err(InspectionError::new(
            "browser_draft_editor",
            "Note text requires an active annotation",
        ));
    }
    if editor.note_text.encode_utf16().count() > BROWSER_VIEW_MAX_NOTE_TEXT_CODE_UNITS {
        return Err(InspectionError::new(
            "browser_draft_editor",
            "Note text exceeds 4000 UTF-16 code units",
        ));
    }
    Ok(())
}
fn validate_annotation(annotation: &BrowserViewDraftAnnotation) -> Result<(), InspectionError> {
    validate_uuid(&annotation.id, "annotation ID")?;
    if annotation.points.len() > 8_192
        || annotation.color.len() > 32
        || annotation
            .comment
            .as_ref()
            .is_some_and(|comment| comment.len() > 64 * 1024)
    {
        return Err(InspectionError::new(
            "browser_draft_annotation",
            "Draft annotation exceeds bounds",
        ));
    }
    Ok(())
}

fn validate_annotation_ids(ids: &[String]) -> Result<(), InspectionError> {
    if ids.len() > 64 || ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(InspectionError::new(
            "browser_draft_annotation",
            "Annotation IDs are invalid",
        ));
    }
    for id in ids {
        validate_uuid(id, "annotation ID")?;
    }
    Ok(())
}

fn insert_consumed(ids: &mut Vec<String>, id: String) -> Result<(), InspectionError> {
    validate_uuid(&id, "annotation ID")?;
    if !ids.contains(&id) {
        ids.push(id);
    }
    if ids.len() > 512 {
        return Err(InspectionError::new(
            "browser_draft_consumed",
            "Draft consumed-ID history exceeds its bound",
        ));
    }
    Ok(())
}

fn validate_association_key(value: &str) -> Result<(), InspectionError> {
    if value.len() != 24 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(InspectionError::new(
            "browser_draft_association",
            "Association key must be 24 hexadecimal characters",
        ));
    }
    Ok(())
}

fn validate_uuid(value: &str, field: &str) -> Result<(), InspectionError> {
    if Uuid::parse_str(value).is_err() {
        return Err(InspectionError::new(
            "browser_draft_id",
            format!("{field} must be a UUID"),
        ));
    }
    Ok(())
}

fn validate_id(value: &str, field: &str) -> Result<(), InspectionError> {
    if value.is_empty() || value.len() > 512 || value.contains(['/', '\\', '\0']) {
        return Err(InspectionError::new(
            "browser_draft_id",
            format!("{field} ID is invalid"),
        ));
    }
    Ok(())
}

fn draft_name(id: &str) -> String {
    format!("draft-{id}.json")
}
fn pending_name(association_key: &str) -> String {
    format!("pending-{association_key}.json")
}
fn preparation_name(association_key: &str) -> String {
    format!("preparation-{association_key}.json")
}

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
                Ok(BrowserViewCommandOutcome::Capture {
                    capture: BrowserViewCaptureOutcome::Absent,
                })
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
                    return Err(InspectionError::new(
                        "browser_draft_target",
                        "Draft command targets another browser tab",
                    ));
                }
                let identity = BrowserDraftIdentity {
                    association_key,
                    browser_incarnation: attachment.browser_incarnation.clone(),
                    target_id: context.target_id,
                    document_generation: context.document_generation,
                };
                match command {
                    BrowserViewDraftCommand::Open {
                        draft_id: requested,
                    } => Ok(BrowserViewCommandOutcome::Draft {
                        draft: store.open(&identity, requested)?,
                    }),
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
            return Err(InspectionError::new(
                "browser_draft_capture",
                "Capture preparation no longer matches the presented frame",
            ));
        }
        let identity = BrowserDraftIdentity {
            association_key: receipt.association_key.clone(),
            browser_incarnation: attachment.browser_incarnation.clone(),
            target_id: attachment.target_id.clone(),
            document_generation: descriptor.document_generation,
        };
        let address = self.association(
            &receipt,
            cockpit_protocol::browser::BrowserConnectionState::Open,
        );
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
        request
            .validate()
            .map_err(|message| InspectionError::new("browser_draft_command", message))?;
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
                Ok(BrowserViewCommandOutcome::Capture {
                    capture: BrowserViewCaptureOutcome::Absent,
                })
            }
            BrowserDraftRecoveryAction::SetEditor {
                draft_id,
                expected_revision,
                editor,
            } => Ok(BrowserViewCommandOutcome::Draft {
                draft: store.set_editor_recovery(
                    &association_key,
                    &draft_id,
                    expected_revision,
                    editor,
                )?,
            }),
            BrowserDraftRecoveryAction::UpsertAnnotation {
                draft_id,
                expected_revision,
                annotation,
            } => Ok(BrowserViewCommandOutcome::Draft {
                draft: store.upsert_annotation_recovery(
                    &association_key,
                    &draft_id,
                    expected_revision,
                    annotation,
                )?,
            }),
            BrowserDraftRecoveryAction::RemoveAnnotation {
                draft_id,
                expected_revision,
                annotation_id,
            } => Ok(BrowserViewCommandOutcome::Draft {
                draft: store.remove_annotation_recovery(
                    &association_key,
                    &draft_id,
                    expected_revision,
                    annotation_id,
                )?,
            }),
            BrowserDraftRecoveryAction::DiscardDraft {
                draft_id,
                expected_revision,
            } => {
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
        let feedback = self.feedback.list(&key)?;
        let mut deliveries = self
            .feedback
            .list_delivery_statuses(&key, &feedback.captures)?;
        if self.settle_interrupted_deliveries(&deliveries)? {
            deliveries = self
                .feedback
                .list_delivery_statuses(&key, &feedback.captures)?;
        }
        Ok(BrowserFeedbackLookup {
            browser,
            feedback,
            deliveries,
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
            return Err(InspectionError::new(
                "browser_draft_association",
                "Browser view belongs to another Space association",
            ));
        }
        let receipt = self.load(&association_key)?.ok_or_else(|| {
            InspectionError::new(
                "browser_draft_association",
                "Browser association is unavailable",
            )
        })?;
        if receipt.incarnation.as_deref() != Some(attachment.browser_incarnation.as_str()) {
            return Err(InspectionError::new(
                "browser_draft_incarnation",
                "Browser view belongs to a previous browser incarnation",
            ));
        }
        Ok(receipt)
    }
}

fn required_draft_id(draft_id: Option<&str>) -> Result<&str, InspectionError> {
    draft_id.ok_or_else(|| InspectionError::new("browser_draft_command", "Draft ID is required"))
}

fn required_revision(revision: Option<u64>) -> Result<u64, InspectionError> {
    revision.ok_or_else(|| {
        InspectionError::new(
            "browser_draft_command",
            "Expected draft revision is required",
        )
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_feedback::{BrowserFeedbackOptions, BrowserFeedbackStore};
    use cockpit_protocol::browser_feedback::{
        BrowserAnnotation, BrowserAnnotationKind, BrowserCaptureSaved, BrowserPageEvidence,
        BrowserPoint, BrowserRect, BrowserViewport,
    };

    fn test_store() -> (BrowserDraftStore, PathBuf) {
        let root = std::env::temp_dir().join(format!("cockpit-browser-drafts-{}", Uuid::new_v4()));
        let feedback = Arc::new(
            BrowserFeedbackStore::new(root.clone(), BrowserFeedbackOptions::default()).unwrap(),
        );
        let store = BrowserDraftStore::new(root.clone(), feedback).unwrap();
        (store, root)
    }

    fn identity(
        association_key: &str,
        browser_incarnation: &str,
        target_id: &str,
        document_generation: u64,
    ) -> BrowserDraftIdentity {
        BrowserDraftIdentity {
            association_key: association_key.to_owned(),
            browser_incarnation: browser_incarnation.to_owned(),
            target_id: target_id.to_owned(),
            document_generation,
        }
    }

    fn draft(identity: &BrowserDraftIdentity, draft_id: String) -> StoredDraft {
        StoredDraft {
            format_version: FORMAT_VERSION,
            identity: stored_identity(identity),
            draft_id,
            revision: 1,
            annotations: Vec::new(),
            freshness: BrowserViewInspectionFreshness::Fresh,
            stale: false,
            editor: BrowserViewDraftEditorState {
                selected_annotation_id: None,
                notes_open: false,
                note_annotation_id: None,
                note_text: String::new(),
            },
            consumed_annotation_ids: Vec::new(),
            tombstoned: false,
        }
    }

    #[test]
    fn opening_new_incarnation_preserves_unsent_work_and_compacts_empty_drafts() {
        let (store, root) = test_store();
        let association_key = "0123456789abcdef01234567";
        let other_association_key = "89abcdef0123456701234567";
        let old_incarnation = Uuid::new_v4().to_string();
        let current_incarnation = Uuid::new_v4().to_string();
        let referenced_id = Uuid::new_v4().to_string();
        let mut old_ids = Vec::new();

        for generation in 1..=MAX_DRAFTS as u64 {
            let draft_id = if generation == 1 {
                referenced_id.clone()
            } else {
                Uuid::new_v4().to_string()
            };
            old_ids.push(draft_id.clone());
            let old_identity = identity(association_key, &old_incarnation, "target", generation);
            let mut stored = draft(&old_identity, draft_id);
            if generation == 2 {
                let annotation_id = Uuid::new_v4().to_string();
                stored.annotations.push(BrowserViewDraftAnnotation {
                    id: annotation_id.clone(),
                    kind: BrowserAnnotationKind::Region,
                    color: "#f00".to_owned(),
                    points: vec![BrowserPoint { x: 1.0, y: 1.0 }],
                    bounds: None,
                    evidence: None,
                    comment: Some("unsent".to_owned()),
                });
                stored.editor.note_annotation_id = Some(annotation_id);
                stored.editor.note_text = "unsent note".to_owned();
            } else {
                stored.stale = true;
                stored.freshness = BrowserViewInspectionFreshness::Stale;
            }
            store.write_draft(&stored).unwrap();
        }

        let preparation = StoredCapturePreparation {
            format_version: FORMAT_VERSION,
            association_key: association_key.to_owned(),
            browser_incarnation: old_incarnation.clone(),
            capture_id: Uuid::new_v4().to_string(),
            draft_id: referenced_id.clone(),
            draft_revision: 1,
            annotation_ids: Vec::new(),
            original_annotation_digests: Vec::new(),
            context: BrowserCaptureContext {
                association_key: association_key.to_owned(),
                session_id: "session".to_owned(),
                space_id: "space".to_owned(),
                space_label: "Space".to_owned(),
                playwright_session: "playwright".to_owned(),
                working_directory: "/tmp".to_owned(),
                invocation: "test".to_owned(),
                browser_instance: old_incarnation.clone(),
                inline_provenance: None,
            },
        };
        atomic_write_json(
            &store.dir().unwrap(),
            &preparation_name(association_key),
            &preparation,
        )
        .unwrap();

        let other_id = Uuid::new_v4().to_string();
        let other_identity = identity(other_association_key, &old_incarnation, "other-target", 1);
        store
            .write_draft(&draft(&other_identity, other_id.clone()))
            .unwrap();

        let current_identity = identity(association_key, &current_incarnation, "target", 1);
        let opened = store.open(&current_identity, None).unwrap();
        let unsent = store.load_draft(&old_ids[1]).unwrap().unwrap();
        assert_eq!(unsent.annotations.len(), 1);
        assert!(!unsent.stale);
        assert_eq!(unsent.editor.note_text, "unsent note");
        assert_eq!(opened.document_generation, 1);
        assert!(store.load_preparation(association_key).unwrap().is_some());
        let referenced = store.load_draft(&referenced_id).unwrap().unwrap();
        assert!(!referenced.tombstoned);
        assert!(referenced.stale);
        assert_eq!(
            store
                .load_draft(&old_ids[1])
                .unwrap()
                .unwrap()
                .editor
                .note_text,
            "unsent note"
        );
        for draft_id in old_ids.into_iter().skip(2) {
            assert!(store.load_draft(&draft_id).unwrap().is_none());
        }
        assert!(store.load_draft(&other_id).unwrap().is_some());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restarting_at_capacity_does_not_evict_unsubmitted_annotations() {
        let (store, root) = test_store();
        let key = "0123456789abcdef01234567";
        let old_incarnation = Uuid::new_v4().to_string();
        let mut ids = Vec::new();
        for generation in 1..=MAX_DRAFTS as u64 {
            let mut stored = draft(
                &identity(key, &old_incarnation, "target", generation),
                Uuid::new_v4().to_string(),
            );
            stored.annotations.push(BrowserViewDraftAnnotation {
                id: Uuid::new_v4().to_string(),
                kind: BrowserAnnotationKind::Region,
                color: "#f00".to_owned(),
                points: vec![BrowserPoint { x: 1.0, y: 1.0 }],
                bounds: None,
                evidence: None,
                comment: Some("unsent".to_owned()),
            });
            ids.push(stored.draft_id.clone());
            store.write_draft(&stored).unwrap();
        }
        let current = identity(key, &Uuid::new_v4().to_string(), "target", 1);
        let error = store
            .open(&current, None)
            .expect_err("a restart cannot evict unsent work");
        assert_eq!(error.code, "browser_draft_capacity");
        assert_eq!(store.list(key).unwrap().drafts.len(), MAX_DRAFTS);
        for id in ids {
            assert_eq!(store.load_draft(&id).unwrap().unwrap().annotations.len(), 1);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_drafts_from_an_earlier_browser_do_not_fill_capacity() {
        let (store, root) = test_store();
        let key = "0123456789abcdef01234567";
        let old_incarnation = Uuid::new_v4().to_string();
        for generation in 1..=MAX_DRAFTS as u64 {
            let stored = draft(
                &identity(key, &old_incarnation, "target", generation),
                Uuid::new_v4().to_string(),
            );
            store.write_draft(&stored).unwrap();
        }
        let incarnation = Uuid::new_v4().to_string();
        let current = identity(key, &incarnation, "target", 1);
        store
            .open(&current, None)
            .expect("empty drafts of a closed browser are compacted");
        assert_eq!(store.list(key).unwrap().drafts.len(), 1);
        // Empty drafts of the live browser (other tabs) are kept.
        let other_tab = identity(key, &incarnation, "other", 1);
        store.open(&other_tab, None).unwrap();
        store
            .open(&identity(key, &incarnation, "third", 1), None)
            .unwrap();
        assert_eq!(store.list(key).unwrap().drafts.len(), 3);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn switching_targets_preserves_drafts_and_capture_authority() {
        let (store, root) = test_store();
        let key = "0123456789abcdef01234567";
        let incarnation = Uuid::new_v4().to_string();
        let a = identity(key, &incarnation, "target-a", 1);
        let first = store.open(&a, None).unwrap();
        let annotation = BrowserViewDraftAnnotation {
            id: Uuid::new_v4().to_string(),
            kind: BrowserAnnotationKind::Region,
            color: "#f00".to_owned(),
            points: vec![BrowserPoint { x: 1.0, y: 1.0 }],
            bounds: None,
            evidence: None,
            comment: Some("unsent note".to_owned()),
        };
        let mut stored = store.load_draft(&first.draft_id).unwrap().unwrap();
        stored.annotations.push(annotation.clone());
        stored.editor.note_annotation_id = Some(annotation.id.clone());
        stored.editor.note_text = "unsent note".to_owned();
        store.write_draft(&stored).unwrap();

        let b = identity(key, &incarnation, "target-b", 1);
        store.open(&b, None).unwrap();
        let reopened = store.open(&a, None).unwrap();
        assert_eq!(reopened.draft_id, first.draft_id);
        assert_eq!(reopened.annotations, vec![annotation.clone()]);
        assert_eq!(reopened.editor.note_text, "unsent note");
        let context = BrowserCaptureContext {
            association_key: key.to_owned(),
            session_id: "session".to_owned(),
            space_id: "space".to_owned(),
            space_label: "Space".to_owned(),
            playwright_session: "playwright".to_owned(),
            working_directory: "/tmp".to_owned(),
            invocation: "test".to_owned(),
            browser_instance: incarnation.clone(),
            inline_provenance: Some(BrowserInlineCaptureProvenance {
                target_id: a.target_id.clone(),
                frame_id: "frame-a".to_owned(),
                document_generation: a.document_generation,
                frame_generation: 1,
                stream_epoch: 1,
                frame_sequence: 1,
                viewport_revision: 1,
                pixel_captured_at_micros: 1,
                capture_as_shown: true,
            }),
        };
        store
            .prepare_capture(
                BrowserDraftCaptureContext {
                    identity: a.clone(),
                    context: context.clone(),
                },
                &reopened.draft_id,
                reopened.revision,
                vec![annotation.id.clone()],
                Uuid::new_v4().to_string(),
            )
            .unwrap();

        let next = identity(key, &incarnation, "target-a", 2);
        store.mark_stale(key, &incarnation, "target-a", 2).unwrap();
        let rejected = store
            .prepare_capture(
                BrowserDraftCaptureContext {
                    identity: a,
                    context,
                },
                &reopened.draft_id,
                reopened.revision,
                vec![annotation.id],
                Uuid::new_v4().to_string(),
            )
            .expect_err("authoritative same-target navigation invalidates capture");
        assert_eq!(rejected.code, "browser_draft_identity");
        assert!(store.open(&next, None).unwrap().annotations.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opening_a_draft_requires_its_exact_durable_identity() {
        let (store, root) = test_store();
        let draft_id = Uuid::new_v4().to_string();
        let owner = identity(
            "0123456789abcdef01234567",
            &Uuid::new_v4().to_string(),
            "target",
            7,
        );
        store.write_draft(&draft(&owner, draft_id.clone())).unwrap();
        let foreign = identity(
            "89abcdef0123456701234567",
            owner.browser_incarnation.as_str(),
            "target",
            7,
        );
        let error = store
            .open(&foreign, Some(draft_id))
            .expect_err("foreign association must not recover draft");
        assert_eq!(error.code, "browser_draft_identity");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_client_revisions_and_consumed_ids_cannot_resurrect_removed_marks() {
        let (store, root) = test_store();
        let key = "0123456789abcdef01234567";
        let owner = identity(key, &Uuid::new_v4().to_string(), "target", 1);
        let initial = store.open(&owner, None).unwrap();
        let mark = BrowserViewDraftAnnotation {
            id: Uuid::new_v4().to_string(),
            kind: BrowserAnnotationKind::Region,
            color: "#f00".to_owned(),
            points: vec![BrowserPoint { x: 1.0, y: 1.0 }],
            bounds: None,
            evidence: None,
            comment: Some("winner".to_owned()),
        };
        let competing = BrowserViewDraftAnnotation {
            id: Uuid::new_v4().to_string(),
            comment: Some("stale competing client".to_owned()),
            ..mark.clone()
        };
        let barrier = std::sync::Barrier::new(2);
        let (left, right) = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                store.upsert_annotation_recovery(
                    key,
                    &initial.draft_id,
                    initial.revision,
                    mark.clone(),
                )
            });
            let second = scope.spawn(|| {
                barrier.wait();
                store.upsert_annotation_recovery(
                    key,
                    &initial.draft_id,
                    initial.revision,
                    competing.clone(),
                )
            });
            (first.join().unwrap(), second.join().unwrap())
        });
        let (saved, winner, loser, conflict) = match (left, right) {
            (Ok(saved), Err(conflict)) => (saved, mark, competing, conflict),
            (Err(conflict), Ok(saved)) => (saved, competing, mark, conflict),
            outcome => panic!("exactly one competing edit must win: {outcome:?}"),
        };
        assert_eq!(conflict.code, "browser_draft_revision");
        assert_eq!(
            store
                .load_draft(&initial.draft_id)
                .unwrap()
                .unwrap()
                .annotations,
            vec![winner.clone()]
        );
        let removed = store
            .remove_annotation_recovery(key, &initial.draft_id, saved.revision, winner.id.clone())
            .unwrap();
        let resurrected = store
            .upsert_annotation_recovery(key, &initial.draft_id, removed.revision, winner)
            .expect_err("a delayed save cannot restore a consumed annotation");
        assert_eq!(resurrected.code, "browser_draft_consumed");
        store
            .discard_draft(key, &initial.draft_id, removed.revision)
            .unwrap();
        let discarded = store
            .upsert_annotation_recovery(key, &initial.draft_id, removed.revision, loser)
            .expect_err("a discarded draft cannot accept delayed edits");
        let context = BrowserCaptureContext {
            association_key: key.to_owned(),
            session_id: "session".to_owned(),
            space_id: "space".to_owned(),
            space_label: "Space".to_owned(),
            playwright_session: "playwright".to_owned(),
            working_directory: "/tmp".to_owned(),
            invocation: "test".to_owned(),
            browser_instance: owner.browser_incarnation.clone(),
            inline_provenance: Some(BrowserInlineCaptureProvenance {
                target_id: owner.target_id.clone(),
                frame_id: "frame-target".to_owned(),
                document_generation: owner.document_generation,
                frame_generation: 1,
                stream_epoch: 1,
                frame_sequence: 1,
                viewport_revision: 1,
                pixel_captured_at_micros: 1,
                capture_as_shown: true,
            }),
        };
        let capture_error = store
            .prepare_capture(
                BrowserDraftCaptureContext {
                    identity: owner,
                    context,
                },
                &initial.draft_id,
                removed.revision,
                Vec::new(),
                Uuid::new_v4().to_string(),
            )
            .expect_err("tombstoned drafts cannot be captured");
        assert_eq!(capture_error.code, "browser_draft_identity");
        assert!(store.list(key).unwrap().drafts.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prepared_capture_preserves_its_draft_against_discard() {
        let (store, root) = test_store();
        let association_key = "0123456789abcdef01234567";
        let browser_incarnation = Uuid::new_v4().to_string();
        let owner = identity(association_key, &browser_incarnation, "target", 1);
        let current = store.open(&owner, None).unwrap();
        let context = BrowserCaptureContext {
            association_key: association_key.to_owned(),
            session_id: "session".to_owned(),
            space_id: "space".to_owned(),
            space_label: "Space".to_owned(),
            playwright_session: "playwright".to_owned(),
            working_directory: "/tmp".to_owned(),
            invocation: "test".to_owned(),
            browser_instance: browser_incarnation.clone(),
            inline_provenance: None,
        };
        let preparation = StoredCapturePreparation {
            format_version: FORMAT_VERSION,
            association_key: association_key.to_owned(),
            browser_incarnation,
            capture_id: Uuid::new_v4().to_string(),
            draft_id: current.draft_id.clone(),
            draft_revision: current.revision,
            annotation_ids: Vec::new(),
            original_annotation_digests: Vec::new(),
            context,
        };
        atomic_write_json(
            &store.dir().unwrap(),
            &preparation_name(association_key),
            &preparation,
        )
        .unwrap();

        let failure = store
            .discard_draft(association_key, &current.draft_id, current.revision)
            .expect_err("a prepared capture still owns this draft");
        assert_eq!(failure.code, "browser_draft_capture");
        assert!(
            store
                .list(association_key)
                .unwrap()
                .drafts
                .iter()
                .any(|draft| draft.draft_id == current.draft_id)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delayed_capture_receipt_preserves_newer_annotation_and_note_edits() {
        let (store, root) = test_store();
        let association_key = "0123456789abcdef01234567";
        let browser_incarnation = Uuid::new_v4().to_string();
        let draft_id = Uuid::new_v4().to_string();
        let annotation_id = Uuid::new_v4().to_string();
        let owner = identity(association_key, &browser_incarnation, "target", 1);
        let frozen = BrowserAnnotation {
            id: annotation_id.clone(),
            kind: BrowserAnnotationKind::Region,
            comment: "old note".to_owned(),
            color: "#f00".to_owned(),
            points: vec![BrowserPoint { x: 1.0, y: 1.0 }],
            bounds: None,
            element: None,
        };
        let original = BrowserViewDraftAnnotation {
            id: annotation_id.clone(),
            kind: BrowserAnnotationKind::Region,
            color: "#f00".to_owned(),
            points: vec![BrowserPoint { x: 1.0, y: 1.0 }],
            bounds: None,
            evidence: None,
            comment: Some("old note".to_owned()),
        };
        let mut current = draft(&owner, draft_id.clone());
        current.revision = 2;
        current.annotations.push(BrowserViewDraftAnnotation {
            id: annotation_id.clone(),
            kind: BrowserAnnotationKind::Region,
            color: "#f00".to_owned(),
            points: vec![BrowserPoint { x: 1.0, y: 1.0 }],
            bounds: None,
            evidence: None,
            comment: Some("newer annotation".to_owned()),
        });
        current.editor.note_annotation_id = Some(annotation_id.clone());
        current.editor.note_text = "newer note".to_owned();
        store.write_draft(&current).unwrap();
        let context = BrowserCaptureContext {
            association_key: association_key.to_owned(),
            session_id: "session".to_owned(),
            space_id: "space".to_owned(),
            space_label: "Space".to_owned(),
            playwright_session: "playwright".to_owned(),
            working_directory: "/tmp".to_owned(),
            invocation: "test".to_owned(),
            browser_instance: browser_incarnation.clone(),
            inline_provenance: None,
        };
        let pending = StoredPendingCapture {
            format_version: FORMAT_VERSION,
            association_key: association_key.to_owned(),
            browser_incarnation,
            draft_id,
            draft_revision: 1,
            annotation_ids: vec![annotation_id.clone()],
            original_annotation_digests: vec![annotation_digest(&original).unwrap()],
            context,
            submission: BrowserCaptureSubmission {
                association_key: association_key.to_owned(),
                browser_instance: current.identity.browser_incarnation.clone(),
                capture_id: Uuid::new_v4().to_string(),
                page: BrowserPageEvidence {
                    url: "https://example.test".to_owned(),
                    title: "Example".to_owned(),
                    tab_id: None,
                    document_id: "document".to_owned(),
                    captured_at: "now".to_owned(),
                    viewport: BrowserViewport {
                        width: 800.0,
                        height: 600.0,
                        scroll_x: 0.0,
                        scroll_y: 0.0,
                        device_pixel_ratio: 1.0,
                        visual_scale: 1.0,
                    },
                    image_width: 800,
                    image_height: 600,
                },
                annotations: vec![frozen],
                png_base64: "png".to_owned(),
            },
            last_error: None,
        };
        let saved = BrowserCaptureSaved {
            capture_id: pending.submission.capture_id.clone(),
            annotation_ids: pending.annotation_ids.clone(),
            image_path: "capture.png".to_owned(),
            pending_count: 0,
        };
        store.apply_capture_receipt(&pending, &saved).unwrap();
        let recovered = store.load_draft(&pending.draft_id).unwrap().unwrap();
        assert_eq!(recovered.annotations.len(), 1);
        assert_eq!(
            recovered.annotations[0].comment.as_deref(),
            Some("newer annotation")
        );
        assert_eq!(recovered.editor.note_text, "newer note");
        assert!(recovered.consumed_annotation_ids.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn capture_receipt_consumes_only_unchanged_document_marks_at_png_density() {
        let (store, root) = test_store();
        let key = "0123456789abcdef01234567";
        let incarnation = Uuid::new_v4().to_string();
        let owner = identity(key, &incarnation, "target", 1);
        let draft = store.open(&owner, None).unwrap();
        let original = BrowserViewDraftAnnotation {
            id: Uuid::new_v4().to_string(),
            kind: BrowserAnnotationKind::Region,
            color: "#f00".to_owned(),
            points: vec![BrowserPoint { x: 10.0, y: 15.0 }],
            bounds: Some(BrowserRect {
                x: 10.0,
                y: 15.0,
                width: 20.0,
                height: 5.0,
            }),
            evidence: None,
            comment: Some("pinned note".to_owned()),
        };
        let selected = store
            .upsert_annotation_recovery(key, &draft.draft_id, draft.revision, original.clone())
            .unwrap();
        let context = BrowserCaptureContext {
            association_key: key.to_owned(),
            session_id: "session".to_owned(),
            space_id: "space".to_owned(),
            space_label: "Space".to_owned(),
            playwright_session: "playwright".to_owned(),
            working_directory: "/tmp".to_owned(),
            invocation: "test".to_owned(),
            browser_instance: incarnation.clone(),
            inline_provenance: Some(BrowserInlineCaptureProvenance {
                target_id: owner.target_id.clone(),
                frame_id: "frame".to_owned(),
                document_generation: owner.document_generation,
                frame_generation: 1,
                stream_epoch: 1,
                frame_sequence: 1,
                viewport_revision: 1,
                pixel_captured_at_micros: 1,
                capture_as_shown: true,
            }),
        };
        let capture_id = Uuid::new_v4().to_string();
        store
            .prepare_capture(
                BrowserDraftCaptureContext {
                    identity: owner,
                    context,
                },
                &selected.draft_id,
                selected.revision,
                vec![original.id.clone()],
                capture_id.clone(),
            )
            .unwrap();
        let prepared = store.load_preparation(key).unwrap().unwrap();
        let later = BrowserViewDraftAnnotation {
            id: Uuid::new_v4().to_string(),
            kind: BrowserAnnotationKind::Region,
            color: "#0f0".to_owned(),
            points: vec![BrowserPoint { x: 50.0, y: 60.0 }],
            bounds: None,
            evidence: None,
            comment: Some("newer independent mark".to_owned()),
        };
        store
            .upsert_annotation_recovery(key, &selected.draft_id, selected.revision, later.clone())
            .unwrap();
        let pending = StoredPendingCapture {
            format_version: FORMAT_VERSION,
            association_key: key.to_owned(),
            browser_incarnation: incarnation.clone(),
            draft_id: selected.draft_id.clone(),
            draft_revision: selected.revision,
            annotation_ids: vec![original.id.clone()],
            original_annotation_digests: prepared.original_annotation_digests,
            context: prepared.context,
            submission: BrowserCaptureSubmission {
                association_key: key.to_owned(),
                browser_instance: incarnation,
                capture_id: capture_id.clone(),
                page: BrowserPageEvidence {
                    url: "https://example.test".to_owned(),
                    title: "Example".to_owned(),
                    tab_id: None,
                    document_id: "frame".to_owned(),
                    captured_at: "now".to_owned(),
                    viewport: BrowserViewport {
                        width: 100.0,
                        height: 100.0,
                        scroll_x: 0.0,
                        scroll_y: 0.0,
                        device_pixel_ratio: 2.0,
                        visual_scale: 1.0,
                    },
                    image_width: 200,
                    image_height: 200,
                },
                annotations: vec![BrowserAnnotation {
                    id: original.id.clone(),
                    kind: original.kind,
                    comment: "pinned note".to_owned(),
                    color: original.color,
                    points: vec![BrowserPoint { x: 20.0, y: 30.0 }],
                    bounds: Some(BrowserRect {
                        x: 20.0,
                        y: 30.0,
                        width: 40.0,
                        height: 10.0,
                    }),
                    element: None,
                }],
                png_base64: "png".to_owned(),
            },
            last_error: None,
        };
        store
            .apply_capture_receipt(
                &pending,
                &BrowserCaptureSaved {
                    capture_id,
                    annotation_ids: pending.annotation_ids.clone(),
                    image_path: "capture.png".to_owned(),
                    pending_count: 1,
                },
            )
            .unwrap();
        let recovered = store.load_draft(&selected.draft_id).unwrap().unwrap();
        assert_eq!(recovered.annotations, vec![later]);
        assert_eq!(recovered.consumed_annotation_ids, vec![original.id]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_revision_is_rejected_without_replacing_recoverable_draft() {
        let (store, root) = test_store();
        let owner = identity(
            "0123456789abcdef01234567",
            &Uuid::new_v4().to_string(),
            "target",
            1,
        );
        let original_id = Uuid::new_v4().to_string();
        let original = draft(&owner, original_id.clone());
        store.write_draft(&original).unwrap();

        let mut oversized = original.clone();
        oversized.revision = 2;
        for _ in 0..64 {
            oversized.annotations.push(BrowserViewDraftAnnotation {
                id: Uuid::new_v4().to_string(),
                kind: BrowserAnnotationKind::Freehand,
                color: "#123456".to_owned(),
                points: vec![BrowserPoint { x: 1.0, y: 2.0 }; 1_024],
                bounds: None,
                evidence: None,
                comment: Some("x".repeat(32 * 1024)),
            });
        }

        let error = store
            .write_draft(&oversized)
            .expect_err("oversized revision must not become unrecoverable");
        assert_eq!(error.code, "browser_draft_capacity");
        let recovered = store.load_draft(&original_id).unwrap().unwrap();
        assert_eq!(recovered.revision, original.revision);
        assert!(recovered.annotations.is_empty());

        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn oversized_pending_capture_does_not_replace_retryable_png() {
        let (store, root) = test_store();
        let association_key = "0123456789abcdef01234567";
        let browser_incarnation = Uuid::new_v4().to_string();
        let context = BrowserCaptureContext {
            association_key: association_key.to_owned(),
            session_id: "session".to_owned(),
            space_id: "space".to_owned(),
            space_label: "Space".to_owned(),
            playwright_session: "playwright".to_owned(),
            working_directory: "/tmp".to_owned(),
            invocation: "test".to_owned(),
            browser_instance: browser_incarnation.clone(),
            inline_provenance: None,
        };
        let submission = BrowserCaptureSubmission {
            association_key: association_key.to_owned(),
            browser_instance: browser_incarnation.clone(),
            capture_id: Uuid::new_v4().to_string(),
            page: BrowserPageEvidence {
                url: "https://example.test".to_owned(),
                title: "Example".to_owned(),
                tab_id: None,
                document_id: "document".to_owned(),
                captured_at: "now".to_owned(),
                viewport: BrowserViewport {
                    width: 800.0,
                    height: 600.0,
                    scroll_x: 0.0,
                    scroll_y: 0.0,
                    device_pixel_ratio: 1.0,
                    visual_scale: 1.0,
                },
                image_width: 800,
                image_height: 600,
            },
            annotations: Vec::new(),
            png_base64: "exact retry bytes".to_owned(),
        };
        let mut pending = StoredPendingCapture {
            format_version: FORMAT_VERSION,
            association_key: association_key.to_owned(),
            browser_incarnation,
            draft_id: Uuid::new_v4().to_string(),
            draft_revision: 1,
            annotation_ids: Vec::new(),
            original_annotation_digests: Vec::new(),
            context,
            submission,
            last_error: None,
        };
        let original_capture_id = pending.submission.capture_id.clone();
        store.write_pending(&pending).unwrap();
        pending.submission.capture_id = Uuid::new_v4().to_string();
        pending.submission.png_base64 = "x".repeat(MAX_PENDING_BYTES as usize);

        let error = store
            .write_pending(&pending)
            .expect_err("oversized pending record must be rejected");
        assert_eq!(error.code, "browser_draft_capacity");
        let retry = store.load_pending(association_key).unwrap().unwrap();
        assert_eq!(retry.submission.capture_id, original_capture_id);
        assert_eq!(retry.submission.png_base64, "exact retry bytes");

        std::fs::remove_dir_all(root).unwrap();
    }
}
