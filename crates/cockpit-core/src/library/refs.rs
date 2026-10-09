//! Item reference set helpers. An item is kept while it holds at least one
//! reference; the caller tombstones it (`purge_after`) when the last one goes.
use crate::{
    InspectionError,
    library::{
        operations,
        store::{Store, error},
    },
    project_store::timestamp,
};
use cockpit_protocol::library::{
    LibraryFollowSource, LibraryFollowSummary, LibraryInclusion, LibraryInclusionHolder,
    LibraryItemRef, LibraryItemSummary, LibraryReportOutcome,
};

/// Grace period between an item losing its last reference and its purge.
pub(crate) const TOMBSTONE_GRACE_MS: u128 = 14 * 24 * 3600 * 1000;

pub(crate) fn has_follow(summary: &LibraryItemSummary, follow_id: &str) -> bool {
    summary
        .refs
        .iter()
        .any(|reference| matches!(reference, LibraryItemRef::Follow { follow_id: id } if id == follow_id))
}

/// The follow that lists this item, if any (the first one in ref order).
pub(crate) fn first_follow(summary: &LibraryItemSummary) -> Option<&str> {
    summary.refs.iter().find_map(|reference| match reference {
        LibraryItemRef::Follow { follow_id } => Some(follow_id.as_str()),
        _ => None,
    })
}

/// Every follow that lists this item.
pub(crate) fn follow_ids(summary: &LibraryItemSummary) -> Vec<String> {
    summary
        .refs
        .iter()
        .filter_map(|reference| match reference {
            LibraryItemRef::Follow { follow_id } => Some(follow_id.clone()),
            _ => None,
        })
        .collect()
}

/// Inserts keeping `refs` sorted and unique. A referenced item is never purged.
pub(crate) fn insert_ref(summary: &mut LibraryItemSummary, reference: LibraryItemRef) {
    if let Err(at) = summary.refs.binary_search(&reference) {
        summary.refs.insert(at, reference);
    }
    summary.purge_after = None;
}

/// Returns whether the reference was present. Does not tombstone. Removing
/// `Follow{F}` also strips the `Follow{F}` inclusion, so drop, stop following and
/// remove cannot leave a reason for a membership that no longer exists.
pub(crate) fn remove_ref(summary: &mut LibraryItemSummary, reference: &LibraryItemRef) -> bool {
    match summary.refs.binary_search(reference) {
        Ok(at) => {
            summary.refs.remove(at);
            if let LibraryItemRef::Follow { follow_id } = reference {
                strip_inclusion(
                    summary,
                    &LibraryInclusionHolder::Follow {
                        follow_id: follow_id.clone(),
                    },
                );
            }
            true
        }
        Err(_) => false,
    }
}

/// Release a follow membership and its inclusion, tombstoning only the last ref.
pub(crate) fn release_follow(summary: &mut LibraryItemSummary, follow_id: &str, now_ms: u128) {
    remove_ref(
        summary,
        &LibraryItemRef::Follow {
            follow_id: follow_id.to_owned(),
        },
    );
    if summary.refs.is_empty() {
        summary.purge_after = Some((now_ms + TOMBSTONE_GRACE_MS).to_string());
    }
}

/// Whether `follow_id` holds this item as a related item (not as a listed seed).
pub(crate) fn related_of(summary: &LibraryItemSummary, follow_id: &str) -> bool {
    summary.included_by.iter().flatten().any(|inclusion| {
        matches!(&inclusion.holder, LibraryInclusionHolder::Follow { follow_id: id } if id == follow_id)
    })
}

/// Records why the holder includes this item. There is one entry per holder;
/// a fresh traversal replaces it so a stale route is never kept.
pub(crate) fn set_inclusion(summary: &mut LibraryItemSummary, inclusion: LibraryInclusion) {
    let list = summary.included_by.get_or_insert_with(Vec::new);
    match list
        .iter_mut()
        .find(|existing| existing.holder == inclusion.holder)
    {
        Some(existing) => *existing = inclusion,
        None => list.push(inclusion),
    }
}

/// Removes the holder's inclusion; an empty list becomes `None`.
pub(crate) fn strip_inclusion(summary: &mut LibraryItemSummary, holder: &LibraryInclusionHolder) {
    if let Some(list) = &mut summary.included_by {
        list.retain(|inclusion| &inclusion.holder != holder);
        if list.is_empty() {
            summary.included_by = None;
        }
    }
}

/// The Confluence space key of a follow, or `None` for other follow kinds.
pub(crate) fn space_key(follow: &LibraryFollowSummary) -> Option<&str> {
    match &follow.source {
        LibraryFollowSource::ConfluenceSpace { space_key, .. } => Some(space_key),
        LibraryFollowSource::JiraQuery { .. } => None,
    }
}

/// Like `space_key`, but a Jira query is a capability error for Confluence-only paths.
pub(crate) fn require_space_key(follow: &LibraryFollowSummary) -> Result<&str, InspectionError> {
    space_key(follow).ok_or_else(|| {
        error(
            "source_capability_unavailable",
            "Jira follows can't be added to a Space yet",
        )
    })
}

pub(crate) fn set_space_name(follow: &mut LibraryFollowSummary, name: &str) {
    if let LibraryFollowSource::ConfluenceSpace { space_name, .. } = &mut follow.source {
        *space_name = name.to_owned();
    }
}

/// `KEY · Name` for a space follow, the JQL for a query follow.
pub(crate) fn follow_title(follow: &LibraryFollowSummary) -> String {
    match &follow.source {
        LibraryFollowSource::ConfluenceSpace {
            space_key,
            space_name,
        } => {
            format!("{space_key} · {space_name}")
        }
        LibraryFollowSource::JiraQuery { jql, .. } => jql.clone(),
    }
}

/// Removes every item that has been unreferenced for the grace
/// period. An item edited in the Library is kept and reported; a busy item, or
/// one that gained a reference since it was listed, is skipped without a row.
/// Does nothing once `operation` was cancelled.
pub(crate) fn purge_expired(store: &Store, operation: &str) -> Result<(), InspectionError> {
    let now = timestamp().parse::<u128>().unwrap_or(0);
    let due = {
        let _lock = store.shared()?;
        store
            .index()?
            .items
            .into_iter()
            .filter(|entry| {
                entry.summary.refs.is_empty()
                    && entry
                        .summary
                        .purge_after
                        .as_deref()
                        .and_then(|at| at.parse::<u128>().ok())
                        .is_some_and(|at| at <= now)
            })
            .collect::<Vec<_>>()
    };
    for entry in due {
        if operations::cancelled(store, operation)? {
            break;
        }
        let edited = {
            let _lock = store.shared()?;
            !store.conflicts(&entry)?.is_empty()
        };
        if edited {
            operations::row(
                store,
                operation,
                Some(&entry.summary),
                LibraryReportOutcome::Dropped,
                Some("kept: edited in Library".into()),
            )?;
            continue;
        }
        let observed = entry.summary.purge_after.clone();
        let removed =
            store.remove_where(&entry.summary.item_id, &entry.summary.revision, |current| {
                if current.summary.refs.is_empty() && current.summary.purge_after == observed {
                    Ok(())
                } else {
                    Err(error(
                        "library_item_referenced",
                        "Library item gained a reference",
                    ))
                }
            });
        match removed {
            Ok(()) => operations::row(
                store,
                operation,
                Some(&entry.summary),
                LibraryReportOutcome::Dropped,
                Some("purged after 14 days unreferenced".into()),
            )?,
            Err(failure)
                if matches!(
                    failure.code.as_str(),
                    "library_item_busy"
                        | "library_item_referenced"
                        | "library_item_not_found"
                        | "library_conflict"
                ) => {}
            Err(failure) => operations::row(
                store,
                operation,
                Some(&entry.summary),
                LibraryReportOutcome::Failed,
                Some(failure.message),
            )?,
        }
    }
    Ok(())
}
