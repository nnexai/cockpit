//! Launch-only, harness-neutral helper. No orchestration, providers or hooks.
use std::{ffi::{OsStr, OsString}, process::Command};
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
use clap::Args;

#[derive(Debug, Args)]
pub(crate) struct ContainArgs {
    #[arg(long)]
    unit: String,
    #[arg(long)]
    description: String,
    #[arg(long, hide = true)]
    in_scope: bool,
    #[arg(long, hide = true)]
    restore_runtime_absent: bool,
    #[arg(long, hide = true, allow_hyphen_values = true, conflicts_with = "restore_runtime_absent")]
    restore_runtime_value: Option<OsString>,
    /// Original executable and arguments, without shell or environment expansion.
    #[arg(last = true, required = true, num_args = 1.., allow_hyphen_values = true)]
    argv: Vec<OsString>,
}

/// Scope mode registers systemd-run's own PID, then execvpe's the command
/// (upstream systemd v259 src/run/run.c:start_transient_scope). It neither
/// replaces the caller's PTY nor creates a service with a separate main PID.
/// The command inherits the original environment and literal argv, including $.
pub(crate) async fn run(args: ContainArgs) -> u8 {
    if args.argv.is_empty() { return 2; }
    #[cfg(target_os = "linux")]
    if !args.in_scope && preconditions(&args.unit) && probe().await
        && let Ok(runtime) = verified_manager_runtime() {
            match scope_command(&args, &runtime) {
                Ok(command) => return execute(command),
                Err(error) => { eprintln!("contain: {error}"); return 126; }
            }
        }
    // Unsupported systems, missing delegation, old systemd versions (without
    // --expand-environment=no) and probe failure run the original command. No
    // receipt can be captured: this does not fake containment.
    let mut command = Command::new(&args.argv[0]);
    command.args(&args.argv[1..]);
    if args.restore_runtime_absent || args.restore_runtime_value.is_some() {
        restore_runtime_environment(&mut command, args.restore_runtime_value.as_deref());
    }
    execute(command)
}

pub(crate) fn execute(mut command: Command) -> u8 {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        eprintln!("contain: could not execute original launch: {error}");
        126
    }
    #[cfg(not(unix))]
    {
        match command.status() {
            Ok(status) => status.code().and_then(|code| u8::try_from(code).ok()).unwrap_or(1),
            Err(error) => { eprintln!("contain: could not execute original launch: {error}"); 126 }
        }
    }
}

#[cfg(target_os = "linux")]
fn scope_command(args: &ContainArgs, manager_runtime: &Path) -> std::io::Result<Command> {
    let original = std::env::var_os("XDG_RUNTIME_DIR");
    let mut command = manager_scope_command(&args.unit, &args.description, manager_runtime, original.as_deref());
    command.arg("--");
    if original.as_deref().is_none_or(|value| value.to_str().is_none()) {
        command.arg(std::env::current_exe()?).args(["contain", "--in-scope", "--unit", &args.unit,
            "--description", &args.description]);
        match &original {
            Some(value) => { command.arg("--restore-runtime-value").arg(value); }
            None => { command.arg("--restore-runtime-absent"); }
        }
        command.arg("--");
    }
    command.args(&args.argv);
    Ok(command)
}

pub(crate) fn restore_runtime_environment(command: &mut Command, original: Option<&OsStr>) {
    match original {
        Some(value) => { command.env("XDG_RUNTIME_DIR", value); }
        None => { command.env_remove("XDG_RUNTIME_DIR"); }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn manager_scope_command(
    unit: &str, description: &str, runtime: &Path, original: Option<&OsStr>,
) -> Command {
    let mut command = Command::new("/usr/bin/systemd-run");
    command.env("XDG_RUNTIME_DIR", runtime).args([
        "--user", "--scope", "--quiet", "--collect", "--no-ask-password",
        "--expand-environment=no", "--unit", unit, "--description", description,
        "--property=TimeoutStopSec=15s",
    ]);
    if let Some(original) = original.filter(|value| value.to_str().is_some()) {
        let mut assignment = OsString::from("XDG_RUNTIME_DIR=");
        assignment.push(original);
        // Scope mode applies this only after connecting to the manager, in
        // strv_env_merge before execvpe. Even an empty original stays present.
        command.arg("--setenv").arg(assignment);
    }
    command
}

#[cfg(target_os = "linux")]
pub(crate) fn verified_manager_runtime() -> std::io::Result<PathBuf> {
    use nix::{fcntl::{AtFlags, OFlag, open, openat}, sys::stat::{Mode, SFlag, fstat, fstatat}};
    let uid = nix::unistd::geteuid().as_raw();
    let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let run = open("/run", flags, Mode::empty())?;
    let users = openat(&run, "user", flags, Mode::empty())?;
    let name = uid.to_string();
    let runtime = openat(&users, name.as_str(), flags, Mode::empty())?;
    let meta = fstat(&runtime)?;
    if meta.st_uid != uid || meta.st_mode & 0o7777 != 0o700 {
        return Err(std::io::Error::other("actual manager runtime is not private UID-owned 0700"));
    }
    let systemd = openat(&runtime, "systemd", flags, Mode::empty())?;
    let socket = fstatat(&systemd, "private", AtFlags::AT_SYMLINK_NOFOLLOW)?;
    if socket.st_uid != uid || socket.st_mode & SFlag::S_IFMT.bits() != SFlag::S_IFSOCK.bits() {
        return Err(std::io::Error::other("actual manager private endpoint is not a UID-owned socket"));
    }
    Ok(PathBuf::from("/run/user").join(name))
}

#[cfg(target_os = "linux")]
fn delegated_membership(text: &str, uid: u32) -> bool {
    let root = format!("/user.slice/user-{uid}.slice/user@{uid}.service");
    let Some(group) = text.strip_prefix("0::") else { return false };
    let group = group.strip_suffix('\n').unwrap_or(group);
    !group.contains('\n') && (group == root || group.strip_prefix(&root).is_some_and(|s| s.starts_with('/')))
}

#[cfg(target_os = "linux")]
pub(crate) fn preconditions(unit: &str) -> bool {
    use std::{io::Read, os::unix::fs::PermissionsExt};
    if !cockpit_core::process_scope::valid_scope_unit(unit)
        || !std::fs::metadata("/sys/fs/cgroup/cgroup.controllers").is_ok_and(|m| m.is_file())
        || !std::fs::metadata("/usr/bin/systemd-run").is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        || verified_manager_runtime().is_err() {
        return false;
    }
    let Ok(file) = std::fs::File::open("/proc/self/cgroup") else { return false };
    let mut text = String::new();
    file.take(4097).read_to_string(&mut text).is_ok() && text.len() <= 4096
        && delegated_membership(&text, nix::unistd::geteuid().as_raw())
}

#[cfg(target_os = "linux")]
pub(crate) async fn probe() -> bool {
    use std::time::Duration;
    // Independent unique name; never reuse or stop another launch's unit. The
    // finite /bin/true scope collects itself. Only manager routing is changed
    // for this subprocess; the original harness environment stays untouched.
    let unit = format!("cockpit-probe-{}.scope", uuid::Uuid::new_v4().simple());
    let mut command = tokio::process::Command::new("/usr/bin/systemd-run");
    let Ok(runtime) = verified_manager_runtime() else { return false };
    command.env("XDG_RUNTIME_DIR", runtime);
    command.args([
        "--user", "--scope", "--quiet", "--collect", "--no-ask-password",
        "--expand-environment=no", "--unit", &unit, "--", "/bin/true",
    ]);
    // Leave half a second for the existing owned-child runner's bounded reap;
    // the outer deadline bounds the whole preflight, not just command runtime.
    tokio::time::timeout(Duration::from_secs(2),
        cockpit_core::process::run_bounded_command(command, 4096, 4096, Duration::from_millis(1500), "scope launch preflight"))
        .await.is_ok_and(|result| result.is_ok_and(|output| output.status.success()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    struct TestCli { #[command(flatten)] args: ContainArgs }

    #[test]
    fn cli_accepts_only_an_explicit_literal_trailing_command() {
        let cli = TestCli::try_parse_from([
            "contain", "--unit", "unit.scope", "--description", "a label", "--",
            "/usr/bin/harness", "$HOME", "a b", "--resume", "$(touch unsafe)",
        ]).unwrap();
        assert_eq!(cli.args.argv, ["/usr/bin/harness", "$HOME", "a b", "--resume", "$(touch unsafe)"].map(OsString::from));
        assert!(TestCli::try_parse_from(["contain", "--unit", "unit.scope", "--description", "label"]).is_err());
        assert!(TestCli::try_parse_from(["contain", "--unit", "unit.scope", "--description", "label", "harness"]).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn wrapper_disables_expansion_and_retains_all_literal_arguments() {
        let args = ContainArgs { unit: "u.scope".into(), description: "label".into(),
            in_scope: false, restore_runtime_absent: false, restore_runtime_value: None,
            argv: vec!["harness".into(), "$HOME".into(), "a b".into(), "--unit".into()] };
        let command = scope_command(&args, Path::new("/run/user/1000")).unwrap();
        let wrapped: Vec<_> = command.get_args().collect();
        assert!(wrapped.contains(&std::ffi::OsStr::new("--expand-environment=no")));
        assert_eq!(&wrapped[wrapped.len() - args.argv.len()..], &args.argv.iter().map(OsString::as_os_str).collect::<Vec<_>>());
        assert!(delegated_membership("0::/user.slice/user-1000.slice/user@1000.service/app.slice/u.scope\n", 1000));
        for group in ["0::/user.slice/user-1000.slice/session-1.scope\n", "0::/user.slice/user-1000.slice/user@1000.service-other/u\n", "0::/user.slice/user-1001.slice/user@1001.service/u\n", "0::/user.slice/user-1000.slice/user@1000.service/u\n0::/another\n"] {
            assert!(!delegated_membership(group, 1000));
        }
    }

    #[cfg(unix)]
    #[test]
    fn actual_consumer_environment_restores_runtime_presence_absence_and_bytes() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        for original in [
            None,
            Some(OsString::from("/private/runtime space/$literal")),
            Some(OsString::new()),
            Some(OsString::from_vec(b"/private/non-utf8-\xff".to_vec())),
        ] {
            let mut child = Command::new("/usr/bin/env");
            child.env_clear().arg("-0")
                .env("XDG_RUNTIME_DIR", "/run/user/1000")
                .env("PATH", "/literal/bin")
                .env("COCKPIT_JOB_CREDENTIAL_FILE", "/private/child.cap")
                .env("PI_CODING_AGENT_SESSION_DIR", "/private/sessions")
                .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/original/bus");
            restore_runtime_environment(&mut child, original.as_deref());
            let output = child.output().unwrap();
            assert!(output.status.success());
            let find = |key: &[u8]| output.stdout.split(|byte| *byte == 0)
                .find_map(|entry| entry.strip_prefix(key));
            assert_eq!(find(b"XDG_RUNTIME_DIR="), original.as_deref().map(OsStr::as_bytes));
            assert_eq!(find(b"PATH="), Some(b"/literal/bin".as_slice()));
            assert_eq!(find(b"COCKPIT_JOB_CREDENTIAL_FILE="), Some(b"/private/child.cap".as_slice()));
            assert_eq!(find(b"PI_CODING_AGENT_SESSION_DIR="), Some(b"/private/sessions".as_slice()));
            assert_eq!(find(b"DBUS_SESSION_BUS_ADDRESS="), Some(b"unix:path=/original/bus".as_slice()));
        }
    }
}
