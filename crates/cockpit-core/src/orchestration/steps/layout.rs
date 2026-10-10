use std::{collections::{HashMap, HashSet}, ops::Range};

use uuid::Uuid;

use super::{
    blank, contains, diagnose, node_diagnostic, pure_headers, range_at, union_ranges, violations,
    Example, Line, ManagedStepTail, ModelNode, StepLayout, StepParseContext, BEGIN, END, STEP,
};

pub(super) fn read_step_markers(
    source: &str,
    inline_code: &[Range<usize>],
    html_tokens: &HashMap<usize, usize>,
    layout: &mut StepLayout,
) {
    // Metadata must be a trailing, real HTML token, not an inline-code example.
    for node in &mut layout.nodes {
        let title = &source[node.title_range.clone()];
        let mut markers = title.match_indices(STEP).filter_map(|(offset, _)| {
            let offset = node.title_range.start + offset;
            (!range_at(inline_code, offset)).then_some(offset)
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
}

pub(super) fn read_managed_tail(
    source: &str,
    context: &StepParseContext,
    physical: &[Line],
    example_ranges: &[Range<usize>],
    inline_code: &[Range<usize>],
    header_lines: &HashSet<usize>,
    layout: &mut StepLayout,
) -> bool {
    let in_example =
        |offset: usize| range_at(example_ranges, offset) || range_at(inline_code, offset);
    let mut controls: Vec<(bool, Range<usize>)> = Vec::new();
    for line in physical {
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
                diagnose(layout, "malformed checklist boundary");
                layout.hierarchy_invalid = true;
            }
        }
        for (relative, _) in raw.match_indices(STEP) {
            let offset = line.start + relative;
            if !in_example(offset) && !header_lines.contains(&line.start) {
                layout.protected_ranges.push(line.start..line.end);
                diagnose(
                    layout,
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
                    layout,
                    "checklist tail contains unrelated text, ambiguous ancestry, or following prose",
                );
                layout.hierarchy_invalid = true;
            }
        } else {
            diagnose(
                layout,
                "multiple, nested, or unclosed checklist boundaries",
            );
            layout.hierarchy_invalid = true;
        }
    }
    controls.is_empty()
}

pub(super) fn prove_subtree_ranges(
    source: &str,
    context: &StepParseContext,
    physical: &[Line],
    header_lines: &HashSet<usize>,
    layout: &mut StepLayout,
) {
    let mut nonheader = Vec::with_capacity(physical.len() + 1);
    nonheader.push(0usize);
    let mut next_nonwhite = vec![context.item_range.end; physical.len() + 1];
    for line in physical {
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
}

pub(super) fn finalize_layout(
    source: &str,
    context: &StepParseContext,
    model: &[ModelNode],
    examples: &[Example],
    controls_empty: bool,
    layout: &mut StepLayout,
) {
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
            layout,
            "step identities are malformed or duplicated; correct canonical source",
        );
    }
    if layout.hierarchy_invalid {
        diagnose(
            layout,
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
            layout,
            "some checkbox subtrees cannot be safely moved or removed",
        );
    }
    let limits = violations(context, model, context.continuation_range.len());
    layout.limits_exceeded = limits.iter().any(|&value| value > 0);
    if layout.limits_exceeded {
        diagnose(
            layout,
            "saved checklist exceeds count, depth, title, or continuation byte limits; safe non-growing edits remain available",
        );
    }
    if controls_empty && !layout.identity_invalid && !layout.hierarchy_invalid {
        let last_nonwhite = source[context.continuation_range.clone()].trim_end().len()
            + context.continuation_range.start;
        if !examples
            .iter()
            .any(|e| e.terminal_open && e.range.end >= last_nonwhite)
        {
            layout.tail_creation_gap = Some(context.continuation_range.end);
        }
    }
    layout.protected_ranges = union_ranges(std::mem::take(&mut layout.protected_ranges));
}
