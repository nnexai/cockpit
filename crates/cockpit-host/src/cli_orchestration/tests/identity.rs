use cockpit_core::orchestration::{
    AgentCaller,
    caller::{location_matches, retirement_read_scope},
};
use cockpit_protocol::orchestration::{
    AgentKind, CloseReason, DispatchStep, RetirementIdentity, Run, RunKind, RunLocation, RunStage,
};

use super::super::caller::{native_process_evidence, parse_env_run};
use super::{accept_retirement, retirement_fixture};

#[test]
fn inherited_attempt_requires_complete_positive_identity() {
    assert!(parse_env_run(None, None).unwrap().is_none());
    assert!(parse_env_run(Some("run".into()), None).is_err());
    assert!(parse_env_run(None, Some("1".into())).is_err());
    assert!(parse_env_run(Some("run".into()), Some("0".into())).is_err());
    assert!(parse_env_run(Some("run".into()), Some("4294967296".into())).is_err());
    assert_eq!(
        parse_env_run(Some("run".into()), Some("2".into())).unwrap(),
        Some(("run".into(), 2))
    );
}

#[test]
fn moved_caller_requires_same_native_terminal_session_boot_and_endpoint() {
    let mut caller = AgentCaller {
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
    retirement_read_scope(&run, &caller).unwrap();
    assert!(run.retirement.is_none());
    run.stage = RunStage::Reported;
    retirement_read_scope(&run, &caller).unwrap();
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
fn retirement_scope_retries_only_exact_unattested_startup() {
    let (mut run, mut caller) = retirement_fixture();
    run.stage = RunStage::Preparing;
    run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchPending;
    run.dispatch.as_mut().unwrap().agent_started = false;
    caller.actual_agent_kind = None;
    for kind in [RunKind::Supervisor, RunKind::Worker] {
        run.kind = kind;
        for step in [
            DispatchStep::LaunchIntent,
            DispatchStep::LaunchPending,
            DispatchStep::LaunchUnknown,
        ] {
            run.dispatch.as_mut().unwrap().step = step;
            assert_eq!(
                retirement_read_scope(&run, &caller).unwrap_err().code,
                "caller_not_ready"
            );
        }
    }
    let changed_callers: &[(&str, fn(&mut AgentCaller))] = &[
        ("caller_mismatch", |c| {
            c.env_run = Some(("replacement".into(), 2))
        }),
        ("attempt_stale", |c| c.env_run.as_mut().unwrap().1 += 1),
        ("caller_mismatch", |c| {
            c.endpoint_identity = "replacement".into()
        }),
        ("caller_mismatch", |c| c.pane_id = "replacement".into()),
        ("caller_mismatch", |c| {
            c.terminal_id = Some("replacement".into())
        }),
        ("caller_mismatch", |c| {
            c.native_session_id = Some("replacement".into())
        }),
        ("caller_mismatch", |c| c.process.as_mut().unwrap().pid += 1),
        ("caller_mismatch", |c| {
            c.process.as_mut().unwrap().start_ticks += 1
        }),
        ("caller_mismatch", |c| {
            c.process.as_mut().unwrap().kernel_boot_id = Some("replacement".into())
        }),
        ("session_mismatch", |c| {
            c.omp_session_id = Some("replacement".into())
        }),
        ("caller_mismatch", |c| {
            c.actual_agent_kind = Some("other".into())
        }),
    ];
    for (code, change) in changed_callers {
        let mut changed = caller.clone();
        change(&mut changed);
        assert_eq!(
            retirement_read_scope(&run, &changed).unwrap_err().code,
            *code,
            "{changed:?}"
        );
    }
    let changed_runs: &[(&str, fn(&mut Run))] = &[
        ("session_mismatch", |r| r.bound_omp_session = None),
        ("caller_mismatch", |r| {
            r.bound_omp_process.as_mut().unwrap().start_ticks += 1
        }),
        ("caller_mismatch", |r| {
            r.dispatch.as_mut().unwrap().endpoint_identity = Some("replacement".into())
        }),
        ("caller_mismatch", |r| {
            r.dispatch.as_mut().unwrap().launch_tag = Some("replacement".into())
        }),
        ("caller_mismatch", |r| {
            r.dispatch.as_mut().unwrap().agent_started = true
        }),
        ("caller_mismatch", |r| {
            r.dispatch.as_mut().unwrap().step = DispatchStep::Launched
        }),
        ("caller_mismatch", |r| r.dispatch = None),
        ("caller_mismatch", |r| r.kind = RunKind::Adopted),
        ("caller_mismatch", |r| r.stage = RunStage::Initializing),
        ("caller_mismatch", |r| r.stage = RunStage::Active),
        ("caller_mismatch", |r| r.stage = RunStage::Working),
        ("caller_mismatch", |r| r.stage = RunStage::Reported),
    ];
    for (code, change) in changed_runs {
        let mut changed = run.clone();
        change(&mut changed);
        assert_eq!(
            retirement_read_scope(&changed, &caller).unwrap_err().code,
            *code,
            "{changed:?}"
        );
    }
    caller.actual_agent_kind = Some("omp".into());
    retirement_read_scope(&run, &caller).unwrap();
    let mut accepted = run.clone();
    accept_retirement(&mut accepted, &caller);
    caller.actual_agent_kind = None;
    assert_eq!(
        retirement_read_scope(&accepted, &caller).unwrap_err().code,
        "caller_mismatch"
    );
    run.retirement = accepted.retirement;
    assert_eq!(
        retirement_read_scope(&run, &caller).unwrap_err().code,
        "caller_mismatch"
    );
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
