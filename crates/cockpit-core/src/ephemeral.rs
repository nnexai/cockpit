use crate::{InspectionError, browser::BrowserService};
use cap_fs_ext::DirExt;
use cap_std::fs::{Dir, MetadataExt};
use std::{io, path::Path};

/// Called only by the browser runtime holding owner.lock, before publishing IPC.
/// Existing pane data is never reset implicitly: an operator must inspect it first.
pub async fn inspect_owner_state(state_root: &Path, browser: &BrowserService) -> Result<(), InspectionError> {
    let error = |path: &Path, cause: io::Error| {
        InspectionError::new("owner_state_inspection_failed", format!("{}: {cause}", path.display()))
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
        if metadata.file_type().is_symlink() || !metadata.is_dir() || metadata.dev() != device {
            return Err(error(&path, io::Error::new(io::ErrorKind::InvalidInput, "saved state root is not a same-device directory; inspect and preserve it before starting")));
        }
        let dir = root.open_dir_nofollow(name).map_err(|cause| error(&path, cause))?;
        let opened = dir.dir_metadata().map_err(|cause| error(&path, cause))?;
        if opened.dev() != device || opened.ino() != metadata.ino() {
            return Err(error(&path, io::Error::new(io::ErrorKind::InvalidInput, "saved state root identity changed")));
        }
        let keep: &[&str] = if name == "browser" { &["owner.lock", "owner.sock"] } else { &[] };
        ensure_empty_tree(&dir, &path, device, 0, keep).map_err(|(path, cause)| error(&path, cause))?;
    }
    browser.prepare_state_dirs().map_err(|cause| {
        InspectionError::new("owner_state_inspection_failed", format!("{}: {}", state_root.join("browser").display(), cause.message))
    })
}

fn ensure_empty_tree(dir: &Dir, path: &Path, device: u64, depth: usize, keep: &[&str]) -> Result<(), (std::path::PathBuf, io::Error)> {
    let fail = |path: &Path, message| (path.to_owned(), io::Error::new(io::ErrorKind::InvalidInput, message));
    if depth >= 128 {
        return Err(fail(path, "saved state nesting exceeds inspection bound"));
    }
    for entry in dir.entries().map_err(|cause| (path.to_owned(), cause))? {
        let name = entry.map_err(|cause| (path.to_owned(), cause))?.file_name();
        if keep.iter().any(|keep| name == std::ffi::OsStr::new(keep)) { continue; }
        let child_path = path.join(&name);
        let metadata = dir.symlink_metadata(&name).map_err(|cause| (child_path.clone(), cause))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() || metadata.dev() != device {
            return Err(fail(&child_path, "saved or unknown state found; inspect and preserve it before starting"));
        }
        let child = dir.open_dir_nofollow(&name).map_err(|cause| (child_path.clone(), cause))?;
        let opened = child.dir_metadata().map_err(|cause| (child_path.clone(), cause))?;
        if opened.dev() != device || opened.ino() != metadata.ino() {
            return Err(fail(&child_path, "saved state directory identity changed"));
        }
        ensure_empty_tree(&child, &child_path, device, depth + 1, &[])?;
    }
    Ok(())
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
            panic!("owner startup inspection must not call Herdr");
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
    async fn owner_start_preserves_saved_files_in_each_root() {
        for name in ["browser/profiles/old/cookies", "browser/scratch-extra/work.json", "comments/batch.json", "review/snapshot-old.json"] {
            let fixture = Fixture(std::env::temp_dir().join(format!("cockpit-ephemeral-{}", Uuid::new_v4())));
            fs::create_dir(&fixture.0).unwrap();
            let service = service(&fixture.0);
            let path = fixture.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"saved work").unwrap();
            let error = inspect_owner_state(&fixture.0, &service).await.unwrap_err();
            assert_eq!(error.code, "owner_state_inspection_failed");
            assert!(error.message.contains(&path.display().to_string()));
            assert_eq!(fs::read(path).unwrap(), b"saved work");
        }
    }

    #[tokio::test]
    async fn owner_start_allows_empty_trees_and_preserves_owner_entries() {
        let fixture = Fixture(std::env::temp_dir().join(format!("cockpit-ephemeral-{}", Uuid::new_v4())));
        fs::create_dir(&fixture.0).unwrap();
        let service = service(&fixture.0);
        for name in ["browser/extra/nested", "comments/nested", "review/nested"] {
            fs::create_dir_all(fixture.0.join(name)).unwrap();
        }
        fs::write(fixture.0.join("browser/owner.lock"), b"lock").unwrap();
        let _socket = std::os::unix::net::UnixListener::bind(fixture.0.join("browser/owner.sock")).unwrap();
        inspect_owner_state(&fixture.0, &service).await.unwrap();
        assert_eq!(fs::read(fixture.0.join("browser/owner.lock")).unwrap(), b"lock");
        assert!(fixture.0.join("browser/owner.sock").exists());
        for name in STATE_DIRS {
            assert!(fixture.0.join("browser").join(name).is_dir());
        }
        assert!(fixture.0.join("comments/nested").is_dir());
    }

    #[tokio::test]
    async fn owner_start_rejects_symlinks_and_unknown_sockets_without_unlinking() {
        for name in ["browser/profiles/link", "comments", "review", "review/nested/link"] {
            let fixture = Fixture(std::env::temp_dir().join(format!("cockpit-ephemeral-{}", Uuid::new_v4())));
            fs::create_dir(&fixture.0).unwrap();
            let service = service(&fixture.0);
            fs::create_dir(fixture.0.join("outside")).unwrap();
            fs::write(fixture.0.join("outside/keep"), b"safe").unwrap();
            let path = fixture.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            symlink(fixture.0.join("outside"), &path).unwrap();
            assert_eq!(inspect_owner_state(&fixture.0, &service).await.unwrap_err().code, "owner_state_inspection_failed");
            assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
            assert_eq!(fs::read(fixture.0.join("outside/keep")).unwrap(), b"safe");
        }
        let fixture = Fixture(std::env::temp_dir().join(format!("cockpit-ephemeral-{}", Uuid::new_v4())));
        fs::create_dir(&fixture.0).unwrap();
        let service = service(&fixture.0);
        let path = fixture.0.join("browser/profiles/unknown.sock");
        let _socket = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert_eq!(inspect_owner_state(&fixture.0, &service).await.unwrap_err().code, "owner_state_inspection_failed");
        assert!(fs::symlink_metadata(path).is_ok());
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
