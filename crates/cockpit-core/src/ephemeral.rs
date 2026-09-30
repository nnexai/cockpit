use crate::{InspectionError, browser::BrowserService};
use cap_fs_ext::DirExt;
use cap_std::fs::{Dir, MetadataExt};
use std::{io, path::Path};

/// Called only by the browser runtime holding owner.lock, before publishing IPC.
pub async fn reset_owner_state(state_root: &Path, browser: &BrowserService) -> Result<(), InspectionError> {
    let error = |path: &Path, error: io::Error| {
        InspectionError::new("ephemeral_reset_failed", format!("{}: {error}", path.display()))
    };
    let root = crate::project_store::open_dir_nofollow_absolute(state_root)
        .map_err(|cause| error(state_root, cause))?;
    let device = root.dir_metadata().map_err(|cause| error(state_root, cause))?.dev();
    for name in ["browser", "comments", "review"] {
        let path = state_root.join(name);
        let metadata = match root.symlink_metadata(name) {
            Ok(metadata) => metadata,
            Err(cause) if cause.kind() == io::ErrorKind::NotFound && name != "browser" => continue,
            Err(cause) => return Err(error(&path, cause)),
        };
        if metadata.file_type().is_symlink() && name != "browser" {
            root.remove_file(name).map_err(|cause| error(&path, cause))?;
            continue;
        }
        if !metadata.is_dir() || metadata.dev() != device {
            return Err(error(&path, io::Error::new(io::ErrorKind::InvalidInput, "scratch root is not a same-device directory")));
        }
        let dir = root.open_dir_nofollow(name).map_err(|cause| error(&path, cause))?;
        let opened = dir.dir_metadata().map_err(|cause| error(&path, cause))?;
        if opened.dev() != device || opened.ino() != metadata.ino() {
            return Err(error(&path, io::Error::new(io::ErrorKind::InvalidInput, "scratch root identity changed")));
        }
        if name == "browser" {
            // Validate the scratch root first; stop must be confirmed before
            // deleting any profile or other pane-local state.
            browser.stop_previous_sessions().await.map_err(|cause| {
                InspectionError::new("ephemeral_reset_failed", format!("{}: {}", path.display(), cause.message))
            })?;
        }
        let keep: &[&str] = if name == "browser" { &["owner.lock", "owner.sock"] } else { &[] };
        empty_dir_contents(&dir, keep).map_err(|cause| error(&path, cause))?;
    }
    browser.prepare_state_dirs().map_err(|cause| {
        InspectionError::new("ephemeral_reset_failed", format!("{}: {}", state_root.join("browser").display(), cause.message))
    })
}

/// Capability-relative deletion: never follow links, cross devices, or recurse without a bound.
pub(crate) fn empty_dir_contents(dir: &Dir, keep: &[&str]) -> io::Result<()> {
    empty_directory(dir, dir.dir_metadata()?.dev(), 0, keep)
}

fn empty_directory(dir: &Dir, device: u64, depth: usize, keep: &[&str]) -> io::Result<()> {
    if depth >= 128 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "scratch nesting exceeds cleanup bound"));
    }
    for entry in dir.entries()? {
        let name = entry?.file_name();
        if keep.iter().any(|keep| name == std::ffi::OsStr::new(keep)) { continue; }
        let metadata = dir.symlink_metadata(&name)?;
        if !metadata.file_type().is_symlink() && metadata.dev() != device {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "cleanup refuses filesystem mount"));
        }
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            let child = dir.open_dir_nofollow(&name)?;
            let expected = child.dir_metadata()?;
            if expected.dev() != device || expected.ino() != metadata.ino() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "scratch directory identity changed"));
            }
            empty_directory(&child, device, depth + 1, &[])?;
            let current = dir.symlink_metadata(&name)?;
            if current.file_type().is_symlink() || !current.is_dir()
                || current.dev() != expected.dev() || current.ino() != expected.ino() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "scratch directory identity changed"));
            }
            dir.remove_dir(&name)?;
        } else {
            // This also unlinks symlinks themselves, never their targets.
            dir.remove_file(&name)?;
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{browser::{BrowserHerdrAdapter, BrowserHerdrSnapshot, STATE_DIRS}, config::BrowserConfiguration};
    use std::{fs, path::PathBuf, sync::Arc};
    use std::os::unix::fs::symlink;
    use uuid::Uuid;

    struct Offline;
    #[async_trait::async_trait]
    impl BrowserHerdrAdapter for Offline {
        async fn browser_snapshot(&self, _: &str) -> Result<BrowserHerdrSnapshot, InspectionError> {
            panic!("scratch reset must not call Herdr");
        }
    }
    struct Fixture(PathBuf);
    impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

    fn service(root: &Path) -> BrowserService {
        BrowserService::new(BrowserConfiguration {
            playwright_cli: root.join("unused-playwright-cli"),
            default_url: "about:blank".into(),
            chromium_executable: None, node_executable: None, browser_helper: None, playwright_core: None,
            feedback_retention_seconds: 3600, feedback_max_store_bytes: 1024 * 1024,
        }, root.to_owned(), Arc::new(Offline)).unwrap()
    }

    #[tokio::test]
    async fn owner_reset_clears_only_pane_scratch_and_unlinks_nested_symlinks() {
        let fixture = Fixture(std::env::temp_dir().join(format!("cockpit-ephemeral-{}", Uuid::new_v4())));
        fs::create_dir(&fixture.0).unwrap();
        let service = service(&fixture.0);
        for name in ["browser/legacy-archive/old.json", "browser/profiles/old/cookies", "comments/batch.json", "review/snapshot-old.json"] {
            let path = fixture.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"old work").unwrap();
        }
        for name in ["project.json", "Library/page.md", "vault/token", "config/settings.json", "outside/sentinel", "browser/owner.lock", "browser/owner.sock"] {
            let path = fixture.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, name).unwrap();
        }
        symlink(fixture.0.join("outside"), fixture.0.join("browser/profiles/link")).unwrap();
        reset_owner_state(&fixture.0, &service).await.unwrap();
        for name in ["project.json", "Library/page.md", "vault/token", "config/settings.json", "outside/sentinel", "browser/owner.lock", "browser/owner.sock"] {
            assert_eq!(fs::read(fixture.0.join(name)).unwrap(), name.as_bytes());
        }
        for name in STATE_DIRS {
            assert_eq!(fs::read_dir(fixture.0.join("browser").join(name)).unwrap().count(), 0);
        }
        assert!(!fixture.0.join("browser/legacy-archive").exists());
        for name in ["comments", "review"] { assert_eq!(fs::read_dir(fixture.0.join(name)).unwrap().count(), 0); }
    }

    #[tokio::test]
    async fn root_symlink_is_unlinked_without_following_comment_or_review_targets() {
        let fixture = Fixture(std::env::temp_dir().join(format!("cockpit-ephemeral-{}", Uuid::new_v4())));
        fs::create_dir(&fixture.0).unwrap();
        let service = service(&fixture.0);
        fs::create_dir(fixture.0.join("outside")).unwrap();
        fs::write(fixture.0.join("outside/keep"), b"safe").unwrap();
        for name in ["comments", "review"] { symlink(fixture.0.join("outside"), fixture.0.join(name)).unwrap(); }
        reset_owner_state(&fixture.0, &service).await.unwrap();
        assert_eq!(fs::read(fixture.0.join("outside/keep")).unwrap(), b"safe");
        for name in ["comments", "review"] { assert!(fs::symlink_metadata(fixture.0.join(name)).is_err()); }
    }

    #[test]
    fn deletion_rejects_device_and_depth_boundaries_without_removing_entries() {
        let fixture = Fixture(std::env::temp_dir().join(format!("cockpit-ephemeral-{}", Uuid::new_v4())));
        fs::create_dir(&fixture.0).unwrap();
        fs::write(fixture.0.join("keep"), b"safe").unwrap();
        let dir = crate::project_store::open_dir_nofollow_absolute(&fixture.0).unwrap();
        let device = dir.dir_metadata().unwrap().dev();
        assert_eq!(empty_directory(&dir, device.wrapping_add(1), 0, &[]).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(empty_directory(&dir, device, 128, &[]).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(fs::read(fixture.0.join("keep")).unwrap(), b"safe");
    }
}
