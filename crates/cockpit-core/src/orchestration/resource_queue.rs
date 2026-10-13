//! Durable exclusion for typed build resources; no launch or shell authority.
use std::path::Path;

use super::*;
use super::claims::ClaimState;
use super::mutate::resume;
use super::store::DurableOperationKind;
use crate::{process_identity, space_git};

/// Fresh core observation bound to the semantic request and its principal.
#[derive(Debug, Clone)]
pub struct ResourceSourceFacts {
    request_digest: String,
    holder: AttachmentReference,
    checkout: CheckoutIdentity,
    resource_key: String,
}

fn job(state: &OrchestrationState, actor: &Actor) -> Result<AuthorizedJob, InspectionError> {
    let Actor::Agent(agent) = actor else {
        return Err(error("actor_forbidden", "Resource operations require a current job credential"));
    };
    credentials::validate_caller(state, agent)
}

fn holder(state: &OrchestrationState, job: &AuthorizedJob) -> AttachmentReference {
    AttachmentReference { run_id: state.runs[job.run_index].run_id.clone(),
        attachment_id: job.attachment_id.clone(), attachment_epoch: job.attachment_epoch }
}

fn fence(record: &ResourceRequestRecord, expected: &AttachmentReference,
    current: &AttachmentReference) -> Result<(), InspectionError> {
    if &record.holder != expected || expected != current {
        return Err(error("resource_holder_conflict", "Resource request belongs to a different current holder"));
    }
    Ok(())
}

pub(crate) fn authorize_request_replay(state: &OrchestrationState, actor: &Actor,
    request: &OrchestrationMutationRequest) -> Result<(), InspectionError> {
    let OrchestrationAction::ResourceRequest { request } = &request.action else { return Ok(()) };
    let job = job(state, actor)?;
    authorize_request(state, &job, request)
}

fn authorize_request(state: &OrchestrationState, job: &AuthorizedJob,
    request: &ResourceRequest) -> Result<(), InspectionError> {
    let current = holder(state, job);
    if request.holder != current {
        return Err(error("resource_holder_conflict", "Request must name the authenticated current attachment"));
    }
    if let Some(record) = state.resource_requests.iter().find(|r| r.request_id == request.operation_id) {
        fence(record, &request.holder, &current)?;
    }
    Ok(())
}

fn checkout_for(state: &OrchestrationState, job: &AuthorizedJob,
    key: &str) -> Result<CheckoutIdentity, InspectionError> {
    let run = &state.runs[job.run_index];
    let checkout = if job.role == JobRole::Root {
        state.runs.iter().filter(|candidate| candidate.root_id == run.root_id
            && candidate.session_id == run.session_id
            && (candidate.run_id == run.run_id || is_ancestor(state, &run.run_id, &candidate.run_id)))
            .filter_map(|candidate| candidate.checkout.as_ref()).find(|c| c.key == key)
    } else {
        run.checkout.as_ref().filter(|c| c.key == key)
    };
    checkout.cloned().ok_or_else(|| error("resource_source_scope", "Source checkout is outside this job's resource scope"))
}

fn canonical_resource(value: &str) -> Result<String, InspectionError> {
    let path = Path::new(value);
    if !path.is_absolute() || value.len() > 4096 || path.parent().is_none()
        || value.split('/').any(|part| matches!(part, "." | "..")) {
        return Err(error("resource_key_invalid", "Resource key must name an exact absolute non-root output directory"));
    }
    // A first build may create its output directory. Resolve its nearest
    // existing ancestor without creating anything or admitting a symlink alias.
    let mut ancestor = path;
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.is_dir() => break,
            Ok(_) => return Err(error("resource_key_invalid", "Output directory contains a non-directory or symlink alias")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                ancestor = ancestor.parent().ok_or_else(|| error("resource_key_invalid", "Output namespace is unavailable"))?;
            }
            Err(_) => return Err(error("resource_key_invalid", "Output namespace is not observable")),
        }
    }
    let mut canonical = std::fs::canonicalize(ancestor)
        .map_err(|_| error("resource_key_invalid", "Output ancestor is not observable"))?;
    if canonical != ancestor {
        return Err(error("resource_key_invalid", "Output directory must be canonical and alias-free"));
    }
    canonical.push(path.strip_prefix(ancestor)
        .map_err(|_| error("resource_key_invalid", "Output namespace changed"))?);
    canonical.into_os_string().into_string()
        .map_err(|_| error("resource_key_invalid", "Output directory must have a UTF-8 identity"))
}

fn validate_source(source: &ResourceSourceIdentity) -> Result<(), InspectionError> {
    for oid in [&source.source_commit, &source.source_tree] {
        if !matches!(oid.len(), 40 | 64)
            || !oid.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(error("resource_source_invalid", "Source commit and tree require full lowercase Git object identities"));
        }
    }
    for value in [&source.checkout_key, &source.input_configuration, &source.toolchain_fingerprint] {
        if value.trim().is_empty() || value.len() > 4096 || value.contains('\0') {
            return Err(error("resource_source_invalid", "Source input configuration and toolchain identities are required and bounded"));
        }
    }
    Ok(())
}

async fn verify_checkout_source(checkout: &CheckoutIdentity,
    source: &ResourceSourceIdentity) -> Result<(), InspectionError> {
    project_identity::revalidate_checkout(checkout)?;
    let root = Path::new(&checkout.path);
    let git = checkout.git.as_ref().ok_or_else(|| error("resource_source_invalid", "Build source requires a known Git checkout"))?;
    if space_git::git_line(root, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await? != git.common_dir
        || space_git::git_line(root, &["rev-parse", "--absolute-git-dir"]).await? != git.worktree_git_dir
        || space_git::git_line(root, &["rev-parse", "--show-toplevel"]).await? != checkout.path
        || space_git::git_line(root, &["show", "-s", "--format=%H %T", "HEAD"]).await?
            != format!("{} {}", source.source_commit, source.source_tree) {
        return Err(error("resource_source_changed", "Actual checkout commit/tree does not match the requested source"));
    }
    // Dirty input/configuration and toolchain fingerprints are reported inputs,
    // not independently discovered authority. They still fence artifact reuse.
    project_identity::revalidate_checkout(checkout)
}

fn source_checkout<'a>(state: &'a OrchestrationState,
    record: &ResourceRequestRecord) -> Option<&'a CheckoutIdentity> {
    let run = state.runs.iter().find(|r| r.run_id == record.holder.run_id)?;
    if run.parent_run_id.is_some() {
        run.checkout.as_ref().filter(|c| c.key == record.source_identity.checkout_key)
    } else {
        state.runs.iter().filter(|candidate| candidate.root_id == run.root_id
            && candidate.session_id == run.session_id
            && (candidate.run_id == run.run_id || is_ancestor(state, &run.run_id, &candidate.run_id)))
            .filter_map(|r| r.checkout.as_ref()).find(|c| c.key == record.source_identity.checkout_key)
    }
}

impl OrchestrationService {
    /// Existing bounded local Git observations, never a caller-supplied command.
    pub async fn resolve_resource_source(&self, actor: &Actor,
        request: &OrchestrationMutationRequest) -> Result<Option<ResourceSourceFacts>, InspectionError> {
        let OrchestrationAction::ResourceRequest { request: resource } = &request.action else { return Ok(None) };
        validate_source(&resource.source_identity)?;
        let (checkout, current) = {
            let state = self.store.lock()?.read()?;
            let job = job(&state, actor)?;
            if state.runs[job.run_index].session_id != request.session_id {
                return Err(error("session_mismatch", "Resource caller belongs to another session"));
            }
            let current = holder(&state, &job);
            if current != resource.holder { return Err(error("resource_holder_conflict", "Resource holder is not current")); }
            (checkout_for(&state, &job, &resource.source_identity.checkout_key)?, current)
        };
        verify_checkout_source(&checkout, &resource.source_identity).await?;
        let resource_key = canonical_resource(&resource.resource_key)?;
        Ok(Some(ResourceSourceFacts { request_digest: resume::request_digest(&request.action, &request.session_id)?,
            holder: current, checkout, resource_key }))
    }

    /// Called by the existing owner revision/periodic observation loop. Unknown
    /// evidence and an empty registration gap preserve the original holder.
    pub async fn reconcile_resource_queue(&self) -> Result<bool, InspectionError> {
        let snapshot = self.store.lock()?.read()?;
        let mut observed = std::collections::BTreeMap::new();
        for record in snapshot.resource_requests.iter().filter(|r|
            matches!(r.state, ResourceRequestState::Queued | ResourceRequestState::Blocked)
                && current_requester(&snapshot, r) && compatible(&snapshot, r) && !occupied(&snapshot, r)) {
            let checkout = source_checkout(&snapshot, record);
            if let Some(checkout) = checkout {
                if canonical_resource(&record.resource_key).is_ok()
                    && verify_checkout_source(checkout, &record.source_identity).await.is_ok() {
                    observed.insert(record.request_id.as_str(), checkout);
                }
            }
        }
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let changed = reconcile(&mut state, &mut native_running, &|state, record|
            observed.get(record.request_id.as_str()).is_some_and(|checkout|
                source_checkout(state, record) == Some(*checkout)))?;
        if changed {
            let revision = locked.save(&mut state)?;
            self.revision.send_replace(revision);
        }
        Ok(changed)
    }
}

pub(in crate::orchestration) fn apply(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let job = job(ctx.state, ctx.actor)?;
    if ctx.caller != Some(job.run_index) { return Err(error("caller_unbound", "Resource caller is not the admitted job")); }
    let current = holder(ctx.state, &job);
    match &action {
        OrchestrationAction::ResourceRequest { request } => {
            authorize_request(ctx.state, &job, request)?;
            if let Some(result) = resume::replay(ctx, &request.operation_id, DurableOperationKind::ResourceRequest, &action)? {
                let OrchestrationActionResult::Resource { request: saved, .. } = result else {
                    return Err(error("operation_conflict", "Resource operation has an incompatible receipt"));
                };
                let record = ctx.state.resource_requests.iter().find(|r| r.request_id == saved.request_id)
                    .ok_or_else(|| error("orchestration_state_invalid", "Resource receipt has no request"))?.clone();
                return Ok(Applied::Document(OrchestrationActionResult::Resource { request: record, duplicate: true }));
            }
            validate_source(&request.source_identity)?;
            let facts = ctx.resolved.and_then(|r| r.resource_source.as_ref())
                .ok_or_else(|| error("resource_source_unverified", "Resource request requires fresh core source observation"))?;
            let checkout = checkout_for(ctx.state, &job, &request.source_identity.checkout_key)?;
            if facts.request_digest != resume::request_digest(&action, ctx.session_id)?
                || facts.holder != current || facts.checkout != checkout
                || facts.resource_key != canonical_resource(&request.resource_key)? {
                return Err(error("resource_source_changed", "Source observation no longer belongs to this request or holder"));
            }
            project_identity::revalidate_checkout(&checkout)?;
            let machine = process_identity::machine_id()
                .ok_or_else(|| error("resource_process_unverified", "Local machine evidence for build tracking is unavailable"))?;
            let record = enqueue(ctx.state, request, machine)?;
            let result = OrchestrationActionResult::Resource { request: record, duplicate: false };
            let root_id = ctx.state.runs[job.run_index].root_id.clone();
            resume::record(ctx, request.operation_id.clone(), DurableOperationKind::ResourceRequest,
                &action, root_id, result.clone(), Vec::new())?;
            Ok(Applied::Machine(result))
        }
        OrchestrationAction::ResourceRelease { release } => {
            let index = request_index(ctx.state, &release.request_id)?;
            fence(&ctx.state.resource_requests[index], &release.expected_holder, &current)?;
            let duplicate = ctx.state.resource_requests[index].state == ResourceRequestState::Released;
            release_request(&mut ctx.state.resource_requests[index], &mut native_running)?;
            let result = OrchestrationActionResult::Resource { request: ctx.state.resource_requests[index].clone(), duplicate };
            Ok(if duplicate { Applied::Document(result) } else { Applied::Machine(result) })
        }
        OrchestrationAction::ResourceBuildRegister { registration } => {
            let index = request_index(ctx.state, &registration.request_id)?;
            let record = &ctx.state.resource_requests[index];
            fence(record, &registration.expected_holder, &current)?;
            if record.state != ResourceRequestState::Held {
                return Err(error("resource_not_held", "A build may register only against its granted resource"));
            }
            let run = &ctx.state.runs[job.run_index];
            let scope = run.attachment.as_ref().and_then(|a| a.process.as_ref())
                .ok_or_else(|| error("resource_process_unverified", "Managed job process scope is unavailable"))?;
            let checkout = checkout_for(ctx.state, &job, &record.source_identity.checkout_key)?;
            project_identity::revalidate_checkout(&checkout)?;
            canonical_resource(&record.resource_key)?;
            let evidence = process_identity::managed_build_identity(&registration.process, scope,
                Path::new(&checkout.path), &record.machine_id)
                .map_err(|_| error("resource_process_unverified", "Build incarnation, executable, ancestry or source cwd could not be verified"))?;
            project_identity::revalidate_checkout(&checkout)?;
            let record = &mut ctx.state.resource_requests[index];
            if record.build_processes.iter().any(|existing| existing.process == registration.process) {
                return Ok(Applied::Document(OrchestrationActionResult::Resource { request: record.clone(), duplicate: true }));
            }
            record.build_processes.push(evidence);
            record.blocked_reason = None;
            record.updated_at = now();
            Ok(Applied::Machine(OrchestrationActionResult::Resource { request: record.clone(), duplicate: false }))
        }
        _ => Err(error("actor_forbidden", "Unsupported resource operation")),
    }
}

fn request_index(state: &OrchestrationState, id: &str) -> Result<usize, InspectionError> {
    tasks_md::validate_uuid(id)?;
    state.resource_requests.iter().position(|r| r.request_id == id)
        .ok_or_else(|| error("resource_request_unknown", "Resource request does not exist"))
}

fn enqueue(state: &mut OrchestrationState, request: &ResourceRequest,
    machine_id: String) -> Result<ResourceRequestRecord, InspectionError> {
    tasks_md::validate_uuid(&request.operation_id)?;
    let sequence = state.resource_sequence.checked_add(1).filter(|s| *s <= MAX_SAFE_COUNTER)
        .ok_or_else(|| error("resource_sequence_exhausted", "JSON-safe resource request sequence is exhausted"))?;
    let record = ResourceRequestRecord { request_id: request.operation_id.clone(), sequence,
        resource_key: request.resource_key.clone(), source_identity: request.source_identity.clone(),
        holder: request.holder.clone(), state: ResourceRequestState::Queued,
        build_processes: Vec::new(), machine_id, blocked_reason: None, updated_at: now() };
    state.resource_sequence = sequence;
    state.resource_requests.push(record.clone());
    Ok(record)
}

fn overlaps(a: &str, b: &str) -> bool {
    let a = Path::new(a); let b = Path::new(b);
    a.starts_with(b) || b.starts_with(a)
}

fn occupied(state: &OrchestrationState, record: &ResourceRequestRecord) -> bool {
    state.resource_requests.iter().any(|other| other.state == ResourceRequestState::Held
        && other.machine_id == record.machine_id && overlaps(&other.resource_key, &record.resource_key))
}

fn compatible(state: &OrchestrationState, record: &ResourceRequestRecord) -> bool {
    state.resource_requests.iter().filter(|other| other.request_id != record.request_id
        && other.machine_id == record.machine_id && overlaps(&other.resource_key, &record.resource_key)
        && (other.state == ResourceRequestState::Held
            || (other.state == ResourceRequestState::Released && !other.build_processes.is_empty())))
        .all(|other| other.source_identity == record.source_identity)
}

fn current_requester(state: &OrchestrationState, record: &ResourceRequestRecord) -> bool {
    let Some(run) = state.runs.iter().find(|run| run.run_id == record.holder.run_id) else { return false };
    let Some(attachment) = &run.attachment else { return false };
    if !has_current_binding(state, run) || attachment.attachment_id != record.holder.attachment_id
        || attachment.epoch != record.holder.attachment_epoch { return false; }
    if state.runs.iter().filter(|ancestor| ancestor.run_id == run.root_id
        || is_ancestor(state, &ancestor.run_id, &run.run_id)).any(|ancestor|
            ancestor.stage == RunStage::Closed || ancestor.attachment.as_ref().is_none_or(|a|
                matches!(a.state, AttachmentState::Revoked | AttachmentState::Quiescing))) { return false; }
    run.parent_run_id.is_none() || run.checkout_claim_id.as_ref().is_some_and(|id|
            state.checkout_claims.iter().any(|c| &c.claim_id == id
                && matches!(c.state, ClaimState::Held | ClaimState::Reserved)
                && c.holder.as_ref() == Some(&record.holder)
                && run.checkout.as_ref() == Some(&c.checkout)
                && !state.checkout_claims.iter().any(|other| other.claim_id != c.claim_id
                    && other.state != ClaimState::Released
                    && claims::checkouts_conflict(&other.checkout, &c.checkout))))
}

fn native_running(record: &ResourceRequestRecord, evidence: &NativeShellIdentity) -> std::io::Result<bool> {
    // Exec does not end the incarnation or grant permission to reuse its output.
    process_identity::same_machine_process_running(&evidence.process, Some(&record.machine_id))
}

fn exited(record: &ResourceRequestRecord,
    running: &mut impl FnMut(&ResourceRequestRecord, &NativeShellIdentity) -> std::io::Result<bool>) -> bool {
    !record.build_processes.is_empty()
        && record.build_processes.iter().all(|e| matches!(running(record, e), Ok(false)))
}

fn release_request(record: &mut ResourceRequestRecord,
    running: &mut impl FnMut(&ResourceRequestRecord, &NativeShellIdentity) -> std::io::Result<bool>) -> Result<(), InspectionError> {
    if record.state == ResourceRequestState::Released { return Ok(()); }
    if record.state == ResourceRequestState::Held && !exited(record, running) {
        return Err(error("resource_build_unresolved", "Live builds, unknown evidence and unregistered grant gaps cannot be released"));
    }
    // Queued/blocked cancellation is an explicit abort before any build grant.
    record.state = ResourceRequestState::Released;
    record.blocked_reason = None;
    record.updated_at = now();
    Ok(())
}

fn reconcile(state: &mut OrchestrationState,
    running: &mut impl FnMut(&ResourceRequestRecord, &NativeShellIdentity) -> std::io::Result<bool>,
    source_current: &impl Fn(&OrchestrationState, &ResourceRequestRecord) -> bool) -> Result<bool, InspectionError> {
    let mut changed = false;
    for record in state.resource_requests.iter_mut().filter(|r| r.state == ResourceRequestState::Held) {
        let mut unknown = false;
        for process in &record.build_processes {
            unknown |= running(record, process).is_err();
        }
        let reason = if record.build_processes.is_empty() {
            Some("Build not registered; original holder remains protected")
        } else if unknown {
            Some("Registered build process evidence is unknown; original holder remains protected")
        } else { None };
        // Native exit alone cannot seal the holder's registration interval. The
        // original holder explicitly releases after awaiting build completion.
        if record.blocked_reason.as_deref() != reason {
            record.blocked_reason = reason.map(str::to_owned);
            record.updated_at = now();
            changed = true;
        }
    }
    let mut ready: Vec<usize> = state.resource_requests.iter().enumerate()
        .filter(|(_, r)| matches!(r.state, ResourceRequestState::Queued | ResourceRequestState::Blocked))
        .map(|(i, _)| i).collect();
    ready.sort_by(|&a, &b| (state.resource_requests[a].sequence, &state.resource_requests[a].request_id)
        .cmp(&(state.resource_requests[b].sequence, &state.resource_requests[b].request_id)));
    for index in ready {
        let record = &state.resource_requests[index];
        let waiting = occupied(state, record);
        let reason = if !current_requester(state, record) {
            Some("Requester attachment, conversation or assignment generation is no longer current")
        } else if !compatible(state, record) {
            Some("Output root contains incompatible checkout/source/input/toolchain artifacts; request an isolated output root")
        } else if !waiting && !source_current(state, record) {
            Some("Actual source checkout, commit/tree or output directory is no longer observable/current")
        } else { None };
        let next = if reason.is_some() { ResourceRequestState::Blocked }
            else if waiting { ResourceRequestState::Queued }
            else { ResourceRequestState::Held };
        let reason = if next == ResourceRequestState::Held {
            Some("Build not registered; original holder remains protected")
        } else { reason };
        if next == record.state && record.blocked_reason.as_deref() == reason { continue; }
        if next == ResourceRequestState::Held { notify_grant(state, index)?; }
        state.resource_requests[index].state = next;
        state.resource_requests[index].blocked_reason = reason.map(str::to_owned);
        state.resource_requests[index].updated_at = now();
        changed = true;
    }
    Ok(changed)
}

fn notify_grant(state: &mut OrchestrationState, index: usize) -> Result<(), InspectionError> {
    let record = &state.resource_requests[index];
    let run = &state.runs[run_index(state, &record.holder.run_id)?];
    // Root verification is observable through the existing revision/snapshot;
    // routine resource observations never wake the liaison root.
    if run.parent_run_id.is_none() { return Ok(()); }
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("cockpit:resource-grant:v1:{}", record.request_id));
    let mut uuid = [0; 16];
    uuid.copy_from_slice(&digest[..16]);
    let message_id = Uuid::from_bytes(uuid).to_string();
    let id = record.request_id.clone();
    let run_id = run.run_id.clone();
    let root_id = run.root_id.clone();
    let text = format!("Build resource granted: request_id={id}; output_root={}. Register the owned build process before treating this grant as running.",
        record.resource_key);
    messages::append(state, messages::AppendMessage { from: ActorRef::Run { run_id: root_id },
        to_run_id: &run_id, message_id: &message_id, kind: MessageKind::Instruction, text: &text,
        in_reply_to: None, report: None, stale: false, escalated_from: None })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, io};

    struct Fixture {
        state: OrchestrationState,
        worker: AuthorizedJob,
        actor: Actor,
    }
    impl Fixture {
        fn new() -> Self {
            let root_id = Uuid::from_u128(1).to_string();
            let worker_id = Uuid::from_u128(2).to_string();
            let mut root = new_run(&root_id, "test", RunKind::Supervisor,
                "root".into(), &root_id, None, None, 1);
            root.stage = RunStage::Active;
            root.attachment.as_mut().unwrap().conversation = Some(ConversationIdentity {
                harness: root.launch_config.harness_kind.clone(), session_id: "root-chat".into(),
            });
            let mut worker = new_run(&worker_id, "test", RunKind::Worker,
                "worker".into(), &root_id, Some(root_id.clone()), Some(id()), 1);
            worker.stage = RunStage::Working;
            worker.assignment_generation = Some(1);
            worker.attachment.as_mut().unwrap().conversation = Some(ConversationIdentity {
                harness: worker.launch_config.harness_kind.clone(), session_id: "worker-chat".into(),
            });
            let mut state = OrchestrationState { runs: vec![root, worker], ..Default::default() };
            credentials::test_job_auth(&mut state, 0, JobRole::Root);
            let auth = credentials::test_job_auth(&mut state, 1, JobRole::Worker);
            let actor = Actor::Agent(AgentCaller {
                auth, conversation: state.runs[1].attachment.as_ref().unwrap().conversation.clone().unwrap(),
                assignment_generation: Some(1),
                endpoint_identity: String::new(), session_id: String::new(),
                workspace_id: String::new(), tab_id: String::new(), pane_id: String::new(),
                boot_id: None, terminal_id: None, native_session_id: None,
                actual_agent_kind: None, process: None,
            });
            let worker = job(&state, &actor).unwrap();
            Self { state, worker, actor }
        }
        fn enqueue(&mut self, operation: u128) -> usize {
            let request = ResourceRequest { operation_id: Uuid::from_u128(operation).to_string(),
                resource_key: "/output/build".into(),
                source_identity: ResourceSourceIdentity {
                    checkout_key: self.state.runs[1].checkout.as_ref().unwrap().key.clone(),
                    source_commit: "a".repeat(40), source_tree: "b".repeat(40),
                    input_configuration: "dirty-input/config-fingerprint".into(),
                    toolchain_fingerprint: "actual-toolchain-fingerprint".into(),
                }, holder: holder(&self.state, &self.worker) };
            enqueue(&mut self.state, &request, "machine".into()).unwrap();
            self.state.resource_requests.len() - 1
        }
        fn tick(&mut self, reader: &mut FakeProcesses) -> bool {
            reconcile(&mut self.state, &mut |r, p| reader.read(r, p), &|_, _| true).unwrap()
        }
    }

    #[derive(Default)]
    struct FakeProcesses { observations: BTreeMap<u32, Option<bool>>, reads: usize }
    impl FakeProcesses {
        fn read(&mut self, _: &ResourceRequestRecord, p: &NativeShellIdentity) -> io::Result<bool> {
            self.reads += 1;
            self.observations.get(&p.process.pid).copied().flatten()
                .ok_or_else(|| io::Error::other("unknown process evidence"))
        }
    }
    fn process(pid: u32) -> NativeShellIdentity {
        NativeShellIdentity { process: NativeProcessIdentity {
            pid, start_ticks: 9, kernel_boot_id: Some("boot".into()),
        }, executable_device: "1".into(), executable_inode: "2".into(), argv_digest: "a".repeat(64) }
    }

    #[test]
    fn worker_builds_serialize_and_emit_one_notification_per_grant() {
        let mut f = Fixture::new();
        let a = f.enqueue(10); let b = f.enqueue(11);
        let mut reader = FakeProcesses::default();
        assert!(f.tick(&mut reader));
        assert_eq!(f.state.resource_requests[a].state, ResourceRequestState::Held);
        assert_eq!(f.state.resource_requests[b].state, ResourceRequestState::Queued);
        assert_eq!(f.state.messages.len(), 1);
        assert!(!f.tick(&mut reader));
        f.state.resource_requests[a].build_processes = vec![process(41), process(42)];
        f.state.resource_requests[a].blocked_reason = None;
        reader.observations.insert(41, Some(false));
        reader.observations.insert(42, Some(true));
        assert!(!f.tick(&mut reader), "one surviving registered process protects the grant");
        assert!(release_request(&mut f.state.resource_requests[a], &mut |r, p| reader.read(r, p)).is_err());
        reader.observations.insert(42, Some(false));
        assert!(!f.tick(&mut reader), "PID exit alone never completes the holder's registration interval");
        assert_eq!(f.state.resource_requests[a].state, ResourceRequestState::Held);
        assert_eq!(f.state.resource_requests[b].state, ResourceRequestState::Queued);
        release_request(&mut f.state.resource_requests[a], &mut |r, p| reader.read(r, p)).unwrap();
        assert!(f.tick(&mut reader));
        assert_eq!(f.state.resource_requests[a].state, ResourceRequestState::Released);
        assert_eq!(f.state.resource_requests[b].state, ResourceRequestState::Held);
        assert_eq!(f.state.messages.len(), 2);
        assert!(!f.tick(&mut reader));
    }

    #[test]
    fn owner_loss_unknown_process_and_unregistered_gap_never_release_or_steal() {
        let mut f = Fixture::new();
        let a = f.enqueue(10); let b = f.enqueue(11);
        let mut reader = FakeProcesses::default();
        f.tick(&mut reader);
        assert!(release_request(&mut f.state.resource_requests[a], &mut |r, p| reader.read(r, p)).is_err());
        assert_eq!(reader.reads, 0, "an empty grant is unknown, not quiescent");
        f.state.resource_requests[a].build_processes.push(process(41));
        f.state.runs[1].attachment.as_mut().unwrap().epoch += 1;
        f.tick(&mut reader);
        assert_eq!(f.state.resource_requests[a].state, ResourceRequestState::Held);
        assert_ne!(f.state.resource_requests[b].state, ResourceRequestState::Held);
        assert!(release_request(&mut f.state.resource_requests[a], &mut |r, p| reader.read(r, p)).is_err());
        reader.observations.insert(41, Some(true));
        f.tick(&mut reader);
        assert_eq!(f.state.resource_requests[a].state, ResourceRequestState::Held);
        reader.observations.insert(41, Some(false));
        f.tick(&mut reader);
        assert_eq!(f.state.resource_requests[a].state, ResourceRequestState::Held, "owner loss never authorizes implicit release");
    }

    #[test]
    fn holder_and_epoch_fences_prevent_retargeting_resource_requests() {
        let mut f = Fixture::new();
        let a = f.enqueue(10);
        let root_auth = credentials::test_job_auth(&mut f.state, 0, JobRole::Root);
        let root = credentials::validate_job_auth(&f.state, &root_auth).unwrap();
        let record = &f.state.resource_requests[a];
        let current = holder(&f.state, &f.worker);
        let other = holder(&f.state, &root);
        assert!(fence(record, &current, &other).is_err());
        assert!(fence(record, &current, &current).is_ok());
        let mut stale = current.clone(); stale.attachment_epoch += 1;
        assert!(fence(record, &stale, &current).is_err());
        let same_operation = ResourceRequest {
            operation_id: record.request_id.clone(), resource_key: record.resource_key.clone(),
            source_identity: record.source_identity.clone(), holder: current,
        };
        assert!(authorize_request(&f.state, &f.worker, &same_operation).is_ok());
        assert!(authorize_request(&f.state, &root, &same_operation).is_err());
    }

    #[test]
    fn incompatible_source_waits_for_isolated_output_and_cancel_is_before_grant_only() {
        let mut f = Fixture::new();
        let a = f.enqueue(10);
        let cancelled = f.enqueue(11);
        release_request(&mut f.state.resource_requests[cancelled], &mut |_, _| panic!("queued abort needs no process proof")).unwrap();
        let b = f.enqueue(12);
        f.state.resource_requests[b].source_identity.input_configuration = "different-dirty-input".into();
        let mut reader = FakeProcesses::default();
        f.tick(&mut reader);
        assert_eq!(f.state.resource_requests[b].state, ResourceRequestState::Blocked);
        assert!(f.state.resource_requests[b].blocked_reason.as_ref().unwrap().contains("isolated"));
        f.state.resource_requests[a].build_processes.push(process(41));
        reader.observations.insert(41, Some(false));
        release_request(&mut f.state.resource_requests[a], &mut |r, p| reader.read(r, p)).unwrap();
        f.tick(&mut reader);
        assert_eq!(f.state.resource_requests[b].state, ResourceRequestState::Blocked, "release cannot erase incompatible artifacts");
        let isolated = f.enqueue(13);
        f.state.resource_requests[isolated].resource_key = "/output/isolated".into();
        f.state.resource_requests[isolated].source_identity.toolchain_fingerprint = "new-toolchain".into();
        f.tick(&mut reader);
        assert_eq!(f.state.resource_requests[isolated].state, ResourceRequestState::Held);
    }

    #[test]
    fn fifo_ready_uses_uuid_tie_break_and_never_wakes_root() {
        let mut f = Fixture::new();
        let b = f.enqueue(20); let a = f.enqueue(10);
        f.state.resource_requests[a].sequence = f.state.resource_requests[b].sequence;
        let mut reader = FakeProcesses::default();
        f.tick(&mut reader);
        assert_eq!(f.state.resource_requests[a].state, ResourceRequestState::Held);
        assert_eq!(f.state.resource_requests[b].state, ResourceRequestState::Queued);
        let root_auth = credentials::test_job_auth(&mut f.state, 0, JobRole::Root);
        let root = credentials::validate_job_auth(&f.state, &root_auth).unwrap();
        let root_request = ResourceRequest { operation_id: Uuid::from_u128(30).to_string(),
            resource_key: "/output/verification".into(), source_identity: f.state.resource_requests[a].source_identity.clone(),
            holder: holder(&f.state, &root) };
        let index = f.state.resource_requests.len();
        enqueue(&mut f.state, &root_request, "machine".into()).unwrap();
        let before = f.state.messages.len();
        f.tick(&mut reader);
        assert_eq!(f.state.resource_requests[index].state, ResourceRequestState::Held);
        assert_eq!(f.state.messages.len(), before);
    }

    #[test]
    fn durable_restart_keeps_fifo_holder_and_one_grant_event() {
        let mut f = Fixture::new();
        let a = f.enqueue(10); let b = f.enqueue(11);
        let mut reader = FakeProcesses::default();
        f.tick(&mut reader);
        f.state.resource_requests[a].build_processes.push(process(41));
        f.tick(&mut reader);
        f.state = serde_json::from_slice(&serde_json::to_vec(&f.state).unwrap()).unwrap();
        assert!(!f.tick(&mut reader), "unknown native evidence after restart protects the holder");
        assert_eq!(f.state.messages.len(), 1);
        reader.observations.insert(41, Some(false));
        release_request(&mut f.state.resource_requests[a], &mut |r, p| reader.read(r, p)).unwrap();
        f.tick(&mut reader);
        assert_eq!(f.state.resource_requests[b].state, ResourceRequestState::Held);
        assert_eq!(f.state.messages.len(), 2);
        let c = f.enqueue(12);
        assert!(f.state.resource_requests[c].sequence > f.state.resource_requests[b].sequence);
    }

    #[test]
    fn every_source_input_and_overlapping_output_root_fences_artifact_reuse() {
        for field in 0..5 {
            let mut f = Fixture::new();
            let a = f.enqueue(10); let b = f.enqueue(11);
            let source = &mut f.state.resource_requests[b].source_identity;
            match field {
                0 => source.checkout_key.push_str("-other-checkout"),
                1 => source.source_commit = "c".repeat(40),
                2 => source.source_tree = "d".repeat(40),
                3 => source.input_configuration.push_str("-different-input"),
                _ => source.toolchain_fingerprint.push_str("-different-toolchain"),
            }
            f.state.resource_requests[b].resource_key = "/output/build/subdirectory".into();
            let mut reader = FakeProcesses::default();
            f.tick(&mut reader);
            assert_eq!(f.state.resource_requests[a].state, ResourceRequestState::Held);
            assert_eq!(f.state.resource_requests[b].state, ResourceRequestState::Blocked);
            assert!(f.state.resource_requests[b].blocked_reason.as_deref().unwrap().contains("incompatible"));
        }
    }

    #[test]
    fn stale_source_observation_cannot_grant_and_unchanged_block_does_not_tick() {
        let mut f = Fixture::new();
        let a = f.enqueue(10);
        let mut reader = FakeProcesses::default();
        assert!(reconcile(&mut f.state, &mut |r, p| reader.read(r, p), &|_, _| false).unwrap());
        assert_eq!(f.state.resource_requests[a].state, ResourceRequestState::Blocked);
        assert!(f.state.messages.is_empty());
        assert!(!reconcile(&mut f.state, &mut |r, p| reader.read(r, p), &|_, _| false).unwrap());
    }

    #[test]
    fn stale_conversation_or_generation_rejects_resource_mutations_and_replay() {
        for invalid in 0..8 {
            let mut f = Fixture::new();
            let index = f.enqueue(10);
            f.state.resource_requests[index].state = ResourceRequestState::Held;
            let record = f.state.resource_requests[index].clone();
            let request = OrchestrationMutationRequest {
                session_id: "test".into(), expected_revision: None,
                action: OrchestrationAction::ResourceRequest { request: ResourceRequest {
                    operation_id: record.request_id.clone(), resource_key: record.resource_key.clone(),
                    source_identity: record.source_identity.clone(), holder: record.holder.clone(),
                } },
            };
            assert!(authorize_request_replay(&f.state, &f.actor, &request).is_ok());
            match invalid {
                0 => f.state.task_ownership[0].generation += 1,
                1 => f.state.task_ownership[0].owner = None,
                2 => f.state.task_ownership[0].owner.as_mut().unwrap().conversation.session_id.push_str("-other"),
                3 => f.state.runs[1].assignment_generation = Some(2),
                4 => f.state.runs[1].attachment.as_mut().unwrap().conversation.as_mut().unwrap().session_id.push_str("-other"),
                5 => {
                    let Actor::Agent(caller) = &mut f.actor else { unreachable!() };
                    caller.assignment_generation = Some(2);
                }
                6 => {
                    let Actor::Agent(caller) = &mut f.actor else { unreachable!() };
                    caller.conversation.session_id.push_str("-other");
                }
                _ => {
                    let run_id = &f.state.runs[1].run_id;
                    f.state.credentials.iter_mut().find(|c| &c.run_id == run_id).unwrap().assignment_generation = Some(2);
                }
            }
            assert!(job(&f.state, &f.actor).is_err());
            assert!(authorize_request_replay(&f.state, &f.actor, &request).is_err());
            let directory = std::env::temp_dir().join(format!("cockpit-resource-admission-{}", id()));
            let service = OrchestrationService {
                store: OrchestrationStore::open(&directory).unwrap(),
                revision: watch::channel(f.state.revision).0,
                launch_config: Default::default(),
            };
            let locked = service.store.lock().unwrap();
            let actions = [
                request.action,
                OrchestrationAction::ResourceRelease { release: ResourceRelease {
                    request_id: record.request_id.clone(), expected_holder: record.holder.clone(),
                } },
                OrchestrationAction::ResourceBuildRegister { registration: ResourceBuildRegistration {
                    request_id: record.request_id.clone(), expected_holder: record.holder.clone(),
                    process: process(41).process,
                } },
            ];
            let before = serde_json::to_value(&f.state).unwrap();
            for action in actions {
                let mut ctx = MutationCtx { service: &service, locked: &locked,
                    state: &mut f.state, actor: &f.actor, caller: Some(1),
                    session_id: "test", resolved: None };
                assert!(apply(&mut ctx, action).is_err());
                assert_eq!(serde_json::to_value(&f.state).unwrap(), before);
            }
            drop(locked);
            drop(service);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn unassign_preserves_held_consumers_and_blocks_queued_grants() {
        for registered in [false, true] {
            let mut f = Fixture::new();
            let held = f.enqueue(10); let queued = f.enqueue(11);
            let mut reader = FakeProcesses::default();
            f.tick(&mut reader);
            if registered {
                f.state.resource_requests[held].build_processes.push(process(41));
                reader.observations.insert(41, Some(false));
            }
            f.state.task_ownership[0].owner = None;
            f.state.task_ownership[0].generation += 1;
            f.state.runs[1].stage = RunStage::Closed;
            for credential in f.state.credentials.iter_mut().filter(|c| c.run_id == f.state.runs[1].run_id) {
                credential.state = credentials::CredentialState::Revoked;
            }
            f.tick(&mut reader);
            assert_eq!(f.state.resource_requests[held].state, ResourceRequestState::Held,
                "logical revocation is not physical release, even after observed process exit");
            assert_eq!(f.state.resource_requests[queued].state, ResourceRequestState::Blocked);
            assert_eq!(f.state.messages.len(), 1);
            assert_eq!(f.state.checkout_claims[0].state, ClaimState::Held);
            assert!(!f.tick(&mut reader));
        }
    }
}
