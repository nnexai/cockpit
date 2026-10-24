//! Process-incarnation evidence for orchestration binding and accepted-worker retirement.
//! These reads grant no signal or process-control authority.

use std::io;
use cockpit_protocol::orchestration::NativeShellIdentity;

fn valid_pid(pid: i32) -> io::Result<()> {
    if pid <= 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "process PID must be positive"));
    }
    Ok(())
}

/// Read the OS start identity of this PID. Unavailable evidence is not an identity.
pub fn start_identity(pid: i32) -> Option<u64> {
    read_process(pid).ok().map(|process| process.start)
}

/// Whether the recorded incarnation still runs. Absence, PID reuse and zombies
/// prove exit; unreadable or malformed evidence does not.
pub fn incarnation_running(pid: i32, start: u64) -> io::Result<bool> {
    valid_pid(pid)?;
    match read_process(pid) {
        Ok(process) => Ok(process.start == start && !process.exited),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Prove the exact incarnation still occupies its controlling terminal's
/// foreground group. Background/stopped survivors cannot identify a new OMP.
#[cfg(target_os = "linux")]
pub(crate) fn incarnation_foreground(pid: i32, start: u64) -> io::Result<bool> {
    let process = read_process(pid)?;
    if !process.foreground(start) {
        return Ok(false);
    }
    let fresh = read_process(pid)?;
    Ok(fresh.foreground(start) && process.tty == fresh.tty
        && process.process_group == fresh.process_group)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn incarnation_foreground(pid: i32, _start: u64) -> io::Result<bool> {
    valid_pid(pid)?;
    Err(io::Error::new(io::ErrorKind::Unsupported, "foreground process evidence unavailable on this OS"))
}

/// Capture executable and argument evidence for the exact live shell incarnation.
/// PID/start alone survives exec; this fingerprint deliberately does not.
#[cfg(target_os = "linux")]
pub fn executable_identity(pid: i32, start: u64) -> io::Result<NativeShellIdentity> {
    use std::os::unix::fs::MetadataExt;

    let verify = || -> io::Result<()> {
        let process = read_process(pid)?;
        if process.start != start || process.exited {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "shell incarnation changed or exited"));
        }
        Ok(())
    };
    verify()?;
    let boot_id = kernel_boot_id().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "kernel boot identity unavailable")
    })?;
    let exe_path = format!("/proc/{pid}/exe");
    let argv_path = format!("/proc/{pid}/cmdline");
    // /proc/PID/exe resolves the actual loaded executable, including deleted
    // files; a pathname or a process name would not prove executable identity.
    let executable = std::fs::metadata(&exe_path)?;
    let argv = std::fs::read(&argv_path)?;
    let argv_digest = digest_argv(&argv)?;
    let fresh_executable = std::fs::metadata(&exe_path)?;
    let fresh_argv = std::fs::read(&argv_path)?;
    verify()?;
    if !executable.is_file()
        || executable.dev() != fresh_executable.dev()
        || executable.ino() != fresh_executable.ino()
        || argv != fresh_argv
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "shell executable or arguments changed during observation"));
    }
    Ok(NativeShellIdentity {
        process: cockpit_protocol::orchestration::NativeProcessIdentity {
            pid: pid as u32,
            start_ticks: start,
            kernel_boot_id: Some(boot_id),
        },
        executable_device: executable.dev().to_string(),
        executable_inode: executable.ino().to_string(),
        argv_digest,
    })
}

#[cfg(not(target_os = "linux"))]
pub fn executable_identity(pid: i32, _start: u64) -> io::Result<NativeShellIdentity> {
    valid_pid(pid)?;
    // libproc pidpath alone is not verified argv evidence.
    Err(io::Error::new(io::ErrorKind::Unsupported, "shell executable and argument evidence unavailable on this OS"))
}

#[cfg(target_os = "linux")]
fn digest_argv(argv: &[u8]) -> io::Result<String> {
    use sha2::{Digest, Sha256};

    if argv.is_empty() || argv[0] == 0 || argv.last() != Some(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "missing or incomplete process argv evidence"));
    }
    // NUL separators preserve argument boundaries and non-UTF8 bytes.
    Ok(format!("{:x}", Sha256::digest(argv)))
}

#[cfg(target_os = "linux")]
pub fn kernel_boot_id() -> Option<String> {
    let value = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    let value = value.trim();
    // The kernel exposes a UUID; malformed evidence must not fence an incarnation.
    if value.len() != 36
        || !value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return None;
    }
    Some(value.to_owned())
}

#[cfg(not(target_os = "linux"))]
pub fn kernel_boot_id() -> Option<String> {
    None
}

/// Application-domain hash of a stable local OS identifier. Never accept a
/// caller environment value, endpoint name or Herdr boot ID as machine proof.
#[cfg(target_os = "linux")]
pub fn machine_id() -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open("/etc/machine-id").ok()?;
    let mut bytes = [0u8; 65];
    let mut size = 0;
    while size < bytes.len() {
        let count = file.read(&mut bytes[size..]).ok()?;
        if count == 0 { break; }
        size += count;
    }
    machine_digest(&bytes[..size])
}

#[cfg(not(target_os = "linux"))]
pub fn machine_id() -> Option<String> { None }

fn machine_digest(bytes: &[u8]) -> Option<String> {
    use sha2::{Digest, Sha256};
    if bytes.len() > 64 { return None; }
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.len() != 32 || !text.bytes().all(|b| b.is_ascii_hexdigit())
        || text.bytes().all(|b| b == b'0') {
        return None;
    }
    let mut digest = Sha256::new();
    digest.update(b"cockpit:local-machine:v1\0");
    let mut normalized = [0u8; 32];
    for (target, source) in normalized.iter_mut().zip(text.bytes()) {
        *target = source.to_ascii_lowercase();
    }
    digest.update(normalized);
    Some(format!("{:x}", digest.finalize()))
}

/// Capture machine proof only alongside a freshly verified local incarnation.
/// A historical PID must never acquire today's machine identity by being loaded.
pub fn verified_machine_id(process: &cockpit_protocol::orchestration::NativeProcessIdentity) -> Option<String> {
    let pid = i32::try_from(process.pid).ok()?;
    let boot = kernel_boot_id()?;
    if process.start_ticks == 0 || process.kernel_boot_id.as_deref() != Some(boot.as_str())
        || !incarnation_running(pid, process.start_ticks).ok()? {
        return None;
    }
    let machine = machine_id()?;
    (kernel_boot_id().as_deref() == Some(boot.as_str())
        && incarnation_running(pid, process.start_ticks).ok()?).then_some(machine)
}

/// Exit is only meaningful on the recorded local machine. A different kernel
/// boot proves old incarnations absent only after stable machine equality.
pub fn same_machine_process_running(
    process: &cockpit_protocol::orchestration::NativeProcessIdentity,
    recorded_machine: Option<&str>,
) -> io::Result<bool> {
    let unavailable = || io::Error::other("local machine or kernel boot evidence unavailable");
    if process.pid == 0 || process.start_ticks == 0 { return Err(unavailable()); }
    let machine = machine_id().ok_or_else(unavailable)?;
    if recorded_machine != Some(machine.as_str()) { return Err(unavailable()); }
    let old_boot = process.kernel_boot_id.as_deref().filter(|s| {
        s.len() == 36 && s.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() }
        })
    }).ok_or_else(unavailable)?;
    let boot = kernel_boot_id().ok_or_else(unavailable)?;
    if old_boot != boot { return Ok(false); }
    incarnation_running(i32::try_from(process.pid).map_err(|_| unavailable())?, process.start_ticks)
}

/// Capture a live build only within the managed job's exact process scope and
/// source checkout. Action JSON can nominate a PID, but cannot supply this proof.
pub(crate) fn managed_build_identity(
    process: &cockpit_protocol::orchestration::NativeProcessIdentity,
    scope: &cockpit_protocol::orchestration::NativeProcessIdentity,
    source: &std::path::Path,
    machine: &str,
) -> io::Result<NativeShellIdentity> {
    if !same_machine_process_running(process, Some(machine))?
        || !same_machine_process_running(scope, Some(machine))? {
        return Err(io::Error::other("build or managed job incarnation is absent"));
    }
    let pid = i32::try_from(process.pid).map_err(io::Error::other)?;
    let scope_pid = i32::try_from(scope.pid).map_err(io::Error::other)?;
    let executable = executable_identity(pid, process.start_ticks)?;
    if executable.process != *process || !managed_descendant(pid, scope_pid, scope.start_ticks)? {
        return Err(io::Error::other("build is not a descendant of the managed job"));
    }
    verify_source_cwd(pid, source)?;
    let fresh = executable_identity(pid, process.start_ticks)?;
    if fresh != executable || !managed_descendant(pid, scope_pid, scope.start_ticks)?
        || !same_machine_process_running(scope, Some(machine))? {
        return Err(io::Error::other("build evidence changed during observation"));
    }
    verify_source_cwd(pid, source)?;
    Ok(executable)
}

fn managed_descendant(pid: i32, scope: i32, scope_start: u64) -> io::Result<bool> {
    managed_descendant_with(pid, scope, scope_start, read_process)
}

fn managed_descendant_with(
    mut pid: i32, scope: i32, scope_start: u64,
    mut read: impl FnMut(i32) -> io::Result<ProcessStatus>,
) -> io::Result<bool> {
    if pid == scope { return Ok(false); }
    let mut chain = [(0, 0, 0); 128];
    for depth in 0..chain.len() {
        let current = read(pid)?;
        if current.exited || current.parent <= 0 || current.parent == pid {
            return Ok(false);
        }
        chain[depth] = (pid, current.start, current.parent);
        if current.parent == scope {
            let parent = read(scope)?;
            if parent.start != scope_start || parent.exited { return Ok(false); }
            for &(pid, start, parent) in &chain[..=depth] {
                let fresh = read(pid)?;
                if fresh.start != start || fresh.exited || fresh.parent != parent { return Ok(false); }
            }
            let fresh = read(scope)?;
            return Ok(fresh.start == scope_start && !fresh.exited);
        }
        pid = current.parent;
    }
    Ok(false)
}

#[cfg(target_os = "linux")]
fn verify_source_cwd(pid: i32, source: &std::path::Path) -> io::Result<()> {
    let source = std::fs::canonicalize(source)?;
    let cwd = std::fs::canonicalize(format!("/proc/{pid}/cwd"))?;
    if !cwd.starts_with(source) {
        return Err(io::Error::other("build cwd is outside the source checkout"));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn verify_source_cwd(_pid: i32, _source: &std::path::Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "build cwd evidence unavailable on this OS"))
}

#[cfg(test)]
mod machine_tests {
    use super::*;
    #[test]
    fn machine_hash_is_domain_separated_and_rejects_missing_identity() {
        assert!(machine_digest(b"").is_none());
        assert!(machine_digest(b"00000000000000000000000000000000\n").is_none());
        assert!(machine_digest(b"not-a-machine").is_none());
        let lower = machine_digest(b"abcdef0123456789abcdef0123456789\n").unwrap();
        assert_eq!(Some(lower.clone()), machine_digest(b"ABCDEF0123456789ABCDEF0123456789"));
        assert_ne!(lower, "abcdef0123456789abcdef0123456789");
    }
    #[test]
    fn missing_foreign_or_malformed_identity_never_proves_exit() {
        let mut process = cockpit_protocol::orchestration::NativeProcessIdentity {
            pid: std::process::id(), start_ticks: 1,
            kernel_boot_id: Some("00000000-0000-0000-0000-000000000001".into()),
        };
        assert!(same_machine_process_running(&process, None).is_err());
        assert!(same_machine_process_running(&process, Some("different-machine")).is_err());
        let machine = machine_id();
        process.start_ticks = 0;
        assert!(same_machine_process_running(&process, machine.as_deref()).is_err());
        process.start_ticks = 1;
        process.kernel_boot_id = Some("unknown".into());
        assert!(same_machine_process_running(&process, machine.as_deref()).is_err());
    }
}

/// Prove that the nominated incarnation is a strict ancestor of this process.
/// Bound the walk and reject replacement of the nominated PID during observation.
pub fn is_ancestor_of_self(pid: i32, max_depth: usize) -> bool {
    let Ok(mut current) = i32::try_from(std::process::id()) else {
        return false;
    };
    if pid == current || max_depth == 0 {
        return false;
    }
    let Ok(candidate) = read_process(pid) else {
        return false;
    };
    if candidate.exited {
        return false;
    }
    for _ in 0..max_depth {
        let Ok(process) = read_process(current) else {
            return false;
        };
        if process.exited || process.parent <= 0 || process.parent == current {
            return false;
        }
        current = process.parent;
        if current == pid {
            return read_process(pid).is_ok_and(|fresh| {
                fresh.start == candidate.start && !fresh.exited
            });
        }
    }
    false
}

struct ProcessStatus {
    parent: i32,
    start: u64,
    exited: bool,
    #[cfg(target_os = "linux")]
    process_group: i32,
    #[cfg(target_os = "linux")]
    tty: i64,
    #[cfg(target_os = "linux")]
    foreground_group: i32,
    #[cfg(target_os = "linux")]
    stopped: bool,
}

#[cfg(target_os = "linux")]
impl ProcessStatus {
    fn foreground(&self, start: u64) -> bool {
        self.start == start && !self.exited && !self.stopped
            && self.tty != 0 && self.process_group > 0
            && self.process_group == self.foreground_group
    }
}

#[cfg(target_os = "linux")]
fn read_process(pid: i32) -> io::Result<ProcessStatus> {
    valid_pid(pid)?;
    match read_linux_stat(std::path::Path::new(&format!("/proc/{pid}/stat")), pid) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A missing procfs is unavailable evidence, not proof of PID exit.
            read_linux_stat(std::path::Path::new("/proc/self/stat"), std::process::id() as i32)
                .map_err(|error| io::Error::other(format!("procfs evidence unavailable: {error}")))?;
            Err(error)
        }
        result => result,
    }
}

#[cfg(target_os = "linux")]
fn read_linux_stat(path: &std::path::Path, pid: i32) -> io::Result<ProcessStatus> {
    let stat = std::fs::read_to_string(path)?;
    parse_linux_stat(&stat, pid)
}

#[cfg(target_os = "linux")]
fn parse_linux_stat(stat: &str, pid: i32) -> io::Result<ProcessStatus> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid process stat evidence");
    let (recorded_pid, rest) = stat.split_once(" (").ok_or_else(invalid)?;
    if recorded_pid.parse::<i32>().ok() != Some(pid) {
        return Err(invalid());
    }
    // comm can contain spaces and ')' characters; only the last ')' ends it.
    let close = rest.rfind(')').ok_or_else(invalid)?;
    let fields = rest.get(close + 1..).and_then(|value| value.strip_prefix(' '))
        .ok_or_else(invalid)?;
    let mut fields = fields.split_whitespace();
    let state = fields.next().ok_or_else(invalid)?;
    if !matches!(state, "R" | "S" | "D" | "Z" | "T" | "t" | "X" | "x" | "K" | "W" | "P" | "I") {
        return Err(invalid());
    }
    let parent = fields.next().and_then(|field| field.parse::<i32>().ok())
        .filter(|parent| *parent >= 0).ok_or_else(invalid)?;
    let process_group = fields.next().and_then(|field| field.parse::<i32>().ok())
        .filter(|group| *group >= 0).ok_or_else(invalid)?;
    fields.next().and_then(|field| field.parse::<i32>().ok())
        .filter(|session| *session >= 0).ok_or_else(invalid)?;
    let tty = fields.next().and_then(|field| field.parse::<i64>().ok()).ok_or_else(invalid)?;
    let foreground_group = fields.next().and_then(|field| field.parse::<i32>().ok())
        .filter(|group| *group >= -1).ok_or_else(invalid)?;
    // /proc stat field 22: starttime (tpgid above is field 8).
    let start = fields.nth(13).and_then(|field| field.parse::<u64>().ok())
        .ok_or_else(invalid)?;
    Ok(ProcessStatus { parent, start, exited: matches!(state, "Z" | "X" | "x"),
        process_group, tty, foreground_group, stopped: matches!(state, "T" | "t") })
}

#[cfg(target_os = "macos")]
fn read_process(pid: i32) -> io::Result<ProcessStatus> {
    use libproc::{bsd_info::BSDInfo, proc_pid::pidinfo};
    use nix::{errno::Errno, sys::signal::kill, unistd::Pid};

    valid_pid(pid)?;
    let info = match pidinfo::<BSDInfo>(pid, 0) {
        Ok(info) => info,
        Err(detail) => {
            // libproc reports text, not a reliable errno. Only a separate ESRCH
            // proves absence; permission/read failures remain unverifiable.
            return match kill(Pid::from_raw(pid), None) {
                Err(Errno::ESRCH) => Err(io::Error::new(io::ErrorKind::NotFound, detail)),
                _ => Err(io::Error::other(detail)),
            };
        }
    };
    if info.pbi_pid != pid as u32 || info.pbi_start_tvusec >= 1_000_000 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid libproc identity"));
    }
    let exited = match info.pbi_status {
        nix::libc::SZOMB => true,
        nix::libc::SIDL | nix::libc::SRUN | nix::libc::SSLEEP | nix::libc::SSTOP => false,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown libproc process state")),
    };
    let start = info.pbi_start_tvsec.checked_mul(1 << 20)
        .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "libproc start identity overflow"))?;
    let parent = i32::try_from(info.pbi_ppid)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(ProcessStatus { parent, start, exited })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_process(pid: i32) -> io::Result<ProcessStatus> {
    valid_pid(pid)?;
    Err(io::Error::new(io::ErrorKind::Unsupported, "process identity unavailable on this OS"))
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;
    use std::process::Command;

    fn fake_status(parent: i32, start: u64) -> ProcessStatus {
        ProcessStatus { parent, start, exited: false,
            #[cfg(target_os = "linux")]
            process_group: 0,
            #[cfg(target_os = "linux")]
            tty: 0,
            #[cfg(target_os = "linux")]
            foreground_group: 0,
            #[cfg(target_os = "linux")]
            stopped: false,
        }
    }

    #[test]
    fn managed_build_scope_rejects_unrelated_and_reused_processes() {
        let reader = |pid| Ok(match pid {
            41 => fake_status(40, 9),
            40 => fake_status(1, 8),
            _ => fake_status(0, 1),
        });
        assert!(managed_descendant_with(41, 40, 8, reader).unwrap());
        assert!(!managed_descendant_with(41, 40, 7, reader).unwrap());
        assert!(!managed_descendant_with(41, 50, 8, reader).unwrap());
        assert!(!managed_descendant_with(40, 40, 8, reader).unwrap());
        assert!(managed_descendant_with(41, 40, 8, |_| Err(io::Error::other("unknown ancestry"))).is_err());
    }

    #[test]
    fn managed_build_scope_rechecks_every_ancestor_before_admission() {
        let mut intermediate_reads = 0;
        let valid = managed_descendant_with(42, 40, 8, |pid| {
            Ok(match pid {
                42 => fake_status(41, 10),
                41 => {
                    intermediate_reads += 1;
                    fake_status(if intermediate_reads == 1 { 40 } else { 99 }, 9)
                }
                40 => fake_status(1, 8),
                _ => fake_status(0, 1),
            })
        }).unwrap();
        assert!(!valid, "reparenting an intermediate ancestor must invalidate job scope");
    }

    #[test]
    fn current_incarnation_has_stable_identity() {
        let pid = std::process::id() as i32;
        let start = start_identity(pid).unwrap();
        assert_eq!(start_identity(pid), Some(start));
        assert!(incarnation_running(pid, start).unwrap());
        assert!(!incarnation_running(pid, start.wrapping_add(1)).unwrap());
        assert!(start_identity(0).is_none());
        assert_eq!(incarnation_running(0, start).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn reaped_child_is_not_running() {
        let mut child = Command::new("sh").args(["-c", "read line"]).stdin(std::process::Stdio::piped())
            .spawn().unwrap();
        let pid = child.id() as i32;
        let start = start_identity(pid).unwrap();
        assert!(incarnation_running(pid, start).unwrap());
        drop(child.stdin.take());
        child.wait().unwrap();
        assert!(!incarnation_running(pid, start).unwrap());
    }

    #[test]
    fn ancestry_requires_a_live_parent_not_self_or_child() {
        let pid = std::process::id() as i32;
        let parent = read_process(pid).unwrap().parent;
        assert!(is_ancestor_of_self(parent, 64));
        assert!(!is_ancestor_of_self(parent, 0));
        assert!(!is_ancestor_of_self(pid, 64));
        let mut child = Command::new("sh").args(["-c", "read line"]).stdin(std::process::Stdio::piped())
            .spawn().unwrap();
        let is_ancestor = is_ancestor_of_self(child.id() as i32, 64);
        drop(child.stdin.take());
        child.wait().unwrap();
        assert!(!is_ancestor);
    }

    #[cfg(target_os = "linux")]
    fn stat(state: &str, start: &str) -> String {
        format!("42 (name with ) parentheses) {state} 12 {} {start} 0", ["0"; 17].join(" "))
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn foreground_proof_rejects_background_stopped_and_reused_processes() {
        for (state, group, tty, foreground, expected) in [
            ("S", "42", "1234", "42", true),
            ("S", "42", "1234", "99", false),
            ("T", "42", "1234", "42", false),
            ("t", "42", "1234", "42", false),
            ("Z", "42", "1234", "42", false),
            ("S", "42", "0", "42", false),
            ("S", "42", "1234", "-1", false),
        ] {
            let stat = format!("42 (omp) {state} 12 {group} 42 {tty} {foreground} {} 123 0",
                ["0"; 13].join(" "));
            let process = parse_linux_stat(&stat, 42).unwrap();
            assert_eq!(process.foreground(123), expected);
            assert!(!process.foreground(124));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_stat_distinguishes_exit_from_malformed_evidence() {
        let process = parse_linux_stat(&stat("S", "123"), 42).unwrap();
        assert_eq!(process.parent, 12);
        assert_eq!(process.start, 123);
        assert!(!process.exited);
        for state in ["Z", "X", "x"] {
            assert!(parse_linux_stat(&stat(state, "123"), 42).unwrap().exited);
        }
        for malformed in ["", "42 (broken", "42 (cmd) S 12", &stat("?", "123"), &stat("S", "bad")] {
            assert_eq!(parse_linux_stat(malformed, 42).err().unwrap().kind(), io::ErrorKind::InvalidData);
        }
        assert_eq!(parse_linux_stat(&stat("S", "123"), 43).err().unwrap().kind(), io::ErrorKind::InvalidData);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unreadable_stat_is_not_absence() {
        // Opening a directory fails even as root, unlike a chmod-based fixture.
        let error = read_linux_stat(std::path::Path::new("/proc/self"), std::process::id() as i32)
            .err().unwrap();
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_boot_id_is_stable_kernel_evidence() {
        let boot = kernel_boot_id().unwrap();
        assert_eq!(boot.len(), 36);
        assert_eq!(kernel_boot_id(), Some(boot));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn argv_evidence_rejects_missing_data_and_preserves_argument_boundaries() {
        for argv in [b"".as_slice(), b"\0", b"sh", b"sh\0argument"] {
            assert_eq!(digest_argv(argv).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
        assert_ne!(digest_argv(b"sh\0a b\0").unwrap(), digest_argv(b"sh\0a\0b\0").unwrap());
        assert!(digest_argv(b"sh\0\xff\0").is_ok());
        let pid = std::process::id() as i32;
        let start = start_identity(pid).unwrap();
        assert!(executable_identity(pid, start.wrapping_add(1)).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exec_replacement_changes_shell_fingerprint_without_changing_incarnation() {
        use std::io::{BufRead, BufReader, Write};
        use std::process::Stdio;
        use std::time::{Duration, Instant};

        let mut child = Command::new("sh")
            .args(["-c", "printf 'ready\\n'; read line; exec sleep 60"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut ready = String::new();
        output.read_line(&mut ready).unwrap();
        assert_eq!(ready, "ready\n");
        let pid = child.id() as i32;
        let start = start_identity(pid).unwrap();
        let baseline = executable_identity(pid, start).unwrap();
        let original_stable = executable_identity(pid, start).unwrap() == baseline;
        child.stdin.as_mut().unwrap().write_all(b"go\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let replacement = loop {
            if let Ok(fresh) = executable_identity(pid, start)
                && fresh != baseline {
                    break Some(fresh);
                }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        // This is only the child created by this test.
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(original_stable);
        let replacement = replacement.expect("exec must change executable/argv evidence");
        assert_eq!(replacement.process, baseline.process);
        assert_ne!(
            (&replacement.executable_device, &replacement.executable_inode),
            (&baseline.executable_device, &baseline.executable_inode),
        );
        assert_ne!(replacement.argv_digest, baseline.argv_digest);
        assert!(executable_identity(pid, start).is_err());
    }
}
