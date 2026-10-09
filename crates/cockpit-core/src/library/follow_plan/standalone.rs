use super::*;

impl LibraryService {
    pub(super) async fn discover_standalone(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        config: &LibrarySyncConfiguration,
        now: i64,
        report: &mut LibrarySyncTick,
    ) -> Result<(), InspectionError> {
        let groups = self.standalone_groups(store, state)?;
        for (group, entries) in groups {
            if state
                .sources
                .entry(group.clone())
                .or_default()
                .next_inventory_ms
                > now
            {
                continue;
            }
            let provider = entries[0]
                .summary
                .provider_id
                .as_deref()
                .unwrap_or_default();
            if self.sync_blocked(state, provider, now) {
                continue;
            }
            let is_issue = entries[0].summary.resource_type.as_deref() == Some("issue");
            freeze_inventory(state, &group, &entries, is_issue, config, now);
            save(store, state)?; // durable cohort before the first remote request
            let mut attempted_size = 100;
            let result = self
                .execute_standalone_inventory(
                    store,
                    state,
                    config,
                    &group,
                    &entries,
                    provider,
                    is_issue,
                    now,
                    report,
                    &mut attempted_size,
                )
                .await;
            self.sync_blocked(state, provider, now);
            match result {
                Ok(true) => {
                    reconcile_inventory(store, state, config, &group, &entries, provider, now)?
                }
                Ok(false) => {
                    state.sources.get_mut(&group).unwrap().last_error =
                        Some("library_sync_budget_deferred".into());
                }
                Err(e) => {
                    let source = state.sources.get_mut(&group).unwrap();
                    // An expensive chunk can exceed the entire budget; shrink
                    // its durable retry size, not just this attempt's remainder.
                    if e.code == "source_rate_limited" {
                        source.inventory.as_mut().unwrap().batch_size = (attempted_size / 2).max(1);
                    }
                    source.failures = source.failures.saturating_add(1);
                    source.last_error = Some(e.code);
                    source.next_inventory_ms =
                        now.saturating_add(backoff(source.failures, 5 * MINUTE, HOUR));
                }
            }
            save(store, state)?;
        }
        Ok(())
    }

    fn standalone_groups(
        &self,
        store: &Store,
        state: &State,
    ) -> Result<Vec<(String, Vec<LibraryIndexEntry>)>, InspectionError> {
        let snapshot = index(store)?;
        let mut groups: BTreeMap<String, Vec<LibraryIndexEntry>> = BTreeMap::new();
        for entry in snapshot.items {
            if entry.summary.refs.is_empty() {
                continue;
            }
            let Some(provider) = self.configuration.providers.iter().find(|p| {
                Some(&p.id) == entry.summary.provider_id.as_ref()
                    && matches!(p.kind, ProviderKind::Jira | ProviderKind::Confluence)
            }) else {
                continue;
            };
            // Related-only items need standalone checks even when a follow owns them.
            let seed = snapshot.follows.iter().any(|follow| {
                refs::has_follow(&entry.summary, &follow.follow_id)
                    && !refs::related_of(&entry.summary, &follow.follow_id)
                    && follow.provider_id == provider.id
                    && Some(&follow.provider_instance) == entry.summary.provider_instance.as_ref()
            });
            if seed {
                continue;
            }
            groups
                .entry(format!(
                    "standalone:{}:{}",
                    provider.id,
                    entry
                        .summary
                        .provider_instance
                        .as_deref()
                        .unwrap_or_default(),
                ))
                .or_default()
                .push(entry);
        }
        let mut groups = groups.into_iter().collect::<Vec<_>>();
        groups.sort_by_key(|(group, _)| {
            state
                .sources
                .get(group)
                .map_or(0, |source| source.next_inventory_ms)
        });
        Ok(groups)
    }

    async fn execute_standalone_inventory(
        &self,
        store: &Store,
        state: &mut State,
        config: &LibrarySyncConfiguration,
        group: &str,
        entries: &[LibraryIndexEntry],
        provider: &str,
        is_issue: bool,
        now: i64,
        report: &mut LibrarySyncTick,
        attempted_size: &mut usize,
    ) -> Result<bool, InspectionError> {
        let cancel = AtomicBool::new(false);
        // Yield among due groups and leave publication a turn even for providers
        // without request-accounting support.
        for _ in 0..10 {
            let progress = state.sources[group].inventory.as_ref().unwrap();
            if progress.cursor == progress.ids.len() {
                return Ok(true);
            }
            let remaining = self.sources.background_requests_remaining(provider);
            // This estimate leaves publication headroom; HTTP still charges each hop.
            let reserve = (config.hourly_request_cap / 4).min(32);
            let affordable = remaining.map_or(100, |remaining| {
                remaining.saturating_sub(reserve) / progress.requests_per_id
            });
            let size = progress.batch_size.min(affordable as usize);
            if size == 0 {
                return Ok(false);
            }
            *attempted_size = size;
            let end = (progress.cursor + size).min(progress.ids.len());
            let ids = progress.ids[progress.cursor..end].to_vec();
            let (observed, candidates) = self
                .list_standalone_chunk(group, entries, provider, is_issue, &ids, now, &cancel)
                .await?;
            // Never advance a cursor without all the chunk's candidates.
            for work in candidates {
                enqueue(state, work)?;
                report.discovered += 1;
            }
            let after = self.sources.background_requests_remaining(provider);
            let progress = state
                .sources
                .get_mut(group)
                .unwrap()
                .inventory
                .as_mut()
                .unwrap();
            if let Some((before, after)) = remaining.zip(after) {
                let cost = before
                    .saturating_sub(after)
                    .div_ceil(ids.len() as u32)
                    .max(1);
                progress.requests_per_id = progress.requests_per_id.max(cost);
            }
            progress.present.extend(observed);
            progress.cursor = end;
            save(store, state)?;
        }
        let progress = state.sources[group].inventory.as_ref().unwrap();
        Ok(progress.cursor == progress.ids.len())
    }

    async fn list_standalone_chunk(
        &self,
        group: &str,
        entries: &[LibraryIndexEntry],
        provider: &str,
        is_issue: bool,
        ids: &[String],
        now: i64,
        cancel: &AtomicBool,
    ) -> Result<(BTreeSet<String>, Vec<Candidate>), InspectionError> {
        let mut observed = BTreeSet::new();
        let mut candidates = Vec::new();
        if is_issue {
            let listing = self
                .sources
                .list_issues(provider, &IssueQuery::Keys(ids), ids.len() as u32, cancel)
                .await?;
            if !listing.complete {
                return Err(error(
                    "library_sync_partial",
                    "Standalone issue check was incomplete",
                ));
            }
            for row in listing.rows {
                if !ids.contains(&row.key) {
                    return Err(error(
                        "source_provider_contract",
                        "Standalone issue check returned an unsolicited identity",
                    ));
                }
                observed.insert(row.key.clone());
                let Some(old) = entries
                    .iter()
                    .find(|entry| entry.summary.canonical_id.as_ref() == Some(&row.key))
                else {
                    continue;
                };
                if follow::plan::ChangeReason::of_issue(old, &row).is_some() {
                    let source = SourceRef {
                        provider_id: provider.into(),
                        provider_instance: old
                            .summary
                            .provider_instance
                            .clone()
                            .unwrap_or_default(),
                        resource_type: "issue".into(),
                        canonical_id: row.key.clone(),
                    };
                    let mut work = candidate(source, group, Some(row.updated.clone()), now);
                    work.row = Some(row);
                    candidates.push(work);
                }
            }
        } else {
            let listing = self.sources.page_versions(provider, ids, cancel).await?;
            if !listing.complete {
                return Err(error(
                    "library_sync_partial",
                    "Standalone page check was incomplete",
                ));
            }
            for page in listing.pages {
                observed.insert(page.page_id.clone());
                let Some(old) = entries
                    .iter()
                    .find(|entry| entry.summary.canonical_id.as_ref() == Some(&page.page_id))
                else {
                    continue;
                };
                if follow::plan::ChangeReason::of_page(old, &page).is_some() {
                    let source = SourceRef {
                        provider_id: provider.into(),
                        provider_instance: old
                            .summary
                            .provider_instance
                            .clone()
                            .unwrap_or_default(),
                        resource_type: "page".into(),
                        canonical_id: page.page_id.clone(),
                    };
                    let mut work = candidate(source, group, Some(page.version.to_string()), now);
                    work.force = true;
                    work.page = Some(page);
                    candidates.push(work);
                }
            }
        }
        Ok((observed, candidates))
    }
}

fn freeze_inventory(
    state: &mut State,
    group: &str,
    entries: &[LibraryIndexEntry],
    is_issue: bool,
    config: &LibrarySyncConfiguration,
    now: i64,
) {
    let source = state.sources.get_mut(group).unwrap();
    if source.inventory.is_none() {
        let ids = entries
            .iter()
            .filter_map(|entry| entry.summary.canonical_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        source.inventory = Some(StandaloneInventory {
            ids,
            cursor: 0,
            present: BTreeSet::new(),
            batch_size: 100,
            requests_per_id: if is_issue {
                1
            } else {
                2.min(config.hourly_request_cap)
            },
        });
    }
    source.next_inventory_ms = now.saturating_add(MINUTE);
}

fn reconcile_inventory(
    store: &Store,
    state: &mut State,
    config: &LibrarySyncConfiguration,
    group: &str,
    entries: &[LibraryIndexEntry],
    provider: &str,
    now: i64,
) -> Result<(), InspectionError> {
    let progress = state.sources[group].inventory.as_ref().unwrap();
    let cohort = progress.ids.iter().cloned().collect::<BTreeSet<_>>();
    // Re-read membership: additions outside the frozen cohort have not been
    // checked, and removals must not be revived.
    let current = index(store)?
        .items
        .into_iter()
        .filter(|entry| {
            !entry.summary.refs.is_empty()
                && entry.summary.provider_id.as_deref() == Some(provider)
                && entry.summary.provider_instance == entries[0].summary.provider_instance
                && entry
                    .summary
                    .canonical_id
                    .as_ref()
                    .is_some_and(|id| cohort.contains(id))
        })
        .collect::<Vec<_>>();
    let missing = current
        .iter()
        .filter_map(|entry| entry.summary.canonical_id.clone())
        .filter(|id| !progress.present.contains(id))
        .collect::<BTreeSet<_>>();
    let previous_absent = state.sources[group].absent.clone();
    if allowed_mass(missing.len(), current.len()) {
        for entry in &current {
            let Some(id) = entry.summary.canonical_id.as_ref() else {
                continue;
            };
            if missing.contains(id)
                && confirmed(&previous_absent, id, now)
                && entry.summary.state != LibraryItemState::RemovedAtSource
            {
                let source = SourceRef {
                    provider_id: provider.into(),
                    provider_instance: entry.summary.provider_instance.clone().unwrap_or_default(),
                    resource_type: entry.summary.resource_type.clone().unwrap_or_default(),
                    canonical_id: id.clone(),
                };
                let mut work = candidate(source, group, None, now);
                work.force = true;
                work.confirm_missing = true;
                enqueue(state, work)?;
            }
        }
    }
    let source = state.sources.get_mut(group).unwrap();
    source.absent = absence(&previous_absent, &missing, now);
    source.inventory = None;
    source.failures = 0;
    source.last_error = None;
    source.last_success_ms = Some(now);
    source.next_inventory_ms = schedule(now, i64::from(config.inventory_hours) * HOUR, group);
    Ok(())
}
