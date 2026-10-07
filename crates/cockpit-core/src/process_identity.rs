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

    #[test]
    fn current_incarnation_has_stable_identity() {
        let pid = std::process::id() as i32;
        let start = start_identity(pid).unwrap();
        assert_eq!(start_identity(pid), Some(start));
        assert_eq!(incarnation_running(pid, start).unwrap(), true);
        assert_eq!(incarnation_running(pid, start.wrapping_add(1)).unwrap(), false);
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
            if let Ok(fresh) = executable_identity(pid, start) {
                if fresh != baseline {
                    break Some(fresh);
                }
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
