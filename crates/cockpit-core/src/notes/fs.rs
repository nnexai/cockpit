use crate::InspectionError;
use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::notes::NotesDocument;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub(super) fn error(code: &str, message: impl Into<String>) -> InspectionError {
    InspectionError::new(code, message)
}
fn outcome_unknown(cause: impl std::fmt::Display) -> InspectionError {
    error(
        "notes_outcome_unknown",
        format!(
            "{cause}; write outcome is unknown. Re-read the affected Notes content before retrying; do not blindly retry the mutation."
        ),
    )
}
pub(super) fn io_error(e: io::Error) -> InspectionError {
    match e.kind() {
        io::ErrorKind::NotFound => error("notes_not_found", "Notes path does not exist"),
        io::ErrorKind::InvalidInput
        | io::ErrorKind::IsADirectory
        | io::ErrorKind::NotADirectory => error(
            "notes_unsafe_path",
            "Notes path is not a safe regular file or directory",
        ),
        _ => error("notes_unavailable", e.to_string()),
    }
}
pub(super) fn revision(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
pub(super) fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("UTC timestamp is representable")
}
pub(super) fn validate_id(value: &str, max: usize) -> Result<(), InspectionError> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(error(
            "notes_invalid_input",
            "Invalid Notes record identifier",
        ));
    }
    Ok(())
}
pub(super) fn validate_uuid(value: &str) -> Result<(), InspectionError> {
    if Uuid::parse_str(value)
        .ok()
        .is_none_or(|id| id.to_string() != value)
    {
        return Err(error(
            "notes_invalid_target",
            "Notes identifier must be a lowercase hyphenated UUID",
        ));
    }
    Ok(())
}
pub(super) fn absolute(path: &Path) -> Result<PathBuf, InspectionError> {
    let raw = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(io_error)?.join(path)
    };
    let mut out = PathBuf::new();
    for part in raw.components() {
        match part {
            Component::RootDir => out.push("/"),
            Component::CurDir => {}
            Component::Normal(name) => out.push(name),
            _ => {
                return Err(error(
                    "notes_unsafe_path",
                    "Notes root must not contain parent traversal",
                ));
            }
        }
    }
    Ok(out)
}
pub(super) fn root(path: &Path, create: bool) -> Result<(PathBuf, Dir), InspectionError> {
    let path = absolute(path)?;
    let mut dir = Dir::open_ambient_dir("/", cap_std::ambient_authority()).map_err(io_error)?;
    for part in path.components() {
        if let Component::Normal(name) = part {
            dir = child_os(&dir, name, create)?;
        }
    }
    Ok((path, dir))
}
fn child_os(dir: &Dir, name: &std::ffi::OsStr, create: bool) -> Result<Dir, InspectionError> {
    match dir.symlink_metadata(name) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            return Err(error(
                "notes_unsafe_path",
                "Notes directory is not a real directory",
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound && create => {
            if let Err(e) = dir.create_dir(name) {
                if e.kind() != io::ErrorKind::AlreadyExists {
                    return Err(io_error(e));
                }
            }
        }
        Err(e) => return Err(io_error(e)),
    }
    dir.open_dir_nofollow(Path::new(name)).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound || e.kind() == io::ErrorKind::InvalidInput {
            error("notes_unsafe_path", "Notes directory changed while opening")
        } else {
            io_error(e)
        }
    })
}
pub(super) fn child(dir: &Dir, name: &str, create: bool) -> Result<Dir, InspectionError> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name == "."
        || name == ".."
        || name.contains('\0')
    {
        return Err(error("notes_unsafe_path", "Invalid directory component"));
    }
    child_os(dir, std::ffi::OsStr::new(name), create)
}
pub(super) fn entries(dir: &Dir, max: usize) -> Result<Vec<String>, InspectionError> {
    let mut out = Vec::new();
    for (scanned, entry) in dir.entries().map_err(io_error)?.enumerate() {
        if scanned >= max {
            return Err(error(
                "notes_too_large",
                "Notes collection exceeds its entry limit",
            ));
        }
        // Valid record IDs are ASCII. Non-UTF-8 filenames cannot be records, but
        // still count toward the bounded directory scan.
        let Ok(name) = entry.map_err(io_error)?.file_name().into_string() else {
            continue;
        };
        out.push(name);
    }
    out.sort();
    Ok(out)
}
pub(super) fn read(dir: &Dir, name: &str, max: usize) -> Result<NotesDocument, InspectionError> {
    let metadata = match dir.symlink_metadata(name) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(NotesDocument {
                content: String::new(),
                revision: "absent".into(),
            });
        }
        Err(e) => return Err(io_error(e)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(error(
            "notes_unsafe_path",
            "Notes document is not a regular file",
        ));
    }
    if metadata.len() > max as u64 {
        return Err(error(
            "notes_too_large",
            "Notes document exceeds its size limit",
        ));
    }
    let mut opts = OpenOptions::new();
    opts.read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let mut file = dir.open_with(name, &opts).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() {
        return Err(error(
            "notes_unsafe_path",
            "Opened Notes document is not a regular file",
        ));
    }
    if metadata.len() > max as u64 {
        return Err(error(
            "notes_too_large",
            "Notes document exceeds its size limit",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&mut file)
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > max {
        return Err(error(
            "notes_too_large",
            "Notes document grew beyond its size limit",
        ));
    }
    let revision = revision(&bytes);
    let content = String::from_utf8(bytes)
        .map_err(|_| error("notes_invalid_encoding", "Notes documents must be UTF-8"))?;
    Ok(NotesDocument { content, revision })
}
pub(super) fn publish(
    dir: &Dir,
    name: &str,
    base: &NotesDocument,
    content: &str,
    max: usize,
) -> Result<(), InspectionError> {
    publish_with_source(dir, name, base, content, max, None)
}
pub(super) fn publish_with_source(
    dir: &Dir,
    name: &str,
    base: &NotesDocument,
    content: &str,
    max: usize,
    source: Option<(&str, &NotesDocument)>,
) -> Result<(), InspectionError> {
    if content.len() > max {
        return Err(error(
            "notes_too_large",
            "Notes document exceeds its size limit",
        ));
    }
    let tmp = format!(".{}.tmp", Uuid::new_v4());
    let result = (|| {
        let mut opts = OpenOptions::new();
        opts.write(true)
            .create_new(true)
            .follow(cap_fs_ext::FollowSymlinks::No);
        let mut file = dir.open_with(&tmp, &opts).map_err(outcome_unknown)?;
        #[cfg(unix)]
        rustix::fs::fchmod(&file, rustix::fs::Mode::from_raw_mode(0o600))
            .map_err(outcome_unknown)?;
        file.write_all(content.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(outcome_unknown)?;
        if read(dir, name, max)?.revision != base.revision {
            return Err(error(
                "notes_conflict",
                "Notes document changed before publish; re-read before retrying",
            ));
        }
        if let Some((source_name, expected)) = source {
            if read(dir, source_name, max)?.revision != expected.revision {
                return Err(error(
                    "notes_conflict",
                    "Source decision changed before replacement publish",
                ));
            }
        }
        dir.rename(&tmp, dir, name)
            .and_then(|_| dir.open(".")?.sync_all())
            .map_err(outcome_unknown)
    })();
    if result.is_err() {
        let _ = dir.remove_file(&tmp);
    }
    result
}
pub(super) fn remove(dir: &Dir, name: &str, base: &NotesDocument) -> Result<(), InspectionError> {
    if read(dir, name, 4 * 1024 * 1024)?.revision != base.revision {
        return Err(error(
            "notes_conflict",
            "Notes document changed before removal",
        ));
    }
    dir.remove_file(name)
        .and_then(|_| dir.open(".")?.sync_all())
        .map_err(outcome_unknown)
}
pub(super) struct Lock {
    _file: std::fs::File,
}
pub(super) fn lock(dir: &Dir, name: &str) -> Result<Lock, InspectionError> {
    match dir.symlink_metadata(name) {
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => {
            return Err(error(
                "notes_unsafe_path",
                "Notes lock must be a regular file",
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_error(e)),
    }
    let mut opts = OpenOptions::new();
    opts.read(true)
        .write(true)
        .create(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let file = dir.open_with(name, &opts).map_err(io_error)?.into_std();
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(error(
            "notes_unsafe_path",
            "Opened Notes lock must be a regular file",
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(Lock { _file: file }),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                return Err(error("notes_busy", "Notes lock acquisition timed out"));
            }
            Err(e) => return Err(io_error(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("cockpit-notes-fs-{}", Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn dir(&self) -> Dir {
            root(&self.0, false).unwrap().1
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn notes_publish_refuses_observed_external_edit_and_discards_temp() {
        let f = Fixture::new();
        let dir = f.dir();
        std::fs::write(f.0.join("todos.md"), "original 🧭\r\n").unwrap();
        let base = read(&dir, "todos.md", 100).unwrap();
        std::fs::write(f.0.join("todos.md"), "external edit\r\n").unwrap();
        assert_eq!(
            publish(&dir, "todos.md", &base, "mine\r\n", 100)
                .unwrap_err()
                .code,
            "notes_conflict"
        );
        assert_eq!(
            std::fs::read_to_string(f.0.join("todos.md")).unwrap(),
            "external edit\r\n"
        );
        assert_eq!(entries(&dir, 10).unwrap(), vec!["todos.md"]);
    }
    #[test]
    fn notes_reads_are_utf8_and_size_bounded_and_locks_stay_persistent() {
        let f = Fixture::new();
        let dir = f.dir();
        std::fs::write(f.0.join("bad.md"), [0xff]).unwrap();
        assert_eq!(
            read(&dir, "bad.md", 100).unwrap_err().code,
            "notes_invalid_encoding"
        );
        std::fs::write(f.0.join("large.md"), "12345").unwrap();
        assert_eq!(
            read(&dir, "large.md", 4).unwrap_err().code,
            "notes_too_large"
        );
        let base = read(&dir, "absent.md", 4).unwrap();
        assert_eq!(
            publish(&dir, "absent.md", &base, "12345", 4)
                .unwrap_err()
                .code,
            "notes_too_large"
        );
        assert!(!f.0.join("absent.md").exists());
        let guard = lock(&dir, "persistent.lock").unwrap();
        drop(guard);
        assert!(f.0.join("persistent.lock").is_file());
    }
    #[cfg(unix)]
    #[test]
    fn notes_publish_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let dir = f.dir();
        let base = read(&dir, "scratchpad.md", 100).unwrap();
        publish(&dir, "scratchpad.md", &base, "private", 100).unwrap();
        assert_eq!(
            std::fs::metadata(f.0.join("scratchpad.md"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[cfg(test)]
mod replacement_boundary_tests {
    use super::*;
    #[test]
    fn notes_replacement_refuses_changed_source_without_publishing_successor() {
        let path = std::env::temp_dir().join(format!("cockpit-notes-replace-{}", Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        let (_, dir) = root(&path, false).unwrap();
        std::fs::write(path.join("old.md"), "old").unwrap();
        let source = read(&dir, "old.md", 100).unwrap();
        let absent = read(&dir, "new.md", 100).unwrap();
        std::fs::write(path.join("old.md"), "edited externally").unwrap();
        assert_eq!(
            publish_with_source(
                &dir,
                "new.md",
                &absent,
                "successor",
                100,
                Some(("old.md", &source))
            )
            .unwrap_err()
            .code,
            "notes_conflict"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("old.md")).unwrap(),
            "edited externally"
        );
        assert_eq!(entries(&dir, 10).unwrap(), vec!["old.md"]);
        std::fs::remove_dir_all(path).unwrap();
    }
}
