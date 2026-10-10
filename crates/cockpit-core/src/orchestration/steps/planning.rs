use std::borrow::Cow;

use uuid::Uuid;

use super::{
    destination, emitted_step, error, insertion, leaf_conversions, mark_ancestors, next_sibling,
    normalize, owned_subtree, reindent, target, title_valid, violations, BytePatch, ModelNode,
    StepLayout, StepParseContext, Summary, MAX_STEP_DEPTH, MAX_STEP_TITLE_SCALARS, MAX_TRACKED_STEPS,
};
use crate::InspectionError;

pub(super) fn plan_add<'a>(
    source: &'a str,
    context: &StepParseContext,
    layout: &StepLayout,
    nodes: &mut Vec<ModelNode>,
    before: &[Summary],
    affected: &mut [bool],
    patches: &mut Vec<BytePatch<'a>>,
    step_id: Uuid,
    parent_step_id: Option<Uuid>,
    before_step_id: Option<Uuid>,
    title: &'a str,
) -> Result<(), InspectionError> {
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
        mark_ancestors(nodes, dest.parent, affected);
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
        let after = leaf_conversions(nodes, before)?;
        normalize(layout, nodes, &after, affected, patches);
    }
    Ok(())
}

pub(super) fn plan_move<'a>(
    source: &'a str,
    context: &StepParseContext,
    layout: &StepLayout,
    nodes: &mut Vec<ModelNode>,
    before: &[Summary],
    affected: &mut [bool],
    patches: &mut Vec<BytePatch<'a>>,
    step_id: Uuid,
    parent_step_id: Option<Uuid>,
    before_step_id: Option<Uuid>,
    previous_limits: [usize; 4],
) -> Result<(), InspectionError> {
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
    if violations(context, nodes, context.continuation_range.len())
        .iter()
        .zip(previous_limits)
        .any(|(&after, before)| after > before)
    {
        return Err(error(
            "task_step_limit",
            "move increases a saved depth-limit violation",
        ));
    }
    mark_ancestors(nodes, nodes[index].parent, affected);
    mark_ancestors(nodes, dest.parent, affected);
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
    let after = leaf_conversions(nodes, before)?;
    normalize(layout, nodes, &after, affected, patches);
    Ok(())
}
