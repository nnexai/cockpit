use super::*;
use super::store::LockedStore;

pub(super) mod tasks;
pub(super) mod runs;
pub(super) mod grants;
pub(super) mod retirement;
pub(super) mod intents;
pub(super) mod reviewed;
mod dispatch_records;

/// The mutation's existing lock and caller admission, borrowed by each handler.
/// Timestamps stay at their original call sites rather than being cached here.
pub(super) struct MutationCtx<'a> {
    pub(super) service: &'a OrchestrationService,
    pub(super) locked: &'a LockedStore<'a>,
    pub(super) state: &'a mut OrchestrationState,
    pub(super) actor: &'a Actor,
    pub(super) caller: Option<usize>,
    pub(super) session_id: &'a str,
    pub(super) reviewed: Option<&'a Run>,
}

pub(super) enum Applied {
    Machine(OrchestrationActionResult),
    Document(OrchestrationActionResult),
    Published(OrchestrationMutationResponse),
}
