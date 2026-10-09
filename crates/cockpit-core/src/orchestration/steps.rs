use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
    ops::Range,
};

use cockpit_protocol::orchestration::{TaskStep, TaskStepProgress, TaskStepScope, TaskStepStatus};
use uuid::Uuid;

use super::tasks_md::BytePatch;
use crate::InspectionError;

mod parser;

use parser::{collect_events, EventInventory};

pub(crate) const MAX_TRACKED_STEPS: usize = 64;
pub(crate) const MAX_STEP_DEPTH: u32 = 4;
pub(crate) const MAX_STEP_TITLE_SCALARS: usize = 200;
const BEGIN: &str = "<!-- cockpit-checklist: begin -->";
const END: &str = "<!-- cockpit-checklist: end -->";
const STEP: &str = "<!-- cockpit-step:";

#[derive(Clone, Copy)]
pub(crate) enum DocumentEol {
    Lf,
    CrLf,
}
impl DocumentEol {
    fn text(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

pub(crate) struct StepParseContext {
    pub item_range: Range<usize>,
    // One-based original document line, supplied by TaskDocument's line inventory.
    pub item_line: usize,
    pub continuation_range: Range<usize>,
    pub top_checkbox_offset: usize,
    pub relation_record_range: Option<Range<usize>>,
    pub eol: DocumentEol,
    pub continuation_write_limit: usize,
}

pub(crate) struct ManagedStepTail {
    pub range: Range<usize>,
    pub end_marker_range: Range<usize>,
    pub root_append_offset: usize,
}

pub(crate) struct StepLayout {
    nodes: Vec<StepNode>,
    by_id: HashMap<Uuid, usize>,
    protected_ranges: Vec<Range<usize>>,
    managed_tail: Option<ManagedStepTail>,
    diagnostics: Vec<String>,
    identity_invalid: bool,
    hierarchy_invalid: bool,
    tail_creation_gap: Option<usize>,
    limits_exceeded: bool,
}

struct StepNode {
    step_id: Option<Uuid>,
    parent_index: Option<usize>,
    depth: u32,
    subtree_end_index: usize,
    line: u32,
    header_range: Range<usize>,
    title_range: Range<usize>,
    checkbox_offset: usize,
    marker_range: Option<Range<usize>>,
    subtree_range: Option<Range<usize>>,
    checked: bool,
    status: TaskStepStatus,
    diagnostic: Option<String>,
    parser_range: Range<usize>,
    physical_line: usize,
    title_scalars: usize,
    // Tabs remain readable, but never become guessed reindent columns.
    indent: Option<usize>,
    child_indent: Option<usize>,
}

pub(crate) struct StepProjection {
    pub steps: Vec<TaskStep>,
    pub progress: Option<TaskStepProgress>,
    pub diagnostic: Option<String>,
}

pub(crate) enum StepIntent<'a> {
    Add {
        step_id: Uuid,
        parent_step_id: Option<Uuid>,
        before_step_id: Option<Uuid>,
        title: &'a str,
    },
    Rename {
        step_id: Uuid,
        title: &'a str,
    },
    SetChecked {
        step_id: Uuid,
        checked: bool,
        scope: TaskStepScope,
    },
    Move {
        step_id: Uuid,
        parent_step_id: Option<Uuid>,
        before_step_id: Option<Uuid>,
    },
    Remove {
        step_id: Uuid,
    },
}

fn error(code: &'static str, message: impl Into<String>) -> InspectionError {
    InspectionError::new(code, message)
}
fn invalid_patch() -> InspectionError {
    error(
        "task_step_patch_invalid",
        "step patches must own disjoint, positively proved checklist slots",
    )
}
fn contains(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.start <= inner.end && inner.end <= outer.end
}
fn context_valid(source: &str, context: &StepParseContext) -> bool {
    let valid = |r: &Range<usize>| {
        r.start <= r.end
            && r.end <= source.len()
            && source.is_char_boundary(r.start)
            && source.is_char_boundary(r.end)
    };
    valid(&context.item_range)
        && valid(&context.continuation_range)
        && contains(&context.item_range, &context.continuation_range)
        && context.item_range.contains(&context.top_checkbox_offset)
        && context.top_checkbox_offset < context.continuation_range.start
        && matches!(
            source.as_bytes()[context.top_checkbox_offset],
            b' ' | b'x' | b'X'
        )
        && context
            .relation_record_range
            .as_ref()
            .is_none_or(|r| valid(r) && contains(&context.continuation_range, r))
}

#[derive(Clone)]
struct Line {
    start: usize,
    content_end: usize,
    end: usize,
}
fn lines(source: &str, range: Range<usize>) -> Vec<Line> {
    let mut result = Vec::new();
    let mut start = range.start;
    for (offset, byte) in source.as_bytes()[range.clone()].iter().enumerate() {
        if *byte == b'\n' {
            let newline = range.start + offset;
            let content_end = if newline > start && source.as_bytes()[newline - 1] == b'\r' {
                newline - 1
            } else {
                newline
            };
            result.push(Line {
                start,
                content_end,
                end: newline + 1,
            });
            start = newline + 1;
        }
    }
    if start < range.end {
        result.push(Line {
            start,
            content_end: range.end,
            end: range.end,
        });
    }
    result
}
fn blank(source: &str, range: Range<usize>) -> bool {
    source.as_bytes()[range].iter().all(u8::is_ascii_whitespace)
}
fn union_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_unstable_by_key(|r| (r.start, r.end));
    let mut result: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = result.last_mut() {
            if range.start <= last.end {
                last.end = last.end.max(range.end);
                continue;
            }
        }
        result.push(range);
    }
    result
}
fn diagnose(layout: &mut StepLayout, message: &str) {
    if !layout.diagnostics.iter().any(|d| d == message) {
        layout.diagnostics.push(message.to_owned());
    }
}
fn node_diagnostic(node: &mut StepNode, message: &str) {
    match &mut node.diagnostic {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(message);
        }
        None => node.diagnostic = Some(message.to_owned()),
    }
}

struct Frame {
    range: Range<usize>,
    node: Option<usize>,
    top: bool,
    nearest_node: Option<usize>,
    under_top: bool,
    ordinary_ancestor: bool,
}
struct Example {
    range: Range<usize>,
    terminal_open: bool,
}

/// There is exactly one offset inventory. All subsequent scans use its original
/// physical line/event spans, never a decoded/deindented body coordinate system.
pub(crate) fn parse(
    source: &str,
    context: &StepParseContext,
) -> Result<StepLayout, InspectionError> {
    if !context_valid(source, context) {
        return Err(invalid_patch());
    }
    let mut layout = StepLayout {
        nodes: Vec::new(),
        by_id: HashMap::new(),
        protected_ranges: Vec::new(),
        managed_tail: None,
        diagnostics: Vec::new(),
        identity_invalid: false,
        hierarchy_invalid: false,
        tail_creation_gap: None,
        limits_exceeded: false,
    };
    let physical = lines(source, context.item_range.clone());
    let EventInventory {
        examples,
        inline_code,
        html_tokens,
    } = collect_events(source, context, &physical, &mut layout)?;
    let example_ranges = union_ranges(examples.iter().map(|e| e.range.clone()).collect());
    let in_example =
        |offset: usize| range_at(&example_ranges, offset) || range_at(&inline_code, offset);
    let header_lines: HashSet<_> = layout.nodes.iter().map(|n| n.header_range.start).collect();
    // Metadata must be a trailing, real HTML token, not an inline-code example.
    for node in &mut layout.nodes {
        let title = &source[node.title_range.clone()];
        let mut markers = title.match_indices(STEP).filter_map(|(offset, _)| {
            let offset = node.title_range.start + offset;
            (!range_at(&inline_code, offset)).then_some(offset)
        });
        if let Some(start) = markers.next() {
            let raw = source[start..node.title_range.end].trim_end();
            let parsed = raw
                .strip_prefix(STEP)
                .and_then(|r| r.strip_suffix("-->"))
                .and_then(|id| Uuid::parse_str(id.trim()).ok());
            if markers.next().is_none()
                && parsed.is_some()
                && html_tokens
                    .get(&start)
                    .is_some_and(|&end| end >= node.title_range.end)
            {
                node.step_id = parsed;
                node.marker_range = Some(start..node.title_range.end);
                node.title_range.end = start;
                while node.title_range.end > node.title_range.start
                    && matches!(source.as_bytes()[node.title_range.end - 1], b' ' | b'\t')
                {
                    node.title_range.end -= 1;
                }
            } else {
                node_diagnostic(node, "malformed or misplaced cockpit-step UUID marker");
                layout.identity_invalid = true;
            }
        }
        if node.title_range.is_empty() {
            node_diagnostic(node, "step title is empty");
            layout.identity_invalid = true;
        }
        node.title_scalars = source[node.title_range.clone()].chars().count();
    }
    // Preorder intervals and derived leaf truth do not rely on persisted branch bytes.
    for index in (0..layout.nodes.len()).rev() {
        if let Some(parent) = layout.nodes[index].parent_index {
            layout.nodes[parent].subtree_end_index = layout.nodes[parent]
                .subtree_end_index
                .max(layout.nodes[index].subtree_end_index);
        }
    }
    let model = model(&layout);
    let summaries = summarize(&model)?;
    for (index, node) in layout.nodes.iter_mut().enumerate() {
        node.status = summaries[index].status();
    }

    let mut controls: Vec<(bool, Range<usize>)> = Vec::new();
    for line in &physical {
        if line.start < context.continuation_range.start {
            continue;
        }
        let raw = &source[line.start..line.content_end];
        let trimmed = raw.trim_start_matches([' ', '\t']);
        let offset = line.start + raw.len() - trimmed.len();
        if in_example(offset) {
            continue;
        }
        if raw
            .match_indices("<!-- cockpit-checklist:")
            .any(|(at, _)| !in_example(line.start + at))
        {
            layout.protected_ranges.push(line.start..line.end);
            if raw.strip_prefix("  ") == Some(BEGIN) {
                controls.push((true, line.start..line.end));
            } else if raw.strip_prefix("  ") == Some(END) {
                controls.push((false, line.start..line.end));
            } else {
                diagnose(&mut layout, "malformed checklist boundary");
                layout.hierarchy_invalid = true;
            }
        }
        for (relative, _) in raw.match_indices(STEP) {
            let offset = line.start + relative;
            if !in_example(offset) && !header_lines.contains(&line.start) {
                layout.protected_ranges.push(line.start..line.end);
                diagnose(
                    &mut layout,
                    "cockpit-step marker is outside a checkbox header",
                );
                layout.identity_invalid = true;
            }
        }
    }
    if !controls.is_empty() {
        if controls.len() == 2
            && controls[0].0
            && !controls[1].0
            && controls[0].1.end <= controls[1].1.start
        {
            let begin = controls[0].1.clone();
            let end = controls[1].1.clone();
            let inner = begin.end..end.start;
            let pure = pure_headers(source, &layout.nodes, inner.clone())
                && layout
                    .nodes
                    .iter()
                    .filter(|n| inner.contains(&n.header_range.start))
                    .all(|n| {
                        contains(&inner, &n.header_range)
                            && n.parent_index
                                .is_none_or(|p| inner.contains(&layout.nodes[p].header_range.start))
                    });
            if pure
                && blank(source, end.end..context.continuation_range.end)
                && !layout.hierarchy_invalid
            {
                layout.managed_tail = Some(ManagedStepTail {
                    range: begin.start..end.end,
                    end_marker_range: end.clone(),
                    root_append_offset: end.start,
                });
            } else {
                diagnose(
                    &mut layout,
                    "checklist tail contains unrelated text, ambiguous ancestry, or following prose",
                );
                layout.hierarchy_invalid = true;
            }
        } else {
            diagnose(
                &mut layout,
                "multiple, nested, or unclosed checklist boundaries",
            );
            layout.hierarchy_invalid = true;
        }
    }
    let mut nonheader = Vec::with_capacity(physical.len() + 1);
    nonheader.push(0usize);
    let mut next_nonwhite = vec![context.item_range.end; physical.len() + 1];
    for line in &physical {
        let bad = !header_lines.contains(&line.start) && !blank(source, line.start..line.end);
        nonheader.push(nonheader.last().copied().unwrap_or(0) + usize::from(bad));
    }
    for (index, line) in physical.iter().enumerate().rev() {
        next_nonwhite[index] = source.as_bytes()[line.start..line.end]
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .map_or(next_nonwhite[index + 1], |at| line.start + at);
    }
    // Whole-subtree ownership requires only discovered checkbox headers between
    // its endpoints, and no unowned continuation prose inside the CommonMark item.
    for index in 0..layout.nodes.len() {
        let end_index = layout.nodes[index].subtree_end_index;
        let range =
            layout.nodes[index].header_range.start..layout.nodes[end_index - 1].header_range.end;
        let parser_end = layout.nodes[index]
            .parser_range
            .end
            .min(context.item_range.end);
        let start_line = layout.nodes[index].physical_line;
        let last_line = layout.nodes[end_index - 1].physical_line;
        let next_text = next_nonwhite[last_line + 1];
        let suffix_safe = parser_end <= range.end
            || next_text >= parser_end
            || layout.managed_tail.as_ref().is_some_and(|tail| {
                tail.end_marker_range.contains(&next_text) && parser_end <= tail.range.end
            });
        if nonheader[last_line + 1] == nonheader[start_line]
            && suffix_safe
            && !layout.hierarchy_invalid
        {
            layout.nodes[index].subtree_range = Some(range);
        } else {
            node_diagnostic(
                &mut layout.nodes[index],
                "subtree is interleaved with unowned text or has unsafe physical bounds",
            );
        }
    }
    for index in 0..layout.nodes.len() {
        if let Some(id) = layout.nodes[index].step_id {
            if let Some(previous) = layout.by_id.insert(id, index) {
                node_diagnostic(&mut layout.nodes[index], "duplicate step UUID");
                node_diagnostic(&mut layout.nodes[previous], "duplicate step UUID");
                layout.identity_invalid = true;
            }
        }
    }
    if layout.identity_invalid {
        diagnose(
            &mut layout,
            "step identities are malformed or duplicated; correct canonical source",
        );
    }
    if layout.hierarchy_invalid {
        diagnose(
            &mut layout,
            "checklist hierarchy or managed boundaries are ambiguous",
        );
    }
    let mut unsafe_bounds = false;
    for node in &layout.nodes {
        layout.protected_ranges.push(node.header_range.clone());
        if let Some(range) = &node.subtree_range {
            layout.protected_ranges.push(range.clone());
        } else {
            unsafe_bounds = true;
        }
    }
    if unsafe_bounds {
        diagnose(
            &mut layout,
            "some checkbox subtrees cannot be safely moved or removed",
        );
    }
    let limits = violations(context, &model, context.continuation_range.len());
    layout.limits_exceeded = limits.iter().any(|&value| value > 0);
    if layout.limits_exceeded {
        diagnose(
            &mut layout,
            "saved checklist exceeds count, depth, title, or continuation byte limits; safe non-growing edits remain available",
        );
    }
    if controls.is_empty() && !layout.identity_invalid && !layout.hierarchy_invalid {
        let last_nonwhite = source[context.continuation_range.clone()].trim_end().len()
            + context.continuation_range.start;
        if !examples
            .iter()
            .any(|e| e.terminal_open && e.range.end >= last_nonwhite)
        {
            layout.tail_creation_gap = Some(context.continuation_range.end);
        }
    }
    layout.protected_ranges = union_ranges(layout.protected_ranges);
    Ok(layout)
}

fn range_at(ranges: &[Range<usize>], offset: usize) -> bool {
    let index = ranges.partition_point(|r| r.start <= offset);
    index > 0 && ranges[index - 1].contains(&offset)
}
fn ends_with_blank_line(source: &str, gap: usize) -> bool {
    let text = source[..gap].strip_suffix('\n').unwrap_or(&source[..gap]);
    let line = text.rsplit_once('\n').map_or(text, |(_, line)| line);
    line.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\r'))
}

fn valid_list_prefix(prefix: &str) -> bool {
    let marker = prefix.trim_end_matches(' ');
    if prefix.len() == marker.len() {
        return false;
    }
    if matches!(marker, "-" | "+" | "*") {
        return true;
    }
    let Some(digits) = marker
        .strip_suffix('.')
        .or_else(|| marker.strip_suffix(')'))
    else {
        return false;
    };
    !digits.is_empty() && digits.len() <= 9 && digits.bytes().all(|b| b.is_ascii_digit())
}
fn closed_fence(source: &str, range: &Range<usize>) -> bool {
    let mut lines = source[range.clone()].lines();
    let Some(first) = lines.next() else {
        return false;
    };
    let first = first.trim_start();
    let Some(byte) = first.bytes().next().filter(|b| matches!(b, b'`' | b'~')) else {
        return false;
    };
    let count = first.bytes().take_while(|&b| b == byte).count();
    lines.any(|line| {
        let line = line.trim();
        line.bytes().take_while(|&b| b == byte).count() >= count && line.bytes().all(|b| b == byte)
    })
}
fn open_html(text: &str) -> bool {
    if text.starts_with("<!--") {
        return !text.contains("-->");
    }
    if text.starts_with("<?") {
        return !text.contains("?>");
    }
    if text.starts_with("<![CDATA[") {
        return !text.contains("]]>");
    }
    if text.starts_with("<!") {
        return !text.contains('>');
    }
    for (prefix, closing) in [
        ("<script", "</script>"),
        ("<pre", "</pre>"),
        ("<style", "</style>"),
        ("<textarea", "</textarea>"),
    ] {
        if text
            .get(..prefix.len())
            .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
            && text
                .as_bytes()
                .get(prefix.len())
                .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'>' | b'/'))
        {
            return !text
                .as_bytes()
                .windows(closing.len())
                .any(|value| value.eq_ignore_ascii_case(closing.as_bytes()));
        }
    }
    // Other CommonMark HTML blocks end at the blank line emitted before a new tail.
    false
}
fn pure_headers(source: &str, nodes: &[StepNode], range: Range<usize>) -> bool {
    let mut cursor = range.start;
    for node in nodes {
        if node.header_range.end <= range.start || node.header_range.start >= range.end {
            continue;
        }
        if node.header_range.start < cursor
            || !contains(&range, &node.header_range)
            || !blank(source, cursor..node.header_range.start)
        {
            return false;
        }
        cursor = node.header_range.end;
    }
    blank(source, cursor..range.end)
}

pub(crate) fn protected_ranges(layout: &StepLayout) -> &[Range<usize>] {
    &layout.protected_ranges
}
pub(crate) fn managed_tail(layout: &StepLayout) -> Option<&ManagedStepTail> {
    layout.managed_tail.as_ref()
}

pub(crate) fn project(
    source: &str,
    context: &StepParseContext,
    layout: &StepLayout,
) -> StepProjection {
    let steps = layout
        .nodes
        .iter()
        .map(|node| TaskStep {
            step_id: node.step_id.map(|id| id.to_string()),
            parent_step_id: node
                .parent_index
                .and_then(|index| layout.nodes[index].step_id)
                .map(|id| id.to_string()),
            depth: node.depth,
            title: source[node.title_range.clone()].to_owned(),
            checked: node.checked,
            status: node.status,
            line: node.line,
            source_offset: u32::try_from(node.header_range.start - context.item_range.start)
                .expect("bounded canonical document offset"),
            diagnostic: node.diagnostic.clone(),
        })
        .collect();
    // Numeric growth limits do not make a proved forest's leaf truth unavailable.
    let progress = if layout.identity_invalid || layout.hierarchy_invalid {
        None
    } else {
        let mut done = 0;
        let mut total = 0;
        for (index, node) in layout.nodes.iter().enumerate() {
            if node.step_id.is_some() && node.subtree_end_index == index + 1 {
                total += 1;
                done += u32::from(node.checked);
            }
        }
        Some(TaskStepProgress { done, total })
    };
    StepProjection {
        steps,
        progress,
        diagnostic: (!layout.diagnostics.is_empty()).then(|| layout.diagnostics.join("; ")),
    }
}

struct ModelNode {
    parent: Option<usize>,
    checked: bool,
    active: bool,
    depth: u32,
    tracked: bool,
    title_scalars: usize,
}
fn model(layout: &StepLayout) -> Vec<ModelNode> {
    layout
        .nodes
        .iter()
        .map(|node| ModelNode {
            parent: node.parent_index,
            checked: node.checked,
            active: true,
            depth: node.depth,
            tracked: node.step_id.is_some(),
            title_scalars: node.title_scalars,
        })
        .collect()
}
#[derive(Clone, Copy, Default)]
struct Summary {
    done: usize,
    total: usize,
    children: usize,
}
impl Summary {
    fn status(self) -> TaskStepStatus {
        if self.done == 0 {
            TaskStepStatus::Open
        } else if self.done == self.total {
            TaskStepStatus::Done
        } else {
            TaskStepStatus::Partial
        }
    }
}
/// Queue reduction also works after a move when source indices no longer form
/// preorder; no recursive traversal or repeated descendant scans are needed.
fn summarize(nodes: &[ModelNode]) -> Result<Vec<Summary>, InspectionError> {
    let mut result = vec![Summary::default(); nodes.len()];
    let mut remaining = vec![0usize; nodes.len()];
    for node in nodes.iter().filter(|n| n.active) {
        if let Some(parent) = node.parent {
            if parent >= nodes.len() || !nodes[parent].active {
                return Err(error(
                    "task_steps_invalid",
                    "step parent is not in the active forest",
                ));
            }
            remaining[parent] += 1;
            result[parent].children += 1;
        }
    }
    let mut queue = VecDeque::new();
    for (index, node) in nodes.iter().enumerate() {
        if node.active && remaining[index] == 0 {
            result[index].total = 1;
            result[index].done = usize::from(node.checked);
            queue.push_back(index);
        }
    }
    let mut visited = 0;
    while let Some(index) = queue.pop_front() {
        visited += 1;
        if let Some(parent) = nodes[index].parent {
            result[parent].total += result[index].total;
            result[parent].done += result[index].done;
            remaining[parent] -= 1;
            if remaining[parent] == 0 {
                queue.push_back(parent);
            }
        }
    }
    if visited != nodes.iter().filter(|n| n.active).count() {
        return Err(error(
            "task_step_cycle",
            "step move would introduce a cycle",
        ));
    }
    Ok(result)
}
fn violations(context: &StepParseContext, nodes: &[ModelNode], bytes: usize) -> [usize; 4] {
    let count = nodes
        .iter()
        .filter(|n| n.active && n.tracked)
        .count()
        .saturating_sub(MAX_TRACKED_STEPS);
    let mut depth = 0usize;
    let mut titles = 0usize;
    for node in nodes.iter().filter(|n| n.active) {
        depth += node.depth.saturating_sub(MAX_STEP_DEPTH) as usize;
        titles += node.title_scalars.saturating_sub(MAX_STEP_TITLE_SCALARS);
    }
    [
        count,
        depth,
        titles,
        bytes.saturating_sub(context.continuation_write_limit),
    ]
}

fn target(layout: &StepLayout, id: Uuid) -> Result<usize, InspectionError> {
    layout
        .by_id
        .get(&id)
        .copied()
        .ok_or_else(|| error("task_step_not_found", "step UUID is not in this task"))
}
fn require_structure(layout: &StepLayout) -> Result<(), InspectionError> {
    if layout.identity_invalid || layout.hierarchy_invalid {
        return Err(error(
            "task_steps_invalid",
            "malformed identities or ambiguous ancestry must be corrected in canonical source",
        ));
    }
    Ok(())
}
fn owned_subtree(layout: &StepLayout, index: usize) -> Result<Range<usize>, InspectionError> {
    layout.nodes[index].subtree_range.clone().ok_or_else(|| {
        error(
            "task_step_unsafe",
            "subtree contains unrelated prose or unsafe physical boundaries",
        )
    })
}
fn title_valid(title: &str) -> Result<usize, InspectionError> {
    if title.trim().is_empty()
        || title != title.trim()
        || title
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
        || [
            "<!--",
            "-->",
            "cockpit-step:",
            "cockpit-task:",
            "cockpit-checklist:",
            "cockpit-relations:",
        ]
        .iter()
        .any(|token| title.contains(token))
    {
        return Err(error(
            "task_step_title_invalid",
            "step title must be nonempty single-line plain text without reserved Markdown or metadata syntax",
        ));
    }
    Ok(title.chars().count())
}

struct Destination {
    parent: Option<usize>,
    before: Option<usize>,
    gap: usize,
    indent: usize,
    create_tail: bool,
}
fn destination(
    layout: &StepLayout,
    parent_id: Option<Uuid>,
    before_id: Option<Uuid>,
) -> Result<Destination, InspectionError> {
    let parent = parent_id.map(|id| target(layout, id)).transpose()?;
    let before = before_id.map(|id| target(layout, id)).transpose()?;
    if before.is_some_and(|index| layout.nodes[index].parent_index != parent) {
        return Err(error(
            "task_step_destination_invalid",
            "before-step must be a direct child of the requested parent",
        ));
    }
    if let Some(parent) = parent {
        owned_subtree(layout, parent)?;
    }
    if let Some(before) = before {
        owned_subtree(layout, before)?;
    }
    let content_column = match parent {
        Some(parent) => layout.nodes[parent].child_indent.ok_or_else(|| {
            error(
                "task_step_unsafe",
                "tabbed or nonstandard destination has no proved content column",
            )
        })?,
        None => 2,
    };
    let indent = match before {
        Some(before) => layout.nodes[before].indent.ok_or_else(|| {
            error(
                "task_step_unsafe",
                "destination sibling has tabs or an unproved physical column",
            )
        })?,
        None => content_column,
    };
    let (gap, create_tail) = if let Some(before) = before {
        (layout.nodes[before].header_range.start, false)
    } else if let Some(parent) = parent {
        (owned_subtree(layout, parent)?.end, false)
    } else if let Some(tail) = &layout.managed_tail {
        (tail.root_append_offset, false)
    } else {
        (
            layout.tail_creation_gap.ok_or_else(|| {
                error(
                    "task_step_unsafe",
                    "no safe terminal boundary for a managed checklist tail",
                )
            })?,
            true,
        )
    };
    Ok(Destination {
        parent,
        before,
        gap,
        indent,
        create_tail,
    })
}
fn next_sibling(layout: &StepLayout, index: usize) -> Option<usize> {
    let next = layout.nodes[index].subtree_end_index;
    (next < layout.nodes.len()
        && layout.nodes[next].parent_index == layout.nodes[index].parent_index)
        .then_some(next)
}
fn mark_ancestors(nodes: &[ModelNode], mut index: Option<usize>, affected: &mut [bool]) {
    while let Some(current) = index {
        if affected[current] {
            break;
        }
        affected[current] = true;
        index = nodes[current].parent;
    }
}
fn leaf_conversions(
    nodes: &mut [ModelNode],
    before: &[Summary],
) -> Result<Vec<Summary>, InspectionError> {
    let mut after = summarize(nodes)?;
    let mut converted = false;
    for (index, node) in nodes.iter_mut().enumerate() {
        if node.active
            && index < before.len()
            && before[index].children > 0
            && after[index].children == 0
        {
            node.checked = before[index].status() == TaskStepStatus::Done;
            converted = true;
        }
    }
    if converted {
        after = summarize(nodes)?;
    }
    Ok(after)
}
fn normalize<'a>(
    layout: &StepLayout,
    nodes: &[ModelNode],
    summaries: &[Summary],
    affected: &[bool],
    patches: &mut Vec<BytePatch<'a>>,
) {
    for (index, node) in layout.nodes.iter().enumerate() {
        if !nodes[index].active || !affected[index] {
            continue;
        }
        let checked = if summaries[index].children == 0 {
            nodes[index].checked
        } else {
            summaries[index].status() == TaskStepStatus::Done
        };
        if checked != node.checked {
            patches.push(BytePatch {
                range: node.checkbox_offset..node.checkbox_offset + 1,
                replacement: Cow::Borrowed(if checked { b"x" } else { b" " }),
            });
        }
    }
}
fn emitted_step(id: Uuid, title: &str, indent: usize, eol: DocumentEol) -> Vec<u8> {
    format!(
        "{:indent$}- [ ] {title} <!-- cockpit-step: {id} -->{}",
        "",
        eol.text()
    )
    .into_bytes()
}
fn insertion<'a>(
    source: &str,
    context: &StepParseContext,
    dest: &Destination,
    bytes: Cow<'a, [u8]>,
) -> Cow<'a, [u8]> {
    let missing_eol = !bytes.ends_with(b"\n");
    let prefix_eol = dest.gap > 0 && source.as_bytes()[dest.gap - 1] != b'\n';
    if !dest.create_tail && !prefix_eol && !missing_eol {
        return bytes;
    }
    let mut result = Vec::with_capacity(
        bytes.len()
            + if dest.create_tail {
                BEGIN.len() + END.len() + 24
            } else {
                4
            },
    );
    if prefix_eol {
        result.extend_from_slice(context.eol.text().as_bytes());
    }
    if dest.create_tail {
        // Exit lazy paragraphs and terminating HTML block types.
        if dest.gap > context.continuation_range.start && !ends_with_blank_line(source, dest.gap) {
            result.extend_from_slice(context.eol.text().as_bytes());
        }
        result.extend_from_slice(b"  ");
        result.extend_from_slice(BEGIN.as_bytes());
        result.extend_from_slice(context.eol.text().as_bytes());
    }
    result.extend_from_slice(&bytes);
    if missing_eol {
        result.extend_from_slice(context.eol.text().as_bytes());
    }
    if dest.create_tail {
        result.extend_from_slice(b"  ");
        result.extend_from_slice(END.as_bytes());
        result.extend_from_slice(context.eol.text().as_bytes());
    }
    Cow::Owned(result)
}
fn reindent<'a>(
    source: &'a str,
    context: &StepParseContext,
    layout: &StepLayout,
    index: usize,
    range: Range<usize>,
    indent: usize,
) -> Result<Cow<'a, [u8]>, InspectionError> {
    let old = layout.nodes[index].indent.ok_or_else(|| {
        error(
            "task_step_unsafe",
            "tabbed subtree cannot be safely reindented",
        )
    })?;
    for node in &layout.nodes[index..layout.nodes[index].subtree_end_index] {
        if node.indent.is_none_or(|value| value < old) || node.child_indent.is_none() {
            return Err(error(
                "task_step_unsafe",
                "subtree has tabs or an unproved indentation column",
            ));
        }
    }
    if old == indent {
        return Ok(Cow::Borrowed(&source.as_bytes()[range]));
    }
    let count = layout.nodes[index].subtree_end_index - index;
    let delta = indent
        .abs_diff(old)
        .checked_mul(count)
        .ok_or_else(invalid_patch)?;
    let capacity = if indent > old {
        let next_bytes = context
            .continuation_range
            .len()
            .checked_add(delta)
            .ok_or_else(invalid_patch)?;
        if next_bytes
            > context
                .continuation_range
                .len()
                .max(context.continuation_write_limit)
        {
            return Err(error(
                "task_step_limit",
                "reindent increases the complete continuation byte-limit violation",
            ));
        }
        range.len().checked_add(delta).ok_or_else(invalid_patch)?
    } else {
        range.len().checked_sub(delta).ok_or_else(invalid_patch)?
    };
    let mut result = Vec::with_capacity(capacity);
    for line in source[range].split_inclusive('\n') {
        if line.trim().is_empty() {
            result.extend_from_slice(line.as_bytes());
            continue;
        }
        let spaces = line.bytes().take_while(|&byte| byte == b' ').count();
        if spaces < old {
            return Err(error(
                "task_step_unsafe",
                "lazy continuation cannot be reindented",
            ));
        }
        result.resize(result.len() + indent + spaces - old, b' ');
        result.extend_from_slice(&line.as_bytes()[spaces..]);
    }
    Ok(Cow::Owned(result))
}

/// Pure plans borrow untouched move bytes and rename text. Publication, CAS,
/// authority, final-document size checks and resulting Task assembly are external.
pub(crate) fn plan<'a>(
    source: &'a str,
    context: &StepParseContext,
    layout: &StepLayout,
    intent: StepIntent<'a>,
) -> Result<Vec<BytePatch<'a>>, InspectionError> {
    if !context_valid(source, context) {
        return Err(invalid_patch());
    }
    require_structure(layout)?;
    let mut nodes = model(layout);
    let before = summarize(&nodes)?;
    let previous_limits = violations(context, &nodes, context.continuation_range.len());
    let mut affected = vec![false; nodes.len()];
    let mut patches = Vec::new();
    match intent {
        StepIntent::Add {
            step_id,
            parent_step_id,
            before_step_id,
            title,
        } => {
            let scalars = title_valid(title)?;
            let dest = destination(layout, parent_step_id, before_step_id)?;
            if dest.parent.is_none()
                && dest.before.is_some_and(|index| {
                    dest.indent != 2
                        || layout.managed_tail.as_ref().is_none_or(|tail| {
                            !tail.range.contains(&layout.nodes[index].header_range.start)
                        })
                })
            {
                return Err(error(
                    "task_step_destination_invalid",
                    "new root steps belong in the managed tail; cannot add before a tracked root outside it",
                ));
            }
            if let Some(&index) = layout.by_id.get(&step_id) {
                let node = &layout.nodes[index];
                // An uncertain add can be observed as no-effect only if every
                // original property, including initial leaf state/order, matches.
                if node.parent_index != dest.parent
                    || next_sibling(layout, index) != dest.before
                    || node.subtree_end_index != index + 1
                    || node.checked
                    || &source[node.title_range.clone()] != title
                {
                    return Err(error(
                        "task_step_id_conflict",
                        "step UUID already exists with different title, parent, order, or initial state",
                    ));
                }
                owned_subtree(layout, index)?;
            } else {
                let depth = dest.parent.map_or(0, |index| nodes[index].depth + 1);
                if scalars > MAX_STEP_TITLE_SCALARS
                    || depth > MAX_STEP_DEPTH
                    || layout.by_id.len() >= MAX_TRACKED_STEPS
                {
                    return Err(error(
                        "task_step_limit",
                        "new step would increase the count, depth, or title-scalar limit violation",
                    ));
                }
                mark_ancestors(&nodes, dest.parent, &mut affected);
                nodes.push(ModelNode {
                    parent: dest.parent,
                    checked: false,
                    active: true,
                    depth,
                    tracked: true,
                    title_scalars: scalars,
                });
                patches.push(BytePatch {
                    range: dest.gap..dest.gap,
                    replacement: insertion(
                        source,
                        context,
                        &dest,
                        Cow::Owned(emitted_step(step_id, title, dest.indent, context.eol)),
                    ),
                });
                let after = leaf_conversions(&mut nodes, &before)?;
                normalize(layout, &nodes, &after, &affected, &mut patches);
            }
        }
        StepIntent::Rename { step_id, title } => {
            let index = target(layout, step_id)?;
            nodes[index].title_scalars = title_valid(title)?;
            if &source[layout.nodes[index].title_range.clone()] != title {
                patches.push(BytePatch {
                    range: layout.nodes[index].title_range.clone(),
                    replacement: Cow::Borrowed(title.as_bytes()),
                });
            }
        }
        StepIntent::SetChecked {
            step_id,
            checked,
            scope,
        } => {
            let index = target(layout, step_id)?;
            if scope == TaskStepScope::Leaf && layout.nodes[index].subtree_end_index != index + 1 {
                return Err(error(
                    "task_step_scope_invalid",
                    "a branch requires explicit subtree scope",
                ));
            }
            let end = layout.nodes[index].subtree_end_index;
            for current in index..end {
                affected[current] = true;
                if before[current].children == 0 {
                    nodes[current].checked = checked;
                }
            }
            mark_ancestors(&nodes, nodes[index].parent, &mut affected);
            let after = summarize(&nodes)?;
            normalize(layout, &nodes, &after, &affected, &mut patches);
        }
        StepIntent::Move {
            step_id,
            parent_step_id,
            before_step_id,
        } => {
            let index = target(layout, step_id)?;
            let range = owned_subtree(layout, index)?;
            let end = layout.nodes[index].subtree_end_index;
            let dest = destination(layout, parent_step_id, before_step_id)?;
            if dest.parent.is_some_and(|p| (index..end).contains(&p))
                || dest.before.is_some_and(|b| (index..end).contains(&b))
            {
                return Err(error(
                    "task_step_cycle",
                    "a subtree cannot move beneath or before itself or its descendants",
                ));
            }
            if dest.gap > range.start && dest.gap < range.end {
                return Err(error(
                    "task_step_unsafe",
                    "move destination lies inside source bytes",
                ));
            }
            let new_depth = dest.parent.map_or(0, |parent| nodes[parent].depth + 1);
            let old_depth = nodes[index].depth;
            for node in &mut nodes[index..end] {
                node.depth = new_depth + node.depth - old_depth;
            }
            if violations(context, &nodes, context.continuation_range.len())
                .iter()
                .zip(previous_limits)
                .any(|(&after, before)| after > before)
            {
                return Err(error(
                    "task_step_limit",
                    "move increases a saved depth-limit violation",
                ));
            }
            mark_ancestors(&nodes, nodes[index].parent, &mut affected);
            mark_ancestors(&nodes, dest.parent, &mut affected);
            nodes[index].parent = dest.parent;
            if layout.nodes[index].parent_index != dest.parent
                || next_sibling(layout, index) != dest.before
            {
                let moved = reindent(source, context, layout, index, range.clone(), dest.indent)?;
                if dest.gap == range.start || dest.gap == range.end {
                    // Boundary moves are one owned replacement, never a deletion
                    // and a second patch competing for the same insertion gap.
                    if dest.create_tail {
                        patches.push(BytePatch {
                            range,
                            replacement: insertion(source, context, &dest, moved),
                        });
                    } else {
                        patches.push(BytePatch {
                            range,
                            replacement: moved,
                        });
                    }
                } else {
                    patches.push(BytePatch {
                        range,
                        replacement: Cow::Borrowed(b""),
                    });
                    patches.push(BytePatch {
                        range: dest.gap..dest.gap,
                        replacement: insertion(source, context, &dest, moved),
                    });
                }
            }
            let after = leaf_conversions(&mut nodes, &before)?;
            normalize(layout, &nodes, &after, &affected, &mut patches);
        }
        StepIntent::Remove { step_id } => {
            let index = target(layout, step_id)?;
            let range = owned_subtree(layout, index)?;
            mark_ancestors(&nodes, nodes[index].parent, &mut affected);
            for node in &mut nodes[index..layout.nodes[index].subtree_end_index] {
                node.active = false;
            }
            patches.push(BytePatch {
                range,
                replacement: Cow::Borrowed(b""),
            });
            let after = leaf_conversions(&mut nodes, &before)?;
            normalize(layout, &nodes, &after, &affected, &mut patches);
        }
    }
    patches.sort_unstable_by_key(|patch| (patch.range.start, patch.range.end));
    validate_patches(source, context, layout, &patches)?;
    let mut bytes = context.continuation_range.len();
    for patch in &patches {
        bytes = bytes
            .checked_sub(patch.range.len())
            .and_then(|n| n.checked_add(patch.replacement.len()))
            .ok_or_else(invalid_patch)?;
    }
    let next_limits = violations(context, &nodes, bytes);
    if next_limits
        .iter()
        .zip(previous_limits)
        .any(|(&after, before)| after > before)
    {
        return Err(error(
            "task_step_limit",
            "edit increases a count, depth, title-scalar, or complete continuation byte-limit violation",
        ));
    }
    Ok(patches)
}

/// Check only existing layout ownership. Protected spans alone deliberately do
/// not confer insertion permissions or allow edits to control/identity records.
pub(crate) fn validate_patches(
    source: &str,
    context: &StepParseContext,
    layout: &StepLayout,
    patches: &[BytePatch<'_>],
) -> Result<(), InspectionError> {
    if !context_valid(source, context) {
        return Err(invalid_patch());
    }
    let mut previous: Option<&Range<usize>> = None;
    let mut node_cursor = 0usize;
    for patch in patches {
        let range = &patch.range;
        if !contains(&context.continuation_range, range)
            || !source.is_char_boundary(range.start)
            || !source.is_char_boundary(range.end)
            || std::str::from_utf8(&patch.replacement).is_err()
            || context
                .relation_record_range
                .as_ref()
                .is_some_and(|record| {
                    range.start < record.end && record.start < range.end
                        || range.is_empty()
                            && record.start < range.start
                            && range.start < record.end
                })
        {
            return Err(invalid_patch());
        }
        if previous.is_some_and(|prior| {
            prior.end > range.start
                || prior.start > range.start
                || prior.is_empty() && range.is_empty() && prior.start == range.start
                || prior.is_empty() && prior.start == range.start && !range.is_empty()
        }) {
            return Err(invalid_patch());
        }
        previous = Some(range);
        while node_cursor < layout.nodes.len()
            && layout.nodes[node_cursor].header_range.start <= range.start
        {
            node_cursor += 1;
        }
        let owner = node_cursor.checked_sub(1).map(|index| &layout.nodes[index]);
        let prior = node_cursor.checked_sub(2).map(|index| &layout.nodes[index]);
        let safe_gap = |node: &StepNode, gap: usize| {
            node.subtree_range
                .as_ref()
                .is_some_and(|subtree| subtree.start == gap || subtree.end == gap)
        };
        let owned = !layout.identity_invalid
            && !layout.hierarchy_invalid
            && if range.is_empty() {
                let gap = range.start;
                layout.tail_creation_gap == Some(gap)
                    || layout
                        .managed_tail
                        .as_ref()
                        .is_some_and(|tail| tail.root_append_offset == gap)
                    || owner.is_some_and(|node| safe_gap(node, gap))
                    || prior.is_some_and(|node| safe_gap(node, gap))
            } else {
                owner.is_some_and(|node| {
                    *range == (node.checkbox_offset..node.checkbox_offset + 1)
                        || *range == node.title_range
                        || node
                            .subtree_range
                            .as_ref()
                            .is_some_and(|subtree| subtree == range)
                        || node.subtree_range.is_some() && *range == node.header_range
                })
            };
        if !owned {
            return Err(invalid_patch());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }
    fn row(depth: usize, value: u128, checked: bool, title: &str) -> String {
        format!(
            "{}- [{}] {title} <!-- cockpit-step: {} -->\n",
            " ".repeat(2 + depth * 2),
            if checked { 'x' } else { ' ' },
            id(value)
        )
    }
    fn managed(rows: &str) -> String {
        format!(
            "# Tasks\n\n- [ ] Task <!-- cockpit-task: {} -->\n  <!-- cockpit-relations: depends_on={} -->\n  Prose stays byte-exact.\n\n  {BEGIN}\n{rows}  {END}\n- [ ] Other <!-- cockpit-task: {} -->\n  - [x] Not in this task\n",
            id(1000),
            id(1001),
            id(1002)
        )
    }
    fn external(rows: &str) -> String {
        format!(
            "# Tasks\n\n- [ ] Task <!-- cockpit-task: {} -->\n  Before prose.\n\n{rows}\n  After prose.\n- [ ] Other <!-- cockpit-task: {} -->\n",
            id(1000),
            id(1002)
        )
    }
    fn context(source: &str) -> StepParseContext {
        let start = source.find("- [ ] Task").unwrap();
        let end = source[start..]
            .find("\n- [ ] Other")
            .map_or(source.len(), |offset| start + offset + 1);
        let continuation_start = source[start..end]
            .find('\n')
            .map_or(end, |offset| start + offset + 1);
        let relation_record_range = source[continuation_start..end]
            .starts_with("  <!-- cockpit-relations:")
            .then(|| {
                continuation_start
                    ..continuation_start + source[continuation_start..end].find('\n').unwrap() + 1
            });
        StepParseContext {
            item_range: start..end,
            continuation_range: continuation_start..end,
            item_line: source.as_bytes()[..start]
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count()
                + 1,
            top_checkbox_offset: start + 3,
            relation_record_range,
            eol: if source.contains("\r\n") {
                DocumentEol::CrLf
            } else {
                DocumentEol::Lf
            },
            continuation_write_limit: 16 * 1024,
        }
    }
    fn apply(source: &str, patches: &[BytePatch<'_>]) -> String {
        let mut output = Vec::new();
        let mut position = 0;
        for patch in patches {
            output.extend_from_slice(&source.as_bytes()[position..patch.range.start]);
            output.extend_from_slice(&patch.replacement);
            position = patch.range.end;
        }
        output.extend_from_slice(&source.as_bytes()[position..]);
        String::from_utf8(output).unwrap()
    }
    fn edit<'a>(source: &'a str, intent: StepIntent<'a>) -> String {
        let context = context(source);
        let layout = parse(source, &context).unwrap();
        let patches = plan(source, &context, &layout, intent).unwrap();
        validate_patches(source, &context, &layout, &patches).unwrap();
        apply(source, &patches)
    }
    fn projection(source: &str) -> StepProjection {
        let context = context(source);
        let layout = parse(source, &context).unwrap();
        project(source, &context, &layout)
    }
    fn refusal<'a>(source: &'a str, intent: StepIntent<'a>) {
        let context = context(source);
        let layout = parse(source, &context).unwrap();
        assert!(plan(source, &context, &layout, intent).is_err());
    }

    #[test]
    fn original_offsets_leaf_progress_and_raw_branch_mismatch_are_read_only() {
        let source = managed(
            &(row(0, 1, true, "Parent") + &row(1, 2, true, "Done") + &row(1, 3, false, "Open")),
        );
        let context = context(&source);
        let layout = parse(&source, &context).unwrap();
        let result = project(&source, &context, &layout);
        assert_eq!(result.steps.len(), 3);
        assert!(result.steps[0].checked);
        assert_eq!(result.steps[0].status, TaskStepStatus::Partial);
        assert_eq!(result.steps[1].parent_step_id, Some(id(1).to_string()));
        assert_eq!(result.steps[1].depth, 1);
        assert_eq!(
            (
                result.progress.as_ref().unwrap().done,
                result.progress.as_ref().unwrap().total
            ),
            (1, 2)
        );
        let offset = source.find("    - [x] Done").unwrap();
        assert_eq!(
            result.steps[1].source_offset as usize,
            offset - context.item_range.start
        );
        assert_eq!(
            result.steps[1].line as usize,
            source[..offset].bytes().filter(|&b| b == b'\n').count() + 1
        );
        assert_eq!(source.as_bytes()[layout.nodes[0].checkbox_offset], b'x');
        assert!(managed_tail(&layout).is_some());
        assert!(
            protected_ranges(&layout)
                .iter()
                .all(|r| contains(&context.continuation_range, r))
        );
    }

    #[test]
    fn subtree_check_is_absolute_and_leaves_top_relations_prose_and_other_task_unchanged() {
        let source = managed(
            &(row(0, 1, false, "Parent")
                + &row(1, 2, true, "Done")
                + &row(1, 3, false, "Open")
                + &row(0, 4, false, "Unrelated")),
        );
        let changed = edit(
            &source,
            StepIntent::SetChecked {
                step_id: id(1),
                checked: true,
                scope: TaskStepScope::Subtree,
            },
        );
        let expected = source
            .replace("  - [ ] Parent", "  - [x] Parent")
            .replace("    - [ ] Open", "    - [x] Open");
        assert_eq!(changed, expected);
        assert_eq!(
            edit(
                &changed,
                StepIntent::SetChecked {
                    step_id: id(1),
                    checked: true,
                    scope: TaskStepScope::Subtree
                }
            ),
            changed
        );
        let result = projection(&changed);
        assert_eq!(
            (
                result.progress.as_ref().unwrap().done,
                result.progress.as_ref().unwrap().total
            ),
            (2, 3)
        );
        assert_eq!(
            changed.as_bytes()[context(&changed).top_checkbox_offset],
            b' '
        );
        refusal(
            &source,
            StepIntent::SetChecked {
                step_id: id(1),
                checked: true,
                scope: TaskStepScope::Leaf,
            },
        );
    }

    #[test]
    fn leaf_check_normalizes_only_its_ancestors_not_unrelated_branch_bytes() {
        let source = managed(
            &(row(0, 1, false, "Affected")
                + &row(1, 2, false, "Leaf")
                + &row(0, 3, false, "External mismatch")
                + &row(1, 4, true, "Already done")),
        );
        let changed = edit(
            &source,
            StepIntent::SetChecked {
                step_id: id(2),
                checked: true,
                scope: TaskStepScope::Leaf,
            },
        );
        assert_eq!(
            changed,
            source
                .replace("  - [ ] Affected", "  - [x] Affected")
                .replace("    - [ ] Leaf", "    - [x] Leaf")
        );
        assert_eq!(projection(&changed).steps[2].status, TaskStepStatus::Done);
        assert!(!projection(&changed).steps[2].checked);
    }

    #[test]
    fn adding_under_checked_leaf_makes_an_open_branch_and_keeps_ids() {
        let source = managed(&row(0, 1, true, "Was a leaf"));
        let changed = edit(
            &source,
            StepIntent::Add {
                step_id: id(2),
                parent_step_id: Some(id(1)),
                before_step_id: None,
                title: "New child",
            },
        );
        let expected = source.replace(
            &row(0, 1, true, "Was a leaf"),
            &(row(0, 1, false, "Was a leaf") + &row(1, 2, false, "New child")),
        );
        assert_eq!(changed, expected);
        let result = projection(&changed);
        assert_eq!(result.steps[0].status, TaskStepStatus::Open);
        assert_eq!(result.steps[1].parent_step_id, Some(id(1).to_string()));
        assert_eq!(result.progress.unwrap().total, 1);
    }

    #[test]
    fn last_child_removal_uses_prior_derived_done_not_raw_parent_checkbox() {
        for (raw_parent, child_done, expected_parent) in [(false, true, true), (true, false, false)]
        {
            let source =
                managed(&(row(0, 1, raw_parent, "Parent") + &row(1, 2, child_done, "Child")));
            let changed = edit(&source, StepIntent::Remove { step_id: id(2) });
            assert_eq!(changed, managed(&row(0, 1, expected_parent, "Parent")));
            let result = projection(&changed);
            assert_eq!(result.progress.unwrap().total, 1);
            assert_eq!(result.steps[0].checked, expected_parent);
        }
    }

    #[test]
    fn removing_last_partial_child_maps_parent_to_open_leaf() {
        let source = managed(
            &(row(0, 1, true, "Parent")
                + &row(1, 2, true, "Branch")
                + &row(2, 3, true, "Done")
                + &row(2, 4, false, "Open")),
        );
        let changed = edit(&source, StepIntent::Remove { step_id: id(2) });
        assert_eq!(changed, managed(&row(0, 1, false, "Parent")));
    }

    #[test]
    fn move_reparents_whole_subtree_and_normalizes_old_and_new_ancestors() {
        let source = managed(
            &(row(0, 1, true, "Old parent")
                + &row(1, 2, true, "Moving")
                + &row(2, 3, false, "Child")
                + &row(0, 4, true, "New parent")),
        );
        let changed = edit(
            &source,
            StepIntent::Move {
                step_id: id(2),
                parent_step_id: Some(id(4)),
                before_step_id: None,
            },
        );
        let expected = managed(
            &(row(0, 1, false, "Old parent")
                + &row(0, 4, false, "New parent")
                + &row(1, 2, true, "Moving")
                + &row(2, 3, false, "Child")),
        );
        assert_eq!(changed, expected);
        let result = projection(&changed);
        assert_eq!(result.steps[2].step_id, Some(id(2).to_string()));
        assert_eq!(result.steps[2].parent_step_id, Some(id(4).to_string()));
        assert_eq!(result.steps[3].parent_step_id, Some(id(2).to_string()));
    }

    #[test]
    fn reorder_borrows_unchanged_subtree_bytes_and_preserves_crlf() {
        let source =
            managed(&(row(0, 1, false, "A") + &row(1, 2, true, "A child") + &row(0, 3, true, "B")))
                .replace('\n', "\r\n");
        let context = context(&source);
        let layout = parse(&source, &context).unwrap();
        let patches = plan(
            &source,
            &context,
            &layout,
            StepIntent::Move {
                step_id: id(1),
                parent_step_id: None,
                before_step_id: None,
            },
        )
        .unwrap();
        assert!(
            patches
                .iter()
                .any(|p| p.range.is_empty() && matches!(&p.replacement, Cow::Borrowed(_)))
        );
        let changed = apply(&source, &patches);
        assert_eq!(
            changed,
            managed(&(row(0, 3, true, "B") + &row(0, 1, false, "A") + &row(1, 2, true, "A child")))
                .replace('\n', "\r\n")
        );
    }

    #[test]
    fn cycles_missing_ids_and_foreign_before_parent_are_refused() {
        let source =
            managed(&(row(0, 1, false, "A") + &row(1, 2, false, "Child") + &row(0, 3, false, "B")));
        refusal(
            &source,
            StepIntent::Move {
                step_id: id(1),
                parent_step_id: Some(id(2)),
                before_step_id: None,
            },
        );
        refusal(
            &source,
            StepIntent::Move {
                step_id: id(1),
                parent_step_id: None,
                before_step_id: Some(id(1)),
            },
        );
        refusal(
            &source,
            StepIntent::Move {
                step_id: id(3),
                parent_step_id: Some(id(1)),
                before_step_id: Some(id(3)),
            },
        );
        refusal(&source, StepIntent::Remove { step_id: id(999) });
        refusal(
            &source,
            StepIntent::Add {
                step_id: id(10),
                parent_step_id: Some(id(999)),
                before_step_id: None,
                title: "No parent",
            },
        );
    }

    #[test]
    fn same_uuid_add_only_acknowledges_exact_valid_initial_properties() {
        let source = managed(&(row(0, 1, false, "A") + &row(0, 2, false, "B")));
        let context = context(&source);
        let layout = parse(&source, &context).unwrap();
        assert!(
            plan(
                &source,
                &context,
                &layout,
                StepIntent::Add {
                    step_id: id(1),
                    parent_step_id: None,
                    before_step_id: Some(id(2)),
                    title: "A"
                }
            )
            .unwrap()
            .is_empty()
        );
        refusal(
            &source,
            StepIntent::Add {
                step_id: id(1),
                parent_step_id: None,
                before_step_id: None,
                title: "A",
            },
        );
        refusal(
            &source,
            StepIntent::Add {
                step_id: id(1),
                parent_step_id: None,
                before_step_id: Some(id(2)),
                title: "Different",
            },
        );
        let checked = source.replace("  - [ ] A", "  - [x] A");
        refusal(
            &checked,
            StepIntent::Add {
                step_id: id(1),
                parent_step_id: None,
                before_step_id: Some(id(2)),
                title: "A",
            },
        );
    }

    #[test]
    fn untracked_mixed_bullets_case_spaces_and_crlf_remain_readable() {
        let source =
            external("  * [X] Parent  \n    + [ ] Child\n  * [ ] Sibling\n").replace('\n', "\r\n");
        let result = projection(&source);
        assert_eq!(result.steps.len(), 3);
        assert_eq!(result.steps[1].depth, 1);
        assert!(result.steps[0].checked);
        assert!(result.steps.iter().all(|step| step.step_id.is_none()));
    }

    #[test]
    fn tracked_child_insertion_and_cross_region_move_preserve_prose_in_place() {
        let source =
            external(&(row(0, 1, true, "Tracked parent") + &row(1, 2, true, "Tracked child")));
        let with_child = edit(
            &source,
            StepIntent::Add {
                step_id: id(3),
                parent_step_id: Some(id(1)),
                before_step_id: None,
                title: "Added in place",
            },
        );
        assert_eq!(
            with_child,
            source
                .replace(
                    &row(0, 1, true, "Tracked parent"),
                    &row(0, 1, false, "Tracked parent")
                )
                .replace(
                    &row(1, 2, true, "Tracked child"),
                    &(row(1, 2, true, "Tracked child") + &row(1, 3, false, "Added in place"))
                )
        );
        let with_tail = edit(
            &with_child,
            StepIntent::Add {
                step_id: id(4),
                parent_step_id: None,
                before_step_id: None,
                title: "Tail parent",
            },
        );
        assert!(with_tail.find("  After prose.").unwrap() < with_tail.find(BEGIN).unwrap());
        let changed = edit(
            &with_tail,
            StepIntent::Move {
                step_id: id(2),
                parent_step_id: Some(id(4)),
                before_step_id: None,
            },
        );
        assert!(changed.contains("  Before prose.\n\n"));
        assert!(changed.contains("\n  After prose.\n"));
        let saved = projection(&changed);
        let moved = saved
            .steps
            .iter()
            .find(|step| step.step_id == Some(id(2).to_string()))
            .unwrap();
        assert_eq!(moved.parent_step_id, Some(id(4).to_string()));
        refusal(
            &source,
            StepIntent::Add {
                step_id: id(7),
                parent_step_id: None,
                before_step_id: Some(id(1)),
                title: "Not an out-of-tail root insertion",
            },
        );
    }

    #[test]
    fn examples_are_prose_not_candidates_or_managed_boundaries() {
        let body = format!(
            "  ```md\n  {BEGIN}\n  - [x] Fenced <!-- cockpit-step: {} -->\n  {END}\n  ```\n\n  > - [ ] Quoted\n\n  <div>\n  {BEGIN}\n  - [ ] HTML example\n  {END}\n  </div>\n\n  - [ ] Actual `<!-- cockpit-step: not-a-uuid -->`\n",
            id(9)
        );
        let source = external(&body);
        let result = projection(&source);
        assert_eq!(result.steps.len(), 1);
        assert!(result.steps[0].step_id.is_none());
        assert!(result.steps[0].title.contains("not-a-uuid"));
        assert!(result.diagnostic.is_none());
        let context = context(&source);
        let layout = parse(&source, &context).unwrap();
        assert!(managed_tail(&layout).is_none());
        assert!(
            protected_ranges(&layout)
                .iter()
                .all(|range| !source[range.clone()].contains("Fenced"))
        );
    }

    #[test]
    fn ordinary_list_ancestor_or_interleaved_continuation_is_diagnosed() {
        let source = external("  - Ordinary parent\n    - [ ] Child\n");
        let result = projection(&source);
        assert_eq!(result.steps.len(), 1);
        assert!(result.progress.is_none());
        assert!(result.steps[0].diagnostic.is_some());
        let source =
            external("  - [ ] Parent\n    Non-checkbox continuation prose\n    - [ ] Child\n");
        let result = projection(&source);
        assert!(result.steps[0].diagnostic.is_some());
    }

    #[test]
    fn duplicate_malformed_and_stray_ids_are_diagnostics_not_task_identity_errors() {
        for rows in [
            row(0, 1, false, "A") + &row(0, 1, false, "B"),
            "  - [ ] Bad <!-- cockpit-step: nope -->\n".to_owned(),
            "  <!-- cockpit-step: nope -->\n".to_owned(),
        ] {
            let source = managed(&rows);
            let result = projection(&source);
            assert!(result.diagnostic.is_some());
            assert!(result.progress.is_none());
            assert!(parse(&source, &context(&source)).is_ok());
            refusal(
                &source,
                StepIntent::Add {
                    step_id: id(3),
                    parent_step_id: None,
                    before_step_id: None,
                    title: "Cannot bypass diagnostic",
                },
            );
        }
    }

    #[test]
    fn malformed_or_nonterminal_tail_and_unclosed_examples_never_grant_creation() {
        for body in [
            format!("  {BEGIN}\n  - [ ] Unclosed\n"),
            format!("  {BEGIN}\n  {BEGIN}\n  {END}\n"),
            format!("  {BEGIN}\n  - [ ] Row\n  {END}\n  Trailing prose\n"),
            "  ```md\n  - [ ] Example\n".to_owned(),
            "  <script>\n  anything\n".to_owned(),
            "  <!-- unclosed HTML comment\n".to_owned(),
        ] {
            let source = format!("- [ ] Task <!-- cockpit-task: {} -->\n{body}", id(1000));
            refusal(
                &source,
                StepIntent::Add {
                    step_id: id(1),
                    parent_step_id: None,
                    before_step_id: None,
                    title: "Unsafe",
                },
            );
        }
    }

    #[test]
    fn malformed_zero_row_managed_tail_has_no_trustworthy_empty_progress() {
        for body in [
            format!("  {BEGIN}\n"),
            format!("  {BEGIN}\n  {BEGIN}\n  {END}\n"),
            format!("  {END}\n"),
        ] {
            let source = format!("- [ ] Task <!-- cockpit-task: {} -->\n{body}", id(1000));
            let saved = projection(&source);
            assert!(saved.steps.is_empty());
            assert!(saved.progress.is_none());
            assert!(saved.diagnostic.is_some());
            refusal(
                &source,
                StepIntent::Add {
                    step_id: id(1),
                    parent_step_id: None,
                    before_step_id: None,
                    title: "Cannot replace malformed tail",
                },
            );
        }
    }

    #[test]
    fn root_creation_and_empty_tail_reuse_handle_no_final_newline_and_crlf() {
        for eol in ["\n", "\r\n"] {
            let source = format!("- [ ] Task <!-- cockpit-task: {} -->", id(1000));
            let mut context = context(&source);
            context.eol = if eol == "\r\n" {
                DocumentEol::CrLf
            } else {
                DocumentEol::Lf
            };
            let layout = parse(&source, &context).unwrap();
            let patches = plan(
                &source,
                &context,
                &layout,
                StepIntent::Add {
                    step_id: id(1),
                    parent_step_id: None,
                    before_step_id: None,
                    title: "Root",
                },
            )
            .unwrap();
            let changed = apply(&source, &patches);
            assert!(changed.starts_with(&format!("{source}{eol}  {BEGIN}{eol}")));
            assert_eq!(
                projection(&changed).steps[0].step_id,
                Some(id(1).to_string())
            );
            let empty = edit(&changed, StepIntent::Remove { step_id: id(1) });
            let restored = edit(
                &empty,
                StepIntent::Add {
                    step_id: id(2),
                    parent_step_id: None,
                    before_step_id: None,
                    title: "Root",
                },
            );
            assert_eq!(restored.matches(BEGIN).count(), 1);
            assert_eq!(projection(&restored).steps.len(), 1);
            assert_eq!(projection(&empty).progress.unwrap().total, 0);
        }
    }

    #[test]
    fn rename_borrows_unicode_title_and_preserves_every_other_byte() {
        let source = managed(&(row(0, 1, true, "Old") + &row(1, 2, false, "Child")));
        let context = context(&source);
        let layout = parse(&source, &context).unwrap();
        let patches = plan(
            &source,
            &context,
            &layout,
            StepIntent::Rename {
                step_id: id(1),
                title: "Réviser 日本語 & paths/file.rs",
            },
        )
        .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(matches!(&patches[0].replacement, Cow::Borrowed(_)));
        assert_eq!(
            apply(&source, &patches),
            source.replace("] Old <!--", "] Réviser 日本語 & paths/file.rs <!--")
        );
        for title in [
            "",
            "two\nlines",
            "forged <!-- cockpit-step: bad -->",
            "reserved cockpit-task: bad",
        ] {
            refusal(
                &source,
                StepIntent::Rename {
                    step_id: id(1),
                    title,
                },
            );
        }
    }

    #[test]
    fn oversized_saved_forest_is_complete_readable_and_safely_shrinkable() {
        let mut rows = String::new();
        for value in 1..=65 {
            rows.push_str(&row(0, value, false, &format!("Row {value}")));
        }
        let source = managed(&rows);
        let result = projection(&source);
        assert_eq!(result.steps.len(), 65);
        assert_eq!(
            (
                result.progress.as_ref().unwrap().done,
                result.progress.as_ref().unwrap().total
            ),
            (0, 65)
        );
        assert!(result.diagnostic.is_some());
        assert!(result.steps.iter().all(|step| step.diagnostic.is_none()));
        refusal(
            &source,
            StepIntent::Add {
                step_id: id(66),
                parent_step_id: None,
                before_step_id: None,
                title: "Growth",
            },
        );
        let checked = edit(
            &source,
            StepIntent::SetChecked {
                step_id: id(65),
                checked: true,
                scope: TaskStepScope::Leaf,
            },
        );
        assert!(checked.contains(&row(0, 65, true, "Row 65")));
        let checked_progress = projection(&checked).progress.unwrap();
        assert_eq!((checked_progress.done, checked_progress.total), (1, 65));
        let with_untracked = checked.replace(
            &format!("  {END}"),
            &format!("  - [x] Untracked leaf\n  {END}"),
        );
        let mixed_projection = projection(&with_untracked);
        assert_eq!(mixed_projection.steps.len(), 66);
        assert!(mixed_projection.steps[65].step_id.is_none());
        assert_eq!(
            (
                mixed_projection.progress.as_ref().unwrap().done,
                mixed_projection.progress.as_ref().unwrap().total
            ),
            (1, 65)
        );
        let shrunk = edit(&checked, StepIntent::Remove { step_id: id(64) });
        assert_eq!(projection(&shrunk).steps.len(), 64);
        let shrunk_progress = projection(&shrunk).progress.unwrap();
        assert_eq!((shrunk_progress.done, shrunk_progress.total), (1, 64));
        let long_title = "界".repeat(201);
        let source = managed(&row(0, 1, false, &long_title));
        assert_eq!(projection(&source).steps[0].title, long_title);
        assert!(projection(&source).steps[0].diagnostic.is_none());
        let long_projection = projection(&source);
        assert_eq!(
            (
                long_projection.progress.as_ref().unwrap().done,
                long_projection.progress.as_ref().unwrap().total
            ),
            (0, 1)
        );
        assert!(long_projection.diagnostic.is_some());
        let checked_long = edit(
            &source,
            StepIntent::SetChecked {
                step_id: id(1),
                checked: true,
                scope: TaskStepScope::Leaf,
            },
        );
        let checked_long_progress = projection(&checked_long).progress.unwrap();
        assert_eq!(
            (checked_long_progress.done, checked_long_progress.total),
            (1, 1)
        );
        refusal(
            &source,
            StepIntent::Rename {
                step_id: id(1),
                title: &"界".repeat(202),
            },
        );
        let changed = edit(
            &source,
            StepIntent::Rename {
                step_id: id(1),
                title: &"界".repeat(200),
            },
        );
        let renamed = projection(&changed);
        assert_eq!(
            (
                renamed.progress.as_ref().unwrap().done,
                renamed.progress.as_ref().unwrap().total
            ),
            (0, 1)
        );
        assert!(renamed.diagnostic.is_none());
    }

    #[test]
    fn excessive_depth_and_continuation_bytes_allow_non_growth_but_not_growth() {
        let mut rows = String::new();
        for depth in 0..=5 {
            rows.push_str(&row(depth, depth as u128 + 1, false, "Deep"));
        }
        let source = managed(&rows);
        assert_eq!(projection(&source).steps.len(), 6);
        let deep_projection = projection(&source);
        assert_eq!(
            (
                deep_projection.progress.as_ref().unwrap().done,
                deep_projection.progress.as_ref().unwrap().total
            ),
            (0, 1)
        );
        assert!(deep_projection.diagnostic.is_some());
        let checked_deep = edit(
            &source,
            StepIntent::SetChecked {
                step_id: id(6),
                checked: true,
                scope: TaskStepScope::Leaf,
            },
        );
        let deep_progress = projection(&checked_deep).progress.unwrap();
        assert_eq!((deep_progress.done, deep_progress.total), (1, 1));
        refusal(
            &source,
            StepIntent::Add {
                step_id: id(7),
                parent_step_id: Some(id(6)),
                before_step_id: None,
                title: "Deeper",
            },
        );
        let shallower = edit(
            &source,
            StepIntent::Move {
                step_id: id(6),
                parent_step_id: None,
                before_step_id: None,
            },
        );
        let shallower_projection = projection(&shallower);
        assert_eq!(
            (
                shallower_projection.progress.as_ref().unwrap().done,
                shallower_projection.progress.as_ref().unwrap().total
            ),
            (0, 2)
        );
        assert!(shallower_projection.diagnostic.is_none());
        let source = managed(&row(0, 1, false, "A title to shrink"));
        let mut context = context(&source);
        context.continuation_write_limit = 1;
        let layout = parse(&source, &context).unwrap();
        let oversized = project(&source, &context, &layout);
        assert_eq!(
            (
                oversized.progress.as_ref().unwrap().done,
                oversized.progress.as_ref().unwrap().total
            ),
            (0, 1)
        );
        assert!(oversized.diagnostic.is_some());
        assert!(
            plan(
                &source,
                &context,
                &layout,
                StepIntent::Rename {
                    step_id: id(1),
                    title: "Short"
                }
            )
            .is_ok()
        );
        let check_patches = plan(
            &source,
            &context,
            &layout,
            StepIntent::SetChecked {
                step_id: id(1),
                checked: true,
                scope: TaskStepScope::Leaf,
            },
        )
        .unwrap();
        let checked = apply(&source, &check_patches);
        let checked_layout = parse(&checked, &context).unwrap();
        let checked_projection = project(&checked, &context, &checked_layout);
        assert_eq!(
            (
                checked_projection.progress.as_ref().unwrap().done,
                checked_projection.progress.as_ref().unwrap().total
            ),
            (1, 1)
        );
        assert!(
            plan(
                &source,
                &context,
                &layout,
                StepIntent::Rename {
                    step_id: id(1),
                    title: "A much longer title that increases the violation"
                }
            )
            .is_err()
        );
    }

    #[test]
    fn patch_validator_rejects_top_relations_prose_marker_bytes_overlap_and_duplicate_gaps() {
        let source = managed(&row(0, 1, false, "A"));
        let context = context(&source);
        let layout = parse(&source, &context).unwrap();
        let node = &layout.nodes[0];
        for range in [
            context.top_checkbox_offset..context.top_checkbox_offset + 1,
            context.relation_record_range.clone().unwrap(),
            source.find("Prose").unwrap()..source.find("Prose").unwrap() + 1,
            node.marker_range.clone().unwrap(),
            node.title_range.start + 1..node.title_range.end + 1,
        ] {
            assert!(
                validate_patches(
                    &source,
                    &context,
                    &layout,
                    &[BytePatch {
                        range,
                        replacement: Cow::Borrowed(b"")
                    }]
                )
                .is_err()
            );
        }
        let gap = layout.managed_tail.as_ref().unwrap().root_append_offset;
        assert!(
            validate_patches(
                &source,
                &context,
                &layout,
                &[
                    BytePatch {
                        range: gap..gap,
                        replacement: Cow::Borrowed(b"a")
                    },
                    BytePatch {
                        range: gap..gap,
                        replacement: Cow::Borrowed(b"b")
                    },
                ]
            )
            .is_err()
        );
        assert!(
            validate_patches(
                &source,
                &context,
                &layout,
                &[
                    BytePatch {
                        range: node.subtree_range.clone().unwrap(),
                        replacement: Cow::Borrowed(b"")
                    },
                    BytePatch {
                        range: node.checkbox_offset..node.checkbox_offset + 1,
                        replacement: Cow::Borrowed(b"x")
                    },
                ]
            )
            .is_err()
        );
        assert!(
            validate_patches(
                &source,
                &context,
                &layout,
                &[BytePatch {
                    range: node.title_range.clone(),
                    replacement: Cow::Borrowed(&[0xff])
                },]
            )
            .is_err()
        );
    }

    #[test]
    fn add_before_managed_child_and_outdent_keep_saved_identity_order() {
        let source = managed(
            &(row(0, 1, true, "Parent")
                + &row(1, 2, true, "First child")
                + &row(1, 3, true, "Last child")
                + &row(0, 4, false, "Following root")),
        );
        let added = edit(
            &source,
            StepIntent::Add {
                step_id: id(5),
                parent_step_id: Some(id(1)),
                before_step_id: Some(id(3)),
                title: "Middle child",
            },
        );
        assert_eq!(
            added,
            managed(
                &(row(0, 1, false, "Parent")
                    + &row(1, 2, true, "First child")
                    + &row(1, 5, false, "Middle child")
                    + &row(1, 3, true, "Last child")
                    + &row(0, 4, false, "Following root"))
            )
        );
        let outdented = edit(
            &added,
            StepIntent::Move {
                step_id: id(5),
                parent_step_id: None,
                before_step_id: Some(id(4)),
            },
        );
        assert_eq!(
            outdented,
            managed(
                &(row(0, 1, true, "Parent")
                    + &row(1, 2, true, "First child")
                    + &row(1, 3, true, "Last child")
                    + &row(0, 5, false, "Middle child")
                    + &row(0, 4, false, "Following root"))
            )
        );
    }

    #[test]
    fn ordered_tracked_content_columns_are_used_without_canonicalizing_its_headers() {
        let source = external(&format!(
            "  1. [x] Parent <!-- cockpit-step: {} -->\n     - [x] Child <!-- cockpit-step: {} -->\n",
            id(1),
            id(2)
        ));
        let added = edit(
            &source,
            StepIntent::Add {
                step_id: id(3),
                parent_step_id: Some(id(1)),
                before_step_id: None,
                title: "Child at column five",
            },
        );
        assert_eq!(added, source.replace("  1. [x] Parent", "  1. [ ] Parent").replace(&format!("     - [x] Child <!-- cockpit-step: {} -->\n", id(2)), &format!("     - [x] Child <!-- cockpit-step: {} -->\n     - [ ] Child at column five <!-- cockpit-step: {} -->\n", id(2), id(3))));
        let removed = edit(&added, StepIntent::Remove { step_id: id(1) });
        assert_eq!(removed, external(""));
    }

    #[test]
    fn tabbed_untracked_headers_remain_readable() {
        let source = external("  -\t[X] Tabbed external\n");
        let result = projection(&source);
        assert_eq!(result.steps.len(), 1);
        assert!(result.steps[0].diagnostic.is_none());
        assert!(result.steps[0].checked);
        assert!(result.steps[0].step_id.is_none());
    }

    #[test]
    fn tabbed_tracked_headers_can_be_checked_but_not_guessed_for_reindent() {
        let source = external(&format!(
            "  -\t[X] Tabbed tracked <!-- cockpit-step: {} -->\n",
            id(1)
        ));
        let result = projection(&source);
        assert_eq!(result.steps.len(), 1);
        assert!(result.steps[0].diagnostic.is_none());
        let checked = edit(
            &source,
            StepIntent::SetChecked {
                step_id: id(1),
                checked: false,
                scope: TaskStepScope::Leaf,
            },
        );
        assert_eq!(checked, source.replace("-\t[X]", "-\t[ ]"));
        refusal(
            &source,
            StepIntent::Add {
                step_id: id(2),
                parent_step_id: Some(id(1)),
                before_step_id: None,
                title: "Cannot guess tabs",
            },
        );
    }

    #[test]
    fn moving_eof_header_without_eol_does_not_concatenate_destination_lines() {
        let source = format!(
            "- [ ] Task <!-- cockpit-task: {} -->\n{}{}",
            id(1000),
            row(0, 1, false, "First"),
            row(0, 2, false, "Last").trim_end_matches('\n')
        );
        let changed = edit(
            &source,
            StepIntent::Move {
                step_id: id(2),
                parent_step_id: None,
                before_step_id: Some(id(1)),
            },
        );
        assert_eq!(
            changed,
            format!(
                "- [ ] Task <!-- cockpit-task: {} -->\n{}{}",
                id(1000),
                row(0, 2, false, "Last"),
                row(0, 1, false, "First")
            )
        );
        assert_eq!(projection(&changed).steps.len(), 2);
    }

    #[test]
    fn external_insertion_gaps_and_terminal_creation_are_proved_not_inferred_from_protected_union()
    {
        let source = external(&row(0, 1, false, "Safe tracked"));
        let context = context(&source);
        let layout = parse(&source, &context).unwrap();
        let child_gap = layout.nodes[0].subtree_range.as_ref().unwrap().end;
        assert!(
            validate_patches(
                &source,
                &context,
                &layout,
                &[BytePatch {
                    range: child_gap..child_gap,
                    replacement: Cow::Borrowed(b"")
                }]
            )
            .is_ok()
        );
        let terminal_gap = context.continuation_range.end;
        assert!(
            validate_patches(
                &source,
                &context,
                &layout,
                &[BytePatch {
                    range: terminal_gap..terminal_gap,
                    replacement: Cow::Borrowed(b"")
                }]
            )
            .is_ok()
        );
        let prose_gap = source.find("After prose").unwrap();
        assert!(
            validate_patches(
                &source,
                &context,
                &layout,
                &[BytePatch {
                    range: prose_gap..prose_gap,
                    replacement: Cow::Borrowed(b"")
                }]
            )
            .is_err()
        );
        let source = format!(
            "- [ ] Task <!-- cockpit-task: {} -->\n  ```\n  unclosed\n",
            id(1000)
        );
        let context = super::tests::context(&source);
        let layout = parse(&source, &context).unwrap();
        let gap = context.continuation_range.end;
        assert!(
            validate_patches(
                &source,
                &context,
                &layout,
                &[BytePatch {
                    range: gap..gap,
                    replacement: Cow::Borrowed(b"")
                }]
            )
            .is_err()
        );
    }
}
