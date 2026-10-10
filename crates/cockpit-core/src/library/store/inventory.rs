use super::{
    LibraryIndexEntry, MAX_FILE, MAX_TREE_BYTES, MAX_TREE_DEPTH, MAX_TREE_ENTRIES, MarkerFile,
    component, corrupt, error, exists, io_error,
};
use crate::InspectionError;
use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::library::{LibraryConflictFile, LibraryItemKind};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    path::{Component, Path},
};

pub(super) fn safe_file_path(path: &str) -> Result<(), InspectionError> {
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\\')
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(corrupt("invalid item file path"));
    }
    Ok(())
}
fn file_hash(dir: &Dir, path: &str) -> Result<(String, u64), InspectionError> {
    safe_file_path(path)?;
    let path = Path::new(path);
    let mut parent = dir.try_clone().map_err(io_error)?;
    if let Some(p) = path.parent() {
        for c in p.components() {
            parent = parent.open_dir_nofollow(c.as_os_str()).map_err(io_error)?;
        }
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let mut file = match parent.open_with(path.file_name().unwrap(), &options) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(("missing".into(), 0)),
        Err(e) => return Err(io_error(e)),
    };
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE {
        return Err(corrupt("item file is not a bounded regular file"));
    }
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0u8; 32768];
    loop {
        let n = file.read(&mut buffer).map_err(io_error)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        if bytes > MAX_FILE {
            return Err(corrupt("item file grew beyond limit"));
        }
        hash.update(&buffer[..n]);
    }
    Ok((format!("sha256:{:x}", hash.finalize()), bytes))
}
/// Inventory is bounded in depth, entries, per-file bytes and total bytes. Directory
/// handles and regular files are opened no-follow; unsupported nodes fail closed.
pub(super) fn inventory(dir: &Dir) -> Result<Vec<MarkerFile>, InspectionError> {
    fn walk(
        dir: &Dir,
        prefix: &str,
        depth: usize,
        files: &mut Vec<MarkerFile>,
        bytes: &mut u64,
    ) -> Result<(), InspectionError> {
        if depth > MAX_TREE_DEPTH {
            return Err(corrupt("item tree exceeds depth limit"));
        }
        for entry in dir.entries().map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| corrupt("item path is not UTF-8"))?;
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            safe_file_path(&path)?;
            if files.len() >= MAX_TREE_ENTRIES {
                return Err(corrupt("item tree exceeds entry limit"));
            }
            let metadata = dir.symlink_metadata(&name).map_err(io_error)?;
            if metadata.is_dir() {
                files.push(MarkerFile {
                    path: path.clone(),
                    hash: "directory".into(),
                    bytes: 0,
                });
                walk(
                    &dir.open_dir_nofollow(&name).map_err(io_error)?,
                    &path,
                    depth + 1,
                    files,
                    bytes,
                )?;
            } else if metadata.is_file() {
                if metadata.len() > MAX_TREE_BYTES.saturating_sub(*bytes) {
                    return Err(corrupt("item tree exceeds byte limit"));
                }
                let (hash, size) = file_hash(dir, &name)?;
                if hash == "missing" {
                    return Err(error("library_conflict", "Item changed during inventory"));
                }
                *bytes += size;
                if *bytes > MAX_TREE_BYTES {
                    return Err(corrupt("item tree exceeds byte limit"));
                }
                files.push(MarkerFile {
                    path,
                    hash,
                    bytes: size,
                });
            } else {
                return Err(error(
                    "library_conflict",
                    format!("Item contains a symlink or special file: {path}"),
                ));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, "", 0, &mut files, &mut 0)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
pub(super) fn owned_inventory(
    dir: &Dir,
    entry: &LibraryIndexEntry,
) -> Result<Vec<MarkerFile>, InspectionError> {
    if entry.summary.kind == LibraryItemKind::FolderCopy {
        return inventory(dir);
    }
    let mut files = Vec::new();
    if let Some(document) = entry
        .summary
        .document_path
        .as_deref()
        .and_then(|path| path.strip_prefix(&format!("{}/", entry.summary.item_path)))
        .filter(|name| component(name))
    {
        if exists(dir, document)? {
            let (hash, bytes) = file_hash(dir, document)?;
            files.push(MarkerFile {
                path: document.to_owned(),
                hash,
                bytes,
            });
        }
    }
    if exists(dir, crate::library::layout::FILES_DIR)? {
        let files_dir = dir
            .open_dir_nofollow(crate::library::layout::FILES_DIR)
            .map_err(io_error)?;
        files.push(MarkerFile {
            path: crate::library::layout::FILES_DIR.into(),
            hash: "directory".into(),
            bytes: 0,
        });
        files.extend(
            inventory(&files_dir)?
                .into_iter()
                .map(|mut file| {
                    file.path = format!("{}/{}", crate::library::layout::FILES_DIR, file.path);
                    file
                }),
        );
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
pub(super) fn conflicts(
    entry: &LibraryIndexEntry,
    actual: &[MarkerFile],
) -> Vec<LibraryConflictFile> {
    let mut expected = entry.inventory.iter().peekable();
    let mut current = actual.iter().peekable();
    let mut changed = Vec::new();
    while expected.peek().is_some() || current.peek().is_some() {
        let order = match (expected.peek(), current.peek()) {
            (Some(old), Some(now)) => old.path.cmp(&now.path),
            (Some(_), None) => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Greater,
        };
        match order {
            std::cmp::Ordering::Less => changed.push(LibraryConflictFile {
                path: expected.next().unwrap().path.clone(),
                current_hash: "missing".into(),
            }),
            std::cmp::Ordering::Greater => {
                let now = current.next().unwrap();
                changed.push(LibraryConflictFile {
                    path: now.path.clone(),
                    current_hash: now.hash.clone(),
                });
            }
            std::cmp::Ordering::Equal => {
                let old = expected.next().unwrap();
                let now = current.next().unwrap();
                if old != now {
                    changed.push(LibraryConflictFile {
                        path: now.path.clone(),
                        current_hash: now.hash.clone(),
                    });
                }
            }
        }
    }
    changed
}
pub(super) fn check_confirmation(
    actual: &[LibraryConflictFile],
    confirmed: Option<&[LibraryConflictFile]>,
) -> Result<(), InspectionError> {
    let mut confirmed: Vec<_> = confirmed
        .unwrap_or_default()
        .iter()
        .map(|f| (&f.path, &f.current_hash))
        .collect();
    confirmed.sort();
    if !confirmed
        .into_iter()
        .eq(actual.iter().map(|f| (&f.path, &f.current_hash)))
    {
        return Err(error(
            "library_conflict",
            "Library files changed; confirm their current hashes before replacing",
        ));
    }
    Ok(())
}

pub(super) fn verify_entry(dir: &Dir, entry: &LibraryIndexEntry) -> Result<(), InspectionError> {
    if owned_inventory(dir, entry)? != entry.inventory {
        return Err(corrupt(
            "published owned-entry inventory differs from the journaled entry",
        ));
    }
    Ok(())
}
pub(super) fn owned_roots(entry: &LibraryIndexEntry) -> Vec<String> {
    let mut roots = Vec::new();
    if entry.summary.kind == LibraryItemKind::FolderCopy {
        return roots;
    }
    if let Some(name) = entry
        .summary
        .document_path
        .as_deref()
        .and_then(|path| path.strip_prefix(&format!("{}/", entry.summary.item_path)))
        .filter(|name| component(name))
    {
        roots.push(name.to_owned());
    }
    if entry
        .inventory
        .iter()
        .any(|file| file.path == crate::library::layout::FILES_DIR || file.path.starts_with("_files/"))
    {
        roots.push(crate::library::layout::FILES_DIR.to_owned());
    }
    roots
}
