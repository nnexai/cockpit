use crate::{
    InspectionError,
    library::{operations, store::Store},
};
use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub(in crate::library) async fn until_cancelled<F: Future>(
    store: &Store,
    operation: &str,
    cancel: &AtomicBool,
    future: F,
) -> Result<Option<F::Output>, InspectionError> {
    tokio::pin!(future);
    let result = loop {
        tokio::select! {
            result = &mut future => break result,
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if operations::cancelled(store, operation)? {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        }
    };
    if cancel.load(Ordering::SeqCst) || operations::cancelled(store, operation)? {
        Ok(None)
    } else {
        Ok(Some(result))
    }
}
