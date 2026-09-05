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

fn split_line_ending(value: &str) -> (&str, &'static str) {
    if let Some(content) = value.strip_suffix("\r\n") {
        (content, "crlf")
    } else if let Some(content) = value.strip_suffix('\n') {
        (content, "lf")
    } else {
        (value, "none")
    }
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
                (CommentAnchor::Lines { .. }, CommentAnchor::WholeFile) => {
                    std::cmp::Ordering::Greater
                }
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
        let (comment, count) = sanitize(&draft.comment_text);
        sanitized_controls = sanitized_controls.saturating_add(count);

        if draft.source_state != CommentSourceState::Current {
            stale_draft_ids.push(draft.draft_id.clone());
        }

        match &draft.anchor {
            CommentAnchor::WholeFile => {
                let (path, count) = sanitize(&draft.file_ref.path);
                sanitized_controls = sanitized_controls.saturating_add(count);
                payload.push_str(&path);
                payload.push_str(" (whole file)\n");
            }
            CommentAnchor::Lines {
                start_line,
                end_line,
                selected_lines,
            } => {
                let (path, count) = sanitize(&draft.file_ref.path);
                sanitized_controls = sanitized_controls.saturating_add(count);
                payload.push_str(&path);
                payload.push(':');
                payload.push_str(&start_line.to_string());
                if end_line != start_line {
                    payload.push('-');
                    payload.push_str(&end_line.to_string());
                }
                let marker = match draft.file_ref.review.as_ref().map(|review| review.side) {
                    Some(cockpit_protocol::review::ReviewSide::Old) => {
                        payload.push_str(" (removed)");
                        '-'
                    }
                    Some(cockpit_protocol::review::ReviewSide::New) => '+',
                    None => ' ',
                };
                payload.push('\n');
                for source_line in selected_lines {
                    let (content, _) = split_line_ending(source_line);
                    let (line, count) = sanitize(content);
                    sanitized_controls = sanitized_controls.saturating_add(count);
                    payload.push(marker);
                    payload.push_str(&line);
                    payload.push('\n');
                }
            }
        }
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
    use super::{capture_lines, format_batch, sanitize, split_line_ending};
    use cockpit_protocol::comments::{
        CommentAnchor, CommentBatch, CommentDraft, CommentFileRef, CommentLocation, CommentOwner,
        CommentReviewRef, CommentSourceState,
    };
    use cockpit_protocol::context::ExtensionKind;
    use cockpit_protocol::review::ReviewSide;

    fn batch(drafts: Vec<CommentDraft>) -> CommentBatch {
        CommentBatch {
            batch_id: "batch".to_owned(),
            generation: 1,
            owner: CommentOwner {
                session_id: "session".to_owned(),
                pane_id: "pane".to_owned(),
                terminal_id: "terminal".to_owned(),
                source_kind: ExtensionKind::Review,
                source_id: "source".to_owned(),
            },
            last_known_location: CommentLocation {
                workspace_id: "workspace".to_owned(),
                tab_id: "tab".to_owned(),
            },
            live_attachment: None,
            drafts,
            updated_at: "1".to_owned(),
        }
    }

    fn review_draft(side: ReviewSide, start_line: u32, lines: &[&str]) -> CommentDraft {
        CommentDraft {
            draft_id: format!("draft-{start_line}"),
            file_ref: CommentFileRef {
                review: Some(CommentReviewRef {
                    review_id: "review".to_owned(),
                    generation: 1,
                    file_id: "file".to_owned(),
                    side,
                }),
                root_id: "root".to_owned(),
                path: "crates/cockpit-core/src/comments/paste.rs".to_owned(),
                absolute_path: "/checkout/crates/cockpit-core/src/comments/paste.rs".to_owned(),
                revision: "ignored-by-export".to_owned(),
                content_hash: Some("ignored-by-export".to_owned()),
            },
            anchor: CommentAnchor::Lines {
                start_line,
                end_line: start_line + lines.len() as u32 - 1,
                selected_lines: lines.iter().map(|line| (*line).to_owned()).collect(),
            },
            comment_text: "this is a comment".to_owned(),
            source_state: CommentSourceState::Current,
            updated_at: "1".to_owned(),
        }
    }

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
    fn preview_line_rendering_distinguishes_crlf_lf_and_missing_newline() {
        assert_eq!(
            split_line_ending("front: true\r\n"),
            ("front: true", "crlf")
        );
        assert_eq!(split_line_ending("body\n"), ("body", "lf"));
        assert_eq!(split_line_ending("last"), ("last", "none"));
    }

    #[test]
    fn preview_uses_reviewr_style_locations_markers_and_compact_blocks() {
        let older = review_draft(
            ReviewSide::Old,
            811,
            &["        if let Err(error) = adapter\n"],
        );
        let newer = review_draft(
            ReviewSide::New,
            830,
            &["        self.paste_store\n", "            .save(receipt)\n"],
        );
        let formatted = format_batch(&batch(vec![newer, older]), false);

        assert_eq!(
            formatted.payload,
            "crates/cockpit-core/src/comments/paste.rs:811 (removed)\n\
-        if let Err(error) = adapter\n\
this is a comment\n\
\n\
crates/cockpit-core/src/comments/paste.rs:830-831\n\
+        self.paste_store\n\
+            .save(receipt)\n\
this is a comment\n"
        );
        assert!(!formatted.payload.contains("review_id:"));
        assert!(!formatted.payload.contains("revision:"));
        assert!(formatted.exportable);
    }

    #[test]
    fn stale_drafts_keep_their_compact_preview_but_require_confirmation() {
        let mut draft = review_draft(ReviewSide::New, 1, &["line\n"]);
        draft.source_state = CommentSourceState::Changed;
        let formatted = format_batch(&batch(vec![draft.clone()]), false);

        assert_eq!(formatted.stale_draft_ids, vec!["draft-1"]);
        assert!(!formatted.exportable);
        assert!(formatted.payload.starts_with(
            "crates/cockpit-core/src/comments/paste.rs:1\n+line\nthis is a comment\n"
        ));
        assert!(format_batch(&batch(vec![draft]), true).exportable);
    }
}
