use crate::{
    library::store::LibraryIndexEntry,
    sources::{IssueRow, SpacePage},
};
use cockpit_protocol::library::{LibraryAttachmentState, LibraryItemState};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::library) enum ChangeReason {
    Moved,
    Changed,
    Renamed,
    Rechecked,
    AttachmentsRequested,
    Confirming,
}
impl fmt::Display for ChangeReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Moved => "moved",
            Self::Changed => "changed",
            Self::Renamed => "renamed",
            Self::Rechecked => "rechecked",
            Self::AttachmentsRequested => "attachments requested",
            Self::Confirming => "confirming",
        })
    }
}
impl ChangeReason {
    pub(in crate::library) fn of_page(old: &LibraryIndexEntry, page: &SpacePage) -> Option<Self> {
        if !old
            .summary
            .ancestors
            .iter()
            .map(|a| a.id.as_str())
            .eq(page.ancestors.iter().map(String::as_str))
        {
            Some(Self::Moved)
        } else if old.summary.source_revision.as_deref() != Some(page.version.to_string().as_str())
        {
            Some(Self::Changed)
        } else if old.summary.title != page.title {
            Some(Self::Renamed)
        } else if matches!(
            old.summary.state,
            LibraryItemState::RemovedAtSource
                | LibraryItemState::Failed
                | LibraryItemState::Unknown
        ) {
            Some(Self::Rechecked)
        } else {
            None
        }
    }
    pub(in crate::library) fn of_issue(old: &LibraryIndexEntry, row: &IssueRow) -> Option<Self> {
        if matches!(
            old.summary.state,
            LibraryItemState::RemovedAtSource
                | LibraryItemState::Failed
                | LibraryItemState::Unknown
        ) {
            Some(Self::Rechecked)
        } else if old
            .summary
            .issue
            .as_ref()
            .and_then(|meta| meta.fetched_updated.as_deref())
            != Some(row.updated.as_str())
        {
            Some(Self::Changed)
        } else {
            None
        }
    }
    pub(in crate::library) fn missing_attachments(old: &LibraryIndexEntry) -> bool {
        old.summary.attachments.iter().any(|a| {
            matches!(
                a.state,
                LibraryAttachmentState::NotDownloaded | LibraryAttachmentState::Failed
            )
        })
    }
}
