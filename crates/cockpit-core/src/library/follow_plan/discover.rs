use super::*;
use crate::library::follow::plan::{
    self, ConfluenceFollowPlan, JiraFollowPlan, MemberScope, PageClassify, PageOrder, PageProbe,
    Pass, Snapshot, Window,
};

impl LibraryService {
    pub(super) async fn discover_follow(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        follow: &LibraryFollowSummary,
        window: Option<(i64, i64)>,
        now: i64,
    ) -> Result<(u32, bool), InspectionError> {
        let site = sources::site_authority(&self.configuration, &follow.provider_id)?;
        if site.provider_instance != follow.provider_instance {
            return Err(error(
                "source_authority_mismatch",
                "Follow belongs to another configured site",
            ));
        }
        let window = Window::from_delta(window);
        match &follow.source {
            LibraryFollowSource::ConfluenceSpace { space_key, .. } => {
                self.discover_pages(store, state, follow, space_key, window, now)
                    .await
            }
            LibraryFollowSource::JiraQuery { jql, mode } => {
                self.discover_issues(store, state, follow, jql, *mode, window, now)
                    .await
            }
        }
    }

    async fn discover_pages(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        follow: &LibraryFollowSummary,
        space_key: &str,
        window: Window,
        now: i64,
    ) -> Result<(u32, bool), InspectionError> {
        let listing = ConfluenceFollowPlan::list(self, follow, space_key, window, Pass::Background)
            .await?
            .expect("background listing is not cancellable");
        let mut pipeline = ConfluenceFollowPlan {
            listing,
            snapshot: Snapshot::read(store, &follow.follow_id)?,
        };
        if pipeline.snapshot.record.is_none() {
            return Ok((0, false));
        }
        // Keep the listed pages borrowed without copying the provider result.
        let pages = std::mem::take(&mut pipeline.listing.pages);
        let classified = pipeline.classify(follow, &pages, PageClassify::default());
        let mut count = 0;
        for planned in classified.fetch {
            let page = planned.row.expect("listed page");
            let revision = page.version.to_string();
            let force = pipeline
                .snapshot
                .items
                .get(&follow::page_item_id(follow, &planned.key))
                .is_some_and(|old| {
                    old.summary.source_revision.as_deref() == Some(revision.as_str())
                });
            let mut work = candidate(
                source(follow, &planned.key, "page"),
                &follow.follow_id,
                Some(revision),
                now,
            );
            work.force = force;
            work.page = Some(page);
            enqueue(state, work)?;
            count += 1;
        }
        self.adopt_pages(store, follow, &pages)?;
        if window == Window::Full && pipeline.listing.complete {
            let present = pages
                .iter()
                .map(|page| page.page_id.clone())
                .chain(pipeline.listing.homepage_id.clone())
                .collect::<BTreeSet<_>>();
            let absent = ConfluenceFollowPlan::absent(
                pipeline.snapshot.items.values(),
                &pipeline.snapshot.excluded,
                follow,
                &present,
            );
            let previous = &state.sources[&follow.follow_id].absent;
            let (confirmed, next) = absence_step(previous, &absent.missing, absent.members, now);
            self.sync_hold(store, state, follow, absent.missing.len(), absent.members)?;
            if !present.is_empty() {
                for id in confirmed {
                    match ConfluenceFollowPlan::probe(self, follow, &id, space_key).await {
                        PageProbe::Gone => {
                            self.mark_sync_missing(
                                store,
                                state,
                                follow,
                                &id,
                                "source_not_found",
                                "Not found or no longer visible at source",
                            )
                            .await?
                        }
                        PageProbe::Moved(_) => {
                            self.drop_sync_members(
                                store,
                                state,
                                follow,
                                &BTreeSet::from([id.clone()]),
                                now,
                            )?;
                            let mut work =
                                candidate(source(follow, &id, "page"), "tracked", None, now);
                            work.force = true;
                            enqueue(state, work)?;
                        }
                        PageProbe::Present => {}
                        PageProbe::Failed(failure) => return Err(failure),
                    }
                }
            }
            state.sources.get_mut(&follow.follow_id).unwrap().absent = next;
        }
        Ok((count, pipeline.listing.complete))
    }

    async fn discover_issues(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        follow: &LibraryFollowSummary,
        jql: &str,
        mode: LibraryFollowMode,
        window: Window,
        now: i64,
    ) -> Result<(u32, bool), InspectionError> {
        let listing = JiraFollowPlan::list(self, follow, jql, window, Pass::Background)
            .await?
            .expect("background listing is not cancellable");
        let pipeline = JiraFollowPlan {
            listing,
            snapshot: Snapshot::read(store, &follow.follow_id)?,
        };
        if pipeline.snapshot.record.is_none() {
            return Ok((0, false));
        }
        let listed = pipeline
            .listing
            .rows
            .iter()
            .map(|row| row.key.clone())
            .collect::<BTreeSet<_>>();
        let members = pipeline.members(follow, MemberScope::SiteIssues);
        let mut checked = Vec::new();
        let mut complete = pipeline.listing.complete;
        if window == Window::Full && mode == LibraryFollowMode::Accumulate {
            let keys = pipeline
                .absent(&members, &listed)
                .missing
                .into_iter()
                .collect::<Vec<_>>();
            let batch = pipeline
                .check_members(self, follow, &keys, Pass::Background)
                .await?
                .expect("background check is not cancellable");
            complete &= batch.complete;
            checked = batch.rows;
        }
        let classified = pipeline.classify(
            follow,
            pipeline.listing.rows.iter().chain(checked.iter()),
            false,
            false,
        );
        let mut count = 0;
        for planned in classified.fetch {
            let row = planned.row.expect("listed issue");
            let owner = if listed.contains(planned.key.as_ref()) {
                &follow.follow_id
            } else {
                "tracked"
            };
            let mut work = candidate(
                source(follow, &planned.key, "issue"),
                owner,
                Some(row.updated.clone()),
                now,
            );
            work.row = Some(row);
            enqueue(state, work)?;
            count += 1;
        }
        self.adopt_issues(store, follow, &pipeline.listing.rows)?;
        if window == Window::Full && complete {
            let present = pipeline
                .listing
                .rows
                .iter()
                .chain(checked.iter())
                .map(|row| row.key.clone())
                .collect();
            let absent = pipeline.absent(&members, &present);
            let (confirmed, next) = absence_step(
                &state.sources[&follow.follow_id].absent,
                &absent.missing,
                absent.members,
                now,
            );
            self.sync_hold(store, state, follow, absent.missing.len(), absent.members)?;
            if !listed.is_empty() {
                if mode == LibraryFollowMode::Live {
                    self.drop_sync_members(store, state, follow, &confirmed, now)?;
                } else {
                    for id in confirmed {
                        let mut work =
                            candidate(source(follow, &id, "issue"), "tracked", None, now);
                        work.force = true;
                        work.confirm_missing = true;
                        enqueue(state, work)?;
                    }
                }
            }
            state.sources.get_mut(&follow.follow_id).unwrap().absent = next;
        }
        Ok((count, complete))
    }

    pub(super) fn adopt_pages(
        &self,
        store: &Store,
        follow: &LibraryFollowSummary,
        pages: &[SpacePage],
    ) -> Result<(), InspectionError> {
        let changed = store.mutate_index_if(|index| {
            let changed =
                plan::apply_pages(index, follow, pages, PageOrder::Position).unwrap_or(false);
            Ok((changed, changed))
        })?;
        self.sync_metadata_report(store, follow, changed)
    }

    pub(super) fn adopt_issues(
        &self,
        store: &Store,
        follow: &LibraryFollowSummary,
        rows: &[IssueRow],
    ) -> Result<(), InspectionError> {
        let listed = rows
            .iter()
            .map(|row| (row.key.as_str(), row))
            .collect::<BTreeMap<_, _>>();
        let changed = store.mutate_index_if(|index| {
            let changed =
                plan::apply_issues(index, follow, &listed, &listed, false).unwrap_or(false);
            Ok((changed, changed))
        })?;
        self.sync_metadata_report(store, follow, changed)
    }

    pub(super) fn sync_hold(
        &self,
        store: &Store,
        state: &mut State,
        follow: &LibraryFollowSummary,
        missing: usize,
        members: usize,
    ) -> Result<(), InspectionError> {
        let held = (!allowed_mass(missing, members)).then(|| {
            format!(
                "{missing} of {members} members absent; automatic removal held for manual Refresh"
            )
        });
        let previous = &mut state.sources.get_mut(&follow.follow_id).unwrap().held;
        if *previous == held {
            return Ok(());
        }
        *previous = held.clone();
        if let Some(message) = held {
            let (operation, _lease) = operations::create_background(store)?;
            let result = operations::follow_row(
                store,
                &operation.operation_id,
                follow,
                LibraryReportOutcome::Partial,
                Some(message),
            );
            operations::finish(store, &operation.operation_id, result)?;
        }
        Ok(())
    }

    pub(super) fn drop_sync_members(
        &self,
        store: &Store,
        state: &mut State,
        follow: &LibraryFollowSummary,
        keys: &BTreeSet<String>,
        now: i64,
    ) -> Result<(), InspectionError> {
        if keys.is_empty() {
            return Ok(());
        }
        let Ok(_follow_lease) = store.lease(&follow.follow_id) else {
            return Ok(());
        };
        let (operation, _lease) = self.sync_operation(store, state)?;
        let result = store.mutate_index_if(|index| {
            let Some(record) = index
                .follows
                .iter()
                .find(|f| f.follow_id == follow.follow_id)
            else {
                return Ok((vec![], false));
            };
            let excluded = record.excluded_ids.clone();
            let mut dropped = vec![];
            for entry in &mut index.items {
                let matches = keys.contains(&entry.summary.item_id)
                    || entry.summary.canonical_id.as_ref().is_some_and(|id| {
                        keys.contains(id)
                            && entry.summary.provider_id.as_deref() == Some(&follow.provider_id)
                            && entry.summary.provider_instance.as_deref()
                                == Some(&follow.provider_instance)
                    });
                if !matches
                    || !refs::has_follow(&entry.summary, &follow.follow_id)
                    || excluded.contains(&entry.summary.item_id)
                    || entry
                        .summary
                        .canonical_id
                        .as_ref()
                        .is_some_and(|id| excluded.contains(id))
                {
                    continue;
                }
                dropped.push(entry.summary.clone());
                refs::release_follow(&mut entry.summary, &follow.follow_id, now as u128);
            }
            let changed = !dropped.is_empty();
            if changed {
                follow::recount(index, &follow.follow_id);
            }
            Ok((dropped, changed))
        });
        let outcome = match result {
            Ok(dropped) => {
                for summary in dropped {
                    operations::row(
                        store,
                        &operation,
                        Some(&summary),
                        LibraryReportOutcome::Dropped,
                        Some("No longer a member after two complete inventories".into()),
                    )?;
                }
                Ok(())
            }
            Err(e) => Err(e),
        };
        operations::finish(store, &operation, outcome)?;
        state.active_operation = None;
        save(store, state)
    }

    pub(super) async fn mark_sync_missing(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        follow: &LibraryFollowSummary,
        id: &str,
        code: &str,
        reason: &str,
    ) -> Result<(), InspectionError> {
        let item = follow::page_item_id(follow, id);
        let Ok(_lease) = store.lease(&item) else {
            return Ok(());
        };
        let Some(mut entry) = self.entry(store, &item)? else {
            return Ok(());
        };
        if entry.summary.state == LibraryItemState::RemovedAtSource {
            return Ok(());
        }
        let (operation, _op_lease) = self.sync_operation(store, state)?;
        entry.summary.state = LibraryItemState::RemovedAtSource;
        entry.summary.diagnostics = vec![ProjectDiagnostic {
            code: code.into(),
            message: reason.into(),
            path: None,
        }];
        let result = store.update(entry.clone()).and_then(|()| {
            operations::row(
                store,
                &operation,
                Some(&entry.summary),
                LibraryReportOutcome::RemovedAtSource,
                Some(reason.into()),
            )
        });
        operations::finish(store, &operation, result)?;
        state.active_operation = None;
        save(store, state)
    }
}
