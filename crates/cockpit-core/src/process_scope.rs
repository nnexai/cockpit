//! Exact, receipt-owned process scopes, not a sandbox or an external-writer oracle.
//! Deliberate migration to another cgroup is outside this adapter's proof.

use std::{path::Path, time::Duration};
use cockpit_protocol::orchestration::{NativeProcessIdentity, OwnedProcessScope, RunKind};
use sha2::{Digest, Sha256};
use crate::InspectionError;

mod latch;
pub use latch::{AgentLaunch, ScopeStartup, agent_launch, latch_deadline_ns, latch_ready, latch_startup, release_latch, wait_latch};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeLaunch {
    pub unit: String,
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeObservation {
    pub scope: OwnedProcessScope,
    pub state: ScopeState,
    pub observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeState {
    Populated,
    Drained,
    Unknown { reason: String },
}

fn scope_unit(store_identity: &str, run_id: &str, kind: &RunKind, epoch: u64) -> String {
    let short_hash = |value: &str| {
        let digest = Sha256::digest(value.as_bytes());
        u32::from_be_bytes(digest[..4].try_into().expect("SHA256 has four bytes"))
    };
    let role = if *kind == RunKind::Worker { "w" } else { "r" };
    format!("cockpit-{role}-{:08x}-{:08x}-e{epoch}-{}.scope",
        short_hash(store_identity), short_hash(run_id), uuid::Uuid::new_v4().simple())
}

/// Wrap the literal harness command. Only newly launched owned attempts get scopes.
pub fn scope_launch(
    cli_path: &Path,
    store_identity: &str,
    run_id: &str,
    kind: &RunKind,
    epoch: u64,
    description: &str,
    argv: Vec<String>,
) -> ScopeLaunch {
    let unit = scope_unit(store_identity, run_id, kind, epoch);
    let mut wrapped = Vec::with_capacity(argv.len() + 7);
    wrapped.extend([
        cli_path.to_string_lossy().into_owned(), "contain".into(), "--unit".into(),
        unit.clone(), "--description".into(), description.into(), "--".into(),
    ]);
    wrapped.extend(argv);
    ScopeLaunch { unit, argv: wrapped }
}

/// Reject aliases, glob patterns, options, traversal and non-owned unit names.
pub fn valid_scope_unit(unit: &str) -> bool {
    let Some(body) = unit.strip_suffix(".scope") else { return false };
    let mut fields = body.split('-');
    if fields.next() != Some("cockpit") || !matches!(fields.next(), Some("w" | "r")) {
        return false;
    }
    let (Some(store), Some(run), Some(epoch), Some(nonce)) =
        (fields.next(), fields.next(), fields.next(), fields.next()) else { return false };
    fields.next().is_none() && hex(store, 8) && hex(run, 8) && hex(nonce, 32)
        && epoch.strip_prefix('e').is_some_and(|s| !s.is_empty()
            && (s == "0" || !s.starts_with('0'))
            && s.bytes().all(|b| b.is_ascii_digit()) && s.parse::<u64>().is_ok())
}

fn hex(text: &str, length: usize) -> bool {
    text.len() == length && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn now() -> String {
    time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)
        .expect("UTC timestamp is representable")
}

fn unknown(scope: &OwnedProcessScope, reason: impl Into<String>) -> ScopeObservation {
    ScopeObservation { scope: scope.clone(), state: ScopeState::Unknown { reason: reason.into() }, observed_at: now() }
}

pub async fn capture(unit: &str, member: &NativeProcessIdentity) -> Result<Option<OwnedProcessScope>, InspectionError> {
    #[cfg(target_os = "linux")]
    { return linux::capture(unit, member).await; }
    #[cfg(not(target_os = "linux"))]
    { let _ = (unit, member); Ok(None) }
}

pub async fn observe(scope: &OwnedProcessScope) -> Result<ScopeObservation, InspectionError> {
    #[cfg(target_os = "linux")]
    { return Ok(match linux::observe(scope, linux::QUERY_TIMEOUT).await {
        Ok(observation) => observation,
        Err(error) => unknown(scope, error.to_string()),
    }); }
    #[cfg(not(target_os = "linux"))]
    { Ok(unknown(scope, "process scopes are unsupported on this platform")) }
}

/// Callers must first seal the attachment and apply accepted/retired/owned,
/// clean/integrated and no-active-build gates. This adapter supplies only the
/// fresh physical identity guard; it never grants semantic cleanup authority.
pub async fn stop_exact(scope: &OwnedProcessScope, deadline: Duration) -> Result<ScopeObservation, InspectionError> {
    #[cfg(target_os = "linux")]
    {
        let bounded = deadline.min(Duration::from_secs(20));
        return Ok(match tokio::time::timeout(bounded, linux::stop_exact(scope, bounded)).await {
            Ok(Ok(observation)) => observation,
            Ok(Err(error)) => unknown(scope, error.to_string()),
            Err(_) => unknown(scope, "exact scope stop exceeded its deadline"),
        });
    }
    #[cfg(not(target_os = "linux"))]
    { let _ = deadline; Ok(unknown(scope, "process scopes are unsupported on this platform")) }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::{fs::File, io::{self, Read}, os::unix::fs::MetadataExt, os::fd::OwnedFd};
    use cockpit_protocol::orchestration::ScopeKind;
    use rustix::fs::{open, openat, Mode, OFlags};
    use tokio::process::Command;
    use crate::{process::run_bounded_command, process_identity};

    pub(super) const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
    const DIRECTORY: OFlags = OFlags::RDONLY.union(OFlags::DIRECTORY).union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC);

    fn error(reason: impl Into<String>) -> InspectionError {
        InspectionError::new("process_scope_unproven", reason)
    }

    fn uid() -> u32 { nix::unistd::geteuid().as_raw() }
    fn manager_group() -> String {
        let uid = uid();
        format!("/user.slice/user-{uid}.slice/user@{uid}.service")
    }

    fn safe_group(group: &str, unit: &str) -> bool {
        let root = manager_group();
        let Some(rest) = group.strip_prefix(&format!("{root}/")) else { return false };
        let mut parts = rest.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() { return part == unit; }
            if !part.ends_with(".slice") || part.starts_with('.')
                || !part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) {
                return false;
            }
        }
        false
    }

    // Open every component relative to a pinned directory descriptor: no
    // symlink follows or canonicalize-then-open race in receipt-directed reads.
    fn directory(root: &str, relative: &str) -> io::Result<OwnedFd> {
        let mut fd = open(root, DIRECTORY, Mode::empty())?;
        for part in relative.split('/') {
            if part.is_empty() || matches!(part, "." | "..") {
                return Err(io::Error::other("invalid directory component"));
            }
            fd = openat(&fd, part, DIRECTORY, Mode::empty())?;
        }
        Ok(fd)
    }

    fn bounded_file(fd: &OwnedFd, name: &str) -> io::Result<String> {
        let file = openat(fd, name, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())?;
        let mut text = String::new();
        File::from(file).take(4097).read_to_string(&mut text)?;
        if text.len() > 4096 { return Err(io::Error::other("cgroup evidence exceeded limit")); }
        Ok(text)
    }

    fn systemctl() -> Result<Command, InspectionError> {
        let runtime = directory("/run", &format!("user/{}", uid())).map_err(|e| error(e.to_string()))?;
        let metadata = File::from(runtime).metadata().map_err(|e| error(e.to_string()))?;
        if metadata.uid() != uid() || metadata.mode() & 0o7777 != 0o700 {
            return Err(error("actual user runtime directory is not private and UID-owned"));
        }
        let runtime_path = format!("/run/user/{}", uid());
        let bus = std::fs::symlink_metadata(format!("{runtime_path}/bus")).map_err(|e| error(e.to_string()))?;
        use std::os::unix::fs::FileTypeExt;
        if bus.uid() != uid() || !bus.file_type().is_socket() {
            return Err(error("actual user manager bus is not a UID-owned socket"));
        }
        let mut command = Command::new("/usr/bin/systemctl");
        command.env_clear().env("XDG_RUNTIME_DIR", &runtime_path)
            .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={runtime_path}/bus"))
            .args(["--user", "--no-pager", "--no-ask-password"]);
        Ok(command)
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Unit {
        invocation: String,
        group: String,
        active: String,
        load: String,
    }

    fn parse_unit(text: &str) -> Result<Unit, InspectionError> {
        let mut invocation = None;
        let mut group = None;
        let mut active = None;
        let mut load = None;
        for line in text.lines() {
            let (name, value) = line.split_once('=').ok_or_else(|| error("malformed systemctl property"))?;
            let target = match name {
                "InvocationID" => &mut invocation,
                "ControlGroup" => &mut group,
                "ActiveState" => &mut active,
                "LoadState" => &mut load,
                _ => return Err(error("unexpected systemctl property")),
            };
            if target.replace(value.to_owned()).is_some() { return Err(error("duplicate systemctl property")); }
        }
        Ok(Unit {
            invocation: invocation.ok_or_else(|| error("missing InvocationID"))?,
            group: group.ok_or_else(|| error("missing ControlGroup"))?,
            active: active.ok_or_else(|| error("missing ActiveState"))?,
            load: load.ok_or_else(|| error("missing LoadState"))?,
        })
    }

    async fn show(unit: &str, timeout: Duration) -> Result<Unit, InspectionError> {
        if !valid_scope_unit(unit) { return Err(error("invalid Cockpit scope unit")); }
        let mut command = systemctl()?;
        command.args(["show", "--all", "--property=InvocationID,ControlGroup,ActiveState,LoadState", "--", unit]);
        let output = run_bounded_command(command, 8192, 4096, timeout, "scope identity query").await?;
        let text = std::str::from_utf8(&output.stdout).map_err(|_| error("non-UTF8 unit evidence"))?;
        let properties = parse_unit(text)?;
        // systemctl show may exit unsuccessfully for a collected unit but
        // provides complete not-found properties. Never infer from stderr.
        if !output.status.success() && properties.load != "not-found" {
            return Err(error("scope identity query failed"));
        }
        Ok(properties)
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Group { Missing, Present { device: u64, inode: u64, populated: bool } }

    fn parse_populated(text: &str) -> Result<bool, InspectionError> {
        let mut populated = None;
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let name = words.next().ok_or_else(|| error("empty cgroup event"))?;
            let value = words.next().ok_or_else(|| error("missing cgroup event value"))?;
            if words.next().is_some() { return Err(error("malformed cgroup event")); }
            if name == "populated" {
                if populated.is_some() { return Err(error("duplicate populated event")); }
                populated = Some(match value { "0" => false, "1" => true, _ => return Err(error("invalid populated event")) });
            }
        }
        populated.ok_or_else(|| error("missing populated event"))
    }

    fn group_state(group: &str, unit: &str) -> Result<Group, InspectionError> {
        if !safe_group(group, unit) { return Err(error("scope cgroup outside current user manager subtree")); }
        let fd = match directory("/sys/fs/cgroup", &group[1..]) {
            Ok(fd) => fd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Group::Missing),
            Err(e) => return Err(error(e.to_string())),
        };
        let metadata = rustix::fs::fstat(&fd).map_err(|e| error(e.to_string()))?;
        let populated = parse_populated(&bounded_file(&fd, "cgroup.events").map_err(|e| error(e.to_string()))?)?;
        Ok(Group::Present { device: metadata.st_dev, inode: metadata.st_ino, populated })
    }

    fn process_group(text: &str) -> Result<&str, InspectionError> {
        let mut lines = text.lines();
        let group = lines.next().and_then(|line| line.strip_prefix("0::"))
            .ok_or_else(|| error("not a unified cgroup membership"))?;
        if lines.next().is_some() || !group.starts_with('/') {
            return Err(error("ambiguous cgroup membership"));
        }
        Ok(group)
    }

    fn member_matches(member: &NativeProcessIdentity, group: &str) -> Result<bool, InspectionError> {
        let pid = i32::try_from(member.pid).map_err(|_| error("invalid member PID"))?;
        if member.start_ticks == 0 || member.kernel_boot_id != process_identity::kernel_boot_id() {
            return Err(error("member incarnation is incomplete or foreign"));
        }
        if !process_identity::incarnation_running(pid, member.start_ticks).map_err(|e| error(e.to_string()))? {
            return Ok(false);
        }
        let proc = directory("/proc", &member.pid.to_string()).map_err(|e| error(e.to_string()))?;
        let text = bounded_file(&proc, "cgroup").map_err(|e| error(e.to_string()))?;
        if process_group(&text)? != group { return Err(error("recorded live member left the owned scope")); }
        Ok(process_identity::incarnation_running(pid, member.start_ticks).map_err(|e| error(e.to_string()))?)
    }

    fn validate(scope: &OwnedProcessScope) -> Result<(), InspectionError> {
        if scope.kind != ScopeKind::SystemdUserScope || !valid_scope_unit(&scope.unit)
            || !hex(&scope.invocation_id, 32) || scope.invocation_id.bytes().all(|b| b == b'0')
            || !safe_group(&scope.control_group, &scope.unit)
            || scope.cgroup_device.parse::<u64>().is_err()
            || scope.cgroup_inode.parse::<u64>().ok().filter(|v| *v != 0).is_none()
            || !hex(&scope.machine_id, 64)
            || scope.member.pid == 0 || scope.member.pid > i32::MAX as u32 || scope.member.start_ticks == 0
            || scope.member.kernel_boot_id.as_deref() != Some(scope.kernel_boot_id.as_str())
            || time::OffsetDateTime::parse(&scope.captured_at, &time::format_description::well_known::Rfc3339).is_err() {
            return Err(error("incomplete or malformed owned scope receipt"));
        }
        if process_identity::machine_id().as_deref() != Some(scope.machine_id.as_str())
            || process_identity::kernel_boot_id().as_deref() != Some(scope.kernel_boot_id.as_str()) {
            return Err(error("scope machine or boot identity differs"));
        }
        Ok(())
    }

    fn classify(scope: &OwnedProcessScope, unit: &Unit, group: Group) -> ScopeState {
        let fail = |reason: &str| ScopeState::Unknown { reason: reason.into() };
        if unit.load == "not-found" {
            return if group == Group::Missing && unit.invocation.is_empty()
                && unit.group.is_empty() && unit.active == "inactive" { ScopeState::Drained }
                else { fail("collected scope has contradictory identity or cgroup evidence") };
        }
        if unit.load != "loaded" || unit.invocation != scope.invocation_id {
            return fail("scope invocation was replaced or cannot be verified");
        }
        if !unit.group.is_empty() && unit.group != scope.control_group {
            return fail("scope ControlGroup changed");
        }
        match group {
            Group::Present { device, inode, populated }
                if scope.cgroup_device.parse::<u64>() == Ok(device) && scope.cgroup_inode.parse::<u64>() == Ok(inode) => {
                    if populated && unit.active == "active" && unit.group == scope.control_group { ScopeState::Populated }
                    else if !populated && unit.active == "inactive" { ScopeState::Drained }
                    else { fail("scope activity and population disagree") }
                }
            Group::Present { .. } => fail("scope cgroup directory was replaced"),
            Group::Missing if unit.active == "inactive" => ScopeState::Drained,
            Group::Missing => fail("live scope cgroup is missing"),
        }
    }

    pub(super) async fn capture(unit: &str, member: &NativeProcessIdentity) -> Result<Option<OwnedProcessScope>, InspectionError> {
        if !valid_scope_unit(unit) { return Err(error("invalid Cockpit scope unit")); }
        // Unsupported tools, fallback launches and any capture race leave no
        // receipt. There is no retrospectively invented containment proof.
        let attempt = async {
            let machine = process_identity::verified_machine_id(member).ok_or_else(|| error("member identity unavailable"))?;
            let first = show(unit, QUERY_TIMEOUT).await?;
            if first.load != "loaded" || first.active != "active" || !hex(&first.invocation, 32) {
                return Err(error("scope is not active"));
            }
            let Group::Present { device, inode, populated: true } = group_state(&first.group, unit)?
                else { return Err(error("scope is not populated")); };
            if !member_matches(member, &first.group)? { return Err(error("capture member exited")); }
            let scope = OwnedProcessScope {
                kind: ScopeKind::SystemdUserScope, unit: unit.into(), invocation_id: first.invocation.clone(),
                control_group: first.group.clone(), cgroup_device: device.to_string(), cgroup_inode: inode.to_string(),
                machine_id: machine, kernel_boot_id: member.kernel_boot_id.clone().ok_or_else(|| error("member boot unavailable"))?,
                member: member.clone(), captured_at: now(),
            };
            validate(&scope)?;
            let fresh = show(unit, QUERY_TIMEOUT).await?;
            let state = group_state(&scope.control_group, unit)?;
            if first != fresh || classify(&scope, &fresh, state) != ScopeState::Populated
                || !member_matches(member, &scope.control_group)? { return Err(error("scope changed during capture")); }
            Ok(scope)
        }.await;
        Ok(attempt.ok())
    }

    pub(super) async fn observe(scope: &OwnedProcessScope, timeout: Duration) -> Result<ScopeObservation, InspectionError> {
        let operation = async {
            validate(scope)?;
            let first = show(&scope.unit, timeout).await?;
            let group = group_state(&scope.control_group, &scope.unit)?;
            let fresh = show(&scope.unit, timeout).await?;
            let fresh_group = group_state(&scope.control_group, &scope.unit)?;
            validate(scope)?;
            let _ = member_matches(&scope.member, &scope.control_group)?;
            let state = if first != fresh || group != fresh_group {
                ScopeState::Unknown { reason: "scope changed during observation".into() }
            } else { classify(scope, &fresh, fresh_group) };
            Ok(ScopeObservation { scope: scope.clone(), state, observed_at: now() })
        };
        tokio::time::timeout(timeout, operation).await
            .map_err(|_| error("scope observation exceeded its deadline"))?
    }

    pub(super) async fn stop_exact(scope: &OwnedProcessScope, deadline: Duration) -> Result<ScopeObservation, InspectionError> {
        if !scope.unit.starts_with("cockpit-w-") { return Ok(unknown(scope, "supervisor scopes are never stopped")); }
        let end = tokio::time::Instant::now() + deadline.min(Duration::from_secs(20));
        let remaining = || end.saturating_duration_since(tokio::time::Instant::now());
        if remaining().is_zero() { return Ok(unknown(scope, "stop deadline expired")); }
        let fresh = observe(scope, remaining().min(QUERY_TIMEOUT)).await?;
        if fresh.state != ScopeState::Populated { return Ok(fresh); }
        // The original member may have exited with descendants still alive.
        // If it still exists, recheck its exact incarnation and membership.
        let _ = member_matches(&scope.member, &scope.control_group)?;
        validate(scope)?;
        // Last complete unit+directory identity check immediately before the
        // only destructive command. Never stop a replacement invocation.
        let unit = show(&scope.unit, remaining().min(QUERY_TIMEOUT)).await?;
        if classify(scope, &unit, group_state(&scope.control_group, &scope.unit)?) != ScopeState::Populated {
            return Ok(unknown(scope, "scope changed before stop"));
        }
        if remaining().is_zero() { return Ok(unknown(scope, "stop deadline expired")); }
        let mut command = systemctl()?;
        command.args(["stop", "--", &scope.unit]);
        let output = run_bounded_command(command, 4096, 4096, remaining(), "exact owned scope stop").await?;
        if !output.status.success() { return Err(error("exact owned scope stop failed")); }
        loop {
            if remaining().is_zero() { return Ok(unknown(scope, "scope did not drain before deadline")); }
            let observation = observe(scope, remaining().min(QUERY_TIMEOUT)).await?;
            if observation.state != ScopeState::Populated { return Ok(observation); }
            tokio::time::sleep(remaining().min(Duration::from_millis(50))).await;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        const UNIT: &str = "cockpit-w-01234567-89abcdef-e1-0123456789abcdef0123456789abcdef.scope";
        fn receipt() -> OwnedProcessScope {
            OwnedProcessScope { kind: ScopeKind::SystemdUserScope, unit: UNIT.into(),
                invocation_id: "0123456789abcdef0123456789abcdef".into(),
                control_group: format!("{}/app.slice/{UNIT}", manager_group()),
                cgroup_device: "1".into(), cgroup_inode: "2".into(), machine_id: "a".repeat(64),
                kernel_boot_id: "00000000-0000-0000-0000-000000000001".into(),
                member: NativeProcessIdentity { pid: 1, start_ticks: 2, kernel_boot_id: Some("00000000-0000-0000-0000-000000000001".into()) },
                captured_at: "2026-10-13T00:00:00Z".into() }
        }
        fn live(scope: &OwnedProcessScope) -> Unit {
            Unit { invocation: scope.invocation_id.clone(), group: scope.control_group.clone(), active: "active".into(), load: "loaded".into() }
        }
        #[test]
        fn property_parser_refuses_missing_duplicate_and_extra_fields() {
            let text = "InvocationID=0123456789abcdef0123456789abcdef\nControlGroup=/a\nActiveState=active\nLoadState=loaded\n";
            assert!(parse_unit(text).is_ok());
            assert!(parse_unit(&text.replace("LoadState=loaded\n", "")).is_err());
            assert!(parse_unit(&format!("{text}LoadState=loaded\n")).is_err());
            assert!(parse_unit(&format!("{text}Untrusted=1\n")).is_err());
        }
        #[test]
        fn membership_and_events_require_unambiguous_kernel_evidence() {
            assert_eq!(process_group("0::/a\n").unwrap(), "/a");
            for text in ["1:cpu:/a\n", "0::/a\n0::/b\n", "0::relative"] { assert!(process_group(text).is_err()); }
            assert_eq!(parse_populated("populated 0\nfrozen 0\n").unwrap(), false);
            for text in ["frozen 0", "populated 2", "populated 0\npopulated 1", "populated 0 extra"] { assert!(parse_populated(text).is_err()); }
        }
        #[test]
        fn group_paths_never_escape_the_current_user_manager() {
            let scope = receipt();
            assert!(safe_group(&scope.control_group, UNIT));
            for group in [format!("{}/../{UNIT}", manager_group()), format!("{}/app.slice//{UNIT}", manager_group()), format!("/system.slice/{UNIT}"), format!("{}/app.slice/other.scope", manager_group())] {
                assert!(!safe_group(&group, UNIT));
            }
        }
        #[test]
        fn replacements_never_prove_drain_or_authorize_stop() {
            let scope = receipt();
            let mut unit = live(&scope);
            let present = Group::Present { device: 1, inode: 2, populated: true };
            assert_eq!(classify(&scope, &unit, present), ScopeState::Populated);
            unit.invocation = "b".repeat(32);
            assert!(matches!(classify(&scope, &unit, Group::Missing), ScopeState::Unknown { .. }));
            unit = live(&scope); unit.active = "inactive".into();
            assert_eq!(classify(&scope, &unit, Group::Present { device: 1, inode: 2, populated: false }), ScopeState::Drained);
            assert!(matches!(classify(&scope, &unit, Group::Present { device: 1, inode: 3, populated: false }), ScopeState::Unknown { .. }));
            unit.load = "not-found".into(); unit.invocation.clear(); unit.group.clear();
            assert_eq!(classify(&scope, &unit, Group::Missing), ScopeState::Drained);
            assert!(matches!(classify(&scope, &unit, present), ScopeState::Unknown { .. }));
        }
        #[tokio::test]
        async fn public_api_refuses_incomplete_receipts_without_executing_tools() {
            let mut scope = receipt();
            scope.unit = "../ssh.service".into();
            assert!(matches!(crate::process_scope::observe(&scope).await.unwrap().state, ScopeState::Unknown { .. }));
            assert!(matches!(crate::process_scope::stop_exact(&scope, Duration::from_secs(1)).await.unwrap().state, ScopeState::Unknown { .. }));
            assert!(crate::process_scope::capture("--all", &scope.member).await.is_err());
            scope = receipt();
            scope.member.start_ticks = 0;
            assert!(matches!(crate::process_scope::observe(&scope).await.unwrap().state, ScopeState::Unknown { .. }));
        }

        #[test]
        fn descriptor_walk_refuses_symlink_components() {
            let root = std::env::temp_dir().join(format!("cockpit-scope-path-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            std::fs::create_dir(root.join("real")).unwrap();
            std::os::unix::fs::symlink("real", root.join("alias")).unwrap();
            let result = directory(root.to_str().unwrap(), "alias");
            std::fs::remove_dir_all(&root).unwrap();
            assert!(result.is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_launch_is_unique_and_role_scoped() {
        let input = vec!["harness".into(), "$HOME".into(), "a b".into(), "--flag".into()];
        let launch = scope_launch(Path::new("/bin/cockpit"), "store", "run", &RunKind::Worker, 1, "label", input.clone());
        let second = scope_launch(Path::new("/bin/cockpit"), "store", "run", &RunKind::Worker, 1, "label", input.clone());
        assert!(valid_scope_unit(&launch.unit));
        assert_ne!(launch.unit, second.unit);
        assert_eq!(&launch.argv[7..], &input);
        let root = scope_launch(Path::new("/bin/cockpit"), "store", "root", &RunKind::Supervisor, 1, "label", vec![]);
        assert!(root.unit.starts_with("cockpit-r-"));
    }
    #[test]
    fn only_complete_generated_scope_names_are_accepted() {
        for name in ["cockpit-w-any.scope", "ssh.service", "--help", "../cockpit-w-a.scope", "cockpit-w-01234567-89abcdef-e1-*.scope", "cockpit-w-01234567-89abcdef-e18446744073709551616-0123456789abcdef0123456789abcdef.scope"] {
            assert!(!valid_scope_unit(name), "{name}");
        }
    }
}
