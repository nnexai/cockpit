use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, File, OpenOptions};
use sha2::{Digest, Sha256};

use crate::InspectionError;

pub(super) fn verified_checkout_path(
    cwd: &Option<String>,
    foreground_cwd: &Option<String>,
) -> Result<PathBuf, InspectionError> {
    let pane_cwd = foreground_cwd.as_ref().or(cwd.as_ref()).ok_or_else(|| {
        InspectionError::new(
            "review_unavailable",
            "pinned Review source did not report a checkout directory",
        )
    })?;
    let pane_path = PathBuf::from(pane_cwd);
    if !pane_path.is_absolute() {
        return Err(InspectionError::new(
            "review_checkout_mismatch",
            "Reviewr checkout path is not absolute",
        ));
    }
    Ok(pane_path)
}

pub(crate) fn checkout_source_id(checkout: &Path) -> Result<String, InspectionError> {
    let metadata = std::fs::symlink_metadata(checkout).map_err(|_| {
        InspectionError::new(
            "review_unavailable",
            "review checkout directory is unavailable",
        )
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(InspectionError::new(
            "review_checkout_mismatch",
            "review checkout identity is not a direct directory",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"cockpit-review-checkout-v1\0");
    hasher.update(checkout.to_string_lossy().as_bytes());
    hasher.update([0]);
    #[cfg(unix)]
    {
        hasher.update(cap_fs_ext::MetadataExt::dev(&metadata).to_le_bytes());
        hasher.update(cap_fs_ext::MetadataExt::ino(&metadata).to_le_bytes());
    }
    #[cfg(not(unix))]
    {
        hasher.update(metadata.len().to_le_bytes());
        hasher.update(
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|time| time.as_nanos().to_le_bytes().to_vec())
                .unwrap_or_default(),
        );
    }
    Ok(format!("review-{:x}", hasher.finalize()))
}

pub(super) fn open_worktree_parent(
    checkout: &Path,
    path: &Path,
) -> Result<(Dir, PathBuf), InspectionError> {
    let leaf = path.file_name().map(PathBuf::from).ok_or_else(|| {
        InspectionError::new(
            "review_invalid_path",
            "working-tree source has no file name",
        )
    })?;
    let mut parent = open_dir_nofollow_absolute(checkout)
        .map_err(|_| InspectionError::new("review_unreadable", "review checkout is unavailable"))?;
    let parent_path = path.parent().unwrap_or_else(|| Path::new(""));
    for component in parent_path.components() {
        let Component::Normal(name) = component else {
            return Err(InspectionError::new(
                "review_invalid_path",
                "Git returned an unsafe working-tree path",
            ));
        };
        parent = parent.open_dir_nofollow(Path::new(name)).map_err(|_| {
            InspectionError::new(
                "review_unreadable",
                "working-tree source parent is unavailable",
            )
        })?;
    }
    Ok((parent, leaf))
}

fn open_dir_nofollow_absolute(path: &Path) -> std::io::Result<Dir> {
    let mut dir = Dir::open_ambient_dir(Path::new("/"), cap_std::ambient_authority())?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => dir = dir.open_dir_nofollow(Path::new(name))?,
            Component::ParentDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "unsafe review checkout path",
                ));
            }
        }
    }
    Ok(dir)
}

pub(super) fn safe_relative_path(path: &str) -> Result<&Path, InspectionError> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(InspectionError::new(
            "review_invalid_path",
            "Git returned an unsafe working-tree path",
        ));
    }
    Ok(path)
}

pub(super) fn open_worktree_file(parent: &Dir, leaf: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    parent.open_with(leaf, &options)
}
