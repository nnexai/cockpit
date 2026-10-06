use super::*;
use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, MetadataExt};
use cockpit_protocol::browser::{BrowserCleanupRetryRequest, BrowserCleanupScope, BrowserCleanupStatus};
use std::io::{self, Write};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ArtifactIdentity { pub dev: u64, pub inode: u64 }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArtifactIdentities {
    pub profile: ArtifactIdentity,
    pub workspace: ArtifactIdentity,
    pub config: ArtifactIdentity,
}
fn identity(metadata: &Metadata) -> ArtifactIdentity {
    ArtifactIdentity { dev: metadata.dev(), inode: metadata.ino() }
}
fn io_error(error: io::Error) -> InspectionError {
    InspectionError::new("browser_cleanup_failed", error.to_string())
}
pub(super) fn valid_key(key: &str) -> bool {
    key.len() == 24 && key.bytes().all(|byte| byte.is_ascii_hexdigit())
}
pub(super) fn derived_paths(root: &Path, key: &str) -> [(PathBuf, bool); 3] {
    [(root.join("profiles").join(key), true), (root.join("workspaces").join(key), true), (root.join("configs").join(format!("{key}.json")), false)]
}

/// The handle that supplied the ownership identity is also the traversal root.
/// No caller can supply a path outside the three exact artifact parents.
pub(super) enum OpenArtifact { Directory(Dir), File(cap_std::fs::File) }
impl OpenArtifact {
    pub(super) fn identity(&self) -> io::Result<ArtifactIdentity> {
        match self { Self::Directory(dir) => dir.dir_metadata().map(|m| identity(&m)), Self::File(file) => file.metadata().map(|m| identity(&m)) }
    }
}
pub(super) fn open_artifact(parent: &Dir, name: &std::ffi::OsStr, directory: bool) -> io::Result<OpenArtifact> {
    let metadata = parent.symlink_metadata(Path::new(name))?;
    if metadata.file_type().is_symlink() || metadata.is_dir() != directory || (!directory && !metadata.is_file()) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "artifact is a symlink or has the wrong kind"));
    }
    if directory { return parent.open_dir_nofollow(Path::new(name)).map(OpenArtifact::Directory); }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(cap_fs_ext::FollowSymlinks::No).nonblock(true);
    let file = parent.open_with(Path::new(name), &options)?;
    if !file.metadata()?.is_file() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "artifact is not a regular file")); }
    Ok(OpenArtifact::File(file))
}
pub(super) fn verify_artifacts(root: &Path, receipt: &BrowserReceipt) -> Result<(), InspectionError> {
    let paths = derived_paths(root, &receipt.association_key);
    let identities = [&receipt.artifacts.profile, &receipt.artifacts.workspace, &receipt.artifacts.config];
    for (index, (path, directory)) in paths.iter().enumerate() {
        let verified = (|| -> io::Result<bool> {
            let parent = crate::project_store::open_dir_nofollow_absolute(path.parent().expect("derived parent"))?;
            Ok(open_artifact(&parent, path.file_name().expect("derived name"), *directory)?.identity()? == *identities[index])
        })();
        if !verified.unwrap_or(false) {
            return Err(InspectionError::new("browser_artifact_unproven", format!("Browser artifact no longer matches its creation proof: {}", path.display())));
        }
    }
    let workspace_parent = crate::project_store::open_dir_nofollow_absolute(paths[1].0.parent().expect("workspace parent")).map_err(|_| {
        InspectionError::new("browser_artifact_unproven", "Browser workspace no longer matches its creation proof")
    })?;
    let workspace = match open_artifact(&workspace_parent, paths[1].0.file_name().expect("workspace name"), true).map_err(|_| {
        InspectionError::new("browser_artifact_unproven", "Browser workspace no longer matches its creation proof")
    })? {
        OpenArtifact::Directory(directory) => directory,
        OpenArtifact::File(_) => unreachable!(),
    };
    for name in ["browser-user-agent.cjs", "browser-user-agent.json"] {
        open_artifact(&workspace, std::ffi::OsStr::new(name), false).map_err(|_| {
            InspectionError::new("browser_artifact_unproven", format!("Browser artifact is missing or unsafe: {}", paths[1].0.join(name).display()))
        })?;
    }
    Ok(())
}
fn same_entry(parent: &Dir, name: &std::ffi::OsStr, expected: &ArtifactIdentity, directory: bool) -> io::Result<()> {
    let metadata = parent.symlink_metadata(Path::new(name))?;
    if metadata.file_type().is_symlink() || metadata.is_dir() != directory || (!directory && !metadata.is_file()) || identity(&metadata) != *expected {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "artifact identity changed"));
    }
    Ok(())
}
pub(super) fn remove_opened(parent: &Dir, name: &std::ffi::OsStr, opened: &OpenArtifact, expected: &ArtifactIdentity) -> io::Result<()> {
    if opened.identity()? != *expected { return Err(io::Error::new(io::ErrorKind::InvalidInput, "artifact identity changed")); }
    match opened {
        OpenArtifact::Directory(dir) => {
            same_entry(parent, name, expected, true)?;
            crate::ephemeral::empty_dir_contents(dir, &[])?;
            same_entry(parent, name, expected, true)?;
            parent.remove_dir(Path::new(name))
        }
        OpenArtifact::File(_) => { same_entry(parent, name, expected, false)?; parent.remove_file(Path::new(name)) }
    }
}

pub(super) fn create_artifacts(root: &Path, key: &str, config: &Value) -> Result<(PathBuf, PathBuf, PathBuf, ArtifactIdentities), InspectionError> {
    let paths = derived_paths(root, key);
    let parents = paths.iter().map(|(path, _)| crate::project_store::open_dir_nofollow_absolute(path.parent().expect("derived parent"))).collect::<io::Result<Vec<_>>>().map_err(io_error)?;
    for (index, (path, _)) in paths.iter().enumerate() {
        match parents[index].symlink_metadata(path.file_name().expect("derived name")) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Ok(_) => return Err(InspectionError::new("browser_artifact_unproven", format!("Existing artifact lacks creation proof: {}", path.display()))),
            Err(error) => return Err(io_error(error)),
        }
    }
    let mut created: Vec<(usize, OpenArtifact, ArtifactIdentity)> = Vec::with_capacity(3);
    let result = (|| -> Result<(), InspectionError> {
        for (index, (path, directory)) in paths.iter().enumerate() {
            let parent = &parents[index];
            let name = path.file_name().expect("derived name");
            let opened = if *directory {
                parent.create_dir(Path::new(name)).map_err(io_error)?;
                OpenArtifact::Directory(parent.open_dir_nofollow(Path::new(name)).map_err(io_error)?)
            } else {
                let mut options = cap_std::fs::OpenOptions::new();
                options.write(true).create_new(true).follow(cap_fs_ext::FollowSymlinks::No);
                OpenArtifact::File(parent.open_with(Path::new(name), &options).map_err(io_error)?)
            };
            let id = opened.identity().map_err(io_error)?;
            created.push((index, opened, id));
            match &mut created.last_mut().expect("created artifact").1 {
                OpenArtifact::Directory(dir) if index == 1 => {
                    dir.create_dir(".playwright").map_err(io_error)?;
                    write_owned_file(dir, "browser-user-agent.cjs", super::BROWSER_USER_AGENT_HOOK.as_bytes())?;
                    let profile_path = paths[0].0.to_str().ok_or_else(|| InspectionError::new("invalid_browser_path", "browser profile path is not UTF-8"))?;
                    write_binding_file(dir, "browser-user-agent.json", profile_path)?;
                }
                OpenArtifact::File(file) => {
                    serde_json::to_writer(&mut *file, config).map_err(|e| InspectionError::new("browser_state_write", e.to_string()))?;
                    file.flush().and_then(|_| file.sync_all()).map_err(io_error)?;
                }
                _ => {},
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        // Roll back only handles created by this attempt; a concurrent replacement
        // fails identity revalidation and is left in place.
        for (index, opened, id) in &created {
            let _ = remove_opened(&parents[*index], paths[*index].0.file_name().expect("derived name"), opened, id);
        }
        return Err(error);
    }
    Ok((paths[1].0.clone(), paths[0].0.clone(), paths[2].0.clone(), ArtifactIdentities { profile: created[0].2.clone(), workspace: created[1].2.clone(), config: created[2].2.clone() }))
}

fn write_owned_file(directory: &Dir, name: &str, bytes: &[u8]) -> Result<(), InspectionError> {
    let mut options = cap_std::fs::OpenOptions::new();
    options.write(true).create_new(true).follow(cap_fs_ext::FollowSymlinks::No);
    let mut file = directory.open_with(Path::new(name), &options).map_err(io_error)?;
    file.write_all(bytes).and_then(|_| file.flush()).and_then(|_| file.sync_all()).map_err(io_error)
}

#[derive(serde::Serialize)]
struct UserAgentBinding<'a> {
    profile_path: &'a str,
}

fn write_binding_file(directory: &Dir, name: &str, profile_path: &str) -> Result<(), InspectionError> {
    let mut options = cap_std::fs::OpenOptions::new();
    options.write(true).create_new(true).follow(cap_fs_ext::FollowSymlinks::No);
    let mut file = directory.open_with(Path::new(name), &options).map_err(io_error)?;
    serde_json::to_writer(&mut file, &UserAgentBinding { profile_path })
        .map_err(|error| InspectionError::new("browser_state_write", error.to_string()))?;
    file.flush().and_then(|_| file.sync_all()).map_err(io_error)
}

impl BrowserService {
    pub(super) fn record_cleanup_failure(&self, receipt: &mut BrowserReceipt, reason: &str, unproven_paths: Vec<String>) -> Result<(), InspectionError> {
        receipt.state = ReceiptState::CleanupFailed;
        receipt.cleanup_reason = Some(reason.to_owned());
        receipt.unproven_paths = unproven_paths;
        self.store(receipt)
    }
    pub(super) async fn finish_cleanup(&self, receipt: &mut BrowserReceipt) -> Result<BrowserResponse, InspectionError> {
        receipt.state = ReceiptState::CleanupPending;
        receipt.intent = ReceiptIntent::None;
        receipt.incarnation = None;
        receipt.cdp_endpoint = None;
        receipt.cdp_browser_identity = None;
        self.store(receipt)?;
        let mut unproven = Vec::new();
        let mut failures = Vec::new();
        let recorded = [&receipt.profile_path, &receipt.working_directory, &receipt.config_path];
        let identities = [&receipt.artifacts.profile, &receipt.artifacts.workspace, &receipt.artifacts.config];
        for (index, (path, directory)) in derived_paths(&self.root, &receipt.association_key).iter().enumerate() {
            // Keep launch config available until directory removal succeeds so
            // an incomplete cleanup can still revalidate the stopped daemon.
            if index == 2 && (!unproven.is_empty() || !failures.is_empty()) { continue; }
            if Path::new(recorded[index]) != path { unproven.push(recorded[index].clone()); continue; }
            let attempt = || -> io::Result<()> {
                let parent = crate::project_store::open_dir_nofollow_absolute(path.parent().expect("derived parent"))?;
                let name = path.file_name().expect("derived name");
                let opened = match open_artifact(&parent, name, *directory) {
                    Ok(opened) => opened,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                    Err(error) => return Err(error),
                };
                if opened.identity()? != *identities[index] { return Err(io::Error::new(io::ErrorKind::InvalidInput, "creation identity no longer matches")); }
                remove_opened(&parent, name, &opened, identities[index])
            };
            let mut result = attempt();
            for _ in 0..4 {
                if result.is_ok() || result.as_ref().is_err_and(|error| matches!(error.kind(), io::ErrorKind::InvalidInput | io::ErrorKind::NotADirectory)) { break; }
                tokio::time::sleep(Duration::from_millis(200)).await;
                result = attempt();
            }
            if let Err(error) = result {
                if matches!(error.kind(), io::ErrorKind::InvalidInput | io::ErrorKind::NotADirectory) || fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
                    unproven.push(path.display().to_string());
                }
                failures.push(format!("{}: {error}", path.display()));
            }
        }
        if !unproven.is_empty() || !failures.is_empty() {
            let reason = if failures.is_empty() { "artifact paths lack ownership proof".into() } else { failures.join("; ") };
            self.record_cleanup_failure(receipt, &reason, unproven)?;
            return Ok(self.response(receipt, BrowserConnectionState::Closed, &reason));
        }
        let discard = self.feedback.discard_association(&receipt.association_key)
            .and_then(|_| self.draft_store()?.discard_association(&receipt.association_key));
        if let Err(error) = discard {
            let reason = format!("Cannot discard browser work: {}", error.message);
            self.record_cleanup_failure(receipt, &reason, Vec::new())?;
            return Ok(self.response(receipt, BrowserConnectionState::Closed, &reason));
        }
        let removal = crate::project_store::open_dir_nofollow_absolute(&self.root.join("tab-associations"))
            .and_then(|parent| parent.remove_file(format!("{}.json", receipt.association_key)));
        if let Err(error) = removal {
            let reason = format!("Cannot remove browser receipt: {error}");
            self.record_cleanup_failure(receipt, &reason, Vec::new())?;
            return Ok(self.response(receipt, BrowserConnectionState::Closed, &reason));
        }
        receipt.state = ReceiptState::Closed;
        receipt.cleanup_reason = None;
        receipt.unproven_paths.clear();
        let mut response = self.response(receipt, BrowserConnectionState::Closed, "browser stopped and managed artifacts removed");
        response.cleanup = BrowserCleanupState::Done;
        Ok(response)
    }
    pub async fn cleanup_status(&self) -> Result<BrowserCleanupStatus, InspectionError> {
        let receipts = self.load_all()?;
        let mut failures = self.cleanup_failures.lock().clone();
        for receipt in receipts {
            if let Some(reason) = receipt.cleanup_reason {
                failures.push(BrowserCleanupFailure { association_key: receipt.association_key, scope: BrowserCleanupScope::Tab { session_id: receipt.session_id, tab_id: receipt.tab_id }, reason, unproven_paths: receipt.unproven_paths });
            }
        }
        Ok(BrowserCleanupStatus { failures })
    }
    pub async fn retry_cleanup(&self, request: BrowserCleanupRetryRequest) -> Result<BrowserCleanupStatus, InspectionError> {
        {
            let _operation = self.operation_lock.lock().await;
            if let Some(mut receipt) = self.load(&request.association_key)? { self.close(&mut receipt).await?; }
        }
        self.cleanup_status().await
    }
}
