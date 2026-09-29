use super::*;
use cockpit_protocol::browser::BrowserSavedTabWork;

/// Immutable source identity for owner-persisted work, independent of a live
/// browser receipt or the current Herdr generation.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserSavedTabAssociation {
    pub(crate) association_key: String,
    pub(crate) endpoint_identity: String,
    pub(crate) endpoint_path: String,
    pub(crate) session_id: String,
    pub(crate) tab_id: String,
    pub(crate) tab_label: String,
    pub(crate) space_id: String,
    pub(crate) space_label: String,
}

impl BrowserService {
    fn saved_tab_path(&self, key: &str) -> PathBuf {
        self.root.join("saved-tab-associations").join(format!("{key}.json"))
    }

    pub(crate) fn load_saved_tab(&self, key: &str) -> Result<Option<BrowserSavedTabAssociation>, InspectionError> {
        if !cleanup::valid_key(key) {
            return Err(InspectionError::new("browser_state_corrupt", "Invalid saved tab association key"));
        }
        let Some(saved) = read_json::<BrowserSavedTabAssociation>(&self.saved_tab_path(key))? else {
            return Ok(None);
        };
        if saved.association_key != key
            || association_key(&saved.endpoint_identity, &saved.session_id, &saved.tab_id) != key
            || saved.endpoint_identity.is_empty()
            || saved.endpoint_path.is_empty()
            || saved.space_id.is_empty()
        {
            return Err(InspectionError::new("browser_state_corrupt", "Saved tab provenance does not match its association"));
        }
        validate_id(&saved.session_id, "saved session")?;
        validate_id(&saved.tab_id, "saved tab")?;
        validate_id(&saved.space_id, "saved Space")?;
        Ok(Some(saved))
    }

    pub(super) fn preserve_saved_tab(&self, receipt: &BrowserReceipt) -> Result<(), InspectionError> {
        let key = &receipt.association_key;
        if !cleanup::valid_key(key)
            || association_key(&receipt.endpoint_identity, &receipt.session_id, &receipt.tab_id) != *key
        {
            return Err(InspectionError::new("browser_state_corrupt", "Browser receipt cannot authorize saved tab provenance"));
        }
        if let Some(saved) = self.load_saved_tab(key)? {
            if saved.endpoint_identity != receipt.endpoint_identity
                || saved.endpoint_path != receipt.endpoint_path
                || saved.session_id != receipt.session_id
                || saved.tab_id != receipt.tab_id
            {
                return Err(InspectionError::new("browser_state_corrupt", "Saved tab provenance was replaced"));
            }
            // Retain the original labels and Space even after a rename or move.
            return Ok(());
        }
        let saved = BrowserSavedTabAssociation {
            association_key: key.clone(),
            endpoint_identity: receipt.endpoint_identity.clone(),
            endpoint_path: receipt.endpoint_path.clone(),
            session_id: receipt.session_id.clone(),
            tab_id: receipt.tab_id.clone(),
            tab_label: receipt.tab_label.clone(),
            space_id: receipt.space_id.clone(),
            space_label: receipt.space_label.clone(),
        };
        atomic_write_json(&self.saved_tab_path(key), &saved)
    }

    /// Enumerate retained work only. Empty provenance is kept so later receipt
    /// cleanup cannot change source identity, but is not shown to the user.
    pub fn saved_tab_work(&self) -> Result<Vec<BrowserSavedTabWork>, InspectionError> {
        let directory = crate::project_store::open_dir_nofollow_absolute(&self.root.join("saved-tab-associations"))
            .map_err(|error| InspectionError::new("browser_state_unavailable", error.to_string()))?;
        let drafts = self.draft_store()?;
        let mut work = Vec::new();
        for entry in directory.entries().map_err(|error| InspectionError::new("browser_state_unavailable", error.to_string()))? {
            let entry = entry.map_err(|error| InspectionError::new("browser_state_unavailable", error.to_string()))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return Err(InspectionError::new("browser_state_corrupt", "Invalid saved tab provenance filename"));
            };
            let Some(key) = name.strip_suffix(".json") else { continue; };
            let Some(saved) = self.load_saved_tab(key)? else { continue; };
            let feedback = self.feedback.list(key)?;
            let inventory = drafts.list(key)?;
            if feedback.captures.is_empty() && inventory.drafts.is_empty() && inventory.pending_capture.is_none() {
                continue;
            }
            work.push(BrowserSavedTabWork {
                association_key: saved.association_key,
                session_id: saved.session_id,
                tab_id: saved.tab_id,
                tab_label: saved.tab_label,
                space_id: saved.space_id,
                space_label: saved.space_label,
                saved_capture_count: feedback.captures.len().min(u32::MAX as usize) as u32,
                draft_count: inventory.drafts.len().min(u32::MAX as usize) as u32,
                pending_capture: inventory.pending_capture.is_some(),
            });
        }
        work.sort_by(|left, right| left.association_key.cmp(&right.association_key));
        Ok(work)
    }
}
