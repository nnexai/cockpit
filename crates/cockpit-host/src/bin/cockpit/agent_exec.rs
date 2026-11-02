//! Placement-only launcher: no harness or hook starts before the private latch.
use std::{collections::BTreeMap, ffi::OsString, process::Command};
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
    require_checkout: bool,
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
            match harness_command(&args.argv, environment, args.require_checkout) {
                Ok(harness) => super::contain::execute(harness),
                Err(error) => { eprintln!("agent-exec: {error}"); 78 }
            }
        }
        Err(error) => {
            eprintln!("agent-exec: {error}");
            if error.code == "latch_timeout" { 75 } else { 78 }
        }
    }
}

const CHECKOUT_DEVICE: &str = "COCKPIT_LAUNCH_CHECKOUT_DEVICE";
const CHECKOUT_INODE: &str = "COCKPIT_LAUNCH_CHECKOUT_INODE";
const CHECKOUT_PATH: &str = "COCKPIT_LAUNCH_CHECKOUT_PATH";

fn harness_command(argv: &[OsString], mut environment: BTreeMap<String, String>, require_checkout: bool) -> Result<Command, String> {
    let device = environment.remove(CHECKOUT_DEVICE);
    let inode = environment.remove(CHECKOUT_INODE);
    let path = environment.remove(CHECKOUT_PATH);
    let fenced = match (device, inode, path) {
        (None, None, None) if !require_checkout => false,
        (Some(device), Some(inode), Some(path)) => {
            let device = device.parse::<u64>().map_err(|_| "invalid checkout device")?;
            let inode = inode.parse::<u64>().map_err(|_| "invalid checkout inode")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let cwd = std::fs::metadata(".").map_err(|error| format!("cannot stat checkout cwd: {error}"))?;
                if cwd.dev() != device || cwd.ino() != inode {
                    return Err("checkout cwd identity mismatch".into());
                }
                let checkout = std::fs::metadata(&path).map_err(|error| format!("cannot stat checkout path: {error}"))?;
                if checkout.dev() != device || checkout.ino() != inode {
                    return Err("checkout path identity mismatch".into());
                }
                true
            }
            #[cfg(not(unix))]
            {
                let _ = (device, inode, path);
                return Err("checkout cwd identity verification requires Unix".into());
            }
        }
        _ => return Err("incomplete checkout cwd identity".into()),
    };

    let mut harness = Command::new(&argv[0]);
    harness.args(&argv[1..]).envs(environment);
    if fenced {
        // Do not leak inherited values of these launch-only keys either.
        harness.env_remove(CHECKOUT_DEVICE).env_remove(CHECKOUT_INODE).env_remove(CHECKOUT_PATH);
    }
    isolate_inherited_conversation(&mut harness);
    // No clearing of PATH, auth, harness configuration or capabilities.
    // Placement is overlaid and foreign conversation hints removed;
    // fresh configured harness/hook markers remain launch-owned.
    Ok(harness)
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
    if args.require_checkout { command.arg("--require-checkout"); }
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
            "--latch-device", "1", "--latch-inode", "2", "--require-checkout", "--", "harness", "$HOME", "a b", "--unit", "$(unsafe)"]).unwrap();
        let waiting = waiting_command(&cli.args, 12345, false).unwrap();
        let args: Vec<_> = waiting.get_args().collect();
        let divider = args.iter().position(|arg| *arg == std::ffi::OsStr::new("--")).unwrap();
        assert_eq!(&args[divider + 1..], &cli.args.argv.iter().map(OsString::as_os_str).collect::<Vec<_>>());
        assert!(args.contains(&std::ffi::OsStr::new("--in-scope")));
        assert!(args.contains(&std::ffi::OsStr::new("12345")));
        assert!(args.contains(&std::ffi::OsStr::new("--require-checkout")));
        assert!(!args.contains(&std::ffi::OsStr::new("--scope-entered")));
        assert!(TestCli::try_parse_from(["agent-exec", "--latch", "path", "--nonce", "n", "--", "harness"]).is_err());
        let ordinary = TestCli::try_parse_from(["agent-exec", "--latch", "path", "--nonce", "n",
            "--latch-device", "1", "--latch-inode", "2", "--", "harness"]).unwrap();
        assert!(!waiting_command(&ordinary.args, 12345, false).unwrap().get_args()
            .any(|arg| arg == std::ffi::OsStr::new("--require-checkout")));
        #[cfg(target_os = "linux")]
        if crate::contain::verified_manager_runtime().is_ok() {
            let mut scoped_args = cli.args;
            scoped_args.unit = Some("cockpit-w-01234567-89abcdef-e1-0123456789abcdef0123456789abcdef.scope".into());
            let scoped = waiting_command(&scoped_args, 12345, true).unwrap();
            let args: Vec<_> = scoped.get_args().collect();
            assert!(args.contains(&std::ffi::OsStr::new("--scope-entered")));
            assert!(args.contains(&std::ffi::OsStr::new("--require-checkout")));
            assert!(args.contains(&std::ffi::OsStr::new("--expand-environment=no")));
        }
    }

    #[cfg(unix)]
    #[test]
    fn checkout_fence_allows_only_matching_cwd_and_strips_launch_keys() {
        use std::os::unix::fs::MetadataExt;
        let cwd = std::fs::metadata(".").unwrap();
        let argv = ["sh", "-c",
            r#"test "${COCKPIT_LAUNCH_CHECKOUT_DEVICE+set}" != set && test "${COCKPIT_LAUNCH_CHECKOUT_INODE+set}" != set && test "${COCKPIT_LAUNCH_CHECKOUT_PATH+set}" != set && test "$COCKPIT_WORKSPACE_ID" = workspace"#]
            .into_iter().map(OsString::from).collect::<Vec<_>>();
        let environment = BTreeMap::from([
            (CHECKOUT_DEVICE.into(), cwd.dev().to_string()),
            (CHECKOUT_INODE.into(), cwd.ino().to_string()),
            (CHECKOUT_PATH.into(), ".".into()),
            ("COCKPIT_WORKSPACE_ID".into(), "workspace".into()),
        ]);
        let mut command = harness_command(&argv, environment, true).unwrap();
        let env: BTreeMap<_, _> = command.get_envs().collect();
        for key in [CHECKOUT_DEVICE, CHECKOUT_INODE, CHECKOUT_PATH] {
            assert_eq!(env.get(std::ffi::OsStr::new(key)), Some(&None));
        }
        assert!(command.status().unwrap().success());
    }

    #[cfg(unix)]
    #[test]
    fn checkout_fence_rejects_missing_malformed_and_mismatched_identity() {
        use std::os::unix::fs::MetadataExt;
        let cwd = std::fs::metadata(".").unwrap();
        let device = cwd.dev().to_string();
        let inode = cwd.ino().to_string();
        let argv = [OsString::from("harness")];
        let env = |device: Option<&str>, inode: Option<&str>, path: Option<&str>| {
            let mut env = BTreeMap::new();
            if let Some(value) = device { env.insert(CHECKOUT_DEVICE.into(), value.into()); }
            if let Some(value) = inode { env.insert(CHECKOUT_INODE.into(), value.into()); }
            if let Some(value) = path { env.insert(CHECKOUT_PATH.into(), value.into()); }
            env
        };
        for mask in 0..7 {
            let partial = env(
                (mask & 1 != 0).then_some(device.as_str()),
                (mask & 2 != 0).then_some(inode.as_str()),
                (mask & 4 != 0).then_some("."),
            );
            assert!(harness_command(&argv, partial.clone(), true).is_err());
            if mask != 0 { assert!(harness_command(&argv, partial, false).is_err()); }
        }
        assert!(harness_command(&argv, env(Some("bad"), Some(&inode), Some(".")), true).is_err());
        assert!(harness_command(&argv, env(Some(&device), Some("18446744073709551616"), Some(".")), true).is_err());
        assert!(harness_command(&argv, env(Some(&(cwd.dev() ^ 1).to_string()), Some(&inode), Some(".")), true).is_err());
        assert!(harness_command(&argv, env(Some(&device), Some(&(cwd.ino() ^ 1).to_string()), Some(".")), true).is_err());
        assert!(harness_command(&argv, env(Some(&device), Some(&inode), Some("")), true).is_err());

        let command = harness_command(&argv, env(None, None, None), false).unwrap();
        let env: BTreeMap<_, _> = command.get_envs().collect();
        for key in [CHECKOUT_DEVICE, CHECKOUT_INODE, CHECKOUT_PATH] {
            assert!(!env.contains_key(std::ffi::OsStr::new(key)));
        }
    }

    #[cfg(unix)]
    #[test]
    fn checkout_fence_rejects_replaced_or_removed_path() {
        use std::{os::unix::fs::{symlink, MetadataExt}, time::{SystemTime, UNIX_EPOCH}};
        struct TempDir(std::path::PathBuf);
        impl Drop for TempDir {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        let root = TempDir(std::env::temp_dir().join(format!("cockpit-checkout-fence-{}-{}",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())));
        std::fs::create_dir(&root.0).unwrap();
        let path = root.0.join("checkout");
        symlink(std::env::current_dir().unwrap(), &path).unwrap();
        let cwd = std::fs::metadata(".").unwrap();
        let environment = BTreeMap::from([
            (CHECKOUT_DEVICE.into(), cwd.dev().to_string()),
            (CHECKOUT_INODE.into(), cwd.ino().to_string()),
            (CHECKOUT_PATH.into(), path.to_str().unwrap().into()),
        ]);
        let argv = [OsString::from("harness")];
        assert!(harness_command(&argv, environment.clone(), true).is_ok());
        assert!(harness_command(&argv, environment.clone(), false).is_ok());
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(harness_command(&argv, environment.clone(), true).is_err());
        std::fs::remove_dir(&path).unwrap();
        assert!(harness_command(&argv, environment, true).is_err());
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
