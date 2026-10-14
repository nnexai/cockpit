//! Placement-only launcher: no harness or hook starts before the private latch.
use std::{ffi::OsString, process::Command};
use clap::Args;
use cockpit_protocol::orchestration::AgentLaunchReceipt;

#[derive(Debug, Args)]
pub(crate) struct AgentExecArgs {
    #[arg(long)]
    unit: Option<String>,
    #[arg(long, default_value = "Cockpit agent")]
    description: String,
    #[arg(long)]
    latch: String,
    #[arg(long)]
    nonce: String,
    #[arg(long)]
    latch_device: String,
    #[arg(long)]
    latch_inode: String,
    #[arg(long, hide = true)]
    in_scope: bool,
    #[arg(long, hide = true)]
    scope_entered: bool,
    #[arg(long, hide = true)]
    deadline_ns: Option<u64>,
    #[arg(long, hide = true)]
    restore_runtime_absent: bool,
    #[arg(long, hide = true, allow_hyphen_values = true, conflicts_with = "restore_runtime_absent")]
    restore_runtime_value: Option<OsString>,
    #[arg(last = true, required = true, num_args = 1.., allow_hyphen_values = true)]
    argv: Vec<OsString>,
}

pub(crate) async fn run(args: AgentExecArgs) -> u8 {
    if args.argv.is_empty() { return 78; }
    if !args.in_scope {
        let deadline = cockpit_core::process_scope::latch_deadline_ns();
        #[cfg(target_os = "linux")]
        let scoped = if let Some(unit) = &args.unit {
            super::contain::preconditions(unit) && super::contain::probe().await
        } else { false };
        #[cfg(not(target_os = "linux"))]
        let scoped = false;
        match waiting_command(&args, deadline, scoped) {
            Ok(command) => return super::contain::execute(command),
            Err(error) => { eprintln!("agent-exec: {error}"); return 78; }
        }
    }
    let Some(deadline) = args.deadline_ns else {
        eprintln!("agent-exec: missing launch deadline"); return 78;
    };
    if args.restore_runtime_absent || args.restore_runtime_value.is_some() {
        // Absence and non-UTF8 values cannot use systemd --setenv. Restore in
        // a PID-preserving child-only exec before opening any latch reader.
        match waiting_command(&args, deadline, false) {
            Ok(mut command) => {
                super::contain::restore_runtime_environment(&mut command, args.restore_runtime_value.as_deref());
                return super::contain::execute(command);
            }
            Err(error) => { eprintln!("agent-exec: {error}"); return 78; }
        }
    }
    let receipt = AgentLaunchReceipt { latch: args.latch, nonce: args.nonce,
        latch_device: args.latch_device, latch_inode: args.latch_inode };
    match cockpit_core::process_scope::wait_latch(&receipt, deadline, args.scope_entered).await {
        Ok(environment) => {
            let mut harness = Command::new(&args.argv[0]);
            harness.args(&args.argv[1..]).envs(environment);
            isolate_inherited_conversation(&mut harness);
            // No clearing of PATH, auth, harness configuration or capabilities.
            // Placement is overlaid and foreign conversation hints removed;
            // fresh configured harness/hook markers remain launch-owned.
            super::contain::execute(harness)
        }
        Err(error) => {
            eprintln!("agent-exec: {error}");
            if error.code == "latch_timeout" { 75 } else { 78 }
        }
    }
}

fn isolate_inherited_conversation(command: &mut Command) {
    // These are the authoritative caller.rs ConversationMetadata inputs.
    // Resume uses explicit literal argv; the new harness/hook supplies its own
    // actual conversation identity. Never mutate the parent's environment.
    command.env_remove("PI_SESSION_FILE").env_remove("COCKPIT_SESSION_ID");
}

fn waiting_command(args: &AgentExecArgs, deadline: u64, scoped: bool) -> Result<Command, std::io::Error> {
    let executable = std::env::current_exe()?;
    let original_runtime = std::env::var_os("XDG_RUNTIME_DIR");
    #[cfg(target_os = "linux")]
    let mut command = if scoped {
        let runtime = super::contain::verified_manager_runtime()?;
        let mut command = super::contain::manager_scope_command(
            args.unit.as_deref().expect("scoped launch has a unit"), &args.description,
            &runtime, original_runtime.as_deref());
        command.arg("--").arg(executable);
        command
    } else { Command::new(executable) };
    #[cfg(not(target_os = "linux"))]
    let mut command = Command::new(executable);
    command.args(["agent-exec", "--in-scope", "--latch", &args.latch, "--nonce", &args.nonce,
        "--latch-device", &args.latch_device, "--latch-inode", &args.latch_inode, "--deadline-ns"])
        .arg(deadline.to_string());
    if scoped || (args.in_scope && args.scope_entered) { command.arg("--scope-entered"); }
    if scoped && original_runtime.as_deref().is_none_or(|value| value.to_str().is_none()) {
        match original_runtime {
            Some(value) => { command.arg("--restore-runtime-value").arg(value); }
            None => { command.arg("--restore-runtime-absent"); }
        }
    }
    command.arg("--").args(&args.argv);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    struct TestCli { #[command(flatten)] args: AgentExecArgs }
    #[test]
    fn parser_and_second_phase_preserve_literal_harness_and_latch_identity() {
        let cli = TestCli::try_parse_from(["agent-exec", "--latch", "/private/latch.fifo", "--nonce", "nonce",
            "--latch-device", "1", "--latch-inode", "2", "--", "harness", "$HOME", "a b", "--unit", "$(unsafe)"]).unwrap();
        let waiting = waiting_command(&cli.args, 12345, false).unwrap();
        let args: Vec<_> = waiting.get_args().collect();
        let divider = args.iter().position(|arg| *arg == std::ffi::OsStr::new("--")).unwrap();
        assert_eq!(&args[divider + 1..], &cli.args.argv.iter().map(OsString::as_os_str).collect::<Vec<_>>());
        assert!(args.contains(&std::ffi::OsStr::new("--in-scope")));
        assert!(args.contains(&std::ffi::OsStr::new("12345")));
        assert!(!args.contains(&std::ffi::OsStr::new("--scope-entered")));
        assert!(TestCli::try_parse_from(["agent-exec", "--latch", "path", "--nonce", "n", "--", "harness"]).is_err());
        #[cfg(target_os = "linux")]
        if crate::contain::verified_manager_runtime().is_ok() {
            let mut scoped_args = cli.args;
            scoped_args.unit = Some("cockpit-w-01234567-89abcdef-e1-0123456789abcdef0123456789abcdef.scope".into());
            let scoped = waiting_command(&scoped_args, 12345, true).unwrap();
            let args: Vec<_> = scoped.get_args().collect();
            assert!(args.contains(&std::ffi::OsStr::new("--scope-entered")));
            assert!(args.contains(&std::ffi::OsStr::new("--expand-environment=no")));
        }
    }

    #[test]
    fn identity_isolation_removes_only_foreign_conversation_metadata() {
        let mut command = Command::new("harness");
        for (key, value) in [
            ("PI_SESSION_FILE", "/parent/session.jsonl"),
            ("COCKPIT_SESSION_ID", "parent-conversation"),
            ("COCKPIT_HARNESS", "configured-child"),
            ("COCKPIT_JOB_CREDENTIAL_FILE", "/private/child.cap"),
            ("COCKPIT_CONFIG_PATH", "/configured/project"),
            ("HERDR_PANE_ID", "actual-child-pane"),
            ("COCKPIT_INBOX_HOOK", "0"),
            ("PATH", "/literal/bin"),
        ] { command.env(key, value); }
        isolate_inherited_conversation(&mut command);
        let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
        for key in ["PI_SESSION_FILE", "COCKPIT_SESSION_ID"] {
            assert_eq!(env.get(std::ffi::OsStr::new(key)), Some(&None));
        }
        for key in ["COCKPIT_HARNESS", "COCKPIT_JOB_CREDENTIAL_FILE", "COCKPIT_CONFIG_PATH", "HERDR_PANE_ID", "COCKPIT_INBOX_HOOK", "PATH"] {
            assert!(env.get(std::ffi::OsStr::new(key)).is_some_and(|value| value.is_some()));
        }
    }
}
