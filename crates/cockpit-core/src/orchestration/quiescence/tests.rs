use super::*;
use super::super::{self as orchestration, credentials, herdr, messages, store, tasks_md};
use cockpit_protocol::orchestration::*;
use cockpit_protocol::projects::ProjectConfiguration;
use std::path::PathBuf;
use uuid::Uuid;

const SESSION: &str = "quiescence-test";
const OLD: &str = "2000-01-01T00:00:00Z";
const QUIET: Duration = Duration::from_secs(60);

struct Fixture {
    dir: PathBuf,
    service: OrchestrationService,
    root: String,
    runtime: RuntimeView,
}

fn location(id: &str) -> RunLocation {
    RunLocation {
        endpoint_identity: "endpoint".into(), session_id: SESSION.into(),
        workspace_id: "space".into(), tab_id: format!("tab-{id}"),
        pane_id: format!("pane-{id}"), terminal_id: Some(format!("terminal-{id}")),
        native_session_id: None, boot_id: None, peer_process: None, launch_tag: id.into(),
    }
}

fn attach(run: &mut Run) {
    run.attachment = Some(ExecutionAttachment {
        attachment_id: Uuid::new_v4().to_string(), epoch: 1, state: AttachmentState::Connected,
        launch_mode: None, location: Some(location(&run.run_id)), conversation: None,
        process_scope: None, process: None, shell: None, machine_id: None,
        launch_attempt: 1, updated_at: OLD.into(),
    });
    run.stage = RunStage::Active;
    run.dispatch = None;
    run.updated_at = OLD.into();
}

fn pane(id: &str, status: &str) -> herdr::RuntimePane {
    let location = location(id);
    herdr::RuntimePane {
        workspace_id: location.workspace_id, workspace_label: "Space".into(),
        tab_id: location.tab_id, tab_label: "Agent".into(), pane_id: location.pane_id,
        cwd: None, terminal_id: location.terminal_id, native_session_id: None,
        native_session_file: None, agent_name: None, agent_kind: None,
        launch_pending: false, agent_status: Some(status.into()), state_changed_at: Some(OLD.into()),
    }
}

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("cockpit-quiescence-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let service = OrchestrationService::open(&ProjectConfiguration::for_tests(&dir)).unwrap();
        let root = Uuid::new_v4().to_string();
        let mut run = orchestration::new_run(&root, SESSION, RunKind::Supervisor,
            "Supervisor".into(), &root, orchestration::RunLineage::default(), 1);
        attach(&mut run);
        let (credential, _) = credentials::stage_credential(service.base(), &run, credentials::JobRole::Root).unwrap();
        let locked = service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.runs.push(run);
        state.credentials.push(credential);
        locked.save(&mut state).unwrap();
        drop(locked);
        let runtime = RuntimeView {
            endpoint_identity: "endpoint".into(), boot_id: None, peer_process: None,
            focused_workspace_id: None, workspaces: vec![], panes: vec![pane(&root, "idle")],
        };
        let fixture = Self { dir, service, root, runtime };
        fixture.tasks(&[]);
        fixture
    }

    fn tasks(&self, ids: &[String]) {
        let contents = ids.iter().map(|id| format!("- [ ] Task {id} <!-- cockpit-task: {id} -->\n")).collect::<String>();
        self.service.store.tasks_dir().write(tasks_md::root_filename(&self.root).unwrap(), contents.as_bytes()).unwrap();
    }

    fn change(&self, f: impl FnOnce(&mut store::OrchestrationState)) {
        let locked = self.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        f(&mut state);
        locked.save(&mut state).unwrap();
    }

    fn age(&self) {
        self.change(|state| {
            for run in &mut state.runs { run.updated_at = OLD.into(); }
            for message in &mut state.messages { message.created_at = OLD.into(); }
            for operation in &mut state.operations { operation.updated_at = OLD.into(); }
        });
    }

    fn check(&self) -> Option<Duration> {
        self.service.record_quiescence(&self.runtime, SESSION, QUIET).unwrap()
    }

    fn events(&self) -> Vec<Message> {
        self.service.store.lock().unwrap().read().unwrap().messages.into_iter()
            .filter(|m| m.message_id.starts_with("reconcile:") || m.message_id.starts_with("stalled:"))
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.dir); }
}

#[test]
fn quiet_board_reconciles_once_per_key_then_stalls_once_without_a_timer() {
    let fixture = Fixture::new();
    let first = Uuid::new_v4().to_string();
    fixture.tasks(&[first.clone()]);
    assert_eq!(fixture.check(), None);
    let events = fixture.events();
    assert_eq!(events.len(), 1);
    assert!(events[0].message_id.starts_with("reconcile:"));
    assert!(messages::inbox_meaningful(&events[0]));
    assert!(events[0].text.contains("Reconcile"));
    assert!(events[0].text.contains(&first));
    // The newly appended message is activity, not an immediate stall.
    assert!(fixture.check().is_some_and(|d| !d.is_zero() && d <= QUIET));
    fixture.age();
    assert_eq!(fixture.check(), None);
    let events = fixture.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].message_id, events[0].message_id.replacen("reconcile:", "stalled:", 1));
    assert!(!messages::inbox_meaningful(&events[1]));
    fixture.age();
    assert_eq!(fixture.check(), None);
    assert_eq!(fixture.check(), None);
    assert_eq!(fixture.events().len(), 2);
    // A different semantic board gets its own reconcile, not another old-key stall.
    fixture.tasks(&[first, Uuid::new_v4().to_string()]);
    assert_eq!(fixture.check(), None);
    let events = fixture.events();
    assert_eq!(events.len(), 3);
    assert!(events[2].message_id.starts_with("reconcile:"));
    assert_ne!(events[0].message_id, events[2].message_id);
}

#[test]
fn working_root_or_subtree_pane_suppresses_reconciliation() {
    let mut fixture = Fixture::new();
    fixture.tasks(&[Uuid::new_v4().to_string()]);
    fixture.runtime.panes[0].agent_status = Some("working".into());
    assert_eq!(fixture.check(), None);
    assert!(fixture.events().is_empty());
    fixture.runtime.panes[0].agent_status = Some("idle".into());
    let worker = Uuid::new_v4().to_string();
    let mut run = orchestration::new_run(&worker, SESSION, RunKind::Worker, "Worker".into(),
        &fixture.root, orchestration::RunLineage { parent_run_id: Some(fixture.root.clone()), task_id: None }, 1);
    attach(&mut run);
    fixture.change(|state| state.runs.push(run));
    fixture.runtime.panes.push(pane(&worker, "working"));
    assert_eq!(fixture.check(), None);
    assert!(fixture.events().is_empty());
    fixture.runtime.panes[1].agent_status = Some("idle".into());
    assert_eq!(fixture.check(), None);
    assert_eq!(fixture.events().len(), 1);
}

#[test]
fn failed_dispatch_reconciles_while_pending_startup_remains_suppressed() {
    let fixture = Fixture::new();
    let task = Uuid::new_v4().to_string();
    fixture.tasks(&[task.clone()]);
    let worker = Uuid::new_v4().to_string();
    let mut run = orchestration::new_run(&worker, SESSION, RunKind::Worker, "Worker".into(),
        &fixture.root, orchestration::RunLineage { parent_run_id: Some(fixture.root.clone()), task_id: Some(task) }, 1);
    run.stage = RunStage::Queued;
    run.dispatch = Some(orchestration::dispatch(DispatchStep::Planning));
    run.updated_at = OLD.into();
    fixture.change(|state| state.runs.push(run));

    assert_eq!(fixture.check(), None);
    assert!(fixture.events().is_empty(), "pending startup must suppress reconciliation");

    fixture.change(|state| {
        let run = state.runs.iter_mut().find(|r| r.run_id == worker).unwrap();
        run.dispatch.as_mut().unwrap().step = DispatchStep::PlanFailed;
    });
    assert_eq!(fixture.check(), None);
    let events = fixture.events();
    assert_eq!(events.len(), 1);
    assert!(events[0].message_id.starts_with("reconcile:"));
    assert!(messages::inbox_meaningful(&events[0]));
}

#[test]
fn no_open_work_or_only_waiting_on_user_never_reconciles() {
    let fixture = Fixture::new();
    assert_eq!(fixture.check(), None);
    assert!(fixture.events().is_empty());
    let task = Uuid::new_v4().to_string();
    let thread = Uuid::new_v4().to_string();
    fixture.tasks(&[task.clone()]);
    fixture.change(|state| {
        state.thread_links.push(store::ThreadLinkRecord { thread_id: thread.clone(), root_id: fixture.root.clone(),
            task_id: task.clone(), linked_by: ActorRef::Operator, at: OLD.into() });
        let report = Report { card: None, message_id: "question".into(), kind: ReportKind::NeedsInput,
            outcome: None, summary: "Decision needed".into(), plan: None, source_output: None, at: OLD.into() };
        messages::append(state, messages::AppendMessage { images: vec![], thread_id: Some(thread.clone()), handoff: None,
            from: ActorRef::Run { run_id: fixture.root.clone() }, to_run_id: &fixture.root,
            message_id: "question", kind: MessageKind::Report, text: "Decision needed", in_reply_to: None,
            report: Some(report), stale: false, escalated_from: None }).unwrap();
        state.messages.last_mut().unwrap().created_at = OLD.into();
    });
    assert_eq!(fixture.check(), None);
    assert!(fixture.events().is_empty());
    // A second, actionable task must still trigger despite the waiting task.
    fixture.tasks(&[task, Uuid::new_v4().to_string()]);
    assert_eq!(fixture.check(), None);
    assert_eq!(fixture.events().len(), 1);
}

#[test]
fn recent_activity_restarts_the_quiet_clock() {
    let mut fixture = Fixture::new();
    fixture.tasks(&[Uuid::new_v4().to_string()]);
    fixture.runtime.panes[0].state_changed_at = Some(orchestration::now());
    let remaining = fixture.check().expect("recent pane activity schedules a deadline");
    assert!(!remaining.is_zero() && remaining <= QUIET);
    assert!(fixture.events().is_empty());
    fixture.runtime.panes[0].state_changed_at = Some(OLD.into());
    fixture.change(|state| state.runs[0].updated_at = orchestration::now());
    assert!(fixture.check().is_some_and(|d| !d.is_zero() && d <= QUIET));
    assert!(fixture.events().is_empty());
    fixture.age();
    assert_eq!(fixture.check(), None);
    assert_eq!(fixture.events().len(), 1);
}
