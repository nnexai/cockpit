use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cockpit_core::InspectionError;
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub(crate) const FINITE_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
pub(crate) const FINITE_WRITE_TIMEOUT: Duration = Duration::from_millis(500);
pub(crate) const FINITE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);
/// Herdr answers a process-spawning or process-stopping mutation after the
/// process work, which slows under machine load.
pub(crate) const PROCESS_MUTATION_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
/// Git worktree mutations scale with checkout size: 120k files took 3.7 s.
pub(crate) const GIT_MUTATION_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline for Herdr's reply to one socket request.
///
/// Reads and in-place mutations answer in milliseconds, so the short bound keeps
/// a hung Herdr from stalling callers. An expired deadline on a mutation leaves
/// its outcome unknown and forces manual recovery, so mutations whose work
/// grows with load or repository size get a bound sized for that work.
pub(crate) fn response_deadline(method: &str) -> Duration {
    match method {
        "worktree.create" | "worktree.open" | "worktree.remove" => GIT_MUTATION_RESPONSE_TIMEOUT,
        "workspace.create" | "workspace.close" | "tab.create" | "tab.close" | "pane.split"
        | "pane.close" | "plugin.pane.open" => PROCESS_MUTATION_RESPONSE_TIMEOUT,
        _ => FINITE_RESPONSE_TIMEOUT,
    }
}

pub(crate) async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    line: &mut String,
    limit: usize,
) -> std::io::Result<usize> {
    line.clear();
    let mut bytes = Vec::with_capacity(limit.min(4096));
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if bytes.is_empty() {
                return Ok(0);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "line is not newline terminated",
            ));
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(chunk.len(), |index| index + 1);
        if bytes.len().saturating_add(take) > limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "line exceeds configured limit",
            ));
        }
        bytes.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if newline.is_some() {
            let text = std::str::from_utf8(&bytes).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "line is not valid UTF-8")
            })?;
            line.push_str(text);
            return Ok(line.len());
        }
    }
}

pub(crate) async fn write_with_progress<W: AsyncWrite + Unpin>(
    writer: &mut W,
    bytes: &[u8],
    progress: &AtomicUsize,
) -> (usize, std::io::Result<()>) {
    let mut written = 0;
    while written < bytes.len() {
        match writer.write(&bytes[written..]).await {
            Ok(0) => {
                return (
                    written,
                    Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "socket write made no progress",
                    )),
                );
            }
            Ok(count) => {
                written += count;
                progress.store(written, Ordering::Release);
            }
            Err(error) => return (written, Err(error)),
        }
    }
    (written, Ok(()))
}

pub(crate) async fn read_response<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    line: &mut String,
    limit: usize,
) -> Result<Value, InspectionError> {
    let read = read_bounded_line(reader, line, limit)
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidData {
                InspectionError::new(
                    "bounded_output",
                    "Herdr response line exceeds configured limit or is unterminated",
                )
            } else {
                InspectionError::new("disconnected", "Herdr response failed")
            }
        })?;
    if read == 0 {
        return Err(InspectionError::new(
            "disconnected",
            "Herdr closed the connection",
        ));
    }
    serde_json::from_str(line.trim_end()).map_err(|error| {
        InspectionError::new(
            "malformed_response",
            format!("Herdr returned invalid response JSON: {error}"),
        )
    })
}
