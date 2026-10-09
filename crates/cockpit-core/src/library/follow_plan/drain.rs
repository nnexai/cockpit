use super::*;

/// Admission holds the item lease through fetch, publication and checkpoint.
struct DrainAdmission {
    work: Candidate,
    snapshot: Index,
    owners: BTreeSet<String>,
    old: Option<LibraryIndexEntry>,
    has_conflicts: bool,
    _item_lease: Lease,
}

impl LibraryService {
    pub(super) async fn drain_sync(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        now: i64,
        report: &mut LibrarySyncTick,
    ) -> Result<(), InspectionError> {
        let due = due_candidates(state, now);
        let started = Instant::now();
        for (_, _, _, id) in due
            .into_iter()
            .take(DRAIN_ITEMS.saturating_sub(report.fetched as usize))
        {
            if started.elapsed() >= Duration::from_secs(DRAIN_SECONDS) {
                break;
            }
            let Some(work) = state.queue.get(&id).cloned() else {
                continue;
            };
            let Some(mut admission) = self.admit_sync_candidate(store, state, &id, work, now)?
            else {
                continue;
            };
            let (operation, _operation_lease) = self.sync_operation(store, state)?;
            let result = self
                .publish_sync_candidate(store, state, &id, &operation, &admission, now)
                .await;
            self.sync_blocked(state, &admission.work.source.provider_id, now);
            self.checkpoint_sync_outcome(
                store,
                state,
                &id,
                &operation,
                &mut admission,
                result,
                now,
                report,
            )?;
        }
        Ok(())
    }

    fn admit_sync_candidate(
        &self,
        store: &Store,
        state: &mut State,
        id: &str,
        work: Candidate,
        now: i64,
    ) -> Result<Option<DrainAdmission>, InspectionError> {
        if self.sync_blocked(state, &work.source.provider_id, now) {
            return Ok(None);
        }
        let Ok(item_lease) = store.lease(id) else {
            return Ok(None);
        };
        let snapshot = index(store)?;
        let owners =
            work.owners
                .iter()
                .filter(|owner| {
                    if owner.starts_with("follow:") {
                        snapshot.follows.iter().any(|follow| {
                            &follow.follow_id == *owner
                                && !follow.excluded_ids.contains(&work.source.canonical_id)
                                && !follow.excluded_ids.iter().any(|excluded| excluded == id)
                        })
                    } else {
                        snapshot.items.iter().any(|entry| {
                            entry.summary.item_id == id && !entry.summary.refs.is_empty()
                        })
                    }
                })
                .cloned()
                .collect::<BTreeSet<_>>();
        if owners.is_empty() {
            state.queue.remove(id);
            save(store, state)?;
            return Ok(None);
        }
        let old = snapshot
            .items
            .iter()
            .find(|entry| entry.summary.item_id == id)
            .cloned();
        if revision_matches(&work, old.as_ref()) {
            self.apply_queue_owners(store, &work, &owners)?;
            state.queue.remove(id);
            save(store, state)?;
            return Ok(None);
        }
        let conflicts = match &old {
            Some(old) => {
                let _lock = store.shared()?;
                store.conflicts(old)?
            }
            None => vec![],
        };
        let already_reported = old.as_ref().is_some_and(|old| {
            old.summary.state == LibraryItemState::Conflict
                && !conflicts.is_empty()
                && old.summary.conflict.len() == conflicts.len()
                && old
                    .summary
                    .conflict
                    .iter()
                    .zip(&conflicts)
                    .all(|(old, current)| {
                        old.path == current.path && old.current_hash == current.current_hash
                    })
        });
        if already_reported {
            retry_candidate(state, id, "library_conflict".into(), now);
            save(store, state)?;
            return Ok(None);
        }
        Ok(Some(DrainAdmission {
            work,
            snapshot,
            owners,
            old,
            has_conflicts: !conflicts.is_empty(),
            _item_lease: item_lease,
        }))
    }

    async fn publish_sync_candidate(
        &self,
        store: &Arc<Store>,
        state: &State,
        id: &str,
        operation: &str,
        admission: &DrainAdmission,
        now: i64,
    ) -> Result<(), InspectionError> {
        let DrainAdmission {
            work,
            snapshot,
            owners,
            old,
            has_conflicts,
            ..
        } = admission;
        if let Some(old) = old {
            if *has_conflicts {
                self.record_conflict(
                    store,
                    operation,
                    old.clone(),
                    "Local edits were preserved during synchronization".into(),
                )?;
                return Err(error(
                    "library_conflict",
                    "Local edits need manual resolution",
                ));
            }
        }
        let request = self.queued_fetch_request(work)?;
        let fetched = {
            let future = self.sources.fetch_assets(request);
            tokio::pin!(future);
            loop {
                tokio::select! {
                    result = &mut future => break result,
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        if operations::cancelled(store, operation)? {
                            return Err(error("library_sync_yielded", "Background synchronization yielded to manual work"));
                        }
                    }
                }
            }
        }?;
        if operations::cancelled(store, operation)? {
            return Err(error(
                "library_sync_yielded",
                "Background synchronization yielded to manual work",
            ));
        }
        let asset = fetched
            .assets
            .into_iter()
            .find(|asset| item_id(&asset.source) == id)
            .ok_or_else(|| {
                error(
                    "source_identity_mismatch",
                    "Queued refresh returned a different source identity",
                )
            })?;
        let seed = ReferenceSeed {
            source: asset.source.clone(),
            label: sources::asset_label(&asset),
            references: sources::asset_references(&self.configuration, &asset),
        };
        let download = snapshot
            .follows
            .iter()
            .any(|follow| owners.contains(&follow.follow_id) && follow.include_attachments);
        // Publish only for a currently eligible owner. Store checks again under
        // the mutation lock after attachment download awaits.
        let current = index(store)?;
        let summary = super::super::asset_entry(&asset, old.as_ref()).summary;
        let reference = owners
            .iter()
            .filter(|owner| owner.starts_with("follow:"))
            .map(|owner| LibraryItemRef::Follow {
                follow_id: owner.clone(),
            })
            .find(|reference| Store::reference_allowed(&current, &summary, reference));
        let standalone = owners.iter().any(|owner| !owner.starts_with("follow:"))
            && current
                .items
                .iter()
                .any(|entry| entry.summary.item_id == id && !entry.summary.refs.is_empty());
        if reference.is_none() && !standalone {
            return Err(error(
                "library_follow_excluded",
                "Queued follow was stopped or this item was excluded",
            ));
        }
        // A listing timestamp is only a hint: fetched_updated requires agreement
        // with the actual fetched revision.
        let row = work.row.as_ref().filter(|row| {
            asset
                .source_revision
                .as_deref()
                .is_some_and(|revision| same_listing_revision(revision, &row.updated))
        });
        self.save_asset_with(
            store,
            operation,
            asset,
            old.clone(),
            SaveOptions {
                download_all: download,
                issue_row: row,
                reference,
                ..SaveOptions::default()
            },
        )
        .await?;
        if operations::cancelled(store, operation)? {
            return Err(error(
                "library_sync_yielded",
                "Background synchronization yielded to manual work",
            ));
        }
        let saved = self.entry(store, id)?.ok_or_else(|| {
            error(
                "library_item_not_found",
                "Queued snapshot was not published",
            )
        })?;
        if saved.summary.state == LibraryItemState::Unknown {
            return Err(error(
                "library_sync_partial",
                "Incomplete source content remains queued",
            ));
        }
        if matches!(
            saved.summary.state,
            LibraryItemState::Conflict
                | LibraryItemState::Failed
                | LibraryItemState::RemovedAtSource
        ) {
            return Err(error(
                "library_conflict",
                "Snapshot publication did not complete",
            ));
        }
        self.apply_queue_owners(store, work, owners)?;
        self.grow_sync_related(store, state, id, operation, admission, seed, now)
            .await
    }

    fn queued_fetch_request(
        &self,
        work: &Candidate,
    ) -> Result<sources::SourceFetchRequest, InspectionError> {
        if work.source.resource_type == "page" {
            let authority = sources::site_authority(&self.configuration, &work.source.provider_id)?;
            if authority.provider_instance != work.source.provider_instance {
                return Err(error(
                    "source_authority_mismatch",
                    "Queued page belongs to another configured site",
                ));
            }
            Ok(sources::SourceFetchRequest {
                provider_id: work.source.provider_id.clone(),
                artifact_url: sources::confluence_page_url(
                    &authority.provider_instance,
                    &work.source.canonical_id,
                ),
                authority,
            })
        } else {
            let base = self
                .configuration
                .providers
                .iter()
                .find(|p| p.id == work.source.provider_id)
                .ok_or_else(|| {
                    error(
                        "source_provider_unsupported",
                        "Queued provider is unavailable",
                    )
                })?
                .base_url
                .trim_end_matches('/');
            self.request(
                &format!("{base}/browse/{}", work.source.canonical_id),
                Some(&work.source.provider_id),
            )
        }
    }

    async fn grow_sync_related(
        &self,
        store: &Arc<Store>,
        state: &State,
        id: &str,
        operation: &str,
        admission: &DrainAdmission,
        seed: ReferenceSeed,
        now: i64,
    ) -> Result<(), InspectionError> {
        // Hourly growth traverses only freshly fetched seeds and never drops.
        for follow in admission.snapshot.follows.iter().filter(|follow| {
            admission.owners.contains(&follow.follow_id)
                && follow.reference_depth.unwrap_or(0) > 0
                && state
                    .sources
                    .get(&follow.follow_id)
                    .is_some_and(|source| source.next_related_ms > now)
        }) {
            let holder = crate::library::related::Holder::Follow(follow);
            let pass = self
                .traverse(
                    store,
                    operation,
                    &holder,
                    follow.reference_depth.unwrap_or(0),
                    crate::library::related::Seeds::one(seed.clone()),
                )
                .await?;
            self.related_row(store, operation, &pass)?;
        }
        if let Some(depth) = admission
            .old
            .as_ref()
            .and_then(|entry| entry.summary.reference_depth)
            .filter(|depth| {
                *depth > 0
                    && state
                        .sources
                        .get(&format!("item:{id}"))
                        .is_some_and(|source| source.next_related_ms > now)
            })
        {
            let holder = crate::library::related::Holder::Item { item_id: id };
            let pass = self
                .traverse(
                    store,
                    operation,
                    &holder,
                    depth,
                    crate::library::related::Seeds::one(seed),
                )
                .await?;
            self.related_row(store, operation, &pass)?;
        }
        Ok(())
    }

    fn checkpoint_sync_outcome(
        &self,
        store: &Store,
        state: &mut State,
        id: &str,
        operation: &str,
        admission: &mut DrainAdmission,
        result: Result<(), InspectionError>,
        now: i64,
        report: &mut LibrarySyncTick,
    ) -> Result<(), InspectionError> {
        match result {
            Ok(()) => {
                report.fetched += 1;
                state.queue.remove(id);
                operations::finish(store, operation, Ok(()))?;
            }
            Err(e) => {
                if admission.work.confirm_missing && e.code == "source_not_found" {
                    if let Some(old) = admission.old.take() {
                        self.fetch_failed(store, operation, old, e, true)?;
                        state.queue.remove(id);
                    }
                    operations::finish(store, operation, Ok(()))?;
                } else {
                    retry_candidate(state, id, e.code.clone(), now);
                    if e.code == "library_sync_yielded" {
                        operations::finish(store, operation, Ok(()))?;
                    } else {
                        operations::finish(store, operation, Err(e))?;
                    }
                }
            }
        }
        state.active_operation = None;
        save(store, state)
    }
}

fn due_candidates(state: &State, now: i64) -> Vec<(i64, i64, usize, String)> {
    let mut due = state
        .queue
        .iter()
        .filter(|(_, work)| work.next_attempt_ms <= now)
        .map(|(id, work)| {
            (
                work.next_attempt_ms,
                work.enqueued_ms,
                work.page.as_ref().map_or(0, |page| page.ancestors.len()),
                id.clone(),
            )
        })
        .collect::<Vec<_>>();
    due.sort();
    due
}

fn revision_matches(work: &Candidate, old: Option<&LibraryIndexEntry>) -> bool {
    old.is_some_and(|entry| {
        !work.force
            && !work.confirm_missing
            && work.revision.is_some()
            && !matches!(
                entry.summary.state,
                LibraryItemState::Failed
                    | LibraryItemState::Unknown
                    | LibraryItemState::RemovedAtSource
                    | LibraryItemState::Conflict
            )
            && if work.source.resource_type == "issue" {
                entry
                    .summary
                    .issue
                    .as_ref()
                    .and_then(|meta| meta.fetched_updated.as_ref())
                    == work.revision.as_ref()
            } else {
                entry.summary.source_revision == work.revision
            }
    })
}

fn retry_candidate(state: &mut State, id: &str, code: String, now: i64) {
    let failure = state.queue.get_mut(id).unwrap();
    failure.failures = failure.failures.saturating_add(1);
    failure.last_error = Some(code);
    failure.next_attempt_ms = now.saturating_add(backoff(failure.failures, MINUTE, 24 * HOUR));
}
