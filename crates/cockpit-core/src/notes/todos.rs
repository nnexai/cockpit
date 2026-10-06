use std::{collections::HashMap, ops::Range, path::Path};

use cap_std::fs::Dir;
use cockpit_protocol::notes::*;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use uuid::Uuid;

use super::fs;
use crate::InspectionError;

const FILE: &str = "todos.md";
const MAX_FILE: usize = 1024 * 1024;
const MAX_TODOS: usize = 5000;
const MAX_TEXT: usize = 2048;

struct Metadata {
    id_value: Range<usize>,
    lane_value: Option<Range<usize>>,
    lane_token: Option<Range<usize>>,
    insert: usize,
}

struct Item {
    todo: NotesTodo,
    span: Range<usize>,
    checkbox: usize,
    text: Range<usize>,
    metadata: Option<Metadata>,
    has_children: bool,
}

// Only an item's first event (or the first event inside its initial paragraph)
// can identify it as a task. A descendant marker never identifies its parent.
struct Frame {
    span: Range<usize>,
    first: u8,
    todo: Option<usize>,
}

pub(super) fn execute(
    dir: &Dir,
    _folder: &Path,
    op: NotesOperation,
) -> Result<(bool, NotesResult), InspectionError> {
    if !matches!(
        &op,
        NotesOperation::TodoList { .. }
            | NotesOperation::TodoAdd { .. }
            | NotesOperation::TodoUpdate { .. }
            | NotesOperation::TodoSetDone { .. }
            | NotesOperation::TodoRemove { .. }
            | NotesOperation::KanbanList
            | NotesOperation::KanbanPromote { .. }
            | NotesOperation::KanbanMove { .. }
            | NotesOperation::KanbanUnboard { .. }
    ) {
        return Err(fs::error(
            "notes_usage",
            "Operation is not a todo operation",
        ));
    }
    let document = fs::read(dir, FILE, MAX_FILE)?;
    let items = parse(&document)?;
    match op {
        NotesOperation::TodoList { filter } => {
            Ok((false, listing(document.revision, items, filter)))
        }
        NotesOperation::KanbanList => {
            let mut columns = NotesBoard {
                backlog: Vec::new(),
                doing: Vec::new(),
                done: Vec::new(),
            };
            for item in items {
                match (item.todo.lane, item.todo.done) {
                    (Some(_), true) => columns.done.push(item.todo),
                    (Some(NotesLane::Backlog), false) => columns.backlog.push(item.todo),
                    (Some(NotesLane::Doing), false) => columns.doing.push(item.todo),
                    (None, _) => {}
                }
            }
            Ok((
                false,
                NotesResult::Board {
                    revision: document.revision,
                    columns,
                },
            ))
        }
        NotesOperation::TodoAdd { text, lane } => add(dir, document, items, text, lane),
        NotesOperation::TodoUpdate { todo, text } => {
            let text = text.map(|text| normalize_text(&text)).transpose()?;
            mutate(dir, document, items, todo, Change::Text(text))
        }
        NotesOperation::TodoSetDone { todo, done } => {
            mutate(dir, document, items, todo, Change::Done(done))
        }
        NotesOperation::TodoRemove { todo } => mutate(dir, document, items, todo, Change::Remove),
        NotesOperation::KanbanPromote { todo } => {
            mutate(dir, document, items, todo, Change::Promote)
        }
        NotesOperation::KanbanMove { todo, to } => {
            mutate(dir, document, items, todo, Change::Move(to))
        }
        NotesOperation::KanbanUnboard { todo } => {
            mutate(dir, document, items, todo, Change::Unboard)
        }
        _ => unreachable!("todo dispatch checked above"),
    }
}

pub(super) fn require_unique_id(dir: &Dir, id: &str) -> Result<(), InspectionError> {
    fs::validate_id(id, 64)?;
    let document = fs::read(dir, FILE, MAX_FILE)?;
    unique_index(&parse(&document)?, id).map(|_| ())
}

fn listing(revision: String, items: Vec<Item>, filter: NotesTodoFilter) -> NotesResult {
    NotesResult::Todos {
        revision,
        todos: items
            .into_iter()
            .map(|item| item.todo)
            .filter(|todo| match filter {
                NotesTodoFilter::All => true,
                NotesTodoFilter::Open => !todo.done,
                NotesTodoFilter::Done => todo.done,
            })
            .collect(),
    }
}

fn parse(document: &NotesDocument) -> Result<Vec<Item>, InspectionError> {
    let content = &document.content;
    if content.len() > MAX_FILE {
        return Err(fs::error("notes_too_large", "Todos file exceeds 1 MiB"));
    }
    let mut lines = vec![0];
    lines.extend(
        content
            .bytes()
            .enumerate()
            .filter_map(|(i, byte)| (byte == b'\n').then_some(i + 1)),
    );
    let mut stack: Vec<Frame> = Vec::new();
    let mut items: Vec<Item> = Vec::new();
    let options =
        Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    for (event, range) in Parser::new_ext(content, options).into_offset_iter() {
        if matches!(event, Event::Start(Tag::Item)) {
            if let Some(parent) = stack.last_mut() {
                parent.first = 2;
            }
            stack.push(Frame {
                span: owned_span(content, range),
                first: 0,
                todo: None,
            });
            continue;
        }
        if matches!(event, Event::End(TagEnd::Item)) {
            stack.pop();
            continue;
        }
        let Some(frame) = stack.last_mut() else {
            continue;
        };
        if frame.first == 2 {
            continue;
        }
        if frame.first == 0 && matches!(event, Event::Start(Tag::Paragraph)) {
            frame.first = 1;
            continue;
        }
        frame.first = 2;
        if let Event::TaskListMarker(done) = event {
            if items.len() == MAX_TODOS {
                return Err(fs::error(
                    "notes_too_large",
                    "Todos file contains more than 5000 tasks",
                ));
            }
            let span = frame.span.clone();
            let depth = stack.len() - 1;
            let item = scan_item(document, &lines, span, range, done, depth as u32)?;
            for ancestor in &stack[..depth] {
                if let Some(index) = ancestor.todo {
                    items[index].has_children = true;
                }
            }
            stack.last_mut().expect("current item").todo = Some(items.len());
            items.push(item);
        }
    }
    let mut ids: HashMap<&str, usize> = HashMap::new();
    for item in &items {
        if let Some(id) = item.todo.id.as_deref() {
            *ids.entry(id).or_default() += 1;
        }
    }
    let duplicates: Vec<usize> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            item.todo
                .id
                .as_deref()
                .is_some_and(|id| ids.get(id).copied().unwrap_or(0) > 1)
                .then_some(index)
        })
        .collect();
    drop(ids);
    for index in duplicates {
        items[index]
            .todo
            .problems
            .push(NotesTodoProblem::DuplicateId);
    }
    Ok(items)
}

fn owned_span(content: &str, mut span: Range<usize>) -> Range<usize> {
    // The parser excludes enclosing-list indentation from nested item starts.
    // It belongs to this physical item line: leaving it behind on removal would
    // concatenate it with the next sibling's indentation.
    let line_start = content[..span.start].rfind('\n').map_or(0, |i| i + 1);
    if content[line_start..span.start]
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
    {
        span.start = line_start;
    }
    // Some parser ranges already include their newline. Never consume the next
    // line (in particular a less-indented sibling) or a second blank line.
    if !content[..span.end].ends_with('\n') {
        if content[span.end..].starts_with("\r\n") {
            span.end += 2;
        } else if content[span.end..].starts_with('\n') {
            span.end += 1;
        }
    }
    span
}

fn scan_item(
    document: &NotesDocument,
    lines: &[usize],
    span: Range<usize>,
    marker: Range<usize>,
    done: bool,
    depth: u32,
) -> Result<Item, InspectionError> {
    let content = &document.content;
    let bytes = content.as_bytes();
    let line = lines.partition_point(|&start| start <= span.start);
    let line_start = lines[line - 1];
    let line_end = content[span.start..]
        .find('\n')
        .map_or(content.len(), |i| span.start + i);
    let line_end = if bytes.get(line_end.wrapping_sub(1)) == Some(&b'\r') {
        line_end - 1
    } else {
        line_end
    };
    let mut p = span.start;
    skip_space(bytes, &mut p, line_end);
    if matches!(bytes.get(p), Some(b'-' | b'+' | b'*')) {
        p += 1;
    } else {
        while p < line_end && bytes[p].is_ascii_digit() {
            p += 1;
        }
        if matches!(bytes.get(p), Some(b'.' | b')')) {
            p += 1;
        } else {
            p = line_end;
        }
    }
    let after_list_marker = p;
    skip_space(bytes, &mut p, line_end);
    let checkbox = p.saturating_add(1);
    let valid_marker = p > after_list_marker
        && p + 3 <= line_end
        && bytes[p] == b'['
        && bytes[p + 2] == b']'
        && (if done {
            matches!(bytes[p + 1], b'x' | b'X')
        } else {
            matches!(bytes[p + 1], b' ' | b'\t')
        })
        && marker.start <= p
        && marker.end == p + 3
        && marker.start >= span.start
        && marker.end <= span.end;
    let content_column = column(&content[line_start..p]);
    let mut text_start = if valid_marker {
        p + 3
    } else {
        marker.end.min(line_end)
    };
    skip_space(bytes, &mut text_start, line_end);
    let mut problems = Vec::new();
    if !valid_marker {
        problems.push(NotesTodoProblem::MetadataMalformed);
    }
    let (metadata, id, lane, metadata_start, malformed, unknown_lane) =
        scan_metadata(content, text_start..line_end);
    if malformed && !problems.contains(&NotesTodoProblem::MetadataMalformed) {
        problems.push(NotesTodoProblem::MetadataMalformed);
    }
    if unknown_lane {
        problems.push(NotesTodoProblem::UnknownLane);
    }
    let mut text_end = metadata_start.unwrap_or(line_end);
    while text_end > text_start && matches!(bytes[text_end - 1], b' ' | b'\t') {
        text_end -= 1;
    }
    if text_end - text_start > MAX_TEXT {
        return Err(fs::error("notes_too_large", "Todo text exceeds 2 KiB"));
    }
    for &start in &lines[line..] {
        if start >= span.end {
            break;
        }
        let end = content[start..span.end]
            .find('\n')
            .map_or(span.end, |i| start + i);
        let value = &content[start..end];
        if !value.trim().is_empty() && continuation_column(value) < content_column {
            problems.push(NotesTodoProblem::LazyContinuation);
            break;
        }
    }
    Ok(Item {
        todo: NotesTodo {
            id,
            reference: format!("L{line}@{}", document.revision),
            text: content[text_start..text_end].to_owned(),
            done,
            lane,
            revision: fs::revision(&bytes[span.clone()]),
            line: line as u32,
            depth,
            problems,
        },
        span,
        checkbox,
        text: text_start..text_end,
        metadata,
        has_children: false,
    })
}

fn skip_space(bytes: &[u8], p: &mut usize, end: usize) {
    while *p < end && matches!(bytes[*p], b' ' | b'\t') {
        *p += 1;
    }
}

fn column(prefix: &str) -> usize {
    prefix.bytes().fold(0, |column, byte| {
        if byte == b'\t' {
            column + 4 - column % 4
        } else {
            column + 1
        }
    })
}

fn continuation_column(line: &str) -> usize {
    // Quote prefixes are containers, not lazy text. Include their columns when
    // comparing against the first line's absolute content indentation.
    let end = line
        .bytes()
        .take_while(|byte| matches!(*byte, b' ' | b'\t' | b'>'))
        .count();
    column(&line[..end])
}

type MetadataScan = (
    Option<Metadata>,
    Option<String>,
    Option<NotesLane>,
    Option<usize>,
    bool,
    bool,
);

fn scan_metadata(content: &str, text: Range<usize>) -> MetadataScan {
    let line = &content[text.clone()];
    let mut cockpit = line.match_indices("<!--").filter_map(|(offset, _)| {
        let body = line[offset + 4..].trim_start_matches([' ', '\t']);
        (body.starts_with("cockpit")
            && body[7..]
                .chars()
                .next()
                .is_none_or(|ch| ch.is_ascii_whitespace() || ch == '-'))
        .then_some(text.start + offset)
    });
    let Some(start) = cockpit.next() else {
        return (None, None, None, None, false, false);
    };
    if cockpit.next().is_some() {
        return (None, None, None, Some(start), true, false);
    }
    let Some(relative_end) = content[start + 4..text.end].find("-->") else {
        return (None, None, None, Some(start), true, false);
    };
    let end = start + 4 + relative_end;
    if !content[end + 3..text.end]
        .trim_matches([' ', '\t'])
        .is_empty()
    {
        return (None, None, None, Some(start), true, false);
    }
    let bytes = content.as_bytes();
    let mut p = start + 4;
    skip_space(bytes, &mut p, end);
    p += 7; // cockpit
    let mut id_value = None;
    let mut lane_value = None;
    let mut lane_token = None;
    let mut insert = p;
    let mut malformed = false;
    while p < end {
        let gap = p;
        skip_space(bytes, &mut p, end);
        if p == end {
            break;
        }
        if p == gap {
            malformed = true;
            break;
        }
        let token_start = p;
        while p < end && !matches!(bytes[p], b' ' | b'\t') {
            p += 1;
        }
        insert = p;
        let token = &content[token_start..p];
        let Some((key, value)) = token.split_once('=') else {
            malformed = true;
            break;
        };
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || value.is_empty()
            || value.contains("<!--")
            || value.contains("-->")
        {
            malformed = true;
            break;
        }
        let value_range = token_start + key.len() + 1..p;
        match key {
            "id" => {
                if id_value.is_some() || fs::validate_id(value, 64).is_err() {
                    malformed = true;
                }
                id_value = Some(value_range);
            }
            "lane" => {
                if lane_value.is_some() {
                    malformed = true;
                }
                lane_value = Some(value_range);
                lane_token = Some(gap..p);
            }
            _ => {}
        }
    }
    let Some(id_value) = id_value else {
        return (None, None, None, Some(start), true, false);
    };
    let id = if fs::validate_id(&content[id_value.clone()], 64).is_ok() {
        Some(content[id_value.clone()].to_owned())
    } else {
        None
    };
    let lane = lane_value
        .as_ref()
        .and_then(|value| match &content[value.clone()] {
            "backlog" => Some(NotesLane::Backlog),
            "doing" => Some(NotesLane::Doing),
            _ => None,
        });
    let unknown_lane = lane_value.is_some() && lane.is_none();
    (
        Some(Metadata {
            id_value,
            lane_value,
            lane_token,
            insert,
        }),
        id,
        lane,
        Some(start),
        malformed,
        unknown_lane,
    )
}

fn normalize_text(text: &str) -> Result<String, InspectionError> {
    if text.contains("<!--") || text.contains("-->") {
        return Err(fs::error(
            "notes_invalid_input",
            "Todo text cannot contain HTML comment markers",
        ));
    }
    let text = text.trim();
    let mut normalized = String::with_capacity(text.len().min(MAX_TEXT));
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' && chars.peek() == Some(&'\n') {
            chars.next();
        }
        normalized.push(if matches!(ch, '\r' | '\n') { ' ' } else { ch });
        if normalized.len() > MAX_TEXT {
            return Err(fs::error("notes_too_large", "Todo text exceeds 2 KiB"));
        }
    }
    Ok(normalized)
}

fn unique_index(items: &[Item], id: &str) -> Result<usize, InspectionError> {
    let mut matches = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.todo.id.as_deref() == Some(id));
    let Some((index, _)) = matches.next() else {
        return Err(fs::error(
            "notes_not_found",
            "Todo identifier was not found",
        ));
    };
    if matches.next().is_some() {
        return Err(fs::error(
            "notes_todo_ambiguous",
            "Todo identifier occurs more than once; select a ref to repair it",
        ));
    }
    Ok(index)
}

fn resolve(
    items: &[Item],
    document: &NotesDocument,
    selector: &NotesTodoSelector,
) -> Result<usize, InspectionError> {
    match selector {
        NotesTodoSelector::Id {
            id,
            expected_revision,
        } => {
            fs::validate_id(id, 64)?;
            let index = unique_index(items, id)?;
            if items[index].todo.revision != *expected_revision {
                return Err(fs::error(
                    "notes_conflict",
                    "Todo item changed; re-read before retrying",
                ));
            }
            Ok(index)
        }
        NotesTodoSelector::Ref { reference } => {
            let parsed = reference
                .strip_prefix('L')
                .and_then(|value| value.split_once('@'));
            let Some((line, revision)) = parsed else {
                return Err(fs::error(
                    "notes_invalid_input",
                    "Todo ref must be L<line>@<file revision>",
                ));
            };
            let line = line
                .parse::<u32>()
                .ok()
                .filter(|line| *line > 0)
                .ok_or_else(|| fs::error("notes_invalid_input", "Invalid todo ref line"))?;
            let index = items
                .iter()
                .position(|item| item.todo.line == line)
                .ok_or_else(|| fs::error("notes_not_found", "Todo ref line was not found"))?;
            if document.revision != revision {
                return Err(fs::error(
                    "notes_conflict",
                    "Todos file changed; re-read before using a ref",
                ));
            }
            Ok(index)
        }
    }
}

fn new_id(items: &[Item]) -> String {
    loop {
        let mut random = Uuid::new_v4().as_u128();
        let mut id = [b'0'; 10];
        for byte in &mut id {
            let digit = (random % 36) as u8;
            *byte = if digit < 10 {
                b'0' + digit
            } else {
                b'a' + digit - 10
            };
            random /= 36;
        }
        let id = std::str::from_utf8(&id)
            .expect("ASCII identifier")
            .to_owned();
        if !items
            .iter()
            .any(|item| item.todo.id.as_deref() == Some(&id))
        {
            return id;
        }
    }
}

fn ending(content: &str) -> &'static str {
    match content.find('\n') {
        Some(index) if index > 0 && content.as_bytes()[index - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

fn add(
    dir: &Dir,
    document: NotesDocument,
    items: Vec<Item>,
    text: String,
    lane: Option<NotesLane>,
) -> Result<(bool, NotesResult), InspectionError> {
    let text = normalize_text(&text)?;
    if text.is_empty() {
        return Ok((
            false,
            listing(document.revision, items, NotesTodoFilter::All),
        ));
    }
    if items.len() == MAX_TODOS {
        return Err(fs::error(
            "notes_too_large",
            "Todos file contains 5000 tasks",
        ));
    }
    let id = new_id(&items);
    let eol = ending(&document.content);
    let lane = lane.map_or("", |lane| match lane {
        NotesLane::Backlog => " lane=backlog",
        NotesLane::Doing => " lane=doing",
    });
    let separator = if document.content.is_empty() || document.content.ends_with('\n') {
        ""
    } else {
        eol
    };
    let mut content = document.content.clone();
    content.push_str(&format!(
        "{separator}- [ ] {text} <!-- cockpit id={id}{lane} -->{eol}"
    ));
    checked_size(&content)?;
    let next = NotesDocument {
        revision: fs::revision(content.as_bytes()),
        content,
    };
    let parsed = parse(&next)?;
    let index = unique_index(&parsed, &id).map_err(|_| {
        fs::error(
            "notes_todo_malformed",
            "Cannot append a task inside an unterminated Markdown block",
        )
    })?;
    let todo = parsed.into_iter().nth(index).expect("new task").todo;
    fs::publish(dir, FILE, &document, &next.content, MAX_FILE)?;
    Ok((
        true,
        NotesResult::Todo {
            revision: next.revision,
            todo,
        },
    ))
}

enum Change {
    Text(Option<String>),
    Done(bool),
    Remove,
    Promote,
    Move(NotesColumn),
    Unboard,
}

fn mutate(
    dir: &Dir,
    document: NotesDocument,
    items: Vec<Item>,
    selector: NotesTodoSelector,
    change: Change,
) -> Result<(bool, NotesResult), InspectionError> {
    let index = resolve(&items, &document, &selector)?;
    let item = &items[index];
    if item.todo.problems.iter().any(|problem| {
        matches!(
            problem,
            NotesTodoProblem::MetadataMalformed | NotesTodoProblem::LazyContinuation
        )
    }) {
        return Err(fs::error(
            "notes_todo_malformed",
            "Todo ownership or metadata is malformed; edit the Markdown directly",
        ));
    }
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    if matches!(change, Change::Remove) {
        if item.has_children {
            return Err(fs::error(
                "notes_todo_has_children",
                "Cannot remove a todo containing nested tasks",
            ));
        }
        let line_start = document.content[..item.span.start]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        if !document.content[line_start..item.span.start]
            .trim_matches([' ', '\t'])
            .is_empty()
        {
            return Err(fs::error(
                "notes_todo_malformed",
                "Cannot remove a task with an unowned container prefix",
            ));
        }
        edits.push((item.span.clone(), String::new()));
        let content = splice(&document.content, edits);
        fs::publish(dir, FILE, &document, &content, MAX_FILE)?;
        return Ok((
            true,
            NotesResult::TodoRemoved {
                revision: fs::revision(content.as_bytes()),
            },
        ));
    }
    let mut done = item.todo.done;
    // None: leave metadata untouched; Some(None): remove even an invalid lane.
    let mut lane_change: Option<Option<NotesLane>> = None;
    match change {
        Change::Text(Some(text)) if text != item.todo.text => edits.push((item.text.clone(), text)),
        Change::Text(_) => {}
        Change::Done(value) => done = value,
        Change::Promote if item.todo.lane.is_none() => lane_change = Some(Some(NotesLane::Backlog)),
        Change::Promote => {}
        Change::Move(column) => {
            // An invalid lane can only be repaired by an explicit valid lane.
            let repair = item.todo.problems.contains(&NotesTodoProblem::UnknownLane)
                && column != NotesColumn::Done;
            if item.todo.lane.is_none() && !repair {
                return Err(fs::error("notes_not_on_board", "Todo is not on the board"));
            }
            match column {
                NotesColumn::Done => done = true,
                NotesColumn::Backlog => {
                    done = false;
                    lane_change = Some(Some(NotesLane::Backlog));
                }
                NotesColumn::Doing => {
                    done = false;
                    lane_change = Some(Some(NotesLane::Doing));
                }
            }
        }
        Change::Unboard => lane_change = Some(None),
        Change::Remove => unreachable!(),
    }
    if done != item.todo.done {
        edits.push((
            item.checkbox..item.checkbox + 1,
            if done { "x" } else { " " }.to_owned(),
        ));
    }
    let adopt = item.todo.id.is_none()
        || (matches!(selector, NotesTodoSelector::Ref { .. })
            && item.todo.problems.contains(&NotesTodoProblem::DuplicateId));
    let adopted_id = adopt.then(|| new_id(&items));
    let id = adopted_id
        .as_deref()
        .or(item.todo.id.as_deref())
        .expect("adopted or annotated task");
    if let Some(metadata) = &item.metadata {
        if adopt {
            edits.push((metadata.id_value.clone(), id.to_owned()));
        }
        if let Some(lane) = lane_change {
            match (lane, &metadata.lane_value, &metadata.lane_token) {
                (Some(lane), Some(value), _) if item.todo.lane != Some(lane) => {
                    edits.push((value.clone(), lane_name(lane).to_owned()))
                }
                (Some(lane), None, _) => edits.push((
                    metadata.insert..metadata.insert,
                    format!(" lane={}", lane_name(lane)),
                )),
                (None, _, Some(token)) => edits.push((token.clone(), String::new())),
                _ => {}
            }
        }
    } else {
        let lane = lane_change
            .flatten()
            .map_or(String::new(), |lane| format!(" lane={}", lane_name(lane)));
        edits.push((
            item.text.end..item.text.end,
            format!(" <!-- cockpit id={id}{lane} -->"),
        ));
    }
    if edits.is_empty() {
        return Ok((
            false,
            NotesResult::Todo {
                revision: document.revision,
                todo: items.into_iter().nth(index).expect("selected task").todo,
            },
        ));
    }
    let content = splice(&document.content, edits);
    checked_size(&content)?;
    let next = NotesDocument {
        revision: fs::revision(content.as_bytes()),
        content,
    };
    let parsed = parse(&next)?;
    if parsed.len() != items.len() || parsed[index].todo.id.as_deref() != Some(id) {
        return Err(fs::error(
            "notes_todo_malformed",
            "Mutation changed Markdown task ownership",
        ));
    }
    let todo = parsed
        .into_iter()
        .nth(index)
        .ok_or_else(|| {
            fs::error(
                "notes_todo_malformed",
                "Mutation changed Markdown task ownership",
            )
        })?
        .todo;
    fs::publish(dir, FILE, &document, &next.content, MAX_FILE)?;
    Ok((
        true,
        NotesResult::Todo {
            revision: next.revision,
            todo,
        },
    ))
}

fn lane_name(lane: NotesLane) -> &'static str {
    match lane {
        NotesLane::Backlog => "backlog",
        NotesLane::Doing => "doing",
    }
}

fn checked_size(content: &str) -> Result<(), InspectionError> {
    if content.len() > MAX_FILE {
        Err(fs::error("notes_too_large", "Todos file exceeds 1 MiB"))
    } else {
        Ok(())
    }
}

fn splice(content: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(range, _)| (range.start, range.end));
    let capacity = edits.iter().fold(content.len(), |size, (range, value)| {
        size - range.len() + value.len()
    });
    let mut out = String::with_capacity(capacity);
    let mut end = 0;
    for (range, value) in edits {
        debug_assert!(range.start >= end, "owned edits must not overlap");
        out.push_str(&content[end..range.start]);
        out.push_str(&value);
        end = range.end;
    }
    out.push_str(&content[end..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Fixture {
        path: PathBuf,
        dir: Dir,
    }

    impl Fixture {
        fn new(content: Option<&str>) -> Self {
            let path = std::env::temp_dir().join(format!("cockpit-notes-todos-{}", Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            if let Some(content) = content {
                std::fs::write(path.join(FILE), content).unwrap();
            }
            let dir = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
            Self { path, dir }
        }

        fn run(&self, op: NotesOperation) -> Result<(bool, NotesResult), InspectionError> {
            execute(&self.dir, &self.path, op)
        }

        fn todos(&self) -> Vec<NotesTodo> {
            match self
                .run(NotesOperation::TodoList {
                    filter: NotesTodoFilter::All,
                })
                .unwrap()
                .1
            {
                NotesResult::Todos { todos, .. } => todos,
                _ => panic!("expected todos"),
            }
        }

        fn content(&self) -> String {
            std::fs::read_to_string(self.path.join(FILE)).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn id(todo: &NotesTodo) -> NotesTodoSelector {
        NotesTodoSelector::Id {
            id: todo.id.clone().unwrap(),
            expected_revision: todo.revision.clone(),
        }
    }

    fn reference(todo: &NotesTodo) -> NotesTodoSelector {
        NotesTodoSelector::Ref {
            reference: todo.reference.clone(),
        }
    }

    fn returned(result: (bool, NotesResult)) -> NotesTodo {
        assert!(result.0);
        match result.1 {
            NotesResult::Todo { todo, .. } => todo,
            _ => panic!("expected todo"),
        }
    }

    fn document(content: &str) -> NotesDocument {
        NotesDocument {
            content: content.to_owned(),
            revision: fs::revision(content.as_bytes()),
        }
    }

    #[test]
    fn recognizes_initial_paragraph_markers_but_not_descendants_or_fences() {
        let content = "- [ ] Loose 🧭 <!-- cockpit id=loose -->\n\n  Owned paragraph.\n\n- Ordinary parent\n  - [X] Child <!-- cockpit id=child -->\n\n```md\n- [ ] Not a task\n```\n\n1. [ ] Ordered <!-- cockpit id=ordered -->\n";
        let items = parse(&document(content)).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(
            items
                .iter()
                .map(|item| item.todo.id.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["loose", "child", "ordered"]
        );
        assert_eq!(items[1].todo.depth, 1);
        assert!(items[1].todo.done);
        assert!(items.iter().all(|item| item.todo.problems.is_empty()));
        assert!(content[items[0].span.clone()].contains("Owned paragraph."));
    }

    #[test]
    fn removes_only_owned_unicode_crlf_item_and_preserves_one_space_sibling() {
        let content = "# Todos\r\n- [ ] Task 🧭 <!-- cockpit id=a -->\r\n - Keep this unrelated note\r\n\r\nProse 🌏";
        let fixture = Fixture::new(Some(content));
        let todo = fixture.todos().remove(0);
        let parsed = parse(&document(content)).unwrap();
        assert_eq!(
            &content[parsed[0].span.clone()],
            "- [ ] Task 🧭 <!-- cockpit id=a -->\r\n"
        );
        assert_eq!(
            todo.revision,
            fs::revision(content[parsed[0].span.clone()].as_bytes())
        );
        let (changed, result) = fixture
            .run(NotesOperation::TodoRemove { todo: id(&todo) })
            .unwrap();
        assert!(changed);
        assert!(matches!(result, NotesResult::TodoRemoved { .. }));
        assert_eq!(
            fixture.content(),
            "# Todos\r\n - Keep this unrelated note\r\n\r\nProse 🌏"
        );
    }

    #[test]
    fn edits_only_first_line_slots_and_retains_metadata_spacing_unknown_keys() {
        let content = "- [X]\tOld 🧭  <!-- cockpit  extra=🦀\tid=a lane=doing  future=v --> \r\n  Rationale 🧠\r\n - sibling\r\n";
        let fixture = Fixture::new(Some(content));
        let todo = fixture.todos().remove(0);
        let edited = returned(
            fixture
                .run(NotesOperation::TodoUpdate {
                    todo: id(&todo),
                    text: Some("New 🌏\r\ntext".into()),
                })
                .unwrap(),
        );
        assert_eq!(edited.text, "New 🌏 text");
        assert_eq!(fixture.content(), content.replace("Old 🧭", "New 🌏 text"));
        let reopened = returned(
            fixture
                .run(NotesOperation::TodoSetDone {
                    todo: id(&edited),
                    done: false,
                })
                .unwrap(),
        );
        assert_eq!(reopened.lane, Some(NotesLane::Doing));
        assert_eq!(
            fixture.content(),
            content
                .replace("Old 🧭", "New 🌏 text")
                .replacen("[X]", "[ ]", 1)
        );
        let unboarded = returned(
            fixture
                .run(NotesOperation::KanbanUnboard {
                    todo: id(&reopened),
                })
                .unwrap(),
        );
        assert_eq!(unboarded.lane, None);
        assert!(fixture.content().contains("extra=🦀\tid=a  future=v"));
        assert!(
            fixture
                .content()
                .ends_with("  Rationale 🧠\r\n - sibling\r\n")
        );
    }

    #[test]
    fn item_cas_accepts_sibling_edit_but_ref_cas_rejects_it() {
        let content = "- [ ] Target <!-- cockpit id=a -->\n - unrelated note\n";
        let fixture = Fixture::new(Some(content));
        let original = fixture.todos().remove(0);
        std::fs::write(
            fixture.path.join(FILE),
            content.replace("unrelated", "externally edited"),
        )
        .unwrap();
        assert_eq!(fixture.todos()[0].revision, original.revision);
        let error = fixture
            .run(NotesOperation::TodoUpdate {
                todo: reference(&original),
                text: None,
            })
            .unwrap_err();
        assert_eq!(error.code, "notes_conflict");
        let updated = returned(
            fixture
                .run(NotesOperation::TodoSetDone {
                    todo: id(&original),
                    done: true,
                })
                .unwrap(),
        );
        assert!(updated.done);
        assert!(fixture.content().contains("externally edited note"));
        assert_eq!(
            fixture
                .run(NotesOperation::TodoSetDone {
                    todo: id(&original),
                    done: true
                })
                .unwrap_err()
                .code,
            "notes_conflict"
        );
        assert!(
            !fixture
                .run(NotesOperation::TodoSetDone {
                    todo: id(&updated),
                    done: true
                })
                .unwrap()
                .0
        );
    }

    #[test]
    fn owned_continuation_edit_invalidates_item_revision() {
        let content = "- [ ] Task <!-- cockpit id=a -->\n  Rationale\n- other\n";
        let fixture = Fixture::new(Some(content));
        let todo = fixture.todos().remove(0);
        std::fs::write(
            fixture.path.join(FILE),
            content.replace("Rationale", "Changed rationale"),
        )
        .unwrap();
        assert_eq!(
            fixture
                .run(NotesOperation::TodoRemove { todo: id(&todo) })
                .unwrap_err()
                .code,
            "notes_conflict"
        );
        assert!(fixture.content().contains("Changed rationale"));
    }

    #[test]
    fn duplicate_ids_are_ambiguous_and_ref_repair_preserves_unknown_fields() {
        let content = "- [ ] First <!-- cockpit id=same custom=one -->\r\n- [ ] Second <!-- cockpit id=same\tfuture=🧭 -->\r\n";
        let fixture = Fixture::new(Some(content));
        let todos = fixture.todos();
        assert!(
            todos
                .iter()
                .all(|todo| todo.problems.contains(&NotesTodoProblem::DuplicateId))
        );
        assert_eq!(
            require_unique_id(&fixture.dir, "same").unwrap_err().code,
            "notes_todo_ambiguous"
        );
        assert_eq!(
            fixture
                .run(NotesOperation::TodoUpdate {
                    todo: id(&todos[1]),
                    text: None
                })
                .unwrap_err()
                .code,
            "notes_todo_ambiguous"
        );
        let repaired = returned(
            fixture
                .run(NotesOperation::TodoUpdate {
                    todo: reference(&todos[1]),
                    text: None,
                })
                .unwrap(),
        );
        let fresh = repaired.id.as_deref().unwrap();
        assert_eq!(fresh.len(), 10);
        assert!(
            fresh
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        );
        assert_ne!(fresh, "same");
        assert!(repaired.problems.is_empty());
        assert_eq!(
            fixture.content(),
            content.replacen("id=same\tfuture", &format!("id={fresh}\tfuture"), 1)
        );
        require_unique_id(&fixture.dir, "same").unwrap();
        require_unique_id(&fixture.dir, fresh).unwrap();
        assert_eq!(
            require_unique_id(&fixture.dir, "missing").unwrap_err().code,
            "notes_not_found"
        );
        assert_eq!(
            require_unique_id(&fixture.dir, "../escape")
                .unwrap_err()
                .code,
            "notes_invalid_input"
        );
    }

    #[test]
    fn read_does_not_adopt_and_ref_mutation_adopts_once() {
        let content = "- [ ] Hand typed 🧭\r\n- sibling\r\n";
        let fixture = Fixture::new(Some(content));
        let todo = fixture.todos().remove(0);
        assert_eq!(todo.id, None);
        assert_eq!(fixture.content(), content);
        let adopted = returned(
            fixture
                .run(NotesOperation::TodoUpdate {
                    todo: reference(&todo),
                    text: None,
                })
                .unwrap(),
        );
        assert!(adopted.id.is_some());
        assert!(
            fixture
                .content()
                .starts_with("- [ ] Hand typed 🧭 <!-- cockpit id=")
        );
        assert!(fixture.content().ends_with(" -->\r\n- sibling\r\n"));
        assert!(
            !fixture
                .run(NotesOperation::TodoUpdate {
                    todo: id(&adopted),
                    text: None
                })
                .unwrap()
                .0
        );
    }

    #[test]
    fn unknown_lane_is_preserved_until_explicit_lane_repair() {
        let content = "- [ ] Task <!-- cockpit id=a lane=Doing unknown=🌏 -->\n";
        let fixture = Fixture::new(Some(content));
        let todo = fixture.todos().remove(0);
        assert_eq!(todo.lane, None);
        assert!(todo.problems.contains(&NotesTodoProblem::UnknownLane));
        let checked = returned(
            fixture
                .run(NotesOperation::TodoSetDone {
                    todo: id(&todo),
                    done: true,
                })
                .unwrap(),
        );
        assert!(fixture.content().contains("lane=Doing unknown=🌏"));
        assert_eq!(
            fixture
                .run(NotesOperation::KanbanMove {
                    todo: id(&checked),
                    to: NotesColumn::Done
                })
                .unwrap_err()
                .code,
            "notes_not_on_board"
        );
        let repaired = returned(
            fixture
                .run(NotesOperation::KanbanMove {
                    todo: id(&checked),
                    to: NotesColumn::Doing,
                })
                .unwrap(),
        );
        assert_eq!(repaired.lane, Some(NotesLane::Doing));
        assert!(!repaired.done);
        assert!(repaired.problems.is_empty());
        assert_eq!(
            fixture.content(),
            content.replace("lane=Doing", "lane=doing")
        );
        let fixture = Fixture::new(Some(content));
        let todo = fixture.todos().remove(0);
        returned(
            fixture
                .run(NotesOperation::KanbanUnboard { todo: id(&todo) })
                .unwrap(),
        );
        assert_eq!(fixture.content(), content.replace(" lane=Doing", ""));
    }

    #[test]
    fn lazy_continuation_and_malformed_metadata_refuse_mutations() {
        for content in [
            "- [ ] Task <!-- cockpit id=a -->\nlazy continuation 🧭\n- sibling\n",
            "- [ ] Task <!-- cockpit id=a --> <!-- cockpit id=b -->\n",
            "- [ ] Task <!-- cockpit id=a badtoken -->\n",
            "- [ ] Task <!-- cockpit id=a lane=doing lane=backlog -->\n",
            "- [ ] Task <!-- cockpit id=../bad -->\n",
            "- [ ] Task <!-- cockpit id=a\n",
        ] {
            let fixture = Fixture::new(Some(content));
            let todo = fixture.todos().remove(0);
            assert!(!todo.problems.is_empty(), "{content}");
            assert_eq!(
                fixture
                    .run(NotesOperation::TodoSetDone {
                        todo: reference(&todo),
                        done: true
                    })
                    .unwrap_err()
                    .code,
                "notes_todo_malformed",
                "{content}"
            );
            assert_eq!(
                fixture
                    .run(NotesOperation::TodoRemove {
                        todo: reference(&todo)
                    })
                    .unwrap_err()
                    .code,
                "notes_todo_malformed",
                "{content}"
            );
            assert_eq!(fixture.content(), content);
        }
    }

    #[test]
    fn nested_parent_remove_is_refused_child_remove_preserves_sibling_indentation() {
        let content = "- [ ] Parent <!-- cockpit id=parent -->\r\n  - [ ] Child 🧭 <!-- cockpit id=child -->\r\n  - Keep nested sibling\r\n- Keep top sibling\r\n";
        let fixture = Fixture::new(Some(content));
        let todos = fixture.todos();
        assert_eq!(
            fixture
                .run(NotesOperation::TodoRemove {
                    todo: id(&todos[0])
                })
                .unwrap_err()
                .code,
            "notes_todo_has_children"
        );
        assert_eq!(fixture.content(), content);
        assert!(
            fixture
                .run(NotesOperation::TodoRemove {
                    todo: id(&todos[1])
                })
                .unwrap()
                .0
        );
        assert_eq!(
            fixture.content(),
            "- [ ] Parent <!-- cockpit id=parent -->\r\n  - Keep nested sibling\r\n- Keep top sibling\r\n"
        );
    }

    #[test]
    fn quoted_remove_refuses_unowned_prefix_but_checkbox_edit_is_safe() {
        let content = "> - [ ] Task <!-- cockpit id=a -->\n> - sibling\n";
        let fixture = Fixture::new(Some(content));
        let todo = fixture.todos().remove(0);
        assert!(todo.problems.is_empty());
        assert_eq!(
            fixture
                .run(NotesOperation::TodoRemove { todo: id(&todo) })
                .unwrap_err()
                .code,
            "notes_todo_malformed"
        );
        assert_eq!(fixture.content(), content);
        returned(
            fixture
                .run(NotesOperation::TodoSetDone {
                    todo: id(&todo),
                    done: true,
                })
                .unwrap(),
        );
        assert_eq!(fixture.content(), content.replacen("[ ]", "[x]", 1));
    }

    #[test]
    fn board_columns_keep_file_order_and_done_keeps_previous_lane() {
        let content = "- [ ] Off <!-- cockpit id=off -->\n- [ ] B2 <!-- cockpit id=b2 lane=backlog -->\n- [x] D2 <!-- cockpit id=d2 lane=doing -->\n- [ ] B1 <!-- cockpit id=b1 lane=backlog -->\n- [x] D1 <!-- cockpit id=d1 lane=backlog -->\n- [ ] W <!-- cockpit id=w lane=doing -->\n";
        let fixture = Fixture::new(Some(content));
        let todos = fixture.todos();
        let columns = match fixture.run(NotesOperation::KanbanList).unwrap().1 {
            NotesResult::Board { columns, .. } => columns,
            _ => panic!("board"),
        };
        assert_eq!(
            columns
                .backlog
                .iter()
                .map(|todo| todo.text.as_str())
                .collect::<Vec<_>>(),
            ["B2", "B1"]
        );
        assert_eq!(
            columns
                .done
                .iter()
                .map(|todo| todo.text.as_str())
                .collect::<Vec<_>>(),
            ["D2", "D1"]
        );
        assert_eq!(columns.doing[0].text, "W");
        assert_eq!(
            fixture
                .run(NotesOperation::KanbanMove {
                    todo: id(&todos[0]),
                    to: NotesColumn::Doing
                })
                .unwrap_err()
                .code,
            "notes_not_on_board"
        );
        let promoted = returned(
            fixture
                .run(NotesOperation::KanbanPromote {
                    todo: id(&todos[0]),
                })
                .unwrap(),
        );
        assert_eq!(promoted.lane, Some(NotesLane::Backlog));
        assert!(
            !fixture
                .run(NotesOperation::KanbanPromote {
                    todo: id(&promoted)
                })
                .unwrap()
                .0
        );
        let done = returned(
            fixture
                .run(NotesOperation::KanbanMove {
                    todo: id(&todos[5]),
                    to: NotesColumn::Done,
                })
                .unwrap(),
        );
        assert_eq!(done.lane, Some(NotesLane::Doing));
        assert!(done.done);
        assert!(
            !fixture
                .run(NotesOperation::KanbanMove {
                    todo: id(&done),
                    to: NotesColumn::Done
                })
                .unwrap()
                .0
        );
    }

    #[test]
    fn append_uses_first_line_ending_and_inserts_missing_final_newline() {
        let fixture = Fixture::new(Some("# Heading\r\nProse 🧭"));
        let todo = returned(
            fixture
                .run(NotesOperation::TodoAdd {
                    text: "  New\r\nline  ".into(),
                    lane: Some(NotesLane::Backlog),
                })
                .unwrap(),
        );
        assert_eq!(todo.text, "New line");
        assert_eq!(todo.lane, Some(NotesLane::Backlog));
        assert_eq!(todo.line, 3);
        assert_eq!(
            fixture.content(),
            format!(
                "# Heading\r\nProse 🧭\r\n- [ ] New line <!-- cockpit id={} lane=backlog -->\r\n",
                todo.id.unwrap()
            )
        );
        let empty = Fixture::new(None);
        assert!(
            !empty
                .run(NotesOperation::TodoAdd {
                    text: " \r\n ".into(),
                    lane: None
                })
                .unwrap()
                .0
        );
        assert!(!empty.path.join(FILE).exists());
        let unterminated = Fixture::new(Some("```md\ninside\n"));
        assert_eq!(
            unterminated
                .run(NotesOperation::TodoAdd {
                    text: "Task".into(),
                    lane: None
                })
                .unwrap_err()
                .code,
            "notes_todo_malformed"
        );
        assert_eq!(unterminated.content(), "```md\ninside\n");
    }

    #[test]
    fn limits_and_invalid_text_fail_without_writing() {
        let fixture = Fixture::new(Some("- [ ] Task <!-- cockpit id=a -->\n"));
        let todo = fixture.todos().remove(0);
        let before = fixture.content();
        for text in [
            "🧭".repeat(513),
            "bad <!-- comment".into(),
            "bad --> comment".into(),
        ] {
            let expected = if text.len() > MAX_TEXT {
                "notes_too_large"
            } else {
                "notes_invalid_input"
            };
            assert_eq!(
                fixture
                    .run(NotesOperation::TodoUpdate {
                        todo: id(&todo),
                        text: Some(text.clone())
                    })
                    .unwrap_err()
                    .code,
                expected
            );
            assert_eq!(
                fixture
                    .run(NotesOperation::TodoAdd { text, lane: None })
                    .unwrap_err()
                    .code,
                expected
            );
            assert_eq!(fixture.content(), before);
        }
        let boundary = returned(
            fixture
                .run(NotesOperation::TodoUpdate {
                    todo: id(&todo),
                    text: Some("🧭".repeat(512)),
                })
                .unwrap(),
        );
        assert_eq!(boundary.text.len(), MAX_TEXT);
        let tasks = "- [ ] Task\n".repeat(MAX_TODOS);
        let full = Fixture::new(Some(&tasks));
        assert_eq!(full.todos().len(), MAX_TODOS);
        assert_eq!(
            full.run(NotesOperation::TodoAdd {
                text: "Overflow".into(),
                lane: None
            })
            .unwrap_err()
            .code,
            "notes_too_large"
        );
        assert_eq!(full.content(), tasks);
        let too_many = format!("{tasks}- [ ] Overflow\n");
        assert_eq!(
            parse(&document(&too_many)).err().unwrap().code,
            "notes_too_large"
        );
        let bytes = format!("# {}\n", "x".repeat(MAX_FILE - 3));
        let full = Fixture::new(Some(&bytes));
        assert_eq!(
            full.run(NotesOperation::TodoAdd {
                text: "Overflow".into(),
                lane: None
            })
            .unwrap_err()
            .code,
            "notes_too_large"
        );
        assert_eq!(full.content(), bytes);
        assert_eq!(
            parse(&document(&"x".repeat(MAX_FILE + 1)))
                .err()
                .unwrap()
                .code,
            "notes_too_large"
        );
    }

    #[test]
    fn filters_are_stable_and_other_domain_operations_return_usage() {
        let fixture = Fixture::new(Some("- [x] First\n- [ ] Second\n- [X] Third\n"));
        match fixture
            .run(NotesOperation::TodoList {
                filter: NotesTodoFilter::Done,
            })
            .unwrap()
            .1
        {
            NotesResult::Todos { todos, .. } => assert_eq!(
                todos
                    .iter()
                    .map(|todo| todo.text.as_str())
                    .collect::<Vec<_>>(),
                ["First", "Third"]
            ),
            _ => panic!("todos"),
        }
        assert_eq!(
            fixture
                .run(NotesOperation::ScratchpadRead)
                .unwrap_err()
                .code,
            "notes_usage"
        );
    }
}
