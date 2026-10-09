use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use super::process::{current_uid, launch_configuration};
use super::{BrowserService, ResolvedTarget, STATE_DIRS, cleanup};
use crate::InspectionError;
use cockpit_protocol::browser::BrowserCleanupFailure;

pub(super) const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
pub(super) const MAX_ASSOCIATIONS: usize = 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserReceipt {
    pub(super) association_key: String,
    pub(super) owner_id: String,
    pub(super) endpoint_identity: String,
    pub(super) endpoint_path: String,
    pub(super) session_id: String,
    pub(super) space_id: String,
    pub(super) space_label: String,
    pub(super) tab_id: String,
    pub(super) tab_label: String,
    pub(super) artifacts: cleanup::ArtifactIdentities,
    pub(super) cleanup_reason: Option<String>,
    pub(super) unproven_paths: Vec<String>,
    pub(super) playwright_session: String,
    pub(super) working_directory: String,
    pub(super) profile_path: String,
    pub(super) config_path: String,
    /// The loopback endpoint and browser identity captured after a verified
    /// inline launch. Both must still match before a helper can attach.
    pub(super) cdp_endpoint: Option<String>,
    pub(super) cdp_browser_identity: Option<String>,
    pub(super) intent: ReceiptIntent,
    pub(super) state: ReceiptState,
    /// Stable CDP target identity; absent during unresolved opens.
    pub(super) target_id: Option<String>,
    /// Historical Playwright CLI tab index, never used as attachment authority.
    pub(super) opened_tab: Option<String>,
    pub(super) incarnation: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReceiptState {
    PendingOpen,
    Open,
    Closing,
    Closed,
    OutcomeUnknown,
    Disconnected,
    CleanupPending,
    CleanupFailed,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReceiptIntent {
    None,
    Launch,
    LaunchNavigation,
    TabNew,
    Close,
}

impl BrowserService {
    pub(crate) fn prepare_state_dirs(&self) -> Result<(), InspectionError> {
        for name in STATE_DIRS {
            prepare_root(&self.root.join(name))?;
        }
        Ok(())
    }

    pub(super) fn load_or_create(
        &self,
        target: &ResolvedTarget,
    ) -> Result<BrowserReceipt, InspectionError> {
        let key = association_key(
            &target.endpoint_identity,
            &target.session_id,
            &target.tab_id,
        );
        if let Some(receipt) = self.load(&key)? {
            if receipt.endpoint_identity == target.endpoint_identity
                && receipt.session_id == target.session_id
                && receipt.tab_id == target.tab_id
            {
                return Ok(receipt);
            }
            return Err(InspectionError::new(
                "association_identity_conflict",
                "browser receipt identity is inconsistent",
            ));
        }
        let paths = cleanup::derived_paths(&self.root, &key);
        let hook_path = paths[1].0.join("browser-user-agent.cjs");
        let (working_directory, profile_path, config_path, artifacts) = cleanup::create_artifacts(
            &self.root,
            &key,
            &launch_configuration(&self.configuration, &hook_path)?,
        )?;
        let receipt = BrowserReceipt {
            association_key: key.clone(),
            owner_id: self.owner_id.clone(),
            endpoint_identity: target.endpoint_identity.clone(),
            endpoint_path: target.endpoint_path.clone(),
            session_id: target.session_id.clone(),
            space_id: target.space_id.clone(),
            space_label: target.space_label.clone(),
            tab_id: target.tab_id.clone(),
            tab_label: target.tab_label.clone(),
            artifacts,
            cleanup_reason: None,
            unproven_paths: Vec::new(),
            playwright_session: format!("cockpit-{key}"),
            working_directory: path_string(&working_directory)?,
            profile_path: path_string(&profile_path)?,
            config_path: path_string(&config_path)?,
            cdp_endpoint: None,
            cdp_browser_identity: None,
            intent: ReceiptIntent::None,
            state: ReceiptState::Closed,
            target_id: None,
            opened_tab: None,
            incarnation: None,
        };
        self.store(&receipt)?;
        Ok(receipt)
    }

    pub(super) fn tab_association_path(&self, key: &str) -> PathBuf {
        self.root
            .join("tab-associations")
            .join(format!("{key}.json"))
    }
    pub(super) fn store(&self, receipt: &BrowserReceipt) -> Result<(), InspectionError> {
        atomic_write_json(
            &self.tab_association_path(&receipt.association_key),
            receipt,
        )
    }
    pub(super) fn load(&self, key: &str) -> Result<Option<BrowserReceipt>, InspectionError> {
        if key.len() != 24 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(InspectionError::new(
                "browser_state_corrupt",
                "Invalid browser association key",
            ));
        }
        let Some(receipt) = read_json::<BrowserReceipt>(&self.tab_association_path(key))? else {
            return Ok(None);
        };
        if receipt.association_key != key
            || association_key(
                &receipt.endpoint_identity,
                &receipt.session_id,
                &receipt.tab_id,
            ) != key
            || receipt.playwright_session != format!("cockpit-{key}")
            || Path::new(&receipt.working_directory) != self.root.join("workspaces").join(key)
            || Path::new(&receipt.profile_path) != self.root.join("profiles").join(key)
            || Path::new(&receipt.config_path)
                != self.root.join("configs").join(format!("{key}.json"))
        {
            return Err(InspectionError::new(
                "browser_state_corrupt",
                "Browser receipt does not match its association",
            ));
        }
        Ok(Some(receipt))
    }
    pub(super) fn load_all(&self) -> Result<Vec<BrowserReceipt>, InspectionError> {
        let directory = self.root.join("tab-associations");
        let mut values = Vec::new();
        for (index, entry) in fs::read_dir(&directory)
            .map_err(|_| {
                InspectionError::new(
                    "browser_state_unavailable",
                    "cannot list browser associations",
                )
            })?
            .enumerate()
        {
            if index >= MAX_ASSOCIATIONS {
                return Err(InspectionError::new(
                    "browser_state_limit",
                    "too many browser associations",
                ));
            }
            let path = entry
                .map_err(|_| {
                    InspectionError::new(
                        "browser_state_unavailable",
                        "cannot inspect browser association",
                    )
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                let key = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| {
                        InspectionError::new(
                            "browser_state_corrupt",
                            "Invalid browser receipt filename",
                        )
                    })?;
                match self.load(key) {
                    Ok(Some(value)) => {
                        self.cleanup_failures.lock().retain(|failure| {
                            !(failure.association_key == key
                                && matches!(
                                    failure.scope,
                                    cockpit_protocol::browser::BrowserCleanupScope::Tab { .. }
                                ))
                        });
                        values.push(value);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let raw = read_json::<BrowserReceipt>(&path).ok().flatten();
                        let mut unproven_paths = vec![path.display().to_string()];
                        if let Some(receipt) = &raw {
                            unproven_paths.extend([
                                receipt.profile_path.clone(),
                                receipt.working_directory.clone(),
                                receipt.config_path.clone(),
                            ]);
                        }
                        let mut failures = self.cleanup_failures.lock();
                        failures.retain(|failure| failure.association_key != key);
                        failures.push(BrowserCleanupFailure {
                            association_key: key.to_owned(),
                            scope: cockpit_protocol::browser::BrowserCleanupScope::Tab {
                                session_id: raw
                                    .as_ref()
                                    .map(|r| r.session_id.clone())
                                    .unwrap_or_default(),
                                tab_id: raw.as_ref().map(|r| r.tab_id.clone()).unwrap_or_default(),
                            },
                            reason: error.message,
                            unproven_paths,
                        });
                    }
                }
            }
        }
        Ok(values)
    }
}

pub(super) fn association_key(endpoint_identity: &str, session_id: &str, tab_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"tab\0");
    hasher.update(endpoint_identity.as_bytes());
    hasher.update([0]);
    hasher.update(session_id.as_bytes());
    hasher.update([0]);
    hasher.update(tab_id.as_bytes());
    format!("{:x}", hasher.finalize())[..24].to_owned()
}

pub(super) fn path_string(path: &Path) -> Result<String, InspectionError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| InspectionError::new("invalid_browser_path", "browser path is not UTF-8"))
}
pub(super) fn prepare_root(path: &Path) -> Result<(), InspectionError> {
    let (_, directory) = crate::project_store::prepare_root(path, "browser")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let directory = directory
            .open(".")
            .map_err(|_| {
                InspectionError::new(
                    "browser_state_unavailable",
                    "Cannot open browser state directory",
                )
            })?
            .into_std();
        if directory
            .metadata()
            .map_err(|_| {
                InspectionError::new(
                    "browser_state_unavailable",
                    "Cannot inspect browser state owner",
                )
            })?
            .uid()
            != current_uid()
        {
            return Err(InspectionError::new(
                "unsafe_path",
                "Browser state belongs to another user",
            ));
        }
        directory
            .set_permissions(fs::Permissions::from_mode(0o700))
            .map_err(|_| {
                InspectionError::new(
                    "browser_state_unavailable",
                    "Cannot secure browser state directory",
                )
            })?;
    }
    Ok(())
}
fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), InspectionError> {
    let parent = path
        .parent()
        .ok_or_else(|| InspectionError::new("unsafe_path", "Browser state has no parent"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| InspectionError::new("unsafe_path", "Invalid browser state filename"))?;
    let directory = crate::project_store::open_dir_nofollow_absolute(parent)
        .map_err(|_| InspectionError::new("unsafe_path", "Browser state directory is unsafe"))?;
    crate::project_store::atomic_write_json(&directory, name, value)
        .map_err(|_| InspectionError::new("browser_state_write", "Cannot publish browser state"))?;
    directory
        .open(name)
        .and_then(|file| file.sync_all())
        .and_then(|_| directory.open(".").and_then(|file| file.sync_all()))
        .map_err(|_| InspectionError::new("browser_state_write", "Cannot sync browser state"))?;
    Ok(())
}
pub(super) fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, InspectionError> {
    use cap_fs_ext::{OpenOptionsFollowExt, OpenOptionsSyncExt};
    let parent = path
        .parent()
        .ok_or_else(|| InspectionError::new("unsafe_path", "Browser state has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| InspectionError::new("unsafe_path", "Invalid browser state filename"))?;
    let directory = crate::project_store::open_dir_nofollow_absolute(parent)
        .map_err(|_| InspectionError::new("unsafe_path", "Browser state directory is unsafe"))?;
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let file = directory.open_with(name, &options).map_err(|_| {
        InspectionError::new("browser_state_read", "Cannot open browser state record")
    })?;
    let metadata = file.metadata().map_err(|_| {
        InspectionError::new("browser_state_read", "Cannot inspect browser state record")
    })?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(InspectionError::new(
            "unsafe_path",
            "Browser state record is not a bounded regular file",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|_| {
        InspectionError::new("browser_state_read", "cannot read browser state record")
    })?;
    if bytes.len() as u64 > limit {
        return Err(InspectionError::new(
            "browser_state_limit",
            "browser state record is too large",
        ));
    }
    Ok(bytes)
}
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, InspectionError> {
    if fs::symlink_metadata(path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) {
        return Ok(None);
    }
    serde_json::from_slice(&read_regular(path, MAX_RECEIPT_BYTES)?)
        .map(Some)
        .map_err(|_| {
            InspectionError::new("browser_state_corrupt", "browser association is invalid")
        })
}
