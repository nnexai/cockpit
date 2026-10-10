use super::*;

impl LibraryService {
    pub(super) async fn run_snapshot_add(
        &self,
        store: &Arc<Store>,
        operation: &str,
        fetch: SourceFetchRequest,
        primary_id: &str,
        request: LibraryAddRequest,
    ) -> Result<(), InspectionError> {
        if operations::cancelled(store, operation)? {
            return Ok(());
        }
        let fetched = self.sources.fetch_assets(fetch).await?;
        // Re-adding an existing item keeps its stored depth unless the add
        // refreshes it, so `Keep in Library` never resets the policy.
        let apply_depth = request.refresh_existing
            || self.entry(store, primary_id)?.is_none();
        let mut seed = None;
        if request.reference_depth > 0 {
            // An add starts with an unknown total; the seed is its first unit.
            operations::add_total(store, operation, 1)?;
        }
        for mut asset in fetched.assets {
            if operations::cancelled(store, operation)? {
                break;
            }
            let asset_id = item_id(&asset.source);
            let _linked_lease = if asset_id != primary_id {
                Some(store.lease(&asset_id)?)
            } else {
                None
            };
            if asset_id == primary_id {
                asset.original_url = Some(request.input.clone());
                if request.reference_depth > 0 {
                    seed = Some(ReferenceSeed {
                        source: asset.source.clone(),
                        label: asset_label(&asset),
                        references: asset_references(&self.configuration, &asset),
                    });
                }
            }
            let old = self.entry(store, &asset_id)?;
            if old.is_some() && !request.refresh_existing && !request.download_attachments {
                let saved = old.as_ref().expect("existing item");
                if !saved.summary.refs.contains(&LibraryItemRef::Manual) {
                    store.add_ref(&asset_id, LibraryItemRef::Manual)?;
                }
                operations::row(
                    store,
                    operation,
                    old.as_ref().map(|e| &e.summary),
                    LibraryReportOutcome::Unchanged,
                    Some("Already saved in Library".into()),
                )?;
                continue;
            }
            self.save_asset_with(store, operation, asset, old, SaveOptions {
                reference: Some(LibraryItemRef::Manual),
                download_all: request.download_attachments && asset_id == primary_id,
                ..SaveOptions::default()
            }).await?;
        }
        if apply_depth && !operations::cancelled(store, operation)? {
            self
                .apply_reference_depth(
                    store,
                    operation,
                    primary_id,
                    seed,
                    request.reference_depth,
                )
                .await?;
        }
        if let Some(target) = &request.target {
            let saved = operations::get(store, operation)?.item_ids;
            if saved.is_empty() && operations::cancelled(store, operation)? {
                return Ok(());
            }
            self.select_saved_items(store, operation, target, &saved).await?;
        }
        Ok(())
    }

    pub(super) fn prepare_snapshot_entry(
        &self,
        asset: &SourceAsset,
        old: Option<&LibraryIndexEntry>,
        reference: Option<&LibraryItemRef>,
        issue_row: Option<&IssueRow>,
    ) -> Result<(LibraryIndexEntry, bool), InspectionError> {
        let canonical_url = asset.source_url.as_deref().ok_or_else(|| {
            error(
                "source_identity_mismatch",
                "Provider asset has no validated canonical URL",
            )
        })?;
        let is_confluence = self.configuration.providers.iter().any(|provider| {
            provider.id == asset.source.provider_id
                && provider.kind == ProviderKind::Confluence
        });
        if is_confluence && asset.source.resource_type != "page" {
            return Err(error(
                "source_identity_mismatch",
                "Confluence provider returned a non-page Library asset",
            ));
        }
        let canonical_url = if is_confluence {
            if !confluence_page_id(&asset.source.canonical_id) {
                return Err(error(
                    "source_identity_mismatch",
                    "Confluence provider asset has an invalid page id",
                ));
            }
            let site = site_authority(&self.configuration, &asset.source.provider_id)?;
            let canonical = confluence_page_url(&site.provider_instance, &asset.source.canonical_id);
            let page = ConfluencePage {
                page_id: asset.source.canonical_id.clone(),
                space_key: asset
                    .container
                    .as_ref()
                    .map(|container| container.id.clone())
                    .unwrap_or_default(),
                title: asset.title.clone(),
                version: asset.source_revision.as_deref().and_then(|version| version.parse().ok()),
                source_url: canonical_url.to_owned(),
                canonical_url: canonical.clone(),
            };
            let authority = confluence_instance_authority(
                &self.configuration,
                &asset.source.provider_id,
                &page,
                &canonical,
            )?;
            if asset.source.provider_instance != authority.provider_instance {
                return Err(error(
                    "source_identity_mismatch",
                    "Confluence asset belongs to a different configured site",
                ));
            }
            canonical
        } else {
            let canonical = resolve_artifact(&self.configuration, canonical_url)?;
            if canonical.provider_id != asset.source.provider_id
                || canonical.kind != asset.source.resource_type
                || canonical.canonical_id != asset.source.canonical_id
            {
                return Err(error(
                    "source_identity_mismatch",
                    "Provider canonical URL identifies a different item",
                ));
            }
            self.request(canonical_url, Some(&asset.source.provider_id))?;
            canonical.canonical_url
        };
        let mut entry = asset_entry(asset, old);
        entry.references = self
            .is_jira_provider(&asset.source.provider_id)
            .then(|| asset_references(&self.configuration, asset));
        entry.relations_captured = entry.references.is_some();
        entry.canonical_url = Some(canonical_url);
        if let Some(row) = issue_row {
            entry.summary.issue = Some(LibraryIssueMeta {
                updated: row.updated.clone(),
                fetched_updated: Some(row.updated.clone()),
                status: row.status.clone(),
                issue_type: row.issue_type.clone(),
                assignee: row.assignee.clone(),
            });
        }
        if old.is_none() {
            if let Some(reference) = reference {
                refs::insert_ref(&mut entry.summary, reference.clone());
            }
        }
        Ok((entry, is_confluence))
    }
}
