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

const MAX_RASTER_PIXELS: u64 = 16_000_000;
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;
const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

impl ContextService {
    /// Read one host-authorized, bounded raster image from the current Context
    /// companion or verified Folder root. This accepts no URL-like input and
    /// does not expose a path to the browser.
    pub async fn media(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ContextMediaRequest,
    ) -> Result<ContextMedia, InspectionError> {
        let authorized = self
            .authorize_media_root(session_id, pane_id, &request.binding_id, &request.root_id)
            .await?;
        let max_bytes = self.configuration.limits.context_preview_bytes as usize;
        read_media(authorized, request, max_bytes)
    }
}

fn read_media(
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
    if let Some(expected) = request.expected_revision.as_deref() {
        if expected != revision {
            return Err(InspectionError::new(
                "context_stale_revision",
                "context media changed since it was listed",
            ));
        }
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
    let result = if bytes.starts_with(&PNG_SIGNATURE) {
        parse_png_header(bytes).map(|(width, height)| ("image/png", width, height))
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        parse_jpeg_header(bytes).map(|(width, height)| ("image/jpeg", width, height))
    } else {
        return Err(InspectionError::new(
            "context_media_type_refused",
            "only PNG and JPEG Context media are supported",
        ));
    }?;
    validate_dimensions(result.1, result.2)?;
    Ok(result)
}

fn parse_png_header(bytes: &[u8]) -> Result<(u32, u32), InspectionError> {
    let mut offset = PNG_SIGNATURE.len();
    let mut dimensions = None;
    let mut saw_idat = false;
    let mut idat_closed = false;

    while offset < bytes.len() {
        let header_end = offset.checked_add(8).ok_or_else(invalid_png)?;
        if header_end > bytes.len() {
            return Err(invalid_png());
        }
        let length = u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("PNG chunk length"),
        ) as usize;
        let chunk_type: [u8; 4] = bytes[offset + 4..header_end]
            .try_into()
            .expect("PNG chunk type");
        if !chunk_type.iter().all(u8::is_ascii_alphabetic) {
            return Err(invalid_png());
        }
        let data_start = header_end;
        let data_end = data_start.checked_add(length).ok_or_else(invalid_png)?;
        let chunk_end = data_end.checked_add(4).ok_or_else(invalid_png)?;
        if chunk_end > bytes.len() {
            return Err(invalid_png());
        }
        let expected_crc = u32::from_be_bytes(
            bytes[data_end..chunk_end]
                .try_into()
                .expect("PNG chunk CRC"),
        );
        if png_crc(&bytes[offset + 4..data_end]) != expected_crc {
            return Err(invalid_png());
        }
        let data = &bytes[data_start..data_end];
        if chunk_type == *b"IHDR" {
            if dimensions.is_some() || offset != PNG_SIGNATURE.len() || data.len() != 13 {
                return Err(invalid_png());
            }
            let width = u32::from_be_bytes(data[0..4].try_into().expect("PNG width"));
            let height = u32::from_be_bytes(data[4..8].try_into().expect("PNG height"));
            let bit_depth = data[8];
            let color_type = data[9];
            if bit_depth != 8
                || !matches!(color_type, 2 | 6)
                || data[10] != 0
                || data[11] != 0
                || data[12] != 0
            {
                return Err(InspectionError::new(
                    "context_media_format_refused",
                    "only static non-interlaced 8-bit RGB or RGBA PNG images are supported",
                ));
            }
            validate_dimensions(width, height)?;
            validate_decoded_bytes(width, height, if color_type == 2 { 3 } else { 4 })?;
            dimensions = Some((width, height));
        } else {
            if dimensions.is_none() {
                return Err(invalid_png());
            }
            if chunk_type == *b"acTL" || chunk_type == *b"fcTL" || chunk_type == *b"fdAT" {
                return Err(InspectionError::new(
                    "context_media_format_refused",
                    "animated PNG images are not supported",
                ));
            }
            if chunk_type == *b"IDAT" {
                if idat_closed {
                    return Err(invalid_png());
                }
                saw_idat = true;
            } else if saw_idat {
                idat_closed = true;
            }
            if chunk_type == *b"IEND" {
                if data.len() != 0 || !saw_idat || chunk_end != bytes.len() {
                    return Err(invalid_png());
                }
                return dimensions.ok_or_else(invalid_png);
            }
        }
        offset = chunk_end;
    }
    Err(invalid_png())
}

fn parse_jpeg_header(bytes: &[u8]) -> Result<(u32, u32), InspectionError> {
    let mut offset = 2usize;
    while offset < bytes.len() {
        if bytes[offset] != 0xff {
            break;
        }
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        if offset >= bytes.len() {
            break;
        }
        let marker = bytes[offset];
        offset += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if offset + 2 > bytes.len() {
            break;
        }
        let length = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        if length < 2 || offset + length > bytes.len() {
            break;
        }
        if is_start_of_frame(marker) {
            if length < 8 {
                break;
            }
            if bytes[offset + 2] != 8 {
                return Err(InspectionError::new(
                    "context_media_format_refused",
                    "only 8-bit JPEG images are supported",
                ));
            }
            let height = u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]) as u32;
            let components = bytes[offset + 7] as u32;
            if !(1..=4).contains(&components) {
                return Err(InspectionError::new(
                    "context_media_format_refused",
                    "JPEG component count exceeds the Context media limit",
                ));
            }
            if length != 8 + components as usize * 3 {
                break;
            }
            validate_dimensions(width, height)?;
            validate_decoded_bytes(width, height, components)?;
            return Ok((width, height));
        }
        offset += length;
    }
    Err(InspectionError::new(
        "context_media_header_invalid",
        "JPEG does not contain a supported frame header",
    ))
}

fn is_start_of_frame(marker: u8) -> bool {
    matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf)
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), InspectionError> {
    if width == 0 || height == 0 || (width as u64).saturating_mul(height as u64) > MAX_RASTER_PIXELS
    {
        return Err(InspectionError::new(
            "context_media_dimensions",
            "image dimensions exceed the Context media pixel limit",
        ));
    }
    Ok(())
}

fn validate_decoded_bytes(width: u32, height: u32, channels: u32) -> Result<(), InspectionError> {
    let decoded = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(channels as u64)
        .saturating_add(height as u64);
    if decoded > MAX_DECODED_BYTES {
        return Err(InspectionError::new(
            "context_media_decoded_bytes",
            "image exceeds the Context media decoded-byte limit",
        ));
    }
    Ok(())
}

fn invalid_png() -> InspectionError {
    InspectionError::new(
        "context_media_header_invalid",
        "PNG does not contain complete valid chunks",
    )
}

fn png_crc(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
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
        assert_eq!(
            raster_header(b"<svg xmlns='http://www.w3.org/2000/svg'>")
                .expect_err("SVG")
                .code,
            "context_media_type_refused"
        );
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
