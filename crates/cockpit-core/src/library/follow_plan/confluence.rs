use super::{Absent, ChangeReason, Classified, Pass, Planned, Snapshot, Window, until_cancelled};
use crate::{
    InspectionError,
    library::{
        LibraryService,
        follow::{page_item_id, recount, same_site_page},
        refs,
        store::Index,
    },
    sources::{SpacePage, SpacePageListing},
};
use cockpit_protocol::library::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
};

pub(in crate::library) struct ConfluenceFollowPlan {
    pub listing: SpacePageListing,
    pub snapshot: Snapshot,
}
#[derive(Default, Clone, Copy)]
pub(in crate::library) struct PageClassify<'a> {
    pub attachments: bool,
    pub homepage_only: Option<&'a str>,
    /// Manual classification consumes existing entries in listing order.
    pub take_existing: bool,
}
pub(in crate::library) enum PageProbe {
    Gone,
    Moved(String),
    Present,
    Failed(InspectionError),
}
pub(in crate::library) enum PageOrder {
    Rank,
    Position,
}

impl ConfluenceFollowPlan {
    pub(in crate::library) async fn list(
        service: &LibraryService,
        follow: &LibraryFollowSummary,
        space_key: &str,
        window: Window,
        pass: Pass<'_>,
    ) -> Result<Option<SpacePageListing>, InspectionError> {
        let cancel = AtomicBool::new(false);
        let limit = service.configuration.limits.library_space_pages.max(1);
        let future = async {
            match window {
                Window::Full => {
                    service
                        .sources
                        .list_space_pages(&follow.provider_id, space_key, limit, &cancel)
                        .await
                }
                Window::Delta { lower, upper } => {
                    service
                        .sources
                        .list_page_changes(
                            &follow.provider_id,
                            space_key,
                            lower,
                            upper,
                            limit,
                            &cancel,
                        )
                        .await
                }
            }
        };
        match pass {
            Pass::Manual { store, operation } => until_cancelled(store, operation, &cancel, future)
                .await?
                .transpose(),
            Pass::Background => future.await.map(Some),
        }
    }
    pub(in crate::library) fn classify<'p>(
        &mut self,
        follow: &LibraryFollowSummary,
        pages: &'p [SpacePage],
        options: PageClassify<'_>,
    ) -> Classified<'p, SpacePage> {
        let mut result = Classified {
            fetch: Vec::new(),
            unchanged: 0,
        };
        for page in pages {
            if self.snapshot.excluded.contains(&page.page_id) {
                continue;
            }
            let id = page_item_id(follow, &page.page_id);
            let old = self.snapshot.items.get(&id);
            let reason = match old {
                None => None,
                Some(_) if options.homepage_only == Some(page.page_id.as_str()) => {
                    result.unchanged += 1;
                    if options.take_existing {
                        self.snapshot.items.remove(&id);
                    }
                    continue;
                }
                Some(old) => match ChangeReason::of_page(old, page) {
                    Some(reason) => Some(reason),
                    None if options.attachments && ChangeReason::missing_attachments(old) => {
                        Some(ChangeReason::AttachmentsRequested)
                    }
                    None => {
                        result.unchanged += 1;
                        if options.take_existing {
                            self.snapshot.items.remove(&id);
                        }
                        continue;
                    }
                },
            };
            let old = if options.take_existing {
                self.snapshot.items.remove(&id)
            } else {
                None
            };
            result.fetch.push(Planned {
                key: page.page_id.as_str().into(),
                row: Some(page.clone()),
                old,
                reason,
            });
        }
        result
    }
    pub(in crate::library) fn absent<'a>(
        items: impl IntoIterator<Item = &'a crate::library::store::LibraryIndexEntry>,
        excluded: &BTreeSet<String>,
        follow: &LibraryFollowSummary,
        present: &BTreeSet<String>,
    ) -> Absent {
        let mut absent = Absent {
            members: 0,
            missing: BTreeSet::new(),
        };
        for entry in items {
            if !same_site_page(entry, follow)
                || !refs::has_follow(&entry.summary, &follow.follow_id)
            {
                continue;
            }
            absent.members += 1;
            if let Some(id) = entry
                .summary
                .canonical_id
                .as_ref()
                .filter(|id| !present.contains(*id) && !excluded.contains(*id))
            {
                absent.missing.insert(id.clone());
            }
        }
        absent
    }
    pub(in crate::library) async fn probe(
        service: &LibraryService,
        follow: &LibraryFollowSummary,
        page_id: &str,
        space_key: &str,
    ) -> PageProbe {
        match service
            .sources
            .page_space(&follow.provider_id, page_id)
            .await
        {
            Ok(None) => PageProbe::Gone,
            Err(failure) if failure.code == "source_not_found" => PageProbe::Gone,
            Ok(Some(key)) if key != space_key => PageProbe::Moved(key),
            Ok(Some(_)) => PageProbe::Present,
            Err(failure) => PageProbe::Failed(failure),
        }
    }
}

pub(in crate::library) fn apply_pages(
    index: &mut Index,
    follow: &LibraryFollowSummary,
    pages: &[SpacePage],
    order: PageOrder,
) -> Option<bool> {
    let excluded = &index
        .follows
        .iter()
        .find(|f| f.follow_id == follow.follow_id)?
        .excluded_ids;
    let listed = match order {
        PageOrder::Rank => {
            let mut ordered = pages.iter().collect::<Vec<_>>();
            ordered.sort_by(|a, b| {
                (a.position.is_none(), a.position, &a.title, &a.page_id).cmp(&(
                    b.position.is_none(),
                    b.position,
                    &b.title,
                    &b.page_id,
                ))
            });
            ordered
                .into_iter()
                .enumerate()
                .map(|(rank, page)| (page.page_id.as_str(), (rank as u32, page)))
                .collect::<BTreeMap<_, _>>()
        }
        PageOrder::Position => pages
            .iter()
            .map(|page| (page.page_id.as_str(), (0, page)))
            .collect(),
    };
    let ids = index
        .items
        .iter()
        .filter(|entry| same_site_page(entry, follow))
        .filter_map(|entry| {
            Some((
                entry.summary.canonical_id.clone()?,
                entry.summary.item_id.clone(),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let mut changed = false;
    for entry in &mut index.items {
        if !same_site_page(entry, follow) {
            continue;
        }
        let Some((rank, page)) = entry
            .summary
            .canonical_id
            .as_deref()
            .and_then(|id| listed.get(id))
        else {
            continue;
        };
        if excluded.contains(&page.page_id) {
            continue;
        }
        if matches!(order, PageOrder::Rank) || !refs::has_follow(&entry.summary, &follow.follow_id)
        {
            let before = entry.summary.refs.len();
            refs::insert_ref(
                &mut entry.summary,
                LibraryItemRef::Follow {
                    follow_id: follow.follow_id.clone(),
                },
            );
            changed |= entry.summary.refs.len() != before;
        }
        let parent = page.ancestors.last().and_then(|id| ids.get(id).cloned());
        if entry.summary.parent_item_id != parent {
            entry.summary.parent_item_id = parent;
            changed = true;
        }
        let position = match order {
            PageOrder::Rank => Some(*rank),
            PageOrder::Position => page.position.and_then(|p| u32::try_from(p).ok()),
        };
        if let Some(position) = position {
            if entry.summary.order != Some(position) {
                entry.summary.order = Some(position);
                changed = true;
            }
        }
    }
    if changed || matches!(order, PageOrder::Rank) {
        recount(index, &follow.follow_id);
    }
    Some(changed)
}
