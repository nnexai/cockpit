//! Shared provider stages; callers retain fetch, publication and absence authority.
mod cancel;
mod confluence;
mod jira;
mod reason;

pub(in crate::library) use cancel::until_cancelled;
pub(in crate::library) use confluence::{
    ConfluenceFollowPlan, PageClassify, PageOrder, PageProbe, apply_pages,
};
pub(in crate::library) use jira::{JiraFollowPlan, MemberScope, apply_issues};
pub(in crate::library) use reason::ChangeReason;

use crate::{
    InspectionError,
    library::store::{LibraryIndexEntry, Store},
};
use cockpit_protocol::library::*;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::library) enum Window {
    Full,
    Delta { lower: i64, upper: i64 },
}
impl Window {
    pub(in crate::library) fn from_delta(window: Option<(i64, i64)>) -> Self {
        window.map_or(Self::Full, |(lower, upper)| Self::Delta { lower, upper })
    }
    fn updated_window(self) -> Option<(i64, i64)> {
        match self {
            Self::Full => None,
            Self::Delta { lower, upper } => Some((lower, upper)),
        }
    }
}
#[derive(Clone, Copy)]
pub(in crate::library) enum Pass<'a> {
    Manual {
        store: &'a Store,
        operation: &'a str,
    },
    Background,
}

pub(in crate::library) struct Snapshot {
    pub items: BTreeMap<String, LibraryIndexEntry>,
    pub record: Option<LibraryFollowSummary>,
    pub excluded: BTreeSet<String>,
}
impl Snapshot {
    pub(in crate::library) fn read(
        store: &Store,
        follow_id: &str,
    ) -> Result<Self, InspectionError> {
        let index = {
            let _lock = store.shared()?;
            store.index()?
        };
        let record = index.follows.into_iter().find(|f| f.follow_id == follow_id);
        let excluded = record
            .as_ref()
            .map(|f| f.excluded_ids.iter().cloned().collect())
            .unwrap_or_default();
        let items = index
            .items
            .into_iter()
            .map(|entry| (entry.summary.item_id.clone(), entry))
            .collect();
        Ok(Self {
            items,
            record,
            excluded,
        })
    }
    pub(in crate::library) fn depth(&self) -> u32 {
        self.record
            .as_ref()
            .and_then(|f| f.reference_depth)
            .unwrap_or(0)
    }
}
pub(in crate::library) struct Planned<'a, R> {
    pub key: Cow<'a, str>,
    pub row: Option<R>,
    pub old: Option<LibraryIndexEntry>,
    pub reason: Option<ChangeReason>,
}
pub(in crate::library) struct Classified<'a, R> {
    pub fetch: Vec<Planned<'a, R>>,
    pub unchanged: u32,
}
pub(in crate::library) struct Absent {
    pub members: usize,
    pub missing: BTreeSet<String>,
}
pub(in crate::library) fn upsert_record(
    store: &Store,
    follow: &LibraryFollowSummary,
    depth: bool,
) -> Result<(), InspectionError> {
    store.mutate_index(|index| {
        match index
            .follows
            .iter_mut()
            .find(|f| f.follow_id == follow.follow_id)
        {
            Some(record) => {
                record.excluded_ids.clear();
                record.source = follow.source.clone();
                record.include_attachments = follow.include_attachments;
                if depth {
                    record.reference_depth = follow.reference_depth;
                }
            }
            None => index.follows.push(follow.clone()),
        }
        Ok(())
    })
}
pub(in crate::library) fn mark_failed(
    store: &Store,
    follow_id: &str,
) -> Result<(), InspectionError> {
    store.mutate_index(|index| {
        if let Some(record) = index.follows.iter_mut().find(|f| f.follow_id == follow_id) {
            record.state = LibraryItemState::Failed;
        }
        Ok(())
    })
}
