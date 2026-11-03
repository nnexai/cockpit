pub(crate) const MAX_RASTER_PIXELS: u64 = 16_000_000;
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

pub(crate) struct RasterInfo {
    pub mime_type: &'static str,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub(crate) enum RasterRefusal {
    TypeRefused,
    FormatRefused(&'static str),
    HeaderInvalid(&'static str),
    Dimensions,
    DecodedBytes,
}

pub(crate) fn validate_raster(bytes: &[u8]) -> Result<RasterInfo, RasterRefusal> {
    let result = if bytes.starts_with(&PNG_SIGNATURE) {
        parse_png_header(bytes).map(|(width, height)| ("image/png", width, height))
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        parse_jpeg_header(bytes).map(|(width, height)| ("image/jpeg", width, height))
    } else {
        return Err(RasterRefusal::TypeRefused);
    }?;
    validate_dimensions(result.1, result.2)?;
    Ok(RasterInfo { mime_type: result.0, width: result.1, height: result.2 })
}


fn parse_png_header(bytes: &[u8]) -> Result<(u32, u32), RasterRefusal> {
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
                return Err(RasterRefusal::FormatRefused("only static non-interlaced 8-bit RGB or RGBA PNG images are supported"));
            }
            validate_dimensions(width, height)?;
            validate_decoded_bytes(width, height, if color_type == 2 { 3 } else { 4 })?;
            dimensions = Some((width, height));
        } else {
            if dimensions.is_none() {
                return Err(invalid_png());
            }
            if chunk_type == *b"acTL" || chunk_type == *b"fcTL" || chunk_type == *b"fdAT" {
                return Err(RasterRefusal::FormatRefused("animated PNG images are not supported"));
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
                if !data.is_empty() || !saw_idat || chunk_end != bytes.len() {
                    return Err(invalid_png());
                }
                return dimensions.ok_or_else(invalid_png);
            }
        }
        offset = chunk_end;
    }
    Err(invalid_png())
}

fn parse_jpeg_header(bytes: &[u8]) -> Result<(u32, u32), RasterRefusal> {
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
                return Err(RasterRefusal::FormatRefused("only 8-bit JPEG images are supported"));
            }
            let height = u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]) as u32;
            let components = bytes[offset + 7] as u32;
            if !(1..=4).contains(&components) {
                return Err(RasterRefusal::FormatRefused("JPEG component count exceeds the Context media limit"));
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
    Err(RasterRefusal::HeaderInvalid("JPEG does not contain a supported frame header"))
}

fn is_start_of_frame(marker: u8) -> bool {
    matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf)
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), RasterRefusal> {
    if width == 0 || height == 0 || (width as u64).saturating_mul(height as u64) > MAX_RASTER_PIXELS
    {
        return Err(RasterRefusal::Dimensions);
    }
    Ok(())
}

fn validate_decoded_bytes(width: u32, height: u32, channels: u32) -> Result<(), RasterRefusal> {
    let decoded = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(channels as u64)
        .saturating_add(height as u64);
    if decoded > MAX_DECODED_BYTES {
        return Err(RasterRefusal::DecodedBytes);
    }
    Ok(())
}

fn invalid_png() -> RasterRefusal {
    RasterRefusal::HeaderInvalid("PNG does not contain complete valid chunks")
}

pub(crate) fn png_crc(bytes: &[u8]) -> u32 {
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
