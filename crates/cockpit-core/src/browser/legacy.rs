use super::*;
use super::cleanup::{self, ArtifactIdentity};
use cap_fs_ext::OpenOptionsFollowExt;
use cockpit_protocol::browser::{BrowserCleanupScope, BrowserLegacyArchive, BrowserLegacyArchiveList, BrowserLegacyArtifactCandidate, BrowserLegacyCandidateKind, BrowserLegacyCandidateState, BrowserLegacyKeepRequest, BrowserLegacyRemovalRequest};
use std::collections::BTreeMap;
use std::io::{self, Write};

/// Deliberately explicit old schema: never deserialize a legacy receipt as a tab receipt.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacySpaceReceipt {
    association_key: String, owner_id: String, endpoint_identity: String, endpoint_path: String,
    session_id: String, space_id: String, space_label: String, playwright_session: String,
    working_directory: String, profile_path: String, config_path: String,
    #[serde(default)] cdp_endpoint: Option<String>,
    #[serde(default)] cdp_browser_identity: Option<String>,
    intent: ReceiptIntent, state: ReceiptState,
    #[serde(default)] target_id: Option<String>, opened_tab: Option<String>, incarnation: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyArchiveRecord {
    association_key: String, endpoint_identity: String, endpoint_path: String,
    session_id: String, space_id: String, space_label: String, archived_at: String,
    session_stopped: bool, candidates: Vec<BrowserLegacyArtifactCandidate>, not_candidates: Vec<String>,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyDecisions { states: BTreeMap<String, BrowserLegacyCandidateState> }
fn io_error(error: io::Error) -> InspectionError { InspectionError::new("browser_legacy_cleanup_failed", error.to_string()) }

impl BrowserService {
    pub(super) fn load_legacy_archive(&self, key: &str) -> Result<Option<LegacyArchiveRecord>, InspectionError> {
        if !cleanup::valid_key(key) { return Err(InspectionError::new("browser_legacy_archive_invalid", "Invalid legacy archive key")); }
        let record = read_json::<LegacyArchiveRecord>(&self.root.join("legacy-archive").join(format!("{key}.json")))?;
        if record.as_ref().is_some_and(|record| record.association_key != key || !record.session_stopped) {
            return Err(InspectionError::new("browser_legacy_archive_invalid", "Legacy archive filename and provenance do not match"));
        }
        Ok(record)
    }
    fn legacy_decisions(&self, key: &str) -> Result<LegacyDecisions, InspectionError> {
        Ok(read_json(&self.root.join("legacy-decisions").join(format!("{key}.json")))?.unwrap_or_default())
    }
    fn store_legacy_decisions(&self, key: &str, decisions: &LegacyDecisions) -> Result<(), InspectionError> {
        atomic_write_json(&self.root.join("legacy-decisions").join(format!("{key}.json")), decisions)
    }
    pub async fn retire_legacy_space_associations(&self) -> Result<(), InspectionError> {
        if self.legacy_started.swap(true, Ordering::AcqRel) { return Ok(()); }
        {
            let mut state = self.cutover.lock().await;
            if *state != BrowserCutoverState::NotNeeded { return Ok(()); }
            *state = BrowserCutoverState::Running;
        }
        let _operation = self.operation_lock.lock().await;
        let result = self.retire_legacy_entries().await;
        *self.cutover.lock().await = match &result { Ok(false) => BrowserCutoverState::NotNeeded, Ok(true) => BrowserCutoverState::Done, Err(_) => BrowserCutoverState::Failed };
        result.map(|_| ())
    }
    async fn retire_legacy_entries(&self) -> Result<bool, InspectionError> {
        let directory = self.root.join("associations");
        let entries = match crate::project_store::open_dir_nofollow_absolute(&directory) {
            Ok(dir) => dir,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(io_error(error)),
        };
        let mut needed = false;
        let mut first_error = None;
        for (index, entry) in entries.entries().map_err(io_error)?.enumerate() {
            if index >= MAX_ASSOCIATIONS { return Err(InspectionError::new("browser_state_limit", "too many legacy browser receipts")); }
            let path = directory.join(entry.map_err(io_error)?.file_name());
            if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
            needed = true;
            let key = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if let Err(error) = self.retire_legacy_entry(key).await {
                if !self.cleanup_failures.lock().iter().any(|failure| failure.association_key == key) {
                    self.legacy_failure(key, "unknown", &error.message, vec![path.display().to_string()]);
                }
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(needed), Err)
    }
    fn legacy_failure(&self, key: &str, space_id: &str, reason: &str, paths: Vec<String>) {
        let mut failures = self.cleanup_failures.lock();
        failures.retain(|failure| failure.association_key != key);
        failures.push(BrowserCleanupFailure { association_key: key.to_owned(), scope: BrowserCleanupScope::LegacySpace { space_id: space_id.to_owned() }, reason: reason.to_owned(), unproven_paths: paths });
    }
    async fn retire_legacy_entry(&self, key: &str) -> Result<(), InspectionError> {
        if !cleanup::valid_key(key) { return Err(InspectionError::new("browser_legacy_receipt_invalid", "Legacy receipt filename is not a Cockpit key")); }
        let path = self.root.join("associations").join(format!("{key}.json"));
        let Some(old) = read_json::<LegacySpaceReceipt>(&path)? else { return Ok(()); };
        if old.association_key != key { return Err(InspectionError::new("browser_legacy_receipt_invalid", "Legacy receipt stem and key differ; preserved")); }
        let paths = cleanup::derived_paths(&self.root, key);
        // Stop names and cwd are derived, never trusted from the old receipt.
        let mut receipt = BrowserReceipt {
            association_key: key.to_owned(), owner_id: old.owner_id.clone(), endpoint_identity: old.endpoint_identity.clone(), endpoint_path: old.endpoint_path.clone(),
            session_id: old.session_id.clone(), space_id: old.space_id.clone(), space_label: old.space_label.clone(),
            tab_id: String::new(), tab_label: String::new(),
            artifacts: cleanup::ArtifactIdentities { profile: ArtifactIdentity { dev: 0, inode: 0 }, workspace: ArtifactIdentity { dev: 0, inode: 0 }, config: ArtifactIdentity { dev: 0, inode: 0 } },
            cleanup_reason: None, unproven_paths: Vec::new(), playwright_session: format!("cockpit-{key}"),
            working_directory: path_string(&paths[1].0)?, profile_path: path_string(&paths[0].0)?, config_path: path_string(&paths[2].0)?,
            cdp_endpoint: old.cdp_endpoint.clone(), cdp_browser_identity: old.cdp_browser_identity.clone(), intent: old.intent, state: old.state,
            target_id: old.target_id.clone(), opened_tab: old.opened_tab.clone(), incarnation: old.incarnation.clone(),
        };
        self.stop_legacy(&mut receipt).await.map_err(|error| {
            self.legacy_failure(key, &old.space_id, &error.message, Vec::new()); error
        })?;
        if self.load_legacy_archive(key)?.is_none() {
            let captured_at = crate::project_store::timestamp();
            let mut candidates = Vec::new();
            let mut not_candidates = Vec::new();
            for (artifact_path, directory) in paths {
                let result = (|| -> io::Result<BrowserLegacyArtifactCandidate> {
                    let parent = crate::project_store::open_dir_nofollow_absolute(artifact_path.parent().expect("derived parent"))?;
                    let opened = cleanup::open_artifact(&parent, artifact_path.file_name().expect("derived name"), directory)?;
                    let id = opened.identity()?;
                    let (entry_count, total_bytes) = opened.stats()?;
                    Ok(BrowserLegacyArtifactCandidate { path: artifact_path.display().to_string(), kind: if directory { BrowserLegacyCandidateKind::Directory } else { BrowserLegacyCandidateKind::File }, dev: id.dev.to_string(), inode: id.inode.to_string(), entry_count, total_bytes, captured_at: captured_at.clone(), state: BrowserLegacyCandidateState::Pending })
                })();
                match result {
                    Ok(candidate) => candidates.push(candidate),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                    Err(_) => not_candidates.push(artifact_path.display().to_string()),
                }
            }
            let record = LegacyArchiveRecord { association_key: key.to_owned(), endpoint_identity: old.endpoint_identity, endpoint_path: old.endpoint_path, session_id: old.session_id, space_id: old.space_id, space_label: old.space_label, archived_at: captured_at, session_stopped: true, candidates, not_candidates };
            self.publish_legacy_archive(&record)?;
        }
        // A prior publish may have linked successfully but failed its directory
        // sync. Retrying must persist that entry too before retiring the source.
        crate::project_store::open_dir_nofollow_absolute(&self.root.join("legacy-archive"))
            .and_then(|directory| directory.open("."))
            .and_then(|file| file.sync_all()).map_err(io_error)?;
        let directory = crate::project_store::open_dir_nofollow_absolute(&self.root.join("associations")).map_err(io_error)?;
        directory.remove_file(format!("{key}.json")).map_err(io_error)?;
        directory.open(".").and_then(|file| file.sync_all()).map_err(io_error)?;
        self.cleanup_failures.lock().retain(|failure| failure.association_key != key);
        Ok(())
    }
    fn publish_legacy_archive(&self, record: &LegacyArchiveRecord) -> Result<(), InspectionError> {
        let directory = crate::project_store::open_dir_nofollow_absolute(&self.root.join("legacy-archive")).map_err(io_error)?;
        let stage = format!(".archive-{}", Uuid::new_v4());
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true).follow(cap_fs_ext::FollowSymlinks::No);
        let mut file = directory.open_with(&stage, &options).map_err(io_error)?;
        let result = (|| {
            serde_json::to_writer(&mut file, record).map_err(|e| InspectionError::new("browser_state_write", e.to_string()))?;
            file.flush().and_then(|_| file.sync_all()).map_err(io_error)?;
            // A hard-link publishes atomically and cannot replace an existing archive.
            directory.hard_link(&stage, &directory, format!("{}.json", record.association_key)).map_err(io_error)?;
            // Persist the replacement provenance before the source receipt
            // can be removed by the caller.
            directory.open(".").and_then(|file| file.sync_all()).map_err(io_error)
        })();
        let _ = directory.remove_file(stage);
        result
    }
    async fn stop_legacy(&self, receipt: &mut BrowserReceipt) -> Result<(), InspectionError> {
        if self.inspect_live(receipt).await.as_ref().is_err_and(may_launch_after_inspection_failure) { return Ok(()); }
        // The verified legacy key authorizes stopping this Cockpit-named session,
        // even when obsolete daemon metadata no longer authorizes attachment.
        let _cwd = crate::project_store::open_dir_nofollow_absolute(Path::new(&receipt.working_directory)).map_err(io_error)?;
        let result = self.run_cli(receipt, &[format!("-s={}", receipt.playwright_session), "close".into()]).await;
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if self.inspect_live(receipt).await.as_ref().is_err_and(may_launch_after_inspection_failure) { return Ok(()); }
        }
        Err(result.err().unwrap_or_else(|| InspectionError::new("browser_legacy_stop_unconfirmed", "Legacy browser stop was not confirmed; artifacts preserved")))
    }
    pub(super) async fn retry_legacy_cleanup(&self, key: &str) -> Result<(), InspectionError> {
        self.retire_legacy_entry(key).await?;
        let remaining = !self.cleanup_failures.lock().is_empty();
        *self.cutover.lock().await = if remaining { BrowserCutoverState::Failed } else { BrowserCutoverState::Done };
        Ok(())
    }
    pub async fn legacy_list(&self) -> Result<BrowserLegacyArchiveList, InspectionError> {
        let directory = crate::project_store::open_dir_nofollow_absolute(&self.root.join("legacy-archive")).map_err(io_error)?;
        let mut archives = Vec::new();
        for (index, entry) in directory.entries().map_err(io_error)?.enumerate() {
            if index >= MAX_ASSOCIATIONS { return Err(InspectionError::new("browser_state_limit", "too many legacy archives")); }
            let name = entry.map_err(io_error)?.file_name();
            let path = Path::new(&name);
            if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
            let key = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let Some(record) = self.load_legacy_archive(key)? else { continue; };
            let decisions = self.legacy_decisions(key)?;
            let feedback = self.feedback.list(key)?;
            let drafts = self.draft_store()?.list(key)?;
            let mut candidates = record.candidates;
            for candidate in &mut candidates { if let Some(state) = decisions.states.get(&candidate.path) { candidate.state = *state; } }
            archives.push(BrowserLegacyArchive { association_key: record.association_key, session_id: record.session_id, space_id: record.space_id, space_label: record.space_label, archived_at: record.archived_at, session_stopped: record.session_stopped, saved_capture_count: feedback.captures.len().min(u32::MAX as usize) as u32, draft_count: drafts.drafts.len().min(u32::MAX as usize) as u32, pending_capture: drafts.pending_capture.is_some(), candidates, not_candidates: record.not_candidates });
        }
        archives.sort_by(|a, b| a.association_key.cmp(&b.association_key));
        Ok(BrowserLegacyArchiveList { archives })
    }
    pub async fn legacy_remove(&self, request: BrowserLegacyRemovalRequest) -> Result<BrowserLegacyArchiveList, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let record = self.load_legacy_archive(&request.association_key)?.ok_or_else(|| InspectionError::new("browser_legacy_archive_absent", "Legacy archive is absent"))?;
        let mut decisions = self.legacy_decisions(&request.association_key)?;
        if request.candidates.is_empty() { return Err(InspectionError::new("browser_legacy_manifest_changed", "Removal requires the reviewed pending items")); }
        // Validate the complete echoed review before deleting any object.
        let mut seen = std::collections::BTreeSet::new();
        for reviewed in &request.candidates {
            let Some(original) = record.candidates.iter().find(|c| c.path == reviewed.path) else { return Err(InspectionError::new("browser_legacy_manifest_changed", "Removal contains an unreviewed artifact")); };
            let state = decisions.states.get(&original.path).copied().unwrap_or(original.state);
            if !seen.insert(&reviewed.path) || reviewed.dev != original.dev || reviewed.inode != original.inode || reviewed.kind != original.kind || reviewed.entry_count != original.entry_count || reviewed.total_bytes != original.total_bytes || reviewed.captured_at != original.captured_at || reviewed.state != state || state != BrowserLegacyCandidateState::Pending {
                return Err(InspectionError::new("browser_legacy_manifest_changed", "Removal no longer matches the reviewed manifest"));
            }
        }
        let mut removal_failed = false;
        let derived = cleanup::derived_paths(&self.root, &request.association_key);
        for reviewed in &request.candidates {
            let allowed = derived.iter().find(|(path, dir)| path.to_str() == Some(reviewed.path.as_str()) && *dir == (reviewed.kind == BrowserLegacyCandidateKind::Directory));
            let result = (|| -> io::Result<()> {
                let (path, directory) = allowed.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "manifest path is not derived"))?;
                let parent = crate::project_store::open_dir_nofollow_absolute(path.parent().expect("derived parent"))?;
                let name = path.file_name().expect("derived name");
                let opened = cleanup::open_artifact(&parent, name, *directory)?;
                let expected = ArtifactIdentity { dev: reviewed.dev.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid dev"))?, inode: reviewed.inode.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid inode"))? };
                cleanup::remove_opened(&parent, name, &opened, &expected)
            })();
            let state = match result {
                Ok(()) => BrowserLegacyCandidateState::Removed,
                Err(error) if matches!(error.kind(), io::ErrorKind::InvalidInput | io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) || fs::symlink_metadata(&reviewed.path).is_ok_and(|m| m.file_type().is_symlink()) => BrowserLegacyCandidateState::Changed,
                Err(error) => { removal_failed = true; self.legacy_failure(&request.association_key, &record.space_id, &error.to_string(), Vec::new()); continue; }
            };
            decisions.states.insert(reviewed.path.clone(), state);
            self.store_legacy_decisions(&request.association_key, &decisions)?;
        }
        if !removal_failed { self.cleanup_failures.lock().retain(|failure| failure.association_key != request.association_key); }
        self.prune_legacy_archive(&request.association_key)?;
        self.legacy_list().await
    }
    pub async fn legacy_keep(&self, request: BrowserLegacyKeepRequest) -> Result<BrowserLegacyArchiveList, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let record = self.load_legacy_archive(&request.association_key)?.ok_or_else(|| InspectionError::new("browser_legacy_archive_absent", "Legacy archive is absent"))?;
        let mut decisions = self.legacy_decisions(&request.association_key)?;
        for candidate in record.candidates {
            if decisions.states.get(&candidate.path) != Some(&BrowserLegacyCandidateState::Removed) { decisions.states.insert(candidate.path, BrowserLegacyCandidateState::Kept); }
        }
        self.store_legacy_decisions(&request.association_key, &decisions)?;
        self.cleanup_failures.lock().retain(|failure| failure.association_key != request.association_key);
        self.prune_legacy_archive(&request.association_key)?;
        self.legacy_list().await
    }
    pub(super) fn prune_legacy_archive(&self, key: &str) -> Result<(), InspectionError> {
        let Some(record) = self.load_legacy_archive(key)? else { return Ok(()); };
        let decisions = self.legacy_decisions(key)?;
        let unresolved = !record.not_candidates.is_empty() || record.candidates.iter().any(|c| !matches!(decisions.states.get(&c.path).copied().unwrap_or(c.state), BrowserLegacyCandidateState::Kept | BrowserLegacyCandidateState::Removed));
        let drafts = self.draft_store()?.list(key)?;
        if unresolved || !drafts.drafts.is_empty() || drafts.pending_capture.is_some() || self.feedback.list(key)?.pending_count > 0 { return Ok(()); }
        let directory = crate::project_store::open_dir_nofollow_absolute(&self.root.join("legacy-archive")).map_err(io_error)?;
        directory.remove_file(format!("{key}.json")).map_err(io_error)?;
        let decisions = crate::project_store::open_dir_nofollow_absolute(&self.root.join("legacy-decisions")).map_err(io_error)?;
        match decisions.remove_file(format!("{key}.json")) { Ok(()) => {}, Err(error) if error.kind() == io::ErrorKind::NotFound => {}, Err(error) => return Err(io_error(error)) }
        Ok(())
    }
}
