//! Optional, launch-local OMP hook. Never installs into the user's extension directory.

use std::{fs, io::{Read, Write}, path::{Path, PathBuf}};
use sha2::{Digest, Sha256};
use crate::InspectionError;

const SOURCE: &str = include_str!("../../../integrations/omp-hook/cockpit-hook.ts");

/// Publish the embedded hook under the caller's existing private state root.
/// Content-addressed names let different binary versions coexist without rewriting
/// a file an already-running OMP process may still import.
pub fn materialize(base: &Path) -> Result<PathBuf, InspectionError> {
    let metadata = fs::symlink_metadata(base).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(unsafe_path());
    }
    require_owner(&metadata)?;
    let root = base.canonicalize().map_err(io_error)?;
    let directory = root.join("omp-hooks");
    match fs::create_dir(&directory) {
        Ok(()) => private_permissions(&directory, false)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(io_error(error)),
    }
    let metadata = fs::symlink_metadata(&directory).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(unsafe_path());
    }
    require_owner(&metadata)?;
    private_permissions(&directory, false)?;
    let digest = Sha256::digest(SOURCE.as_bytes());
    let path = directory.join(format!("cockpit-hook-{digest:x}.ts"));
    if path.try_exists().map_err(io_error)? {
        verify_source(&path)?;
        return Ok(path);
    }
    let temporary = directory.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(io_error)?;
        file.write_all(SOURCE.as_bytes()).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        // Hard-link publication is atomic and never replaces another publisher's file.
        match fs::hard_link(&temporary, &path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => verify_source(&path)?,
            Err(error) => return Err(io_error(error)),
        }
        #[cfg(unix)] fs::File::open(&directory).and_then(|file| file.sync_all()).map_err(io_error)?;
        Ok(path)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn verify_source(path: &Path) -> Result<(), InspectionError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != SOURCE.len() as u64 {
        return Err(unsafe_path());
    }
    require_owner(&metadata)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(io_error)?;
    let mut bytes = Vec::with_capacity(SOURCE.len());
    file.take(SOURCE.len() as u64 + 1).read_to_end(&mut bytes).map_err(io_error)?;
    if bytes != SOURCE.as_bytes() { return Err(unsafe_path()); }
    private_permissions(path, true)
}

fn require_owner(metadata: &fs::Metadata) -> Result<(), InspectionError> {
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != nix::unistd::Uid::effective().as_raw() { return Err(unsafe_path()); }
    }
    #[cfg(not(unix))] { let _ = metadata; }
    Ok(())
}

fn private_permissions(path: &Path, file: bool) -> Result<(), InspectionError> {
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(if file { 0o600 } else { 0o700 })).map_err(io_error)?;
    }
    #[cfg(not(unix))] { let _ = (path, file); }
    Ok(())
}

fn unsafe_path() -> InspectionError {
    InspectionError::new("unsafe_path", "OMP hook materialization requires an owned regular path with exact packaged bytes")
}
fn io_error(error: std::io::Error) -> InspectionError {
    InspectionError::new("omp_hook_materialization_failed", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materializes_exact_private_source_idempotently() {
        let root = std::env::temp_dir().join(format!("cockpit-hook-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let first = materialize(&root).unwrap();
        assert_eq!(fs::read_to_string(&first).unwrap(), SOURCE);
        assert_eq!(materialize(&root).unwrap(), first);
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&first).unwrap().permissions().mode() & 0o777, 0o600);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_directory_and_content_tampering() {
        let root = std::env::temp_dir().join(format!("cockpit-hook-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let elsewhere = root.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("omp-hooks")).unwrap();
        assert_eq!(materialize(&root).unwrap_err().code, "unsafe_path");
        fs::remove_file(root.join("omp-hooks")).unwrap();
        let path = materialize(&root).unwrap();
        fs::write(path, "changed").unwrap();
        assert_eq!(materialize(&root).unwrap_err().code, "unsafe_path");
        fs::remove_dir_all(root).unwrap();
    }
}
