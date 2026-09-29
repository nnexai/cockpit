//! Reference-depth follow-up: saving the items a traversal reached, recording why
//! each is included, and reconciling inclusions when a complete pass no longer
//! reaches an item. Traversal itself lives in `SourceService::collect_related`.
use super::{LibraryService, SaveOptions, item_id, operations, refs, store::Store};
use crate::{
    InspectionError,
    sources::{
        ReferenceSeed, RelatedAsset, RelatedFailure, RelatedResult, TraversalBudget, TraversalStop,
        asset_label,
    },
};
use cockpit_protocol::library::{
    LibraryInclusion, LibraryInclusionHolder, LibraryItemRef, LibraryReportOutcome, SpaceTarget,
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// The most failures spelled out in one report row.
const NOTE_DETAILS: usize = 5;

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
        target: Option<&SpaceTarget>,
        skip: &(dyn Fn(&RelatedAsset) -> bool + Sync),
    ) -> Result<RelatedPass, InspectionError> {
        let cancel = AtomicBool::new(false);
        let result = {
            let traversal = self.sources.collect_related(seeds, depth, budget, &cancel);
            tokio::pin!(traversal);
            loop {
                tokio::select! {
                    result = &mut traversal => break result,
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {
                        if operations::cancelled(store, operation)? {
                            cancel.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
        };
        if cancel.load(Ordering::SeqCst) || operations::cancelled(store, operation)? {
            return Ok(RelatedPass::cancelled());
        }
        let RelatedResult {
            assets,
            failures,
            stopped,
        } = result;
        // Discovered work: each reached item reports a row, so the phase total grows.
        operations::add_total(store, operation, assets.len() as u32)?;
        let saved = self
            .save_related(store, operation, assets, holder, reference, target, skip)
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
        target: Option<&SpaceTarget>,
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
            let lease = match store.lease(&id) {
                Ok(lease) => lease,
                Err(failure) => {
                    out.not_saved
                        .push(format!("{label}: {}", clip(&failure.message)));
                    continue;
                }
            };
            let old = self.entry(store, &id)?;
            match self
                .save_asset_with(
                    store,
                    operation,
                    related.asset,
                    old,
                    SaveOptions {
                        target,
                        reference: Some(reference.clone()),
                        ..SaveOptions::default()
                    },
                )
                .await
            {
                Ok(()) => out.saved += 1,
                Err(failure) => out
                    .not_saved
                    .push(format!("{label}: {}", clip(&failure.message))),
            }
            store.mutate_index(|index| {
                if let Some(entry) = index.items.iter_mut().find(|e| e.summary.item_id == id) {
                    refs::insert_ref(&mut entry.summary, reference.clone());
                    refs::set_inclusion(&mut entry.summary, inclusion.clone());
                }
                Ok(())
            })?;
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
        target: Option<&SpaceTarget>,
    ) -> Result<(), InspectionError> {
        let holder = LibraryInclusionHolder::Item {
            item_id: seed_id.to_owned(),
        };
        if depth == 0 {
            // Nothing was ever followed from this item: no index write.
            let followed = {
                let _lock = store.shared()?;
                store.index()?.items.iter().any(|entry| {
                    (entry.summary.item_id == seed_id && entry.summary.reference_depth.is_some())
                        || entry
                            .summary
                            .included_by
                            .iter()
                            .flatten()
                            .any(|i| i.holder == holder)
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
            self.run_related(
                store,
                operation,
                vec![seed],
                0,
                depth,
                TraversalBudget::Single,
                &holder,
                &LibraryItemRef::Manual,
                target,
                &|_: &RelatedAsset| false,
            )
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
            strip_unreached(store, &holder, &pass.reached)?;
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
        self.apply_reference_depth(store, operation, seed_id, Some(seed), depth, None)
            .await
    }
}

/// Removes `holder`'s inclusion from every item a complete pass did not reach.
/// Skips the index write when there is nothing to remove.
pub(super) fn strip_unreached(
    store: &Store,
    holder: &LibraryInclusionHolder,
    reached: &BTreeSet<String>,
) -> Result<(), InspectionError> {
    let stale = |summary: &cockpit_protocol::library::LibraryItemSummary| {
        !reached.contains(&summary.item_id)
            && summary
                .included_by
                .iter()
                .flatten()
                .any(|i| &i.holder == holder)
    };
    let any = {
        let _lock = store.shared()?;
        store
            .index()?
            .items
            .iter()
            .any(|entry| stale(&entry.summary))
    };
    if !any {
        return Ok(());
    }
    store.mutate_index(|index| {
        for entry in &mut index.items {
            if stale(&entry.summary) {
                refs::strip_inclusion(&mut entry.summary, holder);
            }
        }
        Ok(())
    })
}
