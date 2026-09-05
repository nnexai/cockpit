use cockpit_protocol::comments::{CommentAnchor, CommentBatch, CommentDraft, CommentSourceState};

pub(super) const PREVIEW_LIMIT_BYTES: u32 = 64 * 1024;
/// Bound a rendered response independently from the smaller paste ceiling.
/// An over-paste-limit preview remains inspectable; a response beyond this
/// limit is refused before crossing a host/client boundary.
pub(super) const MAX_PREVIEW_PAYLOAD_BYTES: u32 = 4 * 1024 * 1024;
const FRAMING_BYTES: u32 = 12;

/// Extract inclusive physical lines without normalizing their original bytes.
pub(super) fn capture_lines(
    text: &str,
    start_line: u32,
    end_line: u32,
) -> Result<Vec<String>, String> {
    if start_line == 0 || end_line < start_line {
        return Err("line range must be inclusive, positive, and ordered".to_owned());
    }
    if text.is_empty() {
        return Err("line range is outside the captured document".to_owned());
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let line_count = lines.len() as u32;
    if end_line > line_count {
        return Err("line range is outside the captured document".to_owned());
    }
    Ok(lines[(start_line - 1) as usize..end_line as usize]
        .iter()
        .map(|line| (*line).to_owned())
        .collect())
}

/// Escape terminal controls in untrusted source/comment text in one pass.
/// Newline, carriage return, and tab remain content; all other C0/C1 controls
/// and DEL are represented as a fixed-width hexadecimal escape.
pub(super) fn sanitize(value: &str) -> (String, u32) {
    let mut output = String::with_capacity(value.len());
    let mut count = 0u32;
    for character in value.chars() {
        let code = character as u32;
        let preserve = matches!(character, '\n' | '\r' | '\t');
        if !preserve && ((code <= 0x1f) || code == 0x7f || (0x80..=0x9f).contains(&code)) {
            output.push_str(&format!("\\x{code:02X}"));
            count = count.saturating_add(1);
        } else {
            output.push(character);
        }
    }
    (output, count)
}

fn quote_value(value: &str) -> (String, u32) {
    let (sanitized, count) = sanitize(value);
    let mut quoted = String::with_capacity(sanitized.len().saturating_add(2));
    quoted.push('"');
    for character in sanitized.chars() {
        if matches!(character, '\\' | '"') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    (quoted, count)
}

fn split_line_ending(value: &str) -> (&str, &'static str) {
    if let Some(content) = value.strip_suffix("\r\n") {
        (content, "crlf")
    } else if let Some(content) = value.strip_suffix('\n') {
        (content, "lf")
    } else {
        (value, "none")
    }
}

fn newline_metadata(lines: &[String]) -> String {
    let mut metadata = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            metadata.push(',');
        }
        metadata.push_str(split_line_ending(line).1);
    }
    metadata
}

pub(super) struct FormattedPreview {
    pub payload: String,
    pub payload_bytes: u32,
    pub framed_bytes: u32,
    pub sanitized_controls: u32,
    pub stale_draft_ids: Vec<String>,
    pub exportable: bool,
    pub reason: Option<String>,
}

pub(super) fn format_batch(batch: &CommentBatch, retain_stale_excerpts: bool) -> FormattedPreview {
    let mut drafts: Vec<&CommentDraft> = batch.drafts.iter().collect();
    drafts.sort_by(|left, right| {
        left.file_ref
            .absolute_path
            .cmp(&right.file_ref.absolute_path)
            .then_with(|| match (&left.anchor, &right.anchor) {
                (CommentAnchor::WholeFile, CommentAnchor::WholeFile) => std::cmp::Ordering::Equal,
                (CommentAnchor::WholeFile, CommentAnchor::Lines { .. }) => std::cmp::Ordering::Less,
                (CommentAnchor::Lines { .. }, CommentAnchor::WholeFile) => std::cmp::Ordering::Greater,
                (
                    CommentAnchor::Lines { start_line: a, .. },
                    CommentAnchor::Lines { start_line: b, .. },
                ) => a.cmp(b),
            })
            .then_with(|| left.draft_id.cmp(&right.draft_id))
    });

    let mut payload = String::new();
    let mut sanitized_controls = 0u32;
    let mut stale_draft_ids = Vec::new();
    for (index, draft) in drafts.iter().enumerate() {
        if index > 0 {
            payload.push('\n');
        }
        let (absolute_path, count) = quote_value(&draft.file_ref.absolute_path);
        sanitized_controls = sanitized_controls.saturating_add(count);
        let (relative_path, count) = quote_value(&draft.file_ref.path);
        sanitized_controls = sanitized_controls.saturating_add(count);
        let (revision, count) = sanitize(&draft.file_ref.revision);
        sanitized_controls = sanitized_controls.saturating_add(count);
        let hash = draft.file_ref.content_hash.as_deref().unwrap_or("<none>");
        let (hash, count) = sanitize(hash);
        sanitized_controls = sanitized_controls.saturating_add(count);
        let (comment, count) = sanitize(&draft.comment_text);
        sanitized_controls = sanitized_controls.saturating_add(count);

        if draft.source_state != CommentSourceState::Current {
            stale_draft_ids.push(draft.draft_id.clone());
        }

        payload.push_str("--- comment ");
        payload.push_str(&draft.draft_id);
        payload.push_str(" ---\n");
        payload.push_str("absolute_path: ");
        payload.push_str(&absolute_path);
        payload.push('\n');
        payload.push_str("relative_path: ");
        payload.push_str(&relative_path);
        payload.push('\n');
        payload.push_str("revision: ");
        payload.push_str(&revision);
        payload.push('\n');
        payload.push_str("content_hash: ");
        payload.push_str(&hash);
        payload.push('\n');
        payload.push_str("source_state: ");
        payload.push_str(match draft.source_state {
            CommentSourceState::Current => "current",
            CommentSourceState::Changed => "changed",
            CommentSourceState::Missing => "missing",
            CommentSourceState::Unavailable => "unavailable",
        });
        payload.push('\n');
        match &draft.anchor {
            CommentAnchor::WholeFile => payload.push_str("anchor: whole_file\n"),
            CommentAnchor::Lines {
                start_line,
                end_line,
                selected_lines,
            } => {
                payload.push_str(&format!("anchor: lines {start_line}-{end_line}\n"));
                payload.push_str("excerpt_newlines: [");
                payload.push_str(&newline_metadata(selected_lines));
                payload.push_str("]\n");
                payload.push_str("excerpt:\n");
                for (offset, source_line) in selected_lines.iter().enumerate() {
                    let (content, _) = split_line_ending(source_line);
                    let (line, count) = sanitize(content);
                    sanitized_controls = sanitized_controls.saturating_add(count);
                    payload.push_str(&format!("{:>6} | {}", start_line + offset as u32, line));
                    // This delimiter belongs to the preview record, not the captured source.
                    payload.push('\n');
                }
            }
        }
        payload.push_str("comment:\n");
        payload.push_str(&comment);
        payload.push('\n');
    }

    let payload_bytes = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    let framed_bytes = payload_bytes.saturating_add(FRAMING_BYTES);
    let mut reason = None;
    if framed_bytes > PREVIEW_LIMIT_BYTES {
        reason = Some(format!(
            "preview exceeds the {}-byte framed limit",
            PREVIEW_LIMIT_BYTES
        ));
    } else if !stale_draft_ids.is_empty() && !retain_stale_excerpts {
        reason = Some("one or more sources changed, disappeared, or became unavailable".to_owned());
    }
    let exportable = reason.is_none();
    FormattedPreview {
        payload,
        payload_bytes,
        framed_bytes,
        sanitized_controls,
        stale_draft_ids,
        exportable,
        reason,
    }
}


#[cfg(test)]
mod tests {
    use super::{capture_lines, quote_value, sanitize, split_line_ending};

    #[test]
    fn capture_preserves_physical_newlines_and_missing_final_newline() {
        let lines = capture_lines("front: true\r\nbody\r\nlast", 1, 3).expect("range");
        assert_eq!(lines, vec!["front: true\r\n", "body\r\n", "last"]);
    }

    #[test]
    fn capture_treats_a_lone_carriage_return_as_source_content() {
        let lines = capture_lines("first\rcontent\r\nsecond\nlast", 1, 3).expect("range");
        assert_eq!(lines, vec!["first\rcontent\r\n", "second\n", "last"]);
    }

    #[test]
    fn capture_rejects_empty_document_and_out_of_bounds_ranges() {
        assert!(capture_lines("", 1, 1).is_err());
        assert!(capture_lines("one\n", 2, 2).is_err());
    }

    #[test]
    fn sanitize_counts_controls_without_changing_source_capture() {
        let (sanitized, count) = sanitize("safe\x1b[201~\u{0085}");
        assert_eq!(sanitized, "safe\\x1B[201~\\x85");
        assert_eq!(count, 2);
    }

    #[test]
    fn quoted_paths_escape_delimiters_after_control_sanitizing() {
        let (quoted, count) = quote_value("review\\\".md\x1b");
        assert_eq!(quoted, "\"review\\\\\\\".md\\\\x1B\"");
        assert_eq!(count, 1);
    }

    #[test]
    fn preview_line_metadata_distinguishes_crlf_lf_and_missing_newline() {
        assert_eq!(split_line_ending("front: true\r\n"), ("front: true", "crlf"));
        assert_eq!(split_line_ending("body\n"), ("body", "lf"));
        assert_eq!(split_line_ending("last"), ("last", "none"));
    }
}
