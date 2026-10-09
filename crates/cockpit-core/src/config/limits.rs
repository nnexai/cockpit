use std::fmt::Display;

use cockpit_protocol::projects::ProjectLimits;
use serde::Deserialize;

use crate::InspectionError;

macro_rules! limits {
    ($($field:ident: $ty:ty = $default:expr, $min:expr, $max:expr;)*) => {
        #[derive(Debug, Default, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct TomlLimits {
            $(pub(super) $field: Option<$ty>,)*
        }

        impl TomlLimits {
            pub(super) fn resolve(self) -> Result<ProjectLimits, InspectionError> {
                Ok(ProjectLimits {
                    $($field: bounded(
                        self.$field.unwrap_or($default),
                        $min,
                        $max,
                        stringify!($field),
                    )?,)*
                })
            }
        }
    };
}

limits! {
    catalog_depth: u32 = 3, 1, 32;
    catalog_entries: u32 = 16_384, 1, 100_000;
    git_timeout_ms: u32 = 3000, 1, 120_000;
    git_output_bytes: u32 = 1024 * 1024, 1024, 16 * 1024 * 1024;
    operation_timeout_ms: u32 = 30_000, 1, 600_000;
    context_preview_bytes: u32 = 1024 * 1024, 1024, 8 * 1024 * 1024;
    context_preview_lines: u32 = 5000, 1, 20_000;
    context_directory_entries: u32 = 1000, 1, 10_000;
    context_tree_depth: u32 = 32, 1, 64;
    library_folder_files: u32 = 512, 1, 100_000;
    library_folder_bytes: u64 = 32 * 1024 * 1024, 1024 * 1024, 4 * 1024 * 1024 * 1024 - 1;
    library_file_bytes: u64 = 4 * 1024 * 1024, 1024, 1024 * 1024 * 1024;
    library_space_pages: u32 = 200, 1, 20_000;
    library_attachment_bytes: u64 = 25 * 1024 * 1024, 1024, 1024 * 1024 * 1024;
    library_item_attachment_bytes: u64 = 100 * 1024 * 1024, 1024 * 1024, 4 * 1024 * 1024 * 1024 - 1;
    library_max_items: u32 = 20_000, 100, 1_000_000;
}

fn bounded<T: PartialOrd + Display>(
    value: T,
    min: T,
    max: T,
    field: &str,
) -> Result<T, InspectionError> {
    if value >= min && value <= max {
        Ok(value)
    } else {
        Err(InspectionError::new(
            format!("invalid_{field}"),
            format!("{field} must be between {min} and {max}"),
        ))
    }
}
