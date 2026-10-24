//! Empty-only removal of receipt-owned private attempt directories.
//! Unique non-code output is never recursively deleted or excused by acceptance.
use super::*;
use std::{io, path::Path};

fn kept(code: &str, message: impl Into<String>) -> CleanupEvaluation {
    CleanupEvaluation::Kept(reason(CleanupReasonClass::UserWork, code, message))
}

pub(super) fn inspect(base: &Path, resource: &OwnedResourceRecord, run: &Run) -> Result<CleanupEvaluation, InspectionError> {
    #[cfg(unix)]
    { unix::inspect(base, resource, run) }
    #[cfg(not(unix))]
    { let _ = (base, resource, run); Ok(kept("attempt_directory_unproven", "This platform cannot prove fd-bound private directory removal")) }
}

pub(super) fn apply(base: &Path, resource: &OwnedResourceRecord, run: &Run) -> Result<CleanupEffectOutcome, InspectionError> {
    #[cfg(unix)]
    { unix::apply(base, resource, run) }
    #[cfg(not(unix))]
    { let _ = (base, resource, run); Ok(outcome(CleanupEffectState::Refused,
        Some(reason(CleanupReasonClass::Authority, "attempt_directory_unproven", "This platform cannot prove fd-bound private directory removal")))) }
}

fn outcome(state: CleanupEffectState, reason: Option<CleanupReason>) -> CleanupEffectOutcome {
    CleanupEffectOutcome { state, reason, evidence: None, effects: Vec::new() }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use cap_fs_ext::DirExt;
    use cap_std::fs::{Dir, Metadata, MetadataExt};
    use rustix::fs::{self, AtFlags, Mode, OFlags};

    enum Opened { Absent, Present { parent: Dir, directory: Dir } }

    fn refusal(message: impl Into<String>) -> InspectionError { error("attempt_directory_unproven", message) }

    fn exact(metadata: &Metadata, identity: &OwnedAttemptDirectoryIdentity) -> bool {
        metadata.is_dir() && !metadata.file_type().is_symlink()
            && identity.device.parse::<u64>().ok() == Some(metadata.dev())
            && identity.inode.parse::<u64>().ok() == Some(metadata.ino())
    }

    fn private(directory: &Dir) -> Result<(), InspectionError> {
        let metadata = fs::fstat(directory).map_err(|why| refusal(why.to_string()))?;
        if metadata.st_uid != nix::unistd::geteuid().as_raw() || metadata.st_mode & 0o7777 != 0o700 {
            return Err(refusal("Attempt directory and its parent must retain exact private owner permissions"));
        }
        Ok(())
    }

    fn opened(base: &Path, resource: &OwnedResourceRecord, run: &Run) -> Result<Opened, InspectionError> {
        let OwnedResourceIdentity::AttemptDirectory { identity } = &resource.identity else {
            return Err(refusal("Expected a private attempt-directory receipt"));
        };
        super::super::super::tasks_md::validate_uuid(&run.run_id)?;
        super::super::super::tasks_md::validate_uuid(&identity.creation_operation_id)?;
        if !base.is_absolute() { return Err(refusal("Orchestration root must be an exact absolute path")); }
        let expected = base.join("attempts").join(&run.run_id);
        if expected.to_str() != Some(identity.root.as_str()) || resource.run_id != run.run_id || resource.root_id != run.root_id
            || resource.kind != OwnedResourceKind::AttemptDirectory || resource.ownership != ResourceOwnership::Owned
            || resource.authority != CleanupAuthority::OwnedWhenSafe
            || run.cleanup_policy != Some(WorkerCleanupPolicy::OwnedWhenSafe)
            || resource.creation_operation_id.as_deref() != Some(identity.creation_operation_id.as_str())
            || run.setup.as_ref().and_then(|setup| setup.operation_id.as_deref()) != Some(identity.creation_operation_id.as_str())
            || run.checkout.as_ref().is_none_or(|checkout| checkout.path != identity.root || checkout.git.is_some()) {
            return Err(refusal("Private attempt directory differs from its exact run, setup or checkout receipt"));
        }
        if !matches!(run.project.as_ref().map(|project| &project.source), Some(ProjectSource::Directory { root })
            if !claims::paths_overlap(root, &identity.root)) {
            return Err(refusal("Private attempt removal cannot overlap or substitute its borrowed source project"));
        }
        let parent = crate::project_store::open_dir_nofollow_absolute(&base.join("attempts"))
            .map_err(|why| refusal(format!("Private attempt parent cannot be opened without following links: {why}")))?;
        // Nofollow traversal may return an O_PATH handle on Linux. Reopen its
        // held "." (not the ambient pathname) as readable so parent fsync after
        // unlinkat is supported while retaining the exact directory binding.
        let parent = Dir::from(fs::openat(&parent, ".", OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty()).map_err(|why| refusal(why.to_string()))?);
        private(&parent)?;
        let named = match parent.symlink_metadata(Path::new(&run.run_id)) {
            Ok(named) => named,
            Err(why) if why.kind() == io::ErrorKind::NotFound => return Ok(Opened::Absent),
            Err(why) => return Err(refusal(why.to_string())),
        };
        if !exact(&named, identity) { return Err(refusal("Named attempt directory was replaced or became a symlink")); }
        let directory = parent.open_dir_nofollow(Path::new(&run.run_id)).map_err(|why| refusal(why.to_string()))?;
        private(&directory)?;
        if !exact(&directory.dir_metadata().map_err(|why| refusal(why.to_string()))?, identity) {
            return Err(refusal("Opened attempt directory no longer matches its creation incarnation"));
        }
        Ok(Opened::Present { parent, directory })
    }

    fn acceptance(run: &Run) -> Option<CleanupEvaluation> {
        if run.kind != RunKind::Worker || run.stage != RunStage::Closed || run.close_reason != Some(CloseReason::Accepted)
            || run.result.as_ref().is_none_or(|result| result.outcome != Some(ReportOutcome::Succeeded)) {
            Some(kept("attempt_unaccepted", "Only an accepted successful attempt can remove its empty private directory; superseded, cancelled and failed output is retained"))
        } else if !run.retirement.as_ref().is_some_and(|retirement| matches!(retirement.state, RetirementState::Retired { .. })
            && retirement::identity_matches(run, retirement)) {
            Some(CleanupEvaluation::Waiting(reason(CleanupReasonClass::Reconcilable, "terminal_retirement_pending", "Exact attempt pane retirement is not yet proven")))
        } else { None }
    }

    fn entries(directory: &Dir) -> Result<(usize, bool), InspectionError> {
        let mut count = 0;
        for entry in directory.entries().map_err(|why| refusal(why.to_string()))? {
            entry.map_err(|why| refusal(why.to_string()))?;
            count += 1;
            if count == 4096 { return Ok((count, true)); }
        }
        Ok((count, false))
    }

    fn outputs(count: usize, bounded: bool) -> CleanupEvaluation {
        kept("outputs_kept", format!("Private attempt directory retains {}{count} output entries. Publish or move the outputs to the Library or project; only the subsequently empty directory is eligible for removal",
            if bounded { "at least " } else { "" }))
    }

    pub(super) fn inspect(base: &Path, resource: &OwnedResourceRecord, run: &Run) -> Result<CleanupEvaluation, InspectionError> {
        if let Some(evaluation) = acceptance(run) { return Ok(evaluation); }
        match opened(base, resource, run)? {
            Opened::Absent => Ok(CleanupEvaluation::Absent),
            Opened::Present { directory, .. } => {
                let (count, bounded) = entries(&directory)?;
                Ok(if count == 0 { CleanupEvaluation::Ready } else { outputs(count, bounded) })
            }
        }
    }

    pub(super) fn apply(base: &Path, resource: &OwnedResourceRecord, run: &Run) -> Result<CleanupEffectOutcome, InspectionError> {
        if let Some(evaluation) = acceptance(run) {
            return Ok(match evaluation {
                CleanupEvaluation::Waiting(why) | CleanupEvaluation::Kept(why) | CleanupEvaluation::Unknown(why) => outcome(CleanupEffectState::Refused, Some(why)),
                _ => unreachable!("acceptance never grants a physical effect"),
            });
        }
        let Opened::Present { parent, directory } = opened(base, resource, run)? else {
            return Ok(outcome(CleanupEffectState::Absent, None));
        };
        let (count, bounded) = entries(&directory)?;
        if count != 0 {
            let CleanupEvaluation::Kept(why) = outputs(count, bounded) else { unreachable!() };
            return Ok(outcome(CleanupEffectState::Refused, Some(why)));
        }
        let OwnedResourceIdentity::AttemptDirectory { identity } = &resource.identity else { unreachable!() };
        let named = parent.symlink_metadata(Path::new(&run.run_id)).map_err(|why| refusal(why.to_string()))?;
        if !exact(&named, identity) || !exact(&directory.dir_metadata().map_err(|why| refusal(why.to_string()))?, identity) {
            return Err(refusal("Attempt directory changed immediately before the empty-directory effect"));
        }
        // unlinkat(AT_REMOVEDIR) is the only mutation. A late file prevents it;
        // neither this code nor any failure recovery traverses or deletes output.
        match fs::unlinkat(&parent, run.run_id.as_str(), AtFlags::REMOVEDIR) {
            Ok(()) => {
                fs::fsync(&parent).map_err(|why| refusal(why.to_string()))?;
                match parent.symlink_metadata(Path::new(&run.run_id)) {
                    Err(why) if why.kind() == io::ErrorKind::NotFound => Ok(outcome(CleanupEffectState::Applied, None)),
                    _ => Ok(outcome(CleanupEffectState::Unknown, Some(reason(CleanupReasonClass::Authority,
                        "attempt_directory_outcome_unknown", "Empty-directory removal returned but exact path absence is unconfirmed")))),
                }
            }
            Err(rustix::io::Errno::NOTEMPTY) | Err(rustix::io::Errno::EXIST) => {
                let (count, bounded) = entries(&directory)?;
                let CleanupEvaluation::Kept(why) = outputs(count, bounded) else { unreachable!() };
                Ok(outcome(CleanupEffectState::Refused, Some(why)))
            }
            Err(rustix::io::Errno::NOENT) => Ok(outcome(CleanupEffectState::Absent, None)),
            Err(why) => Ok(outcome(CleanupEffectState::Unknown, Some(reason(CleanupReasonClass::Authority,
                "attempt_directory_outcome_unknown", format!("Empty-directory outcome remains unconfirmed: {why}"))))),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    struct Fixture { root: std::path::PathBuf, directory: std::path::PathBuf,
        source: std::path::PathBuf, service: OrchestrationService, run: Run }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("cockpit-attempt-cleanup-{}", id()));
            let source = root.join("borrowed-source");
            std::fs::create_dir_all(&source).unwrap();
            std::fs::write(source.join("original.txt"), "borrowed source unchanged\n").unwrap();
            let project = project_identity::resolve_owned_directory(&source).unwrap().0;
            let (revision, _) = watch::channel(0);
            let service = OrchestrationService { store: OrchestrationStore::open(&root).unwrap(), revision,
                notifications: std::sync::Mutex::new(None), launch_config: Default::default() };
            let run_id = id();
            let parent = service.base().join("attempts");
            std::fs::create_dir(&parent).unwrap();
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
            let directory = parent.join(&run_id);
            std::fs::create_dir(&directory).unwrap();
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
            let metadata = std::fs::metadata(&directory).unwrap();
            let checkout = project_identity::resolve_owned_directory(&directory).unwrap().1;
            let operation = id();
            let identity = OwnedAttemptDirectoryIdentity { creation_operation_id: operation.clone(),
                root: directory.to_str().unwrap().into(), device: metadata.dev().to_string(), inode: metadata.ino().to_string() };
            let mut run = new_run(&run_id, "attempt-cleanup-tests", RunKind::Worker, "worker".into(),
                &id(), RunLineage::default(), 1);
            run.project = Some(project);
            run.cleanup_policy = Some(WorkerCleanupPolicy::OwnedWhenSafe);
            run.stage = RunStage::Closed;
            run.close_reason = Some(CloseReason::Accepted);
            run.setup = Some(SetupSummary { operation_id: Some(operation), generation: None, workspace_id: None,
                checkout_path: identity.root.clone(), repository_id: None, branch: None, base: None,
                base_commit: None, ownership: None, effects: Vec::new(), warnings: Vec::new(), project_workspace_id: None });
            let attachment = run.attachment.as_mut().unwrap();
            attachment.launch_attempt = 1;
            attachment.launch_mode = Some(AttachmentLaunchMode::DirectProcess);
            attachment.location = Some(RunLocation { endpoint_identity: "endpoint".into(), session_id: run.session_id.clone(),
                workspace_id: "space".into(), tab_id: "tab".into(), pane_id: "pane".into(), terminal_id: Some("terminal".into()),
                launch_tag: "attempt-cleanup".into(), boot_id: Some("legacy-test-boot".into()), peer_process: None, native_session_id: None });
            let mut launch = dispatch(DispatchStep::Launched);
            launch.launch_attempt = 1;
            launch.launch_tag = Some("attempt-cleanup".into());
            launch.endpoint_identity = Some("endpoint".into());
            run.dispatch = Some(launch);
            let source_output = SourceOutput::NonCode { result_message_id: "result".into(), task_revision: "accepted-scope".into(),
                artifacts: vec!["output.txt".into()], focused_checks: vec![CheckOutcome {
                    contract: "output".into(), command_or_scenario: "fixture artifact check".into(),
                    outcome: CheckStatus::Passed, evidence: None, limitation: None,
                }] };
            run.source_output = Some(source_output.clone());
            run.result = Some(Report { message_id: "result".into(), kind: ReportKind::Result,
                outcome: Some(ReportOutcome::Succeeded), summary: "Accepted non-code output".into(), plan: None,
                source_output: Some(source_output), at: now() });
            run.integration = Integration::NotApplicable { result_message_id: "result".into(), reason: "Non-code artifact".into() };
            run.retirement = retirement::retirement_for_accepted(&run, RetirementTrigger::Accept, "accepted-scope", &now());
            run.retirement.as_mut().unwrap().state = RetirementState::Retired { at: now(), terminal: TerminalOutcome::AlreadyAbsent };
            let locked = service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.runs.push(run);
            let attachment = state.runs[0].attachment.as_ref().unwrap();
            let holder = AttachmentReference { run_id, attachment_id: attachment.attachment_id.clone(), attachment_epoch: attachment.epoch };
            claims::reserve_claim(&mut state, &checkout, &holder, &id()).unwrap();
            // Physical scope/reboot proof is tested in the release observer suite.
            // This fixture starts at its trusted persisted Released boundary.
            state.checkout_claims[0].state = claims::ClaimState::Released;
            state.checkout_claims[0].holder = None;
            state.runs[0].attachment.as_mut().unwrap().state = AttachmentState::Revoked;
            let run = state.runs[0].clone();
            insert_resource(&mut state.owned_resources, &run, OwnedResourceIdentity::AttemptDirectory { identity }).unwrap();
            locked.save(&mut state).unwrap();
            drop(locked);
            Self { root, directory, source, service, run }
        }
        fn resource(&self) -> OwnedResourceRecord {
            let state = self.service.store.lock().unwrap().read().unwrap();
            let mut resource = state.owned_resources[0].clone();
            resource.consumers = OrchestrationService::consumers_unresolved(&state, &resource);
            resource
        }
    }

    #[test]
    fn noncode_acceptance_keeps_outputs_then_published_empty_directory_is_removed_without_touching_source() {
        let fixture = Fixture::new();
        std::fs::write(fixture.directory.join("output.txt"), "unique accepted artifact\n").unwrap();
        std::fs::create_dir(fixture.directory.join("nested")).unwrap();
        std::fs::write(fixture.directory.join("nested/data"), "nested output preserved\n").unwrap();
        let resource = fixture.resource();
        let evaluation = inspect(fixture.service.base(), &resource, &fixture.run).unwrap();
        assert!(matches!(&evaluation, CleanupEvaluation::Kept(why) if why.code == "outputs_kept" && why.message.contains("2 output entries")));
        fixture.service.save_evaluation(&resource, evaluation).unwrap();
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        let summary = summaries(&state, &fixture.run.session_id);
        assert_eq!(summary[0].kept_count, 1);
        assert_eq!(state.owned_resources[0].cleanup.reason.as_ref().unwrap().code, "outputs_kept");
        assert_eq!(apply(fixture.service.base(), &resource, &fixture.run).unwrap().state, CleanupEffectState::Refused);
        assert_eq!(std::fs::read_to_string(fixture.directory.join("output.txt")).unwrap(), "unique accepted artifact\n");
        assert_eq!(std::fs::read_to_string(fixture.directory.join("nested/data")).unwrap(), "nested output preserved\n");
        std::fs::rename(fixture.directory.join("output.txt"), fixture.source.join("published-output.txt")).unwrap();
        std::fs::rename(fixture.directory.join("nested"), fixture.source.join("published-nested")).unwrap();
        let resource = fixture.resource();
        assert_eq!(inspect(fixture.service.base(), &resource, &fixture.run).unwrap(), CleanupEvaluation::Ready);
        let intent = fixture.service.begin_cleanup_effect(&resource, &fixture.run, None).unwrap().unwrap();
        let result = apply(fixture.service.base(), &intent, &fixture.run).unwrap();
        assert_eq!(result.state, CleanupEffectState::Applied);
        fixture.service.finish_cleanup_effect(&intent, result).unwrap();
        assert!(!fixture.directory.exists());
        assert_eq!(std::fs::read_to_string(fixture.source.join("original.txt")).unwrap(), "borrowed source unchanged\n");
        assert_eq!(std::fs::read_to_string(fixture.source.join("published-output.txt")).unwrap(), "unique accepted artifact\n");
        assert_eq!(std::fs::read_to_string(fixture.source.join("published-nested/data")).unwrap(), "nested output preserved\n");
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(state.owned_resources[0].cleanup.state, CleanupState::Removed);
        assert_eq!(state.checkout_claims[0].state, claims::ClaimState::Released);
    }

    #[test]
    fn absent_release_supersession_cancellation_and_failed_output_never_get_empty_directory_effect() {
        let fixture = Fixture::new();
        let resource = fixture.resource();
        for close in [CloseReason::Superseded, CloseReason::Cancelled] {
            let mut run = fixture.run.clone();
            run.close_reason = Some(close);
            assert!(matches!(inspect(fixture.service.base(), &resource, &run).unwrap(),
                CleanupEvaluation::Kept(why) if why.code == "attempt_unaccepted"));
            assert_eq!(apply(fixture.service.base(), &resource, &run).unwrap().state, CleanupEffectState::Refused);
        }
        let mut failed = fixture.run.clone();
        failed.result.as_mut().unwrap().outcome = Some(ReportOutcome::Failed);
        assert!(matches!(inspect(fixture.service.base(), &resource, &failed).unwrap(), CleanupEvaluation::Kept(_)));
        {
            let locked = fixture.service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.checkout_claims[0].state = claims::ClaimState::Held;
            let attachment = fixture.run.attachment.as_ref().unwrap();
            state.checkout_claims[0].holder = Some(AttachmentReference { run_id: fixture.run.run_id.clone(),
                attachment_id: attachment.attachment_id.clone(), attachment_epoch: attachment.epoch });
            locked.save(&mut state).unwrap();
        }
        let before = std::fs::read(fixture.service.base().join("state.json")).unwrap();
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        assert!(!attempt_directory_released(&state, &fixture.run));
        assert!(fixture.service.begin_cleanup_effect(&fixture.resource(), &fixture.run, None).unwrap().is_none());
        assert_eq!(std::fs::read(fixture.service.base().join("state.json")).unwrap(), before);
        assert!(fixture.directory.is_dir());
    }

    #[test]
    fn symlink_replacement_and_borrowed_or_foreign_receipts_cannot_remove_source_or_empty_directory() {
        let fixture = Fixture::new();
        let resource = fixture.resource();
        let moved = fixture.directory.with_extension("original");
        std::fs::rename(&fixture.directory, &moved).unwrap();
        std::os::unix::fs::symlink(&fixture.source, &fixture.directory).unwrap();
        assert!(inspect(fixture.service.base(), &resource, &fixture.run).is_err());
        assert!(apply(fixture.service.base(), &resource, &fixture.run).is_err());
        assert!(moved.is_dir());
        assert_eq!(std::fs::read_to_string(fixture.source.join("original.txt")).unwrap(), "borrowed source unchanged\n");
        std::fs::remove_file(&fixture.directory).unwrap();
        std::fs::rename(&moved, &fixture.directory).unwrap();
        for foreign in 0..3 {
            let mut changed = resource.clone();
            match foreign {
                0 => changed.ownership = ResourceOwnership::Borrowed,
                1 => changed.run_id = id(),
                _ => if let OwnedResourceIdentity::AttemptDirectory { identity } = &mut changed.identity { identity.root = fixture.source.to_str().unwrap().into(); },
            }
            assert!(apply(fixture.service.base(), &changed, &fixture.run).is_err());
        }
        assert!(fixture.directory.is_dir());
    }
}
