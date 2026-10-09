use std::{collections::HashMap, ops::Range};

use cockpit_protocol::orchestration::TaskStepStatus;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use super::{
    closed_fence, contains, invalid_patch, node_diagnostic, open_html, valid_list_prefix, Example,
    Frame, Line, StepLayout, StepNode, StepParseContext, BEGIN, END,
};
use crate::InspectionError;

pub(super) struct EventInventory {
    pub(super) examples: Vec<Example>,
    pub(super) inline_code: Vec<Range<usize>>,
    pub(super) html_tokens: HashMap<usize, usize>,
}

pub(super) fn collect_events(
    source: &str,
    context: &StepParseContext,
    physical: &[Line],
    layout: &mut StepLayout,
) -> Result<EventInventory, InspectionError> {
    let mut stack: Vec<Frame> = Vec::new();
    let mut examples: Vec<Example> = Vec::new();
    let mut inline_code: Vec<Range<usize>> = Vec::new();
    let mut quote_depth = 0usize;
    let mut code_depth = 0usize;
    let mut html_tokens: HashMap<usize, usize> = HashMap::new();
    let mut line_cursor = 0usize;
    let base = context.item_range.start;
    for (event, relative) in Parser::new_ext(
        &source[context.item_range.clone()],
        Options::ENABLE_TASKLISTS,
    )
    .into_offset_iter()
    {
        let range = base + relative.start..base + relative.end;
        match event {
            Event::Start(Tag::BlockQuote(_)) => {
                quote_depth += 1;
                examples.push(Example {
                    range,
                    terminal_open: false,
                });
            }
            Event::End(TagEnd::BlockQuote(_)) => quote_depth = quote_depth.saturating_sub(1),
            Event::Start(Tag::CodeBlock(kind)) => {
                code_depth += 1;
                let terminal_open = match kind {
                    CodeBlockKind::Indented => false,
                    CodeBlockKind::Fenced(_) => !closed_fence(source, &range),
                };
                examples.push(Example {
                    range,
                    terminal_open,
                });
            }
            Event::End(TagEnd::CodeBlock) => code_depth = code_depth.saturating_sub(1),
            Event::Start(Tag::HtmlBlock) => {
                let text = source[range.clone()].trim();
                // Our standalone comment records are HTML themselves, not HTML
                // examples. A containing <div>, script or ordinary comment is.
                if !(text == BEGIN || text == END || text.starts_with("<!-- cockpit-checklist:")) {
                    examples.push(Example {
                        terminal_open: open_html(text),
                        range,
                    });
                }
            }
            Event::Code(_) => inline_code.push(range),
            Event::Html(_) | Event::InlineHtml(_) => {
                html_tokens.insert(range.start, range.end);
            }
            Event::Start(Tag::Item) => {
                let parent = stack.last();
                stack.push(Frame {
                    range,
                    node: None,
                    top: false,
                    nearest_node: parent.and_then(|p| p.node.or(p.nearest_node)),
                    under_top: parent.is_some_and(|p| p.top || p.under_top),
                    ordinary_ancestor: parent
                        .is_some_and(|p| p.ordinary_ancestor || !p.top && p.node.is_none()),
                });
            }
            Event::End(TagEnd::Item) => {
                stack.pop();
            }
            Event::TaskListMarker(checked) if quote_depth == 0 && code_depth == 0 => {
                let checkbox_offset = range.end.saturating_sub(2);
                if checkbox_offset == context.top_checkbox_offset {
                    if let Some(frame) = stack.last_mut() {
                        frame.top = true;
                    }
                    continue;
                }
                if !contains(&context.continuation_range, &range)
                    || context
                        .relation_record_range
                        .as_ref()
                        .is_some_and(|r| contains(r, &range))
                {
                    continue;
                }
                let Some(frame) = stack.last() else {
                    continue;
                };
                while line_cursor + 1 < physical.len() && physical[line_cursor].end <= range.start {
                    line_cursor += 1;
                }
                let line_index = line_cursor;
                let line = &physical[line_index];
                let checkbox_start = checkbox_offset - 1;
                u32::try_from(line.start - context.item_range.start)
                    .map_err(|_| invalid_patch())?;
                let prefix = &source[line.start..checkbox_start];
                let indent = prefix.bytes().take_while(|b| *b == b' ').count();
                let simple_indent = (!prefix.contains('\t')
                    && valid_list_prefix(&prefix[indent..]))
                .then_some(indent);
                let child_indent = simple_indent.map(|_| checkbox_start - line.start);
                let mut title_start = range.end;
                while title_start < line.content_end
                    && matches!(source.as_bytes()[title_start], b' ' | b'\t')
                {
                    title_start += 1;
                }
                let mut title_end = line.content_end;
                while title_end > title_start
                    && matches!(source.as_bytes()[title_end - 1], b' ' | b'\t')
                {
                    title_end -= 1;
                }
                let parent_index = frame.nearest_node;
                let depth = parent_index.map_or(0, |i| layout.nodes[i].depth + 1);
                let ambiguous = !frame.under_top || frame.ordinary_ancestor;
                let mut node = StepNode {
                    step_id: None,
                    parent_index,
                    depth,
                    subtree_end_index: layout.nodes.len() + 1,
                    line: u32::try_from(context.item_line + line_index)
                        .map_err(|_| invalid_patch())?,
                    header_range: line.start..line.end,
                    title_range: title_start..title_end,
                    checkbox_offset,
                    marker_range: None,
                    subtree_range: None,
                    checked,
                    status: if checked {
                        TaskStepStatus::Done
                    } else {
                        TaskStepStatus::Open
                    },
                    physical_line: line_index,
                    title_scalars: 0,
                    diagnostic: None,
                    parser_range: frame.range.clone(),
                    indent: simple_indent,
                    child_indent,
                };
                if ambiguous {
                    node_diagnostic(
                        &mut node,
                        "checkbox has an ordinary or noncanonical list ancestor",
                    );
                    layout.hierarchy_invalid = true;
                }
                if source.as_bytes().get(checkbox_offset.wrapping_sub(1)) != Some(&b'[')
                    || source.as_bytes().get(checkbox_offset + 1) != Some(&b']')
                {
                    node_diagnostic(&mut node, "checkbox source slot is not safely identified");
                    layout.hierarchy_invalid = true;
                }
                let index = layout.nodes.len();
                layout.nodes.push(node);
                stack
                    .last_mut()
                    .expect("task marker has an item frame")
                    .node = Some(index);
            }
            _ => {}
        }
    }
    Ok(EventInventory {
        examples,
        inline_code,
        html_tokens,
    })
}
