use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use cockpit_core::InspectionError;
use fs2::FileExt;

use crate::lifecycle::ImageIdentity;

#[derive(Debug, Clone, Copy)]
pub enum Role {
    Gui,
    Cli,
}

#[derive(Debug, Clone)]
pub struct InstalledPaths {
    pub data_home: PathBuf,
    pub bin_home: PathBuf,
    pub app_bundle: Option<PathBuf>,
}

pub fn installed_paths(env: &BTreeMap<String, String>) -> InstalledPaths {
    let home = PathBuf::from(env.get("HOME").map(String::as_str).unwrap_or(""));
    #[cfg(target_os = "macos")]
    let default_data = home.join("Library/Application Support");
    #[cfg(not(target_os = "macos"))]
    let default_data = home.join(".local/share");
    InstalledPaths {
        data_home: env
            .get("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or(default_data),
        bin_home: env
            .get("XDG_BIN_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/bin")),
        #[cfg(target_os = "macos")]
        app_bundle: Some(home.join("Applications/Cockpit.app")),
        #[cfg(not(target_os = "macos"))]
        app_bundle: None,
    }
}

pub fn installed_cli(paths: &InstalledPaths) -> PathBuf {
    paths.data_home.join("cockpit/bin/cockpit-cli")
}

pub fn installed_gui(paths: &InstalledPaths) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        paths
            .app_bundle
            .as_ref()
            .expect("macOS installation requires an app bundle")
            .join("Contents/MacOS/cockpit-tauri")
    }
    #[cfg(not(target_os = "macos"))]
    {
        paths.data_home.join("cockpit/bin/cockpit")
    }
}

fn identity(metadata: &fs::Metadata) -> ImageIdentity {
    ImageIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    }
}

pub fn startup_image() -> io::Result<(PathBuf, ImageIdentity)> {
    #[cfg(target_os = "linux")]
    {
        let file = File::open("/proc/self/exe")?;
        let image = identity(&file.metadata()?);
        Ok((fs::read_link("/proc/self/exe")?, image))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let exe = std::env::current_exe()?;
        let image = identity(&fs::metadata(&exe)?);
        Ok((exe, image))
    }
}

pub fn is_installed(exe: &Path, paths: &InstalledPaths) -> bool {
    let Ok(exe) = fs::canonicalize(exe) else {
        return false;
    };
    let matches = |candidate: PathBuf| fs::canonicalize(candidate).is_ok_and(|path| path == exe);
    if matches(installed_cli(paths)) || matches(installed_gui(paths)) {
        return true;
    }
    paths
        .app_bundle
        .as_ref()
        .is_some_and(|bundle| matches(bundle.join("Contents/MacOS/cockpit-cli")))
}

pub fn update_installed(role: Role, started: &ImageIdentity, paths: &InstalledPaths) -> bool {
    #[cfg(target_os = "macos")]
    if matches!(role, Role::Cli) {
        let standalone = fs::metadata(installed_cli(paths)).ok();
        let bundled = paths
            .app_bundle
            .as_ref()
            .and_then(|bundle| fs::metadata(bundle.join("Contents/MacOS/cockpit-cli")).ok());
        let mut candidates = standalone.iter().chain(bundled.iter()).peekable();
        return candidates.peek().is_some()
            && candidates.all(|metadata| identity(metadata) != *started);
    }
    let path = match role {
        Role::Gui => installed_gui(paths),
        Role::Cli => installed_cli(paths),
    };
    fs::metadata(path).is_ok_and(|metadata| identity(&metadata) != *started)
}

pub fn installed_build_commit(paths: &InstalledPaths) -> Option<String> {
    let receipt = File::open(paths.data_home.join("cockpit/install.json")).ok()?;
    let value: serde_json::Value = serde_json::from_reader(receipt).ok()?;
    value.get("build_commit")?.as_str().map(str::to_owned)
}

/// Holds the same BSD flock as the installer; closing the file releases it.
pub struct UpdateLock {
    _file: File,
}

pub fn acquire_update_lock(
    paths: &InstalledPaths,
    deadline: Duration,
) -> Result<UpdateLock, InspectionError> {
    let root = paths.data_home.join("cockpit");
    fs::create_dir_all(&root).map_err(|e| InspectionError::new("unsafe_path", e.to_string()))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC | nix::libc::O_NONBLOCK)
        .open(root.join("update.lock"))
        .map_err(|e| InspectionError::new("unsafe_path", e.to_string()))?;
    let meta = file
        .metadata()
        .map_err(|e| InspectionError::new("unsafe_path", e.to_string()))?;
    if !meta.is_file()
        || meta.uid() != nix::unistd::Uid::current().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return Err(InspectionError::new(
            "unsafe_path",
            "update lock must be a private regular file owned by the current user",
        ));
    }
    let start = Instant::now();
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(UpdateLock { _file: file }),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let remaining = deadline.saturating_sub(start.elapsed());
                if remaining.is_zero() {
                    return Err(InspectionError::new(
                        "update_busy",
                        "timed out waiting for update lock",
                    ));
                }
                std::thread::sleep(remaining.min(Duration::from_millis(250)));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(InspectionError::new("unsafe_path", error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("cockpit-installation-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(path.join("cockpit/bin")).unwrap();
            Self(path)
        }
        fn paths(&self) -> InstalledPaths {
            InstalledPaths {
                data_home: self.0.clone(),
                bin_home: self.0.join("bin"),
                app_bundle: Some(self.0.join("Cockpit.app")),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn paths_follow_environment_and_defaults() {
        let mut env = BTreeMap::from([(String::from("HOME"), String::from("/home/example"))]);
        let paths = installed_paths(&env);
        assert_eq!(paths.bin_home, PathBuf::from("/home/example/.local/bin"));
        #[cfg(target_os = "linux")]
        assert_eq!(paths.data_home, PathBuf::from("/home/example/.local/share"));
        #[cfg(target_os = "macos")]
        assert_eq!(
            paths.data_home,
            PathBuf::from("/home/example/Library/Application Support")
        );
        env.insert("XDG_DATA_HOME".into(), "/custom/share".into());
        env.insert("XDG_BIN_HOME".into(), "/custom/bin".into());
        let paths = installed_paths(&env);
        assert_eq!(
            installed_cli(&paths),
            PathBuf::from("/custom/share/cockpit/bin/cockpit-cli")
        );
        assert_eq!(paths.bin_home, PathBuf::from("/custom/bin"));
    }

    #[test]
    fn identity_and_receipt_report_actual_files() {
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let cli = installed_cli(&paths);
        fs::write(&cli, b"old").unwrap();
        let started = identity(&fs::metadata(&cli).unwrap());
        assert!(is_installed(&cli, &paths));
        assert!(!update_installed(Role::Cli, &started, &paths));
        let other = fixture.0.join("other");
        fs::write(&other, b"new").unwrap();
        assert!(!is_installed(&other, &paths));
        fs::rename(other, &cli).unwrap();
        assert!(update_installed(Role::Cli, &started, &paths));
        assert_eq!(installed_build_commit(&paths), None);
        fs::write(
            fixture.0.join("cockpit/install.json"),
            r#"{"build_commit":"abc"}"#,
        )
        .unwrap();
        assert_eq!(installed_build_commit(&paths).as_deref(), Some("abc"));
        fs::write(
            fixture.0.join("cockpit/install.json"),
            r#"{"build_commit":null}"#,
        )
        .unwrap();
        assert_eq!(installed_build_commit(&paths), None);
        let (exe, image) = startup_image().unwrap();
        assert_eq!(image, identity(&fs::metadata(exe).unwrap()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bundled_cli_identity_is_compared_with_both_installed_candidates() {
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let standalone = installed_cli(&paths);
        let bundled = paths
            .app_bundle
            .as_ref()
            .unwrap()
            .join("Contents/MacOS/cockpit-cli");
        fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        fs::write(&standalone, b"standalone").unwrap();
        fs::write(&bundled, b"bundled").unwrap();
        let original = identity(&fs::metadata(&bundled).unwrap());
        assert!(is_installed(&bundled, &paths));
        assert!(!update_installed(Role::Cli, &original, &paths));
        let replacement = fixture.0.join("replacement");
        fs::write(&replacement, b"new").unwrap();
        fs::rename(replacement, bundled).unwrap();
        assert!(update_installed(Role::Cli, &original, &paths));
    }

    #[test]
    fn lock_excludes_other_openers_and_rejects_unsafe_files() {
        let fixture = Fixture::new();
        let paths = fixture.paths();
        let lock = acquire_update_lock(&paths, Duration::ZERO).unwrap();
        assert_eq!(
            acquire_update_lock(&paths, Duration::ZERO)
                .err()
                .unwrap()
                .code,
            "update_busy"
        );
        drop(lock);
        drop(acquire_update_lock(&paths, Duration::ZERO).unwrap());
        let path = fixture.0.join("cockpit/update.lock");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(acquire_update_lock(&paths, Duration::ZERO).is_err());
        fs::remove_file(&path).unwrap();
        symlink(fixture.0.join("target"), &path).unwrap();
        assert!(acquire_update_lock(&paths, Duration::ZERO).is_err());
    }
}
