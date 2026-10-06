use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

use cap_std::fs::Dir;
use cockpit_protocol::notes::*;
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use time::{Date, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use super::fs;
use crate::InspectionError;

const FILE_MAX: usize = 256 * 1024;
const ENTRY_MAX: usize = 4096;
const AGGREGATE_MAX: usize = 64 * 1024 * 1024;
const FRONTMATTER_MAX: usize = 64 * 1024;
const TITLE_MAX: usize = 512;

/// Source boundaries are kept independently of scalar validity: edits never
/// serialize front matter, including properties Cockpit does not understand.
pub(super) struct Frontmatter {
    pub body_start: usize,
    pub values: BTreeMap<String, String>,
    pub malformed: bool,
    pub unterminated: bool,
}

pub(super) fn frontmatter(content: &str) -> Frontmatter {
    let mut result = Frontmatter {
        body_start: 0,
        values: BTreeMap::new(),
        malformed: false,
        unterminated: false,
    };
    let mut lines = content.split_inclusive('\n');
    if lines.next().map(line_text) != Some("---") {
        return result;
    }
    let first_end = content.find('\n').map_or(content.len(), |n| n + 1);
    let mut offset = first_end;
    let mut closed = false;
    let mut seen = BTreeSet::new();
    for line in lines {
        let end = offset + line.len();
        if end > FRONTMATTER_MAX {
            break;
        }
        let text = line_text(line);
        if text == "---" {
            result.body_start = end;
            closed = true;
            break;
        }
        if !text.starts_with(char::is_whitespace) && !text.starts_with('#') && !text.is_empty() {
            if let Some((key, value)) = text.split_once(':') {
                if matches!(
                    key,
                    "recorded" | "decided" | "replaces" | "created" | "author"
                ) {
                    if !seen.insert(key) {
                        result.malformed = true;
                    }
                    match scalar(value) {
                        Ok(Some(value)) => {
                            result.values.insert(key.to_owned(), value);
                        }
                        Ok(None) => {}
                        Err(()) => result.malformed = true,
                    }
                }
            } else {
                result.malformed = true;
            }
        }
        offset = end;
    }
    if !closed {
        result.malformed = true;
        result.unterminated = true;
    }
    if result.malformed {
        result.values.clear();
    }
    result
}

fn line_text(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

fn scalar(value: &str) -> Result<Option<String>, ()> {
    let value = value.trim();
    if value.is_empty() || matches!(value, "null" | "Null" | "NULL" | "~") {
        return Ok(None);
    }
    if value.starts_with('"') {
        // Find the terminating quote, allowing escaped quotes and a YAML comment.
        let mut escaped = false;
        for (index, ch) in value.char_indices().skip(1) {
            if ch == '"' && !escaped {
                if !scalar_tail(&value[index + 1..]) {
                    return Err(());
                }
                return serde_json::from_str::<String>(&value[..index + 1])
                    .map(Some)
                    .map_err(|_| ());
            }
            escaped = ch == '\\' && !escaped;
        }
        return Err(());
    }
    if value.starts_with('\'') {
        let mut result = String::new();
        let mut chars = value[1..].char_indices().peekable();
        while let Some((index, ch)) = chars.next() {
            if ch == '\'' {
                if chars.peek().is_some_and(|(_, ch)| *ch == '\'') {
                    chars.next();
                    result.push('\'');
                } else if scalar_tail(&value[index + 2..]) {
                    return Ok(Some(result));
                } else {
                    return Err(());
                }
            } else {
                result.push(ch);
            }
        }
        return Err(());
    }
    if value.starts_with(['[', ']', '{', '}', '|', '>', '&', '*', '!', '@', '`']) {
        return Err(());
    }
    let comment = value
        .char_indices()
        .find(|(index, ch)| {
            *ch == '#' && (*index == 0 || value[..*index].ends_with(char::is_whitespace))
        })
        .map_or(value.len(), |(index, _)| index);
    let value = value[..comment].trim_end();
    if value.chars().any(char::is_control) || value.contains(": ") {
        return Err(());
    }
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

fn scalar_tail(value: &str) -> bool {
    value.is_empty()
        || (value.starts_with(char::is_whitespace)
            && (value.trim().is_empty() || value.trim_start().starts_with('#')))
}

pub(super) fn timestamp(value: &str) -> Option<i128> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .map(|date| date.unix_timestamp_nanos())
}

fn valid_decided(value: &str) -> bool {
    if timestamp(value).is_some() {
        return true;
    }
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return false;
    }
    let year = value[..4].parse::<i32>().expect("four ASCII digits");
    let month = (bytes[5] - b'0') * 10 + bytes[6] - b'0';
    let day = (bytes[8] - b'0') * 10 + bytes[9] - b'0';
    time::Month::try_from(month)
        .is_ok_and(|month| Date::from_calendar_date(year, month, day).is_ok())
}

struct Record {
    document: NotesDocument,
    summary: NotesDecisionSummary,
    heading: Option<Range<usize>>,
    body_start: usize,
    post_frontmatter: usize,
    unterminated: bool,
    recorded_instant: Option<i128>,
}

fn parse(id: String, document: NotesDocument) -> Record {
    let fm = frontmatter(&document.content);
    let mut problems = Vec::new();
    if fm.malformed {
        problems.push("frontmatter_malformed".to_owned());
    }
    let mut recorded = fm.values.get("recorded").cloned();
    let recorded_instant = recorded.as_deref().and_then(timestamp);
    if recorded.is_some() && recorded_instant.is_none() {
        recorded = None;
        problems.push("recorded_invalid".to_owned());
    }
    let mut decided = fm.values.get("decided").cloned();
    if decided
        .as_deref()
        .is_some_and(|value| !valid_decided(value))
    {
        decided = None;
        problems.push("decided_invalid".to_owned());
    }
    let mut replaces = fm.values.get("replaces").cloned();
    if replaces
        .as_deref()
        .is_some_and(|value| fs::validate_id(value, 128).is_err())
    {
        replaces = None;
        problems.push("replaces_invalid".to_owned());
    }
    let (heading, block_end) = first_heading(&document.content, fm.body_start);
    let body_start = skip_blank_lines(&document.content, block_end.unwrap_or(fm.body_start));
    let title = heading.as_ref().map_or_else(
        || id.clone(),
        |range| document.content[range.clone()].to_owned(),
    );
    Record {
        summary: NotesDecisionSummary {
            decision_id: id,
            title,
            recorded,
            decided,
            replaces,
            replaced_by: Vec::new(),
            status: NotesDecisionStatus::Current,
            revision: document.revision.clone(),
            problems,
        },
        document,
        heading,
        body_start,
        post_frontmatter: fm.body_start,
        unterminated: fm.unterminated,
        recorded_instant,
    }
}

/// CommonMark owns both the block and inline source boundaries, including
/// emphasis/code delimiters, ATX closers, and multiline setext headings.
fn first_heading(content: &str, start: usize) -> (Option<Range<usize>>, Option<usize>) {
    let mut heading: Option<Range<usize>> = None;
    let mut inline: Option<Range<usize>> = None;
    for (event, range) in Parser::new(&content[start..]).into_offset_iter() {
        let range = start + range.start..start + range.end;
        if let Some(block) = &heading {
            if matches!(event, Event::End(TagEnd::Heading(HeadingLevel::H1))) {
                let inline = inline.unwrap_or_else(|| {
                    // Empty ATX headings have no inline events. Preserve their
                    // opening marker/spacing and insert into the empty slot.
                    let line = line_text(
                        content[block.clone()]
                            .split_inclusive('\n')
                            .next()
                            .unwrap_or(""),
                    );
                    let mut offset = line.len() - line.trim_start_matches(' ').len() + 1;
                    while line
                        .as_bytes()
                        .get(offset)
                        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
                    {
                        offset += 1;
                    }
                    block.start + offset..block.start + offset
                });
                return (Some(inline), Some(owned_newline(content, block.end)));
            }
            match &mut inline {
                Some(inline) => {
                    inline.start = inline.start.min(range.start);
                    inline.end = inline.end.max(range.end);
                }
                None => inline = Some(range),
            }
        } else if matches!(
            event,
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            })
        ) {
            heading = Some(range);
        }
    }
    (None, None)
}

fn owned_newline(content: &str, end: usize) -> usize {
    if content[end..].starts_with("\r\n") {
        end + 2
    } else if content[end..].starts_with('\n') {
        end + 1
    } else {
        end
    }
}

fn skip_blank_lines(content: &str, mut start: usize) -> usize {
    for line in content[start..].split_inclusive('\n') {
        if !line.trim().is_empty() {
            break;
        }
        start += line.len();
    }
    start
}

struct Collection {
    records: Vec<Record>,
    entries: usize,
    bytes: usize,
}

fn collection(dir: &Dir) -> Result<Collection, InspectionError> {
    let names = fs::entries(dir, ENTRY_MAX)?;
    let mut records = Vec::new();
    let mut bytes = 0usize;
    for name in &names {
        let Some(id) = name.strip_suffix(".md") else {
            continue;
        };
        if name.starts_with('.') || fs::validate_id(id, 128).is_err() {
            continue;
        }
        if !regular_entry(dir, name)? {
            continue;
        }
        let document = fs::read(dir, name, FILE_MAX)?;
        if document.revision == "absent" {
            continue;
        }
        bytes += document.content.len();
        if bytes > AGGREGATE_MAX {
            return Err(too_large());
        }
        let record = parse(id.to_owned(), document);
        if record.summary.title.len() > TITLE_MAX {
            return Err(too_large());
        }
        records.push(record);
    }
    let mut successors: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for record in &records {
        if let Some(old) = &record.summary.replaces {
            successors
                .entry(old.clone())
                .or_default()
                .push(record.summary.decision_id.clone());
        }
    }
    for record in &mut records {
        if let Some(mut ids) = successors.remove(&record.summary.decision_id) {
            ids.sort();
            record.summary.replaced_by = ids;
            record.summary.status = NotesDecisionStatus::Replaced;
        }
    }
    records.sort_by(|a, b| {
        match (a.recorded_instant, b.recorded_instant) {
            (Some(a), Some(b)) => b.cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| a.summary.decision_id.cmp(&b.summary.decision_id))
    });
    Ok(Collection {
        records,
        entries: names.len(),
        bytes,
    })
}

pub(super) fn regular_entry(dir: &Dir, name: &str) -> Result<bool, InspectionError> {
    match dir.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(fs::error(
            "notes_unsafe_path",
            "Symbolic links are not allowed in Notes",
        )),
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(fs::error(
            "notes_unsafe_path",
            &format!("Cannot inspect Notes entry: {error}"),
        )),
    }
}

pub(super) fn optional_child(dir: &Dir, name: &str) -> Result<Option<Dir>, InspectionError> {
    match fs::child(dir, name, false) {
        Ok(dir) => Ok(Some(dir)),
        Err(error) if error.code == "notes_not_found" => Ok(None),
        Err(error) => Err(error),
    }
}

fn too_large() -> InspectionError {
    fs::error(
        "notes_too_large",
        "Decision collection or payload exceeds its limit",
    )
}

fn validate_payload(title: &str, body: &str, decided: Option<&str>) -> Result<(), InspectionError> {
    if title.len() > TITLE_MAX || body.len() > FILE_MAX {
        return Err(too_large());
    }
    if title.trim().is_empty() || title.chars().any(char::is_control) {
        return Err(fs::error(
            "notes_invalid_input",
            "Decision title must be nonempty and single-line",
        ));
    }
    if decided.is_some_and(|value| !valid_decided(value)) {
        return Err(fs::error(
            "notes_invalid_input",
            "Decided must be a date or RFC3339 timestamp",
        ));
    }
    Ok(())
}

fn into_decision(mut record: Record, folder: &Path) -> NotesDecision {
    let relative_path = format!("decisions/{}.md", record.summary.decision_id);
    let path = folder.join(&relative_path).to_string_lossy().into_owned();
    record.document.content.drain(..record.body_start);
    NotesDecision {
        summary: record.summary,
        body: record.document.content,
        relative_path,
        path,
    }
}

fn check_revision(record: &Record, expected: &str) -> Result<(), InspectionError> {
    if record.document.revision != expected {
        return Err(fs::error(
            "notes_conflict",
            "Decision changed; re-read before editing",
        ));
    }
    Ok(())
}

fn new_content(
    title: &str,
    body: &str,
    decided: Option<&str>,
    replaces: Option<&str>,
) -> Result<String, InspectionError> {
    validate_payload(title, body, decided)?;
    let mut content = format!("---\nrecorded: {}\n", fs::now());
    if let Some(decided) = decided {
        content.push_str(&format!("decided: {decided}\n"));
    }
    if let Some(replaces) = replaces {
        content.push_str(&format!("replaces: {replaces}\n"));
    }
    content.push_str("---\n# ");
    content.push_str(title);
    content.push_str("\n\n");
    content.push_str(body);
    if content.len() > FILE_MAX {
        return Err(too_large());
    }
    Ok(content)
}

pub(super) fn execute(
    dir: &Dir,
    folder: &Path,
    op: NotesOperation,
) -> Result<(bool, NotesResult), InspectionError> {
    match &op {
        NotesOperation::DecisionGet { decision_id }
        | NotesOperation::DecisionUpdate { decision_id, .. }
        | NotesOperation::DecisionReplace { decision_id, .. } => fs::validate_id(decision_id, 128)?,
        NotesOperation::DecisionCreate {
            title,
            body,
            decided,
        } => validate_payload(title, body, decided.as_deref())?,
        NotesOperation::DecisionList { .. } => {}
        _ => return Err(fs::error("notes_usage", "Not a decision operation")),
    }
    if let NotesOperation::DecisionReplace {
        title,
        body,
        decided,
        ..
    } = &op
    {
        validate_payload(title, body, decided.as_deref())?;
    }
    // Validate updates before even opening their collection.
    if let NotesOperation::DecisionUpdate { title, body, .. } = &op {
        if let Some(title) = title {
            validate_payload(title, "", None)?;
        }
        if body.as_ref().is_some_and(|body| body.len() > FILE_MAX) {
            return Err(too_large());
        }
    }
    let existing_dir = optional_child(dir, "decisions")?;
    let mut existing = match &existing_dir {
        Some(dir) => collection(dir)?,
        None => Collection {
            records: Vec::new(),
            entries: 0,
            bytes: 0,
        },
    };
    match op {
        NotesOperation::DecisionList { status, query } => {
            let query = query.map(|query| query.to_lowercase());
            let decisions = existing
                .records
                .into_iter()
                .filter(|record| {
                    let status_matches = match status {
                        NotesDecisionFilter::All => true,
                        NotesDecisionFilter::Current => {
                            record.summary.status == NotesDecisionStatus::Current
                        }
                        NotesDecisionFilter::History => {
                            record.summary.status == NotesDecisionStatus::Replaced
                        }
                    };
                    status_matches
                        && query.as_ref().is_none_or(|query| {
                            record.summary.title.to_lowercase().contains(query)
                                || record.document.content[record.body_start..]
                                    .to_lowercase()
                                    .contains(query)
                        })
                })
                .map(|record| record.summary)
                .collect();
            Ok((false, NotesResult::Decisions { decisions }))
        }
        NotesOperation::DecisionGet { decision_id } => {
            let index = find_record(&existing, &decision_id)?;
            Ok((
                false,
                NotesResult::Decision {
                    decision: into_decision(existing.records.remove(index), folder),
                },
            ))
        }
        NotesOperation::DecisionUpdate {
            decision_id,
            expected_revision,
            title,
            body,
        } => {
            let index = find_record(&existing, &decision_id)?;
            let record = existing.records.remove(index);
            check_revision(&record, &expected_revision)?;
            if record.unterminated {
                return Err(fs::error(
                    "notes_invalid_input",
                    "Cannot safely edit unterminated front matter",
                ));
            }
            let content = splice_update(&record, title.as_deref(), body.as_deref());
            if content.len() > FILE_MAX
                || existing.bytes - record.document.content.len() + content.len() > AGGREGATE_MAX
            {
                return Err(too_large());
            }
            let changed = content != record.document.content;
            if changed {
                fs::publish(
                    existing_dir.as_ref().expect("existing record directory"),
                    &format!("{decision_id}.md"),
                    &record.document,
                    &content,
                    FILE_MAX,
                )?;
            }
            let mut updated = parse(
                decision_id,
                NotesDocument {
                    revision: fs::revision(content.as_bytes()),
                    content,
                },
            );
            updated.summary.replaced_by = record.summary.replaced_by;
            updated.summary.status = record.summary.status;
            Ok((
                changed,
                NotesResult::Decision {
                    decision: into_decision(updated, folder),
                },
            ))
        }
        NotesOperation::DecisionCreate {
            title,
            body,
            decided,
        } => {
            let content = new_content(&title, &body, decided.as_deref(), None)?;
            create(dir, existing_dir, &existing, content, folder, None)
        }
        NotesOperation::DecisionReplace {
            decision_id,
            expected_revision,
            title,
            body,
            decided,
        } => {
            let record = &existing.records[find_record(&existing, &decision_id)?];
            check_revision(record, &expected_revision)?;
            if record.summary.status == NotesDecisionStatus::Replaced {
                return Err(fs::error(
                    "notes_decision_replaced",
                    "Decision already has a replacement",
                ));
            }
            let content = new_content(&title, &body, decided.as_deref(), Some(&decision_id))?;
            let source_name = format!("{decision_id}.md");
            create(
                dir,
                existing_dir,
                &existing,
                content,
                folder,
                Some((&source_name, &record.document)),
            )
        }
        _ => Err(fs::error("notes_usage", "Not a decision operation")),
    }
}

fn find_record(collection: &Collection, id: &str) -> Result<usize, InspectionError> {
    collection
        .records
        .iter()
        .position(|record| record.summary.decision_id == id)
        .ok_or_else(|| fs::error("notes_not_found", "Decision not found"))
}

fn splice_update(record: &Record, title: Option<&str>, body: Option<&str>) -> String {
    let source = &record.document.content;
    let mut content = source.clone();
    if let Some(body) = body {
        content.replace_range(record.body_start.., body);
    }
    if let Some(title) = title {
        match &record.heading {
            Some(range) if range.is_empty() => {
                let needs_opening_space =
                    source.as_bytes().get(range.start.wrapping_sub(1)) == Some(&b'#');
                let needs_closing_space = source.as_bytes().get(range.end) == Some(&b'#');
                let replacement = format!(
                    "{}{title}{}",
                    if needs_opening_space { " " } else { "" },
                    if needs_closing_space { " " } else { "" }
                );
                content.replace_range(range.clone(), &replacement);
            }
            Some(range) => content.replace_range(range.clone(), title),
            None => content.insert_str(record.post_frontmatter, &format!("# {title}\n\n")),
        }
    }
    content
}

fn create(
    dir: &Dir,
    existing_dir: Option<Dir>,
    existing: &Collection,
    content: String,
    folder: &Path,
    source: Option<(&str, &NotesDocument)>,
) -> Result<(bool, NotesResult), InspectionError> {
    if existing.entries >= ENTRY_MAX || existing.bytes + content.len() > AGGREGATE_MAX {
        return Err(too_large());
    }
    if let Some(dir) = &existing_dir {
        // Count ignored/non-UTF-8 names too: the post-add scan must stay bounded.
        fs::entries(dir, ENTRY_MAX - 1)?;
    }
    let dir = match existing_dir {
        Some(dir) => dir,
        None => fs::child(dir, "decisions", true)?,
    };
    let id = Uuid::new_v4().to_string();
    let name = format!("{id}.md");
    let base = fs::read(&dir, &name, FILE_MAX)?;
    if base.revision != "absent" {
        return Err(fs::error(
            "notes_conflict",
            "Generated decision already exists",
        ));
    }
    fs::publish_with_source(&dir, &name, &base, &content, FILE_MAX, source)?;
    let record = parse(
        id,
        NotesDocument {
            revision: fs::revision(content.as_bytes()),
            content,
        },
    );
    Ok((
        true,
        NotesResult::Decision {
            decision: into_decision(record, folder),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        path: std::path::PathBuf,
        dir: Dir,
    }

    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("cockpit-notes-decisions-{}", Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            let dir = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
            Self { path, dir }
        }

        fn put(&self, id: &str, content: &str) {
            std::fs::create_dir_all(self.path.join("decisions")).unwrap();
            std::fs::write(
                self.path.join("decisions").join(format!("{id}.md")),
                content,
            )
            .unwrap();
        }

        fn run(&self, op: NotesOperation) -> Result<(bool, NotesResult), InspectionError> {
            execute(&self.dir, &self.path, op)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn decision(result: (bool, NotesResult)) -> NotesDecision {
        match result.1 {
            NotesResult::Decision { decision } => decision,
            _ => panic!("Expected decision"),
        }
    }

    fn document(content: &str) -> NotesDocument {
        NotesDocument {
            content: content.to_owned(),
            revision: fs::revision(content.as_bytes()),
        }
    }

    #[test]
    fn frontmatter_scalars_are_bounded_and_top_level() {
        let source = "---\r\nrecorded: '2026-10-05T10:00:00Z'\r\ndecided: \"2025-03-04\" # historical\r\nreplaces: old-id # link\r\nnested:\r\n  author: not-top-level\r\nauthor: 'Ada''s # team'\r\n---\r\nbody";
        let fm = frontmatter(source);
        assert!(!fm.malformed);
        assert_eq!(&source[fm.body_start..], "body");
        assert_eq!(fm.values["recorded"], "2026-10-05T10:00:00Z");
        assert_eq!(fm.values["decided"], "2025-03-04");
        assert_eq!(fm.values["replaces"], "old-id");
        assert_eq!(fm.values["author"], "Ada's # team");
        for source in [
            "---\nrecorded: [bad]\n---\nbody".to_owned(),
            "---\nrecorded: \"unterminated\n---\nbody".to_owned(),
            "---\nrecorded: null\nrecorded: now\n---\nbody".to_owned(),
            format!("---\n{}---\nbody", "x".repeat(FRONTMATTER_MAX)),
        ] {
            let fm = frontmatter(&source);
            assert!(fm.malformed);
            assert!(fm.values.is_empty());
        }
        assert_eq!(frontmatter("---\nauthor: \"\"\n---\n").values["author"], "");
    }

    #[test]
    fn first_h1_splices_preserve_frontmatter_preamble_and_markers() {
        let prefix = "---\r\nrecorded: 2026-10-05T10:00:00Z\r\ncustom: untouched\r\n---\r\nPreamble 🧭\r\n\r\n";
        for (heading, expected_heading) in [
            ("# Old **title** ###\r\n", "# New 🧭 ###\r\n"),
            ("Old **title**\r\n=====\r\n", "New 🧭\r\n=====\r\n"),
        ] {
            let source = format!("{prefix}{heading}\r\nbody\r\n");
            let record = parse("manual".to_owned(), document(&source));
            assert_eq!(record.summary.title, "Old **title**");
            assert_eq!(&record.document.content[record.body_start..], "body\r\n");
            let updated = splice_update(&record, Some("New 🧭"), Some("replacement\r\n"));
            assert_eq!(
                updated,
                format!("{prefix}{expected_heading}\r\nreplacement\r\n")
            );
        }
        let source = format!("{prefix}No heading\r\n");
        let record = parse("manual".to_owned(), document(&source));
        assert_eq!(record.summary.title, "manual");
        let updated = splice_update(&record, Some("Inserted"), None);
        assert_eq!(
            updated,
            format!(
                "---\r\nrecorded: 2026-10-05T10:00:00Z\r\ncustom: untouched\r\n---\r\n# Inserted\n\nPreamble 🧭\r\n\r\nNo heading\r\n"
            )
        );
    }

    #[test]
    fn fenced_heading_is_not_a_title_and_invalid_dates_are_unknown() {
        let source = "---\nrecorded: yesterday\ndecided: 2026-02-30\n---\n```\n# not a title\n```\n\n## not H1\n\n# Actual\n\nBody";
        let record = parse("fallback".to_owned(), document(source));
        assert_eq!(record.summary.title, "Actual");
        assert_eq!(record.summary.recorded, None);
        assert_eq!(record.summary.decided, None);
        assert!(
            record
                .summary
                .problems
                .contains(&"recorded_invalid".to_owned())
        );
        assert!(
            record
                .summary
                .problems
                .contains(&"decided_invalid".to_owned())
        );
    }

    #[test]
    fn replacement_preserves_old_bytes_and_refuses_second_replacement() {
        let fixture = Fixture::new();
        let source = "---\nrecorded: 2020-01-01T00:00:00Z\ncustom: old\n---\nPreamble\n\n# Old\n\nOld body\n";
        fixture.put("old", source);
        let revision = fs::revision(source.as_bytes());
        let replacement = decision(
            fixture
                .run(NotesOperation::DecisionReplace {
                    decision_id: "old".to_owned(),
                    expected_revision: revision.clone(),
                    title: "New".to_owned(),
                    body: "New body".to_owned(),
                    decided: Some("2019-01-01".to_owned()),
                })
                .unwrap(),
        );
        assert_eq!(replacement.summary.replaces.as_deref(), Some("old"));
        assert_eq!(
            std::fs::read_to_string(fixture.path.join("decisions/old.md")).unwrap(),
            source
        );
        let old = decision(
            fixture
                .run(NotesOperation::DecisionGet {
                    decision_id: "old".to_owned(),
                })
                .unwrap(),
        );
        assert_eq!(old.summary.status, NotesDecisionStatus::Replaced);
        assert_eq!(
            old.summary.replaced_by,
            vec![replacement.summary.decision_id]
        );
        assert_eq!(old.summary.revision, revision);
        let error = fixture
            .run(NotesOperation::DecisionReplace {
                decision_id: "old".to_owned(),
                expected_revision: revision,
                title: "Another".to_owned(),
                body: String::new(),
                decided: None,
            })
            .unwrap_err();
        assert_eq!(error.code, "notes_decision_replaced");
        assert_eq!(
            std::fs::read_to_string(fixture.path.join("decisions/old.md")).unwrap(),
            source
        );
    }

    #[test]
    fn filtering_search_and_sort_use_frozen_instants_not_filenames() {
        let fixture = Fixture::new();
        fixture.put(
            "a",
            "---\nrecorded: 2026-01-01T12:00:00+02:00\n---\n# A\n\nNeedle in body",
        );
        fixture.put(
            "b",
            "---\nrecorded: 2026-01-01T10:30:00Z\nreplaces: a\n---\n# B\n\nnew",
        );
        fixture.put("z", "# Unknown\n\nunknown");
        let (_, NotesResult::Decisions { decisions }) = fixture
            .run(NotesOperation::DecisionList {
                status: NotesDecisionFilter::All,
                query: None,
            })
            .unwrap()
        else {
            panic!("Expected list");
        };
        assert_eq!(
            decisions
                .iter()
                .map(|d| d.decision_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a", "z"]
        );
        let (_, NotesResult::Decisions { decisions }) = fixture
            .run(NotesOperation::DecisionList {
                status: NotesDecisionFilter::History,
                query: Some("nEeDlE".to_owned()),
            })
            .unwrap()
        else {
            panic!("Expected list");
        };
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].decision_id, "a");
        let (_, NotesResult::Decisions { decisions }) = fixture
            .run(NotesOperation::DecisionList {
                status: NotesDecisionFilter::Current,
                query: Some("needle".to_owned()),
            })
            .unwrap()
        else {
            panic!("Expected list");
        };
        assert!(decisions.is_empty());
    }

    #[test]
    fn updates_are_cas_and_never_rewrite_metadata() {
        let fixture = Fixture::new();
        let source = "---\nrecorded: '2026-01-01T00:00:00Z'\ndecided: 2020-01-01\nreplaces: missing\ncustom: exact\n---\n# Title\n\nbody";
        fixture.put("manual", source);
        let error = fixture
            .run(NotesOperation::DecisionUpdate {
                decision_id: "manual".to_owned(),
                expected_revision: "sha256:stale".to_owned(),
                title: Some("new".to_owned()),
                body: None,
            })
            .unwrap_err();
        assert_eq!(error.code, "notes_conflict");
        assert_eq!(
            std::fs::read_to_string(fixture.path.join("decisions/manual.md")).unwrap(),
            source
        );
        let updated = decision(
            fixture
                .run(NotesOperation::DecisionUpdate {
                    decision_id: "manual".to_owned(),
                    expected_revision: fs::revision(source.as_bytes()),
                    title: Some("new".to_owned()),
                    body: Some("new body".to_owned()),
                })
                .unwrap(),
        );
        assert_eq!(
            updated.summary.recorded.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        assert_eq!(updated.summary.decided.as_deref(), Some("2020-01-01"));
        assert_eq!(updated.summary.replaces.as_deref(), Some("missing"));
        assert_eq!(
            std::fs::read_to_string(fixture.path.join("decisions/manual.md")).unwrap(),
            source.replace("# Title\n\nbody", "# new\n\nnew body")
        );
        let decisions = fs::child(&fixture.dir, "decisions", false).unwrap();
        let stale = fs::read(&decisions, "manual.md", FILE_MAX).unwrap();
        std::fs::write(fixture.path.join("decisions/manual.md"), "external save").unwrap();
        assert_eq!(
            fs::publish(&decisions, "manual.md", &stale, "lost update", FILE_MAX)
                .unwrap_err()
                .code,
            "notes_conflict"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.path.join("decisions/manual.md")).unwrap(),
            "external save"
        );
    }

    #[test]
    fn limits_fail_before_creating_storage() {
        let fixture = Fixture::new();
        for (title, body) in [
            ("x".repeat(TITLE_MAX + 1), String::new()),
            ("title".to_owned(), "x".repeat(FILE_MAX)),
        ] {
            assert_eq!(
                fixture
                    .run(NotesOperation::DecisionCreate {
                        title,
                        body,
                        decided: None
                    })
                    .unwrap_err()
                    .code,
                "notes_too_large"
            );
            assert!(!fixture.path.join("decisions").exists());
        }
        for (entries, bytes) in [(ENTRY_MAX, 0), (0, AGGREGATE_MAX)] {
            let existing = Collection {
                records: Vec::new(),
                entries,
                bytes,
            };
            assert_eq!(
                create(
                    &fixture.dir,
                    None,
                    &existing,
                    "new".to_owned(),
                    &fixture.path,
                    None
                )
                .unwrap_err()
                .code,
                "notes_too_large"
            );
            assert!(!fixture.path.join("decisions").exists());
        }
        std::fs::create_dir(fixture.path.join("decisions")).unwrap();
        for index in 0..=ENTRY_MAX {
            std::fs::write(
                fixture
                    .path
                    .join("decisions")
                    .join(format!("ignored-{index}.txt")),
                "",
            )
            .unwrap();
        }
        assert_eq!(
            fixture
                .run(NotesOperation::DecisionList {
                    status: NotesDecisionFilter::All,
                    query: None
                })
                .unwrap_err()
                .code,
            "notes_too_large"
        );
    }
}

#[cfg(test)]
mod heading_boundary_tests {
    use super::*;

    #[test]
    fn empty_atx_and_multiline_setext_inline_slots_remain_headings() {
        for (source, expected) in [
            ("#\n\nbody", "# New\n\nbody"),
            ("# ###\n\nbody", "# New ###\n\nbody"),
            ("#   \r\n\r\nbody", "#   New\r\n\r\nbody"),
            (
                "First 🧭\nsecond **line**   \n===\n\nbody",
                "New   \n===\n\nbody",
            ),
        ] {
            let record = parse(
                "id".to_owned(),
                NotesDocument {
                    content: source.to_owned(),
                    revision: fs::revision(source.as_bytes()),
                },
            );
            let content = splice_update(&record, Some("New"), None);
            assert_eq!(content, expected);
            let reparsed = parse(
                "id".to_owned(),
                NotesDocument {
                    revision: fs::revision(content.as_bytes()),
                    content,
                },
            );
            assert_eq!(reparsed.summary.title, "New");
            assert_eq!(&reparsed.document.content[reparsed.body_start..], "body");
        }
    }
}

#[cfg(test)]
mod imported_boundary_tests {
    use super::*;

    #[test]
    fn imported_title_overlimit_is_rejected_on_get() {
        let path =
            std::env::temp_dir().join(format!("cockpit-notes-title-limit-{}", Uuid::new_v4()));
        std::fs::create_dir_all(path.join("decisions")).unwrap();
        std::fs::write(
            path.join("decisions/manual.md"),
            format!("# {}\n", "x".repeat(TITLE_MAX + 1)),
        )
        .unwrap();
        let dir = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        assert_eq!(
            execute(
                &dir,
                &path,
                NotesOperation::DecisionGet {
                    decision_id: "manual".to_owned()
                }
            )
            .unwrap_err()
            .code,
            "notes_too_large"
        );
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn decided_is_an_exact_date_or_rfc3339_instant() {
        assert!(valid_decided("2024-02-29"));
        assert!(valid_decided("2026-01-01T12:00:00+02:00"));
        for value in [
            "2023-02-29",
            "2026-1-01",
            "2026-01-1",
            "+2026-01-01",
            "yesterday",
            "2026-01-01\n",
        ] {
            assert!(!valid_decided(value));
        }
    }
}
