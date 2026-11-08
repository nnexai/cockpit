use super::*;
use super::store::ThreadLinkRecord;

fn run(id: &str, session: &str, root: &str, task: Option<&str>) -> Run {
    Run {
        run_id: id.into(), session_id: session.into(), brief: String::new(),
        kind: if task.is_some() { RunKind::Worker } else { RunKind::Supervisor },
        label: id.into(), root_id: root.into(),
        parent_run_id: task.map(|_| root.to_owned()), task_id: task.map(str::to_owned),
        attempt: 1, assignment_generation: None, launch_config: Default::default(),
        task_revision_at_start: None, stage: RunStage::Working, close_reason: None,
        dispatch: None, target: None, setup: None, cleanup_policy: None,
        last_report: None, result: None, source_output: None, integration: Integration::Pending,
        annotations: Vec::new(), project: None, checkout: None, attachment: None,
        checkout_claim_id: None, resume: None, retirement: None,
        created_at: "now".into(), updated_at: "now".into(),
    }
}

fn state() -> OrchestrationState {
    OrchestrationState {
        runs: vec![
            run("root", "session-a", "root", None),
            run("worker", "session-a", "root", Some("task-worker")),
            run("sibling", "session-a", "root", Some("task-sibling")),
            run("other-root", "session-a", "other-root", None),
            run("other-worker", "session-a", "other-root", Some("other-task")),
            run("remote-root", "session-b", "remote-root", None),
            run("remote-worker", "session-b", "remote-root", Some("remote-task")),
            // A shared root identifier must not bridge sessions.
            run("remote-same-root", "session-b", "root", Some("remote-task")),
        ],
        ..OrchestrationState::default()
    }
}

fn job(index: usize, role: JobRole) -> AuthorizedJob {
    AuthorizedJob {
        run_index: index, role, assignment_generation: None,
        attachment_id: "attachment".into(), attachment_epoch: 1, checkout_claim_id: None,
    }
}

fn message(id: &str, from: ActorRef, to: &str) -> Message {
    Message {
        message_id: id.into(), to_run_id: to.into(), seq: 1, from,
        kind: MessageKind::Instruction, text: String::new(),
        images: vec![MessageImage { image_id: id.into(), mime_type: "image/png".into(), width: 1, height: 1, bytes: 1 }],
        in_reply_to: None, report: None, thread_id: None, handoff: None,
        stale: false, escalated_from: None, stage: DeliveryStage::Stored,
        created_at: "now".into(), acked_at: None,
    }
}

fn sender(id: &str) -> ActorRef { ActorRef::Run { run_id: id.into() } }

fn handoff(id: &str, task: Option<&str>, thread: Option<&str>) -> Message {
    let mut message = message(id, ActorRef::Operator, "root");
    message.thread_id = thread.map(str::to_owned);
    message.handoff = Some(HandoffRecord {
        task_id: task.map(str::to_owned), task_revision: None, attempt_run_id: None,
        binding: None, origin: OperatorOrigin::Browser,
    });
    message
}

fn link(root: &str, task: &str, thread: &str) -> ThreadLinkRecord {
    ThreadLinkRecord {
        root_id: root.into(), task_id: task.into(), thread_id: thread.into(),
        linked_by: ActorRef::Operator, at: "now".into(),
    }
}

#[test]
fn operator_is_scoped_to_recipient_session_and_non_stale_messages() {
    let mut state = state();
    state.messages = vec![
        message("local", ActorRef::Operator, "worker"),
        message("other-tree", ActorRef::Operator, "other-root"),
        message("remote", sender("worker"), "remote-worker"),
        message("missing-recipient", sender("worker"), "missing"),
    ];
    let mut stale = message("stale", ActorRef::Operator, "root");
    stale.stale = true;
    state.messages.push(stale);
    let local = ImageViewer::Operator { session_id: "session-a" };
    let remote = ImageViewer::Operator { session_id: "session-b" };
    for id in ["local", "other-tree"] { assert!(image_visible(&state, &local, id), "{id}"); }
    for id in ["remote", "missing-recipient", "stale", "unknown"] { assert!(!image_visible(&state, &local, id), "{id}"); }
    assert!(image_visible(&state, &remote, "remote"));
    assert!(!image_visible(&state, &remote, "local"));
}

#[test]
fn root_visibility_requires_sender_or_recipient_in_its_session_and_tree() {
    let mut state = state();
    state.messages = vec![
        message("incoming", sender("other-worker"), "worker"),
        message("outgoing", sender("worker"), "other-worker"),
        message("other-tree", sender("other-root"), "other-worker"),
        message("other-session", sender("remote-root"), "remote-same-root"),
        message("unknown", sender("missing"), "missing"),
    ];
    let mut stale = message("stale", sender("worker"), "root");
    stale.stale = true;
    state.messages.push(stale);
    let root = job(0, JobRole::Root);
    let viewer = ImageViewer::Job(&root);
    for id in ["incoming", "outgoing"] { assert!(image_visible(&state, &viewer, id), "{id}"); }
    for id in ["other-tree", "other-session", "unknown", "stale"] { assert!(!image_visible(&state, &viewer, id), "{id}"); }
    let missing = job(99, JobRole::Root);
    assert!(!image_visible(&state, &ImageViewer::Job(&missing), "incoming"));
}

#[test]
fn worker_direct_messages_are_visible_but_sibling_and_unrelated_messages_are_not() {
    let mut state = state();
    state.messages = vec![
        message("received", sender("root"), "worker"),
        message("sent", sender("worker"), "root"),
        message("sibling", sender("root"), "sibling"),
        message("other-tree", sender("other-root"), "other-worker"),
        message("other-session", sender("remote-root"), "remote-same-root"),
    ];
    let mut stale = message("stale-direct", sender("root"), "worker");
    stale.stale = true;
    state.messages.push(stale);
    let worker = job(1, JobRole::Worker);
    let viewer = ImageViewer::Job(&worker);
    for id in ["received", "sent"] { assert!(image_visible(&state, &viewer, id), "{id}"); }
    for id in ["sibling", "other-tree", "other-session", "stale-direct"] { assert!(!image_visible(&state, &viewer, id), "{id}"); }
}

#[test]
fn worker_handoff_grant_requires_matching_task_or_root_scoped_thread_link() {
    let mut state = state();
    state.messages = vec![
        handoff("task-grant", Some("task-worker"), Some("task-thread")),
        handoff("link-grant", None, Some("linked-thread")),
        handoff("unlinked", None, Some("unlinked-thread")),
        handoff("wrong-task", Some("task-sibling"), Some("wrong-task-thread")),
        handoff("wrong-root-link", None, Some("wrong-root-thread")),
        handoff("wrong-task-link", None, Some("wrong-task-link-thread")),
        handoff("no-thread", Some("task-worker"), None),
    ];
    let mut non_operator = handoff("non-operator", Some("task-worker"), Some("task-thread"));
    non_operator.from = sender("sibling");
    state.messages.push(non_operator);
    let mut wrong_recipient = handoff("wrong-recipient", Some("task-worker"), Some("task-thread"));
    wrong_recipient.to_run_id = "sibling".into();
    state.messages.push(wrong_recipient);
    let mut no_handoff = handoff("no-handoff", None, Some("linked-thread"));
    no_handoff.handoff = None;
    state.messages.push(no_handoff);
    let mut stale = handoff("stale-handoff", Some("task-worker"), Some("task-thread"));
    stale.stale = true;
    state.messages.push(stale);
    state.thread_links = vec![
        link("root", "task-worker", "linked-thread"),
        link("other-root", "task-worker", "wrong-root-thread"),
        link("root", "task-sibling", "wrong-task-link-thread"),
    ];
    let worker = job(1, JobRole::Worker);
    let viewer = ImageViewer::Job(&worker);
    for id in ["task-grant", "link-grant"] { assert!(image_visible(&state, &viewer, id), "{id}"); }
    for id in ["unlinked", "wrong-task", "wrong-root-link", "wrong-task-link", "no-thread", "non-operator", "wrong-recipient", "no-handoff", "stale-handoff"] {
        assert!(!image_visible(&state, &viewer, id), "{id}");
    }
    let sibling = job(2, JobRole::Worker);
    assert!(!image_visible(&state, &ImageViewer::Job(&sibling), "task-grant"));
    assert!(!image_visible(&state, &ImageViewer::Job(&sibling), "link-grant"));
}

fn caller(auth: JobAuth, session: &str, generation: Option<u64>) -> AgentCaller {
    AgentCaller {
        auth, conversation: None, assignment_generation: generation,
        endpoint_identity: "endpoint".into(), session_id: session.into(),
        workspace_id: "space".into(), tab_id: "tab".into(), pane_id: "pane".into(),
        boot_id: None, terminal_id: None, native_session_id: None,
        actual_agent_kind: None, process: None,
    }
}

#[test]
fn service_image_reads_enforce_scoped_visibility_and_current_credentials() {
    let root = std::env::temp_dir().join(format!("cockpit-image-visibility-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let service = OrchestrationService::open(&ProjectConfiguration::for_tests(&root)).unwrap();
    // Structurally complete 1x1 JPEG marker stream, as in the image-store tests.
    let bytes = vec![0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 1, 0, 1, 1, 1, 0x11, 0, 0xff, 0xda, 0, 8, 1, 1, 0, 0, 63, 0, 0x12, 0xff, 0xd9];
    let image = service.image_put("session-a", &bytes).unwrap();
    let mut state = state();
    state.runs[1].assignment_generation = Some(1);
    state.runs[2].assignment_generation = Some(1);
    let root_auth = credentials::test_job_auth(&mut state, 0, JobRole::Root);
    let worker_auth = credentials::test_job_auth(&mut state, 1, JobRole::Worker);
    let sibling_auth = credentials::test_job_auth(&mut state, 2, JobRole::Worker);
    let remote_auth = credentials::test_job_auth(&mut state, 5, JobRole::Root);
    let mut handoff = handoff("handoff", None, Some("linked-thread"));
    handoff.images = vec![image.clone()];
    state.messages.push(handoff);
    state.thread_links.push(link("root", "task-worker", "linked-thread"));
    {
        let locked = service.store.lock().unwrap();
        locked.save(&mut state).unwrap();
    }
    let root_caller = caller(root_auth, "session-a", None);
    let worker_caller = caller(worker_auth, "session-a", Some(1));
    let sibling_caller = caller(sibling_auth, "session-a", Some(1));
    let remote_caller = caller(remote_auth, "session-b", None);
    let media = service.image_for_session("session-a", &image.image_id).unwrap();
    assert_eq!(BASE64.decode(media.data_base64).unwrap(), bytes);
    assert_eq!(service.image_for_session("session-b", &image.image_id).unwrap_err().code, "image_not_found");
    for caller in [&root_caller, &worker_caller] {
        let visible = service.images_for_job(caller, "session-a", Some(&image.image_id)).unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].image, image);
        assert_eq!(std::fs::read(&visible[0].path).unwrap(), bytes);
        assert_eq!(visible[0].thread_id.as_deref(), Some("linked-thread"));
    }
    assert_eq!(service.images_for_job(&sibling_caller, "session-a", Some(&image.image_id)).unwrap_err().code, "image_not_found");
    assert_eq!(service.images_for_job(&remote_caller, "session-b", Some(&image.image_id)).unwrap_err().code, "image_not_found");
    {
        let locked = service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.credentials.iter_mut().find(|record| record.credential_id == worker_caller.auth.credential_id)
            .unwrap().state = credentials::CredentialState::Revoked;
        locked.save(&mut state).unwrap();
    }
    assert_eq!(service.images_for_job(&worker_caller, "session-a", Some(&image.image_id)).unwrap_err().code, "job_unauthorized");
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}
