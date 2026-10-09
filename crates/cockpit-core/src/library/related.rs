//! Reference-depth follow-up: saving the items a traversal reached, recording why
//! each is included, and reconciling inclusions when a complete pass no longer
//! reaches an item. Traversal itself lives in `SourceService::collect_related`.
use super::{
    LibraryService, SaveOptions,
    follow::plan,
    item_id, operations, refs,
    store::{LibraryIndexEntry, Store},
};
use crate::{
    InspectionError,
    sources::{
        ReferenceSeed, RelatedAsset, RelatedFailure, RelatedResult, SourceRef, TraversalBudget,
        TraversalStop, asset_label,
    },
};
use cockpit_protocol::library::{
    LibraryFollowSummary, LibraryInclusion, LibraryInclusionHolder, LibraryItemRef,
    LibraryItemState, LibraryItemSummary, LibraryReportOutcome,
};
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::AtomicBool},
};

/// The most failures spelled out in one report row.
const NOTE_DETAILS: usize = 5;

/// Membership metadata and the caller's exact exclusion rule.
pub(super) enum Holder<'a> {
    Item {
        item_id: &'a str,
    },
    Follow(&'a LibraryFollowSummary),
    /// Manual Jira refresh uses its snapshot exclusions and only issue keys.
    FollowIssues {
        follow: &'a LibraryFollowSummary,
        excluded: &'a BTreeSet<String>,
    },
}

impl Holder<'_> {
    pub(super) fn inclusion(&self) -> LibraryInclusionHolder {
        match self {
            Self::Item { item_id } => LibraryInclusionHolder::Item {
                item_id: (*item_id).to_owned(),
            },
            Self::Follow(follow) | Self::FollowIssues { follow, .. } => {
                LibraryInclusionHolder::Follow {
                    follow_id: follow.follow_id.clone(),
                }
            }
        }
    }

    pub(super) fn reference(&self) -> LibraryItemRef {
        match self {
            Self::Item { .. } => LibraryItemRef::Manual,
            Self::Follow(follow) | Self::FollowIssues { follow, .. } => LibraryItemRef::Follow {
                follow_id: follow.follow_id.clone(),
            },
        }
    }

    pub(super) fn budget(&self) -> TraversalBudget {
        match self {
            Self::Item { .. } => TraversalBudget::Single,
            Self::Follow(_) | Self::FollowIssues { .. } => TraversalBudget::Query,
        }
    }

    pub(super) fn skips(&self, related: &RelatedAsset) -> bool {
        let (follow, issues_only) = match self {
            Self::Item { .. } => return false,
            Self::Follow(follow) => (*follow, false),
            Self::FollowIssues { follow, .. } => (*follow, true),
        };
        let excluded = |id: &String| match self {
            Self::Item { .. } => false,
            Self::Follow(follow) => follow.excluded_ids.contains(id),
            Self::FollowIssues { excluded, .. } => excluded.contains(id),
        };
        let source = &related.asset.source;
        excluded(&item_id(source))
            || (source.provider_id == follow.provider_id
                && source.provider_instance == follow.provider_instance
                && (!issues_only || source.resource_type == "issue")
                && excluded(&source.canonical_id))
    }

    pub(super) fn holds(&self, summary: &LibraryItemSummary) -> bool {
        summary
            .included_by
            .iter()
            .flatten()
            .any(|inclusion| match (self, &inclusion.holder) {
                (Self::Item { item_id }, LibraryInclusionHolder::Item { item_id: id }) => {
                    *item_id == id
                }
                (
                    Self::Follow(follow) | Self::FollowIssues { follow, .. },
                    LibraryInclusionHolder::Follow { follow_id },
                ) => &follow.follow_id == follow_id,
                _ => false,
            })
    }
}

/// Known outgoing references may grow membership even when a seed is stale.
#[derive(Default)]
pub(super) struct Seeds {
    pub seeds: Vec<ReferenceSeed>,
    pub unknown: u32,
}

impl Seeds {
    pub(super) fn one(seed: ReferenceSeed) -> Self {
        Self {
            seeds: vec![seed],
            unknown: 0,
        }
    }

    pub(super) fn push_stored(&mut self, entry: &LibraryIndexEntry, label: String, stale: bool) {
        if stale {
            self.unknown += 1;
        }
        if let (Some(provider), Some(instance), Some(kind), Some(id), Some(references)) = (
            &entry.summary.provider_id,
            &entry.summary.provider_instance,
            &entry.summary.resource_type,
            &entry.summary.canonical_id,
            &entry.references,
        ) {
            self.seeds.push(ReferenceSeed {
                source: SourceRef {
                    provider_id: provider.clone(),
                    provider_instance: instance.clone(),
                    resource_type: kind.clone(),
                    canonical_id: id.clone(),
                },
                label,
                references: references.clone(),
            });
        } else {
            self.unknown += 1;
        }
    }

    pub(super) fn has_outgoing(&self) -> bool {
        self.seeds.iter().any(|seed| !seed.references.is_empty())
    }
}

pub(super) fn stale_state(state: LibraryItemState) -> bool {
    matches!(
        state,
        LibraryItemState::Failed | LibraryItemState::Conflict | LibraryItemState::RemovedAtSource
    )
}

/// What one traversal reached and whether absence may be inferred from it.
pub(super) struct RelatedPass {
    /// Item ids the traversal reached, whether or not this run could save them.
    pub reached: BTreeSet<String>,
    /// No failure, stop, unsaved item or unknown seed: an item that was not
    /// reached is genuinely no longer referenced.
    pub complete: bool,
    pub cancelled: bool,
    /// The Partial report row text; `None` when nothing went wrong.
    pub note: Option<String>,
}

impl RelatedPass {
    /// Depth 0: nothing to traverse, so nothing is reached.
    pub(super) fn none() -> Self {
        Self {
            reached: BTreeSet::new(),
            complete: true,
            cancelled: false,
            note: None,
        }
    }
    fn cancelled() -> Self {
        Self {
            reached: BTreeSet::new(),
            complete: false,
            cancelled: true,
            note: None,
        }
    }
}

struct Saved {
    reached: BTreeSet<String>,
    saved: u32,
    not_saved: Vec<String>,
    cancelled: bool,
}

fn clip(text: &str) -> String {
    text.chars().take(160).collect()
}

fn note(
    saved: u32,
    failures: &[RelatedFailure],
    not_saved: &[String],
    stopped: Option<TraversalStop>,
    unknown_seeds: u32,
) -> String {
    let mut details = failures
        .iter()
        .map(|failure| format!("{}: {}", failure.target, clip(&failure.message)))
        .chain(not_saved.iter().cloned())
        .collect::<Vec<_>>();
    let count = details.len();
    details.truncate(NOTE_DETAILS);
    let mut text = format!("Related: {saved} saved");
    if count > 0 {
        text.push_str(&format!("; {count} not saved ({}", details.join(", ")));
        if count > NOTE_DETAILS {
            text.push_str(&format!(", +{} more", count - NOTE_DETAILS));
        }
        text.push(')');
    }
    match stopped {
        Some(TraversalStop::Items) => text.push_str("; stopped at item limit"),
        Some(TraversalStop::Bytes) => text.push_str("; stopped at size limit"),
        Some(TraversalStop::Time) => text.push_str("; stopped at time limit"),
        Some(TraversalStop::Cancelled) => text.push_str("; cancelled"),
        None => {}
    }
    if unknown_seeds > 0 {
        text.push_str(&format!("; {unknown_seeds} seeds have no known references"));
    }
    text
}

impl LibraryService {
    /// All holder modes use the same bounded traversal and save engine.
    pub(super) async fn traverse(
        &self,
        store: &Arc<Store>,
        operation: &str,
        holder: &Holder<'_>,
        depth: u32,
        seeds: Seeds,
    ) -> Result<RelatedPass, InspectionError> {
        if depth == 0 {
            return Ok(RelatedPass::none());
        }
        self.run_related(
            store,
            operation,
            seeds.seeds,
            seeds.unknown,
            depth,
            holder.budget(),
            &holder.inclusion(),
            &holder.reference(),
            &|related| holder.skips(related),
        )
        .await
    }

    /// Traverses from `seeds`, saves what it reaches under `reference` and
    /// `holder`, and reports what could not be done. `skip` names reached items
    /// the user removed from the holder; they are not brought back.
    /// `unknown_seeds` counts seeds whose outgoing references are not known: the
    /// pass is then incomplete.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_related(
        &self,
        store: &Arc<Store>,
        operation: &str,
        seeds: Vec<ReferenceSeed>,
        unknown_seeds: u32,
        depth: u32,
        budget: TraversalBudget,
        holder: &LibraryInclusionHolder,
        reference: &LibraryItemRef,
        skip: &(dyn Fn(&RelatedAsset) -> bool + Sync),
    ) -> Result<RelatedPass, InspectionError> {
        let cancel = AtomicBool::new(false);
        let Some(result) = plan::until_cancelled(
            store,
            operation,
            &cancel,
            self.sources.collect_related(seeds, depth, budget, &cancel),
        )
        .await?
        else {
            return Ok(RelatedPass::cancelled());
        };
        let RelatedResult {
            assets,
            failures,
            stopped,
        } = result;
        // Discovered work: each reached item reports a row, so the phase total grows.
        operations::add_total(store, operation, assets.len() as u32)?;
        let saved = self
            .save_related(store, operation, assets, holder, reference, skip)
            .await?;
        if saved.cancelled {
            return Ok(RelatedPass::cancelled());
        }
        let complete = failures.is_empty()
            && stopped.is_none()
            && saved.not_saved.is_empty()
            && unknown_seeds == 0;
        let note = (!complete).then(|| {
            note(
                saved.saved,
                &failures,
                &saved.not_saved,
                stopped,
                unknown_seeds,
            )
        });
        Ok(RelatedPass {
            reached: saved.reached,
            complete,
            cancelled: false,
            note,
        })
    }

    /// Saves each reached item under its own lease, then records the reference
    /// and the inclusion through the index. A busy or failing item is reported,
    /// never fatal. A locally edited item reports its Conflict row (its bytes are
    /// untouched) and still keeps the reference and the inclusion, so membership
    /// is never lost to a conflict.
    #[allow(clippy::too_many_arguments)]
    async fn save_related(
        &self,
        store: &Arc<Store>,
        operation: &str,
        assets: Vec<RelatedAsset>,
        holder: &LibraryInclusionHolder,
        reference: &LibraryItemRef,
        skip: &(dyn Fn(&RelatedAsset) -> bool + Sync),
    ) -> Result<Saved, InspectionError> {
        let mut out = Saved {
            reached: BTreeSet::new(),
            saved: 0,
            not_saved: vec![],
            cancelled: false,
        };
        for related in assets {
            if operations::cancelled(store, operation)? {
                out.cancelled = true;
                break;
            }
            if skip(&related) {
                continue;
            }
            let id = item_id(&related.asset.source);
            out.reached.insert(id.clone());
            let label = asset_label(&related.asset);
            let inclusion = LibraryInclusion {
                holder: holder.clone(),
                from_item_id: Some(item_id(&related.from)),
                from_label: related.from_label.clone(),
                relation: related.relation.clone(),
                depth: related.depth,
            };
            let acquired = if crate::sources::lane::current()
                == crate::sources::lane::RequestLane::Background
            {
                store.lease(&id)
            } else {
                super::sync::manual_lease(store, &id).await
            };
            let lease = match acquired {
                Ok(lease) => lease,
                Err(failure) => {
                    out.not_saved
                        .push(format!("{label}: {}", clip(&failure.message)));
                    continue;
                }
            };
            let old = self.entry(store, &id)?;
            // The traversal/skip predicate is only a discovery snapshot. A
            // removal can commit while fetch is awaiting, before this item lease.
            let allowed = {
                let _lock = store.shared()?;
                Store::reference_allowed(
                    &store.index()?,
                    &super::asset_entry(&related.asset, old.as_ref()).summary,
                    reference,
                )
            };
            if !allowed {
                out.reached.remove(&id);
                continue;
            }
            match self
                .save_asset_with(
                    store,
                    operation,
                    related.asset,
                    old,
                    SaveOptions {
                        reference: Some(reference.clone()),
                        ..SaveOptions::default()
                    },
                )
                .await
            {
                Ok(()) => out.saved += 1,
                Err(failure) => {
                    if failure.code == "library_follow_excluded" {
                        out.reached.remove(&id);
                        continue;
                    }
                    let partial_manifest = failure.code == "source_attachments_partial";
                    out.not_saved
                        .push(format!("{label}: {}", clip(&failure.message)));
                    if partial_manifest {
                        continue;
                    }
                }
            }
            let changed = store.mutate_index_if(|index| {
                if index
                    .items
                    .iter()
                    .find(|e| e.summary.item_id == id)
                    .is_none_or(|entry| !Store::reference_allowed(index, &entry.summary, reference))
                {
                    return Ok((false, false));
                }
                let mut changed = false;
                if let Some(entry) = index.items.iter_mut().find(|e| e.summary.item_id == id) {
                    changed = !entry.summary.refs.contains(reference)
                        || entry.summary.purge_after.is_some()
                        || !entry
                            .summary
                            .included_by
                            .iter()
                            .flatten()
                            .any(|old| old == &inclusion);
                    if changed {
                        refs::insert_ref(&mut entry.summary, reference.clone());
                        refs::set_inclusion(&mut entry.summary, inclusion.clone());
                    }
                }
                Ok((changed, changed))
            })?;
            if changed
                && crate::sources::lane::current() == crate::sources::lane::RequestLane::Background
            {
                let receipt = operations::get(store, operation)?;
                if receipt
                    .report
                    .as_ref()
                    .is_some_and(|report| report.new + report.updated == 0)
                {
                    operations::add_total(store, operation, 1)?;
                    operations::row(
                        store,
                        operation,
                        None,
                        LibraryReportOutcome::Updated,
                        Some("Related inclusion reconciled".into()),
                    )?;
                }
            }
            drop(lease);
        }
        Ok(out)
    }

    /// Emits the pass's Partial row, if it has one.
    pub(super) fn related_row(
        &self,
        store: &Store,
        operation: &str,
        pass: &RelatedPass,
    ) -> Result<(), InspectionError> {
        match &pass.note {
            Some(note) => {
                // The row counts as done work, so it counts toward the total.
                operations::add_total(store, operation, 1)?;
                operations::row(
                    store,
                    operation,
                    None,
                    LibraryReportOutcome::Partial,
                    Some(note.clone()),
                )
            }
            None => Ok(()),
        }
    }

    /// Single-import lifecycle after the seed is saved: stores the depth, then
    /// (depth > 0) traverses and saves the related items as `Manual`, held by an
    /// `Item{seed}` inclusion. A complete pass strips the inclusion from items it
    /// no longer reaches; those items keep `Manual`. `seed` is `None` only when
    /// the provider did not return the requested item.
    pub(super) async fn apply_reference_depth(
        &self,
        store: &Arc<Store>,
        operation: &str,
        seed_id: &str,
        seed: Option<ReferenceSeed>,
        depth: u32,
    ) -> Result<(), InspectionError> {
        let holder = Holder::Item { item_id: seed_id };
        if depth == 0 {
            // Nothing was ever followed from this item: no index write.
            let followed = {
                let _lock = store.shared()?;
                store.index()?.items.iter().any(|entry| {
                    (entry.summary.item_id == seed_id && entry.summary.reference_depth.is_some())
                        || holder.holds(&entry.summary)
                })
            };
            if !followed {
                return Ok(());
            }
        }
        store.mutate_index(|index| {
            if let Some(entry) = index
                .items
                .iter_mut()
                .find(|e| e.summary.item_id == seed_id)
            {
                entry.summary.reference_depth = (depth > 0).then_some(depth);
            }
            Ok(())
        })?;
        let pass = if depth == 0 {
            RelatedPass::none()
        } else if let Some(seed) = seed {
            self.traverse(store, operation, &holder, depth, Seeds::one(seed))
                .await?
        } else {
            operations::row(
                store,
                operation,
                None,
                LibraryReportOutcome::Partial,
                Some("Related: the provider did not return the requested item".into()),
            )?;
            return Ok(());
        };
        self.related_row(store, operation, &pass)?;
        if pass.complete && !pass.cancelled {
            let missing = {
                let _lock = store.shared()?;
                unreached(store.index()?.items.iter(), &holder, &pass)
            };
            strip_inclusions(store, &holder.inclusion(), &missing)?;
        }
        Ok(())
    }

    /// Refresh of a single seed: traverses again at its stored depth. Refresh
    /// never fans out to Spaces.
    pub(super) async fn refresh_related(
        &self,
        store: &Arc<Store>,
        operation: &str,
        seed_id: &str,
        seed: ReferenceSeed,
    ) -> Result<(), InspectionError> {
        let depth = self
            .entry(store, seed_id)?
            .and_then(|entry| entry.summary.reference_depth)
            .unwrap_or(0);
        if depth == 0 {
            return Ok(());
        }
        self.apply_reference_depth(store, operation, seed_id, Some(seed), depth)
            .await
    }
}

/// Only a complete, uncancelled pass supplies absence evidence.
pub(super) fn unreached<'e>(
    items: impl IntoIterator<Item = &'e LibraryIndexEntry>,
    holder: &Holder<'_>,
    pass: &RelatedPass,
) -> BTreeSet<String> {
    if !pass.complete || pass.cancelled {
        return BTreeSet::new();
    }
    items
        .into_iter()
        .filter(|entry| {
            holder.holds(&entry.summary) && !pass.reached.contains(&entry.summary.item_id)
        })
        .map(|entry| entry.summary.item_id.clone())
        .collect()
}

/// Strip only this holder's inclusion, preserving all keeping references.
pub(super) fn strip_inclusions(
    store: &Store,
    holder: &LibraryInclusionHolder,
    ids: &BTreeSet<String>,
) -> Result<bool, InspectionError> {
    if ids.is_empty() {
        return Ok(false);
    }
    store.mutate_index_if(|index| {
        let mut changed = false;
        for entry in &mut index.items {
            if ids.contains(&entry.summary.item_id)
                && entry
                    .summary
                    .included_by
                    .iter()
                    .flatten()
                    .any(|i| &i.holder == holder)
            {
                refs::strip_inclusion(&mut entry.summary, holder);
                changed = true;
            }
        }
        Ok((changed, changed))
    })
}
