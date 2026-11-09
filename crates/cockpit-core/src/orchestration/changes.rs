use std::sync::Arc;

use tokio::{sync::watch, task::JoinHandle};

use super::{error, run_index, validate_identity, OrchestrationService};
use crate::InspectionError;

#[derive(Clone, Default)]
pub(crate) struct ChangeState {
    pub generation: u64,
    pub unavailable: Option<String>,
}

pub(crate) struct OrchestrationChangeLease {
    state: watch::Receiver<ChangeState>,
    task: JoinHandle<()>,
}

impl Drop for OrchestrationChangeLease {
    fn drop(&mut self) { self.task.abort(); }
}

impl OrchestrationChangeLease {
    pub fn checkpoint(&self) -> ChangeState { self.state.borrow().clone() }
    pub fn receiver(&self) -> watch::Receiver<ChangeState> { self.state.clone() }
}

impl OrchestrationService {
    pub(crate) fn authorize_change_scope(&self, session: &str, store: &str, root: Option<&str>) -> Result<(), InspectionError> {
        validate_identity(session)?;
        if self.store_identity() != store {
            return Err(error("state_store_mismatch", "Orchestration state store changed"));
        }
        self.store.revalidate_watch_directories()?;
        if let Some(root_id) = root {
            let state = self.store.lock()?.read_shared()?;
            let root = &state.runs[run_index(&state, root_id)?];
            if root.parent_run_id.is_some() || root.root_id != root.run_id || root.session_id != session {
                return Err(error("root_not_found", "Orchestration root is not in this session"));
            }
        }
        self.store.revalidate_watch_directories()?;
        Ok(())
    }

    fn change_cursor(&self, root: Option<&str>) -> Result<(u64, String), InspectionError> {
        self.store.revalidate_watch_directories()?;
        let revision = self.store.lock()?.wait_revision()?;
        let token = match root {
            Some(root) => self.store.tasks_token_for_root(root)?,
            None => self.store.tasks_token()?,
        };
        self.store.revalidate_watch_directories()?;
        Ok((revision, token))
    }

    pub(crate) fn change_lease(self: &Arc<Self>, session: &str, store: &str, root: Option<&str>) -> Result<OrchestrationChangeLease, InspectionError> {
        self.store.revalidate_watch_directories()?;
        let mut filesystem = self.changes()?;
        let mut revision = self.subscribe();
        filesystem.borrow_and_update();
        revision.borrow_and_update();
        self.authorize_change_scope(session, store, root)?;
        if let Some(error) = self.notification_error() { return Err(error); }
        let mut cursor = self.change_cursor(root)?;
        let (sender, state) = watch::channel(ChangeState::default());
        let service = Arc::clone(self);
        let root = root.map(str::to_owned);
        let task = tokio::spawn(async move {
            loop {
                let alive = tokio::select! {
                    result = filesystem.changed() => result.is_ok(),
                    result = revision.changed() => result.is_ok(),
                };
                // Mark both sources before reading. A change during the read remains pending.
                filesystem.borrow_and_update();
                revision.borrow_and_update();
                let result = if !alive || filesystem.has_changed().is_err() || revision.has_changed().is_err() {
                    Err(error("orchestration_watch", "Orchestration watcher stopped"))
                } else if let Some(error) = service.notification_error() {
                    Err(error)
                } else {
                    service.change_cursor(root.as_deref())
                };
                match result {
                    Ok(next) if next != cursor => {
                        cursor = next;
                        if sender.borrow().generation >= cockpit_protocol::local_changes::MAX_SAFE_SEQUENCE {
                            sender.send_modify(|state| state.unavailable = Some("Orchestration change counter exhausted".into()));
                            break;
                        }
                        sender.send_modify(|state| state.generation += 1);
                    }
                    Ok(_) => {},
                    Err(error) => {
                        sender.send_modify(|state| state.unavailable = Some(error.to_string()));
                        break;
                    }
                }
            }
        });
        Ok(OrchestrationChangeLease { state, task })
    }
}
