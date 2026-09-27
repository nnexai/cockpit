use super::{
    LibraryService, operations,
    store::{Store, bounded_write, error},
};
use crate::{
    InspectionError,
    context_assets::{self, LibraryCopyMode, LibraryItemView},
    project_store::{open_dir_nofollow_absolute, read_json_bounded, timestamp},
};
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use cockpit_protocol::{context::ContextRoot, library::*, v1::ErrorResponse};
use sha2::{Digest, Sha256};
use std::{io::ErrorKind, path::Path, sync::Arc};

const MAX_ATTEMPTS: usize = 256;
const MAX_ATTEMPT_BYTES: u64 = 64 * 1024;

/// Fresh, descriptor-bound authority. Callers cannot manufacture a companion.
pub struct AuthorizedSpace {
    pub root: ContextRoot,
    pub space_label: String,
    dir: Dir,
}

fn unavailable(message: impl Into<String>) -> InspectionError {
    error("source_companion_unavailable", message)
}
fn response(error: &InspectionError) -> ErrorResponse {
    ErrorResponse {
        code: error.code.clone(),
        message: error.message.clone(),
    }
}
fn io(error: std::io::Error) -> InspectionError {
    super::store::error("library_unavailable", error.to_string())
}
fn same_target(a: &SpaceTarget, b: &SpaceTarget) -> bool {
    a.session_id == b.session_id && a.space_id == b.space_id
}
fn attempt_name(target: &SpaceTarget, id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(target.session_id.as_bytes());
    hash.update([0]);
    hash.update(target.space_id.as_bytes());
    hash.update([0]);
    hash.update(id.as_bytes());
    format!("{:x}.json", hash.finalize())
}
fn attempts_dir(store: &Store) -> Result<Dir, InspectionError> {
    match store.meta.create_dir("space-adds") {
        Ok(()) => {
            store.meta.open(".").map_err(io)?.sync_all().map_err(io)?;
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(io(error)),
    }
    store.meta.open_dir_nofollow("space-adds").map_err(io)
}
fn read_attempts(dir: &Dir) -> Result<Vec<SpaceAddAttempt>, InspectionError> {
    let mut attempts = Vec::new();
    for entry in dir.entries().map_err(io)? {
        let name = entry
            .map_err(io)?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if !name.ends_with(".json") {
            continue;
        }
        let attempt: SpaceAddAttempt = read_json_bounded(dir, &name, MAX_ATTEMPT_BYTES)?;
        let id = match (&attempt.item_id, &attempt.follow_id) {
            (Some(id), None) | (None, Some(id)) => id,
            _ => return Err(error("library_corrupt", "Invalid Space attempt identity")),
        };
        if attempt_name(&attempt.target, id) != name {
            return Err(error(
                "library_corrupt",
                "Space attempt filename differs from identity",
            ));
        }
        attempts.push(attempt);
    }
    attempts.sort_by(|a, b| {
        a.updated_at
            .parse::<u128>()
            .unwrap_or(0)
            .cmp(&b.updated_at.parse::<u128>().unwrap_or(0))
    });
    Ok(attempts)
}
fn persist_attempt(dir: &Dir, attempt: &SpaceAddAttempt) -> Result<(), InspectionError> {
    let id = attempt
        .item_id
        .as_ref()
        .or(attempt.follow_id.as_ref())
        .ok_or_else(|| error("library_corrupt", "Space attempt has no identity"))?;
    bounded_write(
        dir,
        &attempt_name(&attempt.target, id),
        attempt,
        MAX_ATTEMPT_BYTES,
    )
}
fn remove_attempt(dir: &Dir, target: &SpaceTarget, id: &str) -> Result<(), InspectionError> {
    match dir.remove_file(attempt_name(target, id)) {
        Ok(()) => dir.open(".").map_err(io)?.sync_all().map_err(io),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io(error)),
    }
}

/// Write ahead of Library publication, not merely ahead of companion copying.
/// Recovery drops an uncommitted new item's intent; a committed item always has
/// a retry record even if the process stops before recording its report row.
pub(super) fn prepare_saved_item(
    store: &Store,
    operation: &str,
    target: &SpaceTarget,
    item: &LibraryItemSummary,
) -> Result<(), InspectionError> {
    let _lock = store.exclusive()?;
    let dir = attempts_dir(store)?;
    let attempts = read_attempts(&dir)?;
    let exists = attempts
        .iter()
        .any(|a| same_target(&a.target, target) && a.item_id.as_ref() == Some(&item.item_id));
    if !exists && attempts.len() >= MAX_ATTEMPTS {
        let oldest = attempts
            .iter()
            .find(|a| a.state == SpaceAddAttemptState::Failed)
            .ok_or_else(|| error("library_item_busy", "Too many pending Space adds"))?;
        remove_attempt(
            &dir,
            &oldest.target,
            oldest
                .item_id
                .as_ref()
                .or(oldest.follow_id.as_ref())
                .unwrap(),
        )?;
    }
    persist_attempt(
        &dir,
        &SpaceAddAttempt {
            target: target.clone(),
            space_label: None,
            item_id: Some(item.item_id.clone()),
            follow_id: None,
            title: item.title.clone(),
            state: SpaceAddAttemptState::Pending,
            error: None,
            operation_id: operation.into(),
            updated_at: timestamp(),
        },
    )
}

/// The operation finisher already holds library.lock exclusively.
pub(super) fn fail_pending_attempts_locked(
    store: &Store,
    operation: &str,
    failure: &InspectionError,
) -> Result<(), InspectionError> {
    let dir = attempts_dir(store)?;
    let index = store.index()?;
    for mut attempt in read_attempts(&dir)? {
        if attempt.operation_id != operation || attempt.state != SpaceAddAttemptState::Pending {
            continue;
        }
        if let Some(id) = &attempt.item_id {
            if !index.items.iter().any(|entry| &entry.summary.item_id == id) {
                // The item may have been renamed while its index commit and
                // immediate journal recovery both failed. Only recover_attempts,
                // after successful Store recovery, can decide it never committed.
                continue;
            }
        }
        attempt.state = SpaceAddAttemptState::Failed;
        attempt.error = Some(response(failure));
        attempt.updated_at = timestamp();
        persist_attempt(&dir, &attempt)?;
    }
    Ok(())
}

/// OS operation leases distinguish another live host from an interrupted worker.
pub(super) fn recover_attempts(store: &Store) -> Result<(), InspectionError> {
    let _lock = store.exclusive()?;
    let dir = attempts_dir(store)?;
    let index = store.index()?;
    for mut attempt in read_attempts(&dir)? {
        if attempt.state != SpaceAddAttemptState::Pending {
            continue;
        }
        let _lease = match store.lease(&format!("operation:{}", attempt.operation_id)) {
            Ok(lease) => lease,
            Err(error) if error.code == "library_item_busy" => continue,
            Err(error) => return Err(error),
        };
        if let Some(id) = &attempt.item_id {
            if !index.items.iter().any(|entry| &entry.summary.item_id == id) {
                remove_attempt(&dir, &attempt.target, id)?;
                continue;
            }
        }
        attempt.state = SpaceAddAttemptState::Failed;
        attempt.error = Some(ErrorResponse {
            code: "space_add_interrupted".into(),
            message:
                "Space add stopped before recording completion; retry uses the saved Library item"
                    .into(),
        });
        attempt.updated_at = timestamp();
        persist_attempt(&dir, &attempt)?;
    }
    Ok(())
}

impl LibraryService {
    pub async fn authorize_space(
        &self,
        target: &SpaceTarget,
    ) -> Result<AuthorizedSpace, InspectionError> {
        let result = async {
            let projects = self
                .projects
                .as_ref()
                .ok_or_else(|| unavailable("Project service is unavailable"))?;
            let adapter = self
                .herdr
                .as_ref()
                .ok_or_else(|| unavailable("Herdr is unavailable"))?;
            let snapshot = adapter.session_snapshot(&target.session_id).await?;
            if snapshot.session_id != target.session_id {
                return Err(unavailable("Herdr returned a different session"));
            }
            let space = snapshot
                .spaces
                .iter()
                .find(|space| space.id == target.space_id)
                .ok_or_else(|| unavailable("Space is absent from the fresh Herdr snapshot"))?;
            let endpoint = projects
                .project_endpoint_identity(&target.session_id)
                .await?;
            let mut roots = projects
                .context_companions(&target.session_id, &target.space_id, &endpoint)
                .await?;
            if roots.len() != 1 {
                return Err(unavailable("Exactly one verified companion is required"));
            }
            let mut root = roots.remove(0);
            let companion = root
                .companion_id
                .as_deref()
                .ok_or_else(|| unavailable("Companion identity is missing"))?;
            let dir = open_dir_nofollow_absolute(Path::new(&root.path)).map_err(io)?;
            let metadata = dir.dir_metadata().map_err(io)?;
            let mut hash = Sha256::new();
            hash.update(companion.as_bytes());
            hash.update([0]);
            hash.update(root.path.as_bytes());
            hash.update([0]);
            #[cfg(unix)]
            {
                use cap_std::fs::MetadataExt;
                hash.update(metadata.dev().to_le_bytes());
                hash.update(metadata.ino().to_le_bytes());
            }
            #[cfg(not(unix))]
            {
                hash.update(metadata.len().to_le_bytes());
                hash.update(
                    metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.into_std().duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|duration| duration.as_nanos().to_le_bytes().to_vec())
                        .unwrap_or_default(),
                );
            }
            if !metadata.is_dir() || root.root_id != format!("companion-{:x}", hash.finalize()) {
                return Err(unavailable("Companion directory identity changed"));
            }
            // ContextService presents descriptor identities, not the project
            // store's opaque lookup identity. Return its exact navigation id.
            #[cfg(unix)]
            {
                use cap_std::fs::MetadataExt;
                root.root_id = format!(
                    "companion:{companion}:{}:{}",
                    metadata.dev(),
                    metadata.ino()
                );
            }
            #[cfg(not(unix))]
            {
                root.root_id = format!("companion:{companion}:{}", metadata.len());
            }
            Ok(AuthorizedSpace {
                root,
                dir,
                space_label: space.label.clone(),
            })
        }
        .await;
        result.map_err(|error: InspectionError| unavailable(error.message))
    }

    pub async fn space_listing(
        &self,
        target: SpaceTarget,
    ) -> Result<SpaceContextListing, InspectionError> {
        let store = self.open()?;
        let attempts = {
            let _lock = store.shared()?;
            read_attempts(&attempts_dir(&store)?)?
                .into_iter()
                .filter(|attempt| same_target(&attempt.target, &target))
                .collect()
        };
        let (companion, rows) = match self.authorize_space(&target).await {
            Ok(authorized) => {
                let _lock = store.shared()?;
                let items = store
                    .index()?
                    .items
                    .into_iter()
                    .map(|entry| entry.summary)
                    .collect::<Vec<_>>();
                let rows = context_assets::library_space_rows(
                    &authorized.dir,
                    authorized
                        .root
                        .companion_id
                        .as_deref()
                        .expect("authorized companion"),
                    &items,
                )?;
                (
                    SpaceCompanionStatus::Available {
                        companion_root_id: authorized.root.root_id,
                        companion_label: authorized.root.label,
                    },
                    rows,
                )
            }
            Err(error) => (
                SpaceCompanionStatus::Unavailable {
                    error: response(&error),
                },
                vec![],
            ),
        };
        let behind = rows
            .iter()
            .filter(|row| row.state == SpaceCopyState::LibraryNewer || row.library_newer)
            .count() as u32;
        Ok(SpaceContextListing {
            target,
            companion,
            attempts,
            rows,
            behind,
            diagnostics: vec![],
        })
    }

    pub async fn dismiss_space_attempts(
        &self,
        request: SpaceAttemptsDismissRequest,
    ) -> Result<(), InspectionError> {
        let store = self.open()?;
        let _attempt_leases = request
            .item_ids
            .iter()
            .chain(&request.follow_ids)
            .map(|id| store.lease(&format!("space-add:{}", attempt_name(&request.target, id))))
            .collect::<Result<Vec<_>, _>>()?;
        let _lock = store.exclusive()?;
        let dir = attempts_dir(&store)?;
        for id in request.item_ids.iter().chain(&request.follow_ids) {
            remove_attempt(&dir, &request.target, id)?;
        }
        Ok(())
    }

    pub async fn start_space_add(
        &self,
        request: SpaceAddRequest,
    ) -> Result<LibraryOperation, InspectionError> {
        if !request.follow_ids.is_empty() {
            return Err(error(
                "source_capability_unavailable",
                "Follow copying is not available",
            ));
        }
        let handle = operations::runtime()?;
        let store = self.open()?;
        let mut ids = request.item_ids;
        ids.sort();
        ids.dedup();
        let leases = ids
            .iter()
            .map(|id| store.lease(id))
            .collect::<Result<Vec<_>, _>>()?;
        for id in &ids {
            self.entry(&store, id)?
                .ok_or_else(|| error("library_item_not_found", "Library item does not exist"))?;
        }
        let (record, lease) = operations::create(
            &store,
            LibraryOperationKind::SpaceAdd,
            Some(ids.len() as u32),
        )?;
        let record = operations::set_target(&store, &record.operation_id, request.target.clone())?;
        let service = self.clone();
        let id = record.operation_id.clone();
        let worker_store = store.clone();
        operations::spawn(handle, store, id.clone(), lease, async move {
            let _leases = leases;
            service
                .copy_saved_items(&worker_store, &id, &request.target, &ids)
                .await
        });
        Ok(record)
    }

    pub async fn start_space_update(
        &self,
        request: SpaceUpdateRequest,
    ) -> Result<LibraryOperation, InspectionError> {
        let selected = match &request.scope {
            SpaceUpdateScope::Selection { item_ids, follow_ids } => {
                if !follow_ids.is_empty() {
                    return Err(error("source_capability_unavailable", "Follow updates are not available"));
                }
                if item_ids.len() > 5_000 {
                    return Err(error("invalid_library_request", "Too many selected items"));
                }
                Some(item_ids)
            }
            SpaceUpdateScope::All {} => None,
        };
        validate_confirmed(&request.replace_edited)?;
        let handle = operations::runtime()?;
        let store = self.open()?;
        let authorized = self.authorize_space(&request.target).await?;
        let rows = {
            let _lock = store.shared()?;
            let items = store.index()?.items.into_iter().map(|entry| entry.summary).collect::<Vec<_>>();
            context_assets::library_space_rows(&authorized.dir,
                authorized.root.companion_id.as_deref().expect("authorized companion"), &items)?
                .into_iter().filter(|row| {
                    row.item_id.as_ref().is_some_and(|id| selected.is_none_or(|ids| ids.contains(id)))
                        && row.current_library_revision.is_some()
                        && row.item_id.as_ref().is_some_and(|id| items.iter().any(|item|
                            &item.item_id == id && item.state != LibraryItemState::RemovedAtSource))
                        && matches!(row.state, SpaceCopyState::LibraryNewer | SpaceCopyState::MissingInSpace | SpaceCopyState::EditedInSpace)
                }).collect::<Vec<_>>()
        };
        // Confirmation is bound to this selection, target, and authoritative hash.
        // A stale dialog must fail before any other selected row can be written.
        for confirmed in &request.replace_edited {
            if !rows.iter().any(|row| row.edited.iter().any(|file|
                file.path == confirmed.path && file.current_hash == confirmed.current_hash))
            {
                return Err(error("space_copy_conflict", "The Space copy changed; reload before confirming"));
            }
        }
        let leases = rows.iter().filter_map(|row| row.item_id.as_deref())
            .map(|id| store.lease(id)).collect::<Result<Vec<_>, _>>()?;
        let (record, lease) = operations::create(&store, LibraryOperationKind::SpaceUpdate, Some(rows.len() as u32))?;
        let record = operations::set_target(&store, &record.operation_id, request.target.clone())?;
        let service = self.clone();
        let id = record.operation_id.clone();
        let worker_store = store.clone();
        operations::spawn(handle, store, id.clone(), lease, async move {
            let _leases = leases;
            operations::begin_space(&worker_store, &id, rows.len() as u32)?;
            let mut phase = SpacePhaseResult {
                space_id: request.target.space_id.clone(), copy_mode: None,
                written: vec![], skipped_edited: vec![], companion_root_id: None,
            };
            operations::space_result(&worker_store, &id, phase.clone(), 0)?;
            let mut first_error = None;
            for (position, row) in rows.iter().enumerate() {
                if operations::cancelled(&worker_store, &id)? { break; }
                let item_id = row.item_id.as_deref().expect("selected linked row");
                let confirmed = request.replace_edited.iter()
                    .filter(|file| row.paths.contains(&file.path)).cloned().collect::<Vec<_>>();
                let mode = if confirmed.is_empty() { LibraryCopyMode::Update }
                    else { LibraryCopyMode::Replace { confirmed: &confirmed } };
                let result = service.copy_saved_item_mode(&worker_store, &request.target, item_id, mode).await;
                let (outcome, reason) = match result {
                    Ok((authorized, copied)) => {
                        phase.companion_root_id = Some(authorized.root.root_id);
                        phase.copy_mode = match (phase.copy_mode, copied.copy_mode) {
                            (None, mode) | (mode, None) => mode,
                            (Some(a), Some(b)) => Some(if a == b { a } else { SpaceCopyMode::Mixed }),
                        };
                        let outcome = if !copied.skipped_edited.is_empty() { LibraryReportOutcome::Partial }
                            else if copied.written.is_empty() { LibraryReportOutcome::Unchanged }
                            else { LibraryReportOutcome::Updated };
                        phase.written.extend(copied.written);
                        phase.skipped_edited.extend(copied.skipped_edited);
                        (outcome, None)
                    }
                    Err(failure) => {
                        let outcome = if failure.code == "space_copy_conflict" {
                            LibraryReportOutcome::Conflict
                        } else { LibraryReportOutcome::Failed };
                        let reason = Some(failure.message.clone());
                        if first_error.is_none() { first_error = Some(failure); }
                        (outcome, reason)
                    }
                };
                let entry = service.entry(&worker_store, item_id)?;
                operations::row(&worker_store, &id, entry.as_ref().map(|entry| &entry.summary), outcome, reason)?;
                operations::space_result(&worker_store, &id, phase.clone(), (position + 1) as u32)?;
            }
            match first_error { Some(failure) => Err(failure), None => Ok(()) }
        });
        Ok(record)
    }

    pub async fn space_remove(
        &self,
        request: SpaceRemoveRequest,
    ) -> Result<SpaceContextListing, InspectionError> {
        validate_confirmed(&request.confirmed)?;
        if request.logical_id.starts_with("follow:") {
            return Err(error("source_capability_unavailable", "Follow removal is not available"));
        }
        if request.logical_id.is_empty() || request.logical_id.len() > 4096 {
            return Err(error("invalid_library_request", "Invalid Space copy identity"));
        }
        let authorized = self.authorize_space(&request.target).await?;
        context_assets::remove_library_copy(&authorized.dir,
            authorized.root.companion_id.as_deref().expect("authorized companion"),
            &request.logical_id, &request.confirmed)?;
        self.space_listing(request.target).await
    }

    /// Setup/cutover entry point: run the same central-first operation to completion.
    /// Phase-2 errors stay in the returned operation and durable attempts, not in
    /// the Result, so callers retain the successfully saved Library identities.
    pub async fn add_and_copy(
        &self,
        request: LibraryAddRequest,
    ) -> Result<LibraryOperation, InspectionError> {
        if request.target.is_none() {
            return Err(unavailable("A Space target is required"));
        }
        let operation = self.start_add(request).await?;
        loop {
            let record = self.operation(&operation.operation_id).await?;
            if record.finished {
                return Ok(record);
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    pub(super) async fn copy_saved_items(
        &self,
        store: &Arc<Store>,
        operation: &str,
        target: &SpaceTarget,
        ids: &[String],
    ) -> Result<(), InspectionError> {
        let _attempt_leases = ids
            .iter()
            .map(|id| store.lease(&format!("space-add:{}", attempt_name(target, id))))
            .collect::<Result<Vec<_>, _>>()?;
        let space_only = operations::get(store, operation)?.kind == LibraryOperationKind::SpaceAdd;
        // All retry records are durable before any companion lookup or write.
        {
            let _lock = store.exclusive()?;
            let dir = attempts_dir(store)?;
            let index = store.index()?;
            let existing = read_attempts(&dir)?;
            let added = ids
                .iter()
                .filter(|id| {
                    !existing
                        .iter()
                        .any(|a| same_target(&a.target, target) && a.item_id.as_ref() == Some(*id))
                })
                .count();
            let mut excess = (existing.len() + added).saturating_sub(MAX_ATTEMPTS);
            for attempt in &existing {
                if excess == 0 {
                    break;
                }
                if attempt.state == SpaceAddAttemptState::Failed
                    && !(same_target(&attempt.target, target)
                        && attempt.item_id.as_ref().is_some_and(|id| ids.contains(id)))
                {
                    remove_attempt(
                        &dir,
                        &attempt.target,
                        attempt
                            .item_id
                            .as_ref()
                            .or(attempt.follow_id.as_ref())
                            .unwrap(),
                    )?;
                    excess -= 1;
                }
            }
            if excess > 0 {
                return Err(error("library_item_busy", "Too many pending Space adds"));
            }
            for id in ids {
                let entry = index
                    .items
                    .iter()
                    .find(|entry| &entry.summary.item_id == id)
                    .ok_or_else(|| {
                        error("library_item_not_found", "Library item no longer exists")
                    })?;
                persist_attempt(
                    &dir,
                    &SpaceAddAttempt {
                        target: target.clone(),
                        space_label: None,
                        item_id: Some(id.clone()),
                        follow_id: None,
                        title: entry.summary.title.clone(),
                        state: SpaceAddAttemptState::Pending,
                        error: None,
                        operation_id: operation.into(),
                        updated_at: timestamp(),
                    },
                )?;
            }
        }
        #[cfg(test)]
        {
            let mut fault = store.fault.lock().unwrap_or_else(|e| e.into_inner());
            if *fault == Some("space_after_attempt") {
                *fault = None;
                return Err(error("library_test_crash", "space_after_attempt"));
            }
        }
        operations::begin_space(store, operation, ids.len() as u32)?;
        let mut phase = SpacePhaseResult {
            space_id: target.space_id.clone(),
            copy_mode: None,
            written: vec![],
            skipped_edited: vec![],
            companion_root_id: None,
        };
        let mut first_error = None;
        for (position, id) in ids.iter().enumerate() {
            let result = self.copy_saved_item(store, target, id).await;
            let outcome = match &result {
                Ok((_, copied)) if copied.written.is_empty() => LibraryReportOutcome::Unchanged,
                Ok(_) => LibraryReportOutcome::New,
                Err(error) if error.code == "source_sync_conflict" => {
                    LibraryReportOutcome::Conflict
                }
                Err(_) => LibraryReportOutcome::Failed,
            };
            let _lock = store.exclusive()?;
            let dir = attempts_dir(store)?;
            match result {
                Ok((authorized, copied)) => {
                    phase.companion_root_id = Some(authorized.root.root_id);
                    phase.copy_mode = match (phase.copy_mode, copied.copy_mode) {
                        (None, mode) | (mode, None) => mode,
                        (Some(a), Some(b)) => Some(if a == b { a } else { SpaceCopyMode::Mixed }),
                    };
                    phase.written.extend(copied.written);
                    remove_attempt(&dir, target, id)?;
                }
                Err(error) => {
                    let mut attempt: SpaceAddAttempt =
                        read_json_bounded(&dir, &attempt_name(target, id), MAX_ATTEMPT_BYTES)?;
                    attempt.state = SpaceAddAttemptState::Failed;
                    attempt.error = Some(response(&error));
                    attempt.updated_at = timestamp();
                    persist_attempt(&dir, &attempt)?;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
            drop(_lock);
            if space_only {
                let entry = self.entry(store, id)?;
                operations::row(
                    store,
                    operation,
                    entry.as_ref().map(|entry| &entry.summary),
                    outcome,
                    None,
                )?;
            }
            operations::space_result(store, operation, phase.clone(), (position + 1) as u32)?;
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }

    async fn copy_saved_item(
        &self,
        store: &Store,
        target: &SpaceTarget,
        id: &str,
    ) -> Result<(AuthorizedSpace, context_assets::LibraryCopyResult), InspectionError> {
        self.copy_saved_item_mode(store, target, id, LibraryCopyMode::NewOnly).await
    }

    async fn copy_saved_item_mode(
        &self,
        store: &Store,
        target: &SpaceTarget,
        id: &str,
        mode: LibraryCopyMode<'_>,
    ) -> Result<(AuthorizedSpace, context_assets::LibraryCopyResult), InspectionError> {
        let authorized = self.authorize_space(target).await?;
        let _lock = store.shared()?;
        let index = store.index()?;
        let entry = index
            .items
            .iter()
            .find(|entry| entry.summary.item_id == id)
            .ok_or_else(|| error("library_item_not_found", "Library item no longer exists"))?;
        if !store.conflicts(entry)?.is_empty() {
            return Err(error("library_conflict", "Library item was edited"));
        }
        let root = store.item_dir(&entry.summary.item_path)?;
        let view = LibraryItemView {
            root: &root,
            summary: &entry.summary,
            files: &entry.inventory,
        };
        let copied = context_assets::materialize_library_item(
            &authorized.dir,
            authorized
                .root
                .companion_id
                .as_deref()
                .expect("authorized companion"),
            &view,
            mode,
        )?;
        Ok((authorized, copied))
    }
}

fn validate_confirmed(files: &[LibraryConflictFile]) -> Result<(), InspectionError> {
    let mut paths = std::collections::BTreeSet::new();
    if files.len() > 512 || files.iter().any(|file|
        file.path.is_empty() || file.path.len() > 4096 || file.path.contains('\\')
        || Path::new(&file.path).components().any(|part| !matches!(part, std::path::Component::Normal(_)))
        || !file.current_hash.strip_prefix("sha256:").is_some_and(|digest|
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || !paths.insert(&file.path))
    {
        return Err(error("invalid_library_request", "Invalid Space confirmation files"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{Fixture, add, asset, finished, fixture, linked_add, reopen, saved};
    use super::*;
    use crate::{
        HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession,
        project_adapter::{
            ProjectInventory, ProjectTerminalRequest, ProjectTerminalResult,
            ProjectWorktreeRemoveRequest, ProjectWorktreeRequest, ProjectWorktreeResult,
        },
        projects::ProjectService,
    };
    use cockpit_protocol::{
        projects::{WorkspaceOperationRequest, WorkspaceOperationState, WorkspaceSetupRequest},
        v1::*,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Adapter {
        reachable: AtomicBool,
        space_present: AtomicBool,
        endpoint_changed: AtomicBool,
        space_id: String,
    }
    fn unused<T>() -> Result<T, InspectionError> {
        Err(error("unused", "Unexpected adapter call"))
    }
    #[async_trait::async_trait]
    impl HerdrAdapter for Adapter {
        async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> {
            unused()
        }
        async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> {
            unused()
        }
        async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
            unused()
        }
        async fn session_snapshot(
            &self,
            session: &str,
        ) -> Result<SessionSnapshotResponse, InspectionError> {
            if !self.reachable.load(Ordering::SeqCst) {
                return Err(error("disconnected", "Herdr stopped"));
            }
            Ok(SessionSnapshotResponse {
                session_id: session.into(),
                version: "test".into(),
                protocol: 1,
                focused_space_id: None,
                focused_tab_id: None,
                focused_pane_id: None,
                spaces: if self.space_present.load(Ordering::SeqCst) {
                    vec![SpaceSummary {
                        id: self.space_id.clone(),
                        label: "Test Space".into(),
                        number: 1,
                        tab_count: 0,
                        pane_count: 0,
                        focused: false,
                        agent_status: "none".into(),
                        git: None,
                    }]
                } else {
                    vec![]
                },
                tabs: vec![],
                panes: vec![],
                layouts: vec![],
                agents: vec![],
            })
        }
        async fn focus(&self, _: &str, _: &FocusRequest) -> Result<FocusResponse, InspectionError> {
            unused()
        }
        async fn mutate(
            &self,
            _: &str,
            _: &ResourceMutationRequest,
        ) -> Result<ResourceMutationResponse, InspectionError> {
            unused()
        }
        async fn subscribe_session(
            &self,
            _: &str,
            _: &SessionSnapshotResponse,
        ) -> Result<SessionSubscription, InspectionError> {
            unused()
        }
        async fn open_terminal(
            &self,
            _: &TerminalOpenRequest,
        ) -> Result<TerminalSession, InspectionError> {
            unused()
        }
    }
    #[async_trait::async_trait]
    impl ProjectHerdrAdapter for Adapter {
        async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> {
            Ok(if self.endpoint_changed.load(Ordering::SeqCst) {
                "different-endpoint"
            } else {
                "endpoint"
            }
            .into())
        }
        async fn project_inventory(
            &self,
            _: &str,
            _: &str,
        ) -> Result<ProjectInventory, InspectionError> {
            unused()
        }
        async fn project_worktree(
            &self,
            _: &str,
            request: &ProjectWorktreeRequest,
        ) -> Result<ProjectWorktreeResult, InspectionError> {
            Ok(ProjectWorktreeResult {
                workspace_id: self.space_id.clone(),
                tab_id: None,
                pane_id: None,
                checkout_path: request.checkout_path.clone(),
                branch: None,
                already_open: false,
            })
        }
        async fn project_terminal(
            &self,
            _: &str,
            request: &ProjectTerminalRequest,
        ) -> Result<ProjectTerminalResult, InspectionError> {
            Ok(ProjectTerminalResult {
                workspace_id: request.workspace_id.clone(),
                tab_id: "tab".into(),
                pane_id: "pane".into(),
            })
        }
        async fn project_worktree_dirty(
            &self,
            _: &str,
            _: u32,
            _: u32,
        ) -> Result<bool, InspectionError> {
            unused()
        }
        async fn project_close_workspace(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<(), InspectionError> {
            unused()
        }
        async fn project_remove_worktree(
            &self,
            _: &str,
            _: &ProjectWorktreeRemoveRequest,
        ) -> Result<(), InspectionError> {
            unused()
        }
    }

    fn target() -> SpaceTarget {
        SpaceTarget {
            session_id: "session".into(),
            space_id: "space".into(),
        }
    }
    async fn companion(f: &Fixture) -> (Arc<ProjectService>, Arc<Adapter>, std::path::PathBuf) {
        companion_named(f, "space").await
    }
    async fn companion_named(f: &Fixture, space: &str) -> (Arc<ProjectService>, Arc<Adapter>, std::path::PathBuf) {
        let adapter = Arc::new(Adapter {
            reachable: AtomicBool::new(true),
            space_present: AtomicBool::new(true),
            endpoint_changed: AtomicBool::new(false),
            space_id: space.into(),
        });
        let projects = Arc::new(
            ProjectService::new(f.service.configuration.clone(), adapter.clone()).unwrap(),
        );
        let checkout = f.root.join(format!("checkout-{space}"));
        std::fs::create_dir_all(&checkout).unwrap();
        let plan = projects
            .plan(
                "session",
                &WorkspaceSetupRequest::Open {
                    path: checkout.to_string_lossy().into_owned(),
                    label: Some("Test".into()),
                    task_name: None,
                    focus: false,
                },
            )
            .await
            .unwrap();
        projects
            .start(
                "session",
                &WorkspaceOperationRequest {
                    operation_id: plan.operation_id.clone(),
                    expected_generation: plan.generation,
                },
                Arc::new(f.service.clone()),
            )
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let operation = projects.get("session", &plan.operation_id).await.unwrap();
                match operation.state {
                    WorkspaceOperationState::Planned | WorkspaceOperationState::Running => {
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                    WorkspaceOperationState::Completed => break,
                    _ => panic!("companion setup failed: {operation:?}"),
                }
            }
        })
        .await
        .unwrap();
        let path = Path::new(&f.service.configuration.companion_root).join(plan.companion_id);
        (projects, adapter, path)
    }

    #[tokio::test]
    async fn central_first_failure_survives_reopen_and_retry_never_refetches() {
        let f = fixture();
        let (projects, adapter, companion) = companion(&f).await;
        adapter.reachable.store(false, Ordering::SeqCst);
        let service = reopen(&f).with_projects(projects.clone(), adapter.clone());
        let mut request = add(1);
        request.target = Some(target());
        let result = service.add_and_copy(request).await.unwrap();
        assert_eq!(result.phases[0].state, LibraryPhaseState::Done);
        assert_eq!(result.phases[1].state, LibraryPhaseState::Failed);
        assert_eq!(
            result.phases[1].error.as_ref().unwrap().code,
            "source_companion_unavailable"
        );
        let item_id = result.item_ids[0].clone();
        assert_eq!(
            service.listing(None).await.unwrap().items[0].item_id,
            item_id
        );
        assert!(!companion.join("sources").exists());
        assert!(!companion.join("context-manifest.json").exists());
        let fetches = f.provider.fetches.load(Ordering::SeqCst);
        drop(service);
        let service = reopen(&f).with_projects(projects, adapter.clone());
        let listing = service.space_listing(target()).await.unwrap();
        assert!(matches!(
            listing.companion,
            SpaceCompanionStatus::Unavailable { .. }
        ));
        assert_eq!(listing.attempts[0].state, SpaceAddAttemptState::Failed);
        assert_eq!(listing.attempts[0].item_id.as_ref(), Some(&item_id));
        adapter.reachable.store(true, Ordering::SeqCst);
        let retry = finished(
            &service,
            service
                .start_space_add(SpaceAddRequest {
                    target: target(),
                    item_ids: vec![item_id.clone()],
                    follow_ids: vec![],
                })
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(retry.phases[0].phase, LibraryPhaseName::Space);
        assert_eq!(retry.phases[0].state, LibraryPhaseState::Done);
        assert_eq!(retry.item_ids, vec![item_id.clone()]);
        assert_eq!(f.provider.fetches.load(Ordering::SeqCst), fetches);
        let listing = service.space_listing(target()).await.unwrap();
        assert!(listing.attempts.is_empty());
        assert_eq!(listing.rows[0].state, SpaceCopyState::UpToDate);
        let path = companion.join(&listing.rows[0].paths[0]);
        assert_eq!(
            std::fs::read(path).unwrap(),
            std::fs::read(
                Path::new(&f.service.configuration.library_root).join(
                    service.listing(None).await.unwrap().items[0]
                        .document_path
                        .as_ref()
                        .unwrap()
                )
            )
            .unwrap()
        );
        let manifest = std::fs::read(companion.join("context-manifest.json")).unwrap();
        let current = finished(
            &service,
            service
                .start_space_add(SpaceAddRequest {
                    target: target(),
                    item_ids: vec![item_id],
                    follow_ids: vec![],
                })
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(current.report.as_ref().unwrap().unchanged, 1);
        assert!(current.space.unwrap().written.is_empty());
        assert_eq!(
            std::fs::read(companion.join("context-manifest.json")).unwrap(),
            manifest
        );
    }

    #[tokio::test]
    async fn fresh_space_presence_and_endpoint_are_required_before_writes() {
        let f = fixture();
        let (projects, adapter, path) = companion(&f).await;
        let item = saved(&f, 1).await;
        let service = reopen(&f).with_projects(projects, adapter.clone());
        for endpoint in [false, true] {
            adapter.space_present.store(endpoint, Ordering::SeqCst);
            adapter.endpoint_changed.store(endpoint, Ordering::SeqCst);
            let result = finished(
                &service,
                service
                    .start_space_add(SpaceAddRequest {
                        target: target(),
                        item_ids: vec![item.item_id.clone()],
                        follow_ids: vec![],
                    })
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(
                result.phases[0].error.as_ref().unwrap().code,
                "source_companion_unavailable"
            );
            assert!(!path.join("sources").exists());
            assert!(!path.join("context-manifest.json").exists());
        }
        assert_eq!(
            service
                .space_listing(target())
                .await
                .unwrap()
                .attempts
                .len(),
            1
        );
        service
            .dismiss_space_attempts(SpaceAttemptsDismissRequest {
                target: SpaceTarget {
                    session_id: "other".into(),
                    space_id: "space".into(),
                },
                item_ids: vec![item.item_id.clone()],
                follow_ids: vec![],
            })
            .await
            .unwrap();
        assert_eq!(
            service
                .space_listing(target())
                .await
                .unwrap()
                .attempts
                .len(),
            1
        );
        service
            .dismiss_space_attempts(SpaceAttemptsDismissRequest {
                target: target(),
                item_ids: vec![item.item_id],
                follow_ids: vec![],
            })
            .await
            .unwrap();
        assert!(
            service
                .space_listing(target())
                .await
                .unwrap()
                .attempts
                .is_empty()
        );
    }

    #[tokio::test]
    async fn cancel_after_first_save_preserves_cancelled_library_and_copies_saved_subset() {
        let f = fixture();
        let (projects, adapter, _) = companion(&f).await;
        let service = reopen(&f).with_projects(projects, adapter);
        let store = service.open().unwrap();
        *store.fault.lock().unwrap_or_else(|e| e.into_inner()) =
            Some("space_cancel_after_library_publish");
        let mut request = linked_add(&f);
        request.target = Some(target());
        let operation = service.add_and_copy(request).await.unwrap();
        assert_eq!(operation.phases[0].state, LibraryPhaseState::Cancelled);
        assert_eq!(operation.phases[1].state, LibraryPhaseState::Done);
        let items = service.listing(None).await.unwrap().items;
        assert_eq!(
            items
                .iter()
                .map(|item| item.canonical_id.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("acme/repo#1")]
        );
        let listing = service.space_listing(target()).await.unwrap();
        assert!(listing.attempts.is_empty());
        assert_eq!(listing.rows.len(), 1);
        assert_eq!(listing.rows[0].item_id.as_ref(), Some(&items[0].item_id));
        assert_eq!(listing.rows[0].state, SpaceCopyState::UpToDate);
    }

    #[tokio::test]
    async fn failed_index_roll_forward_keeps_attempt_until_journal_recovery() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let (operation, lease) =
            operations::create(&store, LibraryOperationKind::Add, None).unwrap();
        operations::set_target(&store, &operation.operation_id, target()).unwrap();
        *store.fault.lock().unwrap_or_else(|e| e.into_inner()) = Some("rename_unsynced");
        let publish_failure = f
            .service
            .save_asset(
                &store,
                &operation.operation_id,
                asset(1, "published before index"),
                None,
                None,
                Some(&target()),
            )
            .unwrap_err();
        assert_eq!(publish_failure.code, "library_test_crash");
        *store.fault.lock().unwrap_or_else(|e| e.into_inner()) = Some("recovery_sync");
        let recovery_failure = store.recover_pending().unwrap_err();
        assert_eq!(recovery_failure.code, "library_test_crash");
        {
            let _lock = store.shared().unwrap();
            assert!(store.index().unwrap().items.is_empty());
        }
        operations::finish(&store, &operation.operation_id, Err(recovery_failure)).unwrap();
        {
            let _lock = store.shared().unwrap();
            let attempts = read_attempts(&attempts_dir(&store).unwrap()).unwrap();
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].state, SpaceAddAttemptState::Pending);
        }
        drop(lease);
        let reopened = reopen(&f);
        let items = reopened.listing(None).await.unwrap().items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].canonical_id.as_deref(), Some("acme/repo#1"));
        let attempts = reopened.space_listing(target()).await.unwrap().attempts;
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].item_id.as_ref(), Some(&items[0].item_id));
        assert_eq!(attempts[0].state, SpaceAddAttemptState::Failed);
        assert_eq!(
            attempts[0].error.as_ref().unwrap().code,
            "space_add_interrupted"
        );
    }

    #[tokio::test]
    async fn interrupted_pending_attempt_is_recovered_but_live_worker_is_not() {
        let f = fixture();
        let item = saved(&f, 1).await;
        let store = f.service.open().unwrap();
        let (operation, lease) =
            operations::create(&store, LibraryOperationKind::SpaceAdd, None).unwrap();
        operations::set_target(&store, &operation.operation_id, target()).unwrap();
        *store.fault.lock().unwrap() = Some("space_after_attempt");
        let error = f
            .service
            .copy_saved_items(
                &store,
                &operation.operation_id,
                &target(),
                &[item.item_id.clone()],
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "library_test_crash");
        let live = reopen(&f).space_listing(target()).await.unwrap();
        assert_eq!(live.attempts[0].state, SpaceAddAttemptState::Pending);
        drop(lease);
        drop(store);
        let recovered = reopen(&f).space_listing(target()).await.unwrap();
        assert_eq!(recovered.attempts[0].state, SpaceAddAttemptState::Failed);
        assert_eq!(
            recovered.attempts[0].error.as_ref().unwrap().code,
            "space_add_interrupted"
        );
        assert_eq!(recovered.attempts[0].item_id.as_ref(), Some(&item.item_id));
    }

    #[tokio::test]
    async fn bounded_attempts_drop_oldest_failed_not_pending() {
        let f = fixture();
        let item = saved(&f, 1).await;
        let store = f.service.open().unwrap();
        let (live, _lease) =
            operations::create(&store, LibraryOperationKind::SpaceAdd, None).unwrap();
        {
            let _lock = store.exclusive().unwrap();
            let dir = attempts_dir(&store).unwrap();
            for n in 0..MAX_ATTEMPTS {
                persist_attempt(
                    &dir,
                    &SpaceAddAttempt {
                        target: target(),
                        space_label: None,
                        item_id: Some(format!("old-{n}")),
                        follow_id: None,
                        title: "old".into(),
                        state: if n == 0 {
                            SpaceAddAttemptState::Pending
                        } else {
                            SpaceAddAttemptState::Failed
                        },
                        error: None,
                        operation_id: live.operation_id.clone(),
                        updated_at: n.to_string(),
                    },
                )
                .unwrap();
            }
        }
        finished(
            &f.service,
            f.service
                .start_space_add(SpaceAddRequest {
                    target: target(),
                    item_ids: vec![item.item_id.clone()],
                    follow_ids: vec![],
                })
                .await
                .unwrap(),
        )
        .await;
        let attempts = f.service.space_listing(target()).await.unwrap().attempts;
        assert_eq!(attempts.len(), MAX_ATTEMPTS);
        assert!(
            attempts
                .iter()
                .any(|a| a.item_id.as_deref() == Some("old-0")
                    && a.state == SpaceAddAttemptState::Pending)
        );
        assert!(
            !attempts
                .iter()
                .any(|a| a.item_id.as_deref() == Some("old-1"))
        );
        assert!(
            attempts
                .iter()
                .any(|a| a.item_id.as_ref() == Some(&item.item_id))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_companion_fails_closed_without_touching_its_target() {
        let f = fixture();
        let (projects, adapter, companion) = companion(&f).await;
        let item = saved(&f, 1).await;
        let service = reopen(&f).with_projects(projects, adapter);
        let detached = companion.with_extension("detached");
        std::fs::rename(&companion, &detached).unwrap();
        let outside = f.root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"unchanged").unwrap();
        std::os::unix::fs::symlink(&outside, &companion).unwrap();
        let result = finished(
            &service,
            service
                .start_space_add(SpaceAddRequest {
                    target: target(),
                    item_ids: vec![item.item_id],
                    follow_ids: vec![],
                })
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            result.phases[0].error.as_ref().unwrap().code,
            "source_companion_unavailable"
        );
        assert_eq!(
            std::fs::read(outside.join("sentinel")).unwrap(),
            b"unchanged"
        );
        assert!(!outside.join("sources").exists());
        assert!(!outside.join("context-manifest.json").exists());
        assert!(!detached.join("sources").exists());
    }

    #[tokio::test]
    async fn explicit_update_is_selected_space_only_and_confirmed_edits_are_cas() {
        let f = fixture();
        let (projects_x, adapter_x, x) = companion(&f).await;
        let (projects_y, adapter_y, y) = companion_named(&f, "other-space").await;
        let service_x = reopen(&f).with_projects(projects_x, adapter_x);
        let service_y = reopen(&f).with_projects(projects_y, adapter_y);
        let target_y = SpaceTarget { session_id: "session".into(), space_id: "other-space".into() };
        let a = saved(&f, 1).await;
        let b = saved(&f, 2).await;
        for (service, target) in [(&service_x, target()), (&service_y, target_y.clone())] {
            let result = finished(service, service.start_space_add(SpaceAddRequest {
                target, item_ids: vec![a.item_id.clone(), b.item_id.clone()], follow_ids: vec![],
            }).await.unwrap()).await;
            assert_eq!(result.phases[0].state, LibraryPhaseState::Done);
        }
        let rows = service_x.space_listing(target()).await.unwrap().rows;
        let a_path = rows.iter().find(|row| row.item_id.as_ref() == Some(&a.item_id)).unwrap().paths[0].clone();
        let b_path = rows.iter().find(|row| row.item_id.as_ref() == Some(&b.item_id)).unwrap().paths[0].clone();
        let before_a = std::fs::read(x.join(&a_path)).unwrap();
        let before_b = std::fs::read(x.join(&b_path)).unwrap();
        f.provider.set_body("new provider revision");
        let refresh = finished(&f.service, f.service.start_refresh(LibraryRefreshRequest::All).await.unwrap()).await;
        assert_eq!(refresh.report.unwrap().updated, 2);
        for (service, target, root) in [(&service_x, target(), &x), (&service_y, target_y.clone(), &y)] {
            assert_eq!(std::fs::read(root.join(&a_path)).unwrap(), before_a);
            assert_eq!(std::fs::read(root.join(&b_path)).unwrap(), before_b);
            assert!(service.space_listing(target).await.unwrap().rows.iter().all(|row| row.state == SpaceCopyState::LibraryNewer));
        }
        let update = finished(&service_x, service_x.start_space_update(SpaceUpdateRequest {
            target: target(), scope: SpaceUpdateScope::Selection { item_ids: vec![a.item_id.clone()], follow_ids: vec![] },
            replace_edited: vec![],
        }).await.unwrap()).await;
        assert_eq!(update.kind, LibraryOperationKind::SpaceUpdate);
        assert_eq!(update.phases.len(), 1);
        assert_eq!(update.phases[0].state, LibraryPhaseState::Done);
        assert_eq!(update.space.unwrap().written, vec![a_path.clone()]);
        let rows = service_x.space_listing(target()).await.unwrap().rows;
        assert_eq!(rows.iter().find(|row| row.item_id.as_ref() == Some(&a.item_id)).unwrap().state, SpaceCopyState::UpToDate);
        assert_eq!(rows.iter().find(|row| row.item_id.as_ref() == Some(&b.item_id)).unwrap().state, SpaceCopyState::LibraryNewer);
        assert_eq!(std::fs::read(x.join(&b_path)).unwrap(), before_b);
        assert_eq!(std::fs::read(y.join(&a_path)).unwrap(), before_a);
        assert_eq!(std::fs::read(y.join(&b_path)).unwrap(), before_b);

        std::fs::write(y.join(&a_path), b"edited in Y").unwrap();
        let edited = service_y.space_listing(target_y.clone()).await.unwrap().rows.into_iter()
            .find(|row| row.item_id.as_ref() == Some(&a.item_id)).unwrap();
        assert_eq!(edited.state, SpaceCopyState::EditedInSpace);
        assert!(edited.library_newer);
        let all = finished(&service_y, service_y.start_space_update(SpaceUpdateRequest {
            target: target_y.clone(), scope: SpaceUpdateScope::All {}, replace_edited: vec![],
        }).await.unwrap()).await;
        assert_eq!(all.space.unwrap().skipped_edited, vec![a_path.clone()]);
        assert_eq!(std::fs::read(y.join(&a_path)).unwrap(), b"edited in Y");
        let replacement = |confirmed| SpaceUpdateRequest {
            target: target_y.clone(),
            scope: SpaceUpdateScope::Selection { item_ids: vec![a.item_id.clone()], follow_ids: vec![] },
            replace_edited: confirmed,
        };
        assert_eq!(service_y.start_space_update(replacement(vec![LibraryConflictFile {
            path: a_path.clone(), current_hash: super::super::store::hash(b"stale edit"),
        }])).await.unwrap_err().code, "space_copy_conflict");
        assert_eq!(std::fs::read(y.join(&a_path)).unwrap(), b"edited in Y");
        let replaced = finished(&service_y, service_y.start_space_update(replacement(edited.edited)).await.unwrap()).await;
        assert_eq!(replaced.phases[0].state, LibraryPhaseState::Done);
        assert_eq!(std::fs::read(y.join(&a_path)).unwrap(), std::fs::read(x.join(&a_path)).unwrap());

        std::fs::remove_file(x.join(&b_path)).unwrap();
        let restored = finished(&service_x, service_x.start_space_update(SpaceUpdateRequest {
            target: target(), scope: SpaceUpdateScope::All {}, replace_edited: vec![],
        }).await.unwrap()).await;
        assert_eq!(restored.space.unwrap().written, vec![b_path.clone()]);
        assert_eq!(std::fs::read(x.join(&b_path)).unwrap(), std::fs::read(y.join(&b_path)).unwrap());
    }

    #[tokio::test]
    async fn folder_space_update_confirms_multiple_edited_paths_in_one_operation() {
        let f = fixture();
        let (projects, adapter, root) = companion(&f).await;
        let service = reopen(&f).with_projects(projects, adapter);
        let origin = f.root.join("folder-origin");
        std::fs::create_dir_all(origin.join("nested")).unwrap();
        std::fs::write(origin.join("a.txt"), b"a").unwrap();
        std::fs::write(origin.join("nested/b.txt"), b"b").unwrap();
        let added = finished(&service, service.start_add(LibraryAddRequest {
            input: origin.to_string_lossy().into_owned(), provider_id: None,
            hydrate_references: false, follow_space: false, download_attachments: false,
            refresh_existing: false, label: Some("Folder notes".into()), target: Some(target()),
        }).await.unwrap()).await;
        assert!(added.phases.iter().all(|phase| phase.state == LibraryPhaseState::Done));
        let row = service.space_listing(target()).await.unwrap().rows.remove(0);
        for path in &row.paths {
            std::fs::write(root.join(path), b"edited in Space").unwrap();
        }
        std::fs::write(origin.join("a.txt"), b"new a").unwrap();
        std::fs::write(origin.join("nested/b.txt"), b"new b").unwrap();
        finished(&service, service.start_refresh(LibraryRefreshRequest::Items {
            item_ids: vec![row.item_id.clone().unwrap()],
        }).await.unwrap()).await;
        let edited = service.space_listing(target()).await.unwrap().rows.remove(0);
        assert_eq!(edited.state, SpaceCopyState::EditedInSpace);
        let update = finished(&service, service.start_space_update(SpaceUpdateRequest {
            target: target(),
            scope: SpaceUpdateScope::Selection { item_ids: vec![row.item_id.unwrap()], follow_ids: vec![] },
            replace_edited: edited.edited,
        }).await.unwrap()).await;
        assert_eq!(update.phases[0].state, LibraryPhaseState::Done);
        assert!(update.space.unwrap().skipped_edited.is_empty());
        let row = service.space_listing(target()).await.unwrap().rows.remove(0);
        assert_eq!(row.state, SpaceCopyState::UpToDate);
        let a = row.paths.iter().find(|path| path.ends_with("/a.txt")).unwrap();
        let b = row.paths.iter().find(|path| path.ends_with("/nested/b.txt")).unwrap();
        assert_eq!(std::fs::read(root.join(a)).unwrap(), b"new a");
        assert_eq!(std::fs::read(root.join(b)).unwrap(), b"new b");
        service.space_remove(SpaceRemoveRequest {
            target: target(), logical_id: row.logical_id, confirmed: vec![],
        }).await.unwrap();
        assert!(!root.join(a).exists());
        assert!(!root.join(b).exists());
    }

    #[tokio::test]
    async fn removed_source_and_library_removal_keep_space_bytes_and_remove_requires_cas() {
        let f = fixture();
        let (projects, adapter, root) = companion(&f).await;
        let service = reopen(&f).with_projects(projects, adapter.clone());
        let item = saved(&f, 1).await;
        finished(&service, service.start_space_add(SpaceAddRequest {
            target: target(), item_ids: vec![item.item_id.clone()], follow_ids: vec![],
        }).await.unwrap()).await;
        let row = service.space_listing(target()).await.unwrap().rows.remove(0);
        let path = root.join(&row.paths[0]);
        let original = std::fs::read(&path).unwrap();
        f.provider.set_failure(Some("source_not_found"));
        finished(&service, service.start_refresh(LibraryRefreshRequest::All).await.unwrap()).await;
        assert_eq!(service.space_listing(target()).await.unwrap().rows[0].state, SpaceCopyState::RemovedAtSource);
        let request = || SpaceUpdateRequest {
            target: target(), scope: SpaceUpdateScope::Selection { item_ids: vec![item.item_id.clone()], follow_ids: vec![] },
            replace_edited: vec![],
        };
        let update = finished(&service, service.start_space_update(request()).await.unwrap()).await;
        assert!(update.space.unwrap().written.is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        service.remove(LibraryRemoveRequest::Item {
            item_id: item.item_id.clone(), expected_revision: item.revision.clone(),
        }).await.unwrap();
        assert_eq!(service.space_listing(target()).await.unwrap().rows[0].state, SpaceCopyState::NotInLibrary);
        finished(&service, service.start_space_update(request()).await.unwrap()).await;
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::write(&path, b"my edits").unwrap();
        let remove = |confirmed| SpaceRemoveRequest { target: target(), logical_id: row.logical_id.clone(), confirmed };
        assert_eq!(service.space_remove(remove(vec![])).await.unwrap_err().code, "space_copy_conflict");
        let edited = service.space_listing(target()).await.unwrap().rows.remove(0).edited;
        std::fs::write(&path, b"later edits").unwrap();
        assert_eq!(service.space_remove(remove(edited)).await.unwrap_err().code, "space_copy_conflict");
        assert_eq!(std::fs::read(&path).unwrap(), b"later edits");
        let edited = service.space_listing(target()).await.unwrap().rows.remove(0).edited;
        adapter.reachable.store(false, Ordering::SeqCst);
        assert_eq!(service.space_remove(remove(edited.clone())).await.unwrap_err().code, "source_companion_unavailable");
        assert_eq!(std::fs::read(&path).unwrap(), b"later edits");
        adapter.reachable.store(true, Ordering::SeqCst);
        let listing = service.space_remove(remove(edited)).await.unwrap();
        assert!(listing.rows.is_empty());
        assert!(!path.exists());
        assert_eq!(service.start_space_update(SpaceUpdateRequest {
            target: target(), scope: SpaceUpdateScope::Selection { item_ids: vec![], follow_ids: vec!["follow".into()] },
            replace_edited: vec![],
        }).await.unwrap_err().code, "source_capability_unavailable");
        assert_eq!(service.space_remove(SpaceRemoveRequest {
            target: target(), logical_id: "follow:one".into(), confirmed: vec![],
        }).await.unwrap_err().code, "source_capability_unavailable");
    }
}
