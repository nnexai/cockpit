//! Bounded-text checks and provider asset validation.

use super::{FrontmatterValue, MAX_ASSET_BYTES, MAX_METADATA_BYTES, MAX_URL_BYTES, SourceAsset};
use crate::InspectionError;

pub(super) fn validate_asset(asset: &SourceAsset) -> Result<(), InspectionError> {
    let diagnostics_valid = asset.diagnostics.len() <= 32
        && asset.diagnostics.iter().all(|diagnostic| {
            bounded_text(&diagnostic.code, 128)
                && bounded_text(&diagnostic.message, MAX_METADATA_BYTES)
                && diagnostic
                    .path
                    .as_deref()
                    .is_none_or(|path| bounded_text(path, MAX_URL_BYTES))
        });
    if !bounded_text(&asset.source.provider_id, 128)
        || !valid_extended_metadata(asset)
        || !bounded_text(&asset.source.provider_instance, 512)
        || !identifier(&asset.source.resource_type, 64)
        || !bounded_text(&asset.source.canonical_id, 512)
        || !bounded_text(&asset.title, MAX_METADATA_BYTES)
        || asset.title.contains(['\n', '\r'])
        || asset
            .source_url
            .as_deref()
            .is_some_and(|value| !bounded_text(value, MAX_URL_BYTES))
        || asset
            .original_url
            .as_deref()
            .is_some_and(|value| !bounded_text(value, MAX_URL_BYTES))
        || asset
            .source_revision
            .as_deref()
            .is_some_and(|value| !bounded_text(value, MAX_METADATA_BYTES))
        || !diagnostics_valid
        || asset.body.len() > MAX_ASSET_BYTES
        || asset.body.contains('\0')
    {
        return Err(InspectionError::new(
            "source_asset_invalid",
            "provider returned incomplete or oversized source asset",
        ));
    }
    Ok(())
}
/// `YYYY-MM-DD HH:MM:SS`, the persisted Jira wall-time format retained from legacy listings.
pub(super) fn issue_updated(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 19
        && bytes[16] == b':'
        && bytes[10] == b' '
        && bytes[17..].iter().all(u8::is_ascii_digit)
        && (bytes[17] - b'0') <= 5
        && crate::jira_query::wall_minute(value).is_some()
}
pub(super) fn bounded_text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && !value.contains('\0')
        && !value.chars().any(char::is_control)
}
fn identifier(value: &str, max: usize) -> bool {
    bounded_text(value, max)
        && value
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'_' | b'-'))
}

const RESERVED_FIELDS: &[&str] = &[
    "schema_version",
    "provider",
    "provider_instance",
    "resource_type",
    "canonical_id",
    "source_url",
    "original_url",
    "complete",
    "fetched_at",
    "source_revision",
    "content_hash",
    "generated",
    "container",
    "attachments",
    "library_item_id",
    "library_revision",
];

fn valid_frontmatter_value(value: &FrontmatterValue) -> bool {
    let valid_text = |text: &str| text.len() <= MAX_METADATA_BYTES && !text.contains('\0');
    let valid = match value {
        FrontmatterValue::Null => true,
        FrontmatterValue::String(text) => valid_text(text),
        FrontmatterValue::Strings(values) => {
            values.len() <= 256 && values.iter().all(|text| valid_text(text))
        }
        FrontmatterValue::Number(_) | FrontmatterValue::Boolean(_) => true,
    };
    valid && serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= MAX_METADATA_BYTES)
}

fn valid_extended_metadata(asset: &SourceAsset) -> bool {
    let mut keys = std::collections::BTreeSet::new();
    asset.container.as_ref().is_none_or(|container| {
        bounded_text(&container.id, MAX_METADATA_BYTES)
            && bounded_text(&container.label, MAX_METADATA_BYTES)
    }) && asset.fields.len() <= 32
        && asset.fields.iter().all(|field| {
            !field.key.is_empty()
                && field.key.len() <= 48
                && field
                    .key
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
                && !RESERVED_FIELDS.contains(&field.key.as_str())
                && keys.insert(&field.key)
                && valid_frontmatter_value(&field.value)
        })
        && asset.attachments.len() <= 256
        && asset.attachments.iter().all(|attachment| {
            bounded_text(&attachment.id, MAX_METADATA_BYTES)
                && bounded_text(&attachment.title, MAX_METADATA_BYTES)
                && [
                    &attachment.media_type,
                    &attachment.source_revision,
                    &attachment.not_downloaded,
                ]
                .iter()
                .all(|value| {
                    value
                        .as_deref()
                        .is_none_or(|value| bounded_text(value, MAX_METADATA_BYTES))
                })
                && attachment
                    .source_url
                    .as_deref()
                    .is_none_or(|url| bounded_text(url, MAX_URL_BYTES))
                && attachment.path.as_deref().is_none_or(|path| {
                    path.strip_prefix("_files/").is_some_and(|name| {
                        bounded_text(name, 255)
                            && !matches!(name, "." | "..")
                            && !name.contains(['/', '\\'])
                    })
                })
                && !(attachment.path.is_some() && attachment.not_downloaded.is_some())
        })
}
