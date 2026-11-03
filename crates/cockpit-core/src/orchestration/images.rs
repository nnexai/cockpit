use std::{io::{ErrorKind, Read, Write}, path::{Path, PathBuf}};

use cap_fs_ext::{OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use cockpit_protocol::orchestration::MessageImage;
use fs2::FileExt;
use sha2::{Digest, Sha256};

use crate::{InspectionError, raster::{RasterRefusal, validate_raster}};

pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_MESSAGE_IMAGES: usize = 4;
const MAX_STORE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_STORE_FILES: usize = 4096;
const LOCK: &str = ".lock";
const TEMP_PREFIX: &str = ".upload-";

fn unavailable(error: impl std::fmt::Display) -> InspectionError {
    InspectionError::new("image_store_unavailable", format!("image store unavailable: {error}"))
}

fn corrupt(message: impl std::fmt::Display) -> InspectionError {
    InspectionError::new("image_corrupt", format!("stored image is corrupt: {message}"))
}

pub(crate) fn validate_image_id(id: &str) -> Result<(), InspectionError> {
    if id.len() != 71 || !id.starts_with("sha256-") || !id.as_bytes()[7..].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c)) {
        return Err(InspectionError::new("image_id_invalid", "invalid image ID"));
    }
    Ok(())
}

fn metadata_for(bytes: &[u8]) -> Result<MessageImage, InspectionError> {
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(InspectionError::new("image_too_large", "image exceeds the byte limit"));
    }
    let info = validate_raster(bytes).map_err(|error| {
        let (code, message) = match error {
            RasterRefusal::TypeRefused => ("image_type_refused", "only PNG and JPEG images are supported"),
            RasterRefusal::FormatRefused(message) => ("image_format_refused", message),
            RasterRefusal::HeaderInvalid(message) => ("image_header_invalid", message),
            RasterRefusal::Dimensions => ("image_dimensions", "image dimensions exceed the pixel limit"),
            RasterRefusal::DecodedBytes => ("image_decoded_bytes", "image exceeds the decoded-byte limit"),
        };
        InspectionError::new(code, message)
    })?;
    if info.mime_type == "image/jpeg" && !bytes.ends_with(&[0xff, 0xd9]) {
        return Err(InspectionError::new("image_format_refused", "JPEG must end with its EOI marker"));
    }
    Ok(MessageImage {
        image_id: format!("sha256-{:x}", Sha256::digest(bytes)),
        mime_type: info.mime_type.to_owned(), width: info.width, height: info.height,
        bytes: bytes.len() as u64,
    })
}

fn directory(base: &Path, create: bool) -> Result<Dir, InspectionError> {
    if create { std::fs::create_dir_all(base).map_err(unavailable)?; }
    let root = Dir::open_ambient_dir(base, cap_std::ambient_authority()).map_err(|e| {
        if e.kind() == ErrorKind::NotFound { InspectionError::new("image_not_found", "image not found") } else { unavailable(e) }
    })?;
    if create {
        let mut builder = cap_std::fs::DirBuilder::new();
        #[cfg(unix)] { use cap_std::fs::DirBuilderExt; builder.mode(0o700); }
        match root.create_dir_with("images", &builder) {
            Ok(()) => (),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => (),
            Err(e) => return Err(unavailable(e)),
        }
    }
    let meta = root.symlink_metadata("images").map_err(|e| {
        if e.kind() == ErrorKind::NotFound { InspectionError::new("image_not_found", "image not found") } else { unavailable(e) }
    })?;
    if !meta.is_dir() || meta.file_type().is_symlink() { return Err(unavailable("unsafe image directory")); }
    root.open_dir("images").map_err(unavailable)
}

fn regular(meta: &Metadata) -> bool {
    if !meta.is_file() || meta.file_type().is_symlink() { return false; }
    #[cfg(unix)] { use cap_std::fs::MetadataExt; if meta.nlink() != 1 { return false; } }
    true
}

fn filename(session_id: &str, id: &str, extension: &str) -> String {
    format!("{:x}-{}.{}", Sha256::digest(session_id.as_bytes()), &id[7..], extension)
}

fn open_file(dir: &Dir, name: &str) -> Result<Vec<u8>, InspectionError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(cap_fs_ext::FollowSymlinks::No).nonblock(true);
    let file = dir.open_with(name, &options).map_err(|e| match e.kind() {
        ErrorKind::NotFound => InspectionError::new("image_not_found", "image not found"),
        _ => corrupt(e),
    })?;
    let meta = file.metadata().map_err(corrupt)?;
    if !regular(&meta) || meta.len() > MAX_IMAGE_BYTES { return Err(corrupt("unsafe file or oversized image")); }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes).map_err(corrupt)?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES { return Err(corrupt("image grew while reading")); }
    Ok(bytes)
}

fn read_in(dir: &Dir, session_id: &str, id: &str) -> Result<(MessageImage, Vec<u8>, String), InspectionError> {
    validate_image_id(id)?;
    for ext in ["png", "jpg"] {
        let name = filename(session_id, id, ext);
        match open_file(dir, &name) {
            Ok(bytes) => {
                let image = metadata_for(&bytes).map_err(corrupt)?;
                if image.image_id != id || (ext == "png") != (image.mime_type == "image/png") { return Err(corrupt("hash or type mismatch")); }
                return Ok((image, bytes, name));
            }
            Err(e) if e.code == "image_not_found" => (),
            Err(e) => return Err(e),
        }
    }
    Err(InspectionError::new("image_not_found", "image not found"))
}

pub(crate) fn read_bytes(base: &Path, session_id: &str, id: &str) -> Result<(MessageImage, Vec<u8>), InspectionError> {
    validate_image_id(id)?;
    let (image, bytes, _) = read_in(&directory(base, false)?, session_id, id)?;
    Ok((image, bytes))
}

pub(crate) fn verify(base: &Path, session_id: &str, id: &str) -> Result<(MessageImage, PathBuf), InspectionError> {
    validate_image_id(id)?;
    let (image, _, name) = read_in(&directory(base, false)?, session_id, id)?;
    let path = base.join("images").join(name);
    let path = std::fs::canonicalize(path).map_err(unavailable)?;
    Ok((image, path))
}

pub(crate) fn put(base: &Path, session_id: &str, bytes: &[u8]) -> Result<MessageImage, InspectionError> {
    put_with_limit(base, session_id, bytes, MAX_STORE_BYTES)
}

fn put_with_limit(base: &Path, session_id: &str, bytes: &[u8], limit: u64) -> Result<MessageImage, InspectionError> {
    let image = metadata_for(bytes)?;
    let dir = directory(base, true)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).follow(cap_fs_ext::FollowSymlinks::No).nonblock(true);
    #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.mode(0o600); }
    let lock = dir.open_with(LOCK, &options).map_err(unavailable)?;
    if !regular(&lock.metadata().map_err(unavailable)?) { return Err(unavailable("unsafe lock file")); }
    let lock = lock.into_std();
    FileExt::lock_exclusive(&lock).map_err(unavailable)?;
    // The lock serializes accounting and publication across processes. Interrupted
    // uploads leave at most a bounded temporary file, reclaimed on the next put.
    let mut used = 0u64;
    let mut files = 0usize;
    for entry in dir.entries().map_err(unavailable)? {
        let entry = entry.map_err(unavailable)?;
        let name = entry.file_name();
        if name == LOCK { continue; }
        if name.to_string_lossy().starts_with(TEMP_PREFIX) {
            dir.remove_file(&name).map_err(unavailable)?;
            continue;
        }
        let meta = dir.symlink_metadata(&name).map_err(unavailable)?;
        if !regular(&meta) { return Err(unavailable("unsafe entry in image store")); }
        files = files.checked_add(1).ok_or_else(|| unavailable("store file count overflow"))?;
        used = used.checked_add(meta.len()).ok_or_else(|| unavailable("store size overflow"))?;
    }
    match read_in(&dir, session_id, &image.image_id) {
        Ok((stored, _, _)) => return Ok(stored),
        Err(e) if e.code == "image_not_found" => (),
        Err(e) => return Err(e),
    }
    if files >= MAX_STORE_FILES || used.checked_add(image.bytes).is_none_or(|size| size > limit) {
        return Err(InspectionError::new("image_store_full", "image store byte limit reached"));
    }
    let temp = format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).follow(cap_fs_ext::FollowSymlinks::No);
    #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; options.mode(0o600); }
    let mut file = dir.open_with(&temp, &options).map_err(unavailable)?;
    let name = filename(session_id, &image.image_id, if image.mime_type == "image/png" { "png" } else { "jpg" });
    let result = (|| {
        file.write_all(bytes).map_err(unavailable)?;
        file.sync_all().map_err(unavailable)?;
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
        rustix::fs::renameat_with(&dir, &temp, &dir, &name, rustix::fs::RenameFlags::NOREPLACE)
            .map_err(unavailable)?;
        #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
        dir.hard_link(&temp, &dir, &name).map_err(unavailable)?;
        Ok(())
    })();
    let cleanup = match dir.remove_file(&temp) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(unavailable(e)),
    };
    result?;
    cleanup?;
    dir.open(".").and_then(|directory| directory.sync_all()).map_err(unavailable)?;
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, sync::{Arc, Barrier}, thread};

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("cockpit-images-{}", uuid::Uuid::new_v4()))
    }

    fn jpeg(width: u16) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 1];
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[1, 1, 0x11, 0, 0xff, 0xd9]);
        bytes
    }

    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let start = out.len() - data.len() - 4;
        out.extend_from_slice(&crate::raster::png_crc(&out[start..]).to_be_bytes());
    }

    fn png(animated: bool) -> Vec<u8> {
        let mut data = crate::raster::PNG_SIGNATURE.to_vec();
        chunk(&mut data, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
        if animated { chunk(&mut data, b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]); }
        chunk(&mut data, b"IDAT", &[0x78, 0x01, 0x01, 4, 0, 0xfb, 0xff, 0, 0, 0, 0, 0, 1, 0, 1]);
        chunk(&mut data, b"IEND", &[]);
        data
    }

    #[test]
    fn round_trip_idempotence_and_session_isolation() {
        let root = root();
        let bytes = png(false);
        let image = put(&root, "first", &bytes).unwrap();
        assert_eq!(put(&root, "first", &bytes).unwrap(), image);
        assert_eq!(read_bytes(&root, "first", &image.image_id).unwrap().1, bytes);
        assert!(verify(&root, "first", &image.image_id).unwrap().1.is_absolute());
        assert_eq!(verify(&root, "second", &image.image_id).unwrap_err().code, "image_not_found");
        assert_eq!(verify(&root, "first", "../bad").unwrap_err().code, "image_id_invalid");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_invalid_and_oversized_data() {
        let root = root();
        for bytes in [b"<svg/>".as_slice(), b"<html/>".as_slice()] {
            assert_eq!(put(&root, "s", bytes).unwrap_err().code, "image_type_refused");
        }
        assert_eq!(put(&root, "s", &png(true)).unwrap_err().code, "image_format_refused");
        let mut trailing = jpeg(1);
        trailing.extend_from_slice(b"payload");
        assert_eq!(put(&root, "s", &trailing).unwrap_err().code, "image_format_refused");
        assert_eq!(put(&root, "s", &vec![0; MAX_IMAGE_BYTES as usize + 1]).unwrap_err().code, "image_too_large");
    }

    #[test]
    fn detects_hash_mismatch() {
        let root = root();
        let image = put(&root, "s", &jpeg(1)).unwrap();
        let path = root.join("images").join(filename("s", &image.image_id, "jpg"));
        fs::write(path, jpeg(2)).unwrap();
        assert_eq!(verify(&root, "s", &image.image_id).unwrap_err().code, "image_corrupt");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_hardlink_and_special_file() {
        use std::os::unix::{fs::symlink, net::UnixListener};
        let root = root();
        let image = put(&root, "s", &jpeg(1)).unwrap();
        let path = root.join("images").join(filename("s", &image.image_id, "jpg"));
        let outside = root.join("outside");
        fs::write(&outside, jpeg(1)).unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&outside, &path).unwrap();
        assert_eq!(verify(&root, "s", &image.image_id).unwrap_err().code, "image_corrupt");
        fs::remove_file(&path).unwrap();
        fs::hard_link(&outside, &path).unwrap();
        assert_eq!(verify(&root, "s", &image.image_id).unwrap_err().code, "image_corrupt");
        fs::remove_file(&path).unwrap();
        // Unix-domain socket addresses have a much shorter limit than filesystem
        // paths; bind at a short name and move the socket inode into the store.
        let socket = root.join("socket");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::rename(&socket, &path).unwrap();
        assert_eq!(verify(&root, "s", &image.image_id).unwrap_err().code, "image_corrupt");
        drop(listener);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_uploads_share_one_global_budget_and_recover_temp_files() {
        let root = root();
        let bytes = jpeg(1);
        let limit = bytes.len() as u64;
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (1..=8).map(|n| {
            let root = root.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                put_with_limit(&root, &format!("session-{n}"), &jpeg(n), limit)
            })
        }).collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert!(results.iter().filter_map(|r| r.as_ref().err()).all(|e| e.code == "image_store_full"));
        let temp = root.join("images").join(format!("{TEMP_PREFIX}interrupted"));
        fs::write(&temp, b"partial").unwrap();
        let winner = results.iter().find_map(|r| r.as_ref().ok()).unwrap();
        let session = (1..=8).map(|n| format!("session-{n}")).find(|s| verify(&root, s, &winner.image_id).is_ok()).unwrap();
        put_with_limit(&root, &session, &bytes_for(&root, &session, &winner.image_id), limit).unwrap();
        assert!(!temp.exists());
        fs::remove_dir_all(root).unwrap();
    }

    fn bytes_for(root: &Path, session: &str, id: &str) -> Vec<u8> {
        read_bytes(root, session, id).unwrap().1
    }
}
