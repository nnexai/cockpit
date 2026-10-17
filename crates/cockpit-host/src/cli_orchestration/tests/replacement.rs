use super::*;
use clap::{Parser, Subcommand};
use serde_json::{Value, json};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Task(super::super::args::TaskArgs),
    Run(super::super::args::RunArgs),
}

struct Fixture {
    directory: PathBuf,
    config: PathBuf,
    root: String,
    task: String,
    worker: String,
    state: Value,
}

impl Fixture {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("cockpit-cli-replacement-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(directory.join("state/orchestration/tasks")).unwrap();
        std::fs::create_dir(directory.join("catalog")).unwrap();
        std::fs::create_dir(directory.join("source")).unwrap();
        std::fs::write(directory.join("source/draft.txt"), "Retain old work\n").unwrap();
        let (project, _) = cockpit_core::orchestration::project_identity::resolve_owned_directory(
            &directory.join("source"),
        )
        .unwrap();
        let config = directory.join("config.toml");
        let path = directory.to_str().unwrap();
        std::fs::write(&config, format!(
            "repository_roots = ['{path}/catalog']\nstate_root = '{path}/state'\ncache_root = '{path}/cache'\nworktree_root = '{path}/worktrees'\nlibrary_root = '{path}/library'\nnotes_root = '{path}/notes'\n"
        )).unwrap();
        let root = uuid::Uuid::new_v4().to_string();
        let worker = uuid::Uuid::new_v4().to_string();
        let task = uuid::Uuid::new_v4().to_string();
        std::fs::write(
            directory.join(format!("state/orchestration/tasks/{root}.md")),
            format!("# Tasks\n\n- [ ] Replace pending attempt <!-- cockpit-task: {task} -->\n"),
        )
        .unwrap();
        let run = |id: &str, kind: &str, stage: &str| {
            json!({
                "session_id":"replacement-fixture", "brief":"Preserve prior work", "run_id":id,
                "root_id":root, "kind":kind, "stage":stage, "attempt":1, "label":kind,
                "launch_config":{"harness_kind":"other","command":"/definitely/absent/agent"},
                "annotations":[], "created_at":"2026-10-17T00:00:00Z", "updated_at":"2026-10-17T00:00:00Z"
            })
        };
        let mut pending = run(&worker, "worker", "starting");
        pending["task_id"] = json!(task);
        pending["parent_run_id"] = json!(root);
        pending["assignment_generation"] = json!(1);
        pending["project"] = json!(project);
        pending["dispatch"] = json!({"step":"launch_unknown", "agent_started":true, "launch_attempt":1,
            "updated_at":"2026-10-17T00:00:00Z"});
        pending["attachment"] = json!({"attachment_id":uuid::Uuid::new_v4().to_string(), "epoch":1,
            "state":"connecting", "launch_attempt":1, "updated_at":"2026-10-17T00:00:00Z"});
        pending["attachment"] = serde_json::to_value(
            serde_json::from_value::<cockpit_protocol::orchestration::ExecutionAttachment>(
                pending["attachment"].clone(),
            )
            .unwrap(),
        )
        .unwrap();
        let state = json!({"schema":4,"revision":1,"runs":[run(&root,"supervisor","active"),pending],
            "messages":[], "inbox_hints":[], "task_intents":[], "assignment_intents":[],
            "scope_intents":[], "credentials":[], "checkout_claims":[], "operations":[],
            "task_ownership":[{"root_id":root,"task_id":task,"generation":0,"owner":null}]});
        let fixture = Self {
            directory,
            config,
            root,
            task,
            worker,
            state,
        };
        fixture.save();
        fixture
    }

    fn state_path(&self) -> PathBuf {
        self.directory.join("state/orchestration/state.json")
    }
    fn save(&self) {
        std::fs::write(self.state_path(), serde_json::to_vec(&self.state).unwrap()).unwrap();
    }
    fn read(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.state_path()).unwrap()).unwrap()
    }
    fn cli(&self, command: &[&str]) -> Cli {
        Cli::try_parse_from(["test"].into_iter().chain(command.iter().copied()).chain([
            "--operator",
            "--root",
            &self.root,
            "--config",
            self.config.to_str().unwrap(),
            "--herdr-session",
            "replacement-fixture",
            "--herdr",
            "/definitely/absent/herdr",
            "--json",
        ]))
        .unwrap()
    }
    async fn context(&self) -> Context {
        let Command::Task(args) = self.cli(&["task", "show", &self.task]).command else {
            unreachable!()
        };
        Context::open(&args.common, true).await.unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

#[tokio::test]
#[ignore = "private environment child launched by replacement_cli_boundary"]
async fn replacement_cli_pending_and_active_attempts_preserve_old_attachment_and_work() {
    for active in [false, true] {
        for named_run in [false, true] {
            let mut fixture = Fixture::new();
            if active {
                fixture.state["task_ownership"][0]["generation"] = json!(1);
                fixture.state["task_ownership"][0]["owner"] = json!({"run_id":fixture.worker,
                    "claim_operation_id":uuid::Uuid::new_v4().to_string()});
                fixture.state["runs"][1]["stage"] = json!("working");
                fixture.state["runs"][1]["attachment"]["state"] = json!("connected");
                fixture.save();
            }
            let command = if named_run {
                vec![
                    "run",
                    "worker-replace",
                    &fixture.worker,
                    "--reason",
                    "Continue safely",
                ]
            } else {
                vec![
                    "task",
                    "restart",
                    &fixture.task,
                    "--reason",
                    "Continue safely",
                ]
            };
            match fixture.cli(&command).command {
                Command::Task(args) => Box::pin(args.run()).await.unwrap(),
                Command::Run(args) => Box::pin(args.run()).await.unwrap(),
            }
            let after = fixture.read();
            assert_eq!(after["runs"][1]["stage"], "closed");
            assert_eq!(after["runs"][1]["close_reason"], "superseded");
            assert_eq!(
                after["runs"][1]["attachment"],
                fixture.state["runs"][1]["attachment"]
            );
            let successor = after["runs"].as_array().unwrap().last().unwrap();
            assert_eq!(successor["assignment_generation"], 3);
            assert_eq!(successor["target"]["target"], "owned_attempt_directory");
            assert_eq!(
                successor["target"]["source_root"],
                fixture.directory.join("source").to_str().unwrap()
            );
            assert_eq!(
                std::fs::read_to_string(fixture.directory.join("source/draft.txt")).unwrap(),
                "Retain old work\n"
            );
            // A successor is pending under a nonzero vacant generation too.
            let successor_id = successor["run_id"].as_str().unwrap();
            let Command::Run(args) = fixture
                .cli(&[
                    "run",
                    "worker-replace",
                    successor_id,
                    "--reason",
                    "Replace the next unactivated attempt",
                ])
                .command
            else {
                unreachable!()
            };
            Box::pin(args.run()).await.unwrap();
            let again = fixture.read();
            assert_eq!(again["runs"][2]["close_reason"], "superseded");
            assert_eq!(again["runs"][3]["assignment_generation"], 5);
        }
    }
}

#[tokio::test]
#[ignore = "private environment child launched by replacement_cli_boundary"]
async fn replacement_selector_rejects_stale_closed_ambiguous_and_foreign_attempts_without_writes() {
    for case in [
        "stale",
        "closed",
        "multiple",
        "foreign_root",
        "foreign_task",
        "owner_mismatch",
        "owner_with_pending_sibling",
        "vacant_current",
        "exhausted",
    ] {
        let mut fixture = Fixture::new();
        match case {
            "stale" => fixture.state["runs"][1]["assignment_generation"] = json!(2),
            "closed" => fixture.state["runs"][1]["stage"] = json!("closed"),
            "multiple" => {
                let mut other = fixture.state["runs"][1].clone();
                other["run_id"] = json!(uuid::Uuid::new_v4().to_string());
                fixture.state["runs"].as_array_mut().unwrap().push(other);
            }
            "foreign_root" => {
                fixture.state["runs"][1]["root_id"] = json!(uuid::Uuid::new_v4().to_string())
            }
            "foreign_task" => {
                fixture.state["runs"][1]["task_id"] = json!(uuid::Uuid::new_v4().to_string())
            }
            "owner_mismatch" => {
                fixture.state["task_ownership"][0]["generation"] = json!(1);
                fixture.state["task_ownership"][0]["owner"] = json!({"run_id":uuid::Uuid::new_v4().to_string(),
                    "claim_operation_id":uuid::Uuid::new_v4().to_string()});
            }
            "owner_with_pending_sibling" => {
                fixture.state["task_ownership"][0]["generation"] = json!(1);
                fixture.state["task_ownership"][0]["owner"] = json!({"run_id":fixture.worker,
                    "claim_operation_id":uuid::Uuid::new_v4().to_string()});
                let mut pending = fixture.state["runs"][1].clone();
                pending["run_id"] = json!(uuid::Uuid::new_v4().to_string());
                pending["assignment_generation"] = json!(2);
                fixture.state["runs"].as_array_mut().unwrap().push(pending);
            }
            "vacant_current" => fixture.state["task_ownership"][0]["generation"] = json!(1),
            "exhausted" => {
                fixture.state["task_ownership"][0]["generation"] = json!(9_007_199_254_740_991_u64)
            }
            _ => unreachable!(),
        }
        fixture.save();
        let before = std::fs::read(fixture.state_path()).unwrap();
        let context = fixture.context().await;
        for selected in [None, Some(fixture.worker.as_str())] {
            let error = context
                .replacement_action(
                    &fixture.root,
                    &fixture.task,
                    selected,
                    None,
                    "Continue safely".into(),
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, "assignment_conflict", "{case}, {selected:?}");
            assert_eq!(std::fs::read(fixture.state_path()).unwrap(), before);
        }
    }
}

#[tokio::test]
#[ignore = "private environment child launched by replacement_cli_boundary"]
async fn replacement_selector_forwards_pending_generation_and_rejects_selected_old_run() {
    let fixture = Fixture::new();
    let context = fixture.context().await;
    for selected in [None, Some(fixture.worker.as_str())] {
        let action = context
            .replacement_action(
                &fixture.root,
                &fixture.task,
                selected,
                None,
                "Continue safely".into(),
            )
            .await
            .unwrap();
        assert!(matches!(action, OrchestrationAction::WorkerReplace {
            expected_run_id, expected_assignment_generation: 1, ..
        } if expected_run_id == fixture.worker));
    }
    assert_eq!(
        context
            .replacement_action(
                &fixture.root,
                &fixture.task,
                Some("old-run"),
                None,
                "Continue safely".into()
            )
            .await
            .unwrap_err()
            .code,
        "assignment_conflict"
    );
}

#[test]
fn replacement_cli_boundary() {
    for name in [
        "replacement_cli_pending_and_active_attempts_preserve_old_attachment_and_work",
        "replacement_selector_rejects_stale_closed_ambiguous_and_foreign_attempts_without_writes",
        "replacement_selector_forwards_pending_generation_and_rejects_selected_old_run",
    ] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("cli_orchestration::context::replacement_tests::{name}"),
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", std::env::temp_dir())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
