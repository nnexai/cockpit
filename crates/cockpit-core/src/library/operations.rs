use super::store::{Lease, Store, bounded_write, error};
use crate::{
    InspectionError,
    project_store::{read_json_bounded, timestamp},
};
use cockpit_protocol::{library::*, v1::ErrorResponse};
use std::sync::Arc;
use uuid::Uuid;

const MAX_OPERATION_BYTES: u64 = 4 * 1024 * 1024;
const RETAIN_FINISHED: usize = 64;
const MAX_REPORT_ROWS: usize = 1000;

pub(crate) fn runtime() -> Result<tokio::runtime::Handle, InspectionError> {
    tokio::runtime::Handle::try_current().map_err(|_| {
        error(
            "library_unavailable",
            "Library workers require the host Tokio runtime",
        )
    })
}
fn name(id: &str) -> Result<String, InspectionError> {
    Uuid::parse_str(id).map_err(|_| {
        error(
            "library_operation_not_found",
            "Invalid Library operation identity",
        )
    })?;
    Ok(format!("{id}.json"))
}
pub(crate) fn create(
    store: &Store,
    kind: LibraryOperationKind,
    total: Option<u32>,
) -> Result<(LibraryOperation, Lease), InspectionError> {
    let now = timestamp();
    let record = LibraryOperation {
        operation_id: Uuid::new_v4().to_string(),
        kind,
        phases: vec![LibraryPhase {
            phase: LibraryPhaseName::Library,
            state: LibraryPhaseState::Running,
            done: 0,
            total,
            message: None,
            error: None,
        }],
        item_ids: vec![],
        report: Some(LibraryRefreshReport {
            new: 0,
            updated: 0,
            unchanged: 0,
            removed_at_source: 0,
            partial: 0,
            failed: 0,
            conflict: 0,
            rows: vec![],
            truncated_rows: false,
        }),
        space: None,
        target: None,
        cancel_requested: false,
        finished: false,
        created_at: now.clone(),
        updated_at: now,
    };
    let _lock = store.exclusive()?;
    // Ownership exists before the running record becomes visible, including the
    // interval before spawn first polls the worker.
    let lease = store.lease(&format!("operation:{}", record.operation_id))?;
    persist(store, &record)?;
    Ok((record, lease))
}
fn persist(store: &Store, record: &LibraryOperation) -> Result<(), InspectionError> {
    bounded_write(
        &store.operations,
        &name(&record.operation_id)?,
        record,
        MAX_OPERATION_BYTES,
    )
}
fn load(store: &Store, id: &str) -> Result<LibraryOperation, InspectionError> {
    let record: LibraryOperation =
        read_json_bounded(&store.operations, &name(id)?, MAX_OPERATION_BYTES)
            .map_err(|e| error("library_operation_not_found", e.message))?;
    if record.operation_id != id {
        return Err(error("library_corrupt", "Operation identity mismatch"));
    }
    Ok(record)
}
/// Caller holds library.lock exclusively. OS leases distinguish dead workers
/// from work owned by another host; timestamps and process IDs cannot do that.
fn reconcile(store: &Store, record: &mut LibraryOperation) -> Result<(), InspectionError> {
    if record.finished {
        return Ok(());
    }
    let _lease = match store.lease(&format!("operation:{}", record.operation_id)) {
        Ok(lease) => lease,
        Err(e) if e.code == "library_item_busy" => return Ok(()),
        Err(e) => return Err(e),
    };
    for phase in &mut record.phases {
        if phase.state == LibraryPhaseState::Running {
            phase.state = LibraryPhaseState::Failed;
            phase.error = Some(ErrorResponse {
                code: "library_operation_interrupted".into(),
                message: "Library worker stopped before recording completion".into(),
            });
        }
    }
    record.finished = true;
    record.updated_at = timestamp();
    persist(store, record)
}
pub(crate) fn get(store: &Store, id: &str) -> Result<LibraryOperation, InspectionError> {
    let _lock = store.exclusive()?;
    let mut record = load(store, id)?;
    reconcile(store, &mut record)?;
    Ok(record)
}
pub(crate) fn cancel(store: &Store, id: &str) -> Result<LibraryOperation, InspectionError> {
    let _lock = store.exclusive()?;
    let mut record = load(store, id)?;
    reconcile(store, &mut record)?;
    if !record.finished {
        record.cancel_requested = true;
        record.updated_at = timestamp();
        persist(store, &record)?;
    }
    Ok(record)
}
pub(crate) fn cancelled(store: &Store, id: &str) -> Result<bool, InspectionError> {
    Ok(get(store, id)?.cancel_requested)
}
pub(crate) fn row(
    store: &Store,
    id: &str,
    entry: Option<&LibraryItemSummary>,
    outcome: LibraryReportOutcome,
    reason: Option<String>,
) -> Result<(), InspectionError> {
    let _lock = store.exclusive()?;
    let mut record = load(store, id)?;
    record.phases[0].done += 1;
    if let Some(entry) = entry {
        if !record.item_ids.contains(&entry.item_id) {
            record.item_ids.push(entry.item_id.clone());
        }
    }
    if let Some(report) = &mut record.report {
        match outcome {
            LibraryReportOutcome::New => report.new += 1,
            LibraryReportOutcome::Updated => report.updated += 1,
            LibraryReportOutcome::Unchanged => report.unchanged += 1,
            LibraryReportOutcome::RemovedAtSource => report.removed_at_source += 1,
            LibraryReportOutcome::Partial => report.partial += 1,
            LibraryReportOutcome::Failed => report.failed += 1,
            LibraryReportOutcome::Conflict => report.conflict += 1,
        }
        if report.rows.len() < MAX_REPORT_ROWS {
            report.rows.push(LibraryReportRow {
                item_id: entry.map(|e| e.item_id.clone()),
                follow_id: entry.and_then(|e| e.follow_id.clone()),
                title: entry.map(|e| e.title.clone()).unwrap_or_default(),
                outcome,
                reason,
            });
        } else {
            report.truncated_rows = true;
        }
    }
    record.updated_at = timestamp();
    persist(store, &record)
}
pub(crate) fn finish(
    store: &Store,
    id: &str,
    result: Result<(), InspectionError>,
) -> Result<(), InspectionError> {
    let _lock = store.exclusive()?;
    let mut record = load(store, id)?;
    let phase = &mut record.phases[0];
    phase.state = if let Err(e) = result {
        phase.error = Some(ErrorResponse {
            code: e.code,
            message: e.message,
        });
        LibraryPhaseState::Failed
    } else if record.cancel_requested {
        LibraryPhaseState::Cancelled
    } else if record
        .report
        .as_ref()
        .is_some_and(|r| r.failed + r.conflict + r.partial > 0)
    {
        LibraryPhaseState::Partial
    } else {
        LibraryPhaseState::Done
    };
    record.finished = true;
    record.updated_at = timestamp();
    persist(store, &record)?;
    let mut finished = Vec::new();
    for file in store
        .operations
        .entries()
        .map_err(|e| error("library_unavailable", e.to_string()))?
    {
        let file = file.map_err(|e| error("library_unavailable", e.to_string()))?;
        let filename = file.file_name().to_string_lossy().into_owned();
        let Some(id) = filename.strip_suffix(".json") else {
            continue;
        };
        if Uuid::parse_str(id).is_err() {
            continue;
        }
        let operation = load(store, id)?;
        if operation.finished {
            finished.push((operation.updated_at.parse::<u128>().unwrap_or(0), filename));
        }
    }
    finished.sort();
    let remove = finished.len().saturating_sub(RETAIN_FINISHED);
    for (_, filename) in finished.into_iter().take(remove) {
        store
            .operations
            .remove_file(filename)
            .map_err(|e| error("library_unavailable", e.to_string()))?;
    }
    Ok(())
}
/// The caller obtains the host handle before persisting a running operation.
/// No runtime is captured by LibraryService::new or by a constructor thread.
pub(crate) fn spawn<F>(
    handle: tokio::runtime::Handle,
    store: Arc<Store>,
    id: String,
    lease: Lease,
    work: F,
) where
    F: Future<Output = Result<(), InspectionError>> + Send + 'static,
{
    handle.spawn(async move {
        let _lease = lease;
        let result = work.await;
        // Persistence failure remains observable when the operation is read again;
        // never claim a successful phase that could not be durably recorded.
        let _ = finish(&store, &id, result);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::{fixture, reopen};

    #[test]
    fn operation_lease_survives_create_to_spawn_gap_and_cross_host_cancel() {
        let f = fixture();
        let a = f.service.open().unwrap();
        let b = reopen(&f).open().unwrap();
        let (record, lease) = create(&a, LibraryOperationKind::Add, None).unwrap();
        assert!(!get(&b, &record.operation_id).unwrap().finished);
        let cancelled = cancel(&b, &record.operation_id).unwrap();
        assert!(cancelled.cancel_requested);
        assert!(!cancelled.finished);
        finish(&a, &record.operation_id, Ok(())).unwrap();
        drop(lease);
        let completed = get(&b, &record.operation_id).unwrap();
        assert!(completed.finished);
        assert_eq!(completed.phases[0].state, LibraryPhaseState::Cancelled);
    }

    #[test]
    fn get_and_cancel_terminalize_abandoned_operations_durably() {
        let f = fixture();
        let a = f.service.open().unwrap();
        for cancel_abandoned in [false, true] {
            let (record, lease) = create(&a, LibraryOperationKind::Refresh, Some(1)).unwrap();
            drop(lease); // The same OS lock release occurs on worker panic or process exit.
            let b = reopen(&f).open().unwrap();
            let interrupted = if cancel_abandoned {
                cancel(&b, &record.operation_id)
            } else {
                get(&b, &record.operation_id)
            }
            .unwrap();
            assert!(interrupted.finished);
            assert_eq!(interrupted.phases[0].state, LibraryPhaseState::Failed);
            assert_eq!(
                interrupted.phases[0].error.as_ref().unwrap().code,
                "library_operation_interrupted"
            );
            let again = get(&reopen(&f).open().unwrap(), &record.operation_id).unwrap();
            assert_eq!(
                serde_json::to_value(again).unwrap(),
                serde_json::to_value(interrupted).unwrap()
            );
        }
    }
    #[test]
    fn worker_runtime_shutdown_releases_operation_ownership() {
        let f = fixture();
        let a = f.service.open().unwrap();
        let b = reopen(&f).open().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (record, lease) = create(&a, LibraryOperationKind::Add, None).unwrap();
        let (entered, waiting) = tokio::sync::oneshot::channel();
        spawn(
            runtime.handle().clone(),
            a,
            record.operation_id.clone(),
            lease,
            async move {
                entered.send(()).unwrap();
                std::future::pending::<Result<(), InspectionError>>().await
            },
        );
        runtime.block_on(waiting).unwrap();
        assert!(!get(&b, &record.operation_id).unwrap().finished);
        drop(runtime);
        let interrupted = get(&b, &record.operation_id).unwrap();
        assert!(interrupted.finished);
        assert_eq!(interrupted.phases[0].state, LibraryPhaseState::Failed);
        assert_eq!(
            interrupted.phases[0].error.as_ref().unwrap().code,
            "library_operation_interrupted"
        );
    }
}
