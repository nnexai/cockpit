use std::io::{self, Write};

use cockpit_protocol::{library::LibraryOperation, widget::WIDGET_MAX_SELECTION_BYTES};
use serde::Serialize;

use super::error::OperationError;

pub const MUTATION_BYTES: usize = 64 * 1024;
pub const TERMINAL_COMMAND_BYTES: usize = 96 * 1024;
// 512 confirmations with 4 KiB paths, JSON escaping, hashes, and request metadata.
pub const LIBRARY_BYTES: usize = 13 * 1024 * 1024;
pub const LIBRARY_PAGE_ITEMS: usize = 5_000;
pub const LIBRARY_REPORT_ROWS: usize = 256;
pub const CREDENTIAL_BYTES: usize = 16 * 1024;
pub const NOTES_BYTES: usize = 4 * 1024 * 1024;
pub const WIDGET_BYTES: usize = 4096;
pub const WIDGET_SELECT_BYTES: usize = 2 * WIDGET_MAX_SELECTION_BYTES + WIDGET_BYTES;
pub const BROWSER_VIEW_JSON_BYTES: usize = 128 * 1024;
pub const BROWSER_VIEW_COMMAND_BYTES: usize = 6 * 1024 * 1024;

#[derive(Debug)]
pub enum SerializationLimitError {
    TooLarge,
    Invalid(serde_json::Error),
}

struct SizeLimit {
    remaining: usize,
    exceeded: bool,
}

impl Write for SizeLimit {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            self.exceeded = true;
            return Err(io::Error::other("request exceeds byte limit"));
        }
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Count serialized bytes without allocating or copying an encoded request.
pub fn check_serialized_size<T: Serialize>(
    request: &T,
    limit: usize,
) -> Result<(), SerializationLimitError> {
    let mut writer = SizeLimit {
        remaining: limit,
        exceeded: false,
    };
    match serde_json::to_writer(&mut writer, request) {
        Ok(()) => Ok(()),
        Err(_) if writer.exceeded => Err(SerializationLimitError::TooLarge),
        Err(error) => Err(SerializationLimitError::Invalid(error)),
    }
}

pub fn widget_request_size<T: Serialize>(request: &T) -> Result<(), OperationError> {
    check_serialized_size(request, WIDGET_BYTES).map_err(|_| {
        OperationError::rejected("widget_usage", "Malformed or oversized widget request")
    })
}

pub fn bounded_operation(mut operation: LibraryOperation) -> LibraryOperation {
    if let Some(report) = &mut operation.report
        && report.rows.len() > LIBRARY_REPORT_ROWS
    {
        report.rows.truncate(LIBRARY_REPORT_ROWS);
        report.truncated_rows = true;
    }
    operation
}
