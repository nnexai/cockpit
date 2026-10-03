use std::collections::BTreeSet;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use cockpit_protocol::projects::ProjectConfiguration;
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::InspectionError;
use crate::process::run_bounded_command;

#[derive(Debug)]
pub(crate) struct SourceBytes {
    pub(crate) bytes: Vec<u8>,
    pub(crate) hash: String,
    identity: String,
}

#[derive(Debug)]
pub(crate) struct Gitlink {
    pub(crate) path: PathBuf,
    pub(crate) commit: String,
}

pub(crate) async fn git_inventory(
    configuration: &ProjectConfiguration,
    source: &Path,
) -> Result<
    (
        Vec<PathBuf>,
        Vec<Gitlink>,
        Vec<PathBuf>,
    ),
    InspectionError,
> {
    let tracked = git_output(
        configuration,
        source,
        &["ls-files", "--stage", "-z", "--cached"],
    )
    .await?;
    let untracked = git_output(
        configuration,
        source,
        &["ls-files", "-z", "--others", "--exclude-standard"],
    )
    .await?;
    for output in [&tracked, &untracked] {
        if !output.status.success() {
            return Err(InspectionError::new(
                "context_snapshot_git_failed",
                "Git could not inventory the selected repository without mutation",
            ));
        }
    }
    let mut paths = BTreeSet::new();
    let mut gitlinks = Vec::new();
    let mut excluded = BTreeSet::new();
    for raw in tracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let separator = raw.iter().position(|byte| *byte == b'\t').ok_or_else(|| {
            InspectionError::new(
                "context_snapshot_git_output",
                "Git returned an invalid index record",
            )
        })?;
        let (stage, raw_path) = raw.split_at(separator);
        let raw_path = &raw_path[1..];
        let stage = std::str::from_utf8(stage).map_err(|_| {
            InspectionError::new(
                "context_snapshot_git_output",
                "Git returned non-UTF-8 index metadata",
            )
        })?;
        let mut fields = stage.split_whitespace();
        let mode = fields.next().ok_or_else(|| {
            InspectionError::new("context_snapshot_git_output", "Git index mode is missing")
        })?;
        let object = fields.next().ok_or_else(|| {
            InspectionError::new("context_snapshot_git_output", "Git index object is missing")
        })?;
        let text = std::str::from_utf8(raw_path).map_err(|_| {
            InspectionError::new(
                "context_snapshot_non_utf8_path",
                "folder capture paths must be valid UTF-8",
            )
        })?;
        let path = safe_source_relative(text)?;
        if excluded_source_path(&path) {
            excluded.insert(path);
            continue;
        }
        if mode == "160000" {
            gitlinks.push(Gitlink {
                path,
                commit: object.to_owned(),
            });
        } else {
            paths.insert(path);
        }
    }
    for raw in untracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let text = std::str::from_utf8(raw).map_err(|_| {
            InspectionError::new(
                "context_snapshot_non_utf8_path",
                "folder capture paths must be valid UTF-8",
            )
        })?;
        let path = safe_source_relative(text)?;
        if excluded_source_path(&path) {
            excluded.insert(path);
            continue;
        }
        paths.insert(path);
    }
    Ok((
        paths.into_iter().collect(),
        gitlinks,
        excluded.into_iter().collect(),
    ))
}

pub(crate) async fn git_output(
    configuration: &ProjectConfiguration,
    source: &Path,
    args: &[&str],
) -> Result<std::process::Output, InspectionError> {
    let mut command = Command::new("git");
    command
        .current_dir(source)
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY");
    run_bounded_command(
        command,
        configuration.limits.git_output_bytes as usize,
        configuration.limits.git_output_bytes as usize,
        Duration::from_millis(configuration.limits.git_timeout_ms as u64),
        "context_snapshot_git",
    )
    .await
}

pub(crate) fn read_stable_source_bounded(
    root: &Dir, relative: &Path, max_bytes: u64,
) -> Result<SourceBytes, InspectionError> {
    let (parent, leaf) = resolve_parent(root, relative)?;
    let before = read_regular_bounded(&parent, &leaf, max_bytes)?;
    let after = read_regular_bounded(&parent, &leaf, max_bytes)?;
    if before.identity != after.identity || before.hash != after.hash {
        return Err(InspectionError::new(
            "context_snapshot_source_changed",
            "a source file changed during folder capture; retry to capture a complete generation",
        ));
    }
    Ok(before)
}

fn read_regular_bounded(parent: &Dir, leaf: &Path, max_bytes: u64) -> Result<SourceBytes, InspectionError> {
    let metadata = parent.symlink_metadata(leaf).map_err(|error| {
        InspectionError::new(
            if error.kind() == ErrorKind::NotFound {
                "context_snapshot_file_missing"
            } else {
                "context_snapshot_file_unavailable"
            },
            error.to_string(),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(InspectionError::new(
            "context_snapshot_symlink",
            "symbolic links are recorded as skipped",
        ));
    }
    if !metadata.is_file() {
        return Err(InspectionError::new(
            "context_snapshot_special_file",
            "only regular files are eligible for folder capture",
        ));
    }
    if metadata.len() > max_bytes {
        return Err(InspectionError::new(
            "context_snapshot_file_bytes",
            "a source file exceeds Cockpit's folder capture file limit",
        ));
    }
    if hardlinked(&metadata) {
        return Err(InspectionError::new(
            "context_snapshot_hardlink",
            "hardlinked source files are not captured into the Library",
        ));
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let file = parent.open_with(leaf, &options).map_err(|error| {
        InspectionError::new("context_snapshot_file_unavailable", error.to_string())
    })?;
    let opened = file.metadata().map_err(|error| {
        InspectionError::new("context_snapshot_file_unavailable", error.to_string())
    })?;
    if opened.file_type().is_symlink()
        || !opened.is_file()
        || identity(&opened) != identity(&metadata)
    {
        return Err(InspectionError::new(
            "context_snapshot_source_changed",
            "a source file changed while opening",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            InspectionError::new("context_snapshot_file_unavailable", error.to_string())
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(InspectionError::new(
            "context_snapshot_file_bytes",
            "a source file grew beyond Cockpit's folder capture file limit",
        ));
    }
    if native_executable(&opened, &bytes) {
        return Err(InspectionError::new(
            "context_snapshot_native_binary",
            "native executable binaries are not captured into the Library",
        ));
    }
    Ok(SourceBytes {
        hash: hash(&bytes),
        bytes,
        identity: identity(&opened),
    })
}

pub(crate) fn create_parent(root: &Dir, relative: &Path) -> Result<(Dir, PathBuf), InspectionError> {
    let leaf = relative.file_name().ok_or_else(|| {
        InspectionError::new("context_snapshot_path", "folder capture path has no file name")
    })?;
    let mut current = root
        .try_clone()
        .map_err(io_error("context_snapshot_destination_unavailable"))?;
    for component in relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
    {
        let Component::Normal(name) = component else {
            return Err(InspectionError::new(
                "context_snapshot_path",
                "folder capture path escapes its root",
            ));
        };
        current = ensure_directory(
            &current,
            name.to_str().ok_or_else(|| {
                InspectionError::new("context_snapshot_path", "folder capture paths must be UTF-8")
            })?,
        )?;
    }
    Ok((current, leaf.into()))
}

fn ensure_directory(root: &Dir, name: &str) -> Result<Dir, InspectionError> {
    match root.create_dir(name) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(InspectionError::new(
                "context_snapshot_destination_unavailable",
                error.to_string(),
            ));
        }
    }
    let metadata = root
        .symlink_metadata(name)
        .map_err(io_error("context_snapshot_destination_unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(InspectionError::new(
            "context_snapshot_destination_unavailable",
            "folder capture directory is not a real directory",
        ));
    }
    root.open_dir_nofollow(Path::new(name))
        .map_err(io_error("context_snapshot_destination_unavailable"))
}

fn open_directory(root: &Dir, path: &Path) -> Result<Dir, InspectionError> {
    let mut current = root
        .try_clone()
        .map_err(io_error("context_snapshot_destination_unavailable"))?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(InspectionError::new(
                "context_snapshot_path",
                "folder capture path escapes its root",
            ));
        };
        current = current
            .open_dir_nofollow(Path::new(name))
            .map_err(|error| {
                if error.kind() == ErrorKind::NotFound {
                    InspectionError::new("context_snapshot_file_missing", error.to_string())
                } else {
                    io_error("context_snapshot_destination_unavailable")(error)
                }
            })?;
    }
    Ok(current)
}

fn resolve_parent(root: &Dir, relative: &Path) -> Result<(Dir, PathBuf), InspectionError> {
    let leaf = relative.file_name().ok_or_else(|| {
        InspectionError::new("context_snapshot_path", "folder capture path has no file name")
    })?;
    Ok((
        open_directory(root, relative.parent().unwrap_or_else(|| Path::new("")))?,
        leaf.into(),
    ))
}

fn safe_source_relative(value: &str) -> Result<PathBuf, InspectionError> {
    let candidate = Path::new(value);
    if value.is_empty() || candidate.is_absolute() || value.contains('\0') {
        return Err(InspectionError::new(
            "context_snapshot_path",
            "folder capture paths must be bounded relative paths",
        ));
    }
    let mut result = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => result.push(part),
            _ => {
                return Err(InspectionError::new(
                    "context_snapshot_path",
                    "folder capture path escapes its root",
                ));
            }
        }
    }
    Ok(result)
}

pub(crate) fn excluded_source_path(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(name) = component else {
            return true;
        };
        matches!(
            name.to_str(),
            Some(".git" | "node_modules" | "target" | "build" | "dist" | ".next")
        )
    })
}

pub(crate) fn source_root_revalidate(source_root: &Dir, source: &Path) -> Result<(), InspectionError> {
    let opened = source_root.dir_metadata().map_err(|error| {
        InspectionError::new("context_snapshot_repository_changed", error.to_string())
    })?;
    let reopened = open_absolute_dir_nofollow(source).map_err(|error| {
        InspectionError::new("context_snapshot_repository_changed", error.to_string())
    })?;
    let current = reopened.dir_metadata().map_err(|error| {
        InspectionError::new("context_snapshot_repository_changed", error.to_string())
    })?;
    if !opened.is_dir()
        || !current.is_dir()
        || object_identity(&opened) != object_identity(&current)
    {
        return Err(InspectionError::new(
            "context_snapshot_repository_changed",
            "the selected folder changed after resolution",
        ));
    }
    Ok(())
}

pub(crate) fn open_absolute_dir_nofollow(path: &Path) -> std::io::Result<Dir> {
    let mut dir = Dir::open_ambient_dir(Path::new("/"), cap_std::ambient_authority())?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => dir = dir.open_dir_nofollow(Path::new(name))?,
            Component::ParentDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "unsafe absolute path",
                ));
            }
        }
    }
    Ok(dir)
}

/// A file or directory name a person can read: letters, digits, `.`, `_` and
/// `-`, with other characters replaced by `-`. Never empty or hidden.
pub(crate) fn readable_name(value: &str) -> String {
    let mut name = String::new();
    for character in value.chars() {
        let character = if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        {
            character
        } else {
            '-'
        };
        if character == '-' && name.ends_with('-') {
            continue;
        }
        name.push(character);
        if name.len() >= 80 {
            break;
        }
    }
    let name = name.trim_matches(|character| character == '-' || character == '.');
    if name.is_empty() {
        "item".to_owned()
    } else {
        name.to_owned()
    }
}

fn hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn identity(metadata: &Metadata) -> String {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        return format!(
            "{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime_nsec()
        );
    }
    #[cfg(not(unix))]
    {
        format!("{}:{:?}", metadata.len(), metadata.modified().ok())
    }
}

fn object_identity(metadata: &Metadata) -> String {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        return format!("{}:{}", metadata.dev(), metadata.ino());
    }
    #[cfg(not(unix))]
    {
        format!("{}", metadata.len())
    }
}

fn hardlinked(metadata: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        return metadata.nlink() > 1;
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

fn native_executable(metadata: &Metadata, bytes: &[u8]) -> bool {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.mode() & 0o111 == 0 {
            return false;
        }
        return bytes.starts_with(b"\x7fELF")
            || matches!(
                bytes.get(..4),
                Some(
                    [0xfe, 0xed, 0xfa, 0xce]
                        | [0xfe, 0xed, 0xfa, 0xcf]
                        | [0xcf, 0xfa, 0xed, 0xfe]
                        | [0xce, 0xfa, 0xed, 0xfe]
                )
            );
    }
    #[cfg(not(unix))]
    {
        let _ = (metadata, bytes);
        false
    }
}


fn io_error(code: &'static str) -> impl FnOnce(std::io::Error) -> InspectionError {
    move |error| InspectionError::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn temp_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("cockpit-library-folder-io-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create temporary directory");
        path
    }

    #[test]
    fn generated_names_are_readable_and_safe() {
        assert_eq!(super::readable_name("PROJ-123"), "PROJ-123");
        assert_eq!(super::readable_name("group/project!12"), "group-project-12");
        assert_eq!(super::readable_name("../..//"), "item");
        assert_eq!(super::readable_name(".hidden"), "hidden");
        assert!(super::readable_name(&"x".repeat(500)).len() <= 80);
    }

    #[test]
    fn source_policy_rejects_escape_and_excluded_paths() {
        assert!(safe_source_relative("../escape").is_err());
        assert!(excluded_source_path(
            &safe_source_relative(".git/config").expect("relative")
        ));
        assert!(excluded_source_path(
            &safe_source_relative("node_modules/pkg/index.js").expect("relative")
        ));
        assert_eq!(
            safe_source_relative("src/lib.rs")
                .expect("safe")
                .to_string_lossy(),
            "src/lib.rs"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_hardlink_are_not_eligible_sources() {
        use std::os::unix::fs::symlink;
        let root = temp_dir("links");
        fs::write(root.join("source"), b"bytes").expect("source");
        symlink("source", root.join("link")).expect("symlink");
        fs::hard_link(root.join("source"), root.join("alias")).expect("hardlink");
        let root_dir =
            Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        assert_eq!(
            read_stable_source_bounded(&root_dir, Path::new("link"), 4 * 1024 * 1024)
                .expect_err("symlink")
                .code,
            "context_snapshot_symlink"
        );
        assert_eq!(
            read_stable_source_bounded(&root_dir, Path::new("alias"), 4 * 1024 * 1024)
                .expect_err("hardlink")
                .code,
            "context_snapshot_hardlink"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}
