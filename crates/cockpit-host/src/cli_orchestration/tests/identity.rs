use cockpit_core::orchestration::{
    AgentCaller, CallerProvenance,
    caller::{location_matches, retirement_read_scope},
};
use cockpit_protocol::orchestration::{
    AgentKind, CloseReason, RetirementIdentity, Run, RunLocation, RunStage,
};

use super::super::caller::native_process_evidence;
use super::{accept_retirement, retirement_fixture, test_job_auth};


#[test]
fn moved_caller_requires_same_native_terminal_session_boot_and_endpoint() {
    let mut caller = AgentCaller {
        auth: test_job_auth(),
        provenance: Some(CallerProvenance {
            agent_kind: Some(AgentKind::Subagent),
            child_id: Some("child".into()),
            main_session_id: Some("main".into()),
            native_session_id: Some("child-native".into()),
            process: None,
        }),
        endpoint_identity: "endpoint".into(),
        session_id: "fixture".into(),
        workspace_id: "new-space".into(),
        tab_id: "new-tab".into(),
        pane_id: "new-pane".into(),
        boot_id: Some("boot".into()),
        terminal_id: Some("terminal".into()),
        native_session_id: Some("main".into()),
        actual_agent_kind: Some("omp".into()),
        env_run: Some(("run".into(), 1)),
        omp_session_id: Some("child-native".into()),
        main_omp_session_id: Some("main".into()),
        agent_kind: Some(AgentKind::Subagent),
        subagent_id: Some("child".into()),
        process: None,
    };
    let mut location = RunLocation {
        endpoint_identity: "endpoint".into(),
        session_id: "fixture".into(),
        workspace_id: "old-space".into(),
        tab_id: "old-tab".into(),
        pane_id: "old-pane".into(),
        launch_tag: "tag".into(),
        boot_id: Some("boot".into()),
        terminal_id: Some("terminal".into()),
        native_session_id: Some("main".into()),
    };
    assert!(location_matches(&location, Some("main"), &caller));
    // Herdr 0.9.3 omits boot/native session fields: absence is not disagreement.
    caller.boot_id = None;
    caller.native_session_id = None;
    assert!(location_matches(&location, Some("main"), &caller));
    location.boot_id = None;
    location.native_session_id = None;
    assert!(location_matches(&location, Some("main"), &caller));
    caller.main_omp_session_id = Some("foreign-main".into());
    assert!(!location_matches(&location, Some("main"), &caller));
    caller.main_omp_session_id = Some("main".into());
    caller.boot_id = Some("boot".into());
    location.boot_id = Some("boot".into());
    caller.native_session_id = Some("main".into());
    location.native_session_id = Some("main".into());
    caller.native_session_id = Some("other-session".into());
    assert!(!location_matches(&location, Some("main"), &caller));
    caller.native_session_id = Some("main".into());
    caller.boot_id = Some("restarted".into());
    assert!(!location_matches(&location, Some("main"), &caller));
    caller.boot_id = Some("boot".into());
    caller.pane_id = location.pane_id.clone();
    caller.terminal_id = Some("replacement-terminal".into());
    assert!(!location_matches(&location, Some("main"), &caller));
}

#[test]
fn retirement_scope_survives_only_own_exact_acceptance() {
    let (mut run, caller) = retirement_fixture();
    assert!(retirement_read_scope(&run, &caller).is_err(), "Working is not accepted retirement authority");
    assert!(run.retirement.is_none());
    run.stage = RunStage::Reported;
    assert!(retirement_read_scope(&run, &caller).is_err(), "Result delivery is not acceptance");
    accept_retirement(&mut run, &caller);
    retirement_read_scope(&run, &caller).unwrap();
    for reason in [
        CloseReason::Cancelled,
        CloseReason::Superseded,
        CloseReason::Failed,
    ] {
        run.close_reason = Some(reason);
        assert!(retirement_read_scope(&run, &caller).is_err());
    }
    run.close_reason = Some(CloseReason::Accepted);
    run.retirement = None;
    assert!(retirement_read_scope(&run, &caller).is_err());
}


#[test]
fn retirement_scope_rejects_foreign_or_replaced_native_authority() {
    let (mut run, caller) = retirement_fixture();
    let live = run.clone();
    accept_retirement(&mut run, &caller);
    let mutations: &[fn(&mut AgentCaller)] = &[
        |c| c.env_run = Some(("sibling".into(), 2)),
        |c| c.env_run = Some(("worker".into(), 1)),
        |c| c.env_run = None,
        |c| c.agent_kind = Some(AgentKind::Subagent),
        |c| c.subagent_id = Some("child".into()),
        |c| c.omp_session_id = Some("foreign".into()),
        |c| c.process.as_mut().unwrap().pid += 1,
        |c| c.process.as_mut().unwrap().start_ticks += 1,
        |c| c.process.as_mut().unwrap().kernel_boot_id = Some("new-kernel".into()),
        |c| c.process = None,
        |c| c.endpoint_identity = "new-endpoint".into(),
        |c| c.session_id = "other-fixture".into(),
        |c| c.workspace_id = "other-space".into(),
        |c| c.tab_id = "other-tab".into(),
        |c| c.pane_id = "other-pane".into(),
        |c| c.terminal_id = Some("other-terminal".into()),
        |c| c.boot_id = Some("new-boot".into()),
        |c| c.native_session_id = Some("other-native".into()),
        |c| c.actual_agent_kind = None,
    ];
    for mutate in mutations {
        let mut foreign = caller.clone();
        mutate(&mut foreign);
        assert!(
            retirement_read_scope(&run, &foreign).is_err(),
            "{foreign:?}"
        );
        assert!(
            retirement_read_scope(&live, &foreign).is_err(),
            "{foreign:?}"
        );
    }
    run.dispatch.as_mut().unwrap().launch_attempt += 1;
    assert!(retirement_read_scope(&run, &caller).is_err());
    run.dispatch.as_mut().unwrap().launch_attempt -= 1;
    run.dispatch.as_mut().unwrap().launch_tag = Some("new-launch".into());
    assert!(retirement_read_scope(&run, &caller).is_err());
    run.dispatch.as_mut().unwrap().launch_tag = Some("launch".into());
    run.dispatch.as_mut().unwrap().endpoint_identity = Some("new-endpoint".into());
    assert!(retirement_read_scope(&run, &caller).is_err());
    run.dispatch.as_mut().unwrap().endpoint_identity = Some("endpoint".into());
    run.retirement
        .as_mut()
        .unwrap()
        .identity
        .as_mut()
        .unwrap()
        .process
        .start_ticks += 1;
    assert!(retirement_read_scope(&run, &caller).is_err());
}

#[test]
fn accepted_retirement_read_is_fenced_to_immutable_identity_not_current_binding_alone() {
    let (mut run, caller) = retirement_fixture();
    accept_retirement(&mut run, &caller);
    let mutations: &[fn(&mut RetirementIdentity)] = &[
        |i| i.attachment_id = Some("other-attachment".into()),
        |i| i.attachment_id = None,
        |i| i.attachment_epoch = Some(i.attachment_epoch.unwrap() + 1),
        |i| i.attachment_epoch = None,
        |i| i.run_attempt += 1,
        |i| i.launch_attempt += 1,
        |i| i.launch_tag = "other-launch".into(),
        |i| i.endpoint_identity = "other-endpoint".into(),
        |i| i.session_id = "other-session".into(),
        |i| i.workspace_id = "other-space".into(),
        |i| i.tab_id = "other-tab".into(),
        |i| i.pane_id = "other-pane".into(),
        |i| i.terminal_id = "other-terminal".into(),
        |i| i.herdr_boot_id = Some("other-boot".into()),
        |i| i.omp_session_id = "other-main".into(),
        |i| i.process.start_ticks += 1,
    ];
    for mutate in mutations {
        let mut changed = run.clone();
        mutate(
            changed
                .retirement
                .as_mut()
                .unwrap()
                .identity
                .as_mut()
                .unwrap(),
        );
        assert!(retirement_read_scope(&changed, &caller).is_err());
    }
    run.retirement.as_mut().unwrap().identity = None;
    assert!(retirement_read_scope(&run, &caller).is_err());
}

#[test]
fn native_pid_evidence_rejects_self_and_unrelated_live_child() {
    assert!(native_process_evidence(std::process::id()).is_err());
    assert!(native_process_evidence(0).is_err());
    assert!(native_process_evidence(u32::MAX).is_err());
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let rejected = native_process_evidence(child.id()).is_err();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(rejected);
}
