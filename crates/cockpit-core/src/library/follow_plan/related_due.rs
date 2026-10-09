//! Scheduled holder traversal, with delayed absence authority kept in sync.
use super::*;
use crate::library::related::{
    Holder, RelatedPass, Seeds, stale_state, strip_inclusions, unreached,
};

impl LibraryService {
    pub(super) async fn sync_related_due(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        follows: &[LibraryFollowSummary],
        config: &LibrarySyncConfiguration,
        now: i64,
    ) -> Result<(), InspectionError> {
        for follow in follows
            .iter()
            .filter(|f| f.reference_depth.unwrap_or(0) > 0)
        {
            if state
                .sources
                .get(&follow.follow_id)
                .is_some_and(|s| s.next_related_ms > now)
                || self.sync_blocked(state, &follow.provider_id, now)
            {
                continue;
            }
            self.sync_follow_related(store, state, follow, config, now)
                .await?;
        }
        Ok(())
    }

    async fn sync_follow_related(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        follow: &LibraryFollowSummary,
        config: &LibrarySyncConfiguration,
        now: i64,
    ) -> Result<(), InspectionError> {
        let Ok(lease) = store.lease(&follow.follow_id) else {
            return Ok(());
        };
        state
            .sources
            .entry(follow.follow_id.clone())
            .or_default()
            .next_related_ms = schedule(
            now,
            i64::from(config.related_hours) * HOUR,
            &follow.follow_id,
        );
        save(store, state)?;
        let snapshot = index(store)?;
        let holder = Holder::Follow(follow);
        let member_count = snapshot
            .items
            .iter()
            .filter(|entry| refs::has_follow(&entry.summary, &follow.follow_id))
            .count();
        let mut seeds = Seeds::default();
        for entry in snapshot.items.iter().filter(|entry| {
            refs::has_follow(&entry.summary, &follow.follow_id) && !holder.holds(&entry.summary)
        }) {
            let before = seeds.seeds.len();
            seeds.push_stored(entry, entry.summary.title.clone(), false);
            // Background counted stale only when stored identity and refs were known.
            if seeds.seeds.len() > before && stale_state(entry.summary.state) {
                seeds.unknown += 1;
            }
        }
        // Empty outgoing sets reconcile without creating an operation.
        if !seeds.has_outgoing() {
            let confirmed = if seeds.unknown == 0 {
                let missing = unreached(snapshot.items.iter(), &holder, &RelatedPass::none());
                let previous = &state.sources[&follow.follow_id].related_absent;
                let (confirmed, next) = absence_step(previous, &missing, member_count, now);
                state
                    .sources
                    .get_mut(&follow.follow_id)
                    .unwrap()
                    .related_absent = next;
                confirmed
            } else {
                let source = state.sources.get_mut(&follow.follow_id).unwrap();
                source.next_related_ms = now.saturating_add(5 * MINUTE);
                source.last_error = Some("library_sync_related_partial".into());
                BTreeSet::new()
            };
            drop(lease);
            if seeds.unknown == 0 {
                self.drop_sync_members(store, state, follow, &confirmed, now)?;
            }
            return save(store, state);
        }
        let (operation, _op_lease) = self.sync_operation(store, state)?;
        let result = self
            .traverse(
                store,
                &operation,
                &holder,
                follow.reference_depth.unwrap_or(0),
                seeds,
            )
            .await;
        self.sync_blocked(state, &follow.provider_id, now);
        match result {
            Ok(pass) => {
                self.related_row(store, &operation, &pass)?;
                operations::finish(store, &operation, Ok(()))?;
                state.active_operation = None;
                if pass.complete && !pass.cancelled {
                    let missing = unreached(snapshot.items.iter(), &holder, &pass);
                    let previous = &state.sources[&follow.follow_id].related_absent;
                    let (confirmed, next) = absence_step(previous, &missing, member_count, now);
                    state
                        .sources
                        .get_mut(&follow.follow_id)
                        .unwrap()
                        .related_absent = next;
                    drop(lease);
                    self.drop_sync_members(store, state, follow, &confirmed, now)?;
                }
                if !pass.complete {
                    let source = state.sources.get_mut(&follow.follow_id).unwrap();
                    source.next_related_ms = now.saturating_add(5 * MINUTE);
                    source.last_error = Some("library_sync_related_partial".into());
                }
            }
            Err(failure) => {
                operations::finish(store, &operation, Err(failure))?;
                state.active_operation = None;
                state
                    .sources
                    .get_mut(&follow.follow_id)
                    .unwrap()
                    .next_related_ms = now.saturating_add(5 * MINUTE);
            }
        }
        save(store, state)
    }

    pub(super) async fn sync_single_related_due(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        config: &LibrarySyncConfiguration,
        now: i64,
    ) -> Result<(), InspectionError> {
        let snapshot = index(store)?;
        for entry in snapshot.items.iter().filter(|entry| {
            !entry.summary.refs.is_empty() && entry.summary.reference_depth.unwrap_or(0) > 0
        }) {
            let id = &entry.summary.item_id;
            let key = format!("item:{id}");
            let (Some(provider), Some(instance), Some(kind), Some(canonical), Some(references)) = (
                &entry.summary.provider_id,
                &entry.summary.provider_instance,
                &entry.summary.resource_type,
                &entry.summary.canonical_id,
                &entry.references,
            ) else {
                continue;
            };
            if !self.configuration.providers.iter().any(|p| {
                &p.id == provider && matches!(p.kind, ProviderKind::Jira | ProviderKind::Confluence)
            }) {
                continue;
            }
            let previous = state.sources.entry(key.clone()).or_default().clone();
            if previous.next_related_ms > now
                || self.sync_blocked(state, provider, now)
                || stale_state(entry.summary.state)
            {
                continue;
            }
            let Ok(_item_lease) = store.lease(id) else {
                continue;
            };
            state.sources.get_mut(&key).unwrap().next_related_ms =
                schedule(now, i64::from(config.related_hours) * HOUR, &key);
            save(store, state)?;
            let holder = Holder::Item { item_id: id };
            let pass = if references.is_empty() {
                RelatedPass::none()
            } else {
                let seed = ReferenceSeed {
                    source: SourceRef {
                        provider_id: provider.clone(),
                        provider_instance: instance.clone(),
                        resource_type: kind.clone(),
                        canonical_id: canonical.clone(),
                    },
                    label: entry.summary.title.clone(),
                    references: references.clone(),
                };
                let (operation, _operation_lease) = self.sync_operation(store, state)?;
                let result = self
                    .traverse(
                        store,
                        &operation,
                        &holder,
                        entry.summary.reference_depth.unwrap_or(0),
                        Seeds::one(seed),
                    )
                    .await;
                self.sync_blocked(state, provider, now);
                match result {
                    Ok(pass) => {
                        self.related_row(store, &operation, &pass)?;
                        operations::finish(store, &operation, Ok(()))?;
                        state.active_operation = None;
                        pass
                    }
                    Err(failure) => {
                        operations::finish(store, &operation, Err(failure))?;
                        state.active_operation = None;
                        state.sources.get_mut(&key).unwrap().next_related_ms =
                            now.saturating_add(5 * MINUTE);
                        save(store, state)?;
                        continue;
                    }
                }
            };
            if pass.complete && !pass.cancelled {
                let members = snapshot
                    .items
                    .iter()
                    .filter(|item| holder.holds(&item.summary))
                    .count();
                let missing = unreached(snapshot.items.iter(), &holder, &pass);
                let (confirmed, next) =
                    absence_step(&previous.related_absent, &missing, members, now);
                state.sources.get_mut(&key).unwrap().related_absent = next;
                if strip_inclusions(store, &holder.inclusion(), &confirmed)? {
                    let (operation, _operation_lease) = self.sync_operation(store, state)?;
                    let result = operations::row(
                        store,
                        &operation,
                        Some(&entry.summary),
                        LibraryReportOutcome::Updated,
                        Some("Related inclusions reconciled after two complete passes".into()),
                    );
                    operations::finish(store, &operation, result)?;
                    state.active_operation = None;
                }
            }
            if !pass.complete {
                let source = state.sources.get_mut(&key).unwrap();
                source.next_related_ms = now.saturating_add(5 * MINUTE);
                source.last_error = Some("library_sync_related_partial".into());
            }
            save(store, state)?;
        }
        Ok(())
    }
}
