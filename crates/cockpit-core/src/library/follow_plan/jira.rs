use super::{Absent, ChangeReason, Classified, Pass, Planned, Snapshot, Window, until_cancelled};
use crate::{
    InspectionError,
    library::{
        LibraryService,
        follow::recount,
        jira_follow::{is_issue_of, issue_item_id},
        refs,
        store::Index,
    },
    sources::{IssueListing, IssueQuery, IssueRow},
};
use cockpit_protocol::library::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
};

pub(in crate::library) struct JiraFollowPlan {
    pub listing: IssueListing,
    pub snapshot: Snapshot,
}
/// Manual membership historically includes every non-related holder; background
/// restricts it to issue identities of the followed site.
pub(in crate::library) enum MemberScope {
    Manual,
    SiteIssues,
}
pub(in crate::library) struct CheckedMembers {
    pub rows: Vec<IssueRow>,
    pub complete: bool,
}
impl JiraFollowPlan {
    pub(in crate::library) async fn list(
        service: &LibraryService,
        follow: &LibraryFollowSummary,
        jql: &str,
        window: Window,
        pass: Pass<'_>,
    ) -> Result<Option<IssueListing>, InspectionError> {
        Self::list_with(
            service,
            follow,
            IssueQuery::Jql {
                jql,
                updated_window: window.updated_window(),
            },
            service.configuration.limits.library_space_pages.max(1),
            pass,
        )
        .await
    }
    async fn list_with(
        service: &LibraryService,
        follow: &LibraryFollowSummary,
        query: IssueQuery<'_>,
        max: u32,
        pass: Pass<'_>,
    ) -> Result<Option<IssueListing>, InspectionError> {
        let cancel = AtomicBool::new(false);
        let future = service
            .sources
            .list_issues(&follow.provider_id, &query, max, &cancel);
        match pass {
            Pass::Manual { store, operation } => until_cancelled(store, operation, &cancel, future)
                .await?
                .transpose(),
            Pass::Background => future.await.map(Some),
        }
    }
    pub(in crate::library) fn members(
        &self,
        follow: &LibraryFollowSummary,
        scope: MemberScope,
    ) -> BTreeSet<String> {
        self.snapshot
            .items
            .values()
            .filter(|entry| {
                (matches!(scope, MemberScope::Manual) || is_issue_of(entry, follow))
                    && refs::has_follow(&entry.summary, &follow.follow_id)
                    && !refs::related_of(&entry.summary, &follow.follow_id)
            })
            .filter_map(|entry| entry.summary.canonical_id.clone())
            .filter(|key| {
                matches!(scope, MemberScope::Manual) || !self.snapshot.excluded.contains(key)
            })
            .collect()
    }
    pub(in crate::library) fn absent(
        &self,
        members: &BTreeSet<String>,
        present: &BTreeSet<String>,
    ) -> Absent {
        Absent {
            members: members.len(),
            missing: members
                .difference(present)
                .filter(|key| !self.snapshot.excluded.contains(*key))
                .cloned()
                .collect(),
        }
    }
    pub(in crate::library) async fn check_members(
        &self,
        service: &LibraryService,
        follow: &LibraryFollowSummary,
        keys: &[String],
        pass: Pass<'_>,
    ) -> Result<Option<CheckedMembers>, InspectionError> {
        let mut result = CheckedMembers {
            rows: Vec::new(),
            complete: true,
        };
        let chunk_size = match pass {
            Pass::Manual { .. } => keys.len().max(1),
            Pass::Background => 100,
        };
        for batch in keys.chunks(chunk_size) {
            let Some(listing) = Self::list_with(
                service,
                follow,
                IssueQuery::Keys(batch),
                batch.len() as u32,
                pass,
            )
            .await?
            else {
                return Ok(None);
            };
            result.complete &= listing.complete;
            result.rows.extend(listing.rows);
        }
        Ok(Some(result))
    }
    pub(in crate::library) fn classify<'r>(
        &self,
        follow: &LibraryFollowSummary,
        rows: impl IntoIterator<Item = &'r IssueRow>,
        attachments: bool,
        manual: bool,
    ) -> Classified<'r, IssueRow> {
        let mut result = Classified {
            fetch: Vec::new(),
            unchanged: 0,
        };
        for row in rows {
            if !manual && self.snapshot.excluded.contains(&row.key) {
                continue;
            }
            let old = self.snapshot.items.get(&issue_item_id(follow, &row.key));
            let reason = match old {
                None => None,
                Some(old) => match ChangeReason::of_issue(old, row) {
                    Some(reason) => Some(reason),
                    None if attachments && ChangeReason::missing_attachments(old) => {
                        Some(ChangeReason::AttachmentsRequested)
                    }
                    None => {
                        result.unchanged += 1;
                        continue;
                    }
                },
            };
            result.fetch.push(Planned {
                key: row.key.as_str().into(),
                row: Some(row.clone()),
                old: if manual { old.cloned() } else { None },
                reason,
            });
        }
        result
    }
}

pub(in crate::library) fn apply_issues(
    index: &mut Index,
    follow: &LibraryFollowSummary,
    listed: &BTreeMap<&str, &IssueRow>,
    meta_rows: &BTreeMap<&str, &IssueRow>,
    manual: bool,
) -> Option<bool> {
    let excluded = &index
        .follows
        .iter()
        .find(|f| f.follow_id == follow.follow_id)?
        .excluded_ids;
    let mut changed = false;
    for entry in &mut index.items {
        if !is_issue_of(entry, follow) {
            continue;
        }
        let Some(key) = entry.summary.canonical_id.as_deref() else {
            continue;
        };
        if excluded.iter().any(|id| id == key) {
            continue;
        }
        let listed_row = listed.get(key);
        let meta_row = meta_rows.get(key);
        // Background only mutates listed rows; manual also refreshes checked metadata.
        if !manual && listed_row.is_none() {
            continue;
        }
        if listed_row.is_some()
            && (manual
                || !refs::has_follow(&entry.summary, &follow.follow_id)
                || refs::related_of(&entry.summary, &follow.follow_id))
        {
            refs::insert_ref(
                &mut entry.summary,
                LibraryItemRef::Follow {
                    follow_id: follow.follow_id.clone(),
                },
            );
            refs::strip_inclusion(
                &mut entry.summary,
                &LibraryInclusionHolder::Follow {
                    follow_id: follow.follow_id.clone(),
                },
            );
            changed |= !manual;
        }
        if let Some(row) = meta_row {
            let same = !manual
                && entry.summary.issue.as_ref().is_some_and(|meta| {
                    meta.updated == row.updated
                        && meta.status == row.status
                        && meta.issue_type == row.issue_type
                        && meta.assignee == row.assignee
                });
            if manual || !same {
                entry.summary.issue = Some(LibraryIssueMeta {
                    updated: row.updated.clone(),
                    fetched_updated: entry
                        .summary
                        .issue
                        .as_ref()
                        .and_then(|meta| meta.fetched_updated.clone()),
                    status: row.status.clone(),
                    issue_type: row.issue_type.clone(),
                    assignee: row.assignee.clone(),
                });
                changed |= !same;
            }
        }
    }
    if changed && !manual {
        recount(index, &follow.follow_id);
    }
    Some(changed)
}
