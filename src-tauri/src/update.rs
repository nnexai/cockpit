use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use cockpit_host::{
    backend::{self, RestartResult, RestartStep, RestartTicket},
    transport::{
        error::OperationError,
        operations::backend::BackendContext,
        shutdown::{HostShutdown, ShutdownPolicy},
    },
};
use cockpit_protocol::v1::ErrorResponse;
use tauri::{AppHandle, Runtime, State};

pub struct NativeShutdown {
    started: AtomicBool,
    shutdown: HostShutdown,
}

impl NativeShutdown {
    pub fn new(shutdown: HostShutdown) -> Self {
        Self {
            started: AtomicBool::new(false),
            shutdown,
        }
    }

    pub async fn exit<R: Runtime>(&self, app: AppHandle<R>) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        self.shutdown.run(ShutdownPolicy::Native).await;
        app.exit(0);
    }
}

#[tauri::command]
pub async fn cockpit_update_and_relaunch<R: Runtime>(
    expected_instance: String,
    app: AppHandle<R>,
    context: State<'_, Arc<BackendContext>>,
    shutdown: State<'_, Arc<NativeShutdown>>,
) -> Result<RestartTicket, ErrorResponse> {
    let expected = uuid::Uuid::parse_str(&expected_instance).map_err(|error| {
        ErrorResponse::from(OperationError::rejected(
            "invalid_request",
            error.to_string(),
        ))
    })?;
    let ticket =
        backend::spawn_restart_helper(&context.state_root, Some(expected), true, &context.herdr)
            .await
            .map_err(|error| ErrorResponse::from(OperationError::from(error)))?;
    let state_root = context.state_root.clone();
    let observed = ticket.clone();
    let shutdown = shutdown.inner().clone();
    tauri::async_runtime::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
        while tokio::time::Instant::now() < deadline {
            if let Ok(Some(record)) = backend::read_restart(&state_root) {
                if record.pid == observed.pid && record.started_at == observed.started_at {
                    match (record.step, record.result) {
                        (RestartStep::Done, RestartResult::Ok) => {
                            shutdown.exit(app).await;
                            return;
                        }
                        (_, RestartResult::Failed { .. } | RestartResult::Interrupted) => return,
                        _ => {}
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    Ok(ticket)
}
