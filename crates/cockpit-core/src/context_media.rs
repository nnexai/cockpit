use std::io::{ErrorKind, Read};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use cap_fs_ext::{OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use cockpit_protocol::context_media::{ContextMedia, ContextMediaRequest};
use sha2::{Digest, Sha256};

use crate::{
    InspectionError,
    context::{AuthorizedRoot, ContextService, metadata_revision},
};

#[cfg(test)]
use crate::raster::{MAX_RASTER_PIXELS, PNG_SIGNATURE, png_crc};
use crate::raster::{RasterRefusal, validate_raster};

impl ContextService {
    /// Read one host-authorized, bounded raster image from a Files viewer's
    /// pinned Library, Repository, or verified Folder root.
    /// It accepts no URL-like input and exposes no path to the browser.
    pub async fn media(
        &self,
        session_id: &str,
        viewer_id: &str,
        request: &ContextMediaRequest,
    ) -> Result<ContextMedia, InspectionError> {
        let authorized = self
            .authorize_media_root(session_id, viewer_id, &request.binding_id, &request.root_id)
            .await?;
        let max_bytes = self.configuration.limits.context_preview_bytes as usize;
        read_media(authorized, request, max_bytes)
    }
}

pub(crate) fn read_media(
    authorized: AuthorizedRoot,
    request: &ContextMediaRequest,
    max_bytes: usize,
) -> Result<ContextMedia, InspectionError> {
    let relative = authorized.relative_path(&request.path)?;
    let (parent, leaf) = authorized.resolve_parent(&relative)?;
    let before = parent
        .symlink_metadata(&leaf)
        .map_err(|error| media_metadata_error(&error))?;
    let revision = metadata_revision(&before);
    if let Some(expected) = request.expected_revision.as_deref()
        && expected != revision {
            return Err(InspectionError::new(
                "context_stale_revision",
                "context media changed since it was listed",
            ));
        }
    validate_file(&before)?;

    if before.len() > max_bytes as u64 {
        return Err(InspectionError::new(
            "context_media_bytes",
            "image exceeds the configured preview byte limit",
        ));
    }

    authorized.revalidate()?;
    let bytes = open_and_read(&parent, &leaf, &revision, max_bytes)?;
    let after = parent.symlink_metadata(&leaf).map_err(|error| {
        InspectionError::new(
            "context_changed_during_read",
            format!("cannot recheck context media: {error}"),
        )
    })?;
    if metadata_revision(&after) != revision {
        return Err(InspectionError::new(
            "context_changed_during_read",
            "context media changed while reading",
        ));
    }
    authorized.revalidate()?;

    let (mime_type, width, height) = raster_header(&bytes)?;
    let content_hash = hash_bytes(&bytes);
    Ok(ContextMedia {
        binding_id: request.binding_id.clone(),
        root_id: authorized.root_id().to_owned(),
        path: relative.to_string_lossy().into_owned(),
        revision,
        content_hash,
        bytes: bytes.len() as u64,
        mime_type: mime_type.to_owned(),
        width,
        height,
        data_base64: BASE64.encode(bytes),
    })
}

fn validate_file(metadata: &Metadata) -> Result<(), InspectionError> {
    if metadata.file_type().is_symlink() {
        return Err(InspectionError::new(
            "context_symlink_refused",
            "symbolic links are not followed",
        ));
    }
    if !metadata.is_file() {
        return Err(InspectionError::new(
            "context_special_file_refused",
            "special files cannot be opened",
        ));
    }
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return Err(InspectionError::new(
                "context_hardlink_refused",
                "hard-linked files are not exposed as Context media",
            ));
        }
    }
    Ok(())
}

fn open_and_read(
    parent: &Dir,
    leaf: &std::path::Path,
    revision: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, InspectionError> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let file = parent.open_with(leaf, &options).map_err(|error| {
        let code = if error.kind() == ErrorKind::WouldBlock || is_symlink_open_error(&error) {
            "context_special_file_refused"
        } else if error.kind() == ErrorKind::NotFound {
            "context_file_missing"
        } else {
            "context_file_unavailable"
        };
        InspectionError::new(code, format!("cannot open context media: {error}"))
    })?;
    let opened = file.metadata().map_err(|error| {
        InspectionError::new(
            "context_file_unavailable",
            format!("cannot stat context media: {error}"),
        )
    })?;
    validate_file(&opened)?;
    if metadata_revision(&opened) != revision {
        return Err(InspectionError::new(
            "context_changed_during_read",
            "context media changed while opening",
        ));
    }
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024));
    file.take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            InspectionError::new(
                "context_file_unavailable",
                format!("cannot read context media: {error}"),
            )
        })?;
    if bytes.len() > max_bytes {
        return Err(InspectionError::new(
            "context_media_bytes",
            "image grew beyond the configured preview byte limit",
        ));
    }
    Ok(bytes)
}

fn raster_header(bytes: &[u8]) -> Result<(&'static str, u32, u32), InspectionError> {
    let info = validate_raster(bytes).map_err(|refusal| {
        let (code, message) = match refusal {
            RasterRefusal::TypeRefused => ("context_media_type_refused", "only PNG and JPEG Context media are supported"),
            RasterRefusal::FormatRefused(message) => ("context_media_format_refused", message),
            RasterRefusal::HeaderInvalid(message) => ("context_media_header_invalid", message),
            RasterRefusal::Dimensions => ("context_media_dimensions", "image dimensions exceed the Context media pixel limit"),
            RasterRefusal::DecodedBytes => ("context_media_decoded_bytes", "image exceeds the Context media decoded-byte limit"),
        };
        InspectionError::new(code, message)
    })?;
    Ok((info.mime_type, info.width, info.height))
}


fn media_metadata_error(error: &std::io::Error) -> InspectionError {
    InspectionError::new(
        if error.kind() == ErrorKind::NotFound {
            "context_file_missing"
        } else {
            "context_file_unavailable"
        },
        format!("cannot inspect context media: {error}"),
    )
}

fn is_symlink_open_error(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(nix::libc::ELOOP)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use cap_std::fs::Dir;

    use super::{MAX_RASTER_PIXELS, PNG_SIGNATURE, open_and_read, raster_header, validate_file};

    fn temp_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "cockpit-context-media-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        root
    }

    fn png_chunk(bytes: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
        bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&kind);
        bytes.extend_from_slice(data);
        let start = bytes.len() - data.len() - kind.len();
        bytes.extend_from_slice(&super::png_crc(&bytes[start..]).to_be_bytes());
    }

    fn png_prefix(width: u32, height: u32, bit_depth: u8, color_type: u8) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[bit_depth, color_type, 0, 0, 0]);
        png_chunk(&mut bytes, *b"IHDR", &ihdr);
        bytes
    }

    fn zlib_stored_zeroes(length: usize) -> Vec<u8> {
        assert!(
            length <= u16::MAX as usize,
            "fixture fits one stored DEFLATE block"
        );
        let mut bytes = vec![0x78, 0x01, 0x01];
        bytes.extend_from_slice(&(length as u16).to_le_bytes());
        bytes.extend_from_slice(&(!(length as u16)).to_le_bytes());
        bytes.resize(bytes.len() + length, 0);
        bytes.extend_from_slice(&[0, 1, 0, 1]);
        bytes
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = png_prefix(width, height, 8, 2);
        let raw_bytes = (width as usize * 3 + 1) * height as usize;
        png_chunk(&mut bytes, *b"IDAT", &zlib_stored_zeroes(raw_bytes));
        png_chunk(&mut bytes, *b"IEND", &[]);
        bytes
    }

    fn jpeg(width: u16, height: u16, precision: u8, components: u8) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xc0];
        let length = 8u16 + u16::from(components) * 3;
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.push(precision);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.push(components);
        for component in 1..=components {
            bytes.extend_from_slice(&[component, 0x11, 0]);
        }
        bytes.extend_from_slice(&[0xff, 0xd9]);
        bytes
    }


    #[test]
    fn recognizes_real_png_and_jpeg_headers() {
        assert_eq!(
            raster_header(&png(64, 32)).expect("PNG"),
            ("image/png", 64, 32)
        );
        assert_eq!(
            raster_header(&jpeg(32, 16, 8, 3)).expect("JPEG"),
            ("image/jpeg", 32, 16)
        );
    }

    #[test]
    fn refuses_active_and_malformed_media() {
        for svg in [
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><script>alert(1)</script></svg>"#.as_slice(),
            b"<?xml version='1.0'?>\n<svg viewBox='0 0 120 40'></svg>".as_slice(),
        ] {
            assert_eq!(raster_header(svg).expect_err("SVG").code, "context_media_type_refused");
        }
        assert_eq!(
            raster_header(b"<!doctype html><img>")
                .expect_err("HTML")
                .code,
            "context_media_type_refused"
        );
        assert_eq!(
            raster_header(&[137, 80, 78, 71, 13, 10, 26, 10])
                .expect_err("short PNG")
                .code,
            "context_media_header_invalid"
        );
    }

    #[test]
    fn refuses_animated_or_high_depth_images() {
        let mut animated = png_prefix(1, 1, 8, 2);
        png_chunk(&mut animated, *b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]);
        png_chunk(&mut animated, *b"IDAT", &zlib_stored_zeroes(4));
        png_chunk(&mut animated, *b"IEND", &[]);
        assert_eq!(
            raster_header(&animated).expect_err("APNG").code,
            "context_media_format_refused"
        );
        assert_eq!(
            raster_header(&png_prefix(1, 1, 16, 6))
                .expect_err("16-bit RGBA")
                .code,
            "context_media_format_refused"
        );
        assert_eq!(
            raster_header(&jpeg(1, 1, 12, 3))
                .expect_err("12-bit JPEG")
                .code,
            "context_media_format_refused"
        );
        assert_eq!(
            raster_header(&jpeg(1, 1, 8, 5))
                .expect_err("five-component JPEG")
                .code,
            "context_media_format_refused"
        );
    }

    #[test]
    fn refuses_zero_and_excessive_pixel_counts() {
        assert_eq!(
            raster_header(&png_prefix(0, 1, 8, 2))
                .expect_err("zero width")
                .code,
            "context_media_dimensions"
        );
        let side = ((MAX_RASTER_PIXELS as f64).sqrt() as u32) + 1;
        assert_eq!(
            raster_header(&png_prefix(side, side, 8, 2))
                .expect_err("too many pixels")
                .code,
            "context_media_dimensions"
        );
    }

    #[test]
    fn descriptor_read_is_bounded_and_regular_files_only() {
        let root = temp_root("bounded-read");
        let image = root.join("image.png");
        fs::write(&image, png(1, 1)).expect("write image");
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        let metadata = dir.symlink_metadata("image.png").expect("metadata");
        validate_file(&metadata).expect("regular fixture");
        let revision = crate::context::metadata_revision(&metadata);
        assert_eq!(
            open_and_read(&dir, Path::new("image.png"), &revision, 8)
                .expect_err("byte cap")
                .code,
            "context_media_bytes"
        );
        assert_eq!(
            open_and_read(&dir, Path::new("image.png"), &revision, 4096).expect("read"),
            png(1, 1)
        );
        fs::remove_dir_all(root).expect("cleanup fixture");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_and_hardlink_escape_candidates() {
        use std::os::unix::fs::symlink;

        let root = temp_root("links");
        let outside = temp_root("outside");
        let target = outside.join("outside.png");
        fs::write(&target, png(1, 1)).expect("write outside image");
        symlink(&target, root.join("outside.png")).expect("create symlink");
        let linked = root.join("linked.png");
        fs::hard_link(&target, &linked).expect("create hardlink");
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        assert_eq!(
            validate_file(
                &dir.symlink_metadata("outside.png")
                    .expect("symlink metadata")
            )
            .expect_err("symlink")
            .code,
            "context_symlink_refused"
        );
        assert_eq!(
            validate_file(
                &dir.symlink_metadata("linked.png")
                    .expect("hardlink metadata")
            )
            .expect_err("hardlink")
            .code,
            "context_hardlink_refused"
        );
        fs::remove_dir_all(root).expect("cleanup root");
        fs::remove_dir_all(outside).expect("cleanup outside");
    }
}
